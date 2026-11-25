package io.miyo.agent.inspect;

/**
 * `@Bean` 工厂方法的测试工厂类。
 *
 * 用途：复现 Boot 2.x 的 `@Bean` BeanDefinition 形态——`getBeanClassName()` 与
 * `getResolvableType()` 都拿不到类型，只剩 `getFactoryMethodName()`。分类器必须
 * 能在工厂类上按方法名读出返回类型（{@link SpringBeanEvidenceTest}）。
 *
 * 类名刻意使用普通类（非接口、非内部类），因为测试要按名字 `Class.forName`
 * 加载后进行方法查找。
 */
public class BeanFactoryFixture {

    /** 返回类型即「Bean 类型」，供测试断言。 */
    public NamedType channelManager() {
        return null;
    }

    /** 泛型返回类型：验证剥离签名后仍能归一到外层类。 */
    public java.util.List<NamedType> priceList() {
        return null;
    }
}