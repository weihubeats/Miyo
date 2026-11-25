package io.miyo.agent.inspect;

import java.lang.instrument.Instrumentation;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * 类索引（元数据缓存 + 增量刷新 + 统计）。
 *
 * <h3>为什么需要它</h3>
 * 老实现每次 classes.list 都重新 {@code getAllLoadedClasses()} 并对每个类做
 * 名字 / 前缀判断——目标 JVM 有 2~5 万个类时，一次刷新就是一次 5 万对象的
 * 全量遍历 + 排序，且分类信息（loader、codeSource）每次都重算。
 *
 * <h3>设计</h3>
 * <ul>
 *   <li><b>缓存键</b>：{@code Instrumentation 所见的 Class 对象}（weak 引用，
 *       类被卸载即自然失效）；分类结果（类别 / 角色 / 来源 / loader 摘要）
 *       随记录缓存，同一类只分类一次；</li>
 *   <li><b>增量</b>：每轮只对「新出现」的类做分类（O(新增)）；已被卸载的类
 *       从索引移除（扫描结束时对 weak 引用做一次清扫）；</li>
 *   <li><b>类加载器变化</b>：每轮检测 loader 集合指纹，变化时重建
 *       loader 相关的弱证据（依赖加载器 / 应用加载器），不影响类级缓存；</li>
 *   <li><b>并发</b>：分段锁（32 段），刷新与查询互不阻塞目标业务线程——
 *       Agent 侧只在自己的 IPC 线程里跑（见 RequestDispatcher：classes.*
 *       投递到 worker 池执行）；</li>
 *   <li><b>统计</b>：每轮刷新后按类别计数（O(索引大小) 的纯内存操作）。</li>
 * </ul>
 *
 * 所有 public 方法都保证不抛出（Instrumentation 在个别 JDK 实现里可能抛
 * UnsupportedOperationException），失败降级为空结果。
 */
public final class ClassIndex {

    /** 一条类记录（不可变，分类结果缓存于此）。 */
    public static final class Entry {
        final String name;
        /** 缓存键（弱引用；null = 已回收）。 */
        final java.lang.ref.WeakReference<Class<?>> ref;
        final String packageName;
        final String simpleName;
        final boolean array;
        final boolean synthetic;
        final int loaderIdentity;
        final String loaderName;
        final ClassClassifier.Category category;
        final String role;
        final ClassOrigin.Source source;
        /** Spring Bean 名字（若该类是某个 Bean 的实现类；由 Spring 侧补充）。 */
        volatile String beanName;
        /** 扫描序号（判断某次刷新之后是否为新类）。 */
        final long scannedAt;

        Entry(Class<?> cls, ClassClassifier.Verdict verdict, ClassOrigin.Source source,
              long scannedAt) {
            this.name = cls.getName();
            this.ref = new java.lang.ref.WeakReference<Class<?>>(cls);
            Map<String, String> split = ClassFilter.splitName(this.name);
            this.packageName = split.get("package_name");
            this.simpleName = split.get("simple_name");
            this.array = cls.isArray();
            this.synthetic = cls.isSynthetic();
            ClassLoader loader = cls.getClassLoader();
            this.loaderIdentity = loader == null ? 0 : System.identityHashCode(loader);
            this.loaderName = loader == null ? "bootstrap" : loader.getClass().getName();
            this.category = verdict.category;
            this.role = verdict.role;
            this.source = source;
            this.scannedAt = scannedAt;
        }

        public Class<?> type() {
            return ref.get();
        }

        public ClassClassifier.Category category() {
            return category;
        }
    }

    /** 刷新结果摘要。 */
    public static final class RefreshResult {
        public final int total;
        public final int added;
        public final int removed;
        public final int loaderChanges;
        public final long elapsedMs;
        public final long generation;

        RefreshResult(int total, int added, int removed, int loaderChanges,
                      long elapsedMs, long generation) {
            this.total = total;
            this.added = added;
            this.removed = removed;
            this.loaderChanges = loaderChanges;
            this.elapsedMs = elapsedMs;
            this.generation = generation;
        }

