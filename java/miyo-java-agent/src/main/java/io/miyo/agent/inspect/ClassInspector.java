package io.miyo.agent.inspect;

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import io.miyo.agent.invoke.RiskPolicy;

/**
 * JVM Inspector：已加载类枚举 + 类 / 方法元信息收集。
 *
 * 边界（诚实声明）：Instrumentation.getAllLoadedClasses() 只能看到「已经加载」
 * 的类——懒加载类在首次使用前不可见；且这里发现的是类而非实例（实例解析
 * 策略见 invoke.InstanceResolver）。
 *
 * 方法枚举用 getDeclaredMethods（不含继承）+ 父类 public 方法补充；
 * 重载以参数擦除表区分；synthetic / bridge 标记透传（默认过滤）。
 */
public final class ClassInspector
        implements io.miyo.agent.spring.ContextDiscovery.ClassInspectorLike {

    private final Instrumentation instrumentation;

    public ClassInspector(Instrumentation instrumentation) {
        this.instrumentation = instrumentation;
    }

    /**
     * 枚举已加载类（过滤 + 搜索 + 排序 + 分页）。
     * 返回 {total, returned, items:[...]}；items 不含方法数（避免对数千类
     * 逐一 getDeclaredMethods 的开销，类详情按需取）。
     */
    public Map<String, Object> listClasses(String search, String include, String exclude,
                                           int limit, int offset, boolean includeJdk) {
        Class<?>[] all = instrumentation.getAllLoadedClasses();
        List<Class<?>> matched = new ArrayList<>();
        for (Class<?> cls : all) {
            String name = cls.getName();
            // 数组类 / 匿名类 / lambda 类默认展示但标注；合成 lambda 顶层类过滤
            if (!ClassFilter.matches(name, include, exclude) && !includeJdk) {
                continue;
            }
            if (!ClassFilter.searchMatches(name, search)) {
                continue;
            }
            matched.add(cls);
        }
        matched.sort(Comparator.comparing(Class::getName));
        int total = matched.size();
        int from = Math.max(0, offset);
        int to = Math.min(total, from + Math.max(1, limit));
        List<Map<String, Object>> items = new ArrayList<>();
        for (Class<?> cls : matched.subList(from, to)) {
            items.add(describeBrief(cls));
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("total", total);
        out.put("returned", items.size());
        out.put("offset", from);
        out.put("items", items);
        return out;
    }

    /** 类简要信息（列表行）。 */
    private Map<String, Object> describeBrief(Class<?> cls) {
        Map<String, Object> item = new LinkedHashMap<>();
        String name = cls.getName();
        item.put("name", name);
        Map<String, String> split = ClassFilter.splitName(name);
        item.put("package_name", split.get("package_name"));
        item.put("simple_name", split.get("simple_name"));
        item.put("jdk_internal", ClassFilter.isDefaultExcluded(name));
        item.put("array", cls.isArray());
        item.put("synthetic", cls.isSynthetic());
        ClassLoader loader = cls.getClassLoader();
        item.put("loader", loaderInfo(loader));
        return item;
    }

    /** 类详情（含方法数 / 父类 / 接口 / 修饰符）。 */
    public Map<String, Object> classDetail(String className) throws ClassNotFoundException {
        Class<?> cls = findLoadedClass(className);
        Map<String, Object> out = describeBrief(cls);
        ClassLoader loader = cls.getClassLoader();
        out.put("superclass", cls.getSuperclass() != null ? cls.getSuperclass().getName() : null);
        List<String> interfaces = new ArrayList<>();
        for (Class<?> it : cls.getInterfaces()) {
            interfaces.add(it.getName());
        }
        out.put("interfaces", interfaces);
        out.put("method_count", methodCount(cls));
        out.put("field_count", cls.getDeclaredFields().length);
        out.put("modifiers", Modifier.toString(cls.getModifiers()));
        out.put("interface", cls.isInterface());
        out.put("enum", cls.isEnum());
        out.put("annotation", cls.isAnnotation());
        out.put("proxy", java.lang.reflect.Proxy.isProxyClass(cls));
        out.put("loader_detail", loaderDetail(loader));
        return out;
    }

    /**
     * 类的可调用方法清单（public + protected + package 可见实例/静态方法；
     * private 默认包含但标注——反射可调，由风险策略与 UI 把关）。
     */
    public List<Map<String, Object>> listMethods(String className, boolean includePrivate)
            throws ClassNotFoundException {
        Class<?> cls = findLoadedClass(className);
        List<Map<String, Object>> out = new ArrayList<>();
        // 统计同名重载（含继承链）以便 UI 区分
        Map<String, Integer> nameCounts = new LinkedHashMap<>();
        for (Method m : allMethods(cls)) {
            nameCounts.merge(m.getName(), 1, Integer::sum);
        }
        for (Method m : allMethods(cls)) {
            boolean isPrivate = Modifier.isPrivate(m.getModifiers());
            if (isPrivate && !includePrivate) {
                continue;
            }
            if (m.isSynthetic()) {
                continue; // 编译器合成物（bridge / access$）：不作为可调入口展示
            }
            out.add(describeMethod(cls, m,
                    nameCounts.getOrDefault(m.getName(), 1) > 1));
        }
        return out;
    }

    /** 类可调用方法数（去重计数）。 */
    public int methodCount(Class<?> cls) {
        try {
            return allMethods(cls).size();
        } catch (Throwable e) {
            return -1; // 元信息不可用（个别类反射受限）：不毒化列表
        }
    }

    private List<Method> allMethods(Class<?> cls) {
        List<Method> out = new ArrayList<>();
        Class<?> cursor = cls;
        int depth = 0;
        while (cursor != null && cursor != Object.class && depth < 12) {
            try {
                out.addAll(Arrays.asList(cursor.getDeclaredMethods()));
            } catch (Throwable ignored) {
                // 个别类 getDeclaredMethods 抛错（如反射元数据受限）：跳过该层
            }
            cursor = cursor.getSuperclass();
            depth++;
        }
        return out;
    }

    private Map<String, Object> describeMethod(Class<?> owner, Method m, boolean overloaded) {
        Map<String, Object> out = new LinkedHashMap<>();
        int mods = m.getModifiers();
        StringBuilder params = new StringBuilder();
        List<Map<String, Object>> paramInfos = new ArrayList<>();
        Class<?>[] types = m.getParameterTypes();
        java.lang.reflect.Type[] genericTypes = m.getGenericParameterTypes();
        String[] paramNames = parameterNames(m);
        for (int i = 0; i < types.length; i++) {
            if (i > 0) {
                params.append(',');
            }
            params.append(types[i].getName());
            Map<String, Object> info = new LinkedHashMap<>();
            info.put("name", paramNames[i]);
            info.put("type", types[i].getName());
            info.put("generic", genericType(genericTypes[i]));
            info.put("primitive", types[i].isPrimitive());
            paramInfos.add(info);
        }
        String methodId = owner.getName() + "#" + m.getName()
                + (types.length == 0 ? "()" : "(" + params + ")");
        out.put("method_id", methodId);
        out.put("class_name", owner.getName());
        out.put("declaring_class", m.getDeclaringClass().getName());
        out.put("method_name", m.getName());
        out.put("signature", signatureOf(m));
        out.put("params", paramInfos);
        List<String> paramTypes = new ArrayList<>(types.length);
        for (Class<?> type : types) {
            paramTypes.add(type.getName());
        }
        out.put("param_types", paramTypes);
        out.put("return_type", m.getReturnType().getName());
        out.put("return_generic", genericType(m.getGenericReturnType()));
        out.put("static", Modifier.isStatic(mods));
        out.put("visibility", visibility(mods));
        out.put("synthetic", m.isSynthetic());
        out.put("bridge", m.isBridge());
        out.put("varargs", m.isVarArgs());
        out.put("overloaded", overloaded);
        out.put("risk", RiskPolicy.riskLevel(owner, m));
        out.put("risk_reason", RiskPolicy.riskReason(owner, m));
        return out;
    }

    private static String[] parameterNames(Method m) {
        int count = m.getParameterCount();
        String[] out = new String[count];
        java.lang.reflect.Parameter[] parameters = m.getParameters();
        for (int i = 0; i < count; i++) {
            // -parameters 编译带真名；缺省 arg{i}（Java 8+ Parameter.isNamePresent）
            out[i] = parameters[i].isNamePresent()
                    ? parameters[i].getName()
                    : ("arg" + i);
        }
        return out;
    }

    private static String genericType(java.lang.reflect.Type type) {
        if (type == null || type instanceof Class) {
            return ""; // 无泛型信息
        }
        return type.getTypeName();
    }

    private static String signatureOf(Method m) {
        StringBuilder sb = new StringBuilder();
        int mods = m.getModifiers();
        if (Modifier.isStatic(mods)) {
            sb.append("static ");
        }
        sb.append(m.getReturnType().getSimpleName()).append(' ')
                .append(m.getName()).append('(');
        Class<?>[] types = m.getParameterTypes();
        for (int i = 0; i < types.length; i++) {
            if (i > 0) {
                sb.append(", ");
            }
            sb.append(types[i].getSimpleName());
        }
        sb.append(')');
        return sb.toString();
    }

    private static String visibility(int mods) {
        if (Modifier.isPublic(mods)) {
            return "public";
        }
        if (Modifier.isProtected(mods)) {
            return "protected";
        }
        if (Modifier.isPrivate(mods)) {
            return "private";
        }
        return "package";
    }

    /** 已加载类查找（不触发加载——找不到说明类未加载， honest NOT_FOUND）。 */
    public Class<?> findLoadedClass(String name) {
        Class<?> cls = findLoadedClassOrNull(name);
        if (cls == null) {
            throw new io.miyo.agent.ipc.IpcException(io.miyo.agent.ipc.Protocol.ERR_NOT_FOUND,
                    "类未在目标 JVM 中加载（Instrumentation 只能看到已加载的类）：" + name
                            + "。可先触发该类的使用（调用一次接口 / 查看页面）后刷新。");
        }
        return cls;
    }

    /** 已加载类查找（可空返回；供 Spring 捕获等探测型调用）。 */
    public Class<?> findLoadedClassOrNull(String name) {
        if (name == null || name.trim().isEmpty()) {
            return null;
        }
        Class<?>[] all = instrumentation.getAllLoadedClasses();
        for (Class<?> cls : all) {
            if (cls.getName().equals(name)) {
                return cls;
            }
        }
        return null;
    }

    /** loader 摘要（列表行）。 */
    private static Map<String, Object> loaderInfo(ClassLoader loader) {
        Map<String, Object> out = new LinkedHashMap<>();
        if (loader == null) {
            out.put("id", "bootstrap");
            out.put("name", "bootstrap");
            out.put("identity", 0);
        } else {
            out.put("id", String.valueOf(System.identityHashCode(loader)));
            out.put("name", loader.getClass().getName());
            out.put("identity", System.identityHashCode(loader));
        }
        return out;
    }

    /** loader 详情（类详情页）。 */
    private static Map<String, Object> loaderDetail(ClassLoader loader) {
        Map<String, Object> out = loaderInfo(loader);
        if (loader != null) {
            ClassLoader parent = loader.getParent();
            out.put("parent", parent != null ? parent.getClass().getName() : "bootstrap");
        }
        return out;
    }
}
