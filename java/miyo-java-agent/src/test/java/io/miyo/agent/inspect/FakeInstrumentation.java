package io.miyo.agent.inspect;

import java.lang.instrument.Instrumentation;

/** 测试用 Instrumentation（Java 8 源码级：Module 相关方法不是默认方法，用动态代理）。 */
final class FakeInstrumentation {
    private volatile Class<?>[] loaded = new Class<?>[0];

    void set(Class<?>[] classes) {
        this.loaded = classes;
    }

    Instrumentation proxy() {
        return (Instrumentation) java.lang.reflect.Proxy.newProxyInstance(
                ClassIndexTest.class.getClassLoader(),
                new Class<?>[]{Instrumentation.class},
                new java.lang.reflect.InvocationHandler() {
                    @Override
                    public Object invoke(Object p, java.lang.reflect.Method method, Object[] args) {
                        String name = method.getName();
                        if ("getAllLoadedClasses".equals(name)
                                || "getInitiatedClasses".equals(name)) {
                            return loaded;
                        }
                        if ("getObjectSize".equals(name)) {
                            return 0L;
                        }
                        Class<?> type = method.getReturnType();
                        if (type == boolean.class) {
                            return Boolean.FALSE;
                        }
                        if (type == int.class) {
                            return 0;
                        }
                        if (type == long.class) {
                            return 0L;
                        }
                        return null;
                    }
                });
    }
}
