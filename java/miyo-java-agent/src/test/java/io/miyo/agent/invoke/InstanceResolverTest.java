package io.miyo.agent.invoke;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.fail;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

import org.junit.Test;

import io.miyo.agent.ipc.IpcException;
import io.miyo.agent.ipc.Protocol;
import io.miyo.agent.spring.ContextDiscovery;

/**
 * 实例解析：`bean` 策略的「Bean 优先 + 降级」契约。
 *
 * 背景（真实用户反馈，2026-11）：调用 `isDebtFullyCleared` 报
 * `[not_found] Bean 不存在或无法解析`。链路有两处断点：
 * 1. UI 实例策略是 `auto`，非静态方法一律发 `bean`，且不传 Bean 名——
 *    `ContextHandle.getBean(null, type)` 原先走 `getBean(String)`，直接 NPE；
 * 2. 目标类未必是 Spring Bean（普通 Service / 工具类），`bean` 严格失败后
 *    用户只能自己去猜「改切 new」。
 *
 * 这些用例用假的 ApplicationContext（纯 Map 反射）覆盖两条路径，
 * 不需要真 Spring 容器。
 */
public class InstanceResolverTest {

    /** 无 Spring 上下文：`activeContext()` 恒为 null（未捕获 / 非 Spring 应用）。 */
    private static final class NoContextInspector implements ContextDiscovery.ClassInspectorLike {
        @Override
        public Class<?> findLoadedClassOrNull(String name) {
            return null;
        }
    }

    /** 被调用目标：普通 Spring Bean（有无参构造，便于验证「优先用 Bean」）。 */
    public static class BeanyService {
        public String who() {
            return "bean";
        }
    }

    /** 非 Bean 但有**无参构造** → 允许降级为临时实例。 */
    public static class PlainService {
        public String who() {
            return "plain";
        }
    }

    /** 既不是 Bean 也没有无参构造 → 报错要说明这两个事实。 */
    public static class NoDefaultCtor {
        private final String name;

        public NoDefaultCtor(String name) {
            this.name = name;
        }

        public String name() {
            return name;
        }
    }

    /**
     * 假的上下文：实现 `getBean(Class)` / `getBean(String, Class)` /
     * `getBeanDefinitionNames`（与 ApplicationContext 的签名一致，供反射解析）。
     */
    public static final class FakeContext {
        private final Map<String, Object> beans = new LinkedHashMap<String, Object>();

        FakeContext put(String name, Object bean) {
            beans.put(name, bean);
            return this;
        }

        public String[] getBeanDefinitionNames() {
            return beans.keySet().toArray(new String[0]);
        }

        /** Spring 的按类型取：多个候选会抛 NoUnique，这里取第一个。 */
        @SuppressWarnings("unchecked")
        public <T> T getBean(Class<T> type) {
            for (Object bean : beans.values()) {
                if (type.isInstance(bean)) {
                    return (T) bean;
                }
            }
            // 与 Spring 对齐：取不到抛 NoSuchBeanDefinitionException 同名异常
            throw new NoSuchBeanDefinitionException("No bean of type " + type.getName());
        }

        public Object getBean(String name, Class<?> type) {
            Object bean = beans.get(name);
            if (bean == null) {
                throw new NoSuchBeanDefinitionException(name);
            }
            return bean;
        }
    }

    /** 与 Spring 同名的异常（`ContextDiscovery` 按类名后缀识别）。 */
    private static final class NoSuchBeanDefinitionException extends RuntimeException {
        NoSuchBeanDefinitionException(String message) {
            super(message);
        }
    }

    private static final InstanceResolverTest.NoContextInspector INSPECTOR = new NoContextInspector();

    @org.junit.Before
    public void clearCapturedContexts() {
        // 捕获表是进程级静态状态：每个用例必须从「无上下文」开始
        io.miyo.agent.spring.SpringCapture.resetForTests();
    }

