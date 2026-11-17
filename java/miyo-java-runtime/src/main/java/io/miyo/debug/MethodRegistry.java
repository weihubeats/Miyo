package io.miyo.debug;

import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 白名单方法注册表：显式注册的类 / Bean → 可调用方法元数据。
 *
 * 注册即授权：只有进入本注册表的方法可被调试协议调用；危险类黑名单在
 * 注册期即拒绝。methodId 形如
 * {@code com.example.OrderService#createOrder(CreateOrderRequest,long)}
 * ——重载消歧唯一依据，与 Miyo 侧 protocol.rs 同规则。
 */
final class MethodRegistry {

    /** 单个注册项：目标类 + 实例提供方式 + 暴露的方法。 */
    static final class Registration {
        final String beanName;
        final Class<?> targetClass;
        final InstanceProvider provider;

        Registration(String beanName, Class<?> targetClass, InstanceProvider provider) {
            this.beanName = beanName;
            this.targetClass = targetClass;
            this.provider = provider;
        }
    }

    /** 参数描述（协议字段：name / typeName / genericType / required）。 */
    static final class ParamInfo {
        final String name;
        final String typeName;
        final String genericType;
        final boolean required;

        ParamInfo(String name, String typeName, String genericType, boolean required) {
            this.name = name;
            this.typeName = typeName;
            this.genericType = genericType;
            this.required = required;
        }

        Map<String, Object> toJson() {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("name", name);
            map.put("type_name", typeName);
            map.put("generic_type", genericType);
            map.put("required", required);
            return map;
        }
    }

    /** 方法描述（协议字段见 PROTOCOL v1）。 */
    static final class MethodInfo {
        final String methodId;
        final String className;
        final String methodName;
        final String signature;
        final String returnType;
        final String returnGeneric;
        final List<ParamInfo> params;
        final boolean staticMethod;
        final String visibility;
        final boolean overloaded;
        final String declaringClass;
        final Method method;
        final Registration registration;

        MethodInfo(String methodId, String className, String methodName, String signature,
                   String returnType, String returnGeneric, List<ParamInfo> params,
                   boolean staticMethod, String visibility, boolean overloaded,
                   String declaringClass, Method method, Registration registration) {
            this.methodId = methodId;
            this.className = className;
            this.methodName = methodName;
            this.signature = signature;
            this.returnType = returnType;
            this.returnGeneric = returnGeneric;
            this.params = params;
            this.staticMethod = staticMethod;
            this.visibility = visibility;
            this.overloaded = overloaded;
            this.declaringClass = declaringClass;
            this.method = method;
            this.registration = registration;
        }

        Map<String, Object> toJson() {
            List<Map<String, Object>> paramList = new ArrayList<>();
            for (ParamInfo p : params) {
                paramList.add(p.toJson());
            }
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("method_id", methodId);
            map.put("class_name", className);
            map.put("method_name", methodName);
            map.put("signature", signature);
            map.put("return_type", returnType);
            map.put("return_generic", returnGeneric);
            map.put("params", paramList);
            map.put("static_method", staticMethod);
            map.put("visibility", visibility);
            map.put("overloaded", overloaded);
            map.put("declaring_class", declaringClass);
            return map;
        }
    }

    private final List<Registration> registrations = new ArrayList<>();
    /** methodId → MethodInfo（构建后只读）。 */
    private Map<String, MethodInfo> methods = new LinkedHashMap<>();

    /** 注册类（实例由 provider 解析；静态方法无需实例）。 */
    synchronized void register(String beanName, Class<?> targetClass, InstanceProvider provider) {
        if (targetClass == null) {
            throw new IllegalArgumentException("注册类不能为空");
        }
        String fqn = targetClass.getName();
        if (SecurityPolicy.blacklisted(fqn)) {
            throw new IllegalArgumentException("拒绝注册受限类：" + fqn);
        }
        if (provider == null) {
            throw new IllegalArgumentException("InstanceProvider 不能为空（静态类用 InstanceProvider.staticInstance()）");
        }
        registrations.add(new Registration(beanName, targetClass, provider));
        rebuild();
    }

    private synchronized void rebuild() {
        Map<String, MethodInfo> next = new LinkedHashMap<>();
        List<MethodInfo> collected = new ArrayList<>();
        for (Registration reg : registrations) {
            for (Method m : visibleMethods(reg.targetClass)) {
                collected.add(buildMethodInfo(m, reg));
            }
        }
        // 重载标记：同名（同类）方法 >1 → overloaded。
        Map<String, Integer> nameCounts = new LinkedHashMap<>();
        for (MethodInfo info : collected) {
            String key = info.className + "#" + info.methodName;
            nameCounts.merge(key, 1, Integer::sum);
        }
        for (MethodInfo info : collected) {
            String key = info.className + "#" + info.methodName;
            boolean overloaded = nameCounts.getOrDefault(key, 0) > 1;
            MethodInfo withFlag = new MethodInfo(info.methodId, info.className, info.methodName,
                    info.signature, info.returnType, info.returnGeneric, info.params,
                    info.staticMethod, info.visibility, overloaded, info.declaringClass,
                    info.method, info.registration);
            MethodInfo previous = next.get(withFlag.methodId);
            // methodId 相同（同名同参）出现两次：父类 / 接口重复暴露，保留首个。
            if (previous == null) {
                next.put(withFlag.methodId, withFlag);
            }
        }
        this.methods = next;
    }

    /** 类对外暴露的方法：public + 非 Object 基础方法 + 非合成。 */
    private static List<Method> visibleMethods(Class<?> target) {
        List<Method> out = new ArrayList<>();
        for (Method m : target.getMethods()) {
            int mods = m.getModifiers();
            if (!Modifier.isPublic(mods) || m.isSynthetic() || m.isBridge()) {
                continue;
            }
            if (m.getDeclaringClass() == Object.class) {
                continue;
            }
            String name = m.getName();
            if ("getClass".equals(name) || "wait".equals(name) || "notify".equals(name)
                    || "notifyAll".equals(name) || "hashCode".equals(name)
                    || "equals".equals(name) || "toString".equals(name)) {
                continue;
            }
            out.add(m);
        }
        out.sort(Comparator.comparing(Method::getName).thenComparing(m -> m.getParameterCount()));
        return out;
    }

    private static MethodInfo buildMethodInfo(Method m, Registration reg) {
        Class<?> declaring = m.getDeclaringClass();
        String className = declaring.getName();
        List<String> typeNames = new ArrayList<>();
        for (Class<?> p : m.getParameterTypes()) {
            typeNames.add(simpleName(p));
        }
        String methodId = className + "#" + m.getName() + "(" + String.join(",", typeNames) + ")";

        List<ParamInfo> params = new ArrayList<>();
        String[] paramNames = m.getParameterCount() == 0 ? new String[0] : parameterNames(m);
        Class<?>[] paramTypes = m.getParameterTypes();
        java.lang.reflect.Type[] genericTypes = m.getGenericParameterTypes();
        for (int i = 0; i < paramTypes.length; i++) {
            boolean required = paramTypes[i].isPrimitive();
            String name = i < paramNames.length && paramNames[i] != null
                    ? paramNames[i] : ("arg" + i);
            params.add(new ParamInfo(name, simpleName(paramTypes[i]),
                    displayType(genericTypes[i]), required));
        }

        StringBuilder sig = new StringBuilder();
        sig.append(Modifier.isStatic(m.getModifiers()) ? "static " : "public ");
        sig.append(m.getName()).append('(');
        for (int i = 0; i < paramTypes.length; i++) {
            if (i > 0) {
                sig.append(", ");
            }
            sig.append(simpleName(paramTypes[i]));
        }
        sig.append(") -> ").append(displayType(m.getGenericReturnType()));

        return new MethodInfo(
                methodId,
                className,
                m.getName(),
                sig.toString(),
                simpleName(m.getReturnType()),
                displayType(m.getGenericReturnType()),
                params,
                Modifier.isStatic(m.getModifiers()),
                Modifier.isPublic(m.getModifiers()) ? "public" : "protected",
                false,
                declaring.getName(),
                m,
                reg);
    }

    /** -parameters 编译参数名；缺失返回空数组（回退 arg0..n）。 */
    private static String[] parameterNames(Method m) {
        java.lang.reflect.Parameter[] ps = m.getParameters();
        String[] names = new String[ps.length];
        for (int i = 0; i < ps.length; i++) {
            if (ps[i].isNamePresent()) {
                names[i] = ps[i].getName();
            }
        }
        return names;
    }

    /** 擦除简单名（java.util.List → List，基本类型原样）。 */
    static String simpleName(Class<?> type) {
        if (type.isArray()) {
            return simpleName(type.getComponentType()) + "[]";
        }
        String name = type.getName();
        if (name.indexOf('.') >= 0) {
            String trimmed = type.getSimpleName();
            return trimmed.isEmpty() ? name : trimmed;
        }
        return name;
    }

    /** 泛型展示（List&lt;Long&gt; → "List<Long>"；无泛型退化为擦除名）。 */
    static String displayType(java.lang.reflect.Type type) {
        String raw = String.valueOf(type);
        int lt = raw.indexOf('<');
        if (lt > 0) {
            String base = raw.substring(0, lt);
            int dot = base.lastIndexOf('.');
            String baseSimple = dot >= 0 ? base.substring(dot + 1) : base;
            String args = raw.substring(lt);
            return baseSimple + args;
        }
        int dot = raw.lastIndexOf('.');
        return dot >= 0 && !raw.isEmpty() && raw.charAt(dot + 1) != 0 ? raw.substring(dot + 1) : raw;
    }

    /** 按 methodId 查找方法；未注册返回 null。 */
    synchronized MethodInfo find(String methodId) {
        return methods.get(methodId);
    }

    /** 全部已注册方法（顺序稳定）。 */
    synchronized List<MethodInfo> all() {
        List<MethodInfo> list = new ArrayList<>(methods.values());
        return Collections.unmodifiableList(list);
    }

    /** 白名单类清单（聚合：类 FQN / 包名 / 简名 / 方法数 / Bean 名）。 */
    synchronized List<Map<String, Object>> classesJson() {
        Map<String, Map<String, Object>> classes = new LinkedHashMap<>();
        for (MethodInfo info : methods.values()) {
            Map<String, Object> entry = classes.computeIfAbsent(info.className, k -> {
                Map<String, Object> map = new LinkedHashMap<>();
                int dot = k.lastIndexOf('.');
                map.put("class_name", k);
                map.put("package_name", dot >= 0 ? k.substring(0, dot) : "");
                map.put("simple_name", dot >= 0 ? k.substring(dot + 1) : k);
                map.put("method_count", 0);
                map.put("bean_name", null);
                return map;
            });
            entry.put("method_count", ((Number) entry.get("method_count")).intValue() + 1);
            if (entry.get("bean_name") == null && info.registration.beanName != null) {
                entry.put("bean_name", info.registration.beanName);
            }
        }
        return new ArrayList<>(classes.values());
    }

    synchronized int methodCount() {
        return methods.size();
    }

    synchronized int classCount() {
        return classesJson().size();
    }
}
