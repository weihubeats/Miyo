package io.miyo.launcher;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Properties;

/**
 * Launcher CLI 入口（JSON 单行输出到 stdout；诊断进 stderr；退出码表意）。
 *
 * 用法：
 *   capabilities                      —— 运行环境自检（桥类型 / tools.jar / java 版本）
 *   list                              —— 枚举可 attach 的 JVM
 *   properties --pid N                —— 读目标 JVM 系统属性（预检版本 / 厂商，不加载 agent）
 *   attach --pid N --agent PATH --args "k=v,..." --info-file PATH [--timeout-ms 15000]
 *                                     —— attach → loadAgent → 轮询 infoFile → 输出连接信息
 *
 * 退出码：0 成功；2 attach 不支持；3 无 attach 工具链；4 进程不存在；
 * 5 agent 加载 / 初始化失败；6 超时；7 IO / 参数错误。
 */
public final class LauncherMain {

    // Java 8 兼容：PrintStream(OutputStream,boolean,Charset) 是 Java 10+，改用 OutputStreamWriter
    private static final java.io.PrintWriter OUT = new java.io.PrintWriter(
            new java.io.OutputStreamWriter(System.out, StandardCharsets.UTF_8), true);
    private static final java.io.PrintWriter ERR = new java.io.PrintWriter(
            new java.io.OutputStreamWriter(System.err, StandardCharsets.UTF_8), true);

    public static void main(String[] rawArgs) {
        int exit;
        try {
            exit = run(rawArgs);
        } catch (CliError e) {
            OUT.println(jsonError(e.code, e.getMessage(), e.hint));
            exit = e.exitCode;
        } catch (Exception e) {
            OUT.println(jsonError("internal", "launcher 内部错误：" + e, e.getClass().getName()));
            exit = 7;
        }
        System.exit(exit);
    }

    private static int run(String[] args) throws Exception {
        if (args.length == 0) {
            throw usage();
        }
        String command = args[0];
        Map<String, String> opts = parseOptions(args);
        switch (command) {
            case "capabilities":
                return capabilities();
            case "list":
                return list();
            case "properties":
                return properties(opts);
            case "attach":
                return attach(opts);
            default:
                throw usage();
        }
    }

    // ------------------------------------------------------------------
    // 命令实现
    // ------------------------------------------------------------------

    private static int capabilities() {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("java_version", System.getProperty("java.version"));
        out.put("java_home", System.getProperty("java.home"));
        out.put("os", System.getProperty("os.name") + " " + System.getProperty("os.version"));
        out.put("os_arch", System.getProperty("os.arch"));
        AttachBridge bridge = AttachBridge.Factory.create();
        out.put("bridge_available", bridge != null);
        out.put("bridge_desc", bridge != null ? bridge.describe() : "无可用的 Attach 工具链"
                + "（JDK 9+ 缺 jdk.attach 模块，或 JDK 8 缺 tools.jar）");
        File tools = AttachBridge.Jdk8Bridge.locateToolsJar();
        out.put("tools_jar", tools != null && tools.isFile() ? tools.getPath() : null);
        OUT.println(json(out));
        return 0;
    }

    private static int list() throws Exception {
        AttachBridge bridge = requireBridge();
        List<AttachBridge.VmDescriptor> vms = bridge.list();
        StringBuilder items = new StringBuilder("[");
        for (int i = 0; i < vms.size(); i++) {
            AttachBridge.VmDescriptor vm = vms.get(i);
            if (i > 0) {
                items.append(',');
            }
            items.append("{\"pid\":").append(vm.pid)
                    .append(",\"display_name\":").append(jsonString(vm.displayName))
                    .append('}');
        }
        items.append(']');
        OUT.println("{\"bridge\":\"" + bridge.describe() + "\",\"items\":" + items + '}');
        return 0;
    }

    private static int properties(Map<String, String> opts) throws Exception {
        long pid = requirePid(opts);
        AttachBridge bridge = requireBridge();
        Properties props = bridge.loadSystemProperties(pid);
        StringBuilder out = new StringBuilder("{");
        String[] keys = {"java.version", "java.vm.version", "java.vm.vendor", "java.vm.name",
                "java.home", "java.runtime.version"};
        for (int i = 0; i < keys.length; i++) {
            if (i > 0) {
                out.append(',');
            }
            out.append(jsonString(keys[i])).append(':')
                    .append(jsonString(props.getProperty(keys[i])));
        }
        out.append('}');
        OUT.println(out);
        return 0;
    }

