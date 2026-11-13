//! 接口业务依赖分析：识别「A 的响应字段 → B 的请求参数」链路。
//!
//! 三层证据（置信度递减）：
//! 1. **场景提取链**（high）：场景中第 i 步 extract 出变量 V，后续步骤请求
//!    引用 `{{V}}`——这是用户已经用脚承认过的数据流，最可靠；
//! 2. **响应字段匹配**（medium）：生产者响应示例 / Mock 响应模板 / 用例提取
//!    规则中出现过的字段名，与消费者请求参数（query / 路径变量 / Body 键 /
//!    `{{var}}` 占位）同名；
//! 3. 同名即连边，不做语义推测——宁缺毋滥，未识别的引用进 `unresolved`
//!    供 AI / 用户补齐。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{BodySpec, Endpoint, RequestSpec, Scenario, ScenarioStep};
use miyo_core::Result;

use miyo_storage::repository as repo;

/// 匹配噪声字段：几乎每个接口都有的通用字段，参与匹配会产生全连接假边。
static NOISE_FIELDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "code",
        "message",
        "msg",
        "success",
        "status",
        "timestamp",
        "trace_id",
        "request_id",
        "page",
        "size",
        "total",
        "data",
        "ok",
        "error",
        "requestId",
        "traceId",
    ]
    .into_iter()
    .collect()
});

static VAR_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").unwrap());

static MOCK_TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{[^}]*\}\}").unwrap());

/// 依赖置信度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// 场景提取链实证。
    High,
    /// 响应字段同名匹配。
    Medium,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyNode {
    pub endpoint_id: Uuid,
    pub name: String,
    pub method: String,
    pub path: String,
    /// 可产出字段（响应示例 / Mock 模板 / 提取规则出现过的字段名）。
    pub produces: Vec<String>,
    /// 请求消费的字段名（query / 路径变量 / Body 键）。
    pub consumes: Vec<String>,
    /// 请求里的 `{{var}}` 占位引用。
    pub var_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    /// 生产者（响应方）。
    pub from: Uuid,
    /// 消费者（请求方）。
    pub to: Uuid,
    pub via: Vec<String>,
    pub confidence: Confidence,
    pub evidence: String,
}

/// 未找到生产者的变量引用（环境变量缺失 / 提取规则缺失的候选）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedVar {
    pub endpoint_id: Uuid,
    pub name: String,
    pub vars: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub project_id: Uuid,
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
    pub unresolved: Vec<UnresolvedVar>,
    /// 面向 AI / 用户的中文摘要。
    pub summary: String,
}

