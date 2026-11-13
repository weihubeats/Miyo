//! 测试用例草稿生成（规则模板，不调 LLM、不落库）。
//!
//! 依据接口的参数元数据（`KeyValue.required` / `field_type` / `example`）、
//! JSON Body 结构（`body_docs` 标注优先，缺省按样本保守推断）与响应示例，
//! 生成需求三的九类用例草稿：正常 / 缺少必填参数 / 错误类型 / 边界值 /
//! 非法参数 / 权限不足 / 重复请求 / 资源不存在 / 幂等性。
//!
//! 草稿只是建议：保存由调用方（AI 审阅后 create_test_case，或前端预览勾选）
//! 完成，生成永远无副作用。

use std::collections::BTreeSet;

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{BodySpec, Endpoint, FieldType, TestCase};
use miyo_core::Result;
use miyo_storage::repository as repo;

/// 用例草稿（形状对齐 TestCase 的可保存字段，另带说明性元数据）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseDraft {
    /// 生成类别键：normal / missing_required / wrong_type / boundary /
    /// illegal / no_auth / duplicate / not_found / idempotency。
    pub kind: String,
    /// 存库分类（正向 / 负向 / 边界值 / 安全性 / 其他）。
    pub category: String,
    pub name: String,
    pub description: String,
    pub method: String,
    pub url_path: String,
    pub path_variables: Vec<KeyValueDto>,
    pub params: Vec<KeyValueDto>,
    pub headers: Vec<KeyValueDto>,
    /// body 类型（none / json / urlencoded / …）。
    pub body_type: String,
    pub body_content: String,
    /// 断言（AssertionSpec 形状 JSON 数组）。
    pub assertions: serde_json::Value,
    /// 执行说明（「连续发送两次」「需先获得有效 token」等）。
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueDto {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

impl KeyValueDto {
    fn from_kv(kv: &miyo_core::model::KeyValue) -> Self {
        KeyValueDto {
            key: kv.key.clone(),
            value: kv.value.clone(),
            enabled: kv.enabled,
        }
    }
}

/// 为接口生成测试用例草稿。
pub async fn generate_test_case_drafts(
    db: &SqlitePool,
    endpoint_id: Uuid,
) -> Result<Vec<TestCaseDraft>> {
    let endpoint = repo::get_endpoint(db, endpoint_id).await?;
    let cases = repo::list_test_cases(db, endpoint_id).await?;
    Ok(generate_drafts(&endpoint, &cases))
}

/// 纯函数生成（公开给测试与场景生成复用）。
pub fn generate_drafts(endpoint: &Endpoint, existing: &[TestCase]) -> Vec<TestCaseDraft> {
    let mut drafts = Vec::new();
    let spec = &endpoint.request;
    let write = matches!(
        endpoint.method,
        miyo_core::model::HttpMethod::POST
            | miyo_core::model::HttpMethod::PUT
            | miyo_core::model::HttpMethod::PATCH
            | miyo_core::model::HttpMethod::DELETE
    );
    let required: Vec<RequiredField> = required_fields(endpoint);
    let existing_names: BTreeSet<String> = existing.iter().map(|c| c.name.clone()).collect();
    let unique = |name: String| -> String {
        if existing_names.contains(&name) {
            format!("{name}（AI 生成）")
        } else {
            name
        }
    };

    // ---- 1 正常请求 ----
    let mut normal = base_draft(endpoint, "normal", "正向", "正常请求");
    normal.name = unique(normal.name);
    normal.description = "按接口当前配置原样发送，验证主流程可用".into();
    normal.assertions = serde_json::json!([
        { "type": "status", "op": "gte", "expected": 200 },
        { "type": "status", "op": "lt", "expected": 300 }
    ]);
    drafts.push(normal);

    // ---- 2 缺少必填参数（每个必填字段一条，最多 6 条防爆炸）----
    for field in required.iter().take(6) {
        let mut draft = base_draft(
            endpoint,
            "missing_required",
            "负向",
            &format!("缺少必填参数 {}", field.key),
        );
        draft.name = unique(draft.name);
        draft.description = format!(
            "移除必填{}「{}」后发送，预期服务端返回 4xx 参数校验错误",
            field.location_cn(),
            field.key
        );
        apply_removal(&mut draft, field, endpoint);
        draft.assertions = status_range_assertions(400, 500);
        drafts.push(draft);
    }

    // ---- 3 错误类型（首个「可构造类型不匹配值」的必填字段）----
    if let Some((field, value)) = required
        .iter()
        .find_map(|f| wrong_typed_value(f).map(|v| (f, v)))
    {
        let mut draft = base_draft(
            endpoint,
            "wrong_type",
            "负向",
            &format!("错误类型参数 {}", field.key),
        );
        draft.name = unique(draft.name);
        draft.description = format!(
            "{}「{}」填入与声明类型不匹配的值（{}），预期 4xx",
            field.location_cn(),
            field.key,
            field.type_cn()
        );
        apply_value(&mut draft, field, &value, endpoint);
        draft.assertions = status_range_assertions(400, 500);
        drafts.push(draft);
    }

    // ---- 4 边界值（空串 / 超长 / 数字 0 与 -1，最多 4 条）----
    let mut boundary_count = 0usize;
    for field in &required {
        if boundary_count >= 4 {
            break;
        }
        for (label, value) in boundary_values(field) {
            if boundary_count >= 4 {
                break;
            }
            let mut draft = base_draft(
                endpoint,
                "boundary",
                "边界值",
                &format!("边界值 {} {}", field.key, label),
            );
            draft.name = unique(draft.name);
            draft.description = format!(
                "{}「{}」取{}：{}",
                field.location_cn(),
                field.key,
                label,
                preview_value(&value)
            );
            apply_value(&mut draft, field, &value, endpoint);
            // 边界值断言宽松：2xx-3xx 或 4xx 均记录，不预设失败。
            draft
                .notes
                .push("边界值行为依业务而定：记录实际状态码并与服务端约定对齐".into());
            drafts.push(draft);
            boundary_count += 1;
        }
    }

    // ---- 5 非法参数（畸形 JSON / 注入串）----
    match &spec.body {
        BodySpec::Json { raw } => {
            let mut draft = base_draft(endpoint, "illegal", "负向", "畸形 JSON 请求体");
            draft.name = unique(draft.name);
            draft.description = "请求体 JSON 语法非法（缺右括号），预期 400 且服务端不崩溃".into();
            draft.body_content = format!("{raw}}}");
            draft.assertions = status_range_assertions(400, 500);
            drafts.push(draft);
        }
        _ if !spec.params.is_empty() => {
            let mut draft = base_draft(endpoint, "illegal", "安全性", "参数注入探测");
            draft.name = unique(draft.name);
            draft.description = "必填参数填入脚本注入样式字符串，验证服务端转义与校验".into();
            if let Some(field) = required.first() {
                apply_value(&mut draft, field, "<script>alert(1)</script>", endpoint);
            }
            draft.assertions = serde_json::json!([
                { "type": "status", "op": "lt", "expected": 500 }
            ]);
            draft
                .notes
                .push("核心关注点：状态码不为 5xx，且响应中不回显未转义的注入串".into());
            drafts.push(draft);
        }
        _ => {}
    }

    // ---- 6 权限不足（有认证配置时）----
    if !matches!(spec.auth, miyo_core::model::AuthSpec::None) {
        let mut draft = base_draft(endpoint, "no_auth", "安全性", "去除认证（权限不足）");
        draft.name = unique(draft.name);
        draft.description = "剥离认证信息发送，预期 401/403；验证接口确有鉴权".into();
        draft.headers.retain(|h| !is_auth_header(&h.key));
        draft
            .notes
            .push("用例级认证覆盖为 none：保存后 auth 字段置为 none 即生效".into());
        draft.assertions = serde_json::json!([
            { "type": "status", "op": "gte", "expected": 400 },
            { "type": "status", "op": "lt", "expected": 500 }
        ]);
        drafts.push(draft);
    }

    // ---- 7 资源不存在（改写路径变量 / id 形态参数为随机值）----
    let mut not_found = base_draft(endpoint, "not_found", "负向", "资源不存在");
    not_found.name = unique(not_found.name);
    not_found.description = "路径变量 / id 类参数替换为随机 UUID，预期 404".into();
    let mut mutated = false;
    for pv in &mut not_found.path_variables {
        pv.value = Uuid::new_v4().to_string();
        mutated = true;
    }
    if !mutated {
        for p in &mut not_found.params {
            if p.key.to_lowercase().contains("id") || p.key.to_lowercase().ends_with("no") {
                p.value = Uuid::new_v4().to_string();
                mutated = true;
                break;
            }
        }
    }
    if mutated {
        not_found.assertions = serde_json::json!([
            { "type": "status", "op": "eq", "expected": 404 }
        ]);
        drafts.push(not_found);
    }

    // ---- 8 / 9 写接口：重复请求 + 幂等性 ----
    if write {
        let mut duplicate = base_draft(endpoint, "duplicate", "其他", "重复请求（防重）");
        duplicate.name = unique(duplicate.name);
        duplicate.description = "同一请求连续发送两次，第二次不应产生新的业务资源".into();
        duplicate
            .notes
            .push("执行方式：同名请求原样重发一次；对比两次响应中的资源 id 是否相同".into());
        duplicate.assertions = serde_json::json!([
            { "type": "status", "op": "lt", "expected": 500 }
        ]);
        drafts.push(duplicate);

        let mut idempotency = base_draft(endpoint, "idempotency", "其他", "幂等键重放");
        idempotency.name = unique(idempotency.name);
        idempotency.description =
            "携带相同 Idempotency-Key 重发，预期服务端返回同一结果而非重复执行".into();
        idempotency.headers.push(KeyValueDto {
            key: "Idempotency-Key".into(),
            value: format!("miyo-draft-{}", Uuid::new_v4()),
            enabled: true,
        });
        idempotency
            .notes
            .push("执行方式：同一 Idempotency-Key 连发两次；若服务端支持应返回首次结果".into());
        idempotency.assertions = serde_json::json!([
            { "type": "status", "op": "lt", "expected": 500 }
        ]);
        drafts.push(idempotency);
    }

    drafts
}

#[derive(Debug, Clone)]
struct RequiredField {
    key: String,
    /// query / path / body。
    location: &'static str,
    declared_type: Option<FieldType>,
}

impl RequiredField {
    fn location_cn(&self) -> &'static str {
        match self.location {
            "query" => "查询参数",
            "path" => "路径变量",
            "body" => "Body 字段",
            _ => "参数",
        }
    }

    fn type_cn(&self) -> &'static str {
        match self.declared_type {
            Some(FieldType::Number) => "number",
            Some(FieldType::Boolean) => "boolean",
            Some(FieldType::Object) => "object",
            _ => "string",
        }
    }
}