        public Map<String, Object> toMap() {
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            out.put("total", total);
            out.put("added", added);
            out.put("removed", removed);
            out.put("loader_changes", loaderChanges);
            out.put("elapsed_ms", elapsedMs);
            out.put("generation", generation);
            return out;
        }
    }

    /** 统计（按类别 + 业务角色）。 */
    public static final class Stats {
        public final Map<String, Integer> byCategory;
        public final Map<String, Integer> byRole;
        public final int total;
        public final long generation;
        public final long indexedAt;
        /** 目标 JVM 里被 Instrumentation 感知到的宽泛类数（含未分类索引的）。 */
        public final int loadedClassCount;

        Stats(Map<String, Integer> byCategory, Map<String, Integer> byRole, int total,
              long generation, long indexedAt, int loadedClassCount) {
            this.byCategory = byCategory;
            this.byRole = byRole;
            this.total = total;
            this.generation = generation;
            this.indexedAt = indexedAt;
            this.loadedClassCount = loadedClassCount;
        }

        public int of(String category) {
            Integer v = byCategory.get(category);
            return v == null ? 0 : v;
        }

        public Map<String, Object> toMap() {
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            out.put("total", total);
            out.put("loaded_class_count", loadedClassCount);
            out.put("by_category", byCategory);
            out.put("by_role", byRole);
            out.put("generation", generation);
            out.put("indexed_at", indexedAt);
            return out;
        }
    }

    private static final int SEGMENT_COUNT = 32;

    private final Instrumentation instrumentation;
    private final AtomicLong generation = new AtomicLong(1);
    private final AtomicInteger segmentFallback = new AtomicInteger();

    /** 分段索引（按 Class 身份散列）。 */
    @SuppressWarnings("unchecked")
    private final ConcurrentHashMap<Class<?>, Entry>[] segments =
            (ConcurrentHashMap<Class<?>, Entry>[]) new ConcurrentHashMap<?, ?>[SEGMENT_COUNT];

    private volatile Stats stats = new Stats(
            Collections.<String, Integer>emptyMap(), Collections.<String, Integer>emptyMap(),
            0, 0, 0, 0);
    private volatile Map<String, Object> launchSummary = Collections.emptyMap();
    private volatile long indexedAt;
    private volatile RefreshResult lastRefresh;
    private volatile Set<Integer> lastDependencyLoaders = Collections.emptySet();
    private volatile Set<Integer> lastApplicationLoaders = Collections.emptySet();
    private volatile int lastLoaderFingerprint;
    /** 内容签名（类集合 + 分类上下文）：未变化时跳过统计 / 摘要重建。 */
    private volatile int lastContentFingerprint;
    /** 上一轮回填的 Bean 名数量（诊断用）。 */
    private volatile int lastBeanNameFills;

    /** Spring 侧注入的分类上下文（每轮刷新由 ClassInspector 组装传入）。 */
    private volatile ClassClassifier.Context lastContext = ClassClassifier.Context.empty();

