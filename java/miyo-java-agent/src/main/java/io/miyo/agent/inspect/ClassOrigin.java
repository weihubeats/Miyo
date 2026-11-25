package io.miyo.agent.inspect;

import java.lang.instrument.Instrumentation;
import java.net.URI;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * 类来源解析（业务类 / 依赖类判定的第一手证据）。
 *
 * 三个维度（互相独立，按证据强度排列）：
 * 1. {@link CodeSource}：类的 ProtectionDomain 里记录的 codeSource URL——
 *    与「类从哪个文件加载」直接对应，是唯一不可伪造的来源证据；
 * 2. {@link ClassLoader}：来源的**辅助**信息（只用于分组提示，绝不单独定性——
 *    Boot 的 LaunchedURLClassLoader 同时加载 BOOT-INF/classes 与 BOOT-INF/lib）；
 * 3. JVM 启动参数：sun.java.command / java.class.path / classpath manifest
 *    （见 {@link LaunchInfo}）。
 *
 * Java 8 兼容实现（Docker 镜像里 JDK 8 仍是主流目标）。所有 reflection 面都
 * 用「已加载类 + 反射」探测，避免在目标 JVM 里触发额外的类加载。
 */
public final class ClassOrigin {

    /** 来源种类。 */
    public enum Kind {
        /** JDK / JVM 自身（bootstrap 或 jrt:/ 模块）。 */
        JDK,
        /** 应用自身的 classes 目录（IDEA out/*、Maven target/classes、gradle build/classes）。 */
        APP_CLASSES_DIR,
        /** 应用自身的模块化产物 / 非 fat-jar 普通 jar。 */
        APP_JAR,
        /** Spring Boot / WAR 的嵌套目录（BOOT-INF/classes、WEB-INF/classes）。 */
        NESTED_CLASSES,
        /** 第三方依赖（BOOT-INF/lib、WEB-INF/lib、本地仓库 / gradle 缓存 jar）。 */
        NESTED_LIB,
        /** 无法判定（无 codeSource / 未知协议 / 异常）。 */
        UNKNOWN;

        /** 该来源是否证明「这是被调试应用自己的代码」。 */
        public boolean isApplicationOwned() {
            return this == APP_CLASSES_DIR || this == APP_JAR || this == NESTED_CLASSES;
        }
    }

    /** 来源详情（每个类一份；按 codeSource URL 缓存，见 ClassIndex）。 */
    public static final class Source {
        public final Kind kind;
        /** 原始 codeSource 位置（可空）。 */
        public final String location;
        /** 归一后的可读标签：目录 / 文件名。 */
        public final String label;
        /** 证据说明（UI 展示「为什么这样归类」）。 */
        public final String evidence;

        Source(Kind kind, String location, String label, String evidence) {
            this.kind = kind;
            this.location = location;
            this.label = label;
            this.evidence = evidence;
        }

        public static Source of(Kind kind, String location, String label, String evidence) {
            return new Source(kind, location, label, evidence);
        }

        public Map<String, Object> toMap() {
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            out.put("kind", kind.name().toLowerCase(Locale.ROOT));
            out.put("location", location);
            out.put("label", label);
            out.put("evidence", evidence);
            return out;
        }

        @Override
        public String toString() {
            return kind + "(" + label + ")";
        }
    }

    /** 常见目录名：容器（fat-jar）内的应用 classes 目录。 */
    private static final String[] NESTED_CLASSES_MARKERS = {
            "BOOT-INF/classes/", "WEB-INF/classes/", "classes/", // 裸 tail（按尾段匹配，见 matchesNested）
    };

    /** 常见目录名：容器内的依赖目录。 */
    private static final String[] NESTED_LIB_MARKERS = {
            "BOOT-INF/lib/", "WEB-INF/lib/", "META-INF/lib/",
    };

    private ClassOrigin() {
    }

