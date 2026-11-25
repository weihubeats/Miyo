package io.miyo.agent.inspect;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.io.ByteArrayInputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.junit.Test;

/**
 * 业务类分类算法单测（真实实现，不 mock 判定逻辑）。
 *
 * 覆盖：codeSource 净判定（目录 / fat-jar classes / fat-jar lib / jrt / 依赖仓库）、
 * 生成类识别、多形态包前缀、业务子类型、弱证据降级链、launch 信息解析、
 * 排序与名字过滤、缓存条目语义。
 */
public class ClassClassifierTest {

    // ------------------------------------------------------------------
    // 1) codeSource 净判定
    // ------------------------------------------------------------------

    @Test
    public void applicationClassesDirIsApplication() {
        assertEquals(ClassClassifier.Category.APPLICATION,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService",
                        Fixtures.appDirSource(), Fixtures.context()));
    }

    @Test
    public void bootInfoLibIsDependencyEvenIfPackageLooksBusiness() {
        // 关键回归：包名像业务代码（com.example），但 codeSource 在 BOOT-INF/lib
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService",
                        Fixtures.bootLibSource(), Fixtures.context("com.example")));
    }

    @Test
    public void bootInfoClassesIsApplication() {
        assertEquals(ClassClassifier.Category.APPLICATION,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService",
                        Fixtures.bootClassesSource(), Fixtures.context()));
    }

    // Spring Boot 3.2+ 的嵌套 URL 不再以 "!/" 分隔（`nested:/app.jar/!BOOT-INF/...`）。
    // 端到端实测：logback 等依赖的 codeSource 是这种形态，旧实现全部落到 UNKNOWN。

    @Test
    public void bootNestedLibJarIsDependency() {
        ClassOrigin.Source src = Fixtures.source(
                "nested:/work/spring-boot-demo.jar/!BOOT-INF/lib/logback-classic-1.4.14.jar");
        assertEquals(ClassOrigin.Kind.NESTED_LIB, src.kind);
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService", src, Fixtures.context()));
    }

    @Test
    public void bootNestedLibPackagePathIsDependency() {
        ClassOrigin.Source src = Fixtures.source(
                "nested:/work/spring-boot-demo.jar/!BOOT-INF/lib/logback-classic-1.4.14.jar!/ch/qos/logback/");
        assertEquals(ClassOrigin.Kind.NESTED_LIB, src.kind);
    }

    @Test
    public void bootNestedClassesIsApplication() {
        ClassOrigin.Source src = Fixtures.source(
                "nested:/work/spring-boot-demo.jar/!BOOT-INF/classes");
        assertEquals(ClassOrigin.Kind.NESTED_CLASSES, src.kind);
        assertEquals(ClassClassifier.Category.APPLICATION,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService", src, Fixtures.context()));
    }

    @Test
    public void boot3NestedClassesIsApplication() {
        ClassOrigin.Source src = Fixtures.source(
                "nested:/work/spring-boot-demo.jar/!BOOT-INF/classes!/com/example/");
        assertEquals(ClassOrigin.Kind.NESTED_CLASSES, src.kind);
    }

    @Test
    public void nestedSeparatorsTrimmed() {
        // 入参已是小写（classifyUrl 先 lower-case）；本方法只负责剥分隔符
        assertEquals("boot-inf/lib/a.jar", ClassOrigin.trimNestedSeparators("/!boot-inf/lib/a.jar"));
        assertEquals("boot-inf/classes", ClassOrigin.trimNestedSeparators("!/boot-inf/classes!/"));
        assertEquals("boot-inf/classes", ClassOrigin.trimNestedSeparators("boot-inf/classes/"));
        assertEquals("", ClassOrigin.trimNestedSeparators("!/"));
    }

    @Test
    public void bootLoaderClassesAreDependencyDespiteAppJarSource() {
        // 关键回归（端到端实测）：Boot 3.x loader 类与 app.jar 共享 codeSource，
        // 只看 codeSource 会把 org.springframework.boot.loader.* 判成业务代码
        ClassOrigin.Source src = Fixtures.source(
                "nested:/work/spring-boot-demo.jar/!BOOT-INF/classes");
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOfFixtureName(
                        "org.springframework.boot.loader.jar.ManifestInfo", src, Fixtures.context()));
    }

    @Test
    public void ownAgentClassesAreExcludedFromBusiness() {
        String own = "io.miyo.agent.AgentArgs";
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOfFixtureName(own, Fixtures.bootLibSource(), Fixtures.context()));
        // 测试夹具不属 agent 实现目录枚举 → 仍按 codeSource 判业务
        assertEquals(ClassClassifier.Category.APPLICATION,
                Fixtures.categoryOfFixtureName("com.example.demo.OrderService",
                        Fixtures.appDirSource(), Fixtures.context()));
    }

    @Test
    public void jrtIsJdkInternal() {
        assertEquals(ClassClassifier.Category.JDK_INTERNAL,
                Fixtures.categoryOf(String.class, Fixtures.jdkSource(), Fixtures.context()));
    }

    @Test
    public void bootstrapClassWithoutCodeSourceIsJdk() {
        // String 的真实源码在 jrt:/java.base，但 JDK 8 下可能是 null loader
        ClassOrigin.Source real = ClassOrigin.resolve(String.class);
        assertEquals(ClassOrigin.Kind.JDK, real.kind);
    }

    @Test
    public void mavenRepositoryJarIsDependency() {
        ClassClassifier.Category c = Fixtures.categoryOf(Fixtures.OrderService.class,
                Fixtures.source("file:/Users/dev/.m2/repository/org/apache/commons/commons-lang3-3.14.0.jar"),
                Fixtures.context());
        assertEquals(ClassClassifier.Category.DEPENDENCY, c);
    }

    @Test
    public void plainApplicationJarIsApplication() {
        ClassClassifier.Category c = Fixtures.categoryOf(Fixtures.OrderService.class,
                Fixtures.source("file:/work/demo/target/demo-1.0.0.jar"),
                Fixtures.context());
        assertEquals(ClassClassifier.Category.APPLICATION, c);
    }

    // ------------------------------------------------------------------
    // 2) 生成类
    // ------------------------------------------------------------------

    @Test
    public void cglibClassNameIsGenerated() {
        assertTrue(ClassClassifier.isGeneratedName(
                "com.example.OrderService$$EnhancerBySpringCGLIB$$a1b2c3d4"));
    }

    @Test
    public void jdkProxyNameIsGenerated() {
        assertTrue(ClassClassifier.isGeneratedName("com.sun.proxy.$Proxy123"));
        assertTrue(ClassClassifier.isGeneratedName("jdk.proxy2.$Proxy17"));
    }

    @Test
    public void mockitoAndByteBuddyAreGenerated() {
        assertTrue(ClassClassifier.isGeneratedName(
                "com.example.OrderService$$EnhancerByMockitoWithCGLIB$$112233"));
        assertTrue(ClassClassifier.isGeneratedName("com.example.OrderService$ByteBuddy$abc"));
        assertTrue(ClassClassifier.isGeneratedName(
                "com.example.OrderService$$Lambda$14/0x00000008000c1c40"));
        assertTrue(ClassClassifier.isGeneratedName("com.foo.Bar_$$_javassist_1"));
        assertTrue(ClassClassifier.isGeneratedName("com.example.Order$HibernateProxy$xyz"));
    }

    @Test
    public void anonymousAndNestedClassesAreNotGenerated() {
        // 匿名类（Foo$1）是用户源码，不能误判为生成类
        assertFalse(ClassClassifier.isGeneratedName("com.example.OrderService$1"));
        assertFalse(ClassClassifier.isGeneratedName("com.example.OrderService"));
        assertFalse(ClassClassifier.isGeneratedName("com.example.OrderService$Inner"));
    }

    @Test
    public void generatedBeatsDependencySource() {
        // 代理类即使 codeSource 指向 lib，也按生成类展示（用户视角最需要区分）
        assertEquals(ClassClassifier.Category.GENERATED,
                Fixtures.categoryOfFixtureName("com.example.OrderService$$EnhancerBySpringCGLIB$$1",
                        Fixtures.bootLibSource(), Fixtures.context()));
    }

    // ------------------------------------------------------------------
    // 3) Spring Bean 角色
    // ------------------------------------------------------------------

    @Test
    public void springBeanRoleUpgradesApplication() {
        Set<String> beans = new LinkedHashSet<String>();
        beans.add(Fixtures.appDirClass().getName());
        ClassClassifier.Context ctx = Fixtures.contextWithBeans(beans);
        assertEquals(ClassClassifier.Category.SPRING_BEAN,
                Fixtures.categoryOf(Fixtures.appDirClass(), Fixtures.appDirSource(), ctx));
    }

    @Test
    public void springBeanRoleDoesNotOverrideDependencySource() {
        // starter 自动配置的 Bean（在 lib 里）仍是依赖——角色不能让依赖变业务
        Set<String> beans = new LinkedHashSet<String>();
        beans.add(Fixtures.appDirClass().getName());
        ClassClassifier.Context ctx = Fixtures.contextWithBeans(beans);
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOf(Fixtures.appDirClass(), Fixtures.bootLibSource(), ctx));
    }

    @Test
    public void springBeanRoleWithoutCodeSourceStillClassified() {
        Set<String> beans = new LinkedHashSet<String>();
        beans.add(Fixtures.appDirClass().getName());
        ClassClassifier.Context ctx = Fixtures.contextWithBeans(beans);
        assertEquals(ClassClassifier.Category.SPRING_BEAN,
                Fixtures.categoryOfFixtureName(Fixtures.appDirClass().getName(),
                        Fixtures.source("unknown:generated"), ctx));
    }

    // ------------------------------------------------------------------
    // 4) 业务包前缀归一 / 匹配
    // ------------------------------------------------------------------

    @Test
    public void prefixNormalizationAcceptsCommonShapes() {
        List<String> out = ClassClassifier.normalizePrefixes(Arrays.asList(
                "com.example.*", " com.company.order. ", "com.a;com.b", "*com.wild"));
        assertEquals(Arrays.asList("com.example", "com.company.order", "com.a", "com.b",
                "com.wild"), out);
    }

    @Test
    public void prefixMatchRespectsSegmentBoundary() {
        List<String> prefixes = ClassClassifier.normalizePrefixes(
                Arrays.asList("com.example"));
        assertEquals("com.example", ClassClassifier.matchPrefix(
                "com.example.order.OrderService", prefixes));
        assertNull(ClassClassifier.matchPrefix("com.examples.OrderService", prefixes));
        assertEquals("com.example", ClassClassifier.matchPrefix("com.example", prefixes));
    }

    @Test
    public void prefixNullAndEmptyTolerated() {
        assertTrue(ClassClassifier.normalizePrefixes(null).isEmpty());
        assertTrue(ClassClassifier.normalizePrefixes(
                Arrays.asList("", "  ", "***")).isEmpty());
    }

    // ------------------------------------------------------------------
    // 5) 业务子类型
    // ------------------------------------------------------------------

    @Test
    public void roleByClassNameSuffixAndPackage() {
        assertEquals("controller", ClassClassifier.roleOf(Fixtures.OrderController.class,
                "com.example.order.OrderController", Fixtures.context(), new ArrayList<String>()));
        assertEquals("service", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.OrderService", Fixtures.context(), new ArrayList<String>()));
        assertEquals("service", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.service.Pricing", Fixtures.context(), new ArrayList<String>()));
        assertEquals("manager", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.OrderManager", Fixtures.context(), new ArrayList<String>()));
        assertEquals("repository", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.OrderRepository", Fixtures.context(), new ArrayList<String>()));
        assertEquals("repository", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.OrderMapper", Fixtures.context(), new ArrayList<String>()));
        assertEquals("dto", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.OrderDto", Fixtures.context(), new ArrayList<String>()));
        assertEquals("config", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.MyConfig", Fixtures.context(), new ArrayList<String>()));
        assertEquals("other", ClassClassifier.roleOf(Fixtures.OrderService.class,
                "com.example.order.Helper", Fixtures.context(), new ArrayList<String>()));
    }

    // ------------------------------------------------------------------
    // 6) UNKNOWN 不误判
    // ------------------------------------------------------------------

    @Test
    public void unclassifiableStaysUnknown() {
        ClassClassifier.Context ctx = Fixtures.context(); // 无业务包、无 Bean、无启动信息
        assertEquals(ClassClassifier.Category.UNKNOWN,
                Fixtures.categoryOfFixtureName("mystery.pkg.Thing",
                        Fixtures.source("bundle://7.0:0/META-INF/"), ctx));
    }

    @Test
    public void knownFrameworkRootWithoutCodeSourceIsDependency() {
        assertEquals(ClassClassifier.Category.DEPENDENCY,
                Fixtures.categoryOfFixtureName("io.netty.channel.DefaultChannel",
                        Fixtures.source("unknown:"), Fixtures.context()));
    }

    @Test
    public void jdkPackageRootWithoutCodeSourceIsJdk() {
        assertEquals(ClassClassifier.Category.JDK_INTERNAL,
                Fixtures.categoryOfFixtureName("java.util.concurrent.ForkJoinPool",
                        Fixtures.source("unknown:"), Fixtures.context()));
    }

    @Test
    public void businessPackagePrefixRescuesUnknownSource() {
        ClassClassifier.Context ctx = Fixtures.context("com.myproject");
        assertEquals(ClassClassifier.Category.APPLICATION,
                Fixtures.categoryOfFixtureName("com.myproject.core.Worker",
                        Fixtures.source("encrypted:loader"), ctx));
    }

    // ------------------------------------------------------------------
    // 7) launch 信息
    // ------------------------------------------------------------------

    @Test
    public void manifestParsingHandlesContinuationLines() throws Exception {
        String manifest = "Manifest-Version: 1.0\r\n"
                + "Main-Class: org.springframework.boot.loader.launch.JarLauncher\r\n"
                + "Start-Class: com.example.demo.DemoApplication\r\n"
                + "Class-Path: lib/a.jar lib/b.jar\r\n"
                + "\r\n"
                + "Name: com/example/demo/DemoApplication.class\r\n"
                + "Implementation-Title: demo\r\n";
        InputStream in = new ByteArrayInputStream(manifest.getBytes("UTF-8"));
        Map<String, String> parsed = ClassOrigin.parseManifest(in);
        assertEquals("com.example.demo.DemoApplication", parsed.get("Start-Class"));
        assertEquals("lib/a.jar lib/b.jar", parsed.get("Class-Path"));
        // 空行之后是 per-entry section：不属于全局属性（顺序解析，不在 section 之后取全局值）
        assertNull(parsed.get("Long-Path"));
    }

    @Test
    public void launchInfoReadsRealJvmProperties() {
        // 真实运行在 surefire 里：sun.java.command 是 maven/surefire 主类，
        // 至少不能抛异常且字段可读。
        ClassOrigin.LaunchInfo info = ClassOrigin.launchInfo();
        assertTrue(info.classpathEntries.size() > 0);
        assertFalse(info.evidence.isEmpty());
        Map<String, Object> map = info.toMap();
        assertTrue(map.containsKey("main_class"));
        assertTrue(map.containsKey("start_class"));
    }

    // ------------------------------------------------------------------
    // 8) 排序 / 名字过滤
    // ------------------------------------------------------------------

    @Test
    public void defaultOrderPutsSpringBeanFirstAndJdkLast() {
        List<ClassClassifier.Category> order = new ArrayList<ClassClassifier.Category>(
                ClassIndex.DEFAULT_ORDER);
        assertTrue(order.indexOf(ClassClassifier.Category.SPRING_BEAN)
                < order.indexOf(ClassClassifier.Category.APPLICATION));
        assertTrue(order.indexOf(ClassClassifier.Category.APPLICATION)
                < order.indexOf(ClassClassifier.Category.DEPENDENCY));
        assertTrue(order.indexOf(ClassClassifier.Category.DEPENDENCY)
                < order.indexOf(ClassClassifier.Category.JDK_INTERNAL));
    }

    // ------------------------------------------------------------------
    // 9) 包根 / 包层级
    // ------------------------------------------------------------------

    @Test
    public void packageRootTakesTwoSegments() {
        assertEquals("com.example", ClassClassifier.packageRootOf("com.example.a.B"));
        assertEquals("com.B", ClassClassifier.packageRootOf("com.B"));
        assertNull(ClassClassifier.packageRootOf("Default"));
    }

    @Test
    public void packageLevelsCumulative() {
        assertEquals(Arrays.asList("com", "com.example", "com.example.order"),
                ClassFilter.packageLevels("com.example.order"));
        assertTrue(ClassFilter.packageLevels("").isEmpty());
    }

    @Test
    public void filterPrefixesSegmentBoundary() {
        assertTrue(ClassFilter.matchesAnyPrefix("com.example.a.B", "com.example"));
        assertFalse(ClassFilter.matchesAnyPrefix("com.examples.a.B", "com.example"));
        assertTrue(ClassFilter.matchesAnyPrefix(
                "com.example.a.B", Arrays.asList("org.x", "com.example")));
        assertFalse(ClassFilter.matchesAnyPrefix("com.example.a.B", Collections.<String>emptyList()));
    }

    @Test
    public void legacyMatchesKeepsIncludePriority() {
        // 老接口语义保持不变（二进制 / 源码兼容）
        assertTrue(ClassFilter.matches("java.lang.String", "java.lang", null));
        assertFalse(ClassFilter.matches("java.lang.String", null, "java.lang"));
        assertTrue(ClassFilter.matches("com.example.A", null, null));
    }
}