package io.miyo.agent.spring;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.Comparator;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArrayList;

import io.miyo.agent.ipc.Frames;
import io.miyo.agent.ipc.IpcException;
import io.miyo.agent.ipc.Protocol;

/**
 * Spring 上下文发现（策略梯子，全部纯反射，不修改目标应用行为）。
 *
 * 策略与支持级别（诚实分级，UI 按此展示）：
 * 1. capture（字节码插桩捕获）：agent attach 后 AbstractApplicationContext 的
 *    publishEvent / getBean 被首次调用时捕获——「条件支持」（应用有活动即命中）；
 * 2. live_beans（LiveBeansView 静态表）：仅当启动时设置了
 *    spring.liveBeansView.mbeanDomain 系统属性——「条件支持」；
 * 3. context_loader（ContextLoader.getCurrentWebApplicationContext）：
 *    经典 Spring MVC（非 Boot）——「条件支持」。
 *
 * 找不到上下文时返回明确的 unavailableReason（UI 展示「暂不支持 + 原因」），
 * 不伪造任何 Bean 数据。
 */
public final class ContextDiscovery {

    /** 纯捕获策略上下文容器（弱引用，避免阻止 GC）。 */
    private final CopyOnWriteArrayList<java.lang.ref.WeakReference<Object>> knownContexts =
            new CopyOnWriteArrayList<>();

    private final List<String> notes = new CopyOnWriteArrayList<>();

    /**
     * 已加载类视角（Instrumentation）：Boot fat-jar 的 Spring 类加载在
     * LaunchedURLClassLoader 里，agent 线程的 context / system classloader
     * 看不到——类探测一律走「已加载类」而不是 Class.forName。
     */
    private final ClassInspectorLike inspector;

    public ContextDiscovery(ClassInspectorLike inspector) {
        this.inspector = inspector;
    }

    /** 上下文句柄：包装 context 对象 + 反射操作。 */
    public final class ContextHandle {
        private final Object context;
        private final String id;

        ContextHandle(Object context, String id) {
            this.context = context;
            this.id = id;
        }

        public String id() {
            return id;
        }

        public Object context() {
            return context;
        }

        /** Bean 定义名清单（ApplicationContext.getBeanDefinitionNames）。 */
        @SuppressWarnings("unchecked")
        public List<String> beanDefinitionNames() {
            try {
                Method m = context.getClass().getMethod("getBeanDefinitionNames");
                String[] names = (String[]) m.invoke(context);
                return Arrays.asList(names == null ? new String[0] : names);
            } catch (Exception e) {
                return Collections.emptyList();
            }
        }

        /** Bean 实际实例（含触发惰性创建）；不存在返回 null。 */
        public Object getBean(String name, Class<?> type) {
            try {
                Method m = context.getClass().getMethod("getBean", String.class);
                Object bean = m.invoke(context, name);
                if (bean == null) {
                    return null;
                }
                if (type != null && !type.isInstance(bean)) {
                    throw new IpcException(Protocol.ERR_BAD_ARGUMENTS,
                            "Bean「" + name + "」的实际类型 " + bean.getClass().getName()
                                    + " 与请求的方法类 " + type.getName() + " 不一致（可能是接口代理）");
                }
                return bean;
            } catch (IpcException e) {
                throw e;
            } catch (java.lang.reflect.InvocationTargetException e) {
                Throwable cause = e.getCause();
                String className = cause == null ? "" : cause.getClass().getName();
                if (className.endsWith("NoSuchBeanDefinitionException")) {
                    return null;
                }
                throw new IpcException(Protocol.ERR_UNSUPPORTED,
                        "Bean 获取失败：" + (cause != null ? cause.getClass().getName() + ": "
                                + cause.getMessage() : e.getMessage()));
            } catch (Exception e) {
                throw new IpcException(Protocol.ERR_UNSUPPORTED, "Bean 获取失败：" + e.getMessage());
            }
        }

        /**
         * 单例是否已实例化。containsSingleton 在 SingletonBeanRegistry 上——
         * ApplicationContext 接口链不含它，须经 getBeanFactory()（Configurable-
         * ListableBeanFactory）转发。
         */
        public boolean containsSingleton(String name) {
            try {
                Object factory = beanFactory();
                Method m = factory.getClass().getMethod("containsSingleton", String.class);
                Object r = m.invoke(factory, name);
                return Boolean.TRUE.equals(r);
            } catch (Exception e) {
                return false;
            }
        }