/// 收集必填字段（query / 路径变量 / JSON body 键；body_docs 标注优先）。
fn required_fields(endpoint: &Endpoint) -> Vec<RequiredField> {
    let spec = &endpoint.request;
    let mut out: Vec<RequiredField> = Vec::new();
    for kv in spec.params.iter().filter(|k| k.enabled && k.required) {
        out.push(RequiredField {
            key: kv.key.clone(),
            location: "query",
            declared_type: Some(kv.field_type),
        });
    }
    for kv in spec
        .path_variables
        .iter()
        .filter(|k| k.enabled && k.required)
    {
        out.push(RequiredField {
            key: kv.key.clone(),
            location: "path",
            declared_type: Some(kv.field_type),
        });
    }
    if let BodySpec::Json { raw } = &spec.body {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw) {
            for key in map.keys() {
                let required = spec
                    .body_docs
                    .get(&format!("/{key}"))
                    .map(|d| d.required)
                    .unwrap_or(true);
                if required {
                    out.push(RequiredField {
                        key: key.clone(),
                        location: "body",
                        declared_type: None,
                    });
                }
            }
        }
    }
    out
}

/// 以接口当前配置为底稿构造草稿（含启用的参数 / 头 / Body 原文）。
fn base_draft(endpoint: &Endpoint, kind: &str, category: &str, name: &str) -> TestCaseDraft {
    let spec = &endpoint.request;
    let (body_type, body_content) = match &spec.body {
        BodySpec::Json { raw } => ("json", raw.clone()),
        BodySpec::Text { raw } => ("raw", raw.clone()),
        BodySpec::UrlEncoded { fields } => (
            "urlencoded",
            fields
                .iter()
                .filter(|kv| kv.enabled)
                .map(|kv| format!("{}={}", kv.key, kv.value))
                .collect::<Vec<_>>()
                .join("&"),
        ),
        _ => ("none", String::new()),
    };
    TestCaseDraft {
        kind: kind.into(),
        category: category.into(),
        name: name.into(),
        description: String::new(),
        method: endpoint.method.as_str().to_string(),
        url_path: endpoint.path.clone(),
        path_variables: spec
            .path_variables
            .iter()
            .filter(|kv| kv.enabled)
            .map(KeyValueDto::from_kv)
            .collect(),
        params: spec
            .params
            .iter()
            .filter(|kv| kv.enabled)
            .map(KeyValueDto::from_kv)
            .collect(),
        headers: spec
            .headers
            .iter()
            .filter(|kv| kv.enabled && !is_auth_header(&kv.key))
            .map(KeyValueDto::from_kv)
            .collect(),
        body_type: body_type.into(),
        body_content,
        assertions: serde_json::Value::Null,
        notes: Vec::new(),
    }
}