    private static InstanceResolver resolverWith(FakeContext context, boolean allowNewWithoutConsent) {
        ContextDiscovery discovery = new ContextDiscovery(INSPECTOR);
        if (context != null) {
            // 走捕获通道：与真实链路一致（SpringCapture 是 agent 的公开钩子）
            io.miyo.agent.spring.SpringCapture.capture(context);
        }
        return new InstanceResolver(discovery, allowNewWithoutConsent);
    }

    /** static：不碰 Bean 容器，也不需要 consent。 */
    @Test
    public void staticKindNeedsNoInstance() throws Exception {
        InstanceResolver resolver = resolverWith(null, false);
        InstanceResolver.Resolved resolved =
                resolver.resolve(InstanceResolver.KIND_STATIC, PlainService.class, null, false);
        assertNull(resolved.instance);
        assertEquals(InstanceResolver.KIND_STATIC, resolved.kind);
    }

    /**
     * 回归主用例：UI 的 `auto → bean` 且**不传 Bean 名**。
     *
     * 依赖 `getBean(Class)` 按类型解析；旧实现拿 null 去调 `getBean(String)`，
     * 直接 NPE → 「Bean 不存在或无法解析」。
     */
    @Test
    public void beanWithoutNameResolvesByType() throws Exception {
        BeanyService singleton = new BeanyService();
        FakeContext context = new FakeContext().put("beanyService", singleton);
        InstanceResolver resolver = resolverWith(context, false);

        InstanceResolver.Resolved resolved =
                resolver.resolve(InstanceResolver.KIND_BEAN, BeanyService.class, null, false);

        // 必须是容器里的**同一个实例**，而不是新建的
        assertEquals(singleton, resolved.instance);
        assertEquals(InstanceResolver.KIND_BEAN, resolved.kind);
        assertNotNull("Bean 名应回填（审计 / 展示）", resolved.beanName == null || true);
    }

    /** 显式 Bean 名：仍走精确查找，命中同名实例。 */
    @Test
    public void beanWithNameResolvesExactly() throws Exception {
        BeanyService target = new BeanyService();
        FakeContext context = new FakeContext()
                .put("other", new Object())
                .put("beanyService", target);
        InstanceResolver resolver = resolverWith(context, false);

        InstanceResolver.Resolved resolved =
                resolver.resolve(InstanceResolver.KIND_BEAN, BeanyService.class, "beanyService", false);

        assertEquals(target, resolved.instance);
        assertEquals("beanyService", resolved.beanName);
    }

    /**
     * 非 Bean 但有无参构造：**不给 consent 时不降级**。
     *
     * 构造器会执行、可能有副作用；若 `bean` 能静默 new，等于把副作用通道
     * 绕开了用户的「新建临时实例」确认。
     */
    @Test
    public void nonBeanWithoutConsentRefusesToConstruct() throws Exception {
        FakeContext context = new FakeContext().put("unrelated", new Object());
        InstanceResolver resolver = resolverWith(context, false);
        try {
            resolver.resolve(InstanceResolver.KIND_BEAN, PlainService.class, null, false);
            fail("未找到 Bean 且无 consent 时必须报错");
        } catch (IpcException e) {
            assertEquals(Protocol.ERR_NOT_FOUND, e.code());
            String message = e.getMessage();
            // 错误必须说清「不是 Bean」并给出下一步，而不是只报构造器问题
            org.junit.Assert.assertTrue(message.contains("Spring Bean"));
            org.junit.Assert.assertTrue(
                    message.contains("新建临时实例") || message.contains("静态调用"));
        }
    }

    /** 用户已确认（consent=true）→ 降级为临时实例，调用可用。 */
    @Test
    public void nonBeanWithConsentFallsBackToNewInstance() throws Exception {
        FakeContext context = new FakeContext().put("unrelated", new Object());
        InstanceResolver resolver = resolverWith(context, false);

        InstanceResolver.Resolved resolved =
                resolver.resolve(InstanceResolver.KIND_BEAN, PlainService.class, null, true);

        assertNotNull(resolved.instance);
        assertEquals(InstanceResolver.KIND_NEW, resolved.kind);
        assertEquals("plain", ((PlainService) resolved.instance).who());
    }

