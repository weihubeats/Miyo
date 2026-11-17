package io.miyo.debug;

import java.util.Arrays;
import java.util.List;

/**
 * 安全策略：危险类黑名单 + 体积 / 超时 / 并发 / 频率上限。
 *
 * 与 Miyo 侧（crates/miyo-java `policy.rs`）同规则双保险：Miyo 侧预检把
 * 明显违规的调用留在本地，本类是最终裁决方。黑名单原则：凡是「拿到即等于
 * 任意代码执行 / 进程逃逸 / JVM 自改 / 文件与网络触达」的能力一律拒绝——
 * 显式注册模型下用户不会主动注册这些类，兜底防误配与防协议滥用。
 */
final class SecurityPolicy {

    /** 禁止注册 / 调用的类（前缀匹配；与 Miyo 侧 BLACKLISTED_CLASSES 逐项一致）。 */
    static final List<String> BLACKLISTED_CLASSES = Arrays.asList(
            "java.lang.Runtime",
            "java.lang.ProcessBuilder",
            "java.lang.ProcessHandle",
            "java.lang.System",
            "java.lang.ClassLoader",
            "java.lang.SecurityManager",
            "java.lang.Thread",
            "java.lang.ThreadGroup",
            "java.lang.reflect.",
            "java.lang.invoke.",
            "javax.script.",
            "jdk.nashorn.",
            "jdk.dynalink.",
            "sun.misc.Unsafe",
            "java.lang.Module",
            "java.lang.ModuleLayer",
            "java.lang.instrument.",
            "java.io.File",
            "java.io.RandomAccessFile",
            "java.nio.file.Files",
            "java.nio.file.Paths",
            "java.net.Socket",
            "java.net.ServerSocket",
            "javax.net.ssl.SSLSocketFactory");

    /** 请求体上限（字节）——参数 JSON 的硬顶。 */
    final int maxArgsBytes;
    /** 返回值上限（字节）——超出截断并标记 resultTruncated。 */
    final int maxResultBytes;
    /** 单次调用超时上限（毫秒）。 */
    final long maxTimeoutMs;
    /** 单方法参数个数上限。 */
    final int maxParams;
    /** 最大并发调用数。 */
    final int maxConcurrent;
    /** 每分钟最大调用次数。 */
    final int maxCallsPerMinute;

    SecurityPolicy(int maxArgsBytes, int maxResultBytes, long maxTimeoutMs,
                   int maxParams, int maxConcurrent, int maxCallsPerMinute) {
        this.maxArgsBytes = maxArgsBytes;
        this.maxResultBytes = maxResultBytes;
        this.maxTimeoutMs = maxTimeoutMs;
        this.maxParams = maxParams;
        this.maxConcurrent = maxConcurrent;
        this.maxCallsPerMinute = maxCallsPerMinute;
    }

    /** 黑名单前缀匹配（大小写敏感——Java 类名本就大小写敏感）。 */
    static boolean blacklisted(String className) {
        for (String prefix : BLACKLISTED_CLASSES) {
            if (className.startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    /** 默认策略（与 Miyo 侧 Default 对齐）。 */
    static SecurityPolicy defaults() {
        return new SecurityPolicy(
                1024 * 1024,      // 1 MiB 参数
                1024 * 1024,      // 1 MiB 返回值
                300_000L,         // 5 分钟超时上限
                32,               // 参数个数
                4,                // 并发
                60);              // 每分钟
    }
}
