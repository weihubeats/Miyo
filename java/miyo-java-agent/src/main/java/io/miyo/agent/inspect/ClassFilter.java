package io.miyo.agent.inspect;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * 类过滤规则（前缀匹配 / 搜索 / 名字拆分）。
 *
 * 2.1 起「是不是 JDK 内部类」不再由本类的前缀黑名单决定——类归属一律走
 * {@link ClassClassifier} 的多维证据（codeSource / 启动信息 / Bean 角色）。
 *
 * 本类只保留两类**用户显式输入**的规则：
 * <ul>
 *   <li>{@code include} / {@code exclude} 前缀（老参数，语义向后兼容：include
 *       命中即保留，exclude 只在与 include 并存时生效）；</li>
 *   <li>{@code package_prefix} 包名前缀筛选（业务包配置 / 浏览过滤）。</li>
 * </ul>
 */
public final class ClassFilter {

    /** 老调用方的兼容前缀表（仅用于 include/exclude 的语义说明，不再用于默认过滤）。 */
    private static final String[] LEGACY_JDK_PREFIXES = {
            "java.", "javax.", "jdk.", "sun.", "com.sun.", "org.w3c.", "org.xml.",
            "org.ietf.", "org.omg.", "io.miyo.agent.",
    };

    /**
     * 老接口保留（二进制 / 源码兼容）：返回该名字是否属于「传统 JDK 前缀」。
     * 注意：这只是名字层面的弱判断，新代码请使用 {@link ClassClassifier}。
     */
    public static boolean isDefaultExcluded(String className) {
        if (className == null) {
            return false;
        }
        for (String prefix : LEGACY_JDK_PREFIXES) {
            if (className.startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    /**
     * 老接口保留：include 优先（命中即 true），其次 exclude。
     * 新代码用 {@link #matchesAnyPrefix(String, String)} / {@link #parsePrefixes(String)}。
     */
    public static boolean matches(String className, String include, String exclude) {
        List<String> includes = parsePrefixes(include);
        if (!includes.isEmpty()) {
            return matchesAnyPrefix(className, includes);
        }
        List<String> excludes = parsePrefixes(exclude);
        if (!excludes.isEmpty()) {
            return !matchesAnyPrefix(className, excludes);
        }
        return true;
    }

    /** 多形态前缀解析：`a.b,c.*` / `a.b c.d` / 单值；归一后返回。 */
    public static List<String> parsePrefixes(String raw) {
        return ClassClassifier.normalizePrefixes(
                raw == null ? null : java.util.Arrays.asList(raw));
    }

    /** 段边界前缀匹配（com.ex 不匹配 com.example）。 */
    public static boolean matchesAnyPrefix(String className, String rawPrefixes) {
        return matchesAnyPrefix(className, parsePrefixes(rawPrefixes));
    }

    public static boolean matchesAnyPrefix(String className, List<String> prefixes) {
        if (className == null || prefixes == null || prefixes.isEmpty()) {
            return false;
        }
        for (String prefix : prefixes) {
            if (className.equals(prefix)) {
                return true;
            }
            if (className.length() > prefix.length() && className.startsWith(prefix)
                    && className.charAt(prefix.length()) == '.') {
                return true;
            }
        }
        return false;
    }

    /** search 不区分大小写子串匹配（类名 / 包名 / 简名）。 */
    public static boolean searchMatches(String className, String search) {
        if (search == null || search.trim().isEmpty()) {
            return true;
        }
        String term = search.trim().toLowerCase(Locale.ROOT);
        return className.toLowerCase(Locale.ROOT).contains(term);
    }

    /** 类名 → 包名 / 简名（工具方法， inspector 与 UI 共用约定）。 */
    public static Map<String, String> splitName(String className) {
        Map<String, String> out = new LinkedHashMap<String, String>();
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

    /** 包名 → 前缀层级列表（`a.b.c` → [a, a.b, a.b.c]），供包树展示。 */
    public static List<String> packageLevels(String packageName) {
        List<String> out = new ArrayList<String>();
        if (packageName == null || packageName.isEmpty()) {
            return out;
        }
        StringBuilder sb = new StringBuilder();
        for (String segment : packageName.split("\\.")) {
            if (segment.isEmpty()) {
                continue;
            }
            if (sb.length() > 0) {
                sb.append('.');
            }
            sb.append(segment);
            out.add(sb.toString());
        }
        return out;
    }

    private ClassFilter() {
    }
}