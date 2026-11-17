package io.miyo.agent.ipc;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * 帧工具（公共静态助手）：请求 / 响应形状与类型读取。
 * invoke 与 ipc 两包共用——避免暴露 IpcConnection 本体。
 */
public final class Frames {

    /** 成功响应帧。 */
    public static Map<String, Object> ok(String id, Map<String, Object> data) {
        Map<String, Object> resp = new LinkedHashMap<>();
        resp.put("id", id);
        resp.put("ok", Boolean.TRUE);
        resp.put("data", data);
        return resp;
    }

    /** 错误响应帧。 */
    public static Map<String, Object> error(String id, String code, String message) {
        Map<String, Object> err = new LinkedHashMap<>();
        err.put("code", code);
        err.put("message", message);
        Map<String, Object> resp = new LinkedHashMap<>();
        resp.put("id", id == null ? "" : id);
        resp.put("ok", Boolean.FALSE);
        resp.put("error", err);
        return resp;
    }

    /** 取请求 params（缺失返回空 Map）。 */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> paramsOf(Map<String, Object> request) {
        Object params = request.get("params");
        if (params instanceof Map) {
            return (Map<String, Object>) params;
        }
        return new LinkedHashMap<>();
    }

    /** null 安全文本化。 */
    public static String asText(Object value) {
        return value == null ? "" : String.valueOf(value);
    }

    /** Number 视图（非 Number 返回 null）。 */
    public static Number asNumber(Object value) {
        return value instanceof Number ? (Number) value : null;
    }

    private Frames() {
    }
}
