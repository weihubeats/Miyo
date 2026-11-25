package io.miyo.agent.inspect;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;

/**
 * Spring 证据 → 分类（`@Bean` 返回类型、同源实现类）。
 *
 * 背景（真实用户反馈，2026-11）：真实项目里绝大多数 Bean 是 `@Bean` 工厂方法声明
 * 的——`BeanDefinition.getBeanClassName()` 返回 null，只有 resolvable type。
 * 只收集 beanClassName 时，这类 Bean 与它们的实现类全落回 APPLICATION，
 * 「Spring Beans」组永远是 0，用户看到的就是「分类根本没用」。
 */
public class SpringBeanEvidenceTest {

    @Before
    public void setUp() {
        ClassOrigin.setTestApplicationPrefixForTests(
                "io.miyo.agent.inspect.SpringBeanEvidenceTest");
    }

    @After
    public void tearDown() {
        ClassOrigin.setTestApplicationPrefixForTests(null);
    }

    @Test
    public void beanReturnTypeIsSpringBean() {
        Set<String> types = new LinkedHashSet<String>();
        types.add("com.example.demo.PriceService");
        ClassClassifier.Context ctx = new ClassClassifier.Context(
                Collections.<String>emptyList(), Collections.<String>emptySet(), types,
                Collections.<String, String>emptyMap(), Collections.<String>emptyList(),
                Collections.<Integer>emptySet(), Collections.<Integer>emptySet(),
                Collections.<String>emptyList(), true);

        ClassClassifier.Verdict verdict = ClassClassifier.classify(NamedFixture.class,
                "com.example.demo.PriceService", Fixtures.appDirSource(), ctx);
        assertEquals(ClassClassifier.Category.SPRING_BEAN, verdict.category);
        assertTrue(verdict.evidence.toString().contains("@Bean"));
    }

    @Test
    public void implementationSharingFactoryRootIsSpringBean() {
        // @Bean 声明的类型 com.example.demo.PriceSource → fm:com.example.demo#pricesource
        Map<String, String> factories = new LinkedHashMap<String, String>();
        factories.put("com.example.demo.PriceSource", "fm:com.example.demo#pricesource");
        // 实现类同 factoryMethodName → 同源键一致
        factories.put("com.example.demo.RemotePriceSource", "fm:com.example.demo#pricesource");
        ClassClassifier.Context ctx = new ClassClassifier.Context(
                Collections.<String>emptyList(), Collections.<String>emptySet(),
                Collections.<String>emptySet(), factories, Collections.<String>emptyList(),
                Collections.<Integer>emptySet(), Collections.<Integer>emptySet(),
                Collections.<String>emptyList(), true);

        ClassClassifier.Verdict verdict = ClassClassifier.classify(NamedFixture.class,
                "com.example.demo.RemotePriceSource", Fixtures.appDirSource(), ctx);
        assertEquals(ClassClassifier.Category.SPRING_BEAN, verdict.category);
        assertTrue(verdict.evidence.toString().contains("同源"));
    }

    @Test
    public void samePackageDtoIsNotBean() {
        // 同包根的 DTO 不能因为「隔壁是 Bean」就被吞进 Bean 组
        Map<String, String> factories = new LinkedHashMap<String, String>();
        factories.put("com.example.demo.Order", "fm:com.example.demo#order");
        ClassClassifier.Context ctx = new ClassClassifier.Context(
                Collections.<String>emptyList(), Collections.<String>emptySet(),
                Collections.<String>emptySet(), factories, Collections.<String>emptyList(),
                Collections.<Integer>emptySet(), Collections.<Integer>emptySet(),
                Collections.<String>emptyList(), true);

        ClassClassifier.Verdict verdict = ClassClassifier.classify(NamedFixture.class,
                "com.example.demo.OrderQuery", Fixtures.appDirSource(), ctx);
        assertEquals(ClassClassifier.Category.APPLICATION, verdict.category);
        // 角色按包名 / 类名后缀判定（伪类名 Query 不在 roleOf 的后缀表里）
        assertEquals("other", verdict.role);
    }

