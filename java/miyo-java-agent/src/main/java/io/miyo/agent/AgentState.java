package io.miyo.agent;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

import io.miyo.agent.util.Json;

/**
 * Agent 运行时单例状态（同一 JVM 内的重复 attach 共享）。
 *
 * 职责：
 * - 重复 attach 检测：static 单例存在即已运行（第二次 agentmain 不再起
 *   第二个服务，改为「换发 token + 重写 infoFile」，连接信息保持单一）；
 * - token 常数时间比较（MessageDigest.isEqual）；
 * - infoFile 写盘（0600，POSIX；Windows 走默认 ACL）——Miyo 经 launcher
 *   轮询该文件获取连接信息；token 不落盘（Miyo 自持）；
 * - 生命周期计数（uptime / 已处理请求数）。
 */
public final class AgentState {

    /** JVM 内唯一实例；agentmain 由 HotSpot attach 线程串行加载。 */
    private static volatile AgentState instance;

    /** 连接凭证（Miyo 生成的一次性随机值）；重复 attach 时换发。 */
    private volatile String token;

    private final long startedAt = System.currentTimeMillis();
    private final int maxConcurrent;
    private final long maxTimeoutMs;
    private final int maxDepth;
    private final boolean allowStaticScan;
    private volatile Path infoFile;
    private volatile boolean stopped;
    /** 本次 agentmain 是否命中「已有实例运行」（重复 attach 识别）。 */
    private volatile boolean existingAttach;
    private final AtomicLong requestsTotal = new AtomicLong();

    private AgentState(String token, int maxConcurrent, long maxTimeoutMs, int maxDepth,
                       boolean allowStaticScan) {
        this.token = token;
        this.maxConcurrent = maxConcurrent;
        this.maxTimeoutMs = maxTimeoutMs;
        this.maxDepth = maxDepth;
        this.allowStaticScan = allowStaticScan;
    }

    /**
     * 初始化 / 重复 attach 换发。返回 true = 新实例（本次 agentmain 应启动服务）；
     * false = 已有实例运行（仅换发 token + 重写 infoFile）。
     */
    public static synchronized boolean initialize(AgentArgs args, Path infoFile)
            throws IllegalArgumentException {
        String token = args.require("token");
        if (token.length() < 32) {
            throw new IllegalArgumentException("token 长度不足（至少 32 字符）");
        }
        AgentState existing = instance;
        if (existing != null && !existing.stopped) {
            // 重复 attach：换发新 token（旧 token 立即失效），重写 infoFile。
            // existing 标记回传 Miyo（UI 提示「Agent 已在运行，无需重新注入」）。
            existing.existingAttach = true;
            existing.token = token;
            existing.writeInfoFile(infoFile);
            return false;
        }
        instance = new AgentState(
                token,
                clamp(args.getInt("maxConcurrent", AgentVersion.DEFAULT_MAX_CONCURRENT), 1, 32),
                Math.max(1_000L, args.getLong("maxTimeoutMs", AgentVersion.DEFAULT_MAX_TIMEOUT_MS)),
                Math.max(2, args.getInt("maxDepth", AgentVersion.DEFAULT_MAX_DEPTH)),
                args.getBool("allowStaticScan", false));
        instance.infoFile = infoFile;
        return true;
    }

    /** 当前实例（未初始化为 null）。 */
    public static AgentState current() {
        return instance;
    }

    /** 已初始化且未停止（重复 attach 判定依据）。 */
    public static boolean isRunning() {
        AgentState s = instance;
        return s != null && !s.stopped;
    }

    /** token 常数时间比较。 */
    public boolean tokenMatches(String candidate) {
        String t = token;
        if (candidate == null || t == null) {
            return false;
        }
        return MessageDigest.isEqual(
                t.getBytes(StandardCharsets.UTF_8), candidate.getBytes(StandardCharsets.UTF_8));
    }

    public int maxConcurrent() {
        return maxConcurrent;
    }

    public long maxTimeoutMs() {
        return maxTimeoutMs;
    }

    public int maxDepth() {
        return maxDepth;
    }

    public boolean staticScanAllowed() {
        return allowStaticScan;
    }

    public long uptimeMs() {
        return System.currentTimeMillis() - startedAt;
    }

    public long incrementRequests() {
        return requestsTotal.incrementAndGet();
    }

    public long requestsTotal() {
        return requestsTotal.get();
    }

    public boolean isStopped() {
        return stopped;
    }

