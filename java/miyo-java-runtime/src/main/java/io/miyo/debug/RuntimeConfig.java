package io.miyo.debug;

/**
 * Runtime 配置（builder 构造；token 必配——禁止无鉴权的调试端点）。
 */
public final class RuntimeConfig {

    public static final String SDK_VERSION = "0.1.0";

    final String token;
    final int port;
    final String bindHost;
    final String appName;
    final int httpThreads;
    final SecurityPolicy policy;
    final String instanceProviderName;

    private RuntimeConfig(Builder builder) {
        this.token = builder.token;
        this.port = builder.port;
        this.bindHost = builder.bindHost;
        this.appName = builder.appName;
        this.httpThreads = builder.httpThreads;
        this.policy = builder.policy;
        this.instanceProviderName = builder.instanceProviderName;
    }

    public static Builder builder(String token) {
        if (token == null || token.trim().isEmpty()) {
            throw new IllegalArgumentException(
                    "必须配置调试 token（Runtime 禁止无鉴权暴露）——建议从环境变量读取，不要硬编码");
        }
        return new Builder(token.trim());
    }

    public static final class Builder {
        private final String token;
        private int port = 9918;
        private String bindHost = "127.0.0.1";
        private String appName = "miyo-debug-app";
        private int httpThreads = 4;
        private SecurityPolicy policy = SecurityPolicy.defaults();
        private String instanceProviderName = "static";

        private Builder(String token) {
            this.token = token;
        }

        /** 监听端口（默认 9918）。 */
        public Builder port(int port) {
            this.port = port;
            return this;
        }

        /** 监听地址（默认 127.0.0.1；跨机调试才开放 0.0.0.0，务必配强 token）。 */
        public Builder bindHost(String bindHost) {
            this.bindHost = bindHost;
            return this;
        }

        /** 应用名（握手 info 展示）。 */
        public Builder appName(String appName) {
            if (appName != null && !appName.trim().isEmpty()) {
                this.appName = appName.trim();
            }
            return this;
        }

        /** HTTP 线程数。 */
        public Builder httpThreads(int httpThreads) {
            this.httpThreads = httpThreads;
            return this;
        }

        /** 自定义安全策略（默认 {@link SecurityPolicy#defaults()}）。 */
        public Builder policy(SecurityPolicy policy) {
            if (policy != null) {
                this.policy = policy;
            }
            return this;
        }

        /** 实例提供方式标注（"static" / "spring"；握手 info 展示）。 */
        public Builder instanceProvider(String name) {
            if (name != null && !name.trim().isEmpty()) {
                this.instanceProviderName = name.trim();
            }
            return this;
        }

        RuntimeConfig build() {
            return new RuntimeConfig(this);
        }
    }
}
