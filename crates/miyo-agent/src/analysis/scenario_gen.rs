//! 集成测试场景草稿物化（不落库；保存由调用方走既有 save_scenario）。
//!
//! 输入是 AI / 前端给出的步骤意图（接口引用 + 变量绑定 + 提取 / 断言），
//! 物化器负责：
//! 1. 校验 endpointId 存在、inline 步骤 method/url 齐全；
//! 2. 变量绑定（bindings）：引用式步骤翻译为 `overrides`（params /
//!    path_variables 按 key 合并、body 整体替换，语义与
//!    `miyo-core::apply_step_overrides` 一致）；内联步骤直接改规格；
//! 3. **自动补提取**：后续步骤引用的 `{{var}}` 无产出时，依据依赖分析
//!    （响应示例 / Mock 模板字段）回溯生产者步骤并补充 extract 规则；
//! 4. 产出变量依赖图与执行计划（写操作风险清单）。

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{
    BodySpec, Endpoint, KeyValue, RequestSpec, Scenario, ScenarioInlineRequest, ScenarioStep,
};
use miyo_core::{AppError, Result};
use miyo_storage::repository as repo;

static VAR_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").unwrap());
static MOCK_TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{[^}]*\}\}").unwrap());

/// 单步意图（LLM / 前端输入；字段尽量宽松）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDraftStepSpec {
    /// 引用式步骤的接口 id（与 inline 二选一；引用式优先）。
    #[serde(default)]
    pub endpoint_id: Option<Uuid>,
    /// 内联请求：method + url（支持 `{{var}}`）。
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// 内联 JSON Body 原文（可选）。
    #[serde(default)]
    pub body: Option<String>,
    /// 步骤名（缺省用接口名 / `步骤 N`）。
    #[serde(default)]
    pub name: Option<String>,
    /// 变量绑定：请求字段名 → 值模板（`{{var}}` 或字面量）。
    /// 引用式步骤 → overrides；内联步骤 → 直接写入规格。
    #[serde(default)]
    pub bindings: HashMap<String, String>,
    /// 变量提取（ExtractSpec 形状：name / path / from 可选）。
    #[serde(default)]
    pub extract: Option<serde_json::Value>,
    /// 断言（AssertionSpec 形状：type / path / op / expected）。
    #[serde(default)]
    pub assertions: Option<serde_json::Value>,
    /// 执行前等待毫秒。
    #[serde(default)]
    pub wait_ms: Option<u64>,
    /// 执行条件（StepCondition JSON 透传）。
    #[serde(default)]
    pub condition: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDraftRequest {
    /// 目标项目（MCP 路径缺省时由 server 层自动选中唯一项目后回填）。
    #[serde(default)]
    pub project_id: Option<Uuid>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    pub steps: Vec<ScenarioDraftStepSpec>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VarDependency {
    pub step_index: usize,
    pub step_name: String,
    /// 本步提取产出的变量。
    pub produces: Vec<String>,
    /// 本步请求引用的 `{{var}}`。
    pub consumes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOperation {
    pub step_index: usize,
    pub step_name: String,
    pub method: String,
    pub url: String,
    /// 风险说明（写操作副作用提示）。
    pub risk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDraftResult {
    /// 可直接 save_scenario 的完整场景（含生成的 id / 时间戳）。
    pub scenario: Scenario,
    /// 物化期间自动做的调整（供 AI / 用户复核）。
    pub adjustments: Vec<String>,
    /// 未解决 / 需人工确认的警告。
    pub warnings: Vec<String>,
    pub var_dependencies: Vec<VarDependency>,
    pub write_operations: Vec<WriteOperation>,
}

/// 物化场景草稿。
pub async fn materialize_scenario_draft(
    db: &SqlitePool,
    request: &ScenarioDraftRequest,
) -> Result<ScenarioDraftResult> {
    if request.name.trim().is_empty() {
        return Err(AppError::Validation("场景名称不能为空".into()));
    }
    if request.steps.is_empty() {
        return Err(AppError::Validation("场景至少需要一个步骤".into()));
    }
    let Some(project_id) = request.project_id else {
        return Err(AppError::Validation(
            "缺少 projectId（调用方应先解析目标项目）".into(),
        ));
    };
    let endpoints = repo::list_endpoints(db, project_id).await?;
    let endpoint_ids: Vec<Uuid> = endpoints.iter().map(|e| e.id).collect();
    let response_examples = repo::list_response_examples_by_endpoints(db, &endpoint_ids).await?;
    let mock_rules = repo::list_mock_rules(db, project_id).await?;
    let endpoint_by_id: HashMap<Uuid, &Endpoint> = endpoints.iter().map(|e| (e.id, e)).collect();

    // Mock 响应模板按 endpoint 聚合（响应示例缺失时的自动补提取证据兜底）。
    let mut mock_templates: HashMap<Uuid, Vec<String>> = HashMap::new();
    for rule in &mock_rules {
        if let Some(ep) = rule.endpoint_id {
            mock_templates
                .entry(ep)
                .or_default()
                .push(rule.response_body_template.clone());
        }
    }

    let mut adjustments = Vec::new();
    let mut warnings = Vec::new();
    let mut steps: Vec<ScenarioStep> = Vec::new();
    // 每步请求文本（`{{var}}` 引用扫描用）与变量产出表（var → 步骤位次）。
    let mut step_texts: Vec<String> = Vec::new();
    let mut produced_vars: HashMap<String, usize> = HashMap::new();

    for (index, spec) in request.steps.iter().enumerate() {
        // ---- 解析步骤请求底稿 ----
        let (step_name, method, url, inline_spec, endpoint_id) = if let Some(ep_id) =
            spec.endpoint_id
        {
            let ep = endpoint_by_id.get(&ep_id).copied().ok_or_else(|| {
                AppError::Validation(format!(
                    "步骤 {}：接口（{ep_id}）不存在，请先 list_endpoints 获取有效 id",
                    index + 1
                ))
            })?;
            (
                spec.name.clone().unwrap_or_else(|| ep.name.clone()),
                ep.method.as_str().to_string(),
                ep.path.clone(),
                None,
                Some(ep_id),
            )
        } else {
            let method = spec
                .method
                .clone()
                .ok_or_else(|| {
                    AppError::Validation(format!("步骤 {}：缺少 method 或 endpointId", index + 1))
                })?
                .to_uppercase();
            let url = spec.url.clone().ok_or_else(|| {
                AppError::Validation(format!("步骤 {}：缺少 url 或 endpointId", index + 1))
            })?;
            let body_spec = spec.body.clone().map(|raw| BodySpec::Json { raw });
            let mut inline = RequestSpec::default();
            if let Some(b) = body_spec {
                inline.body = b;
            }
            (
                spec.name
                    .clone()
                    .unwrap_or_else(|| format!("步骤 {}", index + 1)),
                method,
                url,
                Some(inline),
                None,
            )
        };

        // ---- 变量绑定：引用式 → overrides；内联 → 改规格 ----
        let mut overrides: Option<serde_json::Value> = None;
        let mut effective_spec = inline_spec.clone().unwrap_or_default();
        if !spec.bindings.is_empty() {
            if let Some(ep_id) = endpoint_id {
                let ep = endpoint_by_id.get(&ep_id).copied().unwrap();
                let ov = build_overrides(ep, &spec.bindings);
                let ov_has_keys = ov.as_object().map(|m| !m.is_empty()).unwrap_or(false);
                if ov_has_keys {
                    adjustments.push(format!(
                        "步骤 {}：应用变量绑定 {:?}（overrides）",
                        index + 1,
                        spec.bindings.keys().collect::<Vec<_>>()
                    ));
                    overrides = Some(serde_json::to_value(&ov).unwrap_or_default());
                }
            } else {
                apply_bindings_inline(&mut effective_spec, &spec.bindings);
                adjustments.push(format!(
                    "步骤 {}：应用变量绑定 {:?}（内联规格）",
                    index + 1,
                    spec.bindings.keys().collect::<Vec<_>>()
                ));
            }
        }

        // 请求文本（引用扫描）：引用式 = 接口配置 + overrides；内联 = 规格 + url。
        let text = match (&inline_spec, &overrides) {
            (Some(s), _) => format!("{url} {}", serde_json::to_string(s).unwrap_or_default()),
            (None, Some(ov)) => {
                let ep = endpoint_by_id.get(&endpoint_id.unwrap()).copied().unwrap();
                format!("{} {} {}", ep.method.as_str(), ep.path, ov)
            }
            (None, None) => {
                let ep = endpoint_by_id.get(&endpoint_id.unwrap()).copied().unwrap();
                format!(
                    "{} {} {}",
                    ep.method.as_str(),
                    ep.path,
                    serde_json::to_string(&ep.request).unwrap_or_default()
                )
            }
        };

        // ---- 提取与断言 ----
        let extract = validate_extract(spec.extract.as_ref(), index)?;
        let assertions = validate_assertions(spec.assertions.as_ref(), index)?;
        for name in extract_names(&extract) {
            produced_vars.insert(name, index);
        }

        steps.push(ScenarioStep {
            endpoint_id,
            enabled: true,
            extract,
            assertions,
            name: Some(step_name.clone()),
            request: inline_spec.map(|s| ScenarioInlineRequest {
                method: method.clone(),
                url: url.clone(),
                spec: s,
            }),
            condition: spec.condition.clone(),
            r#loop: None,
            wait_ms: spec.wait_ms,
            parallel: false,
            overrides,
            retry: None,
        });
        step_texts.push(text.clone());

        // ---- 自动补提取：引用的变量无产出时回溯生产者 ----
        for var in var_refs_in(&text) {
            if produced_vars.contains_key(&var) {
                continue;
            }
            let mut producer: Option<(usize, String, String)> = None; // (idx, name, jsonpath)
            for (producer_idx, producer_step) in steps.iter().enumerate() {
                let Some(pe) = producer_step
                    .endpoint_id
                    .and_then(|id| endpoint_by_id.get(&id).copied())
                else {
                    continue;
                };
                if let Some((_, path)) =
                    response_field_paths(pe, &response_examples, &mock_templates)
                        .into_iter()
                        .find(|(f, _)| f == &var)
                {
                    producer = Some((
                        producer_idx,
                        producer_step.name.clone().unwrap_or_default(),
                        path,
                    ));
                    break;
                }
            }
            if let Some((producer_idx, producer_name, path)) = producer {
                let rule = serde_json::json!([{ "name": var, "from": "body", "path": path }]);
                steps[producer_idx].extract = merge_extract(&steps[producer_idx].extract, &rule);
                produced_vars.insert(var.clone(), producer_idx);
                adjustments.push(format!(
                    "自动补充提取：步骤 {}（{producer_name}）新增提取 {var} ← {path}（下游步骤 {} 引用了该变量）",
                    producer_idx + 1,
                    index + 1
                ));
            } else {
                warnings.push(format!(
                    "步骤 {} 引用变量 {{{{{var}}}}}：未找到产出步骤（也非项目 / 环境变量），运行前请配置提取规则或变量",
                    index + 1
                ));
            }
        }
    }

    // ---- 变量依赖图 + 写操作清单 ----
    let mut var_dependencies = Vec::new();
    let mut write_operations = Vec::new();
    for (index, (step, text)) in steps.iter().zip(&step_texts).enumerate() {
        let produces = extract_names(&step.extract);
        let consumes: Vec<String> = var_refs_in(text)
            .into_iter()
            .filter(|v| v != "base_url")
            .collect();
        var_dependencies.push(VarDependency {
            step_index: index,
            step_name: step
                .name
                .clone()
                .unwrap_or_else(|| format!("步骤 {}", index + 1)),
            produces,
            consumes,
        });
        // 方法与 URL：引用式取接口当前配置，内联取自身。
        let (method, url) = match &step.request {
            Some(inline) => (inline.method.clone(), inline.url.clone()),
            None => {
                let ep = step
                    .endpoint_id
                    .and_then(|id| endpoint_by_id.get(&id).copied());
                (
                    ep.map(|e| e.method.as_str().to_string())
                        .unwrap_or_default(),
                    ep.map(|e| e.path.clone()).unwrap_or_default(),
                )
            }
        };
        if matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            write_operations.push(WriteOperation {
                step_index: index,
                step_name: step
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("步骤 {}", index + 1)),
                method,
                url,
                risk: "写操作：真实执行会产生业务副作用，运行前需要确认".into(),
            });
        }
    }

    let now = chrono::Utc::now();
    let scenario = Scenario {
        id: Uuid::new_v4(),
        project_id,
        name: request.name.trim().to_string(),
        description: request.description.clone(),
        category: request.category.clone(),
        sort_order: 0,
        steps,
        script: None,
        setup: vec![],
        teardown: vec![],
        teardown_enabled: true,
        data_sets: vec![],
        created_at: now,
        updated_at: now,
    };

    Ok(ScenarioDraftResult {
        scenario,
        adjustments,
        warnings,
        var_dependencies,
        write_operations,
    })
}

/// 把绑定翻译为引用式步骤的 overrides：请求字段名在接口 params /
/// path_variables / JSON Body 顶层键中命中则覆写对应位置，未命中追加为
/// query 参数（运行期按 `{{var}}` 渲染）。
fn build_overrides(endpoint: &Endpoint, bindings: &HashMap<String, String>) -> serde_json::Value {
    let spec = &endpoint.request;
    let mut params: Vec<serde_json::Value> = Vec::new();
    let mut path_variables: Vec<serde_json::Value> = Vec::new();
    let mut body_override: Option<serde_json::Value> = None;
    // Body 绑定集中在一份替换后的 JSON 上。
    let mut body_map: Option<serde_json::Map<String, serde_json::Value>> = match &spec.body {
        BodySpec::Json { raw } => serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.as_object().cloned()),
        _ => None,
    };

    for (key, value) in bindings {
        if spec.params.iter().any(|kv| kv.key == *key) {
            params.push(serde_json::json!({ "key": key, "value": value, "enabled": true }));
        } else if spec.path_variables.iter().any(|kv| kv.key == *key) {
            path_variables.push(serde_json::json!({ "key": key, "value": value, "enabled": true }));
        } else if body_map
            .as_ref()
            .is_some_and(|m| m.contains_key(key.as_str()))
        {
            if let Some(map) = &mut body_map {
                map.insert(
                    key.clone(),
                    serde_json::from_str::<serde_json::Value>(value)
                        .unwrap_or(serde_json::Value::String(value.clone())),
                );
            }
        } else {
            params.push(serde_json::json!({ "key": key, "value": value, "enabled": true }));
        }
    }
    if let Some(map) = body_map {
        let raw = serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_default();
        body_override = Some(serde_json::json!({ "mode": "json", "raw": raw }));
    }

    let mut obj = serde_json::Map::new();
    if !params.is_empty() {
        obj.insert("params".into(), serde_json::Value::Array(params));
    }
    if !path_variables.is_empty() {
        obj.insert(
            "path_variables".into(),
            serde_json::Value::Array(path_variables),
        );
    }
    if let Some(body) = body_override {
        obj.insert("body".into(), body);
    }
    serde_json::Value::Object(obj)
}