    /**
     * 解析类的来源。只读 ProtectionDomain（不触发任何类初始化）。
     * 失败一律降级为 {@link Kind#UNKNOWN}，绝不猜测。
     */
    public static Source resolve(Class<?> cls) {
        String testPrefix = testApplicationPrefix;
        if (testPrefix != null && cls.getName().startsWith(testPrefix)) {
            return Source.of(Kind.APP_CLASSES_DIR, null, "test-classes",
                    "测试夹具（按测试目录模拟应用 classes：surefire 打包会改变 codeSource）");
        }
        try {
            ClassLoader loader = cls.getClassLoader();
            if (loader == null) {
                // bootstrap 加载 = JDK 核心；但 jrt 协议也会带 codeSource，下面再细化
                ProtectionDomain pd = safeProtectionDomain(cls);
                if (pd != null && pd.getCodeSource() != null
                        && pd.getCodeSource().getLocation() != null) {
                    return classifyUrl(pd.getCodeSource().getLocation().toString());
                }
                return Source.of(Kind.JDK, null, "jvm-bootstrap",
                        "类由 bootstrap classloader 加载（loader=null），属 JVM 核心");
            }
            ProtectionDomain pd = safeProtectionDomain(cls);
            CodeSource cs = pd == null ? null : pd.getCodeSource();
            if (cs == null || cs.getLocation() == null) {
                return Source.of(Kind.UNKNOWN, null, "no-codesource",
                        "ProtectionDomain 无 codeSource（动态生成 / 加密加载器 / 受限策略）");
            }
            return classifyUrl(cs.getLocation().toString());
        } catch (Throwable t) {
            return Source.of(Kind.UNKNOWN, null, "unresolved",
                    "来源解析异常：" + t.getClass().getSimpleName());
        }
    }

    private static ProtectionDomain safeProtectionDomain(Class<?> cls) {
        try {
            return cls.getProtectionDomain();
        } catch (Throwable t) {
            return null; // SecurityManager / 受限策略：降级
        }
    }

    /**
     * 按 codeSource URL 分类。URL 形态覆盖：
     * file:/dir/（目录）、file:/x.jar（普通 jar）、jar:file:/x.jar!/BOOT-INF/classes/、
     * jar:file:/x.jar!/BOOT-INF/lib/a.jar!/、jrt:/java.base/、bundleresource/ 等。
     */
    public static Source classifyUrl(String url) {
        String lower = url.toLowerCase(Locale.ROOT);
        String fileName = fileNameOf(url);
        try {
            // 嵌套容器：`!/` 是 Java 8~Spring Boot 2 的形态；Spring Boot 3.2+ 改用
            // `nested:/path/app.jar/!BOOT-INF/lib/x.jar`（`!` 后无 `/`），两者都要认。
            int bang = lower.indexOf('!');
            if (bang >= 0) {
                // 「!之后的段」决定 classes 还是 lib
                String inner = trimNestedSeparators(lower.substring(bang + 1));
                String outer = url.substring(0, bang);
                String outerName = fileNameOf(outer);
                for (String marker : NESTED_LIB_MARKERS) {
                    if (inner.startsWith(marker)) {
                        return Source.of(Kind.NESTED_LIB, url, fileName,
                                "嵌套容器依赖路径 " + marker + "（外层 " + outerName + "）");
                    }
                }
                for (String marker : NESTED_CLASSES_MARKERS) {
                    if (!marker.equals("classes/") && inner.startsWith(marker)) {
                        return Source.of(Kind.NESTED_CLASSES, url, fileName,
                                "嵌套容器应用路径 " + marker + "（外层 " + outerName + "）");
                    }
                }
                // Boot 的 URL 常以 "!/" 结尾（BOOT-INF/classes!/）：trimNestedSeparators
                // 已去掉末尾分隔符（"classes!/" → "classes"）
                if (inner.equals("classes") || inner.endsWith("/classes")
                        || inner.contains("classes!/")) {
                    return Source.of(Kind.NESTED_CLASSES, url, fileName,
                            "嵌套容器应用路径 classes（外层 " + outerName + "）");
                }
                // 嵌套 jar 自身（nested:/app.jar/!BOOT-INF/lib/x.jar）：末段是 jar，
                // 归依赖——它一定来自容器的 lib 目录（或更深的 lib 链）
                if (inner.endsWith(".jar") || inner.endsWith(".zip")) {
                    return Source.of(Kind.NESTED_LIB, url, fileName,
                            "嵌套容器内 jar（" + fileName + "，外层 " + outerName + "）");
                }
                // 更深的嵌套（a.jar!/lib/b.jar!/）仍按 lib 处理
                if (inner.contains("lib/")) {
                    return Source.of(Kind.NESTED_LIB, url, fileName,
                            "嵌套容器依赖路径（外层 " + outerName + "）");
                }
                return Source.of(Kind.UNKNOWN, url, fileName,
                        "嵌套容器路径未匹配已知布局（外层 " + outerName + "）");
            }
            if (lower.startsWith("jrt:")) {
                return Source.of(Kind.JDK, url, "java.base",
                        "jrt: 模块镜像（JDK 9+ 运行时类）");
            }
            if (lower.contains("/modules/") && lower.endsWith("x-java")) {
                return Source.of(Kind.JDK, url, "jimage",
                        "jimage 模块镜像（JDK 运行时）");
            }
            if (lower.startsWith("file:")) {
                String path = stripProtocol(url);
                String p = path.toLowerCase(Locale.ROOT);
                if (p.endsWith(".jar") || p.endsWith(".zip") || p.endsWith(".war")) {
                    if (pathLooksLikeDependencyJar(p)) {
                        return Source.of(Kind.NESTED_LIB, url, fileName,
                                "普通 jar，路径特征命中依赖仓库 / 缓存（" + fileName + "）");
                    }
                    return Source.of(Kind.APP_JAR, url, fileName,
                            "普通 jar，路径不含依赖仓库特征（" + fileName + "）");
                }
                if (isClassesDir(p)) {
                    return Source.of(Kind.APP_CLASSES_DIR, url, fileName,
                            "目录型 classes 输出（" + tail(fileName, 3) + "）");
                }
                if (p.endsWith("/")) {
                    // 未知目录：可能是 classes 也可能是资源目录——无法证明，标 UNKNOWN
                    return Source.of(Kind.UNKNOWN, url, fileName,
                            "目录型来源但非标准 classes 输出目录");
                }
                return Source.of(Kind.UNKNOWN, url, fileName, "文件型来源，协议 file: 但无法归类");
            }
            return Source.of(Kind.UNKNOWN, url, fileName,
                    "未知 codeSource 协议（" + protocolOf(url) + "）");
        } catch (Throwable t) {
            return Source.of(Kind.UNKNOWN, url, fileName, "分类异常：" + t.getMessage());
        }
    }

