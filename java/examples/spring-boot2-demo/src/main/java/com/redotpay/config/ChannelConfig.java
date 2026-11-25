package com.redotpay.config;

import com.redotpay.manager.ChannelManager;
import java.util.Arrays;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

/**
 * `@Bean` 工厂方法声明的 Bean。
 *
 * 关键：`ChannelManager` 的 BeanDefinition 里 `getBeanClassName()` 为 null，
 * 只有 resolvable type。只读 beanClassName 的实现会把这类 Bean 整个漏掉。
 */
@Configuration
public class ChannelConfig {

    @Bean
    public ChannelManager channelManager() {
        return new ChannelManager(Arrays.asList("alipay", "wechat"));
    }
}