/// 内联步骤绑定：query / 路径变量 / JSON Body 顶层键同名覆写，未命中追加 query。
fn apply_bindings_inline(spec: &mut RequestSpec, bindings: &HashMap<String, String>) {
    for (key, value) in bindings {
        let mut applied = false;
        for kv in spec.params.iter_mut().chain(spec.path_variables.iter_mut()) {
            if kv.key == *key {
                kv.value = value.clone();
                kv.enabled = true;
                applied = true;
            }
        }
        if !applied {
            if let BodySpec::Json { raw } = &mut spec.body {
                if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(raw) {
                    if let serde_json::Value::Object(map) = &mut v {
                        map.insert(
                            key.clone(),
                            serde_json::from_str::<serde_json::Value>(value)
                                .unwrap_or(serde_json::Value::String(value.clone())),
                        );
                        *raw = serde_json::to_string(&v).unwrap_or_default();
                        applied = true;
                    }
                }
            }
        }
        if !applied {
            spec.params.push(KeyValue {
                key: key.clone(),
                value: value.clone(),
                enabled: true,
                description: String::new(),
                field_type: Default::default(),
                required: false,
                example: String::new(),
            });
        }
    }
}

/// `{{var}}` 引用收集。
fn var_refs_in(text: &str) -> Vec<String> {
    VAR_REF_RE
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect()
}

