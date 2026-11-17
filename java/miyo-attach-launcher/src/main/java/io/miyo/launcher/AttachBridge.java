package io.miyo.launcher;

import java.io.File;
import java.io.IOException;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.util.ArrayList;
import java.util.List;

/**
 * Attach 能力桥（JDK 8 / 9+ 统一接口）。
 *
 * 实现选择（LauncherMain 在启动时判定）：
 * - Jdk9Bridge：直接引用 com.sun.tools.attach（jdk.attach 模块）；
 *   仅在检测到 VirtualMachine 类可加载时才被 link（类加载即失败则回退）；
 * - Jdk8Bridge：纯反射 + URLClassLoader 加载 {java.home}/../lib/tools.jar
 *   （macOS/Windows JDK8 布局）或 {java.home}/lib/tools.jar（Linux 布局）。
 *
 * 所有方法抛 AttachBridgeException（含结构化 code）——上层归一为错误 JSON。
 */
public interface AttachBridge {

    /** 枚举可 attach 的 JVM（pid + 显示名）。 */
    List<VmDescriptor> list() throws AttachBridgeException;

    /** attach 指定 pid 并加载 agent；返回前 detach。 */
    void attachAndLoad(long pid, String agentPath, String agentArgs)
            throws AttachBridgeException;

    /** attach 指定 pid 并读取系统属性（不加载 agent；预检版本 / 厂商）。 */
    java.util.Properties loadSystemProperties(long pid) throws AttachBridgeException;

    /** 当前桥的描述（诊断输出）。 */
    String describe();

    /** JVM 描述。 */
    final class VmDescriptor {
        public final long pid;
        public final String displayName;

        public VmDescriptor(long pid, String displayName) {
            this.pid = pid;
            this.displayName = displayName;
        }
    }

    /** 结构化异常（code → 上层错误码映射）。 */
    final class AttachBridgeException extends Exception {
        private static final long serialVersionUID = 1L;
        public final String code;

        public AttachBridgeException(String code, String message) {
            super(message);
            this.code = code;
        }

        public AttachBridgeException(String code, String message, Throwable cause) {
            super(message, cause);
            this.code = code;
        }
    }

    // ------------------------------------------------------------------
    // 工厂：探测当前 JVM 的 attach 能力
    // ------------------------------------------------------------------

    final class Factory {

        /**
         * 创建可用 bridge。JDK 9+ 优先（jdk.attach 在 boot layer）；
         * JDK 8 尝试 tools.jar 反射桥。两者皆失败 → null（诊断信息给上层）。
         */
        public static AttachBridge create() {
            // JDK 9+：类可加载即直接桥（不 link Jdk9Bridge 之外的内容）
            try {
                Class<?> vmClass = Class.forName("com.sun.tools.attach.VirtualMachine");
                // link 检查：实际加载 bridge 类（引用 attach API，JDK9+ 可加载）
                Class<?> bridgeClass = Class.forName("io.miyo.launcher.AttachBridge$Jdk9Bridge");
                return (AttachBridge) bridgeClass.getDeclaredConstructor().newInstance();
            } catch (ClassNotFoundException | LinkageError e) {
                // 继续尝试 JDK 8 桥
            } catch (ReflectiveOperationException e) {
                return null;
            }
            Jdk8Bridge jdk8 = Jdk8Bridge.tryCreate();
            return jdk8;
        }

        private Factory() {
        }
    }

    // ------------------------------------------------------------------
    // JDK 9+ 实现（直接 API）
    // ------------------------------------------------------------------

    final class Jdk9Bridge implements AttachBridge {

        @Override
        public List<VmDescriptor> list() throws AttachBridgeException {
            try {
                List<com.sun.tools.attach.VirtualMachineDescriptor> descriptors =
                        com.sun.tools.attach.VirtualMachine.list();
                List<VmDescriptor> out = new ArrayList<>();
                for (com.sun.tools.attach.VirtualMachineDescriptor d : descriptors) {
                    out.add(new VmDescriptor(parsePidDefault(d.id()), d.displayName()));
                }
                return out;
            } catch (Throwable t) {
                throw wrap("list_failed", "进程枚举失败", t);
            }
        }