        /** 按 Bean 名取实例（getBean(String)——已实例化单例安全；惰性 Bean 会触发创建）。 */
        public Object getBeanByName(String name) throws Exception {
            Method m = context.getClass().getMethod("getBean", String.class);
            return m.invoke(context, name);
        }

        /** BeanFactory（getBeanFactory——ConfigurableApplicationContext 实现）。 */
        public Object beanFactory() throws Exception {
            Method m = context.getClass().getMethod("getBeanFactory");
            return m.invoke(context);
        }
    }

    // ------------------------------------------------------------------
    // 状态查询（beans.contexts op）
    // ------------------------------------------------------------------

    /** Spring 支持状态总览。 */
    public Map<String, Object> status() {
        Map<String, Object> out = new LinkedHashMap<>();
        boolean present = springPresent();
        out.put("spring_present", present);
        if (!present) {
            out.put("framework_version", null);
            out.put("boot_version", null);
            out.put("contexts", new ArrayList<>());
            out.put("capture_state", "disabled");
            out.put("note", "目标 JVM 未加载 Spring 类（spring_present=false 不代表禁用，"
                    + "懒加载场景可先触发应用刷新后重试）");
            return out;
        }
        out.put("framework_version", frameworkVersion());
        out.put("boot_version", bootVersion());
        List<Map<String, Object>> contexts = new ArrayList<>();
        for (Object ctx : enumerateContexts()) {
            contexts.add(describeContext(ctx));
        }
        out.put("contexts", contexts);
        out.put("capture_state", SpringCapture.state());
        out.put("capture_count", SpringCapture.captureCount());
        if (contexts.isEmpty()) {
            out.put("note", unavailableReason());
        } else if (!notes.isEmpty()) {
            out.put("note", String.join("；", notes));
        }
        return out;
    }

    /** 当前可用上下文（InstanceResolver 用；无则 null）。 */
    public ContextHandle activeContext() {
        List<Object> contexts = enumerateContexts();
        if (contexts.isEmpty()) {
            return null;
        }
        Object best = contexts.get(0); // enumerateContexts 已按优选排序
        return new ContextHandle(best, contextId(best));
    }

    /** 找不到上下文时的诚实原因（UI 展示）。 */
    public String unavailableReason() {
        if (!springPresent()) {
            return "目标 JVM 未加载 Spring 类，Bean 探测不可用";
        }
        if ("waiting".equals(SpringCapture.state())) {
            return "已注册上下文捕获插桩，等待目标应用下一次访问 Spring 上下文"
                    + "（发一次 HTTP 请求 / 触发定时任务即可）；或应用启动时设置"
                    + " -Dspring.liveBeansView.mbeanDomain=miyo 可改用静态注册通道";
        }
        return "未能通过任何策略找到 ApplicationContext："
                + "应用可能未启动完成、上下文已关闭，或为不受支持的上下文形态。"
                + "可尝试向应用发送一次请求后重试";
    }

    // ------------------------------------------------------------------
    // Bean 清单（beans.list op）
    // ------------------------------------------------------------------