    /** 去掉嵌套 URL 段两侧的 `!` 与 `/`（`/!BOOT-INF/` → `boot-inf/`）。 */
    static String trimNestedSeparators(String inner) {
        String s = inner;
        while (s.startsWith("!/") || s.startsWith("/!")) {
            s = s.substring(2);
        }
        if (s.startsWith("!") || s.startsWith("/")) {
            s = s.substring(1);
        }
        while (s.endsWith("!/")) {
            s = s.substring(0, s.length() - 2);
        }
        if (s.endsWith("!")) {
            s = s.substring(0, s.length() - 1);
        }
        if (s.endsWith("/")) {
            s = s.substring(0, s.length() - 1);
        }
        return s;
    }

    /**
     * 路径是否像「依赖 jar」：命中本地仓库 / 构建缓存 / 常见中间件目录。
     * 只是**加分证据**（用于普通 jar 的 APP_JAR / NESTED_LIB 区分）。
     */
    static boolean pathLooksLikeDependencyJar(String lowerPath) {
        String[] hints = {
                "/.m2/",              // Maven 本地仓库（~/.m2/repository/...）
                "/.gradle/caches/", "/.ivy2/", "/.sbt/",
                "/lib/",              // 传统 lib 目录（部署包 / WEB-INF/lib 等）
                "/repository/",       // Nexus / Artifactory 缓存路径
                "/node_modules/",
        };
        for (String hint : hints) {
            if (lowerPath.contains(hint)) {
                return true;
            }
        }
        return false;
    }

    /** 目录是否标准 classes 输出目录。 */
    static boolean isClassesDir(String lowerPath) {
        String p = lowerPath.endsWith("/") ? lowerPath.substring(0, lowerPath.length() - 1) : lowerPath;
        if (p.equalsIgnoreCase("classes")) {
            return true;
        }
        return p.endsWith("/classes")
                || p.endsWith("/classes/java/main")
                || p.endsWith("/classes/kotlin/main")
                || p.endsWith("/classes/java/test")
                || p.endsWith("/classes/kotlin/test")
                || p.endsWith("/out/production")
                || p.endsWith("/out/test")
                || p.endsWith("/build/classes")
                || p.endsWith("/target/classes")
                || p.contains("/out/production/")
                || p.contains("/out/test/");
    }

