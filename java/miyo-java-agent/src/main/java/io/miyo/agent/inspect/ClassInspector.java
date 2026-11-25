package io.miyo.agent.inspect;

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

import io.miyo.agent.invoke.RiskPolicy;

/**
 * JVM Inspector：已加载类枚举 + 类 / 方法元信息收集。
 *
 * 边界（诚实声明）：Instrumentation.getAllLoadedClasses() 只能看到「已经加载」
 * 的类——懒加载类在首次使用前不可见；且这里发现的是类而非实例（实例解析
 * 策略见 invoke.InstanceResolver）。
 *
 * 方法枚举用 getDeclaredMethods（不含继承）+ 父类 public 方法补充；
 * 重载以参数擦除表区分；synthetic / bridge 标记透传（默认过滤）。
 *
 * <h3>类发现（2.1 重构）</h3>
 * 列表走 {@link ClassIndex}（元数据缓存 + 增量刷新），按
 * {@link ClassClassifier} 分类后过滤 / 排序 / 分页——不再每次刷新遍历 +
 * 全量反射，也不再依赖包名黑名单判断「是不是业务类」。
 */
public final class ClassInspector
        implements io.miyo.agent.spring.ContextDiscovery.ClassInspectorLike {

    /** 默认展示的类别（其余类别需要显式 include_dependencies / include_jdk）。 */
    private static final Set<ClassClassifier.Category> DEFAULT_CATEGORIES =
            new LinkedHashSet<ClassClassifier.Category>(Arrays.asList(
                    ClassClassifier.Category.SPRING_BEAN,
                    ClassClassifier.Category.APPLICATION));

    private final Instrumentation instrumentation;
    private final ClassIndex index;

    /** Spring 侧上下文提供者（agent 装配时注入；缺失时降级为 codeSource 判定）。 */
    private volatile SpringBeanSource springBeanSource;

    public ClassInspector(Instrumentation instrumentation) {
        this(instrumentation, new ClassIndex(instrumentation));
    }

    public ClassInspector(Instrumentation instrumentation, ClassIndex index) {
        this.instrumentation = instrumentation;
        this.index = index;
    }

    /** 注入 Spring Bean 提供者（AppMain 装配：ContextDiscovery::applicationBeanClasses）。 */
    public void setSpringBeanSource(SpringBeanSource source) {
        this.springBeanSource = source;
    }

    public ClassIndex index() {
        return index;
    }

    /** Spring Bean 角色来源（避免 inspect → spring 包的编译期依赖方向反转）。 */
    public interface SpringBeanSource {
        /** ROLE_APPLICATION Bean 实现类名（可能为空）。 */
        Set<String> applicationBeanClasses();

        /**
         * Spring 侧全部分类证据（`@Bean` 返回类型 + 同源键）。默认空集：老实现
         * 只提供类名也能工作，只是 `@Bean` 声明与实现类归不到 Bean 组。
         */
        default SpringBeanEvidence springBeanEvidence() {
            return SpringBeanEvidence.EMPTY;
        }

        /** 类名 → Bean 名（展示用；可空）。 */
        Map<String, String> beanNamesByClass();

        /** 是否探测到 Spring。 */
        boolean springPresent();
    }

    /**
     * Spring 侧分类证据（不可变视图）。
     *
     * `beanTypes` 是「BeanDefinition 没有 beanClassName、类型来自 `@Bean` 工厂方法
     * 返回值」的类——这类类在真实项目里很常见（`@Bean public Foo foo()`），只靠
     * `applicationBeanClasses` 会漏掉；`beanFactories` 给出同源键（factoryMethodName
     * 或 包根+简单名），用于把 `Foo`/`FooImpl` 这类实现归到同一组。
     */
    public static final class SpringBeanEvidence {
        public static final SpringBeanEvidence EMPTY =
                new SpringBeanEvidence(Collections.<String>emptySet(),
                        Collections.<String, String>emptyMap());

        private final Set<String> beanTypes;
        private final Map<String, String> beanFactories;

        public SpringBeanEvidence(Set<String> beanTypes, Map<String, String> beanFactories) {
            this.beanTypes = beanTypes == null
                    ? Collections.<String>emptySet() : beanTypes;
            this.beanFactories = beanFactories == null
                    ? Collections.<String, String>emptyMap() : beanFactories;
        }

        public Set<String> beanTypes() {
            return beanTypes;
        }

        public Map<String, String> beanFactories() {
            return beanFactories;
        }
    }

    // ------------------------------------------------------------------
    // 类清单（分类 / 过滤 / 分页）
    // ------------------------------------------------------------------

    /**
     * 类清单查询参数（与 Rust / 前端字段一一对应）。
     *
     * 兼容老调用方：旧参数 {@code include} / {@code exclude} 前缀规则与
     * {@code include_jdk} 继续生效（include 命中即保留，不受类别过滤限制）。
     */
    public static final class ListQuery {
        public String search;
        public String include;
        public String exclude;
        /** 类别筛选（空 = 默认：Spring Bean + 业务类）。 */
        public Set<ClassClassifier.Category> categories = new LinkedHashSet<ClassClassifier.Category>();
        /** 包名前缀（多形态字符串，可空）。 */
        public String packagePrefix;
        public boolean includeDependencies;
        public boolean includeJdk;
        /** true = 展示全部类别（高级浏览模式）。 */
        public boolean all;
        /** 业务包前缀（用户配置；供分类弱证据使用）。 */
        public String businessPackages;
        /** 强制刷新索引（默认增量；索引为空时自动刷新）。 */
        public boolean refreshIndex = true;
        /** 只返回某类别计数（不返回明细）——给统计面板用。 */
        public boolean statsOnly;
        public int limit = 200;
        public int offset = 0;
    }

    public Map<String, Object> listClasses(ListQuery query) {
        ClassIndex index = this.index;
        // 增量刷新（只分类新出现的类）：首次访问 / 显式 refresh_index 时执行；
        // 刷新本身是 O(新增类)，不是 O(全部类)，所以每次列表请求都刷新也安全。
        ClassIndex.RefreshResult refresh = null;
        if (query.refreshIndex || index.size() == 0) {
            refresh = refreshIndex(query.businessPackages);
        }

        Set<ClassClassifier.Category> categories = effectiveCategories(query);
        List<String> prefixes = ClassFilter.parsePrefixes(query.packagePrefix);
        String term = query.search == null ? null : query.search.trim().toLowerCase(Locale.ROOT);
        if (term != null && term.isEmpty()) {
            term = null;
        }
        String include = query.include;
        String exclude = query.exclude;

        List<ClassIndex.Entry> matched = new ArrayList<ClassIndex.Entry>();
        for (ClassIndex.Entry entry : index.snapshot()) {
            if (entry.array) {
                continue; // 数组类（[Ljava.lang.String;）不进列表
            }
            boolean forcedInclude = ClassFilter.matchesAnyPrefix(entry.name, include);
            if (!forcedInclude) {
                if (!categories.contains(entry.category)) {
                    continue;
                }
                if (!prefixes.isEmpty() && !ClassFilter.matchesAnyPrefix(entry.name, prefixes)) {
                    continue;
                }
                if (!prefixes.isEmpty() && ClassFilter.matchesAnyPrefix(entry.name, exclude)) {
                    continue;
                }
            }
            if (term != null && !ClassIndex.nameMatches(entry, term)) {
                continue;
            }
            matched.add(entry);
        }
        matched.sort(ClassIndex.DEFAULT_COMPARATOR);

        int total = matched.size();
        int from = Math.max(0, query.offset);
        int size = Math.max(1, query.limit);
        int to = Math.min(total, from + size);
        List<Map<String, Object>> items = new ArrayList<Map<String, Object>>();
        if (!query.statsOnly && from < total) {
            for (ClassIndex.Entry entry : matched.subList(from, to)) {
                items.add(describeBrief(entry));
            }
        }
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        out.put("total", total);
        out.put("returned", items.size());
        out.put("offset", from);
        out.put("items", items);
        out.put("categories", wireNames(categories));
        out.put("stats", index.stats().toMap());
        if (refresh != null) {
            out.put("refresh", refresh.toMap());
            out.put("incremental", false);
        } else {
            out.put("incremental", true);
        }
        out.put("launch", index.launchSummary());
        return out;
    }

    /** 索引内容变化摘要（不重跑全量扫描：statistics 走缓存）。 */
    public Map<String, Object> stats(String businessPackages) {
        ClassIndex index = this.index;
        ClassIndex.RefreshResult refresh = null;
        if (index.size() == 0) {
            refresh = refreshIndex(businessPackages);
        }
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        out.put("stats", index.stats().toMap());
        out.put("launch", index.launchSummary());
        out.put("loaded_class_count", loadedClassCount());
        if (refresh != null) {
            out.put("refresh", refresh.toMap());
        }
        return out;
    }

    /** 全量重建索引（用户点「重新扫描」/ 业务包配置变化后）。 */
    public Map<String, Object> rebuildIndex(String businessPackages) {
        index.clear();
        ClassIndex.RefreshResult refresh = refreshIndex(businessPackages);
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        out.put("refresh", refresh.toMap());
        out.put("stats", index.stats().toMap());
        out.put("launch", index.launchSummary());
        return out;
    }

    private ClassIndex.RefreshResult refreshIndex(String businessPackages) {
        Set<String> beanClasses = new LinkedHashSet<String>();
        Set<String> beanTypes = new LinkedHashSet<String>();
        Map<String, String> beanFactories = new LinkedHashMap<String, String>();
        Map<String, String> beanNames = new LinkedHashMap<String, String>();
        boolean springPresent = false;
        SpringBeanSource source = this.springBeanSource;
        if (source != null) {
            try {
                springPresent = source.springPresent();
                if (springPresent) {
                    SpringBeanEvidence evidence = source.springBeanEvidence();
                    if (evidence != null) {
                        beanTypes.addAll(evidence.beanTypes());
                        beanFactories.putAll(evidence.beanFactories());
                    }
                    Set<String> classes = source.applicationBeanClasses();
                    if (classes != null) {
                        beanClasses.addAll(classes);
                    }
                    Map<String, String> names = source.beanNamesByClass();
                    if (names != null) {
                        beanNames.putAll(names);
                    }
                }
            } catch (Throwable t) {
                // Spring 探测异常不影响类发现：降级为 codeSource 判定
            }
        }
        String[] packages = businessPackages == null
                ? new String[0] : businessPackages.split(",");
        return index.refresh(packages, beanClasses, beanTypes, beanFactories, beanNames,
                springPresent);
    }

    private static Set<ClassClassifier.Category> effectiveCategories(ListQuery query) {
        Set<ClassClassifier.Category> out = new LinkedHashSet<ClassClassifier.Category>();
        if (query.all) {
            // 高级浏览模式：全部类别（含 JDK / 依赖），但 GENERATED 仍默认排除
            out.addAll(Arrays.asList(ClassClassifier.Category.values()));
            out.remove(ClassClassifier.Category.GENERATED);
            if (query.includeJdk) {
                out.add(ClassClassifier.Category.JDK_INTERNAL);
            }
            return out;
        }
        if (query.categories != null && !query.categories.isEmpty()) {
            out.addAll(query.categories);
            if (!query.includeJdk) {
                out.remove(ClassClassifier.Category.JDK_INTERNAL);
            }
            if (!query.includeDependencies) {
                out.remove(ClassClassifier.Category.DEPENDENCY);
            }
            return out;
        }
        out.addAll(DEFAULT_CATEGORIES);
        if (query.includeDependencies) {
            out.add(ClassClassifier.Category.DEPENDENCY);
        }
        if (query.includeJdk) {
            out.add(ClassClassifier.Category.JDK_INTERNAL);
        }
        out.add(ClassClassifier.Category.GENERATED);
        return out;
    }

    private static List<String> wireNames(Set<ClassClassifier.Category> categories) {
        List<String> out = new ArrayList<String>();
        for (ClassClassifier.Category c : categories) {
            out.add(c.wire());
        }
        return out;
    }

    /** 类简要信息（列表行）。 */
    private Map<String, Object> describeBrief(ClassIndex.Entry entry) {
        Map<String, Object> item = new LinkedHashMap<String, Object>();
        item.put("name", entry.name);
        item.put("package_name", entry.packageName);
        item.put("simple_name", entry.simpleName);
        item.put("category", entry.category.wire());
        item.put("role", entry.role);
        item.put("source", entry.source == null ? null : entry.source.toMap());
        item.put("bean_name", entry.beanName);
        item.put("jdk_internal", entry.category == ClassClassifier.Category.JDK_INTERNAL);
        item.put("dependency", entry.category == ClassClassifier.Category.DEPENDENCY);
        item.put("array", entry.array);
        item.put("synthetic", entry.synthetic);
        Map<String, Object> loader = new LinkedHashMap<String, Object>();
        loader.put("id", String.valueOf(entry.loaderIdentity));
        loader.put("name", entry.loaderName);
        loader.put("identity", entry.loaderIdentity);
        item.put("loader", loader);
        return item;
    }

    /** 类详情（含方法数 / 父类 / 接口 / 修饰符 / 分类证据）。 */
    public Map<String, Object> classDetail(String className) throws ClassNotFoundException {
        Class<?> cls = findLoadedClass(className);
        ClassIndex.Entry entry = index.find(cls);
        Map<String, Object> out;
        if (entry != null) {
            out = describeBrief(entry);
        } else {
            ClassOrigin.Source source = ClassOrigin.resolve(cls);
            ClassClassifier.Verdict verdict = ClassClassifier.classify(cls, source,
                    index.context());
            out = new LinkedHashMap<String, Object>();
            out.put("name", cls.getName());
            Map<String, String> split = ClassFilter.splitName(cls.getName());
            out.put("package_name", split.get("package_name"));
            out.put("simple_name", split.get("simple_name"));
            out.put("category", verdict.category.wire());
            out.put("role", verdict.role);
            out.put("source", source.toMap());
            out.put("evidence", verdict.evidence);
        }
        if (!out.containsKey("evidence")) {
            // 索引命中：重新计算证据链（很便宜：只读 codeSource）
            ClassClassifier.Verdict verdict = ClassClassifier.classify(cls,
                    ClassOrigin.resolve(cls), index.context());
            out.put("evidence", verdict.evidence);
        }
        ClassLoader loader = cls.getClassLoader();
        out.put("superclass", cls.getSuperclass() != null ? cls.getSuperclass().getName() : null);
        List<String> interfaces = new ArrayList<String>();
        for (Class<?> it : cls.getInterfaces()) {
            interfaces.add(it.getName());
        }
        out.put("interfaces", interfaces);
        out.put("method_count", methodCount(cls));
        out.put("field_count", cls.getDeclaredFields().length);
        out.put("modifiers", Modifier.toString(cls.getModifiers()));
        out.put("interface", cls.isInterface());
        out.put("enum", cls.isEnum());
        out.put("annotation", cls.isAnnotation());
        out.put("proxy", java.lang.reflect.Proxy.isProxyClass(cls));
        out.put("loader_detail", loaderDetail(loader));
        return out;
    }

    /**
     * 类的可调用方法清单（public + protected + package 可见实例/静态方法；
     * private 默认包含但标注——反射可调，由风险策略与 UI 把关）。
     */
    public List<Map<String, Object>> listMethods(String className, boolean includePrivate)
            throws ClassNotFoundException {
        Class<?> cls = findLoadedClass(className);
        List<Map<String, Object>> out = new ArrayList<Map<String, Object>>();
        // 统计同名重载（含继承链）以便 UI 区分
        Map<String, Integer> nameCounts = new LinkedHashMap<String, Integer>();
        for (Method m : allMethods(cls)) {
            nameCounts.merge(m.getName(), 1, Integer::sum);
        }
        for (Method m : allMethods(cls)) {
            boolean isPrivate = Modifier.isPrivate(m.getModifiers());
            if (isPrivate && !includePrivate) {
                continue;
            }
            if (m.isSynthetic()) {
                continue; // 编译器合成物（bridge / access$）：不作为可调入口展示
            }
            out.add(describeMethod(cls, m,
                    nameCounts.getOrDefault(m.getName(), 1) > 1));
        }
        return out;
    }

    /** 类可调用方法数（去重计数）。 */
    public int methodCount(Class<?> cls) {
        try {
            return allMethods(cls).size();
        } catch (Throwable e) {
            return -1; // 元信息不可用（个别类反射受限）：不毒化列表
        }
    }

    private List<Method> allMethods(Class<?> cls) {
        List<Method> out = new ArrayList<Method>();
        Class<?> cursor = cls;
        int depth = 0;
        while (cursor != null && cursor != Object.class && depth < 12) {
            try {
                out.addAll(Arrays.asList(cursor.getDeclaredMethods()));
            } catch (Throwable ignored) {
                // 个别类 getDeclaredMethods 抛错（如反射元数据受限）：跳过该层
            }
            cursor = cursor.getSuperclass();
            depth++;
        }
        return out;
    }

    private Map<String, Object> describeMethod(Class<?> owner, Method m, boolean overloaded) {
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        int mods = m.getModifiers();
        StringBuilder params = new StringBuilder();
        List<Map<String, Object>> paramInfos = new ArrayList<Map<String, Object>>();
        Class<?>[] types = m.getParameterTypes();
        java.lang.reflect.Type[] genericTypes = m.getGenericParameterTypes();
        String[] paramNames = parameterNames(m);
        for (int i = 0; i < types.length; i++) {
            if (i > 0) {
                params.append(',');
            }
            params.append(types[i].getName());
            Map<String, Object> info = new LinkedHashMap<String, Object>();
            info.put("name", paramNames[i]);
            info.put("type", types[i].getName());
            info.put("generic", genericType(genericTypes[i]));
            info.put("primitive", types[i].isPrimitive());
            paramInfos.add(info);
        }
        String methodId = owner.getName() + "#" + m.getName()
                + (types.length == 0 ? "()" : "(" + params + ")");
        out.put("method_id", methodId);
        out.put("class_name", owner.getName());
        out.put("declaring_class", m.getDeclaringClass().getName());
        out.put("method_name", m.getName());
        out.put("signature", signatureOf(m));
        out.put("params", paramInfos);
        List<String> paramTypes = new ArrayList<String>(types.length);
        for (Class<?> type : types) {
            paramTypes.add(type.getName());
        }
        out.put("param_types", paramTypes);
        out.put("return_type", m.getReturnType().getName());
        out.put("return_generic", genericType(m.getGenericReturnType()));
        out.put("static", Modifier.isStatic(mods));
        out.put("visibility", visibility(mods));
        out.put("synthetic", m.isSynthetic());
        out.put("bridge", m.isBridge());
        out.put("varargs", m.isVarArgs());
        out.put("overloaded", overloaded);
        out.put("risk", RiskPolicy.riskLevel(owner, m));
        out.put("risk_reason", RiskPolicy.riskReason(owner, m));
        return out;
    }

    private static String[] parameterNames(Method m) {
        int count = m.getParameterCount();
        String[] out = new String[count];
        java.lang.reflect.Parameter[] parameters = m.getParameters();
        for (int i = 0; i < count; i++) {
            // -parameters 编译带真名；缺省 arg{i}（Java 8+ Parameter.isNamePresent）
            out[i] = parameters[i].isNamePresent()
                    ? parameters[i].getName()
                    : ("arg" + i);
        }
        return out;
    }

    private static String genericType(java.lang.reflect.Type type) {
        if (type == null || type instanceof Class) {
            return ""; // 无泛型信息
        }
        return type.getTypeName();
    }

    private static String signatureOf(Method m) {
        StringBuilder sb = new StringBuilder();
        int mods = m.getModifiers();
        if (Modifier.isStatic(mods)) {
            sb.append("static ");
        }
        sb.append(m.getReturnType().getSimpleName()).append(' ')
                .append(m.getName()).append('(');
        Class<?>[] types = m.getParameterTypes();
        for (int i = 0; i < types.length; i++) {
            if (i > 0) {
                sb.append(", ");
            }
            sb.append(types[i].getSimpleName());
        }
        sb.append(')');
        return sb.toString();
    }

    private static String visibility(int mods) {
        if (Modifier.isPublic(mods)) {
            return "public";
        }
        if (Modifier.isProtected(mods)) {
            return "protected";
        }
        if (Modifier.isPrivate(mods)) {
            return "private";
        }
        return "package";
    }

    /**
     * 已加载类查找（不触发加载——找不到说明类未加载， honest NOT_FOUND）。
     *
     * 走索引优先（O(1) 命中），未索引时再回落到全量扫描（首次 / 冷启动）。
     */
    public Class<?> findLoadedClass(String name) {
        Class<?> cls = findLoadedClassOrNull(name);
        if (cls == null) {
            throw new io.miyo.agent.ipc.IpcException(io.miyo.agent.ipc.Protocol.ERR_NOT_FOUND,
                    "类未在目标 JVM 中加载（Instrumentation 只能看到已加载的类）：" + name
                            + "。可先触发该类的使用（调用一次接口 / 查看页面）后刷新。");
        }
        return cls;
    }

    /** 已加载类查找（可空返回；供 Spring 捕获等探测型调用）。 */
    public Class<?> findLoadedClassOrNull(String name) {
        if (name == null || name.trim().isEmpty()) {
            return null;
        }
        // 索引优先：命中缓存零成本
        ClassIndex.Entry cached = index.findByName(name);
        if (cached != null) {
            Class<?> type = cached.type();
            if (type != null) {
                return type;
            }
        }
        Class<?>[] all;
        try {
            all = instrumentation.getAllLoadedClasses();
        } catch (Throwable t) {
            return null;
        }
        for (Class<?> cls : all) {
            if (cls.getName().equals(name)) {
                return cls;
            }
        }
        return null;
    }

    /** Instrumentation 报告的已加载类数（宽度指标）。 */
    public int loadedClassCount() {
        try {
            return instrumentation.getAllLoadedClasses().length;
        } catch (Throwable t) {
            return -1;
        }
    }

    /** loader 详情（类详情页）。 */
    private static Map<String, Object> loaderDetail(ClassLoader loader) {
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        if (loader == null) {
            out.put("id", "bootstrap");
            out.put("name", "bootstrap");
            out.put("identity", 0);
        } else {
            out.put("id", String.valueOf(System.identityHashCode(loader)));
            out.put("name", loader.getClass().getName());
            out.put("identity", System.identityHashCode(loader));
            ClassLoader parent = loader.getParent();
            out.put("parent", parent != null ? parent.getClass().getName() : "bootstrap");
        }
        return out;
    }

    /** 供排序 / 测试使用（不对外暴露实现细节）。 */
    static Comparator<ClassIndex.Entry> comparator() {
        return ClassIndex.DEFAULT_COMPARATOR;
    }
}