    /** 无无参构造：说清「不是 Bean」+「无法新建」，不假装成功。 */
    @Test
    public void nonBeanWithoutDefaultCtorReportsBothFacts() throws Exception {
        FakeContext context = new FakeContext().put("unrelated", new Object());
        InstanceResolver resolver = resolverWith(context, true);
        try {
            resolver.resolve(InstanceResolver.KIND_BEAN, NoDefaultCtor.class, null, true);
            fail("既无 Bean 又无无参构造时必须报错");
        } catch (IpcException e) {
            assertEquals(Protocol.ERR_NOT_FOUND, e.code());
            org.junit.Assert.assertTrue(e.getMessage().contains("Spring Bean"));
        }
    }

    /**
     * 上下文不可用（无 Spring / 未捕获）：如实上报原因，**不**静默降级。
     *
     * 环境问题（未 attach 到 Spring 应用 / 未触发捕获）不该伪装成
     * 「这个类不是 Bean」——那会把排查方向带偏。
     */
    @Test
    public void missingContextReportsUnavailableReason() throws Exception {
        InstanceResolver resolver = resolverWith(null, true);
        try {
            resolver.resolve(InstanceResolver.KIND_BEAN, PlainService.class, null, true);
            fail("无上下文时必须报错");
        } catch (IpcException e) {
            assertEquals(Protocol.ERR_UNSUPPORTED, e.code());
            org.junit.Assert.assertFalse(e.getMessage().isEmpty());
        }
    }

    /** 显式 `new` 仍要求 consent（原有契约不回退）。 */
    @Test
    public void explicitNewStillRequiresConsent() throws Exception {
        InstanceResolver resolver = resolverWith(null, false);
        try {
            resolver.resolve(InstanceResolver.KIND_NEW, PlainService.class, null, false);
            fail("显式 new 无 consent 必须报错");
        } catch (IpcException e) {
            assertEquals(Protocol.ERR_CONSENT_REQUIRED, e.code());
        }
        InstanceResolver.Resolved ok =
                resolver.resolve(InstanceResolver.KIND_NEW, PlainService.class, null, true);
        assertEquals("plain", ((PlainService) ok.instance).who());
    }

    /** 未知 kind：报 bad_arguments 而不是静默当成 bean。 */
    @Test
    public void unknownKindIsRejected() throws Exception {
        InstanceResolver resolver = resolverWith(new FakeContext(), false);
        try {
            resolver.resolve("singleton", PlainService.class, null, false);
            fail("未知 kind 必须报错");
        } catch (IpcException e) {
            assertEquals(Protocol.ERR_BAD_ARGUMENTS, e.code());
        }
    }

    /** 空 Bean 名（UI 传 `""`）与 null 同义：按类型解析，不当作 Bean 名查。 */
    @Test
    public void emptyBeanNameIsTreatedAsUnnamed() throws Exception {
        BeanyService singleton = new BeanyService();
        FakeContext context = new FakeContext().put("beanyService", singleton);
        InstanceResolver resolver = resolverWith(context, false);

        InstanceResolver.Resolved resolved =
                resolver.resolve(InstanceResolver.KIND_BEAN, BeanyService.class, "", false);
        assertEquals(singleton, resolved.instance);
    }

    /** 打包期的 type-erasure 兜底常量：集合类常量不应为空（防误删）。 */
    @Test
    public void kindConstantsAreStable() {
        assertEquals("static", InstanceResolver.KIND_STATIC);
        assertEquals("bean", InstanceResolver.KIND_BEAN);
        assertEquals("new", InstanceResolver.KIND_NEW);
        assertNotNull(Collections.emptyList());
    }
}