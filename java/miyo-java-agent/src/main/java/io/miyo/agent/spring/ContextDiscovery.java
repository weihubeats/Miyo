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

        /**
         * Bean 定义名清单。
         *
         * 解析顺序：本类 public → 实现的接口 → 父类链。只查 `getClass()
         * .getMethod(...)` 在两种真实形态下会拿不到：
         * <ul>
         *   <li>上下文是 CGLIB / JDK 代理（Boot 2.x 常见 `@Configuration` 代理、
         *       `ObjectProvider` 包装），代理类不声明也不继承该方法；</li>
         *   <li>BeanFactory 由内部类实现，`getBeanDefinitionNames` 来自
         *       `BeanDefinitionRegistry` / `ListableBeanFactory` 接口。</li>
         * </ul>
         * 拿不到名字 → Bean 列表为空（真实用户现象：Spring 已就绪但「暂无 Bean」）。
         */
        @SuppressWarnings("unchecked")
        public List<String> beanDefinitionNames() {
            try {
                Object namesObj = invokeResolved(context, "getBeanDefinitionNames");
                if (namesObj instanceof String[]) {
                    String[] names = (String[]) namesObj;
                    return Arrays.asList(names);
                }
                if (namesObj instanceof List) {
                    return new ArrayList<String>((List<String>) namesObj);
                }
                return Collections.emptyList();
            } catch (Exception e) {
                return Collections.emptyList();
            }
        }

        /** Bean 实际实例（含触发惰性创建）；不存在返回 null。
         *
         *  `name == null` 时按**类型**解析（`getBean(Class)`）：UI 的实例策略是
         *  `auto → bean`，并不选具体 Bean 名，直接按名取会用 `getBean(null)` 抛
         *  NPE / BeanDefinitionStoreException（真实用户调用 bean 模式方法报
         *  「Bean 不存在或无法解析」）。
         */
        public Object getBean(String name, Class<?> type) {
            try {
                Object bean;
                if (name == null || name.isEmpty()) {
                    if (type == null) {
                        return null;
                    }
                    bean = invokeResolved(context, "getBean",
                            new Class<?>[]{Class.class}, type);
                } else {
                    try {
                        // 优先 getBean(String, Class)（Spring 2.5+，带类型校验）；
                        // 老实现 / 代理只有 getBean(String) 时降级。这里是**按名字**
                        // 取 Bean，必须传 1 参重载，不能沿用同名的 Class 版。
                        bean = invokeResolved(context, "getBean",
                                new Class<?>[]{String.class, Class.class}, name, type);
                    } catch (NoSuchMethodException e) {
                        bean = invokeResolved(context, "getBean",
                                new Class<?>[]{String.class}, name);
                    }
                }
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
                // 按名/按类型都取不到：如实返回 null，由调用方报错
                if (className.endsWith("NoSuchBeanDefinitionException")
                        || className.endsWith("NoUniqueBeanDefinitionException")
                        || cause instanceof NullPointerException) {
                    return null;
                }
                throw new IpcException(Protocol.ERR_UNSUPPORTED,
                        "Bean 获取失败：" + (cause != null ? cause.getClass().getName() + ": "
                                + cause.getMessage() : e.getMessage()));
            } catch (Exception e) {
                // 同时输出方法解析失败的类型信息，避免只看到「xxx#getBean」而不知
                // 是「找不到重载」还是「取不到 Bean」
                throw new IpcException(Protocol.ERR_UNSUPPORTED,
                        "Bean 获取失败：" + e.getClass().getSimpleName() + ": " + e.getMessage());
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
                Object r = invokeResolved(factory, "containsSingleton",
                        new Class<?>[]{String.class}, name);
                return Boolean.TRUE.equals(r);
            } catch (Exception e) {
                return false;
            }
        }

        /** 按 Bean 名取实例（getBean(String)——已实例化单例安全；惰性 Bean 会触发创建）。 */
        public Object getBeanByName(String name) throws Exception {
            return invokeResolved(context, "getBean", new Class<?>[]{String.class}, name);
        }

        /** BeanFactory（getBeanFactory——ConfigurableApplicationContext 实现）。 */
        public Object beanFactory() throws Exception {
            return invokeResolved(context, "getBeanFactory");
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
            String definitionType = stringOf(definition, "getBeanClassName", null);
            if (definitionType == null || definitionType.isEmpty()) {
                // @Bean 工厂方法声明的 Bean 没有 beanClassName：给出真实返回类型，
                // 前端「暂无 Bean 类型」与分类视图都靠这一项（与分类证据同源）。
                String resolved = resolvableTypeName(definition);
                if (resolved == null || resolved.isEmpty()) {
                    Object factoryForType = null;
                    try {
                        factoryForType = handle.beanFactory();
                    } catch (Exception ignored) {
                        // 非 Configurable 上下文：按类名直解析
                    }
                    resolved = io.miyo.agent.inspect.ClassClassifier.factoryMethodReturnType(
                            definition, stringOf(definition, "getFactoryMethodName", null),
                            factoryForType);
                }
                definitionType = resolved;
            }
            out.put("definition_type", definitionType);
            return out;
        } catch (Exception e) {
            // 单个 Bean 元信息不可读：跳过（不影响整体），诚实标注
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("name", name);
            out.put("error", String.valueOf(e.getMessage()));
            return out;
        }
    }

    /**
     * 稳健的方法解析 + 调用。
     *
     * 回退顺序：本类 public（含继承）→ 全部实现接口 → 父类链逐层
     * `getDeclaredMethod`。为什么需要：Spring 的容器/上下文在 Boot 2.x 里
     * 常以代理或内部实现类形态出现，`getClass().getMethod("getBeanFactory")`
     * 会抛 NoSuchMethodException，表现为「Spring 已就绪但 Bean 列表为空」。
     *
     * 只读调用，不触发实例化。
     */
    private static Object invokeResolved(Object target, String method, Class<?>[] types,
                                         Object... args) throws Exception {
        Method m = resolveMethod(target.getClass(), method, types);
        if (m == null) {
            throw new NoSuchMethodException(target.getClass().getName() + "#" + method);
        }
        try {
            m.setAccessible(true);
        } catch (RuntimeException ignored) {
            // 模块系统限制：public 路径仍可直接调用
        }
        return m.invoke(target, args);
    }

    private static Object invokeResolved(Object target, String method) throws Exception {
        return invokeResolved(target, method, new Class<?>[0]);
    }

    /** 方法解析：本类 → 接口 → 父类链（`types` 为 null 表示无参）。
     *
     *  **同名同参数个数的重载必须按参数类型精确匹配**：反射 `getMethod(name, params)`
     *  本身是带类型的，但父类链上的 `getDeclaredMethod` 与部分容器代理类会因擦除 /
     *  桥接方法返回「名字对的另一个重载」。实证：`getBean(Class)` 曾被解析成
     *  `getBean(String)`（两者都是 1 个参数），按类型取 Bean 直接报
     *  「Bean 获取失败」，UI 表现为「Bean 不存在或无法解析」。
     */
    private static Method resolveMethod(Class<?> type, String name, Class<?>[] types) {
        if (type == null || type == Object.class) {
            return null;
        }
        Class<?>[] params = types == null ? new Class<?>[0] : types;
        // 1) 类自身（含继承的 public）
        try {
            Method m = type.getMethod(name, params);
            if (paramTypesMatch(m, params)) {
                return m;
            }
        } catch (NoSuchMethodException ignored) {
            // 继续找接口 / 父类
        }
        // 2) 实现的接口（BeanFactory / ApplicationContext 等方法常声明在接口上）
        for (Class<?> iface : allInterfaces(type)) {
            try {
                Method m = iface.getMethod(name, params);
                if (paramTypesMatch(m, params)) {
                    return m;
                }
            } catch (NoSuchMethodException ignored) {
                // 下一个接口
            }
        }
        // 3) 父类链的 declared 方法（含非 public 实现）
        for (Class<?> c = type; c != null && c != Object.class; c = c.getSuperclass()) {
            try {
                Method m = c.getDeclaredMethod(name, params);
                if (paramTypesMatch(m, params)) {
                    return m;
                }
            } catch (NoSuchMethodException ignored) {
                // 继续父类
            }
        }
        return null;
    }

    /**
     * 参数表精确校验。
     *
     * `getMethod` 在**普通类**上已按类型匹配，但经过代理 / 桥接方法（CGLIB 子类、
     * 泛型擦除生成的 bridge）后可能返回「同名不同形参」的重载；本校验把这种
     * 情况挡在调用之前，避免 `IllegalArgumentException: argument type mismatch`。
     */
    private static boolean paramTypesMatch(Method m, Class<?>[] expected) {
        Class<?>[] actual = m.getParameterTypes();
        if (actual.length != expected.length) {
            return false;
        }
        for (int i = 0; i < actual.length; i++) {
            if (actual[i] != expected[i]) {
                return false;
            }
        }
        return true;
    }

    /** 类型实现的全部接口（递归展开父接口）。 */
    private static java.util.List<Class<?>> allInterfaces(Class<?> type) {
        java.util.List<Class<?>> out = new ArrayList<Class<?>>();
        java.util.Set<Class<?>> seen = new java.util.HashSet<Class<?>>();
        for (Class<?> c = type; c != null && c != Object.class; c = c.getSuperclass()) {
            collectInterfaces(c, out, seen);
        }
        return out;
    }

    private static void collectInterfaces(Class<?> type, java.util.List<Class<?>> out,
                                          java.util.Set<Class<?>> seen) {
        for (Class<?> iface : type.getInterfaces()) {
            if (seen.add(iface)) {
                out.add(iface);
                collectInterfaces(iface, out, seen);
            }
        }
    }

    private Object beanDefinition(ContextHandle handle, String name) throws Exception {
        Object factory = handle.beanFactory();
        return invokeResolved(factory, "getBeanDefinition",
                new Class<?>[]{String.class}, name);
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
    // 类发现（2.1）：ROLE_APPLICATION Bean 的实现类（**只读定义，不实例化**）
    // ------------------------------------------------------------------

    /**
     * 收集当前上下文中 ROLE_APPLICATION Bean 的实现类名，供类分类器判定
     * 「这个类是不是 Spring Bean」。绝不调用 getBean——不会触发惰性 Bean
     * 创建（类发现是高频只读操作，副作用必须为零）。
     */
    public java.util.Set<String> applicationBeanClasses() {
        return beanEvidence().classes;
    }

    /** Spring 侧收集到的全部分类证据（一次遍历产出，避免重复读 BeanDefinition）。 */
    public static final class BeanEvidence {
        /** ROLE_APPLICATION Bean 的声明类型 / `@Bean` 返回类型。 */
        public final java.util.Set<String> classes = new java.util.LinkedHashSet<String>();
        /** BeanDefinition 无 beanClassName、类型来自 `@Bean` 工厂方法返回值的类。 */
        public final java.util.Set<String> types = new java.util.LinkedHashSet<String>();
        /** 实现类全名 → 同源键（`fm:包根#factoryMethod` / `root:包根`）。 */
        public final java.util.Map<String, String> factories =
                new java.util.LinkedHashMap<String, String>();

        /** 转成 inspect 包的中立视图（避免 inspect → spring 的编译期依赖）。 */
        public io.miyo.agent.inspect.ClassInspector.SpringBeanEvidence toInspectorEvidence() {
            return new io.miyo.agent.inspect.ClassInspector.SpringBeanEvidence(types, factories);
        }
    }

    /** 收集 Spring 分类证据（只读 BeanDefinition，不实例化 Bean）。 */
    public BeanEvidence beanEvidence() {
        BeanEvidence evidence = new BeanEvidence();
        for (Object ctx : enumerateContexts()) {
            Object factory = null;
            try {
                // 必须走 invokeResolved：Boot 2.x 的上下文常是代理/内部实现类，
                // `getClass().getMethod("getBeanFactory")` 拿不到 → factory 为 null
                // → 整轮 Bean 证据为空（真实用户现象：Spring Beans 组恒为 0）。
                factory = invokeResolved(ctx, "getBeanFactory");
            } catch (Exception ignored) {
                // 上下文无 getBeanFactory（非 Configurable）：跳过
            }
            if (factory == null) {
                continue;
            }
            io.miyo.agent.inspect.ClassClassifier.collectBeanEvidence(
                    factory, 4000, evidence.classes, evidence.types, evidence.factories);
        }
        return evidence;
    }

    /**
     * 类名 → Bean 名映射（UI 在类列表上展示「属于哪个 Bean」）。
     * 只读 BeanDefinition 的类型信息，不实例化。
     */
    public java.util.Map<String, String> beanNamesByClass() {
        java.util.Map<String, String> out = new java.util.LinkedHashMap<String, String>();
        for (Object ctx : enumerateContexts()) {
            ContextHandle handle = new ContextHandle(ctx, contextId(ctx));
            for (String name : handle.beanDefinitionNames()) {
                try {
                    Object definition = beanDefinition(handle, name);
                    if (definition == null || roleOf(definition) != 0) {
                        continue;
                    }
                    String type = stringOf(definition, "getBeanClassName", null);
                    if (type == null || type.isEmpty()) {
                        type = resolvableTypeName(definition);
                    }
                    if (type == null || type.isEmpty()) {
                        // 与 collectBeanEvidence 同一条降级路径：Boot 2.x 的 @Bean
                        // 定义只有工厂方法名，按方法签名读返回类型（需 BeanFactory
                        // 把 factoryBeanName 解析成工厂类）
                        Object factory = null;
                        try {
                            factory = invokeResolved(ctx, "getBeanFactory");
                        } catch (Exception ignored) {
                            // 非 Configurable 上下文
                        }
                        type = io.miyo.agent.inspect.ClassClassifier.factoryMethodReturnType(
                                definition, stringOf(definition, "getFactoryMethodName", null),
                                factory);
                    }
                    if (type != null && !type.isEmpty() && !out.containsKey(type)) {
                        out.put(type, name);
                    }
                } catch (Exception ignored) {
                    // 单个定义不可读：跳过
                }
            }
        }
        return out;
    }

    /** BeanDefinition.getResolvableType().resolve()（@Bean 工厂方法返回类型）。 */
    private static String resolvableTypeName(Object definition) {
        try {
            Object type = invokeResolved(definition, "getResolvableType");
            if (type == null) {
                return null;
            }
            // getTypeName() 保留参数化信息（@Bean 返回 List<Foo> 时归一键才准确），
            // resolve() 是擦除后的裸类型，作兜底。
            Object typeName = invokeResolved(type, "getTypeName");
            String name = typeName == null ? null : String.valueOf(typeName);
            if (name != null && !name.isEmpty() && name.indexOf('<') > 0) {
                return name;
            }
            Object cls = invokeResolved(type, "resolve");
            if (cls instanceof Class) {
                return ((Class<?>) cls).getName();
            }
            return name;
        } catch (Exception e) {
            return null;
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

    /**
     * Spring 是否在场（ApplicationContext 类已加载）。
     *
     * 只缓存**正**结果：agent 是运行期注入，索引可能尚未建立、Spring 也可能在
     * attach 之后才加载；负结果缓存会让「attach 早期一次探测」永久关闭 Bean 分类
     * （端到端实测：spring_bean 恒为 0，但 beans.list 有 144 个 Bean）。
     */
    public boolean springPresent() {
        if (Boolean.TRUE.equals(springPresentCache)) {
            return true;
        }
        if (findSpringClass("org.springframework.context.ApplicationContext") == null) {
            return false;
        }
        springPresentCache = Boolean.TRUE;
        return true;
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
