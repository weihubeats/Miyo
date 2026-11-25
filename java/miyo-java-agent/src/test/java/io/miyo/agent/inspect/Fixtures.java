package io.miyo.agent.inspect;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * 分类测试夹具：用「真实类 + 注入的 codeSource URL」构造分类场景。
 *
 * 为什么不用真的 BOOT-INF/lib jar：分类算法的输入是 (Class, codeSource URL,
 * 启动信息, Bean 角色)，把输入显式构造出来比搭一套 fat-jar 更能穷举边界。
 * codeSource 的真实解析路径由 {@link ClassOrigin#classifyUrl(String)} 单独覆盖。
 */
final class Fixtures {

    /** 夹具类所在的「应用」包（分类器只看 URL，不关心名字含义）。 */
    static final String APP_DIST = "count";

    /** 构造一个带指定 codeSource 的 Source（等价于 ProtectionDomain 解析结果）。 */
    static ClassOrigin.Source source(String url) {
        return ClassOrigin.classifyUrl(url);
    }

    static ClassOrigin.Source appDirSource() {
        return source("file:/work/demo/target/classes/");
    }

    static ClassOrigin.Source bootLibSource() {
        return source("jar:file:/work/demo.jar!/BOOT-INF/lib/netty-4.1.100.jar!/");
    }

    static ClassOrigin.Source bootClassesSource() {
        return source("jar:file:/work/demo.jar!/BOOT-INF/classes!/");
    }

    static ClassOrigin.Source jdkSource() {
        return source("jrt:/java.base");
    }

    static ClassClassifier.Context context(String... businessPackages) {
        List<String> pkgs = new ArrayList<String>();
        Collections.addAll(pkgs, businessPackages);
        return new ClassClassifier.Context(pkgs, new LinkedHashSet<String>(),
                Collections.<String>emptyList(), Collections.<Integer>emptySet(),
                Collections.<Integer>emptySet(), Collections.<String>emptyList(), false);
    }

    static ClassClassifier.Context contextWithBeans(Set<String> beanClasses,
                                                    String... businessPackages) {
        List<String> pkgs = new ArrayList<String>();
        Collections.addAll(pkgs, businessPackages);
        return new ClassClassifier.Context(pkgs, beanClasses,
                Collections.<String>emptyList(), Collections.<Integer>emptySet(),
                Collections.<Integer>emptySet(), Collections.<String>emptyList(), true);
    }

    static ClassClassifier.Category categoryOf(Class<?> cls, ClassOrigin.Source src,
                                               ClassClassifier.Context ctx) {
        return ClassClassifier.classify(cls, src, ctx).category;
    }

    /**
     * 按「伪类名」分类：分类算法对生成类的判定只看名字（不看 Class 对象），
     * 测试用一个属主类承载名字，等价于目标 JVM 里那个真类的行为。
     */
    static ClassClassifier.Category categoryOfFixtureName(String fakeName,
                                                          ClassOrigin.Source src,
                                                          ClassClassifier.Context ctx) {
        return ClassClassifier.classify(NamedFixture.class, fakeName, src, ctx).category;
    }

    static Class<?> appDirClass() {
        return OrderService.class;
    }

    /** 业务类夹具（本身从 test-classes 目录加载，codeSource 是目录）。 */
    static final class OrderController {
    }

    static final class OrderService {
    }

    static final class CglibProxyFixture {
    }

    private Fixtures() {
    }

    static List<String> list(String... values) {
        return Arrays.asList(values);
    }
}