/// 分析项目接口依赖。
pub async fn analyze_api_dependencies(
    db: &SqlitePool,
    project_id: Uuid,
) -> Result<DependencyReport> {
    let endpoints = repo::list_endpoints(db, project_id).await?;
    let scenarios = repo::list_scenarios(db, project_id).await?;
    let endpoint_ids: Vec<Uuid> = endpoints.iter().map(|e| e.id).collect();
    let response_examples = repo::list_response_examples_by_endpoints(db, &endpoint_ids).await?;
    let flat_cases = repo::list_test_cases_by_endpoints(db, &endpoint_ids).await?;
    let mut test_cases: HashMap<Uuid, Vec<miyo_core::model::TestCase>> = HashMap::new();
    for case in flat_cases {
        test_cases.entry(case.request_id).or_default().push(case);
    }
    let mock_rules = repo::list_mock_rules(db, project_id).await?;
    let project = repo::get_project(db, project_id).await?;
    let environments = repo::list_environments(db, project_id).await?;

    // ---- 生产者字段表：endpoint_id → 响应中出现过（或被提取过）的字段名 ----
    let mut produces: HashMap<Uuid, BTreeSet<String>> = HashMap::new();
    for (endpoint_id, examples) in &response_examples {
        for ex in examples {
            if let Ok(v) = serde_json::from_str::<Value>(&ex.body) {
                collect_json_fields(&v, 3, produces.entry(*endpoint_id).or_default());
            }
        }
    }
    for rule in &mock_rules {
        let Some(ep) = rule.endpoint_id else { continue };
        // Mock 模板值可能是 "SO{{mock.uuid}}"：把模板占位替换成 1 再解析。
        let concrete = MOCK_TEMPLATE_RE.replace_all(&rule.response_body_template, "1");
        if let Ok(v) = serde_json::from_str::<Value>(&concrete) {
            collect_json_fields(&v, 3, produces.entry(ep).or_default());
        }
    }
    for (endpoint_id, cases) in &test_cases {
        for case in cases {
            if let Ok(list) =
                serde_json::from_value::<Vec<serde_json::Value>>(case.extractors.clone())
            {
                for item in list {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        produces
                            .entry(*endpoint_id)
                            .or_default()
                            .insert(name.to_string());
                    }
                }
            }
        }
    }
    // 场景 extract 也算生产者产出（引用式步骤的 endpoint 为生产者）。
    let endpoint_by_id: HashMap<Uuid, &Endpoint> = endpoints.iter().map(|e| (e.id, e)).collect();
    for sc in &scenarios {
        for step in &sc.steps {
            let Some(ep) = step.endpoint_id else { continue };
            for name in step_extract_names(step) {
                produces.entry(ep).or_default().insert(name);
            }
        }
    }

    // ---- 消费者字段与 {{var}} 引用 ----
    struct Consumer<'a> {
        endpoint: &'a Endpoint,
        fields: BTreeSet<String>,
        var_refs: BTreeSet<String>,
    }
    let mut consumers = Vec::new();
    for ep in &endpoints {
        let (fields, var_refs) = request_fields(&ep.request);
        consumers.push(Consumer {
            endpoint: ep,
            fields,
            var_refs,
        });
    }

    // ---- 场景链：extract(步骤 i) → {{var}}(步骤 j>i)，高置信度 ----
    // (from, to) → (via 集合, evidence 集合)
    let mut high_edges: HashMap<(Uuid, Uuid), (BTreeSet<String>, BTreeSet<String>)> =
        HashMap::new();
    for sc in &scenarios {
        // 每步的请求文本（引用式步骤取接口当前配置；内联取自身）。
        let step_texts: Vec<String> = sc
            .steps
            .iter()
            .map(|step| step_request_text(step, &endpoint_by_id))
            .collect();
        for (i, step) in sc.steps.iter().enumerate() {
            let Some(producer) = step.endpoint_id else {
                continue;
            };
            let extracted = step_extract_names(step);
            if extracted.is_empty() {
                continue;
            }
            for (j, text) in step_texts.iter().enumerate().skip(i + 1) {
                let consumer_id = sc.steps[j].endpoint_id;
                let Some(consumer_id) = consumer_id else {
                    continue;
                };
                if consumer_id == producer {
                    continue;
                }
                let hits: BTreeSet<String> = extracted
                    .iter()
                    .filter(|v| text_contains_var(text, v))
                    .cloned()
                    .collect();
                if hits.is_empty() {
                    continue;
                }
                let entry = high_edges.entry((producer, consumer_id)).or_default();
                entry.0.extend(hits);
                entry.1.insert(format!(
                    "场景「{}」步骤 {} → 步骤 {}",
                    sc.name,
                    i + 1,
                    j + 1
                ));
            }
        }
    }

    // ---- 同名字段匹配：中置信度 ----
    let mut medium_edges: HashMap<(Uuid, Uuid), BTreeSet<String>> = HashMap::new();
    for c in &consumers {
        let wants: BTreeSet<String> = c
            .fields
            .iter()
            .chain(c.var_refs.iter())
            .map(|s| s.to_lowercase())
            .filter(|f| !NOISE_FIELDS.contains(f.as_str()))
            .collect();
        if wants.is_empty() {
            continue;
        }
        for ep in &endpoints {
            if ep.id == c.endpoint.id {
                continue;
            }
            let Some(produced) = produces.get(&ep.id) else {
                continue;
            };
            let hits: BTreeSet<String> = produced
                .iter()
                .filter(|p| !NOISE_FIELDS.contains(p.to_lowercase().as_str()))
                .filter(|p| wants.contains(&p.to_lowercase()))
                .cloned()
                .collect();
            if !hits.is_empty() {
                medium_edges
                    .entry((ep.id, c.endpoint.id))
                    .or_default()
                    .extend(hits);
            }
        }
    }

    // ---- 合并：高置信度优先，同 (from,to) 不重复出边 ----
    let mut edges = Vec::new();
    let mut covered: HashSet<(Uuid, Uuid)> = HashSet::new();
    for ((from, to), (via, evidence)) in high_edges {
        covered.insert((from, to));
        edges.push(DependencyEdge {
            from,
            to,
            via: via.into_iter().collect(),
            confidence: Confidence::High,
            evidence: evidence.into_iter().collect::<Vec<_>>().join("；"),
        });
    }
    for ((from, to), via) in medium_edges {
        if !covered.insert((from, to)) {
            continue;
        }
        edges.push(DependencyEdge {
            from,
            to,
            via: via.into_iter().collect(),
            confidence: Confidence::Medium,
            evidence: "响应字段与请求参数同名匹配（启发式）".into(),
        });
    }
    edges.sort_by_key(|a| (a.from, a.to));

    // ---- 未解析变量：请求引用了但项目变量 / 环境变量 / 场景产出都覆盖不了 ----
    let mut known_vars: BTreeSet<String> = project.variables.keys().cloned().collect();
    for env in &environments {
        for v in &env.variables {
            if v.is_effective() {
                known_vars.insert(v.key.clone());
            }
        }
        if !env.base_url.is_empty() {
            known_vars.insert("base_url".into());
        }
    }
    for sc in &scenarios {
        for step in &sc.steps {
            for name in step_extract_names(step) {
                known_vars.insert(name);
            }
        }
    }
    let mut unresolved = Vec::new();
    for c in &consumers {
        let missing: Vec<String> = c
            .var_refs
            .iter()
            .filter(|v| !known_vars.contains(*v))
            .cloned()
            .collect();
        if !missing.is_empty() {
            unresolved.push(UnresolvedVar {
                endpoint_id: c.endpoint.id,
                name: c.endpoint.name.clone(),
                vars: missing,
            });
        }
    }

    let nodes: Vec<DependencyNode> = endpoints
        .iter()
        .map(|ep| {
            let c = consumers.iter().find(|c| c.endpoint.id == ep.id);
            DependencyNode {
                endpoint_id: ep.id,
                name: ep.name.clone(),
                method: ep.method.as_str().to_string(),
                path: ep.path.clone(),
                produces: produces
                    .get(&ep.id)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default(),
                consumes: c
                    .map(|c| c.fields.iter().cloned().collect())
                    .unwrap_or_default(),
                var_refs: c
                    .map(|c| c.var_refs.iter().cloned().collect())
                    .unwrap_or_default(),
            }
        })
        .collect();

    let summary = format!(
        "共 {} 个接口、{} 条依赖边（场景链实证 {} 条、同名匹配 {} 条）；{} 个接口存在未解析变量引用。",
        nodes.len(),
        edges.len(),
        edges.iter().filter(|e| e.confidence == Confidence::High).count(),
        edges.iter().filter(|e| e.confidence == Confidence::Medium).count(),
        unresolved.len()
    );

    Ok(DependencyReport {
        project_id,
        nodes,
        edges,
        unresolved,
        summary,
    })
}