    private static String stripProtocol(String url) {
        String path = url;
        int scheme = path.indexOf("://");
        if (scheme >= 0) {
            path = path.substring(scheme + 3);
        } else if (path.startsWith("file:")) {
            path = path.substring("file:".length());
        }
        try {
            return decodeUri(path);
        } catch (Throwable t) {
            return path;
        }
    }

    /** %XX 解码（仅处理路径中常见的编码，避免依赖 java.net.URLDecoder 的 + 语义）。 */
    private static String decodeUri(String raw) {
        if (raw.indexOf('%') < 0) {
            return raw;
        }
        StringBuilder sb = new StringBuilder(raw.length());
        for (int i = 0; i < raw.length(); i++) {
            char c = raw.charAt(i);
            if (c == '%' && i + 2 < raw.length()) {
                try {
                    sb.append((char) Integer.parseInt(raw.substring(i + 1, i + 3), 16));
                    i += 2;
                    continue;
                } catch (NumberFormatException ignored) {
                    // 保留原样
                }
            }
            sb.append(c);
        }
        return sb.toString();
    }

    /** 取末段（文件 / 目录名）。 */
    static String fileNameOf(String url) {
        if (url == null || url.isEmpty()) {
            return "";
        }
        String s = url;
        if (s.endsWith("/")) {
            s = s.substring(0, s.length() - 1);
        }
        if (s.endsWith("!/")) {
            s = s.substring(0, s.length() - 2);
        }
        int slash = Math.max(s.lastIndexOf('/'), s.lastIndexOf('\\'));
        String name = slash >= 0 ? s.substring(slash + 1) : s;
        return name.isEmpty() ? s : name;
    }

    private static String protocolOf(String url) {
        int idx = url.indexOf(':');
        return idx > 0 ? url.substring(0, idx) : "unknown";
    }

    private static String tail(String path, int segments) {
        if (path == null || path.isEmpty()) {
            return "";
        }
        List<String> parts = new ArrayList<String>();
        for (String part : path.split("[/\\\\]")) {
            if (!part.isEmpty()) {
                parts.add(part);
            }
        }
        StringBuilder sb = new StringBuilder();
        int from = Math.max(0, parts.size() - segments);
        for (int i = from; i < parts.size(); i++) {
            if (sb.length() > 0) {
                sb.append('/');
            }
            sb.append(parts.get(i));
        }
        return sb.toString();
    }

    // ------------------------------------------------------------------
    // JVM 启动信息（Main-Class / Start-Class / classpath）
    // ------------------------------------------------------------------

    /**
     * JVM 启动信息（进程级单例；全部来自 System.getProperty，零副作用）。
     *
     * - 可执行 jar：Manifest 的 Start-Class / Main-Class（Spring Boot 的
     *   Start-Class 才是业务入口）→ 业务包根；
     * - 普通 main：sun.java.command 的首段是主类；
     * - classpath：java.class.path / LauncherHelper 的 MANIFEST classpath——
     *   用于识别「应用自己的 classes 目录 / jar」（非 Spring Boot 的普通应用）。
     *
     * Java 9+ 对模块化应用 java.class.path 可能为组装模块路径，可能为空——
     * 所有字段都可能缺失，调用方必须能接受 empty。
     */
    public static final class LaunchInfo {

        /** 主类（Start-Class 优先）。 */
        public final String mainClass;
        /** Spring Boot Start-Class（可空）。 */
        public final String startClass;
        /** Manifest Main-Class（可空）。 */
        public final String manifestMainClass;
        /** 应用 jar 位置（可空）。 */
        public final String appLocation;
        /** 应用自身 classpath 条目（目录 / jar；绝对归一化后）。 */
        public final List<String> classpathEntries;
        /** 证据说明（UI 展示）。 */
        public final List<String> evidence;

        LaunchInfo(String mainClass, String startClass, String manifestMainClass,
                   String appLocation, List<String> classpathEntries, List<String> evidence) {
            this.mainClass = mainClass;
            this.startClass = startClass;
            this.manifestMainClass = manifestMainClass;
            this.appLocation = appLocation;
            this.classpathEntries = classpathEntries;
            this.evidence = evidence;
        }

        static final LaunchInfo EMPTY = new LaunchInfo(null, null, null, null,
                new ArrayList<String>(), new ArrayList<String>());

