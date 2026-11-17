package io.miyo.debug;

import java.io.IOException;

/**
 * Miyo Java Debug Runtime 入口。
 *
 * <pre>{@code
 * // Spring Boot 项目示例（ApplicationRunner 中启动）：
 * MiyoDebugRuntime runtime = MiyoDebugRuntime.builder(System.getenv("MIYO_DEBUG_TOKEN"))
 *         .appName("order-service")
 *         .port(9918)
 *         .instanceProvider("spring")
 *         .register("orderService", OrderService.class,
 *                 () -> applicationContext.getBean(OrderService.class))
 *         .register(null, MathUtil.class, InstanceProvider.staticInstance())
 *         .start();
 * // 应用关闭时：runtime.stop();
 * }</pre>
 *
 * 安全约束（违背即拒绝启动）：
 * <ul>
 *   <li>token 必配——Runtime 禁止无鉴权暴露；</li>
 *   <li>只能显式注册白名单类；黑名单类（Runtime / ProcessBuilder /
 *       ClassLoader / System 等）注册即抛异常；</li>
 *   <li>未注册的类 / 方法经协议调用一律 not_registered。</li>
 * </ul>
 */
public final class MiyoDebugRuntime {

    private final RuntimeConfig config;
    private final MethodRegistry registry = new MethodRegistry();
    private final InvocationManager invocations;
    private final RuntimeRegistry localRegistry;
    private HttpApi api;
    private volatile boolean started;
    private Thread shutdownHook;

    private MiyoDebugRuntime(RuntimeConfig config) {
        this.config = config;
        this.invocations = new InvocationManager(config.policy);
        this.localRegistry = new RuntimeRegistry(config);
    }

    /** 创建 builder（token 必配）。 */
    public static RuntimeConfig.Builder builder(String token) {
        return RuntimeConfig.builder(token);
    }

    /** builder → Runtime（未 start 前 registers 完毕）。 */
    public static MiyoDebugRuntime create(RuntimeConfig.Builder builder) {
        return new MiyoDebugRuntime(builder.build());
    }

    /**
     * 注册白名单类。beanName 可空；provider 决定实例来源（静态类传
     * {@link InstanceProvider#staticInstance()}）。
     */
    public MiyoDebugRuntime register(String beanName, Class<?> targetClass,
                                     InstanceProvider provider) {
        registry.register(beanName, targetClass, provider);
        return this;
    }

    /** 注册固定单例（便捷重载）。 */
    public MiyoDebugRuntime registerInstance(String beanName, Object instance) {
        Class<?> target = instance.getClass();
        registry.register(beanName, target, InstanceProvider.of(instance));
        return this;
    }

    /** 启动 HTTP 端点。 */
    public synchronized MiyoDebugRuntime start() throws IOException {
        if (started) {
            throw new IllegalStateException("Runtime 已启动");
        }
        if (registry.methodCount() == 0) {
            throw new IllegalStateException("未注册任何白名单类（拒绝空端点启动）");
        }
        api = new HttpApi(config, registry, invocations);
        api.start();
        started = true;
        localRegistry.register();
        // 崩溃兜底：应用漏调 stop() 时，进程退出至少清掉注册文件
        //（HTTP 端点随进程消亡，无需在此关闭）。
        shutdownHook = new Thread(localRegistry::unregister, "miyo-debug-registry-cleanup");
        Runtime.getRuntime().addShutdownHook(shutdownHook);
        return this;
    }

    /** 停止（应用关闭钩子调用）。 */
    public synchronized void stop() {
        if (api != null) {
            api.stop();
            started = false;
            localRegistry.unregister();
            if (shutdownHook != null) {
                Runtime.getRuntime().removeShutdownHook(shutdownHook);
                shutdownHook = null;
            }
        }
    }

    public boolean isStarted() {
        return started;
    }

    /** 注册表访问（包内测试用）。 */
    MethodRegistry registry() {
        return registry;
    }

    public int port() {
        return config.port;
    }

    /** 自描述信息（info 端点同源；启动前可用于日志）。 */
    public String describe() {
        return "MiyoDebugRuntime{app=" + config.appName + ", bind=" + config.bindHost
                + ":" + config.port + ", methods=" + registry.methodCount() + "}";
    }
}
