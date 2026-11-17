package io.miyo.agent.invoke;

import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 实例解析：方法调用的目标实例从哪来。
 *
 * 零侵入模型下没有「显式注册的实例供给器」，可选来源：
 * 1. static：无需实例；
 * 2. bean：Spring 容器按 Bean 名 / 类型取单例（Spring 可用时）；
 * 3. new：无参构造临时实例（会执行构造器——副作用风险，必须用户确认）。
 *
 * 不可解析的诚实报错：没有实例来源时说明原因，不造假实例。
 */
public final class InstanceResolver {

    /** 解析请求（IPC invoke.params.instance）。 */
    public static final String KIND_STATIC = "static";
    public static final String KIND_BEAN = "bean";
    public static final String KIND_NEW = "new";

    /** 解析结果：实例 + 描述（instance 非 null 时描述其实际类）。 */
    public static final class Resolved {
        public final Object instance;
        public final String kind;
        public final String actualType;
        /** 实例来自 Bean 容器时的 Bean 名（审计 / 展示）。 */
        public final String beanName;
        /** 实例是否代理（JDK proxy / CGLIB）。 */
        public final boolean proxied;

        Resolved(Object instance, String kind, String beanName) {
            this.instance = instance;
            this.kind = kind;
            this.beanName = beanName;
            this.actualType = instance == null ? null : instance.getClass().getName();
            this.proxied = instance != null && isProxy(instance.getClass());
        }
    }

    private final io.miyo.agent.spring.ContextDiscovery discovery;
    private final boolean allowNewInstanceWithoutConsent;

    public InstanceResolver(io.miyo.agent.spring.ContextDiscovery discovery,
                            boolean allowNewInstanceWithoutConsent) {
        this.discovery = discovery;
        this.allowNewInstanceWithoutConsent = allowNewInstanceWithoutConsent;
    }

    /**
     * 解析实例。consent = 用户已在 UI 确认「新建实例」副作用。
     * static → null instance；失败抛 IpcException（含诚实原因）。
     */
    public Resolved resolve(String kind, Class<?> targetClass, String beanName, boolean consent)
            throws Exception {
        if (KIND_STATIC.equals(kind)) {
            return new Resolved(null, KIND_STATIC, null);
        }
        if (KIND_BEAN.equals(kind)) {
            return resolveBean(targetClass, beanName);
        }
        if (KIND_NEW.equals(kind)) {
            if (!consent && !allowNewInstanceWithoutConsent) {
                throw new io.miyo.agent.ipc.IpcException(
                        io.miyo.agent.ipc.Protocol.ERR_CONSENT_REQUIRED,
                        "新建临时实例会执行目标类构造器（可能有副作用），需要用户确认（consent=true）");
            }
            return newInstance(targetClass);
        }
        throw new io.miyo.agent.ipc.IpcException(
                io.miyo.agent.ipc.Protocol.ERR_BAD_ARGUMENTS,
                "未知实例解析方式：" + kind + "（static / bean / new）");
    }

    private Resolved resolveBean(Class<?> targetClass, String beanName) throws Exception {
        io.miyo.agent.spring.ContextDiscovery.ContextHandle handle = discovery.activeContext();
        if (handle == null) {
            throw new io.miyo.agent.ipc.IpcException(
                    io.miyo.agent.ipc.Protocol.ERR_UNSUPPORTED,
                    discovery.unavailableReason());
        }
        Object bean = handle.getBean(beanName, targetClass);
        if (bean == null) {
            throw new io.miyo.agent.ipc.IpcException(
                    io.miyo.agent.ipc.Protocol.ERR_NOT_FOUND,
                    "Bean 不存在或无法解析：" + (beanName != null ? beanName : targetClass.getName()));
        }
        return new Resolved(bean, KIND_BEAN, beanName);
    }

    /** 新建临时实例（无参构造；构造器抛错原样上抛为 failed）。 */
    private Resolved newInstance(Class<?> targetClass) throws Exception {
        Constructor<?> ctor;
        try {
            ctor = targetClass.getDeclaredConstructor();
        } catch (NoSuchMethodException e) {
            throw new io.miyo.agent.ipc.IpcException(
                    io.miyo.agent.ipc.Protocol.ERR_UNSUPPORTED,
                    "类没有无参构造，无法新建临时实例：" + targetClass.getName()
                            + "。若该类是 Spring Bean，请改用 Bean 调用。");
        }
        try {
            ctor.setAccessible(true);
            Object instance = ctor.newInstance();
            return new Resolved(instance, KIND_NEW, null);
        } catch (InvocationTargetException e) {
            // 构造器抛异常：保留真实异常（业务失败，不是链路错误）
            throw e;
        }
    }

    /** 代理判定（JDK proxy / CGLIB 类名模式 / cglib 子类包名）。 */
    public static boolean isProxy(Class<?> cls) {
        if (java.lang.reflect.Proxy.isProxyClass(cls)) {
            return true;
        }
        String name = cls.getName();
        return name.contains("$$") || name.startsWith("com.sun.proxy.");
    }

    /** 代理描述（Bean 列表展示）。 */
    public static Map<String, Object> proxyInfo(Class<?> cls) {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("proxy", isProxy(cls));
        if (java.lang.reflect.Proxy.isProxyClass(cls)) {
            out.put("proxy_kind", "jdk_dynamic");
            List<String> ifaces = new ArrayList<>();
            for (Class<?> iface : cls.getInterfaces()) {
                ifaces.add(iface.getName());
            }
            out.put("proxy_interfaces", ifaces);
        } else if (isProxy(cls)) {
            out.put("proxy_kind", "cglib");
            out.put("proxy_super", cls.getSuperclass() != null ? cls.getSuperclass().getName() : null);
        }
        return out;
    }

    /** 是否可以直接实例化（有可用的无参构造；包私有也算——setAccessible 可开）。 */
    public static boolean canInstantiate(Class<?> cls) {
        if (cls.isInterface() || Modifier.isAbstract(cls.getModifiers()) || cls.isEnum()) {
            return false;
        }
        try {
            cls.getDeclaredConstructor();
            return true;
        } catch (NoSuchMethodException e) {
            return false;
        }
    }
}