        public Map<String, Object> toMap() {
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            out.put("main_class", mainClass);
            out.put("start_class", startClass);
            out.put("manifest_main_class", manifestMainClass);
            out.put("app_location", appLocation);
            out.put("classpath_entries", classpathEntries);
            out.put("evidence", evidence);
            return out;
        }

        @Override
        public String toString() {
            return "LaunchInfo{main=" + mainClass + ", start=" + startClass
                    + ", cp=" + classpathEntries.size() + "}";
        }
    }

    /** 解析 JVM 启动信息（缓存一次——启动参数在 JVM 生命周期内不变）。 */
    public static LaunchInfo launchInfo() {
        if (launchInfoCache != null) {
            return launchInfoCache;
        }
        synchronized (ClassOrigin.class) {
            if (launchInfoCache == null) {
                launchInfoCache = parseLaunchInfo();
            }
            return launchInfoCache;
        }
    }

    private static volatile LaunchInfo launchInfoCache;

    /** 供测试注入（不污染真实 JVM 信息）。 */
    static void setLaunchInfoForTests(LaunchInfo info) {
        launchInfoCache = info;
    }

    /**
     * 测试钩子：surefire 会把测试类打进一个 jar（target/test-classes 打成的
     * 临时 jar），其 codeSource 与「真实 IDEA / Maven 运行时的目录」不一致。
     * 测试用这个名字前缀把自己的夹具视为「应用 classes 目录」。
     */
    private static volatile String testApplicationPrefix;

    static void setTestApplicationPrefixForTests(String prefix) {
        testApplicationPrefix = prefix;
    }

    static String testApplicationPrefix() {
        return testApplicationPrefix;
    }

    private static LaunchInfo parseLaunchInfo() {
        List<String> evidence = new ArrayList<String>();
        String mainClass = null;
        String startClass = null;
        String manifestMain = null;
        String appLocation = null;

        String sunCommand = System.getProperty("sun.java.command");
        if (sunCommand != null && !sunCommand.trim().isEmpty()) {
            String first = sunCommand.trim().split("\\s+")[0];
            if (first.endsWith(".jar") || first.endsWith(".war")) {
                appLocation = first;
                evidence.add("sun.java.command 首段是可执行 jar：" + first);
            } else if (first.contains(".")) {
                mainClass = first;
                evidence.add("sun.java.command 首段是主类：" + first);
            }
        }

        // javaw / -jar 启动时未设置 sun.java.command（少见）：启动脚本回退
        String commandLine = System.getProperty("sun.java.launcher.command_line");
        if (mainClass == null && commandLine != null) {
            String[] parts = commandLine.trim().split("\\s+");
            for (int i = 0; i < parts.length; i++) {
                if ("-jar".equals(parts[i]) && i + 1 < parts.length) {
                    appLocation = parts[i + 1];
                    evidence.add("launcher 命令行 -jar：" + parts[i + 1]);
                    break;
                }
            }
        }

        List<String> classpath = new ArrayList<String>();
        String cp = System.getProperty("java.class.path");
        if (cp != null && !cp.trim().isEmpty()) {
            for (String entry : cp.split(java.io.File.pathSeparator)) {
                if (!entry.trim().isEmpty()) {
                    classpath.add(entry.trim());
                }
            }
            evidence.add("java.class.path 条目数：" + classpath.size());
        }

        // 可执行 jar 的 Manifest（Start-Class / Main-Class / Class-Path）
        String jarForManifest = appLocation != null && appLocation.endsWith(".war")
                ? null : appLocation;
        if (jarForManifest != null) {
            Map<String, String> manifest = readManifest(jarForManifest);
            if (manifest != null) {
                String bootStart = manifest.get("Start-Class");
                if (bootStart != null && !bootStart.trim().isEmpty()) {
                    startClass = bootStart.trim();
                    evidence.add("Manifest Start-Class：" + startClass + "（Spring Boot）");
                }
                String mMain = manifest.get("Main-Class");
                if (mMain != null && !mMain.trim().isEmpty()) {
                    manifestMain = mMain.trim();
                    evidence.add("Manifest Main-Class：" + manifestMain);
                }
                String mCp = manifest.get("Class-Path");
                if (mCp != null && !mCp.trim().isEmpty()) {
                    for (String rel : mCp.trim().split("\\s+")) {
                        if (!rel.isEmpty()) {
                            classpath.add(rel);
                        }
                    }
                    evidence.add("Manifest Class-Path 条目数 +" + mCp.trim().split("\\s+").length);
                }
            }
        }

        if (startClass != null) {
            mainClass = startClass;
        } else if (mainClass == null) {
            mainClass = manifestMain;
        }
        if (appLocation == null && manifestMain != null) {
            // 无 -jar 信息：不臆造位置
            appLocation = null;
        }
        return new LaunchInfo(mainClass, startClass, manifestMain, appLocation,
                classpath, evidence);
    }

