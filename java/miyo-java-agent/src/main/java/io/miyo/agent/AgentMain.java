package io.miyo.agent;

import java.io.IOException;
import java.lang.instrument.Instrumentation;
import java.nio.file.Path;
import java.nio.file.Paths;

import io.miyo.agent.inspect.ClassInspector;
import io.miyo.agent.invoke.InstanceResolver;
import io.miyo.agent.invoke.InvocationEngine;
import io.miyo.agent.ipc.IpcException;
import io.miyo.agent.ipc.IpcServer;
import io.miyo.agent.ipc.Protocol;
import io.miyo.agent.ipc.RequestDispatcher;
import io.miyo.agent.spring.ContextDiscovery;
import io.miyo.agent.util.Json;

/**
 * Agent 入口（agentmain 动态加载 / premain 静态加载共用 boot 流程）。
 *
 * 加载顺序（loadAgent 阻塞至 agentmain 返回，必须快进快出）：
 * 1. 解析参数（token 必配，缺失即失败——写 errorFile 供 launcher 诊断）；
 * 2. 重复 attach 检测（已运行 → 换发 token + 重写 infoFile，直接返回）；
 * 3. expectedPid 身份绑定校验（可选，防御参数串线）；
 * 4. 绑定 IPC 服务（127.0.0.1 随机端口）→ 写 infoFile（连接信息）；
 * 5. Spring 捕获插桩后台安装（不阻塞 agentmain）。
 *
 * 诚实边界：Agent 无法从目标 JVM 卸载已加载的类；agent.stop 只停服务、
 * 关闭监听、清 infoFile，内存占用（少量）与插桩（若有）残留至 JVM 退出。
 */
public final class AgentMain {

    private static volatile IpcServer server;
    private static volatile RequestDispatcher dispatcher;
    private static volatile ContextDiscovery discovery;
    private static volatile ClassInspector inspector;
    private static volatile InvocationEngine engine;

    /** 动态加载入口。 */
    public static void agentmain(String args, Instrumentation inst) {
        boot(args, inst, false);
    }

    /** 静态加载入口（-javaagent；测试与预启动共用）。 */
    public static void premain(String args, Instrumentation inst) {
        boot(args, inst, true);
    }

    private static synchronized void boot(String args, Instrumentation inst, boolean premain) {
        AgentArgs parsed;
        Path infoFile = null;
        try {
            parsed = AgentArgs.parse(args);
            String infoFileRaw = parsed.get("infoFile");
            infoFile = infoFileRaw == null || infoFileRaw.trim().isEmpty()
                    ? null : Paths.get(infoFileRaw.trim());
        } catch (Exception e) {
            failFast(infoFile, "agent 参数解析失败：" + e.getMessage());
            return;
        }

        try {
            // initialize 幂等：新 JVM 建实例（返回 true，继续装配）；重复 attach
            // 换发 token + 重写 infoFile（返回 false，直接结束——服务已在运行）。
            if (!AgentState.initialize(parsed, infoFile)) {
                return;
            }
            // 进程身份绑定（可选）：expectedPid 不匹配即拒绝启动（防参数串线到错误 JVM）
            long expectedPid = parsed.getLong("expectedPid", -1);
            if (expectedPid > 0 && AgentState.currentPid() != expectedPid) {
                throw new IllegalStateException(
                        "进程身份校验失败：期望 pid=" + expectedPid + "，实际 pid="
                                + AgentState.currentPid());
            }
        } catch (Exception e) {
            failFast(infoFile, e.getMessage());
            return;
        }

        // 核心组件装配（失败 = 初始化失败，写诊断文件）
        try {
            inspector = new ClassInspector(inst);
            discovery = new ContextDiscovery(inspector);
            engine = new InvocationEngine(
                    inspector,
                    new InstanceResolver(discovery, false),
                    AgentState.current());
            dispatcher = new RequestDispatcher(AgentState.current(), inspector, discovery, engine);
            IpcServer ipcServer = new IpcServer(dispatcher);
            int port = ipcServer.start();
            server = ipcServer;
            // 绑定成功后写 infoFile（重复 attach 路径已写——端口未变）
            AgentState.current().writeInfoFile(infoFile);
            System.err.println("[miyo-agent] 已加载（v" + AgentVersion.VERSION
                    + "，IPC 127.0.0.1:" + port + "，pid=" + AgentState.currentPid() + "）");
        } catch (Exception e) {
            failFast(infoFile, "Agent 初始化失败：" + e.getMessage());
            return;
        }

        // Spring 捕获插桩：后台安装（retransform 短暂停顿不该阻塞 agentmain）
        Thread capture = new Thread(() -> {
            try {
                discovery.installCapture(inst, inspector);
            } catch (Throwable t) {
                System.err.println("[miyo-agent] Spring 捕获安装失败: " + t);
            }
        }, "miyo-agent-capture-install");
        capture.setDaemon(true);
        capture.start();

        // JVM 退出时清理 infoFile（不阻止退出）
        try {
            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                AgentState current = AgentState.current();
                if (current != null) {
                    current.removeInfoFile();
                }
            }, "miyo-agent-cleanup"));
        } catch (IllegalStateException ignored) {
            // JVM 已在关闭中
        }
    }

    /** agent.stop / 停机：停服务 + 标记停止 + 清 infoFile（类不可卸载——诚实语义）。 */
    public static void shutdown() {
        AgentState current = AgentState.current();
        if (current != null) {
            current.markStopped();
        }
        IpcServer ipcServer = server;
        if (ipcServer != null) {
            ipcServer.stop();
        }
        System.err.println("[miyo-agent] 已停止（已加载的类无法卸载，随 JVM 退出回收）");
    }

    private static void failFast(Path infoFile, String message) {
        System.err.println("[miyo-agent] 启动失败：" + message);
        if (infoFile != null) {
            try {
                AgentState.writeErrorFile(infoFile, message);
            } catch (Throwable ignored) {
                // 尽力而为
            }
        }
    }

    /** 供测试 / 诊断：当前装配状态摘要（JSON）。 */
    public static String diagnose() {
        java.util.Map<String, Object> out = new java.util.LinkedHashMap<>();
        out.put("agent_version", AgentVersion.VERSION);
        out.put("protocol", Protocol.PROTOCOL_VERSION);
        out.put("running", AgentState.isRunning());
        out.put("server_started", server != null && server.isStarted());
        out.put("capture_state", discovery != null ? io.miyo.agent.spring.SpringCapture.state()
                : "not_installed");
        return Json.write(out);
    }
}
