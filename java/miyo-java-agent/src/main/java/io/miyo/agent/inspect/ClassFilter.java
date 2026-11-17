package io.miyo.agent.inspect;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * 类过滤规则（类清单默认排除 JDK / 框架内部类；include / exclude 前缀可覆盖）。
 *
 * 默认排除（前缀匹配）：java. / javax. / jdk. / sun. / com.sun. / org.w3c. /
 * org.xml. / org.ietf. / org.omg. / io.miyo.agent.（自身）。include 规则优先级
 * 高于 exclude（先命中 include 即保留），用于白名单式查看 JDK 类。
 */
public final class ClassFilter {

    private static final String[] DEFAULT_EXCLUDE = {
            "java.", "javax.", "jdk.", "sun.", "com.sun.", "org.w3c.", "org.xml.",
            "org.ietf.", "org.omg.", "io.miyo.agent.",
    };

    /** 是否默认排除（JDK 内部类等）。 */
    public static boolean isDefaultExcluded(String className) {
        for (String prefix : DEFAULT_EXCLUDE) {
            if (className.startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    /**
     * 应用过滤：include / exclude 为空表示用默认规则；两者均可多前缀（`,` 分隔）。
     */
    public static boolean matches(String className, String include, String exclude) {
        String[] includes = split(include);
        if (includes.length > 0) {
            for (String prefix : includes) {
                if (className.startsWith(prefix)) {
                    return true;
                }
            }
            return false;
        }
        String[] excludes = split(exclude);
        if (excludes.length > 0) {
            for (String prefix : excludes) {
                if (className.startsWith(prefix)) {
                    return false;
                }
            }
        } else if (isDefaultExcluded(className)) {
            return false;
        }
        return true;
    }

    /** search 不区分大小写子串匹配（类名 / 包名 / 简名）。 */
    public static boolean searchMatches(String className, String search) {
        if (search == null || search.trim().isEmpty()) {
            return true;
        }
        String term = search.trim().toLowerCase(Locale.ROOT);
        return className.toLowerCase(Locale.ROOT).contains(term);
    }

    private static String[] split(String value) {
        if (value == null || value.trim().isEmpty()) {
            return new String[0];
        }
        List<String> out = new ArrayList<>();
        for (String part : value.split(",")) {
            String p = part.trim();
            if (!p.isEmpty()) {
                out.add(p);
            }
        }
        return out.toArray(new String[0]);
    }

    /** 类名 → 包名 / 简名（工具方法， inspector 与 UI 共用约定）。 */
    public static Map<String, String> splitName(String className) {
        Map<String, String> out = new LinkedHashMap<>();
        int dot = className.lastIndexOf('.');
        String simple = dot >= 0 ? className.substring(dot + 1) : className;
        String pkg = dot >= 0 ? className.substring(0, dot) : "";
        // 嵌套类简名取外层（Order$Inner → Order）
        int dollar = simple.indexOf('$');
        if (dollar > 0) {
            simple = simple.substring(0, dollar);
        }
        out.put("package_name", pkg);
        out.put("simple_name", simple);
        return out;
    }

    private ClassFilter() {
    }
}
