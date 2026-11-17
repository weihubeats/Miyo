package io.miyo.debug;

/**
 * 实例提供方式：方法调用时解析目标对象实例。
 *
 * 静态类用 {@link #staticInstance()}；Spring 项目实现本接口返回
 * {@code applicationContext.getBean(beanName)}——优先经 Bean 容器取实例，
 * AOP 代理 / 事务 / 依赖注入语义与正常业务调用一致（不要用反射 new）。
 */
@FunctionalInterface
public interface InstanceProvider {

    /** 返回调用目标实例（每次调用重新解析，返回 null 即以静态方式执行）。 */
    Object resolve() throws Exception;

    /** 静态方法注册用（resolve 返回 null → 直接以静态方式调用）。 */
    static InstanceProvider staticInstance() {
        return () -> null;
    }

    /** 固定单例。 */
    static InstanceProvider of(Object instance) {
        return () -> instance;
    }
}