/// 递归收集 JSON 字段名（限深）。
fn collect_json_fields(value: &Value, depth: u8, out: &mut BTreeSet<String>) {
    if depth == 0 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                out.insert(k.clone());
                collect_json_fields(v, depth - 1, out);
            }
        }
        Value::Array(list) => {
            if let Some(first) = list.first() {
                collect_json_fields(first, depth - 1, out);
            }
        }
        _ => {}
    }
}

/// 消费者请求字段（query / 路径变量 / urlencoded / JSON body 顶层键）与
/// `{{var}}` 占位引用。
fn request_fields(spec: &RequestSpec) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut fields = BTreeSet::new();
    let mut var_refs = BTreeSet::new();
    let mut scan_value = |v: &str| {
        for cap in VAR_REF_RE.captures_iter(v) {
            var_refs.insert(cap[1].to_string());
        }
    };
    for kv in spec.params.iter().chain(&spec.path_variables) {
        if kv.enabled {
            fields.insert(kv.key.clone());
            scan_value(&kv.value);
        }
    }
    for kv in &spec.headers {
        scan_value(&kv.value);
    }
    match &spec.body {
        BodySpec::Json { raw } => {
            scan_value(raw);
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) {
                for k in map.keys() {
                    fields.insert(k.clone());
                }
            }
        }
        BodySpec::Text { raw } => scan_value(raw),
        BodySpec::UrlEncoded { fields: kvs } => {
            for kv in kvs {
                if kv.enabled {
                    fields.insert(kv.key.clone());
                    scan_value(&kv.value);
                }
            }
        }
        _ => {}
    }
    (fields, var_refs)
}

