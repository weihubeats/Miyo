//! 变量提取（SPEC §17.3 extract）：body JSONPath 或 header。

use std::collections::HashMap;

use jsonpath_rust::JsonPath;
use miyo_http::client::HttpResponseData;
use serde_json::Value;

use crate::config::ExtractSpec;

/// JSONPath 提取：取第一个匹配项，转字符串；无匹配返回 None。
///
/// 1.0 起为 `json.query(path)` 引用式查询（返回 `Vec<&Value>` 指向原 JSON），
/// 每个 extract spec 不再 `Box::new(json.clone())` 全拷贝一次 body。
pub fn extract_body_json(body_value: Option<&Value>, path: &str) -> Option<String> {
    let json = body_value?;
    let matched = json.query(path).ok()?.into_iter().next()?;
    match matched {
        Value::String(s) => Some(s.clone()),
        // Null 视为未提取到（缺字段）。
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// JSONPath 提取（类型保留）：命中值保持原始 JSON 类型（数字 / 布尔 /
/// 数组 / 对象原样返回），字符串返回 `Value::String`；无匹配或 Null 返回 None。
///
/// 供场景链路做「JSON Body 类型保留渲染」：`{{amount}}` 整值占位在 Body
/// 里替换为数字 `388.2` 而非字符串 `"388.2"`。
pub fn extract_body_json_typed(body_value: Option<&Value>, path: &str) -> Option<Value> {
    let json = body_value?;
    let matched = json.query(path).ok()?.into_iter().next()?;
    match matched {
        Value::Null => None,
        other => Some(other.clone()),
    }
}

/// 按 extract 配置提取变量。
pub fn extract_variables(
    specs: &[ExtractSpec],
    resp: &HttpResponseData,
    body_value: Option<&Value>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for spec in specs {
        let value = match spec.from.as_str() {
            "header" => resp
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&spec.path))
                .map(|(_, v)| v.clone()),
            _ => extract_body_json(body_value, &spec.path),
        };
        if let Some(value) = value {
            out.insert(spec.name.clone(), value);
        }
    }
    out
}

/// 提取变量（带类型层）：字符串表与 [`extract_variables`] 同形（现有渲染 /
/// 快照 / 历史链路不变），`typed` 只收 body 提取里非字符串类型的 JSON 值
///（数字 / 布尔 / 数组 / 对象），供 JSON Body 类型保留渲染。
pub fn extract_variables_typed(
    specs: &[ExtractSpec],
    resp: &HttpResponseData,
    body_value: Option<&Value>,
) -> (HashMap<String, String>, HashMap<String, Value>) {
    let mut out = HashMap::new();
    let mut typed = HashMap::new();
    for spec in specs {
        let value = match spec.from.as_str() {
            "header" => resp
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&spec.path))
                .map(|(_, v)| v.clone()),
            _ => extract_body_json(body_value, &spec.path),
        };
        if let Some(value) = value {
            out.insert(spec.name.clone(), value);
        }
        if matches!(spec.from.as_str(), "header" | "body") {
            if let Some(t) = extract_body_json_typed(body_value, &spec.path) {
                // 字符串在字符串表里已有，typed 只放「渲染时需要还原类型」的值。
                if !matches!(t, Value::String(_)) {
                    typed.insert(spec.name.clone(), t);
                }
            }
        }
    }
    (out, typed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde_json::json;

    fn resp_with(body: &str, headers: Vec<(&str, &str)>) -> HttpResponseData {
        HttpResponseData {
            status: 200,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Bytes::from(body.to_string()),
            duration_ms: 1.0,
            size_bytes: body.len(),
            cookies: Vec::new(),
            truncated: false,
            request: None,
        }
    }

    #[test]
    fn jsonpath_scalar_and_nested() {
        let v = json!({"id": "u1", "nested": {"x": 42}, "list": [{"k": 1}]});
        assert_eq!(extract_body_json(Some(&v), "$.id"), Some("u1".into()));
        assert_eq!(extract_body_json(Some(&v), "$.nested.x"), Some("42".into()));
        assert_eq!(extract_body_json(Some(&v), "$.list[0].k"), Some("1".into()));
        assert_eq!(extract_body_json(Some(&v), "$.missing"), None);
    }

    #[test]
    fn numbers_become_strings() {
        let v = json!({"n": 7.5, "flag": true});
        assert_eq!(extract_body_json(Some(&v), "$.n"), Some("7.5".into()));
        assert_eq!(extract_body_json(Some(&v), "$.flag"), Some("true".into()));
    }

    #[test]
    fn invalid_path_returns_none() {
        let v = json!({"a": 1});
        assert_eq!(extract_body_json(Some(&v), "$$bad("), None);
        assert_eq!(extract_body_json(None, "$.a"), None);
    }

    #[test]
    fn header_and_body_extraction() {
        let r = resp_with(r#"{"token":"t-9"}"#, vec![("X-Request-Id", "req-1")]);
        let specs = vec![
            ExtractSpec {
                name: "tid".into(),
                from: "body".into(),
                path: "$.token".into(),
            },
            ExtractSpec {
                name: "rid".into(),
                from: "header".into(),
                path: "x-request-id".into(),
            },
            ExtractSpec {
                name: "none".into(),
                from: "body".into(),
                path: "$.missing".into(),
            },
        ];
        let body_value = serde_json::from_str::<Value>(r#"{"token":"t-9"}"#).ok();
        let vars = extract_variables(&specs, &r, body_value.as_ref());
        assert_eq!(vars.get("tid"), Some(&"t-9".to_string()));
        assert_eq!(vars.get("rid"), Some(&"req-1".to_string()));
        assert!(!vars.contains_key("none"));
    }

    #[test]
    fn typed_extraction_preserves_json_types() {
        let r = resp_with(
            r#"{"amount": 388.2, "flag": true, "tags": ["a"], "name": "x", "miss": null}"#,
            vec![],
        );
        let specs: Vec<ExtractSpec> = ["amount", "flag", "tags", "name", "miss", "nope"]
            .iter()
            .map(|n| ExtractSpec {
                name: n.to_string(),
                from: "body".into(),
                path: format!("$.{n}"),
            })
            .collect();
        let body_value = serde_json::from_str::<Value>(
            r#"{"amount": 388.2, "flag": true, "tags": ["a"], "name": "x", "miss": null}"#,
        )
        .ok();
        let (vars, typed) = extract_variables_typed(&specs, &r, body_value.as_ref());
        // 字符串表与旧口径一致（全部转字符串）。
        assert_eq!(vars.get("amount"), Some(&"388.2".to_string()));
        assert_eq!(vars.get("name"), Some(&"x".to_string()));
        // typed 只收非字符串 JSON 值；字符串 / Null / 缺失不入 typed。
        assert_eq!(typed.get("amount"), Some(&json!(388.2)));
        assert_eq!(typed.get("flag"), Some(&json!(true)));
        assert_eq!(typed.get("tags"), Some(&json!(["a"])));
        assert!(!typed.contains_key("name"));
        assert!(!typed.contains_key("miss"));
        assert!(!typed.contains_key("nope"));
    }
}
