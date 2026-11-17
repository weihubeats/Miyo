package io.miyo.debug;

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 本地注册文件：start 时写 {@code ~/.miyo/debug-runtimes/{app}-{port}.json}，
 * stop 时删除。Miyo 桌面端扫描该目录即可列出本机所有正在运行的调试 Runtime，
 * 免去用户手抄端口。
 *
 * <p>隐私边界：注册文件只含自述信息（应用名 / 端口 / Java 版本 / PID），
 * <b>绝不写 token</b>——连接鉴权仍由用户在 Miyo 侧填入。注册可通过环境变量
 * {@code MIYO_DEBUG_REGISTRY=off} 关闭（企业环境禁止写用户目录时）；
 * 关闭后 Miyo 仍可用端口探测兜底发现（对默认端口段做指纹探测）。
 *
 * <p>进程崩溃时文件残留：文件里带 PID，Miyo 扫描后会先探测端口活性，
 * 探测失败即视为过期条目（不会误展示）。文件系统失败只记 stderr，
 * 不影响 Runtime 启动——注册是发现辅助，不是功能前提。
 */
final class RuntimeRegistry {

    /** 注册目录（用户主目录下；Windows 同理 %USERPROFILE%）。 */
    static final String REGISTRY_DIR_ENV = "MIYO_DEBUG_REGISTRY";
    private static final String REGISTRY_SUBDIR = ".miyo" + java.io.File.separator + "debug-runtimes";

    private final RuntimeConfig config;
    private final boolean enabled;
    private Path file;

    RuntimeRegistry(RuntimeConfig config) {
        this.config = config;
        this.enabled = !"off".equalsIgnoreCase(System.getenv(REGISTRY_DIR_ENV));
    }

    /** 写注册文件（start 时调用；失败不抛——不影响启动）。 */
    void register() {
        if (!enabled) {
            return;
        }
        try {
            Path dir = dir();
            Files.createDirectories(dir);
            Map<String, Object> doc = new LinkedHashMap<>();
            doc.put("app_name", config.appName);
            doc.put("port", config.port);
            doc.put("bind_host", config.bindHost);
            doc.put("java_version", System.getProperty("java.version", ""));
            doc.put("jvm_version", System.getProperty("java.vm.name", ""));
            doc.put("pid", pid());
            doc.put("sdk_version", RuntimeConfig.SDK_VERSION);
            doc.put("instance_provider", config.instanceProviderName);
            doc.put("registered_at", Instant.now().toString());
            doc.put("protocol_version", 1);
            Path target = dir.resolve(fileName());
            Files.write(target, Json.write(doc).getBytes(StandardCharsets.UTF_8));
            this.file = target;
        } catch (Exception e) {
            // 注册失败不影响 Runtime 功能；提示用户可改用端口探测。
            System.err.println("[miyo-debug] 注册文件写入失败（Miyo 将改用端口探测发现）："
                    + e.getMessage());
        }
    }

    /** 删除注册文件（stop / 崩溃钩子时调用；文件不存在静默）。 */
    void unregister() {
        if (file == null) {
            return;
        }
        try {
            Files.deleteIfExists(file);
        } catch (Exception e) {
            System.err.println("[miyo-debug] 注册文件删除失败： " + e.getMessage());
        } finally {
            file = null;
        }
    }

    /** 注册文件名：{appName}-{port}.json（app 名限制为文件安全字符）。 */
    private String fileName() {
        String safe = config.appName.replaceAll("[^A-Za-z0-9._-]", "_");
        return safe + "-" + config.port + ".json";
    }

    private static Path dir() {
        return Path.of(System.getProperty("user.home"), REGISTRY_SUBDIR.split(java.io.File.separator));
    }

    /** 当前进程 PID（取不到返回 -1）。 */
    static long pid() {
        try {
            String name = ManagementFactory.getRuntimeMXBean().getName();
            int at = name.indexOf('@');
            return at > 0 ? Long.parseLong(name.substring(0, at)) : -1;
        } catch (Exception e) {
            return -1;
        }
    }

    /**
     * 读取注册目录里所有条目（Miyo 侧扫描用；损坏 / 非法文件跳过）。
     * 包内可见：SelfTest 与跨包工具类需要时可改 public。
     */
    static List<Map<String, Object>> readAll() {
        List<Map<String, Object>> out = new ArrayList<>();
        Path dir;
        try {
            dir = dir();
        } catch (Exception e) {
            return out;
        }
        if (!Files.isDirectory(dir)) {
            return out;
        }
        try (DirectoryStream<Path> stream = Files.newDirectoryStream(dir, "*.json")) {
            for (Path p : stream) {
                try {
                    String text = Files.readString(p, StandardCharsets.UTF_8);
                    Object parsed = Json.parse(text);
                    if (parsed instanceof Map) {
                        @SuppressWarnings("unchecked")
                        Map<String, Object> doc = (Map<String, Object>) parsed;
                        out.add(doc);
                    }
                } catch (Exception ignore) {
                    // 单个文件损坏不影响整体扫描。
                }
            }
        } catch (IOException ignore) {
            // 目录不可读：返回空列表（Miyo 侧走端口探测兜底）。
        }
        return out;
    }
}