/// 步骤请求文本（用于 `{{var}}` 引用扫描）：引用式取接口当前配置，
/// 内联取步骤自身定义。
fn step_request_text(step: &ScenarioStep, endpoints: &HashMap<Uuid, &Endpoint>) -> String {
    if let Some(inline) = &step.request {
        let mut text = format!("{} {}", inline.method, inline.url);
        text.push_str(&serde_json::to_string(&inline.spec).unwrap_or_default());
        text
    } else if let Some(ep) = step.endpoint_id.and_then(|id| endpoints.get(&id)) {
        let mut text = format!("{} {}", ep.method.as_str(), ep.path);
        text.push_str(&serde_json::to_string(&ep.request).unwrap_or_default());
        if let Some(overrides) = &step.overrides {
            text.push_str(&overrides.to_string());
        }
        text
    } else {
        String::new()
    }
}

/// 步骤提取的变量名（extract 为 `Vec<ExtractSpec>` 形状 JSON 透传）。
fn step_extract_names(step: &ScenarioStep) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(list) = serde_json::from_value::<Vec<serde_json::Value>>(step.extract.clone()) {
        for item in list {
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                if !name.trim().is_empty() {
                    out.insert(name.trim().to_string());
                }
            }
        }
    }
    out
}

/// 文本中是否引用了 `{{var}}`。
fn text_contains_var(text: &str, var: &str) -> bool {
    VAR_REF_RE.captures_iter(text).any(|c| &c[1] == var)
}