    @Test
    public void beanRootNormalizesImplementationSuffixes() {
        assertEquals(ClassClassifier.beanRootOf("com.example.SmsSender"),
                ClassClassifier.beanRootOf("com.example.SmsSenderImpl"));
        assertEquals(ClassClassifier.beanRootOf("com.example.FooAutoConfiguration"),
                ClassClassifier.beanRootOf("com.example.DefaultFoo"));
        // 泛型签名必须剥掉，否则归一失败
        assertEquals("com.example#pricesource",
                ClassClassifier.beanRootOf("com.example.PriceSource<java.math.BigDecimal>"));
        // 嵌套类取外层（外层名才是 Bean 身份）
        assertEquals("com.example#foo", ClassClassifier.beanRootOf("com.example.Foo$Config"));
    }

    @Test
    public void collectBeanEvidenceReadsBeanDefinitionOnly() {
        // 真实 BeanFactory 形态（反射契约）：getBeanDefinitionNames / getBeanDefinition
        Object factory = Proxy.newProxyInstance(
                getClass().getClassLoader(),
                new Class<?>[]{BeanFactoryStub.class},
                new InvocationHandler() {
                    @Override
                    public Object invoke(Object proxy, Method method, Object[] args) {
                        if ("getBeanDefinitionNames".equals(method.getName())) {
                            return new String[]{"priceService", "orderConfig"};
                        }
                        if ("getBeanDefinition".equals(method.getName())) {
                            String name = (String) args[0];
                            if ("priceService".equals(name)) {
                                return new BeanDefinitionStub("com.example.PriceService",
                                        null, null);
                            }
                            return new BeanDefinitionStub(null, "com.example.Order",
                                    "order");
                        }
                        return null;
                    }
                });

        Set<String> classes = new LinkedHashSet<String>();
        Set<String> types = new LinkedHashSet<String>();
        Map<String, String> factories = new LinkedHashMap<String, String>();
        ClassClassifier.collectBeanEvidence(factory, 100, classes, types, factories);

        // beanClassName 声明的类（@Component 路径）与 @Bean 返回类型都要进 classes
        assertTrue(classes.contains("com.example.PriceService"));
        // @Bean 路径：类型来自 getResolvableType().getTypeName()
        assertTrue(classes.toString().contains("Order"));
        assertTrue(!types.contains("com.example.PriceService"));
        // 归一键：@Bean 带 factoryMethodName，@Component 没有（root:）
        assertTrue(factories.get("com.example.PriceService").startsWith("root:com.example#"));
    }

    /** BeanFactory 最小反射契约（分类只调这两个方法）。 */
    public interface BeanFactoryStub {
        String[] getBeanDefinitionNames();

        Object getBeanDefinition(String name);
    }

    /** BeanDefinition 最小反射契约（全是无参 getter）。 */
    public static final class BeanDefinitionStub {
        private final String beanClassName;
        private final String resolvableType;
        private final String factoryMethodName;

        BeanDefinitionStub(String beanClassName, String resolvableType, String factoryMethodName) {
            this.beanClassName = beanClassName;
            this.resolvableType = resolvableType;
            this.factoryMethodName = factoryMethodName;
        }

        public int getRole() {
            return 0;
        }

        public String getBeanClassName() {
            return beanClassName;
        }

        public String getFactoryMethodName() {
            return factoryMethodName;
        }

        public Object getResolvableType() {
            return resolvableType == null ? null : new ResolvableTypeStub(resolvableType);
        }
    }

    /**
     * `ResolvableType` 的替身：`getTypeName()` 带泛型签名（真实形态：
     * `com.example.Supplier<java.lang.String>`），`resolve()` 返回擦除后的类。
     */
    public static final class ResolvableTypeStub {
        private final String className;

        ResolvableTypeStub(String className) {
            this.className = className;
        }

        public Class<?> resolve() {
            return NamedType.class;
        }

        public String getTypeName() {
            return className + "<java.lang.String>";
        }
    }

    // ------------------------------------------------------------------
    // Boot 2.x：@Bean 定义只有工厂方法名，return type 必须按签名解析
    // ------------------------------------------------------------------

