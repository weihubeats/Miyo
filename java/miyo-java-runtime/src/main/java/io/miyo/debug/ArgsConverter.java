package io.miyo.debug;

import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 参数转换：JSON（Map/List/标量）→ 方法形参类型的 Java 对象。
 *
 * 支持：基本类型与包装类型、String/char、枚举、BigDecimal/BigInteger、
 * 集合（List/Set/Collection，元素按泛型可判时递归）、Map、数组、自定义
 * DTO（no-arg 构造 + 字段 / setter 注入，深度受限）。泛型擦除导致集合
 * 元素类型不可判时，元素保留 JSON 原生形态（Map/List/String/Number）。
 */
final class ArgsConverter {

    /** DTO 注入深度上限（防深嵌套递归）。 */
    private static final int MAX_DEPTH = 12;

    private final List<String> warnings = new ArrayList<>();

    List<String> warnings() {
        return warnings;
    }

    /** 按目标类型转换单个参数；失败抛 IllegalArgumentException（bad_arguments）。 */
    Object convert(Object value, Class<?> target) {
        return convert(value, target, null);
    }

    /**
     * 泛型感知转换：genericHint 提供目标位置的泛型信息（字段 / setter 形参的
     * {@code getGenericType()}），List&lt;Item&gt; / Map&lt;String,Item&gt; 等嵌套元素
     * 得以递归到具体类型；无提示时元素保留 JSON 原生形态（擦除限制）。
     */
    Object convert(Object value, Class<?> target, java.lang.reflect.Type genericHint) {
        if (value == null) {
            if (target.isPrimitive()) {
                throw new IllegalArgumentException("基本类型参数不能为 null：" + target.getName());
            }
            return null;
        }
        if (target == Object.class) {
            return value;
        }
        if (target == String.class) {
            return String.valueOf(jsonScalar(value));
        }
        if (target == Character.class || target == char.class) {
            String s = String.valueOf(jsonScalar(value));
            if (s.length() != 1) {
                throw new IllegalArgumentException("char 参数必须是单字符字符串，收到：" + s);
            }
            return s.charAt(0);
        }
        if (target == Boolean.class || target == boolean.class) {
            if (value instanceof Boolean) {
                return value;
            }
            throw new IllegalArgumentException("期望 boolean，收到 " + kindOf(value));
        }
        if (target == Byte.class || target == byte.class) {
            return checkedNumber(value, "byte").byteValueExact();
        }
        if (target == Short.class || target == short.class) {
            return checkedNumber(value, "short").shortValueExact();
        }
        if (target == Integer.class || target == int.class) {
            return checkedNumber(value, "int").intValueExact();
        }
        if (target == Long.class || target == long.class) {
            return checkedNumber(value, "long").longValueExact();
        }
        if (target == Float.class || target == float.class) {
            return checkedNumber(value, "float").floatValue();
        }
        if (target == Double.class || target == double.class) {
            return checkedNumber(value, "double").doubleValue();
        }
        if (target == BigDecimal.class) {
            return new BigDecimal(checkedNumber(value, "decimal").toString());
        }
        if (target == BigInteger.class) {
            return checkedNumber(value, "BigInteger").toBigIntegerExact();
        }
        if (target.isEnum()) {
            String name = String.valueOf(jsonScalar(value));
            @SuppressWarnings({"unchecked", "rawtypes"})
            Object enumValue = Enum.valueOf((Class<? extends Enum>) target, name);
            return enumValue;
        }
        if (target.isArray()) {
            return convertArray(value, target.getComponentType());
        }
        if (Collection.class.isAssignableFrom(target)) {
            return convertCollection(value, target, genericHint);
        }
        if (Map.class.isAssignableFrom(target)) {
            return convertMap(value, target, genericHint);
        }
        if (value instanceof Map) {
            return populateBean((Map<?, ?>) value, target, 0);
        }
        throw new IllegalArgumentException("无法把 " + kindOf(value) + " 转换为 " + target.getName());
    }

    private static Object jsonScalar(Object value) {
        return value;
    }

    private BigDecimal checkedNumber(Object value, String targetName) {
        if (value instanceof Number) {
            if (value instanceof Double || value instanceof Float) {
                return new BigDecimal(value.toString());
            }
            if (value instanceof Long || value instanceof Integer || value instanceof Short
                    || value instanceof Byte) {
                return BigDecimal.valueOf(((Number) value).longValue());
            }
            return new BigDecimal(value.toString());
        }
        if (value instanceof String) {
            try {
                return new BigDecimal(((String) value).trim());
            } catch (NumberFormatException e) {
                throw new IllegalArgumentException("字符串不是合法数字：" + value);
            }
        }
        throw new IllegalArgumentException("期望数字（" + targetName + "），收到 " + kindOf(value));
    }

    private Object[] convertArray(Object value, Class<?> component) {
        if (!(value instanceof List)) {
            throw new IllegalArgumentException("数组参数必须是 JSON 数组，收到 " + kindOf(value));
        }
        List<?> items = (List<?>) value;
        Object array = Array.newInstance(component, items.size());
        for (int i = 0; i < items.size(); i++) {
            Array.set(array, i, convert(items.get(i), component));
        }
        return (Object[]) array;
    }

