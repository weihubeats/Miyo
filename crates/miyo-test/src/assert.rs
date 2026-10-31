//! 断言执行（SPEC §17.4 / §17.5）。

use jsonpath_rust::JsonPath;
use miyo_http::client::HttpResponseData;
use serde_json::Value;

use crate::config::AssertionSpec;

/// 单条断言结果。
/// Deserialize 供运行历史落库回读（scenario_runs.results_json）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub description: String,
    pub passed: bool,
    /// 失败原因（供失败高亮展示）。
    pub reason: Option<String>,
    /// 期望值文本（断言显式给出 expected 时随行，供结构化对比展示）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// 实际值文本（取到实际值时随行）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

impl Outcome {
    fn pass(description: String) -> Self {
        Outcome {
            description,
            passed: true,
            reason: None,
            expected: None,
            actual: None,
        }
    }

    fn fail(description: String, reason: String) -> Self {
        Outcome {
            description,
            passed: false,
            reason: Some(reason),
            expected: None,
            actual: None,
        }
    }
}

/// 断言描述文本。
pub fn describe(a: &AssertionSpec) -> String {
    if let Some(name) = &a.name {
        if !name.is_empty() {
            return name.clone();
        }
    }
    let op = a.op.as_deref().unwrap_or("eq");
    let expected = a
        .expected
        .as_ref()
        .map(v_to_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| a.path.clone().unwrap_or_default());
    format!("{} {} {}", a.r#type, op, expected)
}

fn v_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// 值是否等于期望文本（数字/布尔/字符串宽松比较）。
fn value_eq(v: &Value, text: &str) -> bool {
    match v {
        Value::String(s) => s == text,
        Value::Number(n) => n.to_string() == text,
        Value::Bool(b) => b.to_string() == text,
        _ => false,
    }
}

/// 取断言的实际值；无匹配返回 None。
fn actual(a: &AssertionSpec, resp: &HttpResponseData, body_value: Option<&Value>) -> Option<Value> {
    match a.r#type.as_str() {
        "status" => Some(Value::Number(resp.status.into())),
        "header" => {
            let name = a.path.as_deref().unwrap_or_default();
            resp.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| Value::String(v.clone()))
        }
        "response_time_ms" => Some(
            serde_json::Number::from_f64(resp.duration_ms)
                .map(Value::Number)
                .unwrap_or(Value::Null),
        ),
        "body" => Some(Value::String(resp.body_text())),
        "jsonpath" => {
            let path = a.path.as_deref()?;
            crate::extract::extract_body_json(body_value, path).map(Value::String)
        }
        // GraphQL errors 断言：body.errors 数组（缺失按空数组，配合 empty/not_empty）。
        "graphql_errors" => Some(
            body_value
                .and_then(|v| v.get("errors"))
                .and_then(|e| e.as_array())
                .map(|arr| Value::Array(arr.clone()))
                .unwrap_or(Value::Array(Vec::new())),
        ),
        // 长度断言：path 为 JSONPath（缺省为整个 body 文本）→ 其长度；
        // 后续走数字比较（eq/neq/gt/gte/lt/lte）。
        "length" => {
            let len = match a.path.as_deref() {
                Some(path) => {
                    let json = body_value?;
                    let matched = json.query(path).ok().and_then(|v| v.into_iter().next())?;
                    json_len(matched)
                }
                None => body_value
                    .map(|v| match v {
                        Value::String(s) => s.chars().count() as u64,
                        other => other.to_string().len() as u64,
                    })
                    .unwrap_or(0),
            };
            Some(Value::Number(len.into()))
        }
        _ => body_value.cloned(),
    }
}

/// 值是否视为空：空串 / 空数组 / 空对象 / Null。
fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// JSON 值的长度：字符串按字符数，数组/对象按成员数，其余按序列化文本长度。
fn json_len(v: &Value) -> u64 {
    match v {
        Value::String(s) => s.chars().count() as u64,
        Value::Array(a) => a.len() as u64,
        Value::Object(o) => o.len() as u64,
        Value::Null => 0,
        other => other.to_string().len() as u64,
    }
}