    /**
     * 真实 Boot 2.7 形态：`ConfigurationClassBeanDefinition` 的
     * `getBeanClassName()` 为 null、`getResolvableType()` 也解析不到，
     * `getFactoryBeanName()` 返回的是**工厂 Bean 的名字**（"channelConfig"，
     * 不是类名）。此时必须经 BeanFactory 把名字解析成工厂类，再读工厂方法的
     * 返回类型；否则「Spring Beans」组恒为 0（真实用户截图复现）。
     */
    @Test
    public void factoryMethodReturnTypeResolvesViaBeanFactoryName() {
        Object factory = Proxy.newProxyInstance(
                getClass().getClassLoader(),
                new Class<?>[]{BeanFactoryStub.class},
                new InvocationHandler() {
                    @Override
                    public Object invoke(Object proxy, Method method, Object[] args) {
                        if ("getBeanDefinitionNames".equals(method.getName())) {
                            return new String[]{"channelConfig"};
                        }
                        if ("getBeanDefinition".equals(method.getName())
                                && "channelConfig".equals(args[0])) {
                            // 工厂 Bean 自身定义：CGLIB 增强类，需剥后缀才能加载
                            return new BeanDefinitionStub(
                                    BeanFactoryFixture.class.getName()
                                            + "$$EnhancerBySpringCGLIB$$abc", null, null);
                        }
                        return null;
                    }
                });
        Boot2BeanDefinitionStub definition = new Boot2BeanDefinitionStub("channelManager");
        String type = ClassClassifier.factoryMethodReturnType(definition, "channelManager", factory);
        assertEquals("io.miyo.agent.inspect.NamedType", type);
    }

    /** `factoryBeanName` 被当作类名直解析的兼容路径（部分容器实现如此）。 */
    @Test
    public void factoryMethodReturnTypeFallsBackToDirectClassName() {
        Boot2BeanDefinitionStub definition = new Boot2BeanDefinitionStub(
                "channelManager", BeanFactoryFixture.class.getName());
        String type = ClassClassifier.factoryMethodReturnType(definition, "channelManager", null);
        assertEquals("io.miyo.agent.inspect.NamedType", type);
    }

    /** CGLIB 增强的配置类：剥 `$$EnhancerBySpringCGLIB$$` 后缀后仍能读到方法。 */
    @Test
    public void factoryMethodReturnTypeStripsCglibSuffix() {
        Boot2BeanDefinitionStub definition = new Boot2BeanDefinitionStub(
                "channelManager", BeanFactoryFixture.class.getName()
                        + "$$EnhancerBySpringCGLIB$$abc123");
        String type = ClassClassifier.factoryMethodReturnType(definition, "channelManager", null);
        assertEquals("io.miyo.agent.inspect.NamedType", type);
    }

    /** 方法名不存在时返回 null（不抛异常、不误判）。 */
    @Test
    public void factoryMethodReturnTypeReturnsNullForUnknownMethod() {
        Boot2BeanDefinitionStub definition = new Boot2BeanDefinitionStub(
                "noSuchBeanMethod", BeanFactoryFixture.class.getName());
        assertEquals(null,
                ClassClassifier.factoryMethodReturnType(definition, "noSuchBeanMethod", null));
    }

    /** 泛型返回类型保留参数化签名（归一键剥签名后取外层类）。 */
    @Test
    public void factoryMethodReturnTypeKeepsGenericSignature() {
        Boot2BeanDefinitionStub definition = new Boot2BeanDefinitionStub(
                "priceList", BeanFactoryFixture.class.getName());
        String type = ClassClassifier.factoryMethodReturnType(definition, "priceList", null);
        assertTrue(type, type.startsWith("java.util.List<")
                && type.contains("io.miyo.agent.inspect.NamedType"));
    }

    /** Boot 2.x 形态的 BeanDefinition：只有工厂方法名 + 可选工厂 Bean 名。 */
    public static final class Boot2BeanDefinitionStub {
        private final String factoryMethodName;
        private final String factoryBeanName;

        Boot2BeanDefinitionStub(String factoryMethodName) {
            this(factoryMethodName, "channelConfig");
        }

        Boot2BeanDefinitionStub(String factoryMethodName, String factoryBeanName) {
            this.factoryMethodName = factoryMethodName;
            this.factoryBeanName = factoryBeanName;
        }

        public int getRole() {
            return 0;
        }

        public String getBeanClassName() {
            return null;
        }

        public String getFactoryMethodName() {
            return factoryMethodName;
        }

        public String getFactoryBeanName() {
            return factoryBeanName;
        }

        public Object getResolvableType() {
            return null;
        }
    }
}
