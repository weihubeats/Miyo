package io.miyo.agent.inspect;

/** 测试钩子：判断分类算法是否「只看名字 + codeSource」处理该类。 */
interface NamedType {

    /** 伪类名（替代 Class#getName）。 */
    String name();
}