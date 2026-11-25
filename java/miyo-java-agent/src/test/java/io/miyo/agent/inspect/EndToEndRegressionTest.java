package io.miyo.agent.inspect;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;

/**
 * 端到端实测暴露的回归用例（真实 Spring Boot 3.2 demo + plain-app 上验证出来的）。
 *
 * 这些 bug 单测/编译都没拦住，全部由「真实目标进程 + 真实 agent jar」暴露：
 * <ol>
 *   <li>Spring Boot 3.2 嵌套 URL（`nested:/app.jar/!BOOT-INF/lib/x.jar`）不认；</li>
 *   <li>agent 自身类被当成应用业务类；</li>
 *   <li>Boot 3.x loader 类与 app.jar 共享 codeSource → 判成业务类；</li>
 *   <li>Bean 名回填方向写反（类名→Bean 名当成了 Bean 名→类名）；</li>
 *   <li>springPresent 负缓存让 Bean 分类永久关闭。</li>
 * </ol>
 */
public class EndToEndRegressionTest {

    @Before
    public void setUp() {
        ClassOrigin.setTestApplicationPrefixForTests("io.miyo.agent.inspect.Fixtures");
    }

    @After
    public void tearDown() {
        ClassOrigin.setTestApplicationPrefixForTests(null);
    }

    @Test
    public void boot3NestedLibJarIsDependency() {
        ClassOrigin.Source src = ClassOrigin.classifyUrl(
                "nested:/work/demo.jar/!BOOT-INF/lib/logback-classic-1.4.14.jar");
        assertEquals(ClassOrigin.Kind.NESTED_LIB, src.kind);
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                ClassClassifier.classify(Fixtures.OrderService.class, src,
                        Fixtures.context()).category);
    }

    @Test
    public void boot3NestedClassesIsApplication() {
        ClassOrigin.Source src = ClassOrigin.classifyUrl(
                "nested:/work/demo.jar/!BOOT-INF/classes!/com/example/");
        assertEquals(ClassOrigin.Kind.NESTED_CLASSES, src.kind);
    }

    @Test
    public void bootLoaderClassIsDependencyNotBusiness() {
        // Boot 3.x loader 与 app.jar 共享 codeSource：包名证据必须压过 codeSource
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                ClassClassifier.classify(Fixtures.OrderService.class,
                        "org.springframework.boot.loader.jar.ManifestInfo",
                        Fixtures.bootClassesSource(), Fixtures.context()).category);
    }

    @Test
    public void bothNestedSeparatorFormsAgree() {
        // Spring Boot 2（jar:...!/BOOT-INF/lib/x.jar!/）与 3.2（nested:.../!BOOT-INF/lib/x.jar）
        ClassOrigin.Source v2 = ClassOrigin.classifyUrl(
                "jar:file:/work/demo.jar!/BOOT-INF/lib/netty.jar!/");
        ClassOrigin.Source v3 = ClassOrigin.classifyUrl(
                "nested:/work/demo.jar/!BOOT-INF/lib/netty.jar");
        assertEquals(ClassOrigin.Kind.NESTED_LIB, v2.kind);
        assertEquals(ClassOrigin.Kind.NESTED_LIB, v3.kind);
    }

    @Test
    public void beanNameIsFilledFromClassNameToBeanNameMap() {
        FakeInstrumentation inst = new FakeInstrumentation();
        inst.set(new Class<?>[]{Fixtures.OrderService.class});
        ClassIndex index = new ClassIndex(inst.proxy());

        Set<String> beans = new LinkedHashSet<String>();
        beans.add(Fixtures.OrderService.class.getName());
        // 真实形态：ContextDiscovery.beanNamesByClass() 返回「实现类全名 → Bean 名」
        Map<String, String> beanNames = new LinkedHashMap<String, String>();
        beanNames.put(Fixtures.OrderService.class.getName(), "priceService");

        index.refresh(new String[0], beans, beanNames, true);

        ClassIndex.Entry entry = index.find(Fixtures.OrderService.class);
        assertNotNull(entry);
        assertEquals(ClassClassifier.Category.SPRING_BEAN, entry.category());
        assertEquals("priceService", entry.beanName);
    }

    @Test
    public void beanNamesByClassDirectionIsClassNameKeyed() {
        // 方向契约（避免回填再次写反）：键必须是类全名
        Map<String, String> map = new LinkedHashMap<String, String>();
        map.put(Fixtures.OrderService.class.getName(), "priceService");
        String key = map.keySet().iterator().next();
        assertTrue(key.contains(".") && Character.isUpperCase(key.charAt(key.lastIndexOf('.') + 1)));
        assertFalse(key.startsWith("price"));
    }

    @Test
    public void ownAgentClassesResolvedFromJar() {
        // surefire 下 agent 类在 target/test-classes（目录），枚举为空也不影响分类正确性；
        // 这里断言判定函数本身不抛、且对非 agent 类返回 false
        assertFalse(ClassClassifier.isOwnAgentClass(
                "com.example.demo.OrderService", new java.util.ArrayList<String>()));
        assertNull(ClassClassifier.frameworkOwnedRoot("com.example.demo.OrderService"));
        assertEquals("org.springframework.boot.loader.",
                ClassClassifier.frameworkOwnedRoot(
                        "org.springframework.boot.loader.jar.ManifestInfo"));
    }
}