    /** 读 jar manifest（只读 zip 目录，不加载类；失败静默返回 null）。 */
    private static Map<String, String> readManifest(String path) {
        java.util.zip.ZipFile zip = null;
        try {
            java.io.File file = new java.io.File(path);
            if (!file.isFile()) {
                return null;
            }
            zip = new java.util.zip.ZipFile(file);
            java.util.zip.ZipEntry entry = zip.getEntry("META-INF/MANIFEST.MF");
            if (entry == null) {
                return null;
            }
            return parseManifest(zip.getInputStream(entry));
        } catch (Throwable t) {
            return null;
        } finally {
            if (zip != null) {
                try {
                    zip.close();
                } catch (Throwable ignored) {
                    // 关闭失败无影响
                }
            }
        }
    }

    /** Manifest 解析（续行以空格开头，RFC 822）。 */
    static Map<String, String> parseManifest(java.io.InputStream in) throws java.io.IOException {
        Map<String, String> out = new LinkedHashMap<String, String>();
        java.io.BufferedReader reader = new java.io.BufferedReader(
                new java.io.InputStreamReader(in, "UTF-8"));
        String line;
        String lastKey = null;
        StringBuilder lastValue = new StringBuilder();
        while ((line = reader.readLine()) != null) {
            if (line.isEmpty()) {
                // 空行 = main section 结束；后续是按条目分组的 section，不属于全局属性
                if (lastKey != null) {
                    out.put(lastKey, lastValue.toString().trim());
                    lastKey = null;
                }
                break;
            }
            if (line.startsWith(" ")) {
                if (lastKey != null) {
                    lastValue.append(line.substring(1));
                }
                continue;
            }
            if (lastKey != null) {
                out.put(lastKey, lastValue.toString().trim());
            }
            int idx = line.indexOf(':');
            if (idx <= 0) {
                lastKey = null;
                lastValue = new StringBuilder();
                continue;
            }
            lastKey = line.substring(0, idx);
            lastValue = new StringBuilder(line.substring(idx + 1).trim());
        }
        if (lastKey != null) {
            out.put(lastKey, lastValue.toString().trim());
        }
        return out;
    }

    /** 系统的 sun.java.command（诊断展示用）。 */
    public static String sunJavaCommand() {
        String v = System.getProperty("sun.java.command");
        return v == null ? "" : v;
    }

    /**
     * 目标的 JVM 基本信息快照（一次反射 + 属性读取；诊断面板展示用）。
     */
    public static Map<String, Object> jvmSummary(Instrumentation inst, int loadedClassCount) {
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        try {
            java.lang.management.RuntimeMXBean runtime =
                    java.lang.management.ManagementFactory.getRuntimeMXBean();
            out.put("jvm_name", runtime.getVmName());
            out.put("jvm_vendor", runtime.getVmVendor());
            out.put("jvm_version", runtime.getVmVersion());
            out.put("spec_version", runtime.getSpecVersion());
            out.put("input_arguments", runtime.getInputArguments());
            out.put("class_path", runtime.getClassPath());
            out.put("start_time", runtime.getStartTime());
            out.put("uptime_ms", runtime.getUptime());
        } catch (Throwable t) {
            out.put("jvm_name", "unknown");
            out.put("error", t.getClass().getSimpleName() + ": " + t.getMessage());
        }
        out.put("sun_java_command", sunJavaCommand());
        out.put("loaded_class_count", loadedClassCount);
        out.put("instrumentation", inst != null);
        if (inst != null) {
            try {
                out.put("retransform_supported", inst.isRetransformClassesSupported());
                out.put("redefine_supported", inst.isRedefineClassesSupported());
            } catch (Throwable ignored) {
                // 平台不支持查询：不阻塞
            }
        }
        return out;
    }
}