fn cmp_ops(op: &str, got: f64, want: f64) -> bool {
    match op {
        "gt" => got > want,
        "gte" => got >= want,
        "lt" => got < want,
        _ => got <= want,
    }
}

/// 执行单条断言。
pub fn evaluate(a: &AssertionSpec, resp: &HttpResponseData, body_value: Option<&Value>) -> Outcome {
    let description = describe(a);
    let op = a.op.as_deref().unwrap_or("eq").to_string();
    let expected = a.expected.as_ref().map(v_to_text).unwrap_or_default();

    let Some(actual) = actual(a, resp, body_value) else {
        let mut o = match op.as_str() {
            "not_exists" => Outcome::pass(description),
            "exists" => Outcome::fail(description, "值不存在（未获取到实际值）".into()),
            _ => Outcome::fail(
                description,
                "未获取到断言的实际值（JSONPath 无匹配或响应头不存在）".into(),
            ),
        };
        if a.expected.is_some() {
            o.expected = Some(expected);
        }
        return o;
    };
    let actual_text = v_to_text(&actual);

    let result = match op.as_str() {
        "eq" => {
            if value_eq(&actual, &expected) {
                Ok(())
            } else {
                Err(format!(
                    "实际值 {} 不等于期望值 {expected}",
                    v_to_text(&actual)
                ))
            }
        }
        "neq" => {
            if value_eq(&actual, &expected) {
                Err(format!("实际值 {} 应不等于 {expected}", v_to_text(&actual)))
            } else {
                Ok(())
            }
        }
        "contains" => {
            let content = v_to_text(&actual);
            if content.contains(&expected) {
                Ok(())
            } else {
                Err(format!("「{content}」不包含「{expected}」"))
            }
        }
        "not_contains" => {
            let content = v_to_text(&actual);
            if content.contains(&expected) {
                Err(format!("「{content}」不应包含「{expected}」"))
            } else {
                Ok(())
            }
        }
        "gt" | "gte" | "lt" | "lte" => {
            let got = to_f64(&actual);
            let want = expected.parse::<f64>();
            match (got, want) {
                (Some(g), Ok(w)) if cmp_ops(&op, g, w) => Ok(()),
                (Some(g), Ok(w)) => Err(format!("实际值 {g} 不满足 {op} {w}")),
                (None, _) => Err(format!("实际值 {} 不是数字", v_to_text(&actual))),
                (_, Err(_)) => Err(format!("期望值「{expected}」不是数字")),
            }
        }
        "exists" => Ok(()),
        "not_exists" => Err(format!("值存在（{}），应不存在", v_to_text(&actual))),
        // 正则（部分匹配；非法模式直接报失败而非 panic）。
        "matches" => match regex::Regex::new(&expected) {
            Ok(re) => {
                let content = v_to_text(&actual);
                if re.is_match(&content) {
                    Ok(())
                } else {
                    Err(format!("「{content}」不匹配正则「{expected}」"))
                }
            }
            Err(e) => Err(format!("正则无效「{expected}」：{e}")),
        },
        "not_matches" => match regex::Regex::new(&expected) {
            Ok(re) => {
                let content = v_to_text(&actual);
                if re.is_match(&content) {
                    Err(format!("「{content}」不应匹配正则「{expected}」"))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(format!("正则无效「{expected}」：{e}")),
        },
        // 空 / 非空（字符串空、数组/对象无成员、Null 视为空）。
        "empty" => {
            if is_empty_value(&actual) {
                Ok(())
            } else {
                Err(format!("值非空（{}），应为空", v_to_text(&actual)))
            }
        }
        "not_empty" => {
            if is_empty_value(&actual) {
                Err("值为空，应非空".into())
            } else {
                Ok(())
            }
        }
        other => Err(format!("不支持的操作符：{other}")),
    };

    let mut outcome = match result {
        Ok(()) => Outcome::pass(description),
        Err(reason) => Outcome::fail(description, reason),
    };
    // 结构化对比值：期望仅在断言显式给出时随行，实际取到才随行——
    // 前端「期望 x · 实际 y」以此渲染，不再从 reason 文案反解。
    if a.expected.is_some() {
        outcome.expected = Some(expected);
    }
    outcome.actual = Some(actual_text);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde_json::json;

    fn make_resp(
        status: u16,
        headers: Vec<(&str, &str)>,
        body: &str,
        duration_ms: f64,
    ) -> HttpResponseData {
        HttpResponseData {
            status,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Bytes::from(body.to_string()),
            duration_ms,
            size_bytes: body.len(),
            cookies: Vec::new(),
            truncated: false,
            request: None,
        }
    }

    fn spec(t: &str, op: &str, expected: Value, path: Option<&str>) -> AssertionSpec {
        AssertionSpec {
            name: None,
            r#type: t.to_string(),
            path: path.map(|s| s.to_string()),
            op: Some(op.to_string()),
            expected: Some(expected),
        }
    }

    #[test]
    fn regex_matches_and_invalid_pattern() {
        let r = make_resp(200, vec![], r#"{"id":"u-123"}"#, 5.0);
        assert!(
            evaluate(
                &spec("body", "matches", json!(r#""id":"u-\d+""#), None),
                &r,
                None
            )
            .passed
        );
        assert!(!evaluate(&spec("body", "matches", json!(r#"^nope"#), None), &r, None).passed);
        assert!(
            evaluate(
                &spec("body", "not_matches", json!(r#"^nope"#), None),
                &r,
                None
            )
            .passed
        );
        let bad = evaluate(&spec("body", "matches", json!("(["), None), &r, None);
        assert!(!bad.passed);
        assert!(bad.reason.is_some_and(|m| m.contains("正则无效")));
    }

    #[test]
    fn length_assertions() {
        let body = serde_json::json!({"items": [1, 2, 3], "name": "abcd"});
        let r = make_resp(200, vec![], &body.to_string(), 5.0);
        let bv = serde_json::from_str::<Value>(&body.to_string()).ok();
        assert!(
            evaluate(
                &spec("length", "eq", json!(3), Some("$.items")),
                &r,
                bv.as_ref()
            )
            .passed
        );
        assert!(
            evaluate(
                &spec("length", "gte", json!(4), Some("$.name")),
                &r,
                bv.as_ref()
            )
            .passed
        );
        assert!(
            !evaluate(
                &spec("length", "eq", json!(5), Some("$.items")),
                &r,
                bv.as_ref()
            )
            .passed
        );
    }

    #[test]
    fn graphql_errors_assertions() {
        let ok_body = serde_json::json!({"data": {"a": 1}});
        let err_body = serde_json::json!({"errors": [{"message": "bad"}]});
        let r_ok = make_resp(200, vec![], &ok_body.to_string(), 5.0);
        let r_err = make_resp(200, vec![], &err_body.to_string(), 5.0);
        let bv_ok = serde_json::from_str::<Value>(&ok_body.to_string()).ok();
        let bv_err = serde_json::from_str::<Value>(&err_body.to_string()).ok();
        assert!(
            evaluate(
                &spec("graphql_errors", "empty", json!(null), None),
                &r_ok,
                bv_ok.as_ref()
            )
            .passed
        );
        assert!(
            !evaluate(
                &spec("graphql_errors", "empty", json!(null), None),
                &r_err,
                bv_err.as_ref()
            )
            .passed
        );
        assert!(
            evaluate(
                &spec("graphql_errors", "not_empty", json!(null), None),
                &r_err,
                bv_err.as_ref()
            )
            .passed
        );
        assert!(
            evaluate(
                &spec("graphql_errors", "contains", json!("bad"), None),
                &r_err,
                bv_err.as_ref()
            )
            .passed
        );
    }

    #[test]
    fn empty_ops_on_strings() {
        let r = make_resp(200, vec![], "", 5.0);
        assert!(evaluate(&spec("body", "empty", json!(null), None), &r, None).passed);
        assert!(!evaluate(&spec("body", "not_empty", json!(null), None), &r, None).passed);
    }

    #[test]
    fn status_eq_and_neq() {
        let r = make_resp(200, vec![], "x", 5.0);
        assert!(evaluate(&spec("status", "eq", json!(200), None), &r, None).passed);
        assert!(!evaluate(&spec("status", "eq", json!(404), None), &r, None).passed);
        assert!(evaluate(&spec("status", "neq", json!(500), None), &r, None).passed);
        // 字符串期望宽松比较
        assert!(evaluate(&spec("status", "eq", json!("200"), None), &r, None).passed);
    }

    #[test]
    fn header_contains_and_not_exists() {
        let r = make_resp(
            201,
            vec![("X-Token", "abc123"), ("Set-Cookie", "sid=1")],
            "x",
            5.0,
        );
        let a = spec("header", "contains", json!("bc"), Some("x-token")); // 大小写不敏感
        assert!(evaluate(&a, &r, None).passed);
        let missing = AssertionSpec {
            name: None,
            r#type: "header".into(),
            path: Some("X-Nope".into()),
            op: Some("exists".into()),
            expected: None,
        };
        assert!(!evaluate(&missing, &r, None).passed);
        let absent = AssertionSpec {
            name: None,
            r#type: "header".into(),
            path: Some("X-Nope".into()),
            op: Some("not_exists".into()),
            expected: None,
        };
        assert!(evaluate(&absent, &r, None).passed);
    }

    #[test]
    fn jsonpath_eq_and_contains() {
        let body = json!({"data": {"id": 7, "name": "miyo", "tags": ["a", "b"]}});
        let r = make_resp(200, vec![], &body.to_string(), 3.0);
        let a = spec("jsonpath", "eq", json!(7), Some("$.data.id"));
        assert!(evaluate(&a, &r, Some(&body)).passed);
        let a2 = spec("jsonpath", "contains", json!("miyo"), Some("$.data.name"));
        assert!(evaluate(&a2, &r, Some(&body)).passed);
        // jsonpath 无匹配 → 失败
        let a3 = spec("jsonpath", "eq", json!(7), Some("$.data.nope"));
        let out = evaluate(&a3, &r, None);
        assert!(!out.passed);
        assert!(out.reason.is_some());
    }

    #[test]
    fn numeric_ops() {
        let r = make_resp(200, vec![], "x", 1500.0);
        let a = spec("response_time_ms", "lt", json!(2000), None);
        assert!(evaluate(&a, &r, None).passed);
        let a2 = spec("response_time_ms", "gt", json!(2000), None);
        assert!(!evaluate(&a2, &r, None).passed);
        let a3 = spec("status", "gte", json!(200), None);
        assert!(evaluate(&a3, &r, None).passed);
    }

    #[test]
    fn body_text_contains() {
        let r = make_resp(200, vec![], "hello world from miyo", 1.0);
        let a = spec("body", "contains", json!("world"), None);
        assert!(evaluate(&a, &r, None).passed);
        let a2 = spec("body", "contains", json!("banana"), None);
        assert!(!evaluate(&a2, &r, None).passed);
    }

    #[test]
    fn describe_name_preferred() {
        let a = AssertionSpec {
            name: Some("接口正常".into()),
            r#type: "status".into(),
            path: None,
            op: Some("eq".into()),
            expected: Some(json!(200)),
        };
        assert_eq!(describe(&a), "接口正常");
    }

    #[test]
    fn outcome_carries_structured_expected_actual() {
        // 失败：期望 / 实际随行（前端结构化对比展示）。
        let r = make_resp(500, vec![], "{}", 1.0);
        let o = evaluate(&spec("status", "eq", json!(200), None), &r, None);
        assert!(!o.passed);
        assert_eq!(o.expected.as_deref(), Some("200"));
        assert_eq!(o.actual.as_deref(), Some("500"));

        // 成功同样随行；无 expected 的操作符不携带期望。
        let ok = evaluate(&spec("status", "eq", json!(500), None), &r, None);
        assert!(ok.passed);
        assert_eq!(ok.actual.as_deref(), Some("500"));
        let exists_spec = AssertionSpec {
            name: None,
            r#type: "status".into(),
            path: None,
            op: Some("exists".into()),
            expected: None,
        };
        let exists = evaluate(&exists_spec, &r, None);
        assert!(exists.expected.is_none());
        assert!(exists.actual.is_some());
    }
}
