package io.miyo.debug;

import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 结果序列化：Java 返回对象 → JSON 值（Map/List/标量）。
 *
 * 策略：record / POJO 走 getter（getXxx / isXxx），无 getter 的 public 字段
 * 兜底；enum 输出 name()；循环引用以 {@code __circular__} 截断；深度超限
 * 以 {@code __depth_limit__} 截断；序列化产物字符数超限整体置为截断占位
 *（由调用方标记 resultTruncated）。
 */
final class ResultSerializer {

    static final String CIRCULAR = "__circular__";
    static final String DEPTH_LIMIT = "__depth_limit__";

    private final int maxDepth;
    private final int maxChars;
    private final List<IdentityHashMap<Object, Boolean>> seenStack = new ArrayList<>();

    ResultSerializer(int maxDepth, int maxChars) {
        this.maxDepth = maxDepth;
        this.maxChars = maxChars;
    }

    /** 序列化入口；返回 JSON 值 + 是否超限截断。 */
    static Result serialize(Object value, int maxDepth, int maxChars) {
        ResultSerializer serializer = new ResultSerializer(maxDepth, maxChars);
        Object json = serializer.toValue(value, 0);
        String text = Json.write(json);
        boolean truncated = text.length() > maxChars;
        return new Result(truncated ? truncatedPlaceholder(text.length()) : json, truncated);
    }

    private static Map<String, Object> truncatedPlaceholder(int originalChars) {
        Map<String, Object> placeholder = new LinkedHashMap<>();
        placeholder.put("__truncated", true);
        placeholder.put("original_chars", originalChars);
        return placeholder;
    }

    private Object toValue(Object value, int depth) {
        if (value == null) {
            return null;
        }
        if (value instanceof String || value instanceof Number || value instanceof Boolean) {
            return value;
        }
        if (value instanceof Character) {
            return String.valueOf(value);
        }
        if (value instanceof Enum<?>) {
            return ((Enum<?>) value).name();
        }
        if (value instanceof java.time.temporal.Temporal || value instanceof java.util.Date) {
            return String.valueOf(value);
        }
        if (value instanceof Map<?, ?>) {
            if (depth >= maxDepth) {
                return DEPTH_LIMIT;
            }
            if (visited(value)) {
                return CIRCULAR;
            }
            try {
                Map<String, Object> out = new LinkedHashMap<>();
                for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
                    out.put(String.valueOf(entry.getKey()), toValue(entry.getValue(), depth + 1));
                }
                return out;
            } finally {
                popVisited();
            }
        }
        if (value instanceof Iterable<?>) {
            if (depth >= maxDepth) {
                return DEPTH_LIMIT;
            }
            if (visited(value)) {
                return CIRCULAR;
            }
            try {
                List<Object> out = new ArrayList<>();
                for (Object item : (Iterable<?>) value) {
                    out.add(toValue(item, depth + 1));
                }
                return out;
            } finally {
                popVisited();
            }
        }
        if (value.getClass().isArray()) {
            if (depth >= maxDepth) {
                return DEPTH_LIMIT;
            }
            int length = java.lang.reflect.Array.getLength(value);
            List<Object> out = new ArrayList<>(length);
            for (int i = 0; i < length; i++) {
                out.add(toValue(java.lang.reflect.Array.get(value, i), depth + 1));
            }
            return out;
        }
        if (value instanceof Class<?>) {
            return ((Class<?>) value).getName();
        }
        // POJO / record：getter 优先，public 字段兜底。
        if (depth >= maxDepth) {
            return DEPTH_LIMIT;
        }
        if (visited(value)) {
            return CIRCULAR;
        }
        try {
            Map<String, Object> out = new LinkedHashMap<>();
            for (MethodAccess access : propertiesOf(value.getClass())) {
                try {
                    out.put(access.name, toValue(access.read(value), depth + 1));
                } catch (ReflectiveOperationException ignored) {
                    // 单属性读取失败不毒化整体结果。
                }
            }
            return out;
        } finally {
            popVisited();
        }
    }

    private boolean visited(Object value) {
        for (IdentityHashMap<Object, Boolean> frame : seenStack) {
            if (frame.containsKey(value)) {
                return true;
            }
        }
        IdentityHashMap<Object, Boolean> frame = new IdentityHashMap<>();
        frame.put(value, Boolean.TRUE);
        seenStack.add(frame);
        return false;
    }

    private void popVisited() {
        if (!seenStack.isEmpty()) {
            seenStack.remove(seenStack.size() - 1);
        }
    }

    private static final class MethodAccess {
        final String name;
        final java.lang.reflect.Method method;
        final Field field;

        MethodAccess(String name, java.lang.reflect.Method method, Field field) {
            this.name = name;
            this.method = method;
            this.field = field;
        }

        Object read(Object target) throws ReflectiveOperationException {
            if (method != null) {
                return method.invoke(target);
            }
            field.setAccessible(true);
            return field.get(target);
        }
    }

    /** 类的属性访问器（getter 优先于 public 字段；按名称去重、字母序稳定）。 */
    private static List<MethodAccess> propertiesOf(Class<?> type) {
        Map<String, MethodAccess> out = new LinkedHashMap<>();
        for (java.lang.reflect.Method m : type.getMethods()) {
            if (m.getDeclaringClass() == Object.class || m.isSynthetic()
                    || m.getParameterCount() != 0 || Modifier.isStatic(m.getModifiers())) {
                continue;
            }
            String name = m.getName();
            String property = null;
            if (name.length() > 3 && name.startsWith("get")) {
                property = Character.toLowerCase(name.charAt(3)) + name.substring(4);
            } else if (name.length() > 2 && name.startsWith("is")
                    && (m.getReturnType() == boolean.class || m.getReturnType() == Boolean.class)) {
                property = Character.toLowerCase(name.charAt(2)) + name.substring(3);
            }
            if (property == null || property.isEmpty()) {
                continue;
            }
            out.putIfAbsent(property, new MethodAccess(property, m, null));
        }
        Class<?> cursor = type;
        while (cursor != null && cursor != Object.class) {
            for (Field field : cursor.getDeclaredFields()) {
                int mods = field.getModifiers();
                if (Modifier.isStatic(mods) || !Modifier.isPublic(mods) || field.isSynthetic()) {
                    continue;
                }
                Class<?> sup = type.getSuperclass();
                if (sup != null && "java.lang.Record".equals(sup.getName())) {
                    continue; // record（Java 16+）组件走 getter，已覆盖
                }
                out.putIfAbsent(field.getName(), new MethodAccess(field.getName(), null, field));
            }
            cursor = cursor.getSuperclass();
        }
        List<MethodAccess> list = new ArrayList<>(out.values());
        list.sort(java.util.Comparator.comparing(a -> a.name));
        return list;
    }

    /** 序列化结果（值 + 截断标记）。 */
    static final class Result {
        final Object value;
        final boolean truncated;

        Result(Object value, boolean truncated) {
            this.value = value;
            this.truncated = truncated;
        }
    }
}
