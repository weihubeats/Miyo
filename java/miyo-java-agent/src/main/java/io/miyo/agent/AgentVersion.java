package io.miyo.agent;

/**
 * Agent 常量：版本 / 协议版本 / 限制。
 *
 * 协议版本（PROTOCOL_VERSION）与 Miyo 侧 `miyo-java-attach` 的期望值对齐；
 * 不兼容变更必须 +1（握手不匹配即拒绝工作，防止语义漂移）。
 */
public final class AgentVersion {

    /** Agent 版本。 */
    public static final String VERSION = "2.0.0";

    /** 通信协议版本。 */
    public static final int PROTOCOL_VERSION = 2;

    /** 单帧（请求 / 响应）最大字节数（16 MiB；调用策略在更小的口径上截断）。 */
    public static final int MAX_FRAME_BYTES = 16 * 1024 * 1024;

    /** 握手超时（毫秒）：连接建立后第一帧必须在该时限内携带正确 token。 */
    public static final int HANDSHAKE_TIMEOUT_MS = 10_000;

    /** 默认最大并发调用数（Miyo 侧策略可降不可升）。 */
    public static final int DEFAULT_MAX_CONCURRENT = 4;

    /** 默认单次调用超时上限（毫秒）。 */
    public static final long DEFAULT_MAX_TIMEOUT_MS = 300_000L;

    /** 默认 DTO / 结果序列化深度上限。 */
    public static final int DEFAULT_MAX_DEPTH = 8;

    private AgentVersion() {
    }
}
