//! 敏感数据识别与脱敏单源（P0 安全修复）。
//!
//! 全仓所有「向外部 / AI / 导出文件返回数据」的路径共用本模块：
//! - 键名敏感判定：[`is_sensitive_key`]（内建词根 + 用户自定义规则）；
//! - 值形态判定：[`looks_like_secret`]（JWT / 签名类形态）；
//! - JSON 嵌套脱敏：[`redact_json_value`] / [`redact_json_text`]；
//! - URL query 脱敏：[`redact_url_query`]（错误日志 / 历史落库 / MCP 返回共用）；
//! - 自定义规则：[`set_custom_sensitive_patterns`]（settings 表持久化，
//!   启动时加载，进程内全局生效——miyo-sync / miyo-http / miyo-agent /
//!   miyo-tauri 全部经本模块读规则）。
//!
//! 脱敏策略（与 miyo-sync::redact 一致）：键名命中或值形态像凭证 → 替换为
//! [`PLACEHOLDER`]；无法可靠判定（自由文本 body）时**不**整段替换以保留
//! 可读性，由调用方的「人工检查提示」兜底——启发式脱敏是尽力而为，
//! 不是安全保证。

use std::sync::RwLock;

use serde_json::Value;

/// 敏感值占位符（历史值与 miyo-sync 的 `__MIYO_REDACTED__` 一致）。
pub const PLACEHOLDER: &str = "__MIYO_REDACTED__";

/// 敏感键名内建词根：命中即视为需要脱敏。`*_url` / `endpoint` 之类的地址键
/// **不**命中（团队内通常需要共享，且不是凭证）。
const TOKENS: &[&str] = &[
    "authorization",
    "cookie",
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "api-key",
    "apikey",
    "api_key",
    "access_key",
    "accesskey",
    "private",
    "credential",
    "sessionid",
    "session_id",
    "x-auth",
    "signature",
    // 不收录裸 "sign"：子串会误伤 design / assigned 等业务键。
    "x-sign",
];

/// 用户自定义敏感键规则（小写词根；进程内全局，settings 表持久化）。
/// `RwLock` 读取在热路径（每次键名判定）上开销可忽略，写入仅在设置变更时发生。
static CUSTOM_PATTERNS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// 覆盖用户自定义敏感键规则（空串 / 纯空白条目忽略；统一转小写）。
/// miyo-tauri 启动时从 settings 表加载，`set_sensitive_patterns` 命令变更时刷新。
pub fn set_custom_sensitive_patterns(patterns: Vec<String>) {
    let normalized: Vec<String> = patterns
        .into_iter()
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    *CUSTOM_PATTERNS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = normalized;
}

/// 当前自定义规则副本（设置回显用）。
pub fn custom_sensitive_patterns() -> Vec<String> {
    CUSTOM_PATTERNS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 敏感键名判定：内建词根或自定义规则命中（大小写不敏感的子串匹配）。
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    if TOKENS.iter().any(|t| k.contains(t)) {
        return true;
    }
    CUSTOM_PATTERNS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|p| k.contains(p.as_str()))
}

/// 值形态启发式：像 JWT / 长随机签名串时即便键名不敏感也脱敏。
pub fn looks_like_secret(value: &str) -> bool {
    // JWT：三段 base64url 以 . 分隔。
    value.matches('.').count() == 2
        && value.len() > 40
        && value.split('.').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// JSON 值递归脱敏：敏感键（或形态像凭证）的字符串值替换为占位符。
/// 数组逐项递归——嵌套路径（如 `data.items[0].access_token`）同样命中。
pub fn redact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if let Some(s) = v.as_str() {
                    if !s.is_empty()
                        && s != PLACEHOLDER
                        && (is_sensitive_key(key) || looks_like_secret(s))
                    {
                        *v = serde_json::Value::String(PLACEHOLDER.to_string());
                        continue;
                    }
                }
                redact_json_value(v);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

/// JSON 文本脱敏（保守策略）：可解析时对敏感字段替换占位符并重新序列化；
/// 解析失败或未命中时原文返回（不毁掉正常文档），`(文本, 是否有改动)`。
pub fn redact_json_text(raw: &str) -> (String, bool) {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (raw.to_string(), false);
    };
    redact_json_value(&mut value);
    match serde_json::to_string(&value) {
        Ok(new_raw) if new_raw != raw.trim() => (new_raw, true),
        _ => (raw.to_string(), false),
    }
}

/// URL query 脱敏：命中敏感键的参数值替换为占位符，其余原样保留。
/// 凭据在 query（`?access_token=…`）时不落明文——日志 / 历史 / MCP 返回共用。
pub fn redact_url_query(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let redacted: Vec<String> = query
        .split('&')
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if !v.is_empty() && is_sensitive_key(k) {
                format!("{k}={PLACEHOLDER}")
            } else {
                pair.to_string()
            }
        })
        .collect();
    format!("{base}?{}", redacted.join("&"))
}