/// 场景步骤数（给 coverage 复用）。
#[allow(dead_code)]
pub(crate) fn scenario_step_count(scenario: &Scenario) -> usize {
    scenario.steps.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{
        BodySpec, Endpoint, EndpointStatus, HttpMethod, KeyValue, RequestSpec, Scenario,
        ScenarioInlineRequest, ScenarioStep,
    };
    use miyo_storage::db::memory_pool;
    use miyo_storage::repository as repo;

    fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: value.into(),
            enabled: true,
            description: String::new(),
            field_type: Default::default(),
            required: true,
            example: String::new(),
        }
    }

    async fn seed_chain(db: &sqlx::SqlitePool) -> (Uuid, Vec<Uuid>) {
        let now = chrono::Utc::now();
        let project = repo::create_project(db, "依赖分析", "").await.unwrap();
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
        repo::save_endpoint(db, &create_order).await.unwrap();
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
                params: vec![kv("payToken", "{{payToken}}")],
                path_variables: vec![kv("orderNo", "{{orderNo}}")],
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        };
        repo::save_endpoint(db, &preview).await.unwrap();

        // 生产者证据：创建订单的响应示例含 orderNo / payToken。
        let example = miyo_core::model::ResponseExample {
            id: Uuid::new_v4(),
            endpoint_id: create_order.id,
            name: "成功".into(),
            status: 201,
            headers: Default::default(),
            body: r#"{"orderNo":"SO1","payToken":"pt1","status":"CREATED"}"#.into(),
            content_type: "application/json".into(),
            docs: Default::default(),
            created_at: now,
            updated_at: now,
        };
        repo::create_response_example(db, example.endpoint_id, &example)
            .await
            .unwrap();

        // 场景链：步骤 1 提取 orderNo/payToken，步骤 2 引用。
        let scenario = Scenario {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "下单链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![
                ScenarioStep {
                    endpoint_id: Some(create_order.id),
                    extract: serde_json::json!([
                        {"name": "orderNo", "from": "body", "path": "$.orderNo"},
                        {"name": "payToken", "from": "body", "path": "$.payToken"}
                    ]),
                    ..Default::default()
                },
                ScenarioStep {
                    endpoint_id: Some(preview.id),
                    ..Default::default()
                },
            ],
            script: None,
            setup: vec![],
            teardown: vec![],
            teardown_enabled: true,
            data_sets: vec![],
            created_at: now,
            updated_at: now,
        };
        repo::save_scenario(db, &scenario).await.unwrap();
        (project.id, vec![create_order.id, preview.id])
    }

    #[tokio::test]
    async fn detects_chain_and_field_edges() {
        let db = memory_pool().await.unwrap();
        let (project_id, endpoint_ids) = seed_chain(&db).await;
        let report = analyze_api_dependencies(&db, project_id).await.unwrap();

        // 场景链高置信度边：创建订单 → 预览支付。
        let high = report
            .edges
            .iter()
            .find(|e| e.confidence == Confidence::High)
            .expect("应有场景链边");
        assert_eq!(high.from, endpoint_ids[0]);
        assert_eq!(high.to, endpoint_ids[1]);
        assert!(high.via.contains(&"orderNo".to_string()));
        assert!(high.via.contains(&"payToken".to_string()));
        assert!(high.evidence.contains("下单链路"));

        // 响应字段匹配（同一条边已被高置信度覆盖，不重复）。
        assert_eq!(report.edges.len(), 1);

        // 生产者字段来自响应示例。
        let node = report
            .nodes
            .iter()
            .find(|n| n.endpoint_id == endpoint_ids[0])
            .unwrap();
        assert!(node.produces.contains(&"orderNo".to_string()));

        // 消费者 var_refs。
        let consumer = report
            .nodes
            .iter()
            .find(|n| n.endpoint_id == endpoint_ids[1])
            .unwrap();
        assert!(consumer.var_refs.contains(&"orderNo".to_string()));
    }

    #[tokio::test]
    async fn inline_step_chain_detected() {
        let db = memory_pool().await.unwrap();
        let now = chrono::Utc::now();
        let project = repo::create_project(&db, "内联链", "").await.unwrap();
        let a = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "A".into(),
            method: HttpMethod::POST,
            path: "/a".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec::default(),
            created_at: now,
            updated_at: now,
        };
        repo::save_endpoint(&db, &a).await.unwrap();
        let scenario = Scenario {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "内联链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![
                ScenarioStep {
                    endpoint_id: Some(a.id),
                    extract: serde_json::json!([{"name": "tok", "path": "$.token"}]),
                    ..Default::default()
                },
                ScenarioStep {
                    request: Some(ScenarioInlineRequest {
                        method: "GET".into(),
                        url: "{{base_url}}/b?auth={{tok}}".into(),
                        spec: RequestSpec::default(),
                    }),
                    ..Default::default()
                },
            ],
            script: None,
            setup: vec![],
            teardown: vec![],
            teardown_enabled: true,
            data_sets: vec![],
            created_at: now,
            updated_at: now,
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let report = analyze_api_dependencies(&db, project.id).await.unwrap();
        // 内联步骤无 endpoint_id，不产生边（引用式才有生产者 id），但 A 的
        // extract 记入 produces。
        let node = report.nodes.iter().find(|n| n.endpoint_id == a.id).unwrap();
        assert!(node.produces.contains(&"tok".to_string()));
    }
}