    private Object convertCollection(Object value, Class<?> target,
                                     java.lang.reflect.Type genericHint) {
        if (!(value instanceof List)) {
            throw new IllegalArgumentException("集合参数必须是 JSON 数组，收到 " + kindOf(value));
        }
        Class<?> elementType = Object.class;
        if (genericHint instanceof java.lang.reflect.ParameterizedType) {
            java.lang.reflect.Type[] args =
                    ((java.lang.reflect.ParameterizedType) genericHint).getActualTypeArguments();
            if (args.length == 1 && args[0] instanceof Class) {
                elementType = (Class<?>) args[0];
            }
        }
        Collection<Object> collection;
        if (target.isInterface() || Modifier.isAbstract(target.getModifiers())) {
            collection = new ArrayList<>();
        } else {
            try {
                @SuppressWarnings("unchecked")
                Collection<Object> built =
                        (Collection<Object>) target.getDeclaredConstructor().newInstance();
                collection = built;
            } catch (ReflectiveOperationException e) {
                throw new IllegalArgumentException(
                        "集合类型无法实例化（需要 no-arg 构造）：" + target.getName());
            }
        }
        for (Object item : (List<?>) value) {
            collection.add(elementType == Object.class ? item
                    : convert(item, elementType, null));
        }
        return collection;
    }

    private Object convertMap(Object value, Class<?> target,
                              java.lang.reflect.Type genericHint) {
        if (!(value instanceof Map)) {
            throw new IllegalArgumentException("Map 参数必须是 JSON 对象，收到 " + kindOf(value));
        }
        Class<?> valueType = Object.class;
        if (genericHint instanceof java.lang.reflect.ParameterizedType) {
            java.lang.reflect.Type[] args =
                    ((java.lang.reflect.ParameterizedType) genericHint).getActualTypeArguments();
            if (args.length == 2 && args[1] instanceof Class) {
                valueType = (Class<?>) args[1];
            }
        }
        Map<?, ?> source = (Map<?, ?>) value;
        Map<Object, Object> out = new LinkedHashMap<>();
        for (Map.Entry<?, ?> entry : source.entrySet()) {
            out.put(entry.getKey(), valueType == Object.class ? entry.getValue()
                    : convert(entry.getValue(), valueType, null));
        }
        if (target.isInterface() || Modifier.isAbstract(target.getModifiers())) {
            return out;
        }
        try {
            @SuppressWarnings("unchecked")
            Map<Object, Object> built =
                    (Map<Object, Object>) target.getDeclaredConstructor().newInstance();
            built.putAll(out);
            return built;
        } catch (ReflectiveOperationException e) {
            throw new IllegalArgumentException(
                    "Map 类型无法实例化（需要 no-arg 构造）：" + target.getName());
        }
    }

    /** DTO 注入：public 字段优先，其次 setter，未知键记 warning。 */
    private Object populateBean(Map<?, ?> source, Class<?> target, int depth) {
        if (depth > MAX_DEPTH) {
            throw new IllegalArgumentException("DTO 嵌套超过深度上限 " + MAX_DEPTH);
        }
        Object bean;
        try {
            Constructor<?> ctor = target.getDeclaredConstructor();
            ctor.setAccessible(true);
            bean = ctor.newInstance();
        } catch (ReflectiveOperationException e) {
            throw new IllegalArgumentException(
                    "DTO 无法实例化（需要 no-arg 构造）：" + target.getName());
        }
        Class<?> cursor = target;
        while (cursor != null && cursor != Object.class) {
            for (Field field : cursor.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers())) {
                    continue;
                }
                Object raw = source.get(field.getName());
                if (raw == null && !source.containsKey(field.getName())) {
                    continue;
                }
                try {
                    field.setAccessible(true);
                    field.set(bean, convert(raw, field.getType(), field.getGenericType()));
                } catch (IllegalAccessException e) {
                    throw new IllegalArgumentException(
                            "字段注入失败：" + target.getName() + "." + field.getName());
                }
            }
            cursor = cursor.getSuperclass();
        }
        // setter 优先级高于同名字段注入（重跑一遍，语义对齐 JavaBean）。
        for (Method method : target.getMethods()) {
            String name = method.getName();
            if (!name.startsWith("set") || name.length() <= 3
                    || method.getParameterCount() != 1
                    || method.getDeclaringClass() == Object.class) {
                continue;
            }
            String property = Character.toLowerCase(name.charAt(3)) + name.substring(4);
            if (!source.containsKey(property)) {
                continue;
            }
            try {
                method.invoke(bean, convert(source.get(property),
                        method.getParameterTypes()[0], method.getGenericParameterTypes()[0]));
            } catch (ReflectiveOperationException e) {
                throw new IllegalArgumentException(
                        "setter 注入失败：" + target.getName() + "." + name);
            }
        }
        for (Object keyObj : source.keySet()) {
            String key = String.valueOf(keyObj);
            if (!beanHasProperty(target, key)) {
                warnings.add("DTO " + target.getName() + " 无属性：" + key + "（已忽略）");
            }
        }
        return bean;
    }

    private static boolean beanHasProperty(Class<?> type, String property) {
        Class<?> cursor = type;
        while (cursor != null && cursor != Object.class) {
            for (Field field : cursor.getDeclaredFields()) {
                if (field.getName().equals(property)) {
                    return true;
                }
            }
            cursor = cursor.getSuperclass();
        }
        for (Method method : type.getMethods()) {
            String name = method.getName();
            if (name.length() > 3 && (name.startsWith("set") || name.startsWith("get"))
                    && Character.toLowerCase(name.charAt(3)) == Character.toLowerCase(property.charAt(0))
                    && name.regionMatches(true, 4, property, 1, property.length() - 1)) {
                return true;
            }
        }
        return false;
    }

    static String kindOf(Object value) {
        if (value == null) {
            return "null";
        }
        if (value instanceof Map) {
            return "object";
        }
        if (value instanceof List) {
            return "array";
        }
        if (value instanceof String) {
            return "string";
        }
        if (value instanceof Number) {
            return "number";
        }
        if (value instanceof Boolean) {
            return "boolean";
        }
        return value.getClass().getName();
    }
}