fn status_range_assertions(min: u16, max: u16) -> serde_json::Value {
    serde_json::json!([
        { "type": "status", "op": "gte", "expected": min },
        { "type": "status", "op": "lt", "expected": max }
    ])
}

fn is_auth_header(key: &str) -> bool {
    let k = key.to_lowercase();
    k.contains("authorization")
        || k.contains("token")
        || k.contains("cookie")
        || k.contains("signature")
}

/// 移除字段（缺必填用例）：query 参数禁用 / 路径变量清值 / Body 删键。
fn apply_removal(draft: &mut TestCaseDraft, field: &RequiredField, endpoint: &Endpoint) {
    match field.location {
        "query" => {
            if let Some(p) = draft.params.iter_mut().find(|p| p.key == field.key) {
                p.enabled = false;
            }
        }
        "path" => {
            if let Some(p) = draft.path_variables.iter_mut().find(|p| p.key == field.key) {
                p.value = String::new();
            }
        }
        "body" => {
            if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&draft.body_content) {
                if let serde_json::Value::Object(map) = &mut value {
                    map.remove(&field.key);
                    draft.body_content = serde_json::to_string(&value).unwrap_or_default();
                }
            }
        }
        _ => {}
    }
    let _ = endpoint;
}

/// 覆写字段值（错误类型 / 边界值 / 注入 / 资源不存在用例）。
fn apply_value(draft: &mut TestCaseDraft, field: &RequiredField, value: &str, endpoint: &Endpoint) {
    match field.location {
        "query" => {
            if let Some(p) = draft.params.iter_mut().find(|p| p.key == field.key) {
                p.value = value.to_string();
            } else {
                draft.params.push(KeyValueDto {
                    key: field.key.clone(),
                    value: value.to_string(),
                    enabled: true,
                });
            }
        }
        "path" => {
            if let Some(p) = draft.path_variables.iter_mut().find(|p| p.key == field.key) {
                p.value = value.to_string();
            }
        }
        "body" => {
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&draft.body_content) {
                if let Ok(new_value) = serde_json::from_str::<serde_json::Value>(value) {
                    if let serde_json::Value::Object(map) = &mut v {
                        map.insert(field.key.clone(), new_value);
                        draft.body_content = serde_json::to_string(&v).unwrap_or_default();
                    }
                }
            }
        }
        _ => {}
    }
    let _ = endpoint;
}