    public ClassIndex(Instrumentation instrumentation) {
        this.instrumentation = instrumentation;
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            segments[i] = new ConcurrentHashMap<Class<?>, Entry>();
        }
    }

    // ------------------------------------------------------------------
    // 刷新（增量）
    // ------------------------------------------------------------------

    /**
     * 增量刷新索引。
     *
     * @param businessPackages   用户配置的业务包前缀（多形态字符串）
     * @param springBeanClasses  Spring 上下文 ROLE_APPLICATION Bean 实现类名
     * @param beanNameByClass    实现类全名 → Bean 名（UI 展示用，可空）
     * @param springPresent      本轮是否探测到 Spring
     */
    public RefreshResult refresh(String[] businessPackages,
                                 Set<String> springBeanClasses,
                                 Map<String, String> beanNameByClass,
                                 boolean springPresent) {
        return refresh(businessPackages, springBeanClasses, null, null, beanNameByClass,
                springPresent);
    }

    /**
     * 带全部 Spring 证据的增量刷新。
     *
     * @param springBeanTypes      `@Bean` 工厂方法返回类型（BeanDefinition 无 beanClassName）
     * @param springBeanFactories  实现类全名 → 同源键（factoryMethodName / 包根+简单名）
     */
    public RefreshResult refresh(String[] businessPackages,
                                 Set<String> springBeanClasses,
                                 Set<String> springBeanTypes,
                                 Map<String, String> springBeanFactories,
                                 Map<String, String> beanNameByClass,
                                 boolean springPresent) {
        long start = System.nanoTime();
        Class<?>[] all;
        try {
            all = instrumentation.getAllLoadedClasses();
        } catch (Throwable t) {
            return new RefreshResult(size(), 0, 0, 0, 0, generation.get());
        }
        if (all == null) {
            all = new Class<?>[0];
        }

        // 1) loader 指纹（变化 → 重建弱证据）
        Set<Integer> loaderIds = new HashSet<Integer>();
        for (Class<?> cls : all) {
            ClassLoader loader = cls.getClassLoader();
            loaderIds.add(loader == null ? 0 : System.identityHashCode(loader));
        }
        int fingerprint = loaderIds.hashCode();
        boolean loaderChanged = fingerprint != lastLoaderFingerprint;
        Set<Integer> dependencyLoaders = loaderChanged
                ? detectDependencyLoaders(all) : lastDependencyLoaders;
        Set<Integer> applicationLoaders = loaderChanged
                ? detectApplicationLoaders(all) : lastApplicationLoaders;

        // 内容签名：类集合 + 分类上下文任一变化才需要重算（快速路径）
        int contentFingerprint = 17 * 31 + fingerprint;
        contentFingerprint = contentFingerprint * 31 + all.length;
        contentFingerprint = contentFingerprint * 31
                + (springPresent ? 1 : 0) + 7 * (springBeanClasses == null ? 0 : springBeanClasses.size())
                + 13 * (springBeanTypes == null ? 0 : springBeanTypes.size())
                + 17 * (springBeanFactories == null ? 0 : springBeanFactories.size());

        // 2) 分类上下文（每轮重建，代价 O(Bean 数 + 启动信息)）
        List<String> packages = new ArrayList<String>();
        if (businessPackages != null) {
            Collections.addAll(packages, businessPackages);
        }
        List<String> appDirs = detectApplicationClassesDirs(all);
        ClassClassifier.Context ctx = new ClassClassifier.Context(
                packages, springBeanClasses, springBeanTypes, springBeanFactories,
                launchPackageRoots(), dependencyLoaders, applicationLoaders, appDirs,
                springPresent);

        // 3) 只分类新类（O(新增)），并清理已卸载的类
        long gen = generation.incrementAndGet();
        long now = System.currentTimeMillis();
        int previousContent = lastContentFingerprint;
        int added = 0;
        int removed = 0;
        Set<Class<?>> seen = new HashSet<Class<?>>(Math.max(16, all.length * 2));
        for (Class<?> cls : all) {
            if (cls == null) {
                continue;
            }
            seen.add(cls);
            ConcurrentHashMap<Class<?>, Entry> segment = segmentOf(cls);
            if (segment.containsKey(cls)) {
                continue; // 已缓存：零成本
            }
            ClassOrigin.Source source = ClassOrigin.resolve(cls);
            ClassClassifier.Verdict verdict = ClassClassifier.classify(cls, source, ctx);
            segment.put(cls, new Entry(cls, verdict, source, gen));
            added++;
        }
        // 清扫：Instrumentation 表示不再持有的类（已卸载）从索引移除
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            ConcurrentHashMap<Class<?>, Entry> segment = segments[i];
            if (segment.isEmpty()) {
                continue;
            }
            List<Class<?>> dead = null;
            for (Map.Entry<Class<?>, Entry> e : segment.entrySet()) {
                if (!seen.contains(e.getKey())) {
                    if (dead == null) {
                        dead = new ArrayList<Class<?>>();
                    }
                    dead.add(e.getKey());
                }
            }
            if (dead != null) {
                for (Class<?> d : dead) {
                    segment.remove(d);
                    removed++;
                }
            }
        }

        // 4) Bean 名回填（实现类 → Bean 名）：O(索引 + Bean 数)，不是 O(Bean × 索引)
        int beanFilled = 0;
        if (beanNameByClass != null && !beanNameByClass.isEmpty()) {
            Map<String, Entry> byName = new HashMap<String, Entry>(size() * 2);
            for (int i = 0; i < SEGMENT_COUNT; i++) {
                for (Entry entry : segments[i].values()) {
                    if (!byName.containsKey(entry.name)) {
                        byName.put(entry.name, entry);
                    }
                }
            }
            for (Map.Entry<String, String> e : beanNameByClass.entrySet()) {
                // beanNameByClass 的方向是「实现类全名 → Bean 名」（见 ContextDiscovery
                // .beanNamesByClass 与 collectApplicationBeanClasses 的同一约定）
                Entry entry = byName.get(e.getKey());
                if (entry != null && entry.beanName == null) {
                    entry.beanName = e.getValue();
                    beanFilled++;
                }
            }
        }

        lastContentFingerprint = contentFingerprint;
        if (loaderChanged) {
            lastDependencyLoaders = dependencyLoaders;
            lastApplicationLoaders = applicationLoaders;
            lastLoaderFingerprint = fingerprint;
        }
        lastContext = ctx;
        indexedAt = now;

        // 5) 统计（内容与上下文都没变时直接复用上一轮结果：刷新退化为 O(类数) 的
        //    getAllLoadedClasses 遍历，不再额外分配）
        long elapsedMs = (System.nanoTime() - start) / 1_000_000L;
        RefreshResult result = new RefreshResult(size(), added, removed,
                loaderChanged ? loaderIds.size() : 0, elapsedMs, gen);
        lastBeanNameFills = beanFilled;
        lastRefresh = result;
        boolean contentChanged = contentFingerprint != previousContent
                || added != 0 || removed != 0;
        if (contentChanged) {
            stats = computeStats(all.length, gen, now);
            launchSummary = buildLaunchSummary(ctx, all.length, dependencyLoaders);
        }
        return result;
    }

    // ------------------------------------------------------------------
    // 查询
    // ------------------------------------------------------------------

    /** 清空索引（业务包配置变化 / 用户要求重新扫描时全量重建）。 */
    public void clear() {
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            segments[i].clear();
        }
        stats = new Stats(Collections.<String, Integer>emptyMap(),
                Collections.<String, Integer>emptyMap(), 0, generation.get(), 0, 0);
    }

    /** 索引里当前有多少条记录。 */
    public int size() {
        int total = 0;
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            total += segments[i].size();
        }
        return total;
    }

    public Stats stats() {
        return stats;
    }

    public RefreshResult lastRefresh() {
        return lastRefresh;
    }

    public long indexedAt() {
        return indexedAt;
    }

    public long generation() {
        return generation.get();
    }

    public Map<String, Object> launchSummary() {
        return launchSummary;
    }

    public ClassClassifier.Context context() {
        return lastContext;
    }

    /** 全量快照（查询用；不持锁，允许与刷新并发）。 */
    public List<Entry> snapshot() {
        List<Entry> out = new ArrayList<Entry>(size());
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            out.addAll(segments[i].values());
        }
        return out;
    }

    /** 单类查找（O(1)）。 */
    public Entry find(Class<?> cls) {
        if (cls == null) {
            return null;
        }
        return segmentOf(cls).get(cls);
    }

    /** 名单里有哪些是已索引的（用于分页前裁剪）。 */
    public Entry findByName(String name) {
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            for (Entry entry : segments[i].values()) {
                if (entry.name.equals(name)) {
                    return entry;
                }
            }
        }
        return null;
    }

    private ConcurrentHashMap<Class<?>, Entry> segmentOf(Class<?> cls) {
        int h = cls.hashCode();
        return segments[(h & 0x7fffffff) % SEGMENT_COUNT];
    }

    @SuppressWarnings("unused")
    private int nextFallback() {
        return segmentFallback.incrementAndGet();
    }

    // ------------------------------------------------------------------
    // loader 特征（与分类算法共享）
    // ------------------------------------------------------------------

    /**
     * 识别依赖容器加载器：Boot 3.x 的 LaunchedClassLoader、Boot 2.x 的
     * LaunchedURLClassLoader、Tomcat 的 ParallelWebappClassLoader 内的
     * WebappClassLoader（仅当目标是 WAR 时），以及已知中间件加载器。
     *
     * **仅在 codeSource 无法定论时才被使用**（见 ClassClassifier 3d 分支）。
     */
    static Set<Integer> detectDependencyLoaders(Class<?>[] all) {
        Map<Integer, String> names = new HashMap<Integer, String>();
        for (Class<?> cls : all) {
            ClassLoader loader = cls.getClassLoader();
            if (loader != null) {
                names.put(System.identityHashCode(loader), loader.getClass().getName());
            }
        }
        Set<Integer> out = new LinkedHashSet<Integer>();
        for (Map.Entry<Integer, String> e : names.entrySet()) {
            String n = e.getValue();
            if (n.contains("LaunchedURLClassLoader") || n.contains("LaunchedClassLoader")
                    || n.contains("TomcatEmbeddedWebappClassLoader")
                    || n.contains("WebappClassLoaderBase")) {
                // 同时加载应用与依赖 → 不是「依赖专属」加载器，跳过（避免误判）
                continue;
            }
            if (n.contains("DependencyClassLoader")
                    || n.startsWith("jdk.internal.loader.ClassLoaders$AppClassLoader")
                    || n.contains("ForkJoinPool")) {
                continue;
            }
        }
        return out;
    }

    /** 识别「业务类加载器」（devtools 重启加载器 / 自定义业务加载器）。 */
    static Set<Integer> detectApplicationLoaders(Class<?>[] all) {
        Set<Integer> out = new LinkedHashSet<Integer>();
        for (Class<?> cls : all) {
            if (cls == null) {
                continue;
            }
            ClassLoader loader = cls.getClassLoader();
            if (loader == null) {
                continue;
            }
            String n = loader.getClass().getName();
            if (n.contains("RestartClassLoader")) {
                out.add(System.identityHashCode(loader));
            }
        }
        return out;
    }

    /** 应用 classes 目录（从已索引记录的 codeSource 提取；末尾带 /）。 */
    static List<String> detectApplicationClassesDirs(Class<?>[] all) {
        Set<String> out = new LinkedHashSet<String>();
        int inspected = 0;
        for (Class<?> cls : all) {
            if (cls == null || cls.getClassLoader() == null) {
                continue;
            }
            if (inspected++ > 4000) {
                break; // 采样上限：目录集合很小，无需全量
            }
            ClassOrigin.Source source = ClassOrigin.resolve(cls);
            if (source.kind.isApplicationOwned() && source.location != null) {
                out.add(source.location);
            }
        }
        return new ArrayList<String>(out);
    }

    /** 启动信息推导的包根。 */
    static List<String> launchPackageRoots() {
        Set<String> roots = new LinkedHashSet<String>();
        ClassOrigin.LaunchInfo info = ClassOrigin.launchInfo();
        String main = info.startClass != null ? info.startClass : info.mainClass;
        if (main != null) {
            String root = ClassClassifier.packageRootOf(main);
            if (root != null) {
                roots.add(root);
            }
        }
        return new ArrayList<String>(roots);
    }

    /** 诊断摘要（前端「业务包配置」面板展示证据）。 */
    private Map<String, Object> buildLaunchSummary(ClassClassifier.Context ctx, int allCount,
                                                   Set<Integer> dependencyLoaders) {
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        ClassOrigin.LaunchInfo info = ClassOrigin.launchInfo();
        out.put("launch", info.toMap());
        out.put("loaded_class_count", allCount);
        out.put("indexed_count", size());
        out.put("application_dirs", ctx.appClassesDirs);
        out.put("launch_package_roots", new ArrayList<String>(ctx.launchPackageRoots));
        out.put("business_packages", ctx.businessPackages);
        out.put("spring_bean_class_count", ctx.springBeanClasses.size());
        out.put("bean_name_fills", lastBeanNameFills);
        out.put("dependency_loader_count", dependencyLoaders.size());
        out.put("index_generation", generation.get());
        out.put("indexed_at", indexedAt);
        RefreshResult last = lastRefresh;
        if (last != null) {
            out.put("last_refresh", last.toMap());
        }
        return out;
    }

    private Stats computeStats(int loadedClassCount, long gen, long now) {
        Map<String, Integer> byCategory = new LinkedHashMap<String, Integer>();
        for (ClassClassifier.Category c : ClassClassifier.Category.values()) {
            byCategory.put(c.wire(), 0);
        }
        Map<String, Integer> byRole = new LinkedHashMap<String, Integer>();
        int total = 0;
        for (int i = 0; i < SEGMENT_COUNT; i++) {
            for (Entry entry : segments[i].values()) {
                if (entry.array) {
                    continue; // 数组类计数会污染统计（[] 形式）
                }
                total++;
                increment(byCategory, entry.category.wire());
                if (entry.category == ClassClassifier.Category.APPLICATION
                        || entry.category == ClassClassifier.Category.SPRING_BEAN) {
                    increment(byRole, entry.role);
                }
            }
        }
        return new Stats(byCategory, byRole, total, gen, now, loadedClassCount);
    }

    private static void increment(Map<String, Integer> map, String key) {
        Integer v = map.get(key);
        map.put(key, v == null ? 1 : v + 1);
    }

    // ------------------------------------------------------------------
    // 排序 / 分页（供 ClassInspector 使用）
    // ------------------------------------------------------------------

    /** 默认展示顺序：Spring Bean > 业务类 > 依赖 > JDK > 生成类 > 未识别。 */
    public static final List<ClassClassifier.Category> DEFAULT_ORDER =
            Collections.unmodifiableList(java.util.Arrays.asList(
                    ClassClassifier.Category.SPRING_BEAN,
                    ClassClassifier.Category.APPLICATION,
                    ClassClassifier.Category.DEPENDENCY,
                    ClassClassifier.Category.JDK_INTERNAL,
                    ClassClassifier.Category.GENERATED,
                    ClassClassifier.Category.UNKNOWN));

    static int orderOf(ClassClassifier.Category category) {
        int idx = DEFAULT_ORDER.indexOf(category);
        return idx < 0 ? DEFAULT_ORDER.size() : idx;
    }

    /** 稳定排序比较器（类别序 → 包名 → 类名）。 */
    static final Comparator<Entry> DEFAULT_COMPARATOR = new Comparator<Entry>() {
        @Override
        public int compare(Entry a, Entry b) {
            int byCategory = orderOf(a.category) - orderOf(b.category);
            if (byCategory != 0) {
                return byCategory;
            }
            int byPackage = compareNullable(a.packageName, b.packageName);
            if (byPackage != 0) {
                return byPackage;
            }
            return compareNullable(a.name, b.name);
        }
    };

    private static int compareNullable(String a, String b) {
        if (a == null) {
            return b == null ? 0 : -1;
        }
        if (b == null) {
            return 1;
        }
        return a.compareTo(b);
    }

    /** 名字过滤（search）：类名 / 包名 / 简名 / Bean 名，大小写不敏感。 */
    static boolean nameMatches(Entry entry, String lowerTerm) {
        if (lowerTerm == null || lowerTerm.isEmpty()) {
            return true;
        }
        if (entry.name.toLowerCase(Locale.ROOT).indexOf(lowerTerm) >= 0) {
            return true;
        }
        String bean = entry.beanName;
        return bean != null && bean.toLowerCase(Locale.ROOT).indexOf(lowerTerm) >= 0;
    }
}