        @Override
        public void attachAndLoad(long pid, String agentPath, String agentArgs)
                throws AttachBridgeException {
            com.sun.tools.attach.VirtualMachine vm = attach(pid);
            try {
                vm.loadAgent(agentPath, agentArgs);
            } catch (com.sun.tools.attach.AgentLoadException e) {
                throw wrap("agent_load_failed",
                        "Agent 加载失败（AgentLoadException，通常见目标进程 stderr）", e);
            } catch (com.sun.tools.attach.AgentInitializationException e) {
                throw wrap("agent_init_failed",
                        "Agent 初始化失败（AgentInitializationException，通常见目标进程 stderr；"
                                + "动态 attach 被禁用 / Agent 参数非法等）", e);
            } catch (IOException e) {
                throw wrap("agent_load_failed", "Agent 写入失败：" + e.getMessage(), e);
            } finally {
                detachQuietly(vm);
            }
        }

        @Override
        public java.util.Properties loadSystemProperties(long pid) throws AttachBridgeException {
            com.sun.tools.attach.VirtualMachine vm = attach(pid);
            try {
                return vm.getSystemProperties();
            } catch (IOException e) {
                throw wrap("properties_failed", "系统属性读取失败：" + e.getMessage(), e);
            } finally {
                detachQuietly(vm);
            }
        }

        @Override
        public String describe() {
            return "jdk9+ attach bridge（jdk.attach 模块，运行于 "
                    + System.getProperty("java.version") + "）";
        }

        private com.sun.tools.attach.VirtualMachine attach(long pid) throws AttachBridgeException {
            try {
                return com.sun.tools.attach.VirtualMachine.attach(String.valueOf(pid));
            } catch (com.sun.tools.attach.AttachNotSupportedException e) {
                throw wrap("attach_not_supported",
                        "目标 JVM 不支持 attach（非 HotSpot / DisableAttachMechanism / 不同用户 / 架构不匹配）", e);
            } catch (IOException e) {
                throw wrap(mapIoError(e), "attach 失败：" + e.getMessage(), e);
            }
        }
    }

    // ------------------------------------------------------------------
    // JDK 8 实现（tools.jar 反射桥）
    // ------------------------------------------------------------------

    final class Jdk8Bridge implements AttachBridge {

        private final Class<?> vmClass;
        private final ClassLoader toolsLoader;

        private Jdk8Bridge(Class<?> vmClass, ClassLoader toolsLoader) {
            this.vmClass = vmClass;
            this.toolsLoader = toolsLoader;
        }

        /** 尝试创建（tools.jar 不存在返回 null）。 */
        static Jdk8Bridge tryCreate() {
            File toolsJar = locateToolsJar();
            if (toolsJar == null || !toolsJar.isFile()) {
                return null;
            }
            try {
                URLClassLoader loader = new URLClassLoader(
                        new URL[] {toolsJar.toURI().toURL()},
                        Jdk8Bridge.class.getClassLoader());
                Class<?> vmClass = Class.forName("com.sun.tools.attach.VirtualMachine", false, loader);
                return new Jdk8Bridge(vmClass, loader);
            } catch (Throwable t) {
                return null;
            }
        }

        static File locateToolsJar() {
            String home = System.getProperty("java.home");
            if (home == null) {
                return null;
            }
            // macOS JDK8: .../Contents/Home/jre/../lib/tools.jar；Linux: <home>/lib/tools.jar
            File[] candidates = {
                    new File(home, "lib/tools.jar"),
                    new File(home, ".." + File.separator + "lib/tools.jar"),
            };
            for (File candidate : candidates) {
                if (candidate.isFile()) {
                    try {
                        return candidate.getCanonicalFile();
                    } catch (IOException e) {
                        return candidate;
                    }
                }
            }
            return null;
        }

        @Override
        public List<VmDescriptor> list() throws AttachBridgeException {
            try {
                Class<?> descClass = Class.forName(
                        "com.sun.tools.attach.VirtualMachineDescriptor", true, toolsLoader);
                Method list = vmClass.getMethod("list");
                List<?> descriptors = (List<?>) list.invoke(null);
                List<VmDescriptor> out = new ArrayList<>();
                Method id = descClass.getMethod("id");
                Method displayName = descClass.getMethod("displayName");
                for (Object d : descriptors) {
                    out.add(new VmDescriptor(parsePidDefault((String) id.invoke(d)),
                            (String) displayName.invoke(d)));
                }
                return out;
            } catch (Exception e) {
                throw wrap("list_failed", "进程枚举失败：" + e.getMessage(), e);
            }
        }

