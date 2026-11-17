package io.miyo.agent;

import java.io.UnsupportedEncodingException;
import java.net.URLDecoder;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Agent 启动参数解析（loadAgent 的 options 单字符串）。
 *
 * 形如 {@code k1=v1,k2=v2,...}；value 做 URL 编码（%XX）以容忍路径中的
 * 逗号 / 等号。缺 token 视为参数非法——Agent 拒绝启动（fail-fast）。
 *
 * 必需：token（64 位 hex，Miyo 生成的一次性凭证）。
 * 可选：infoFile（连接信息落盘路径）、expectedPid（进程身份绑定）、
 *       maxConcurrent / maxTimeoutMs / maxDepth（策略上限）、
 *       allowStaticScan（静态字段扫描开关，默认 false）。
 */
public final class AgentArgs {

    private final Map<String, String> values;

    private AgentArgs(Map<String, String> values) {
        this.values = values;
    }

    public static AgentArgs parse(String raw) {
        Map<String, String> out = new LinkedHashMap<>();
        if (raw == null || raw.trim().isEmpty()) {
            return new AgentArgs(out);
        }
        for (String pair : raw.split(",")) {
            if (pair.trim().isEmpty()) {
                continue;
            }
            int eq = pair.indexOf('=');
            if (eq <= 0) {
                throw new IllegalArgumentException("非法 agent 参数段（缺少 k=v）：" + pair);
            }
            String key = pair.substring(0, eq).trim();
            String value = decode(pair.substring(eq + 1));
            out.put(key, value);
        }
        return new AgentArgs(out);
    }

    private static String decode(String value) {
        if (value.indexOf('%') < 0) {
            return value;
        }
        try {
            return URLDecoder.decode(value, "UTF-8");
        } catch (UnsupportedEncodingException e) {
            return value; // UTF-8 恒在，防御分支
        }
    }

    public String get(String key) {
        return values.get(key);
    }

    public String require(String key) throws IllegalArgumentException {
        String v = values.get(key);
        if (v == null || v.trim().isEmpty()) {
            throw new IllegalArgumentException("缺少必需参数：" + key);
        }
        return v.trim();
    }

    public long getLong(String key, long fallback) {
        String v = values.get(key);
        if (v == null || v.trim().isEmpty()) {
            return fallback;
        }
        try {
            return Long.parseLong(v.trim());
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    public int getInt(String key, int fallback) {
        return (int) getLong(key, fallback);
    }

    public boolean getBool(String key, boolean fallback) {
        String v = values.get(key);
        if (v == null || v.trim().isEmpty()) {
            return fallback;
        }
        return "true".equalsIgnoreCase(v.trim()) || "1".equals(v.trim());
    }
}
