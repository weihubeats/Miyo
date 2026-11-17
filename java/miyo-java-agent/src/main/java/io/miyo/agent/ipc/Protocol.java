package io.miyo.agent.ipc;

import io.miyo.agent.AgentVersion;

/**
 * 通信协议常量与错误码（协议 v2）。
 *
 * 帧格式：4 字节大端长度 + UTF-8 JSON。请求 {@code {"id":..,"op":..,"params":{..}}}；
 * 响应 {@code {"id":..,"ok":true,"data":..}} 或
 * {@code {"id":..,"ok":false,"error":{"code":..,"message":..}}}。
 *
 * 传输：仅绑定 127.0.0.1 随机端口；鉴权 = 握手首帧 token 常数时间比较；
 * 未握手超时 / 鉴权失败即断开。
 */
public final class Protocol {

    public static final int PROTOCOL_VERSION = AgentVersion.PROTOCOL_VERSION;

    // ---- 错误码（snake_case；Miyo 侧按码分型） ----

    /** 握手 token 缺失 / 不匹配。 */
    public static final String ERR_NOT_AUTHORIZED = "not_authorized";
    /** 协议版本不匹配。 */
    public static final String ERR_PROTOCOL_MISMATCH = "protocol_mismatch";
    /** 请求形状非法（op 未知 / params 缺失）。 */
    public static final String ERR_BAD_REQUEST = "bad_request";
    /** 参数非法（个数 / 类型 / 形状）。 */
    public static final String ERR_BAD_ARGUMENTS = "bad_arguments";
    /** 目标类 / 方法受限（黑名单 / 可赋值性闸）。 */
    public static final String ERR_NOT_ALLOWED = "not_allowed";
    /** 请求的类 / 方法 / Bean 不存在。 */
    public static final String ERR_NOT_FOUND = "not_found";
    /** 并发 / 体积超限。 */
    public static final String ERR_RATE_LIMITED = "rate_limited";
    /** 取消目标不存在（已结束）。 */
    public static final String ERR_UNKNOWN_INVOCATION = "unknown_invocation";
    /** 需要用户确认（新建实例等有副作用操作）。 */
    public static final String ERR_CONSENT_REQUIRED = "consent_required";
    /** 功能在当前目标 JVM 不可用（附原因）。 */
    public static final String ERR_UNSUPPORTED = "unsupported";
    /** Agent 内部错误。 */
    public static final String ERR_INTERNAL = "internal";

    private Protocol() {
    }
}