    private static int attach(Map<String, String> opts) throws Exception {
        long pid = requirePid(opts);
        String agentPath = requireOpt(opts, "agent");
        String agentArgs = opts.getOrDefault("args", "");
        String infoFile = requireOpt(opts, "info-file");
        long timeoutMs = Long.parseLong(opts.getOrDefault("timeout-ms", "20000"));
        AttachBridge bridge = requireBridge();

        Path infoPath = Paths.get(infoFile);
        // 幂等起点：清掉陈旧文件，避免读到上一轮 attach 的端口
        try {
            Files.deleteIfExists(infoPath);
        } catch (Exception ignored) {
            // 尽力而为
        }

        boolean loadError = false;
        String loadErrorMessage = null;
        try {
            bridge.attachAndLoad(pid, agentPath, agentArgs);
        } catch (AttachBridge.AttachBridgeException e) {
            // 版本偏斜兜底：JDK 21+ 客户端 attach JDK 8 目标时，attach 协议返回码
            // 解读有偏斜（agent 实际已加载成功，客户端误报 AgentLoadException）。
            // 这里不立即失败——继续轮询 infoFile；轮询不到才上报原错误。
            loadError = true;
            loadErrorMessage = e.getMessage();
            ERR.println("[launcher] loadAgent 报异常，继续轮询 infoFile 兜底: " + e.getMessage());
        }

        // 轮询 infoFile（agentmain 快进快出，但冷 JVM / 插桩等待仍需余量）
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (Files.isRegularFile(infoPath)) {
                String content = new String(Files.readAllBytes(infoPath), StandardCharsets.UTF_8);
                if (content.contains("\"ok\":false")) {
                    ERR.println("[launcher] agent 报告失败: " + content);
                    return 5;
                }
                OUT.println(content);
                return 0;
            }
            Thread.sleep(120);
        }
        if (loadError) {
            throw new CliError(5, "agent_load_failed",
                    "Agent 加载失败：" + loadErrorMessage
                            + "（且未收到连接信息）",
                    "确认目标进程 stderr 中的 [miyo-agent] 日志；"
                            + "动态 attach 被禁用 / Agent 参数非法等会在目标 stderr 有详情");
        }
        throw new CliError(6, "timeout",
                "超时未收到 Agent 连接信息（infoFile 未生成：" + infoFile + "）",
                "目标 JVM 可能禁止动态 agent 加载（JDK 21+ 有告警但可用；"
                        + "DisableAttachMechanism 会直接拒绝 attach），或 agentmain 挂起");
    }

    // ------------------------------------------------------------------
    // 工具
    // ------------------------------------------------------------------

    private static AttachBridge requireBridge() throws CliError {
        AttachBridge bridge = AttachBridge.Factory.create();
        if (bridge == null) {
            throw new CliError(3, "no_attach_toolchain",
                    "当前 JVM 无可用 Attach 工具链（JDK 9+ 需要 jdk.attach 模块；JDK 8 需要 tools.jar）",
                    "在 Miyo 设置中指定一个完整 JDK（非 jlink 裁剪 / 非 JRE）后重试");
        }
        return bridge;
    }

    private static long requirePid(Map<String, String> opts) throws CliError {
        String raw = requireOpt(opts, "pid");
        try {
            return Long.parseLong(raw.trim());
        } catch (NumberFormatException e) {
            throw new CliError(7, "bad_usage", "非法 pid：" + raw, null);
        }
    }

    private static String requireOpt(Map<String, String> opts, String key) throws CliError {
        String value = opts.get(key);
        if (value == null || value.trim().isEmpty()) {
            throw new CliError(7, "bad_usage", "缺少必需参数 --" + key, null);
        }
        return value.trim();
    }

    private static Map<String, String> parseOptions(String[] args) {
        Map<String, String> out = new LinkedHashMap<>();
        for (int i = 1; i < args.length; i++) {
            String arg = args[i];
            if (arg.startsWith("--")) {
                String key = arg.substring(2);
                String value = "";
                if (i + 1 < args.length && !args[i + 1].startsWith("--")) {
                    value = args[++i];
                }
                out.put(key, value);
            }
        }
        return out;
    }

    private static CliError usage() {
        return new CliError(7, "bad_usage",
                "用法：launcher <capabilities|list|properties|attach> [options]", null);
    }

    private static String jsonError(String code, String message, String hint) {
        StringBuilder sb = new StringBuilder("{\"error\":{\"code\":\"");
        sb.append(code).append("\",\"message\":").append(jsonString(message));
        if (hint != null) {
            sb.append(",\"hint\":").append(jsonString(hint));
        }
        sb.append("}}");
        return sb.toString();
    }

    private static String json(Map<String, Object> map) {
        StringBuilder sb = new StringBuilder("{");
        boolean first = true;
        for (Map.Entry<String, Object> entry : map.entrySet()) {
            if (!first) {
                sb.append(',');
            }
            first = false;
            sb.append(jsonString(entry.getKey())).append(':');
            Object value = entry.getValue();
            if (value == null) {
                sb.append("null");
            } else if (value instanceof Number || value instanceof Boolean) {
                sb.append(value);
            } else {
                sb.append(jsonString(String.valueOf(value)));
            }
        }
        sb.append('}');
        return sb.toString();
    }

    private static String jsonString(String value) {
        if (value == null) {
            return "null";
        }
        StringBuilder sb = new StringBuilder("\"");
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"': sb.append("\\\""); break;
                case '\\': sb.append("\\\\"); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                default:
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
            }
        }
        sb.append('"');
        return sb.toString();
    }

    /** CLI 级错误（结构化输出 + 退出码）。 */
    private static final class CliError extends RuntimeException {
        private static final long serialVersionUID = 1L;
        final int exitCode;
        final String code;
        final String hint;

        CliError(int exitCode, String code, String message, String hint) {
            super(message);
            this.exitCode = exitCode;
            this.code = code;
            this.hint = hint;
        }
    }
}