        @Override
        public void attachAndLoad(long pid, String agentPath, String agentArgs)
                throws AttachBridgeException {
            Object vm = attach(pid);
            try {
                Method loadAgent = vmClass.getMethod("loadAgent", String.class, String.class);
                loadAgent.invoke(vm, agentPath, agentArgs);
            } catch (java.lang.reflect.InvocationTargetException e) {
                Throwable cause = e.getCause() != null ? e.getCause() : e;
                String causeClass = cause.getClass().getName();
                if (causeClass.endsWith("AgentLoadException")) {
                    throw wrap("agent_load_failed", "Agent 加载失败（AgentLoadException）", cause);
                }
                if (causeClass.endsWith("AgentInitializationException")) {
                    throw wrap("agent_init_failed",
                            "Agent 初始化失败（AgentInitializationException，通常见目标进程 stderr）", cause);
                }
                if (causeClass.endsWith("AttachNotSupportedException")) {
                    throw wrap("attach_not_supported", "目标 JVM 不支持 attach", cause);
                }
                throw wrap("io_error", "attach / loadAgent 失败：" + cause.getMessage(), cause);
            } catch (Exception e) {
                throw wrap("io_error", "attach / loadAgent 失败：" + e.getMessage(), e);
            } finally {
                detachQuietly(vm);
            }
        }

        @Override
        public java.util.Properties loadSystemProperties(long pid) throws AttachBridgeException {
            Object vm = attach(pid);
            try {
                Method getProps = vmClass.getMethod("getSystemProperties");
                return (java.util.Properties) getProps.invoke(vm);
            } catch (Exception e) {
                throw wrap("properties_failed", "系统属性读取失败：" + e.getMessage(), e);
            } finally {
                detachQuietly(vm);
            }
        }

        @Override
        public String describe() {
            return "jdk8 tools.jar 反射桥（tools.jar="
                    + (locateToolsJar() == null ? "缺失" : locateToolsJar().getPath())
                    + "，运行于 " + System.getProperty("java.version") + "）";
        }

        private Object attach(long pid) throws AttachBridgeException {
            try {
                Method attach = vmClass.getMethod("attach", String.class);
                return attach.invoke(null, String.valueOf(pid));
            } catch (java.lang.reflect.InvocationTargetException e) {
                Throwable cause = e.getCause() != null ? e.getCause() : e;
                String causeClass = cause.getClass().getName();
                if (causeClass.endsWith("AttachNotSupportedException")) {
                    throw wrap("attach_not_supported",
                            "目标 JVM 不支持 attach（非 HotSpot / DisableAttachMechanism / 不同用户）", cause);
                }
                throw wrap(mapIoError(cause), "attach 失败：" + cause.getMessage(), cause);
            } catch (Exception e) {
                throw wrap(mapIoError(e), "attach 失败：" + e.getMessage(), e);
            }
        }

        private void detachQuietly(Object vm) {
            try {
                Method detach = vmClass.getMethod("detach");
                detach.invoke(vm);
            } catch (Throwable ignored) {
                // detach 失败不影响结果（agent 已加载即达成目的）
            }
        }
    }

    // ------------------------------------------------------------------
    // 共享助手
    // ------------------------------------------------------------------

    static void detachQuietly(Object vm) {
        try {
            Method detach = vm.getClass().getMethod("detach");
            detach.invoke(vm);
        } catch (Throwable ignored) {
            // detach 失败不影响结果
        }
    }

    static long parsePidDefault(String id) throws AttachBridgeException {
        try {
            return Long.parseLong(id.trim());
        } catch (NumberFormatException e) {
            throw new AttachBridgeException("bad_pid", "非法 pid：" + id);
        }
    }

    static String mapIoError(Throwable e) {
        String message = e.getMessage() == null ? "" : e.getMessage().toLowerCase();
        if (message.contains("no such process")) {
            return "process_not_found";
        }
        if (message.contains("permission") || message.contains("denied")
                || message.contains("operation not permitted")) {
            return "permission_denied";
        }
        if (message.contains("no such file")) {
            return "process_not_found";
        }
        return "io_error";
    }

    static AttachBridgeException wrap(String code, String message, Throwable cause) {
        // IOException 细分（进程消失 / 权限）优先于调用方给的粗码
        if (cause != null) {
            String mapped = mapIoError(cause);
            if (!"io_error".equals(mapped) && "io_error".equals(code)) {
                return new AttachBridgeException(mapped, message, cause);
            }
        }
        return new AttachBridgeException(code, message, cause);
    }
}
