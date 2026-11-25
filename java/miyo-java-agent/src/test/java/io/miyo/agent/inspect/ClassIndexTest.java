package io.miyo.agent.inspect;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.io.File;
import java.lang.instrument.Instrumentation;
import java.lang.reflect.Proxy;
import java.net.URL;
import java.net.URLClassLoader;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.junit.After;
import org.junit.Test;

/**
 * 类索引 / 列表查询测试（真实 ClassIndex + 真实 Class 对象）。
 *
 * Instrumentation 用一个「返回当前类加载器可见的类」的替身实现：
 * 索引关心的语义是「一批 Class 对象 + 增量」，与 JDK 内部实现无关。
 */
public class ClassIndexTest {

    private final List<ClassLoader> createdLoaders = new ArrayList<ClassLoader>();

    @org.junit.Before
    public void before() {
        // surefire 会把测试类打进 jar：显式声明本测试的夹具是「应用目录」
        ClassOrigin.setTestApplicationPrefixForTests("io.miyo.agent.inspect.Fixtures");
    }

    @After
    public void cleanup() throws Exception {
        ClassOrigin.setTestApplicationPrefixForTests(null);
        for (ClassLoader loader : createdLoaders) {
            ((URLClassLoader) loader).close();
        }
        ClassOrigin.setLaunchInfoForTests(null);
    }

    // ------------------------------------------------------------------
    // 1) 增量刷新语义
    // ------------------------------------------------------------------

