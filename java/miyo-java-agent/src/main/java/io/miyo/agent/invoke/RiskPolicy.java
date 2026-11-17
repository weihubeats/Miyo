package io.miyo.agent.invoke;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;

/**
 * 调用风险策略（Agent 侧硬闸 + 风险分级，供 UI 提示与二次确认）。
 *
 * 硬闸（blocked）：类名黑名单（前缀）+ 可赋值性闸（拿到 Class 对象后按
 * 类型判定——覆盖 ClassLoader / Runtime 的任意子类）。
 * 分级（destructive）：方法名命中破坏性动词（删除 / 支付 / 关闭…）——
 * 仅提示，Miyo UI 负责二次确认；Agent 不拦。
 */
public final class RiskPolicy {

    /** 类名前缀黑名单（与 Miyo 侧 SecurityPolicy 同源双保险）。 */
    private static final String[] BLACKLISTED_PREFIXES = {
            "java.lang.Runtime", "java.lang.ProcessBuilder", "java.lang.ProcessHandle",
            "java.lang.System", "java.lang.ClassLoader", "java.lang.SecurityManager",
            "java.lang.Thread", "java.lang.ThreadGroup", "java.lang.reflect.",
            "java.lang.invoke.", "java.lang.instrument.", "java.lang.Module",
            "java.lang.ModuleLayer", "javax.script.", "jdk.nashorn.", "jdk.dynalink.",
            "jdk.internal.", "sun.misc.Unsafe", "sun.reflect.", "io.miyo.agent.",
            "java.io.File", "java.io.RandomAccessFile", "java.nio.file.Files",
            "java.nio.file.Paths", "java.net.Socket", "java.net.ServerSocket",
            "javax.net.ssl.SSLSocketFactory",
    };

    /** 破坏性方法名动词（word 片段匹配，小写）。 */
    private static final Set<String> DESTRUCTIVE_WORDS = new HashSet<>(Arrays.asList(
            "delete", "remove", "drop", "truncate", "clear", "purge", "destroy",
            "shutdown", "exit", "halt", "kill", "stop", "restart", "reset",
            "exec", "pay", "charge", "refund", "transfer", "withdraw",
            "evict", "unsubscribe", "cancel", "revoke", "flush"));

    private static final Set<Class<?>> BLOCKED_TYPES = new HashSet<>(Arrays.asList(
            ClassLoader.class, java.lang.Runtime.class, java.lang.ProcessBuilder.class,
            java.lang.Thread.class, java.lang.ThreadGroup.class,
            java.lang.SecurityManager.class, java.lang.System.class));

    /** 硬闸：类名黑名单（发现期快筛）。 */
    public static boolean blacklisted(String className) {
        for (String prefix : BLACKLISTED_PREFIXES) {
            if (className.startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    /**
     * 硬闸：可赋值性判定（调用期强校验）。黑名单前缀只按名字匹配，
     * 这里按类型兜底——任何 ClassLoader/Runtime/Thread 的子类一律拒绝。
     */
    public static String blockedTypeReason(Class<?> cls) {
        if (blacklisted(cls.getName())) {
            return "类命中黑名单：" + cls.getName();
        }
        for (Class<?> blocked : BLOCKED_TYPES) {
            if (blocked.isAssignableFrom(cls)) {
                return "类是受限类型（" + blocked.getName() + "）的实例/子类：禁止调用";
            }
        }
        // ProcessHandle 在 Java 9+，Java 8 编译目标用名字判断（前缀黑名单已含）
        return null;
    }

    /** 风险分级：blocked / destructive / construct（新建实例）/ safe。 */
    public static String riskLevel(Class<?> owner, Method method) {
        String blocked = blockedTypeReason(owner);
        if (blocked != null) {
            return "blocked";
        }
        if (isDestructiveName(method.getName())) {
            return "destructive";
        }
        if (!Modifier.isStatic(method.getModifiers())) {
            return "instance";
        }
        return "safe";
    }

    /** 风险原因（human 可读，UI tooltip）。 */
    public static String riskReason(Class<?> owner, Method method) {
        String blocked = blockedTypeReason(owner);
        if (blocked != null) {
            return blocked;
        }
        String name = method.getName().toLowerCase(Locale.ROOT);
        for (String word : DESTRUCTIVE_WORDS) {
            String w = word.endsWith("?") ? word.substring(0, word.length() - 1) : word;
            if (name.contains(w)) {
                return "方法名含破坏性动词（" + w + "）：执行前请确认副作用";
            }
        }
        if (!Modifier.isStatic(method.getModifiers())) {
            return "实例方法：需要 Bean 实例或新建临时实例（构造器可能带副作用）";
        }
        return null;
    }

    /** 破坏性动词判定（词边界：下划线 / 驼峰边界近似处理为 contains）。 */
    private static boolean isDestructiveName(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        for (String word : DESTRUCTIVE_WORDS) {
            if (lower.contains(word)) {
                return true;
            }
        }
        return false;
    }

    private RiskPolicy() {
    }
}
