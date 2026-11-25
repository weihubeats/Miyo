package com.redotpay.manager;

import java.util.Collections;
import java.util.List;

/** 管理器（@Bean 工厂方法声明的类型——真实项目里最常见，BeanDefinition 无 beanClassName）。 */
public class ChannelManager {

    private final List<String> channels;

    public ChannelManager(List<String> channels) {
        this.channels = channels == null ? Collections.<String>emptyList() : channels;
    }

    public List<String> channels() {
        return channels;
    }
}