/// 按声明类型生成类型不匹配的值。
fn wrong_typed_value(field: &RequiredField) -> Option<String> {
    match field.declared_type {
        Some(FieldType::Number) => Some("\"not-a-number\"".into()),
        Some(FieldType::Boolean) => Some("\"not-a-boolean\"".into()),
        Some(FieldType::Object) => Some("42".into()),
        // 字符串字段填数组（JSON body 才有意义；query 参数无类型系统，跳过）。
        _ if field.location == "body" => Some("[1,2,3]".into()),
        _ => None,
    }
}

/// 边界值候选（每字段最多 2 个，防爆炸）。
fn boundary_values(field: &RequiredField) -> Vec<(&'static str, String)> {
    match field.declared_type {
        Some(FieldType::Number) => vec![("零值", "0".into()), ("负值", "-1".into())],
        _ => vec![("空串", String::new()), ("超长串", "x".repeat(256))],
    }
}

fn preview_value(value: &str) -> String {
    if value.len() > 32 {
        format!("{}…（{} 字符）", &value[..32], value.len())
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{EndpointStatus, HttpMethod, RequestSpec};

    fn endpoint(body: &str, params: Vec<miyo_core::model::KeyValue>) -> Endpoint {
        let now = chrono::Utc::now();
        Endpoint {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            folder_id: None,
            name: "创建订单".into(),
            method: HttpMethod::POST,
            path: "/shop/orders".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec {
                params,
                body: BodySpec::Json { raw: body.into() },
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn generates_nine_kinds_for_write_endpoint() {
        let mut ep = endpoint(
            r#"{"sku": "S1", "quantity": 2}"#,
            vec![miyo_core::model::KeyValue::new("channel", "alipay")],
        );
        // 带路径变量，覆盖「资源不存在」的变异目标。
        ep.path = "/shop/orders/{orderNo}".into();
        ep.request.path_variables = vec![miyo_core::model::KeyValue::new("orderNo", "SO-1")];
        let drafts = generate_drafts(&ep, &[]);

        let kinds: BTreeSet<&str> = drafts.iter().map(|d| d.kind.as_str()).collect();
        for expected in [
            "normal",
            "missing_required",
            "wrong_type",
            "boundary",
            "illegal",
            "not_found",
            "duplicate",
            "idempotency",
        ] {
            assert!(kinds.contains(expected), "缺少 {expected} 草稿");
        }
        // 全部草稿归类到存库类目。
        assert!(drafts.iter().all(|d| {
            matches!(
                d.category.as_str(),
                "正向" | "负向" | "边界值" | "安全性" | "其他"
            )
        }));

        // 缺必填：Body 删 sku 键的那条草稿。
        let missing = drafts
            .iter()
            .filter(|d| d.kind == "missing_required")
            .find(|d| d.description.contains("sku"))
            .unwrap();
        assert!(!missing.body_content.contains("sku"));
        // 错误类型：字符串字段填数组。
        let wrong = drafts.iter().find(|d| d.kind == "wrong_type").unwrap();
        assert!(wrong.body_content.contains("[1,2,3]"));
        // 畸形 JSON。
        let illegal = drafts.iter().find(|d| d.kind == "illegal").unwrap();
        assert!(illegal.body_content.ends_with('}'));
        // 幂等键。
        let idem = drafts.iter().find(|d| d.kind == "idempotency").unwrap();
        assert!(idem.headers.iter().any(|h| h.key == "Idempotency-Key"));
    }

    #[test]
    fn no_auth_draft_only_when_auth_configured() {
        let ep = endpoint(r#"{"a":1}"#, vec![]);
        let drafts = generate_drafts(&ep, &[]);
        assert!(!drafts.iter().any(|d| d.kind == "no_auth"));

        let mut with_auth = endpoint(r#"{"a":1}"#, vec![]);
        with_auth.request.auth = miyo_core::model::AuthSpec::Bearer {
            token: "{{token}}".into(),
        };
        let drafts = generate_drafts(&with_auth, &[]);
        assert!(drafts.iter().any(|d| d.kind == "no_auth"));
    }

    #[test]
    fn duplicate_names_get_suffix() {
        let ep = endpoint(r#"{"a":1}"#, vec![]);
        let existing = vec![TestCase {
            id: Uuid::new_v4(),
            request_id: ep.id,
            name: "正常请求".into(),
            category: "正向".into(),
            folder: String::new(),
            method: HttpMethod::POST,
            url_path: "/shop/orders".into(),
            path_variables: vec![],
            params: vec![],
            headers: vec![],
            body_type: "json".into(),
            body_content: r#"{"a":1}"#.into(),
            assertions: serde_json::Value::Null,
            extractors: serde_json::Value::Null,
            auth: None,
            timeout_ms: None,
            disable_cookies: false,
            sort_order: 0,
            last_run_status: miyo_core::model::TestCaseStatus::Untested,
            last_run_at: None,
            last_run_ok: None,
            last_run_fail: None,
            created_at: chrono::Utc::now(),
        }];
        let drafts = generate_drafts(&ep, &existing);
        let normal = drafts.iter().find(|d| d.kind == "normal").unwrap();
        assert!(normal.name.contains("AI 生成"));
    }
}