    public void markStopped() {
        stopped = true;
        removeInfoFile();
    }

    /**
     * 写连接信息文件（0600）。内容不含 token；Miyo 读到 port 后凭自持 token 连接。
     *
     * 注意：父目录仅在实际缺失时创建（macOS /tmp 是 symlink，JDK 11/17 的
     * createDirectories 对已存在 symlink 目录会抛 FileAlreadyExistsException）。
     */
    public void writeInfoFile(Path path) {
        try {
            Path parent = path.getParent();
            if (parent != null && !Files.isDirectory(parent)) {
                Files.createDirectories(parent);
            }
            Map<String, Object> info = new LinkedHashMap<>();
            info.put("ok", true);
            info.put("protocol", AgentVersion.PROTOCOL_VERSION);
            info.put("agent_version", AgentVersion.VERSION);
            info.put("pid", currentPid());
            info.put("port", IpcPortHolder.port);
            info.put("vm_start_ms", vmStartTime());
            info.put("started_at", startedAt);
            info.put("existing", existingAttach);
            info.put("written_at", System.currentTimeMillis());
            Files.write(path, Json.write(info).getBytes(StandardCharsets.UTF_8),
                    java.nio.file.StandardOpenOption.CREATE,
                    java.nio.file.StandardOpenOption.WRITE,
                    java.nio.file.StandardOpenOption.TRUNCATE_EXISTING);
            trySetOwnerOnly(path);
            this.infoFile = path;
        } catch (IOException e) {
            throw new IllegalStateException("infoFile 写入失败：" + e, e);
        }
    }

    /** 写「attach 失败」诊断文件（launcher 据此返回结构化错误）。 */
    public static void writeErrorFile(Path path, String message) {
        try {
            Path parent = path.getParent();
            if (parent != null && !Files.isDirectory(parent)) {
                Files.createDirectories(parent);
            }
            Map<String, Object> err = new LinkedHashMap<>();
            err.put("ok", false);
            err.put("error", message);
            Files.write(path, Json.write(err).getBytes(StandardCharsets.UTF_8),
                    java.nio.file.StandardOpenOption.CREATE,
                    java.nio.file.StandardOpenOption.WRITE,
                    java.nio.file.StandardOpenOption.TRUNCATE_EXISTING);
            trySetOwnerOnly(path);
        } catch (IOException ignored) {
            // 连诊断文件都写不了：launcher 只能靠 loadAgent 异常兜底。
        }
    }

    public void removeInfoFile() {
        Path path = this.infoFile;
        if (path != null) {
            try {
                Files.deleteIfExists(path);
            } catch (IOException ignored) {
                // 清理失败无害（文件在数据目录内，Miyo 可按过期时间忽略）。
            }
        }
    }

    /** 当前 JVM 进程 pid（Java 8 兼容：RuntimeMXBean name "pid@host" 前缀）。 */
    public static long currentPid() {
        try {
            String name = java.lang.management.ManagementFactory.getRuntimeMXBean().getName();
            int at = name.indexOf('@');
            return at > 0 ? Long.parseLong(name.substring(0, at)) : -1;
        } catch (Exception e) {
            return -1;
        }
    }

    /** JVM 启动时刻（epoch ms）——Miyo 用于重启身份识别。 */
    public static long vmStartTime() {
        return java.lang.management.ManagementFactory.getRuntimeMXBean().getStartTime();
    }

    /** POSIX 尽力而为 0600（Windows / 非 POSIX 文件系统跳过）。 */
    private static void trySetOwnerOnly(Path path) {
        try {
            java.util.Set<java.nio.file.attribute.PosixFilePermission> ownerOnly =
                    java.util.EnumSet.of(java.nio.file.attribute.PosixFilePermission.OWNER_READ,
                            java.nio.file.attribute.PosixFilePermission.OWNER_WRITE);
            Files.setPosixFilePermissions(path, ownerOnly);
        } catch (UnsupportedOperationException | IOException ignored) {
            // 非 POSIX 文件系统：默认 ACL（当前用户目录下）已足够。
        }
    }

    /** IPC 端口的过渡持有者（IpcServer 绑定后回填；infoFile 写盘时读取）。 */
    public static final class IpcPortHolder {
        /** volatile：写盘线程可见。0 = 未绑定（此时不该写 infoFile）。 */
        public static volatile int port = 0;
    }

    private static int clamp(int v, int min, int max) {
        return Math.max(min, Math.min(max, v));
    }
}