    /** Bean 清单（默认只列应用级 BeanDefinition；不触发惰性 Bean 实例化）。 */
    public Map<String, Object> listBeans(String search, int limit, String contextId) {
        List<Object> contexts = enumerateContexts();
        if (contexts.isEmpty()) {
            throw new IpcException(Protocol.ERR_UNSUPPORTED, unavailableReason());
        }
        Object chosen = pickContext(contexts, contextId);
        ContextHandle handle = new ContextHandle(chosen, contextId(chosen));
        List<Map<String, Object>> items = new ArrayList<>();
        int total = 0;
        for (String name : handle.beanDefinitionNames()) {
            Map<String, Object> info = describeBean(handle, name);
            if (info == null) {
                continue; // 非应用级（基础设施）Bean：默认隐藏
            }
            if (!searchMatches(name, String.valueOf(info.getOrDefault("type", "")), search)) {
                continue;
            }
            total++;
            if (items.size() < limit) {
                items.add(info);
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("context_id", handle.id());
        out.put("context_class", chosen.getClass().getName());
        out.put("total", total);
        out.put("returned", items.size());
        out.put("items", items);
        return out;
    }

    /**
     * Bean 描述。非应用级 Bean 返回 null（过滤）。
     * 关键安全边界：绝不为了取类型而实例化 Bean——已实例化单例经
     * containsSingleton 安全取回实际类型；惰性 Bean 只报定义信息并标注。
     */
    private Map<String, Object> describeBean(ContextHandle handle, String name) {
        try {
            Object definition = beanDefinition(handle, name);
            int role = roleOf(definition);
            if (role != 0) { // BeanDefinition.ROLE_APPLICATION = 0
                return null;
            }
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("name", name);
            out.put("scope", stringOf(definition, "getScope", "singleton"));
            out.put("lazy", boolOf(definition, "isLazyInit", false));
            out.put("primary", boolOf(definition, "isPrimary", false));
            out.put("bean_class", stringOf(definition, "getBeanClassName", null));
            boolean instantiated = handle.containsSingleton(name);
            out.put("instantiated", instantiated);
            String actualType = null;
            Object proxyInfo = null;
            if (instantiated) {
                // 已实例化单例：getBean 返回现成实例（不触发新创建）
                Object bean = handle.getBeanByName(name);
                if (bean != null) {
                    Class<?> cls = bean.getClass();
                    actualType = cls.getName();
                    out.putAll(io.miyo.agent.invoke.InstanceResolver.proxyInfo(cls));
                }
            }
            out.put("actual_type", actualType);
            out.put("definition_type", stringOf(definition, "getBeanClassName", null));
            return out;
        } catch (Exception e) {
            // 单个 Bean 元信息不可读：跳过（不影响整体），诚实标注
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("name", name);
            out.put("error", String.valueOf(e.getMessage()));
            return out;
        }
    }

    private Object beanDefinition(ContextHandle handle, String name) throws Exception {
        Object factory = handle.beanFactory();
        Method m = factory.getClass().getMethod("getBeanDefinition", String.class);
        return m.invoke(factory, name);
    }

    private static int roleOf(Object definition) {
        try {
            Method m = definition.getClass().getMethod("getRole");
            Object r = m.invoke(definition);
            return r instanceof Number ? ((Number) r).intValue() : 0;
        } catch (Exception e) {
            return 0;
        }
    }

    private static String stringOf(Object target, String getter, String fallback) {
        try {
            Method m = target.getClass().getMethod(getter);
            Object r = m.invoke(target);
            return r == null ? fallback : String.valueOf(r);
        } catch (Exception e) {
            return fallback;
        }
    }

    private static boolean boolOf(Object target, String getter, boolean fallback) {
        try {
            Method m = target.getClass().getMethod(getter);
            Object r = m.invoke(target);
            return r instanceof Boolean ? (Boolean) r : fallback;
        } catch (Exception e) {
            return fallback;
        }
    }

    // ------------------------------------------------------------------
    // 策略实现
    // ------------------------------------------------------------------

    /** 枚举所有已知上下文（去重 + 活着优先 + 排序）。 */
    private List<Object> enumerateContexts() {
        Set<Object> seen = Collections.newSetFromMap(new IdentityHashMap<>());
        List<Object> out = new ArrayList<>();
        // 策略 1：捕获
        for (Object ctx : SpringCapture.snapshot()) {
            if (seen.add(ctx)) {
                out.add(ctx);
            }
        }
        // 策略 2：LiveBeansView 静态表（属性未设置时集合为空——读它无副作用）
        for (Object ctx : liveBeansViewContexts()) {
            if (seen.add(ctx)) {
                out.add(ctx);
            }
        }
        // 策略 3：ContextLoader（经典 Web 应用）
        Object webCtx = contextLoaderContext();
        if (webCtx != null && seen.add(webCtx)) {
            out.add(webCtx);
        }
        // 排序：非关闭在前，启动时间新在前
        out.sort(Comparator
                .comparingInt((Object c) -> isActive(c) ? 0 : 1)
                .thenComparing((Object c) -> -startupDate(c)));
        // 记忆到已知表（弱引用）
        for (Object ctx : out) {
            boolean present = false;
            for (java.lang.ref.WeakReference<Object> ref : knownContexts) {
                if (ref.get() == ctx) {
                    present = true;
                    break;
                }
            }
            if (!present) {
                knownContexts.add(new java.lang.ref.WeakReference<>(ctx));
            }
        }
        return out;
    }

    /** LiveBeansView 静态表（Spring 3.3+；仅当系统属性设置过才有内容）。 */
    private List<Object> liveBeansViewContexts() {
        try {
            Class<?> cls = Class.forName(
                    "org.springframework.context.support.LiveBeansView", false,
                    springClassLoader());
            Field field = cls.getDeclaredField("applicationContexts");
            field.setAccessible(true);
            Object value = field.get(null);
            if (!(value instanceof Collection)) {
                return Collections.emptyList();
            }
            List<Object> out = new ArrayList<>();
            for (Object item : (Collection<?>) value) {
                out.add(item);
            }
            if (!out.isEmpty()) {
                addNote("live_beans");
            }
            return out;
        } catch (ClassNotFoundException e) {
            return Collections.emptyList(); // Spring 版本无该类
        } catch (Exception e) {
            return Collections.emptyList();
        }
    }

    /** ContextLoader 静态 Web 上下文（经典 Spring MVC）。 */
    private Object contextLoaderContext() {
        try {
            Class<?> cls = Class.forName(
                    "org.springframework.web.context.ContextLoader", false,
                    springClassLoader());
            Method m = cls.getMethod("getCurrentWebApplicationContext");
            Object ctx = m.invoke(null);
            if (ctx != null) {
                addNote("context_loader");
            }
            return ctx;
        } catch (ClassNotFoundException e) {
            return null;
        } catch (Exception e) {
            return null;
        }
    }

    private synchronized void addNote(String strategy) {
        if (!notes.contains(strategy)) {
            notes.add(strategy);
        }
    }

    // ------------------------------------------------------------------
    // 反射助手
    // ------------------------------------------------------------------

    private static boolean isActive(Object ctx) {
        try {
            Method m = ctx.getClass().getMethod("isActive");
            Object r = m.invoke(ctx);
            return Boolean.TRUE.equals(r);
        } catch (Exception e) {
            return true; // 无法判定时按活处理（不误判为关闭）
        }
    }

    private static long startupDate(Object ctx) {
        try {
            Method m = ctx.getClass().getMethod("getStartupDate");
            Object r = m.invoke(ctx);
            return r instanceof Number ? ((Number) r).longValue() : 0;
        } catch (Exception e) {
            return 0;
        }
    }

    private static String contextId(Object ctx) {
        try {
            Method m = ctx.getClass().getMethod("getId");
            Object r = m.invoke(ctx);
            if (r != null && !String.valueOf(r).isEmpty()) {
                return String.valueOf(r);
            }
        } catch (Exception ignored) {
            // 用类名兜底
        }
        return ctx.getClass().getSimpleName() + "@" + Integer.toHexString(
                System.identityHashCode(ctx));
    }

    private Map<String, Object> describeContext(Object ctx) {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("id", contextId(ctx));
        out.put("class", ctx.getClass().getName());
        out.put("active", isActive(ctx));
        out.put("startup_date", startupDate(ctx));
        out.put("parent", parentOf(ctx));
        out.put("bean_count", beanCount(ctx));
        return out;
    }

    private static String parentOf(Object ctx) {
        try {
            Method m = ctx.getClass().getMethod("getParent");
            Object parent = m.invoke(ctx);
            return parent == null ? null : parent.getClass().getName();
        } catch (Exception e) {
            return null;
        }
    }

    private static int beanCount(Object ctx) {
        try {
            Method m = ctx.getClass().getMethod("getBeanDefinitionCount");
            Object r = m.invoke(ctx);
            return r instanceof Number ? ((Number) r).intValue() : -1;
        } catch (Exception e) {
            return -1;
        }
    }

    private Object pickContext(List<Object> contexts, String contextId) {
        if (contextId == null || contextId.trim().isEmpty()) {
            return contexts.get(0);
        }
        for (Object ctx : contexts) {
            if (contextId(ctx).equals(contextId)) {
                return ctx;
            }
        }
        throw new IpcException(Protocol.ERR_NOT_FOUND,
                "指定上下文不存在或已不可见：" + contextId);
    }

    private static boolean searchMatches(String name, String type, String search) {
        if (search == null || search.trim().isEmpty()) {
            return true;
        }
        String term = search.trim().toLowerCase(java.util.Locale.ROOT);
        return name.toLowerCase(java.util.Locale.ROOT).contains(term)
                || (type != null && type.toLowerCase(java.util.Locale.ROOT).contains(term));
    }

    // ------------------------------------------------------------------
    // Spring 存在性 / 版本
    // ------------------------------------------------------------------

    private Boolean springPresentCache;

    /** Spring 是否在场（ApplicationContext 类已加载；缓存结果）。 */
    public boolean springPresent() {
        Boolean cached = springPresentCache;
        if (cached != null) {
            return cached;
        }
        boolean present = findSpringClass("org.springframework.context.ApplicationContext") != null;
        springPresentCache = present;
        return present;
    }

    /** 已加载类探测（Instrumentation 视角；不触发加载，找不到返回 null）。 */
    private Class<?> findSpringClass(String name) {
        return inspector.findLoadedClassOrNull(name);
    }

    /**
     * Spring 类的 classloader（fat-jar 嵌套加载器）：以 AbstractApplicationContext
     * 的实际加载器为准；找不到时退回 context classloader。
     */
    private ClassLoader springClassLoader() {
        Class<?> anchor = findSpringClass(
                "org.springframework.context.support.AbstractApplicationContext");
        if (anchor != null && anchor.getClassLoader() != null) {
            return anchor.getClassLoader();
        }
        ClassLoader cl = Thread.currentThread().getContextClassLoader();
        return cl != null ? cl : ContextDiscovery.class.getClassLoader();
    }

    /** Spring Framework 版本（SpringVersion 静态方法；缺省 unknown）。 */
    public String frameworkVersion() {
        try {
            Class<?> cls = Class.forName("org.springframework.core.SpringVersion", false,
                    springClassLoader());
            Method m = cls.getMethod("getVersion");
            Object v = m.invoke(null);
            return v != null && !String.valueOf(v).isEmpty()
                    ? String.valueOf(v) : "unknown";
        } catch (Exception e) {
            return "unknown";
        }
    }

    /** Spring Boot 版本（SpringBootVersion.VERSION 静态字段；缺省 unknown）。 */
    public String bootVersion() {
        try {
            Class<?> cls = Class.forName("org.springframework.boot.SpringBootVersion", false,
                    springClassLoader());
            Field f = cls.getField("VERSION");
            Object v = f.get(null);
            return v != null && !String.valueOf(v).isEmpty()
                    ? String.valueOf(v) : "unknown";
        } catch (Exception e) {
            return "unknown";
        }
    }

    /** 注册捕获插桩（attach 后异步执行；失败降级为 disabled 并保留原因）。 */
    public void installCapture(java.lang.instrument.Instrumentation inst,
                               ClassInspectorLike inspector) {
        if (!springPresent()) {
            SpringCapture.setState("disabled");
            return;
        }
        if (!inst.isRetransformClassesSupported()) {
            SpringCapture.setState("unsupported");
            notes.add("目标 JVM 不支持 retransform（isRetransformClassesSupported=false）");
            return;
        }
        try {
            Class<?> target = inspector.findLoadedClassOrNull(
                    "org.springframework.context.support.AbstractApplicationContext");
            if (target == null) {
                SpringCapture.setState("not_loaded");
                notes.add("AbstractApplicationContext 尚未加载（Spring 未完成启动？）");
                return;
            }
            inst.addTransformer(new ContextTransformer(), true);
            inst.retransformClasses(target);
            SpringCapture.setState("waiting");
        } catch (Throwable t) {
            SpringCapture.setState("failed");
            notes.add("上下文捕获插桩失败：" + t.getClass().getName()
                    + (t.getMessage() != null ? ": " + t.getMessage() : ""));
        }
    }

    /** 解耦接口：避免 spring 包依赖 inspect 包的完整实现。 */
    public interface ClassInspectorLike {
        Class<?> findLoadedClassOrNull(String name);
    }
}
