package io.miyo.agent.inspect;

/**
 * 可指定名字的测试类（分类算法只读 {@code getName()} 判断生成类形态）。
 *
 * 用途：把「目标 JVM 里那个名字的类」的行为复现到测试里——例如 CGLIB 代理
 * `Foo$$EnhancerBySpringCGLIB$$1` 在分类阶段只看名字与 codeSource，
 * 不需要真的生成字节码。
 */
final class NamedFixture extends java.text.SimpleDateFormat implements NamedType {

    private static final long serialVersionUID = 1L;

    private final String fixtureName;

    NamedFixture(String name) {
        super("yyyy-MM-dd");
        this.fixtureName = name;
    }

    @Override
    public String name() {
        return fixtureName;
    }
}