/// 自由文本中的 URL 脱敏：扫描 `http(s)://` 起的 URL 片段，对 query 部分
/// 做敏感参数替换。用于错误消息 / 日志——reqwest 错误 Display 含完整 URL
/// （可能带 `?token=…`），不得原样进入日志或 AI 上下文。
pub fn redact_message_urls(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(pos) = rest.find("http://").or_else(|| rest.find("https://")) {
        let head_len = if rest[pos..].starts_with("https://") {
            "https://".len()
        } else {
            "http://".len()
        };
        out.push_str(&rest[..pos]);
        // URL 终止符：空白 / 引号 / 括号 / 反引号（reqwest 错误消息用 `(url)` 包裹）。
        let after = &rest[pos + head_len..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '`'))
            .map(|i| pos + head_len + i)
            .unwrap_or(rest.len());
        // ')' 可能属于包裹错误消息的括号而非 URL；URL 内部不含裸 `)`（保留在内）。
        out.push_str(&redact_url_query(&rest[pos..end]));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// HTTP 头名是否敏感（重定向跨源时按此剥离）：Authorization / Cookie 等
/// 内建敏感词全部命中——覆盖 API Key 自定义头（x-api-key）、云签名头
/// （x-amz-security-token / signature）等 reqwest 不剥离的自定义凭证头。
pub fn is_sensitive_header_name(name: &str) -> bool {
    is_sensitive_key(name)
}

/// 序列化后的 RequestSpec / 材料化步骤形状 JSON 递归脱敏：
/// - `headers` / `params` / `path_variables` / `fields`（{key, value} 对数组）；
/// - `body.raw`（JSON 文本 → redact_json_text）；
/// - `auth`（按变体字段：token / password / value / app_secret / client_secret /
///   secret_key / session_token / Hawk key；ApiKey 的 `key` 是头名，不掩）；
/// - `url_template` / `url` 的 query。
///
/// 供场景 meta.steps 快照等「已渲染材料化请求」的落库前脱敏共用。
pub fn redact_request_shape_json(v: &mut serde_json::Value) {
    const PAIR_KEYS: &[&str] = &["headers", "params", "path_variables", "fields"];
    /// auth 变体中的凭据字段（ApiKey 的 `key` 是头名/参数名，不是凭据）。
    const AUTH_SECRET_FIELDS: &[&str] = &[
        "token",
        "password",
        "value",
        "app_secret",
        "client_secret",
        "secret_key",
        "session_token",
    ];
    match v {
        serde_json::Value::Object(map) => {
            for pair_key in PAIR_KEYS {
                if let Some(serde_json::Value::Array(items)) = map.get_mut(*pair_key) {
                    for item in items.iter_mut() {
                        let Some(k) = item.get("key").and_then(|x| x.as_str()).map(str::to_string)
                        else {
                            continue;
                        };
                        let Some(Value::String(val)) = item.get_mut("value") else {
                            continue;
                        };
                        if !val.is_empty() && is_sensitive_key(&k) {
                            *val = PLACEHOLDER.to_string();
                        }
                    }
                }
            }
            if let Some(serde_json::Value::String(raw)) = map.get_mut("raw") {
                let (masked, changed) = redact_json_text(raw);
                if changed {
                    *raw = masked;
                }
            }
            if let Some(serde_json::Value::Object(auth_map)) = map.get_mut("auth") {
                let is_apikey = auth_map
                    .get("type")
                    .and_then(|t| t.as_str())
                    .is_some_and(|t| t == "apikey");
                for (field, val) in auth_map.iter_mut() {
                    let Value::String(s) = val else { continue };
                    if s.is_empty() || s.as_str() == PLACEHOLDER {
                        continue;
                    }
                    let sensitive_field = if is_apikey {
                        field == "value"
                    } else {
                        AUTH_SECRET_FIELDS.contains(&field.as_str()) || field == "key"
                    };
                    if sensitive_field {
                        *val = Value::String(PLACEHOLDER.to_string());
                    }
                }
            }
            for url_key in ["url_template", "url"] {
                if let Some(serde_json::Value::String(u)) = map.get_mut(url_key) {
                    *u = redact_url_query(u);
                }
            }
            for (_, child) in map.iter_mut() {
                redact_request_shape_json(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_request_shape_json(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builtin_tokens_match_case_insensitive() {
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("x-api-key"));
        assert!(is_sensitive_key("ApiKey"));
        assert!(is_sensitive_key("SESSION_ID"));
        assert!(is_sensitive_key("X-Signature"));
        assert!(!is_sensitive_key("X-Trace-Id"));
        assert!(!is_sensitive_key("baseUrl"));
    }

    #[test]
    fn custom_patterns_extend_builtin() {
        set_custom_sensitive_patterns(vec!["CorpInternal".into(), "  ".into()]);
        assert!(is_sensitive_key("X-CorpInternal-Id"));
        assert!(is_sensitive_key("corpinternal"));
        assert!(is_sensitive_key("authorization"), "内建词根始终生效");
        assert_eq!(custom_sensitive_patterns(), vec!["corpinternal"]);
        set_custom_sensitive_patterns(Vec::new());
        assert!(
            !is_sensitive_key("X-CorpInternal-Id"),
            "清空后自定义规则失效"
        );
        assert!(is_sensitive_key("authorization"), "内建词根不受影响");
    }

    #[test]
    fn redact_json_value_masks_nested_and_jwt() {
        let mut v = json!({
            "data": {
                "items": [ { "access_token": "tok-1", "name": "keep" } ],
                "jwt": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
                "note": "short.a.b"
            },
            "password": "pw",
            "already": PLACEHOLDER,
            "count": 3,
        });
        redact_json_value(&mut v);
        assert_eq!(v["data"]["items"][0]["access_token"], PLACEHOLDER);
        assert_eq!(v["data"]["items"][0]["name"], "keep");
        assert_eq!(
            v["data"]["jwt"], PLACEHOLDER,
            "JWT 形态值即便键名不敏感也应脱敏"
        );
        assert_eq!(v["data"]["note"], "short.a.b", "非凭证形态保留");
        assert_eq!(v["password"], PLACEHOLDER);
        assert_eq!(v["already"], PLACEHOLDER, "占位符不重复包裹");
        assert_eq!(v["count"], 3);
    }

    #[test]
    fn redact_json_text_conservative() {
        let (out, changed) = redact_json_text(r#"{"token":"t","keep":1}"#);
        assert!(changed);
        assert!(out.contains(PLACEHOLDER));
        assert!(out.contains("\"keep\":1"));
        // 非 JSON 原文返回
        let raw = "plain text body";
        assert_eq!(redact_json_text(raw), (raw.to_string(), false));
    }

    #[test]
    fn redact_url_query_masks_sensitive_params() {
        assert_eq!(
            redact_url_query("https://a.com/p?access_token=sec&page=2"),
            format!("https://a.com/p?access_token={PLACEHOLDER}&page=2")
        );
        assert_eq!(redact_url_query("https://a.com/p"), "https://a.com/p");
    }

    #[test]
    fn redact_message_urls_scrubs_query_in_errors() {
        let msg = "error sending request for url (https://api.example.com/login?access_token=sec&x=1) after 3s".to_string();
        let out = redact_message_urls(&msg);
        assert!(out.contains("access_token="));
        assert!(out.contains(PLACEHOLDER));
        assert!(!out.contains("sec&"));
        // 无 URL 的消息原样保留
        assert_eq!(
            redact_message_urls("timeout of 30s exceeded"),
            "timeout of 30s exceeded"
        );
    }

    #[test]
    fn redact_request_shape_json_masks_materialized_steps() {
        let mut v = json!([{
            "endpoint": { "name": "login", "request": { "headers": [
                {"key": "Authorization", "value": "Bearer real-tok", "enabled": true}
            ]}},
            "url_template": "{{base_url}}/login?access_token=sec",
            "spec_template": {
                "auth": {"type": "bearer", "token": "real-tok"},
                "headers": [{"key": "X-Api-Key", "value": "ak-9"}],
                "body": {"type": "json", "raw": "{\"password\":\"pw-1\",\"keep\":1}"},
                "params": [{"key": "page", "value": "2"}]
            }
        }]);
        redact_request_shape_json(&mut v);
        let step = &v[0];
        let hdr = &step["endpoint"]["request"]["headers"][0];
        assert_eq!(hdr["value"], PLACEHOLDER);
        assert!(step["url_template"].as_str().unwrap().contains(PLACEHOLDER));
        let spec = &step["spec_template"];
        assert_eq!(spec["auth"]["token"], PLACEHOLDER);
        assert_eq!(spec["headers"][0]["value"], PLACEHOLDER);
        let raw = spec["body"]["raw"].as_str().unwrap();
        assert!(!raw.contains("pw-1") && raw.contains("keep"));
        assert_eq!(spec["params"][0]["value"], "2", "非敏感参数保留");
    }

    #[test]
    fn redact_request_shape_json_apikey_key_field_is_header_name() {
        let mut v = json!({
            "auth": {"type": "apikey", "key": "X-API-Key", "value": "secret-v", "location": "header"}
        });
        redact_request_shape_json(&mut v);
        assert_eq!(v["auth"]["key"], "X-API-Key", "头名保留");
        assert_eq!(v["auth"]["value"], PLACEHOLDER);
        // Hawk：key 是密钥本身。
        let mut hawk = json!({"auth": {"type": "hawk", "key_id": "kid", "key": "hawk-secret"}});
        redact_request_shape_json(&mut hawk);
        assert_eq!(hawk["auth"]["key"], PLACEHOLDER);
    }

    #[test]
    fn sensitive_header_names_cover_custom_auth_headers() {
        assert!(is_sensitive_header_name("x-amz-security-token"));
        assert!(is_sensitive_header_name("X-Signature"));
        assert!(is_sensitive_header_name("Proxy-Authorization"));
        assert!(!is_sensitive_header_name("X-Trace-Id"));
        assert!(!is_sensitive_header_name("Content-Type"));
    }
}
