package io.miyo.agent.spring;

import java.util.List;
import java.util.Map;

/**
 * Spring 上下文捕获钩子（被重转换后的 AbstractApplicationContext 调用）。
 *
 * 关键设计：retransform 不能给目标类加字段，只能改方法体——因此捕获值存放在
 * 本类的静态字段（按 identityWeakRef 保活，避免阻止上下文被 GC）。
 *
 * 该类必须零 Spring 依赖（任何 JVM 都能加载它）。
 */
public final class SpringCapture {

    /** 捕获到的上下文（弱引用保活策略：使用 WeakReference，不阻止 GC）。 */
    private static volatile java.util.List<java.lang.ref.WeakReference<Object>> captured =
            new java.util.concurrent.CopyOnWriteArrayList<>();

    /** 捕获发生次数（含重复捕获同一实例）。 */
    private static volatile long captureCount;

    /** 捕获启用状态（transformer 是否已注册 + retransform 是否已执行）。 */
    private static volatile String state = "disabled";

    private SpringCapture() {
    }

    /**
     * 字节码插桩调用点：AbstractApplicationContext 的 publishEvent / getBean
     * 等热路径方法入口插入 {@code SpringCapture.capture(this)}。
     * 必须绝对无异常、无锁、无阻塞（运行在目标应用热路径上）。
     */
    public static void capture(Object applicationContext) {
        try {
            if (applicationContext == null) {
                return;
            }
            // 已捕获过同一实例则跳过（弱引用表小，线性扫够用）
            List<java.lang.ref.WeakReference<Object>> current = captured;
            for (java.lang.ref.WeakReference<Object> ref : current) {
                if (ref.get() == applicationContext) {
                    return;
                }
            }
            captured.add(new java.lang.ref.WeakReference<>(applicationContext));
            captureCount++;
            // 上限保护：截断最老条目（正常场景上下文数量极少）
            if (captured.size() > 8) {
                List<java.lang.ref.WeakReference<Object>> trimmed = current.subList(
                        current.size() - 8, current.size());
                captured = new java.util.concurrent.CopyOnWriteArrayList<>(trimmed);
            }
        } catch (Throwable ignored) {
            // 绝不让捕获钩子影响目标应用
        }
    }

    /** 捕获到的上下文快照（强引用临时持有；调用方用完即弃）。 */
    static List<Object> snapshot() {
        List<Object> out = new java.util.ArrayList<>();
        List<java.lang.ref.WeakReference<Object>> current = captured;
        for (java.lang.ref.WeakReference<Object> ref : current) {
            Object ctx = ref.get();
            if (ctx != null) {
                out.add(ctx);
            }
        }
        return out;
    }

    public static String state() {
        return state;
    }

    public static void setState(String newState) {
        state = newState;
    }

    public static long captureCount() {
        return captureCount;
    }

    /** 重置（测试用）。 */
    static void reset() {
        captured = new java.util.concurrent.CopyOnWriteArrayList<>();
        captureCount = 0;
        state = "disabled";
    }

    /**
     * 清空已捕获的上下文（测试用，跨包可见）。
     *
     * 捕获表是**进程级静态状态**，单测里多个用例共用 JVM；不清空会让
     * 前一个用例注册的假上下文被后一个用例命中（实证：`beanWithoutName`
     * 拿到别的用例的实例）。
     */
    public static void resetForTests() {
        reset();
    }
}