    @Test
    public void refreshIsIncrementalAndTracksAdditions() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String.class, ArrayList.class, Fixtures.OrderService.class));

        ClassIndex.RefreshResult first = index.refresh(new String[0],
                Collections.<String>emptySet(), null, false);
        assertEquals(3, first.total);
        assertEquals(3, first.added);

        // 第二轮：同样的类集合 → 零新增（缓存命中）
        ClassIndex.RefreshResult second = index.refresh(new String[0],
                Collections.<String>emptySet(), null, false);
        assertEquals(3, second.total);
        assertEquals(0, second.added);

        // 新增一个类 → 只分类这一个
        inst.set(classes(String.class, ArrayList.class, Fixtures.OrderService.class,
                Fixtures.OrderController.class));
        ClassIndex.RefreshResult third = index.refresh(new String[0],
                Collections.<String>emptySet(), null, false);
        assertEquals(4, third.total);
        assertEquals(1, third.added);
        assertEquals(0, third.removed);
    }

    @Test
    public void removedClassesAreSweptFromIndex() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String.class, ArrayList.class));
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);

        inst.set(classes(String.class));
        ClassIndex.RefreshResult result = index.refresh(new String[0],
                Collections.<String>emptySet(), null, false);
        assertEquals(1, result.total);
        assertEquals(1, result.removed);
        assertNull(index.find(ArrayList.class));
        assertNotNull(index.find(String.class));
    }

    // ------------------------------------------------------------------
    // 2) 统计
    // ------------------------------------------------------------------

    @Test
    public void statsSeparateCategories() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        inst.set(classes(String.class, Fixtures.OrderService.class,
                Fixtures.OrderController.class));
        ClassIndex index = new ClassIndex(inst.proxy());
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);

        ClassIndex.Stats stats = index.stats();
        assertEquals(1, stats.of("jdk_internal"));
        assertEquals(2, stats.of("application"));
        assertEquals(3, stats.total);
        assertEquals(3, stats.loadedClassCount);
    }

    @Test
    public void statsCountRolesForBusinessClasses() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        inst.set(classes(Fixtures.OrderService.class, Fixtures.OrderController.class));
        ClassIndex index = new ClassIndex(inst.proxy());
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);

        ClassIndex.Stats stats = index.stats();
        assertEquals(Integer.valueOf(1), stats.byRole.get("service"));
        assertEquals(Integer.valueOf(1), stats.byRole.get("controller"));
        // JDK 类不参与角色统计
        assertTrue(stats.byRole.size() == 2);
    }

    // ------------------------------------------------------------------
    // 3) 排序 / 分页 / 过滤
    // ------------------------------------------------------------------

    @Test
    public void defaultOrderSortsBusinessBeforeJdk() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String.class, Fixtures.OrderService.class));
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);

        List<ClassIndex.Entry> sorted = index.snapshot();
        Collections.sort(sorted, ClassIndex.DEFAULT_COMPARATOR);
        assertEquals(Fixtures.OrderService.class.getName(), sorted.get(0).name);
        assertEquals(String.class.getName(), sorted.get(1).name);
    }

    @Test
    public void javaLangProxyIsGenerated() throws Exception {
        ClassLoader cl = new URLClassLoader(new URL[0], getClass().getClassLoader());
        createdLoaders.add(cl);
        Object proxy = Proxy.newProxyInstance(cl, new Class<?>[]{Runnable.class},
                (p, m, args) -> null);
        ClassOrigin.Source source = ClassOrigin.resolve(proxy.getClass());
        ClassClassifier.Verdict verdict = ClassClassifier.classify(proxy.getClass(), source,
                Fixtures.context());
        assertEquals(ClassClassifier.Category.GENERATED, verdict.category);
    }

    @Test
    public void bridgeClassMatchesBeanName() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        inst.set(classes(Fixtures.OrderService.class));
        ClassIndex index = new ClassIndex(inst.proxy());
        // 方向：实现类全名 → Bean 名（与 ContextDiscovery.beanNamesByClass 一致）
        Map<String, String> beanNames = new LinkedHashMap<String, String>();
        beanNames.put(Fixtures.OrderService.class.getName(), "orderService");
        Set<String> beans = new LinkedHashSet<String>();
        beans.add(Fixtures.OrderService.class.getName());
        index.refresh(new String[0], beans, beanNames, true);

        ClassIndex.Entry entry = index.find(Fixtures.OrderService.class);
        assertNotNull(entry);
        assertEquals(ClassClassifier.Category.SPRING_BEAN, entry.category());
        assertEquals("orderService", entry.beanName);
        assertTrue(ClassIndex.nameMatches(entry, "orderservice"));
        assertTrue(ClassIndex.nameMatches(entry, "fixtures"));
        assertTrue(ClassIndex.nameMatches(entry, null));
        assertTrue(!ClassIndex.nameMatches(entry, "zzz"));
    }

    @Test
    public void beanRoleOnlyAppliesToApplicationOwnedClasses() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String.class, Fixtures.OrderService.class));
        Set<String> beans = new LinkedHashSet<String>();
        beans.add(String.class.getName());
        index.refresh(new String[0], beans, null, true);
        // String 的 codeSource 是 jrt: → JDK 判定优先于 Bean 角色
        assertEquals(ClassClassifier.Category.JDK_INTERNAL,
                index.find(String.class).category());
    }

    @Test
    public void clearDropsEverything() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String.class));
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);
        assertEquals(1, index.size());
        index.clear();
        assertEquals(0, index.size());
        assertEquals(0, index.stats().total);
    }

    @Test
    public void launchSummaryExposesEvidence() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        inst.set(classes(String.class));
        ClassIndex index = new ClassIndex(inst.proxy());
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);

        Map<String, Object> summary = index.launchSummary();
        assertTrue(summary.containsKey("launch"));
        assertTrue(summary.containsKey("loaded_class_count"));
        assertEquals(1, summary.get("loaded_class_count"));
        assertTrue(summary.containsKey("last_refresh"));
    }

    @Test
    public void arrayClassesAreExcludedFromStats() throws Exception {
        FakeInstrumentation inst = new FakeInstrumentation();
        ClassIndex index = new ClassIndex(inst.proxy());
        inst.set(classes(String[].class, String.class));
        index.refresh(new String[0], Collections.<String>emptySet(), null, false);
        // String[] 计入索引（Instrumentation 会返回它），但不计入统计
        assertEquals(2, index.size());
        assertEquals(1, index.stats().total);
    }

    // ------------------------------------------------------------------
    // 4) 业务包前缀 → 应用类
    // ------------------------------------------------------------------

    @Test
    public void normalizePrefixesDedupesAndStripsWildcards() {
        List<String> prefixes = ClassClassifier.normalizePrefixes(
                java.util.Arrays.asList("io.miyo.agent.inspect", " io.miyo.agent.inspect.* "));
        assertEquals(1, prefixes.size());
        assertEquals("io.miyo.agent.inspect", prefixes.get(0));
    }

    // ------------------------------------------------------------------

    static Class<?>[] classes(Class<?>... values) {
        return values;
    }

    /**
     * Instrumentation 替身：Java 8 源码级别编译时，Instrumentation 的
     * Module 相关方法是「抽象」的（接口演进导致），无法静态实现——
     * 用动态代理按方法名应答，其余方法返回默认值。
     */

}