/// extract JSON 校验与归一（数组形状；非法时报 Validation）。
fn validate_extract(value: Option<&serde_json::Value>, index: usize) -> Result<serde_json::Value> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(serde_json::Value::Null),
        Some(v) if v.is_array() => {
            for item in v.as_array().unwrap() {
                if item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err(AppError::Validation(format!(
                        "步骤 {}：提取规则缺少 name",
                        index + 1
                    )));
                }
            }
            Ok(v.clone())
        }
        Some(_) => Err(AppError::Validation(format!(
            "步骤 {}：extract 必须为数组",
            index + 1
        ))),
    }
}

/// assertions JSON 校验（数组形状，type 必填）。
fn validate_assertions(
    value: Option<&serde_json::Value>,
    index: usize,
) -> Result<serde_json::Value> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(serde_json::Value::Null),
        Some(v) if v.is_array() => {
            for item in v.as_array().unwrap() {
                if item
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err(AppError::Validation(format!(
                        "步骤 {}：断言缺少 type",
                        index + 1
                    )));
                }
            }
            Ok(v.clone())
        }
        Some(_) => Err(AppError::Validation(format!(
            "步骤 {}：assertions 必须为数组",
            index + 1
        ))),
    }
}

fn extract_names(extract: &serde_json::Value) -> Vec<String> {
    serde_json::from_value::<Vec<serde_json::Value>>(extract.clone())
        .map(|list| {
            list.iter()
                .filter_map(|i| {
                    i.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn merge_extract(base: &serde_json::Value, addition: &serde_json::Value) -> serde_json::Value {
    let mut list = base.as_array().cloned().unwrap_or_default();
    if let Some(add) = addition.as_array() {
        list.extend(add.iter().cloned());
    }
    serde_json::Value::Array(list)
}

/// 生产者响应字段与 JSONPath（响应示例优先，Mock 模板兜底）。
pub(crate) fn response_field_paths(
    endpoint: &Endpoint,
    response_examples: &HashMap<Uuid, Vec<miyo_core::model::ResponseExample>>,
    mock_templates: &HashMap<Uuid, Vec<String>>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let collect_from =
        |body: &str, out: &mut Vec<(String, String)>, seen: &mut BTreeSet<String>| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
                collect_leaf_paths(&v, "$", 3, out, seen);
            }
        };
    if let Some(examples) = response_examples.get(&endpoint.id) {
        for ex in examples {
            collect_from(&ex.body, &mut out, &mut seen);
        }
    }
    if out.is_empty() {
        if let Some(templates) = mock_templates.get(&endpoint.id) {
            // 模板值形如 "SO{{mock.uuid}}"：占位替换为 1 再解析。
            for tpl in templates {
                let concrete = MOCK_TEMPLATE_RE.replace_all(tpl, "1");
                collect_from(&concrete, &mut out, &mut seen);
            }
        }
    }
    out
}

/// 递归收集叶子字段路径（限深；数组取首元素）。
fn collect_leaf_paths(
    value: &serde_json::Value,
    prefix: &str,
    depth: u8,
    out: &mut Vec<(String, String)>,
    seen: &mut BTreeSet<String>,
) {
    if depth == 0 {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let path = format!("{prefix}.{k}");
                if seen.insert(k.clone()) {
                    out.push((k.clone(), path.clone()));
                }
                collect_leaf_paths(v, &path, depth - 1, out, seen);
            }
        }
        serde_json::Value::Array(list) => {
            if let Some(first) = list.first() {
                collect_leaf_paths(first, &format!("{prefix}[0]"), depth - 1, out, seen);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{EndpointStatus, HttpMethod};
    use miyo_storage::db::memory_pool;

    async fn seed_shop(db: &sqlx::SqlitePool) -> (Uuid, Uuid, Uuid) {
        let now = chrono::Utc::now();
        let project = repo::create_project(db, "场景物化", "").await.unwrap();
        let create_order = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "创建订单".into(),
            method: HttpMethod::POST,
            path: "/shop/orders".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec {
                body: BodySpec::Json {
                    raw: r#"{"sku":"S1","quantity":2}"#.into(),
                },
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        };
        let preview = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "预览支付".into(),
            method: HttpMethod::GET,
            path: "/shop/orders/{orderNo}/pay-preview".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 1,
            request: RequestSpec {
                params: vec![KeyValue::new("payToken", "{{payToken}}")],
                path_variables: vec![KeyValue::new("orderNo", "{{orderNo}}")],
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        };
        repo::save_endpoint(db, &create_order).await.unwrap();
        repo::save_endpoint(db, &preview).await.unwrap();
        let example = miyo_core::model::ResponseExample {
            id: Uuid::new_v4(),
            endpoint_id: create_order.id,
            name: "成功".into(),
            status: 201,
            headers: Default::default(),
            body: r#"{"orderNo":"SO1","payToken":"pt1"}"#.into(),
            content_type: "application/json".into(),
            docs: Default::default(),
            created_at: now,
            updated_at: now,
        };
        repo::create_response_example(db, example.endpoint_id, &example)
            .await
            .unwrap();
        (project.id, create_order.id, preview.id)
    }

    #[tokio::test]
    async fn materializes_and_auto_fills_extract() {
        let db = memory_pool().await.unwrap();
        let (project_id, create_id, preview_id) = seed_shop(&db).await;

        let request = ScenarioDraftRequest {
            project_id: Some(project_id),
            name: "下单支付链路".into(),
            description: "AI 生成".into(),
            category: String::new(),
            steps: vec![
                ScenarioDraftStepSpec {
                    endpoint_id: Some(create_id),
                    method: None,
                    url: None,
                    body: None,
                    name: None,
                    bindings: HashMap::new(),
                    extract: Some(serde_json::json!([{"name": "orderNo", "path": "$.orderNo"}])),
                    assertions: Some(
                        serde_json::json!([{"type": "status", "op": "eq", "expected": 201}]),
                    ),
                    wait_ms: None,
                    condition: None,
                },
                ScenarioDraftStepSpec {
                    endpoint_id: Some(preview_id),
                    method: None,
                    url: None,
                    body: None,
                    name: None,
                    bindings: HashMap::new(),
                    extract: None,
                    assertions: None,
                    wait_ms: None,
                    condition: None,
                },
            ],
        };
        let draft = materialize_scenario_draft(&db, &request).await.unwrap();

        assert_eq!(draft.scenario.steps.len(), 2);
        assert!(draft.scenario.steps[0].endpoint_id == Some(create_id));
        // 步骤 2 引用 {{payToken}} 无产出 → 自动补到步骤 1（响应示例含 payToken）。
        assert!(!draft.adjustments.is_empty(), "应自动补充 payToken 提取");
        assert!(draft.adjustments[0].contains("payToken"));
        let step1_extract = &draft.scenario.steps[0].extract;
        assert!(step1_extract.to_string().contains("payToken"));
        // 引用式步骤不带内联请求。
        assert!(draft.scenario.steps[0].request.is_none());
        // 写操作清单：步骤 1 是 POST。
        assert_eq!(draft.write_operations.len(), 1);
        assert_eq!(draft.write_operations[0].step_index, 0);
        assert_eq!(draft.write_operations[0].method, "POST");
        // 变量依赖。
        assert!(draft.var_dependencies[0]
            .produces
            .contains(&"orderNo".to_string()));
        assert!(draft.var_dependencies[1]
            .consumes
            .contains(&"orderNo".to_string()));
    }

    #[tokio::test]
    async fn bindings_translate_to_overrides() {
        let db = memory_pool().await.unwrap();
        let (project_id, _, preview_id) = seed_shop(&db).await;
        let mut bindings = HashMap::new();
        bindings.insert("orderNo".to_string(), "{{orderNo}}".to_string());
        bindings.insert("payToken".to_string(), "static-token".to_string());
        let request = ScenarioDraftRequest {
            project_id: Some(project_id),
            name: "绑定场景".into(),
            description: String::new(),
            category: String::new(),
            steps: vec![ScenarioDraftStepSpec {
                endpoint_id: Some(preview_id),
                method: None,
                url: None,
                body: None,
                name: None,
                bindings,
                extract: None,
                assertions: None,
                wait_ms: None,
                condition: None,
            }],
        };
        let draft = materialize_scenario_draft(&db, &request).await.unwrap();
        let ov = draft.scenario.steps[0]
            .overrides
            .as_ref()
            .expect("应有 overrides");
        let pv = ov["path_variables"].as_array().unwrap();
        assert!(pv
            .iter()
            .any(|r| r["key"] == "orderNo" && r["value"] == "{{orderNo}}"));
        let params = ov["params"].as_array().unwrap();
        assert!(params.iter().any(|r| r["key"] == "payToken"));
    }

    #[tokio::test]
    async fn unknown_endpoint_rejected() {
        let db = memory_pool().await.unwrap();
        let (project_id, _, _) = seed_shop(&db).await;
        let request = ScenarioDraftRequest {
            project_id: Some(project_id),
            name: "坏场景".into(),
            description: String::new(),
            category: String::new(),
            steps: vec![ScenarioDraftStepSpec {
                endpoint_id: Some(Uuid::new_v4()),
                method: None,
                url: None,
                body: None,
                name: None,
                bindings: HashMap::new(),
                extract: None,
                assertions: None,
                wait_ms: None,
                condition: None,
            }],
        };
        let err = materialize_scenario_draft(&db, &request).await.unwrap_err();
        assert!(err.to_string().contains("不存在"));
    }

    #[tokio::test]
    async fn inline_steps_supported() {
        let db = memory_pool().await.unwrap();
        let (project_id, _, _) = seed_shop(&db).await;
        let request = ScenarioDraftRequest {
            project_id: Some(project_id),
            name: "内联场景".into(),
            description: String::new(),
            category: String::new(),
            steps: vec![ScenarioDraftStepSpec {
                endpoint_id: None,
                method: Some("GET".into()),
                url: Some("{{base_url}}/health".into()),
                body: None,
                name: Some("健康检查".into()),
                bindings: HashMap::new(),
                extract: None,
                assertions: Some(
                    serde_json::json!([{"type": "status", "op": "eq", "expected": 200}]),
                ),
                wait_ms: None,
                condition: None,
            }],
        };
        let draft = materialize_scenario_draft(&db, &request).await.unwrap();
        let step = &draft.scenario.steps[0];
        assert!(step.endpoint_id.is_none());
        let inline = step.request.as_ref().unwrap();
        assert_eq!(inline.method, "GET");
        assert!(inline.url.contains("{{base_url}}"));
        // GET 不产生写操作。
        assert!(draft.write_operations.is_empty());
    }
}
