package io.miyo.agent.inspect;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * 业务类 / 依赖类 / JDK 内部类的多维度分类器（纯函数 + 只读探测）。
 *
 * <h3>类别</h3>
 * <ul>
 *   <li>{@link Category#APPLICATION}：被调试应用自己的业务代码；</li>
 *   <li>{@link Category#SPRING_BEAN}：Spring 上下文里 ROLE_APPLICATION 的 Bean 实现类
 *       （且仍属应用自身代码——第三方 starter 的 Bean 归 {@link Category#DEPENDENCY}）；</li>
 *   <li>{@link Category#DEPENDENCY}：第三方依赖（BOOT-INF/lib、.m2 仓库等）；</li>
 *   <li>{@link Category#GENERATED}：CGLIB / JDK Proxy / Mockito 等动态生成类；</li>
 *   <li>{@link Category#JDK_INTERNAL}：JDK / JVM 内部；</li>
 *   <li>{@link Category#UNKNOWN}：证据不足——宁可 UNKNOWN，不误判。</li>
 * </ul>
 *
 * <h3>判定顺序（证据从强到弱）</h3>
 * <ol>
 *   <li>代理 / 生成类特征（名字形态 + Proxy.isProxyClass）→ GENERATED；</li>
 *   <li>codeSource 净判定：JDK / 嵌套 classes / 嵌套 lib / 依赖仓库 jar /
 *       应用 classes 目录 / 应用 jar —— 直接定论；</li>
 *   <li>仅当 codeSource 为 UNKNOWN 时才使用弱证据：Spring Bean 角色 → ClassLoader
 *       分组 → 启动信息（Main-Class 同包前缀 / classpath 条目）→ 用户配置的
 *       业务包前缀 → 包名根注册表；</li>
 *   <li>仍无证据 → UNKNOWN（并记录「为什么无法识别」）。</li>
 * </ol>
 *
 * 明确拒绝的做法：纯包名黑名单。包名只作为「用户显式配置的偏好」参与，
 * 并且允许被更强的 codeSource 证据覆盖（{@link Source#APP_CLASSES_DIR} 等）。
 */
public final class ClassClassifier {

    /** 类类别（与前端 JavaClassCategory 一一对应）。 */
    public enum Category {
        APPLICATION("application"),
        SPRING_BEAN("spring_bean"),
        DEPENDENCY("dependency"),
        GENERATED("generated"),
        JDK_INTERNAL("jdk_internal"),
        UNKNOWN("unknown");

        private final String wire;

        Category(String wire) {
            this.wire = wire;
        }

        public String wire() {
            return wire;
        }

        /** 宽松解析（大小写不敏感；非法值 → null，调用方忽略）。 */
        public static Category parse(String raw) {
            if (raw == null || raw.trim().isEmpty()) {
                return null;
            }
            String v = raw.trim().toLowerCase(Locale.ROOT);
            for (Category c : values()) {
                if (c.wire.equals(v)) {
                    return c;
                }
            }
            return null;
        }
    }

    /** 判定结果（一次分类的完整证据链，UI 可原样展示）。 */
    public static final class Verdict {
        public final Category category;
        /** 业务子类型：controller / service / manager / repository / mapper / dto / config / 其他。 */
        public final String role;
        /** 命中证据（人类可读，逐条）。 */
        public final List<String> evidence;

        Verdict(Category category, String role, List<String> evidence) {
            this.category = category;
            this.role = role;
            this.evidence = evidence;
        }

        static Verdict of(Category category, String role, String... evidence) {
            List<String> list = new ArrayList<String>();
            Collections.addAll(list, evidence);
            return new Verdict(category, role, list);
        }
    }

    /** 分类上下文（每轮扫描构建一次；不可变）。 */
    public static final class Context {
        /** 用户配置的业务包前缀（已归一：无通配符，小写比较）。 */
        public final List<String> businessPackages;
        /** 从 Spring 上下文收集的 ROLE_APPLICATION Bean 实现类名。 */
        public final Set<String> springBeanClasses;
        /**
         * `@Bean` 工厂方法的返回类型（BeanDefinition 无 beanClassName 时用
         * resolvable type 解析出来的类名）。
         */
        public final Set<String> springBeanTypes;
        /**
         * Bean 来源标识（同源实现类的弱证据）：`实现类全名 → factoryMethodName`
         * 或 `包根 + 简单名前后缀` 归一后的键。
         */
        public final Map<String, String> springBeanFactories;
        /** 从 JVM 启动信息推导出的应用包根（如 com.example）。 */
        public final List<String> launchPackageRoots;
        /**
         * 已知的依赖类加载器（映射到 Boot fat-jar 的依赖子加载器）。
         * 命中的类若 codeSource 缺失，可降级判为 DEPENDENCY。
         */
        public final Set<Integer> dependencyLoaderIds;
        /** 业务类加载器（spring-boot-devtools 的 RestartClassLoader 等）。 */
        public final Set<Integer> applicationLoaderIds;
        /** 已推断出的包根 → 类别（同一轮扫描内先到先得）。 */
        public final Map<String, CategoryWithRole> packageRootHints;
        /** 应用 classes 目录（归一化，末尾带 /）。 */
        public final List<String> appClassesDirs;
        /** 当前扫描是否检测到 Spring。 */
        public final boolean springPresent;

        public Context(List<String> businessPackages,
                       Set<String> springBeanClasses,
                       List<String> launchPackageRoots,
                       Set<Integer> dependencyLoaderIds,
                       Set<Integer> applicationLoaderIds,
                       List<String> appClassesDirs,
                       boolean springPresent) {
            this(businessPackages, springBeanClasses, null, null, launchPackageRoots,
                    dependencyLoaderIds, applicationLoaderIds, appClassesDirs, springPresent);
        }

        /**
         * @param springBeanTypes     `@Bean` 工厂方法返回类型（可空）
         * @param springBeanFactories 实现类全名 → 来源键（可空）
         */
        public Context(List<String> businessPackages,
                       Set<String> springBeanClasses,
                       Set<String> springBeanTypes,
                       Map<String, String> springBeanFactories,
                       List<String> launchPackageRoots,
                       Set<Integer> dependencyLoaderIds,
                       Set<Integer> applicationLoaderIds,
                       List<String> appClassesDirs,
                       boolean springPresent) {
            this.businessPackages = normalizePrefixes(businessPackages);
            this.springBeanClasses = springBeanClasses == null
                    ? Collections.<String>emptySet() : springBeanClasses;
            this.springBeanTypes = springBeanTypes == null
                    ? Collections.<String>emptySet() : springBeanTypes;
            this.springBeanFactories = springBeanFactories == null
                    ? Collections.<String, String>emptyMap()
                    : Collections.unmodifiableMap(new LinkedHashMap<String, String>(springBeanFactories));
            this.launchPackageRoots = launchPackageRoots == null
                    ? Collections.<String>emptyList() : launchPackageRoots;
            this.dependencyLoaderIds = dependencyLoaderIds == null
                    ? Collections.<Integer>emptySet() : dependencyLoaderIds;
            this.applicationLoaderIds = applicationLoaderIds == null
                    ? Collections.<Integer>emptySet() : applicationLoaderIds;
            this.appClassesDirs = appClassesDirs == null
                    ? Collections.<String>emptyList() : appClassesDirs;
            this.springPresent = springPresent;
            this.packageRootHints = Collections.synchronizedMap(
                    new LinkedHashMap<String, CategoryWithRole>());
        }

        public static Context empty() {
            return new Context(null, null, null, null, null, null, false);
        }
    }

    /** 包根提示值。 */
    public static final class CategoryWithRole {
        public final Category category;
        public final String role;

        public CategoryWithRole(Category category, String role) {
            this.category = category;
            this.role = role;
        }
    }

    // ------------------------------------------------------------------
    // 包名前缀归一（用户输入的多种形态都接受）
    // ------------------------------------------------------------------

    /** 归一用户输入：`com.example.*` / `com.example.` / ` com.example ` → `com.example`. */
    public static List<String> normalizePrefixes(Collection<String> raw) {
        Set<String> out = new LinkedHashSet<String>();
        if (raw == null) {
            return new ArrayList<String>();
        }
        for (String item : raw) {
            if (item == null) {
                continue;
            }
            for (String part : item.split("[,;\\s]+")) {
                String p = part.trim();
                while (p.endsWith("*") || p.endsWith(".")) {
                    p = p.substring(0, p.length() - 1);
                }
                if (p.startsWith("*")) {
                    p = p.substring(1);
                }
                if (!p.isEmpty() && p.length() < 512) {
                    out.add(p);
                }
            }
        }
        return new ArrayList<String>(out);
    }

    /** Miyo agent 自身的类名前缀（工具实现，不是被调试应用的业务代码）。 */
    static final String OWN_AGENT_PREFIX = "io.miyo.agent.";
    /**
     * agent 运行时自身的类清单（懒解析，失败降级为空 = 老行为）。
     *
     * 用「agent jar 里实际存在的类」而不是前缀：前缀会误伤同包名的测试夹具
     * （`io.miyo.agent.inspect.Fixtures$OrderService`），也会误伤将来的同前缀
     * 子模块。
     */
    private static volatile Set<String> ownAgentClasses;

    /**
     * Miyo agent 自带的类不属被调试应用。
     *
     * <p>为什么需要这条：agent jar 由 launcher 通过 {@code -javaagent} / attach 注入，
     * 在 Spring Boot 容器里由 LaunchedClassLoader 追加加载——它报上来的 codeSource
     * 与 app.jar 同源，最强证据会把 agent 自身判成「应用 jar」（端到端实测：
     * `io.miyo.agent.AgentArgs` 出现在默认业务列表里）。agent 代码是工具实现，
     * 归 DEPENDENCY（「含依赖」开关统一收纳），避免污染业务代码视图。
     */
    static boolean isOwnAgentClass(String name, List<String> evidence) {
        if (name == null || !name.startsWith(OWN_AGENT_PREFIX)) {
            return false;
        }
        if (!ownAgentClasses().contains(name)) {
            return false;
        }
        evidence.add("Miyo agent 自身的调试实现（" + OWN_AGENT_PREFIX + "*），不计入被调试应用");
        return true;
    }

    /** 懒加载 agent 自身类清单（见 {@link #isOwnAgentClass}）。 */
    static Set<String> ownAgentClasses() {
        Set<String> cached = ownAgentClasses;
        if (cached != null) {
            return cached;
        }
        Set<String> out = new LinkedHashSet<String>();
        try {
            // 用分类器自身的 codeSource 枚举同 jar 内的类：agent jar 的清单就是
            // 「我们自己」的定义，不依赖外部配置（清单文件可被 shade 保留）。
            java.security.CodeSource cs = null;
            java.security.ProtectionDomain pd = ClassClassifier.class.getProtectionDomain();
            if (pd != null) {
                cs = pd.getCodeSource();
            }
            if (cs != null && cs.getLocation() != null
                    && "file".equals(cs.getLocation().getProtocol())) {
                java.io.File self = new java.io.File(cs.getLocation().toURI());
                if (self.isFile()) {
                    java.util.jar.JarFile jar = new java.util.jar.JarFile(self);
                    try {
                        java.util.Enumeration<java.util.jar.JarEntry> entries = jar.entries();
                        while (entries.hasMoreElements()) {
                            String entry = entries.nextElement().getName();
                            if (entry.endsWith(".class")) {
                                out.add(entry.substring(0, entry.length() - 6).replace('/', '.'));
                            }
                        }
                    } finally {
                        jar.close();
                    }
                }
            }
        } catch (Throwable t) {
            // 只读优化：枚举失败时退化为「不排除」（不影响分类正确性，只是多几行）
        }
        ownAgentClasses = out;
        return out;
    }

    // ------------------------------------------------------------------
    // 主判定
    // ------------------------------------------------------------------

    /**
     * 分类一个已加载类。只读 codeSource / 修饰符 / 名字 / 上下文集合——
     * 不做昂贵反射（不展开方法 / 字段 / 注解）。
     */
    public static Verdict classify(Class<?> cls, ClassOrigin.Source source, Context ctx) {
        return classify(cls, cls.getName(), source, ctx);
    }

    /**
     * 分类内核（名字与 Class 分离：生成类判定只看名字，测试可注入伪名字）。
     * 生产路径永远经 {@link #classify(Class, ClassOrigin.Source, Context)} 调用。
     */
    static Verdict classify(Class<?> cls, String name, ClassOrigin.Source source, Context ctx) {
        List<String> evidence = new ArrayList<String>();

        // 1) 生成类（代理 / lambda / 合成）——名字形态是最可靠的判据
        if (isGeneratedName(name) || isProxyClass(cls)) {
            evidence.add("名字 / 类型命中动态生成特征：" + describeGenerated(name, cls));
            evidence.add("上述类由框架在运行期生成，不属于用户源码");
            return new Verdict(Category.GENERATED, "other", evidence);
        }

        // 1.5) 框架 / 容器自身实现包：codeSource 可能指向 app.jar（Boot 3.x loader
        //      与 app 共享来源），包名证据在此强于 codeSource
        String frameworkRoot = frameworkOwnedRoot(name);
        if (frameworkRoot != null) {
            evidence.add("命中框架 / 容器自身实现包：" + frameworkRoot + "（不受 codeSource 影响）");
            return new Verdict(Category.DEPENDENCY, "other", evidence);
        }

        // 2) codeSource 净证据
        if (source != null) {
            switch (source.kind) {
                case JDK:
                    evidence.add(source.evidence);
                    return new Verdict(Category.JDK_INTERNAL, "other", evidence);
                case NESTED_LIB:
                    evidence.add(source.evidence);
                    evidence.add("嵌套依赖容器（BOOT-INF/lib、WEB-INF/lib）里的类一律视为第三方");
                    return new Verdict(Category.DEPENDENCY, "other", evidence);
                case NESTED_CLASSES:
                    // codeSource 已明确是「应用的 classes 目录」：这是最强证据，不再用
                    // 包名根覆盖（否则业务包恰好叫 io.miyo / com.github 就会被误降级）
                    evidence.add(source.evidence);
                    if (isOwnAgentClass(name, evidence)) {
                        return new Verdict(Category.DEPENDENCY, "other", evidence);
                    }
                    return new Verdict(appCategory(name, ctx, evidence),
                            roleOf(cls, name, ctx, evidence), evidence);
                case APP_CLASSES_DIR:
                    evidence.add(source.evidence);
                    if (isOwnAgentClass(name, evidence)) {
                        return new Verdict(Category.DEPENDENCY, "other", evidence);
                    }
                    return new Verdict(appCategory(name, ctx, evidence), roleOf(cls, name, ctx, evidence),
                            evidence);
                case APP_JAR:
                    // 普通 jar（非依赖仓库路径）：默认按应用自身 jar 处理。
                    // 不拿包名根反推——business 项目自己打出的 fat-jar 里也有
                    // org.apache.* 这类包（shade/relocate 前的残留），代码位置比
                    // 包名更能说明「这段代码从哪来」。
                    evidence.add(source.evidence);
                    if (isOwnAgentClass(name, evidence)) {
                        return new Verdict(Category.DEPENDENCY, "other", evidence);
                    }
                    return new Verdict(appCategory(name, ctx, evidence), roleOf(cls, name, ctx, evidence),
                            evidence);
                case UNKNOWN:
                    evidence.add(source.evidence);
                    break;
                default:
                    break;
            }
        }

        // 3) 弱证据（仅 codeSource 无法定论时使用）
        if (ctx != null) {
            // 3a) Spring Bean 角色（要求类名精确匹配——CGLIB 子类已在第 1 步归生成类）
            if (ctx.springBeanClasses != null && ctx.springBeanClasses.contains(name)) {
                evidence.add("Spring 上下文 ROLE_APPLICATION Bean 实现类");
                return new Verdict(Category.SPRING_BEAN, roleOf(cls, name, ctx, evidence), evidence);
            }
            // 3b) 用户显式配置的业务包
            String matchedPkg = matchPrefix(name, ctx.businessPackages);
            if (matchedPkg != null) {
                evidence.add("命中用户配置的业务包前缀：" + matchedPkg);
                return new Verdict(Category.APPLICATION, roleOf(cls, name, ctx, evidence), evidence);
            }
            // 3c) 启动信息推导的包根
            String matchedLaunchRoot = matchPrefix(name, ctx.launchPackageRoots);
            if (matchedLaunchRoot != null) {
                evidence.add("命中 JVM 启动信息推导的应用包根：" + matchedLaunchRoot);
                return new Verdict(Category.APPLICATION, roleOf(cls, name, ctx, evidence), evidence);
            }
            // 3d) ClassLoader 分组（只作辅助：命中文档化的依赖加载器才降级）
            ClassLoader loader = cls.getClassLoader();
            if (loader != null) {
                int id = System.identityHashCode(loader);
                if (ctx.dependencyLoaderIds.contains(id)) {
                    evidence.add("类加载器 " + loader.getClass().getSimpleName()
                            + " 已确认为依赖容器加载器（codeSource 缺失时降级）");
                    return new Verdict(Category.DEPENDENCY, "other", evidence);
                }
            }
            // 3e) 包根注册表（同一轮扫描中先到先得：见 packageRootHints）
            Category byHint = hintedCategory(name, ctx, evidence);
            if (byHint != null) {
                return new Verdict(byHint, "other", evidence);
            }
            // 3f) 已知框架包根
            Category byRoot = packageRootCategory(name, ctx, evidence);
            if (byRoot != null) {
                evidence.add("包名根属于已知框架 / 依赖根（弱证据）");
                return new Verdict(byRoot, "other", evidence);
            }
        } else {
            Category byRoot = packageRootCategory(name, null, evidence);
            if (byRoot != null) {
                evidence.add("包名根属于已知框架 / 依赖根（弱证据）");
                return new Verdict(byRoot, "other", evidence);
            }
        }

        evidence.add("无法从 codeSource / 启动信息 / 加载器 / Bean 角色中得出归属");
        return new Verdict(Category.UNKNOWN, "other", evidence);
    }

    /** 应用自身代码的类别：Spring Bean 身份优先，其次 APPLICATION。 */
    /**
     * 框架 / 容器自身实现包（由 codeSource 看似「应用 jar」但实为启动器 / 容器代码）。
     *
     * <p>为什么需要：Spring Boot 3.2 起 loader 的类与 app.jar 共享 codeSource（由
     * 父加载器加载），codeSource 证据会把 `org.springframework.boot.loader.*` 判成
     * 应用代码（端到端实测：`ManifestInfo` 出现在业务列表里）。这些包在任何应用里
     * 都是框架实现，无论打包方式如何都归依赖。
     */
    private static final String[] FRAMEWORK_OWNED_ROOTS = {
            "org.springframework.boot.loader.",
            "org.springframework.boot.devtools.",
            "sun.reflect.", "jdk.internal.reflect.",
            "com.sun.proxy.",
    };

    /** 名字是否属于框架自身实现（不受 codeSource 影响）。 */
    static String frameworkOwnedRoot(String name) {
        if (name == null) {
            return null;
        }
        for (String root : FRAMEWORK_OWNED_ROOTS) {
            if (name.startsWith(root)) {
                return root;
            }
        }
        return null;
    }

    /**
     * 应用自身 classes 目录里的类：APPLICATION 还是 SPRING_BEAN？
     *
     * 三类命中都算 Spring Bean（**类型级**，不看是否已实例化）：
     * <ul>
     *   <li>类本身是 ROLE_APPLICATION Bean（`@Component` / `@Service` / `@Controller`）；</li>
     *   <li>是 `@Bean` 工厂方法的返回类型（BeanDefinition 无 beanClassName，只有
     *       resolvable type——这是真实项目里最常见的 Bean 声明方式）；</li>
     *   <li>是 Bean 类型的**同源实现**（同一 `factoryMethodName` 或同包根 + 简单名
     *       互为前后缀，如 `Foo`/`FooImpl`、`FooAutoConfiguration`）。</li>
     * </ul>
     * 不做「同包根一律算 Bean」的猜测：那会把同包的 DTO / 枚举一起吞进 Bean 组。
     */
    private static Category appCategory(String name, Context ctx, List<String> evidence) {
        if (ctx == null) {
            return Category.APPLICATION;
        }
        if (ctx.springBeanClasses.contains(name)) {
            evidence.add("类同时是 Spring 上下文 ROLE_APPLICATION Bean 实现类");
            return Category.SPRING_BEAN;
        }
        if (ctx.springBeanTypes.contains(name)) {
            evidence.add("@Bean 工厂方法返回类型（BeanDefinition 无 beanClassName）");
            return Category.SPRING_BEAN;
        }
        if (ctx.springBeanFactories.containsKey(name)) {
            evidence.add("与 @Bean 声明同源（同 factoryMethodName / 同包根的实现类）");
            return Category.SPRING_BEAN;
        }
        return Category.APPLICATION;
    }

    /**
     * 业务子类型（Controller / Service / Manager / Repository / Mapper / DTO / Config）。
     * 判据：类名后缀 + 包名段（不读注解——注解查找会对每个类触发元数据解析）。
     */
    public static String roleOf(Class<?> cls, String name, Context ctx, List<String> evidence) {
        String simple = name;
        int dot = simple.lastIndexOf('.');
        if (dot >= 0) {
            simple = simple.substring(dot + 1);
        }
        int dollar = simple.lastIndexOf('$');
        if (dollar > 0) {
            simple = simple.substring(dollar + 1);
        }
        String pkg = dot >= 0 ? name.substring(0, dot).toLowerCase(Locale.ROOT) : "";
        String s = simple.toLowerCase(Locale.ROOT);

        if (s.endsWith("controller") || s.endsWith("ctrl") || pkg.endsWith(".controller")
                || pkg.endsWith(".web") || pkg.contains(".controller.")) {
            return "controller";
        }
        if (s.endsWith("service") || s.endsWith("serviceimpl") || pkg.endsWith(".service")
                || pkg.endsWith(".services") || pkg.contains(".service.")) {
            return "service";
        }
        if (s.endsWith("manager") || pkg.endsWith(".manager") || pkg.contains(".manager.")) {
            return "manager";
        }
        if (s.endsWith("repository") || s.endsWith("repo") || s.endsWith("dao")
                || s.endsWith("mapper") || pkg.endsWith(".repository") || pkg.endsWith(".dao")
                || pkg.endsWith(".mapper") || pkg.contains(".persistence.")) {
            return "repository";
        }
        if (s.endsWith("dto") || s.endsWith("vo") || s.endsWith("bo") || s.endsWith("po")
                || s.endsWith("entity") || s.endsWith("request") || s.endsWith("response")
                || pkg.endsWith(".dto") || pkg.endsWith(".vo") || pkg.endsWith(".entity")
                || pkg.endsWith(".model") || pkg.endsWith(".domain")) {
            return "dto";
        }
        if (s.endsWith("config") || s.endsWith("configuration") || s.endsWith("properties")
                || pkg.endsWith(".config") || pkg.endsWith(".configuration")) {
            return "config";
        }
        if (cls != null && (cls.isEnum() || cls.isInterface() || cls.isAnnotation())) {
            return "other";
        }
        return "other";
    }

    // ------------------------------------------------------------------
    // 生成类
    // ------------------------------------------------------------------

    /** 名字形态判定（不触发类初始化）。 */
    static boolean isGeneratedName(String name) {
        if (name == null || name.isEmpty()) {
            return true;
        }
        if (name.indexOf("$$") >= 0) {
            return true;      // CGLIB（Foo$$EnhancerBySpringCGLIB）、Mockito、lambda
        }
        if (name.indexOf("$ByteBuddy$") >= 0) {
            return true;      // Byte Buddy 生成类（单 $ 分隔）
        }
        if (name.indexOf("$Proxy") >= 0) {
            return true;      // JDK 动态代理
        }
        if (name.endsWith("_$$_javassist") || name.indexOf("_$$_javassist_") >= 0) {
            return true;      // Javassist
        }
        if (name.indexOf("$HibernateProxy$") >= 0) {
            return true;
        }
        if (name.contains("$JaxbAccessor") || name.contains("$FastClass")
                || name.contains("$QuickClass")) {
            return true;
        }
        // lambdas：com.example.Foo$$Lambda$12/0x00000001 —— 已由 $$ 覆盖
        if (name.contains("$JSP") || name.startsWith("sun.reflect.Generated")
                || name.startsWith("jdk.internal.reflect.Generated")
                || name.startsWith("com.sun.proxy.$Proxy")) {
            return true;
        }
        // 匿名类 Foo$1 —— 不是「代理」，保留为业务类（用户源码），不在此拦截
        return false;
    }

    private static boolean isProxyClass(Class<?> cls) {
        try {
            // java.lang.reflect.Proxy 必在 JDK 里，直接用不会额外触发目标类加载
            return java.lang.reflect.Proxy.isProxyClass(cls);
        } catch (Throwable t) {
            return false;
        }
    }

    private static String describeGenerated(String name, Class<?> cls) {
        if (name.indexOf("$$EnhancerBy") >= 0) {
            return "CGLIB 代理";
        }
        if (name.indexOf("$Proxy") >= 0 || (cls != null && isProxyClass(cls))) {
            return "JDK 动态代理";
        }
        if (name.indexOf("$$") >= 0) {
            return "ByteBuddy / Mockito / 其他运行期生成";
        }
        return "运行期生成类";
    }

    // ------------------------------------------------------------------
    // 包名根
    // ------------------------------------------------------------------

    /** 已知 JDK 包根。 */
    private static final String[] JDK_ROOTS = {
            "java.", "javax.", "jdk.", "sun.", "com.sun.", "org.w3c.", "org.xml.",
            "org.ietf.", "org.omg.", "org.jcp.", "netscape.", "apple.", "com.apple.",
    };

    /**
     * 已知框架 / 依赖包根（**仅作为弱证据**，且只在 codeSource 无法定论时使用）。
     * 刻意保守：只收录「几乎不可能是用户业务包」的根。
     */
    private static final String[] DEPENDENCY_ROOTS = {
            "org.springframework.", "org.apache.", "org.junit.", "org.slf4j.",
            "org.hibernate.", "org.mybatis.", "org.jboss.", "org.eclipse.",
            "org.glassfish.", "org.jetbrains.", "org.yaml.", "org.dom4j.",
            "org.junit5.", "org.mockito.", "org.objenesis.", "org.ow2.",
            "org.bouncycastle.", "org.antlr.", "org.codehaus.", "org.jooq.",
            "org.thymeleaf.", "org.freemarker.", "org.yaml.snakeyaml.", "org.aspectj.",
            "org.apache.logging.", "org.apache.catalina.", "org.apache.tomcat.",
            "org.apache.http.", "org.apache.commons.", "org.apache.ibatis.",
            "org.apache.kafka.", "org.apache.zookeeper.", "org.apache.hadoop.",
            "org.apache.rocketmq.", "org.apache.dubbo.", "org.apache.shardingsphere.",
            "com.fasterxml.", "com.google.", "com.netflix.", "com.alibaba.",
            "com.alipay.", "com.ctrip.", "com.zaxxer.", "com.mysql.", "com.zendesk.",
            "com.sun.xml.", "com.github.", "com.codahale.", "com.uber.",
            "io.netty.", "io.grpc.", "io.lettuce.", "io.micrometer.", "io.micronaut.",
            "io.projectreactor.", "io.reactivex.", "io.swagger.", "io.undertow.",
            "io.vertx.", "io.jsonwebtoken.", "io.opentelemetry.", "io.prometheus.",
            "io.miyo.agent.", "io.milvus.", "io.seata.", "io.fabric8.",
            "ch.qos.", "net.bytebuddy.", "net.sf.", "net.java.", "net.javacrumbs.",
            "org.redisson.", "redis.clients.", "lombok.", "kotlin.", "scala.",
            "groovy.", "freemarker.", "oshi.", "oshi.architecture.",
            "oracle.", "com.oracle.", "com.ibm.", "com.microsoft.", "com.azure.",
            "com.amazonaws.", "software.amazon.", "vendor.", "graphql.",
            "reactor.", "rx.", "de.flapdoodle.", "com.h2database.",
    };

    /** 包名根判定（无 codeSource 时的弱证据）。 */
    static Category packageRootCategory(String name, Context ctx, List<String> evidence) {
        if (name == null) {
            return null;
        }
        for (String root : JDK_ROOTS) {
            if (name.startsWith(root)) {
                return Category.JDK_INTERNAL;
            }
        }
        for (String root : DEPENDENCY_ROOTS) {
            if (name.startsWith(root)) {
                return Category.DEPENDENCY;
            }
        }
        return null;
    }

    /**
     * 包根注册表：同类中的大多数（>=3 个类）共享同一包根时，把该根登记下来，
     * 供**没有 codeSource 的同类**使用——避免为了一个普通 jar 里的类逐个猜。
     */
    static Category hintedCategory(String name, Context ctx, List<String> evidence) {
        String root = packageRootOf(name);
        if (root == null) {
            return null;
        }
        CategoryWithRole hint = ctx.packageRootHints.get(root);
        if (hint == null) {
            return null;
        }
        evidence.add("包根 " + root + " 在本轮扫描中已由其他类确认为 " + hint.category.wire());
        return hint.category;
    }

    /** 注册包根提示（扫描结束时按类别多数投票调用）。 */
    static void registerHint(Context ctx, String className, Category category, String role) {
        String root = packageRootOf(className);
        if (root == null || category == Category.UNKNOWN || category == Category.GENERATED) {
            return;
        }
        synchronized (ctx.packageRootHints) {
            if (!ctx.packageRootHints.containsKey(root)) {
                ctx.packageRootHints.put(root, new CategoryWithRole(category, role));
            }
        }
    }

    /** 包根 = 前两段（com.example）；单段包原样返回。 */
    static String packageRootOf(String name) {
        if (name == null) {
            return null;
        }
        int first = name.indexOf('.');
        if (first < 0) {
            return null;
        }
        int second = name.indexOf('.', first + 1);
        return second < 0 ? name : name.substring(0, second);
    }

    /** 前缀匹配（按段边界：com.ex 不匹配 com.example）。返回命中的前缀或 null。 */
    static String matchPrefix(String name, List<String> prefixes) {
        if (name == null || prefixes == null) {
            return null;
        }
        for (String prefix : prefixes) {
            if (name.equals(prefix)) {
                return prefix;
            }
            if (name.length() > prefix.length() && name.startsWith(prefix)
                    && name.charAt(prefix.length()) == '.') {
                return prefix;
            }
        }
        return null;
    }

    // ------------------------------------------------------------------
    // Spring Bean 类收集（不实例化 Bean）
    // ------------------------------------------------------------------

    /**
     * 从 Spring 上下文收集 ROLE_APPLICATION Bean 的**声明类型**（不创建实例）。
     *
     * 只读 BeanDefinition：
     * 1. getBeanClassName()（普通 XML / @Bean 声明的工厂方法除外）；
     * 2. getResolvableType().resolve()（@Bean 工厂方法的返回类型）；
     * 3. 已实例化单例的实际类型（containsSingleton + getBean 的安全子集——
     *    此方法**从不**调用 getBean，避免触发惰性 Bean 创建）。
     *
     * 任一反射调用失败都不影响整体（返回已收集到的部分）。
     */
    public static Set<String> collectApplicationBeanClasses(Object context, Object beanFactory,
                                                            int maxBeans) {
        Set<String> out = new LinkedHashSet<String>();
        if (beanFactory == null) {
            return out;
        }
        try {
            Method namesMethod = beanFactory.getClass().getMethod("getBeanDefinitionNames");
            Object namesObj = namesMethod.invoke(beanFactory);
            if (!(namesObj instanceof String[])) {
                return out;
            }
            String[] names = (String[]) namesObj;
            Method definitionMethod = null;
            Method roleMethod = null;
            methodLoop:
            for (Method m : beanFactory.getClass().getMethods()) {
                if (definitionMethod == null && "getBeanDefinition".equals(m.getName())
                        && m.getParameterTypes().length == 1
                        && m.getParameterTypes()[0] == String.class) {
                    definitionMethod = m;
                }
                if (definitionMethod != null) {
                    break methodLoop;
                }
            }
            if (definitionMethod == null) {
                return out;
            }
            int limit = Math.min(names.length, Math.max(1, maxBeans));
            for (int i = 0; i < limit; i++) {
                String name = names[i];
                try {
                    Object definition = definitionMethod.invoke(beanFactory, name);
                    if (definition == null) {
                        continue;
                    }
                    if (roleMethod == null) {
                        roleMethod = findNoArgMethod(definition.getClass(), "getRole");
                    }
                    int role = 0;
                    if (roleMethod != null) {
                        Object r = roleMethod.invoke(definition);
                        role = r instanceof Number ? ((Number) r).intValue() : 0;
                    }
                    if (role != 0) {
                        continue; // ROLE_INFRASTRUCTURE / ROLE_SUPPORT
                    }
                    String className = beanClassName(definition);
                    if (className != null && !className.isEmpty()) {
                        out.add(className);
                    }
                } catch (Throwable ignored) {
                    // 单个 BeanDefinition 不可读：跳过
                }
            }
        } catch (Throwable t) {
            // 上下文形态不支持：空集合（调用方降级为 codeSource 判定）
        }
        return out;
    }

    /**
     * 遍历 ROLE_APPLICATION Bean，一次性产出分类需要的**全部** Spring 证据：
     * 实现类（`classes`）、`@Bean` 返回类型（`types`）、来源键（`factories`）。
     *
     * 与 `collectApplicationBeanClasses` 的区别：后者只给类名，丢掉了「这个类是怎么
     * 来的」（factoryMethodName / 包根+简单名），而后者正是判断「实现类 vs DTO」
     * 的唯一弱证据。一次遍历同时收集，避免二次读 BeanDefinition。
     *
     * @param out 输出（`classes` / `types` / `factories` 三个键预先放好可变集合）
     */
    public static void collectBeanEvidence(Object beanFactory, int maxBeans,
                                          Set<String> classes, Set<String> types,
                                          Map<String, String> factories) {
        if (beanFactory == null) {
            return;
        }
        try {
            Method namesMethod = beanFactory.getClass().getMethod("getBeanDefinitionNames");
            Object namesObj = namesMethod.invoke(beanFactory);
            if (!(namesObj instanceof String[])) {
                return;
            }
            String[] names = (String[]) namesObj;
            Method definitionMethod = null;
            for (Method m : beanFactory.getClass().getMethods()) {
                if ("getBeanDefinition".equals(m.getName())
                        && m.getParameterTypes().length == 1
                        && m.getParameterTypes()[0] == String.class) {
                    definitionMethod = m;
                    break;
                }
            }
            if (definitionMethod == null) {
                return;
            }
            int limit = Math.min(names.length, Math.max(1, maxBeans));
            for (int i = 0; i < limit; i++) {
                try {
                    Object definition = definitionMethod.invoke(beanFactory, names[i]);
                    if (definition == null) {
                        continue;
                    }
                    Object role = invokeNoArg(definition, "getRole");
                    if (role instanceof Number && ((Number) role).intValue() != 0) {
                        continue; // ROLE_INFRASTRUCTURE / ROLE_SUPPORT
                    }
                    String declared = asText(invokeNoArg(definition, "getBeanClassName"));
                    String type = declared;
                    String factory = asText(invokeNoArg(definition, "getFactoryMethodName"));
                    if (type == null || type.isEmpty()) {
                        type = resolvableType(definition);
                        // 降级路径：Boot 2.x 的 ConfigurationClassBeanDefinition 在
                        // Bean 尚未实例化时 getResolvableType() 可能解析不到返回类型
                        // （实测：`@Bean public ChannelManager channelManager()` 只拿到
                        // 工厂类自身或 null，导致「Spring Beans」组永远为 0）。此时按
                        // 工厂方法名到工厂类上读方法签名——纯元数据，不实例化。
                        if (type == null || type.isEmpty()) {
                            type = factoryMethodReturnType(definition, factory, beanFactory);
                        }
                        if (type != null && !type.isEmpty()) {
                            types.add(type);
                        }
                    }
                    if (type == null || type.isEmpty()) {
                        continue;
                    }
                    classes.add(type);
                    // 只读 BeanDefinition，不碰 Bean 实例（类发现必须零副作用）
                    String sourceKey = factory != null && !factory.isEmpty()
                            ? "fm:" + beanRootOf(type) + "#" + factory
                            : "root:" + beanRootOf(type);
                    factories.put(type, sourceKey);
                } catch (Throwable ignored) {
                    // 单个 BeanDefinition 不可读：跳过
                }
            }
        } catch (Throwable ignored) {
            // 上下文形态不支持：调用方降级为 codeSource 判定
        }
    }

    /**
     * 「同源实现」的归一键：包根 + 去掉常见实现后缀的简单名。
     *
     * `com.acme.SmsSender` 与 `com.acme.SmsSenderImpl` / `DefaultSmsSender` 归一到
     * `com.acme#smssender`；`FooAutoConfiguration` 是配置类本身（也是 Bean），它的
     * 实现类通常是 `FooImpl`，故剥离 `AutoConfiguration` 后缀。
     */
    static String beanRootOf(String className) {
        if (className == null || className.isEmpty()) {
            return "";
        }
        String name = className;
        int generic = name.indexOf('<');
        if (generic > 0) {
            name = name.substring(0, generic);
        }
        // 嵌套类取外层（@Bean 返回内部类时实现类通常也在外层）
        int dollar = name.indexOf('$');
        if (dollar > 0) {
            name = name.substring(0, dollar);
        }
        int dot = name.lastIndexOf('.');
        String pkg = dot > 0 ? name.substring(0, dot).toLowerCase(Locale.ROOT) : "";
        String simple = dot > 0 ? name.substring(dot + 1) : name;
        String lower = simple.toLowerCase(Locale.ROOT);
        for (String suffix : BEAN_IMPL_SUFFIXES) {
            if (lower.endsWith(suffix) && lower.length() > suffix.length()) {
                lower = lower.substring(0, lower.length() - suffix.length());
                break;
            }
            // 前缀形态：DefaultFoo / SimpleFoo / CustomFoo（Spring 生态常见）
            if (lower.startsWith(suffix) && lower.length() > suffix.length()) {
                lower = lower.substring(suffix.length());
                break;
            }
        }
        return pkg + "#" + lower;
    }

    /** 实现类常见后缀（剥离后与接口 / @Bean 返回类型归一）。 */
    private static final String[] BEAN_IMPL_SUFFIXES = {
            "autoconfiguration", "implementation", "impl", "provider", "adapter",
            "delegate", "proxy", "handler", "listener", "filters", "filter",
            "default", "simple", "custom", "abstract", "basic",
    };

    /**
     * 反射读无参 getter：先本类声明，再 public（含继承来的）。
     *
     * 为什么要两段：Spring 的 BeanDefinition 实现常把 getter 放在父类
     * （Generic / Scanned / Configuration BeanDefinition），只遍历
     * `getDeclaredMethods()` 会把 `@Bean` 返回类型与 factoryMethodName 全丢掉。
     */
    private static Object invokeNoArg(Object target, String method) {
        if (target == null) {
            return null;
        }
        try {
            Method m = findNoArgMethod(target.getClass(), method);
            if (m == null) {
                m = target.getClass().getMethod(method);
            }
            return m.invoke(target);
        } catch (Throwable ignored) {
            return null;
        }
    }

    /**
     * BeanDefinition.getResolvableType().resolve() 的类名。
     *
     * `resolve()` 返回 null（无泛型信息）时退回 `getTypeName()`——它是 String，
     * 不会触发类加载；泛型签名（`Foo<Bar>`）在归一前由 {@link #beanRootOf} 剥掉。
     */
    /**
     * `@Bean` 工厂方法的返回类型（`getBeanClassName` / `getResolvableType` 都拿不到时）。
     *
     * Boot 2.x 的 `ConfigurationClassBeanDefinition` 在 Bean 尚未实例化时可能：
     * <ul>
     *   <li>`getBeanClassName()` 返回 null（工厂方法声明的 Bean 本就没有类名）；</li>
     *   <li>`getResolvableType()` 解析不到，或解析成工厂类自身。</li>
     * </ul>
     * 于是只剩工厂方法名可用：在工厂类上按名找方法读返回类型。工厂类是
     * `getFactoryBeanName()` 指向的配置类（也可能是 CGLIB 增强类，按父类链找）。
     * 纯元数据反射，不触发任何实例化。
     */
    public static String factoryMethodReturnType(Object definition, String factoryMethod) {
        return factoryMethodReturnType(definition, factoryMethod, null);
    }

    /**
     * `@Bean` 工厂方法返回类型（可指定 BeanFactory 以解析工厂 Bean 名）。
     *
     * `getFactoryBeanName()` 返回的是**工厂 Bean 的名字**，不是类名
     * （实测 Boot 2.7：`"channelConfig"`），所以必须经 BeanFactory 再取一次
     * 该 Bean 的定义才能拿到工厂类。不传 beanFactory 时按类名直接尝试（兼容
     * 某些把类名写进 factoryBeanName 的实现）。
     */
    public static String factoryMethodReturnType(Object definition, String factoryMethod,
                                                 Object beanFactory) {
        if (factoryMethod == null || factoryMethod.isEmpty()) {
            return null;
        }
        Class<?> factory = resolveFactoryClass(definition, beanFactory);
        if (factory == null) {
            return null;
        }
        for (Class<?> c = factory; c != null && c != Object.class; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                if (m.getName().equals(factoryMethod) && m.getParameterTypes().length == 0) {
                    Class<?> ret = m.getReturnType();
                    if (ret == null || ret == void.class) {
                        return null;
                    }
                    // 泛型返回类型（如 List<Foo>）用泛型字符串，剥签名后归一更准
                    return m.getGenericReturnType().getTypeName();
                }
            }
        }
        return null;
    }

    private static Class<?> resolveFactoryClass(Object definition, Object beanFactory) {
        String factoryName = asText(invokeNoArg(definition, "getFactoryBeanName"));
        if (factoryName == null || factoryName.isEmpty()) {
            // 无 factoryBeanName：退化为「声明类」（@Configuration 类本身）
            Object source = invokeNoArg(definition, "getSource");
            return source == null ? null : source.getClass();
        }
        // 1) 当作 Bean 名，经 BeanFactory 取该 Bean 的定义 → 工厂类名
        String className = factoryName;
        if (beanFactory != null) {
            try {
                Method getDefinition = null;
                for (Method m : beanFactory.getClass().getMethods()) {
                    if ("getBeanDefinition".equals(m.getName())
                            && m.getParameterTypes().length == 1
                            && m.getParameterTypes()[0] == String.class) {
                        getDefinition = m;
                        break;
                    }
                }
                if (getDefinition != null) {
                    Object factoryDefinition = getDefinition.invoke(beanFactory, factoryName);
                    String resolved = factoryDefinition == null
                            ? null : asText(invokeNoArg(factoryDefinition, "getBeanClassName"));
                    if (resolved != null && !resolved.isEmpty()) {
                        className = resolved;
                    }
                }
            } catch (Throwable ignored) {
                // 工厂 Bean 名解析失败：退回把 factoryBeanName 当类名
            }
        }
        Class<?> factory = loadClassQuietly(className, definition.getClass().getClassLoader());
        if (factory != null) {
            return factory;
        }
        // 2) CGLIB / JDK 代理形态：剥代理后缀再试
        String stripped = stripProxySuffix(className);
        return stripped.equals(className)
                ? null : loadClassQuietly(stripped, definition.getClass().getClassLoader());
    }

    /** `Foo$$EnhancerBySpringCGLIB$$abc123` → `Foo`（也处理 `$Proxy12` 之外的常见形态）。 */
    private static String stripProxySuffix(String name) {
        int cglib = name.indexOf("$$EnhancerBySpringCGLIB$$");
        if (cglib > 0) {
            return name.substring(0, cglib);
        }
        int cglib2 = name.indexOf("$$EnhancerByCGLIB$$");
        if (cglib2 > 0) {
            return name.substring(0, cglib2);
        }
        return name;
    }

    private static Class<?> loadClassQuietly(String name, ClassLoader preferred) {
        try {
            return Class.forName(name, false, preferred == null
                    ? ClassClassifier.class.getClassLoader() : preferred);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static String resolvableType(Object definition) {
        Object type = invokeNoArg(definition, "getResolvableType");
        if (type == null) {
            return null;
        }
        Object cls = invokeNoArg(type, "resolve");
        // resolve() 可能是裸类型（泛型擦除），getTypeName() 保留参数化信息；
        // @Bean 返回类型优先用后者，剥签名后归一键才准确
        String typeName = asText(invokeNoArg(type, "getTypeName"));
        if (typeName != null && !typeName.isEmpty() && typeName.indexOf('<') > 0) {
            return typeName;
        }
        if (cls instanceof Class) {
            return ((Class<?>) cls).getName();
        }
        return typeName;
    }

    private static String asText(Object value) {
        return value == null ? null : String.valueOf(value);
    }

    /** BeanDefinition → 声明类名（beanClassName 优先，其次泛型解析）。 */
    private static String beanClassName(Object definition) {
        try {
            Method m = findNoArgMethod(definition.getClass(), "getBeanClassName");
            if (m != null) {
                Object v = m.invoke(definition);
                if (v instanceof String && !((String) v).isEmpty()) {
                    return (String) v;
                }
            }
        } catch (Throwable ignored) {
            // 继续尝试 resolvable type
        }
        try {
            Method m = findNoArgMethod(definition.getClass(), "getResolvableType");
            if (m != null) {
                Object type = m.invoke(definition);
                if (type != null) {
                    Method resolve = findNoArgMethod(type.getClass(), "resolve");
                    if (resolve != null) {
                        Object cls = resolve.invoke(type);
                        if (cls instanceof Class) {
                            return ((Class<?>) cls).getName();
                        }
                    }
                }
            }
        } catch (Throwable ignored) {
            // 字段注入型 Bean 无 beanClassName：留给 codeSource 判定
        }
        return null;
    }

    /** 在类**及其父类**上找无参方法（Spring 的 *BeanDefinition 多为子类实现）。 */
    private static Method findNoArgMethod(Class<?> type, String name) {
        Class<?> cursor = type;
        int depth = 0;
        while (cursor != null && cursor != Object.class && depth < 8) {
            try {
                Method m = cursor.getDeclaredMethod(name);
                m.setAccessible(true);
                return m;
            } catch (Throwable ignored) {
                // 尝试接口
                try {
                    Method m = cursor.getMethod(name);
                    return m;
                } catch (Throwable ignored2) {
                    cursor = cursor.getSuperclass();
                    depth++;
                }
            }
        }
        return null;
    }

    /** 无参 getter 的布尔字段读取（诊断用，供 Future 扩展）。 */
    static Object fieldValue(Object target, String field) {
        try {
            Field f = target.getClass().getDeclaredField(field);
            f.setAccessible(true);
            return f.get(target);
        } catch (Throwable t) {
            return null;
        }
    }

    static boolean isPublicClass(Class<?> cls) {
        return Modifier.isPublic(cls.getModifiers());
    }

    private ClassClassifier() {
    }
}