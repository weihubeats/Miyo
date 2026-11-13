//! 项目测试覆盖率分析（需求六的七个维度，规则判定，不调 LLM）。
//!
//! 七个维度：
//! 1. `no_cases` 没有测试用例的接口
//! 2. `no_success_run` 没有成功场景的接口（用例从未通过 且 未被成功场景覆盖）
//! 3. `no_negative` 没有异常测试的接口（负向 / 安全性类用例缺失）
//! 4. `no_boundary` 没有边界测试的参数（有必填参数但无边界值类用例）
//! 5. `no_response_assertion` 缺少响应 Schema 校验（无 jsonpath / body 类断言）
//! 6. `write_no_idempotency` 写接口缺少幂等性 / 重复请求测试
//! 7. `no_scenario` 未被任何业务流程（场景）覆盖
//!
//! 输出机器可读的 `issues` 键 + 中文摘要（与 analyze_run_failure 口径一致：
//! 中文面向 AI / 用户直接可读，前端 UI 另有 i18n 键映射）。

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{BodySpec, Endpoint, HttpMethod, TestCase};
use miyo_core::Result;
use miyo_storage::repository as repo;

/// 用例异常类目（负向 / 安全性）。
const NEGATIVE_CATEGORIES: &[&str] = &["负向", "安全性"];
/// 边界值类目。
const BOUNDARY_CATEGORY: &str = "边界值";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointCoverage {
    pub endpoint_id: Uuid,
    pub name: String,
    pub method: String,
    pub path: String,
    /// 命中的问题维度键（见模块文档七维）。
    pub issues: Vec<String>,
    pub case_count: usize,
    /// 最近用例执行状态（success / failed / untested；无用例为 none）。
    pub last_run_status: String,
    /// 是否被场景（引用式步骤）覆盖。
    pub scenario_covered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionSummary {
    /// 维度键（no_cases / no_success_run / no_negative / no_boundary /
    /// no_response_assertion / write_no_idempotency / no_scenario）。
    pub key: String,
    /// 中文说明（模板 + 命中数量）。
    pub label: String,
    pub endpoint_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    pub project_id: Uuid,
    /// 0-100：7 × 接口数 个检查点中未命中问题的比例。
    pub score: u8,
    pub endpoint_total: usize,
    pub dimensions: Vec<DimensionSummary>,
    pub endpoints: Vec<EndpointCoverage>,
    /// 中文总评（面向 AI / 用户）。
    pub summary: String,
}

/// 分析项目测试覆盖率。
pub async fn analyze_project_test_coverage(
    db: &SqlitePool,
    project_id: Uuid,
) -> Result<CoverageReport> {
    let endpoints = repo::list_endpoints(db, project_id).await?;
    let scenarios = repo::list_scenarios(db, project_id).await?;
    let endpoint_ids: Vec<Uuid> = endpoints.iter().map(|e| e.id).collect();
    let flat_cases: Vec<TestCase> = repo::list_test_cases_by_endpoints(db, &endpoint_ids).await?;
    let mut cases_by_endpoint: HashMap<Uuid, Vec<TestCase>> = HashMap::new();
    for case in flat_cases {
        cases_by_endpoint
            .entry(case.request_id)
            .or_default()
            .push(case);
    }
    let run_summaries = repo::latest_run_summaries_for_project(db, project_id).await?;

    // 场景覆盖：引用式步骤的 endpoint_id + 内联步骤 method+path 匹配。
    let mut scenario_covered: HashSet<Uuid> = HashSet::new();
    for sc in &scenarios {
        for step in &sc.steps {
            if let Some(ep) = step.endpoint_id {
                scenario_covered.insert(ep);
            }
        }
    }

    // 场景最近一次运行成功的场景 → 覆盖的接口视为「有成功场景」。
    let mut successful_scenario_endpoints: HashSet<Uuid> = HashSet::new();
    for sc in &scenarios {
        let ok = run_summaries
            .iter()
            .any(|s| s.scenario_id == sc.id.to_string() && s.fail_count == 0 && !s.cancelled);
        if !ok {
            continue;
        }
        for step in &sc.steps {
            if let Some(ep) = step.endpoint_id {
                successful_scenario_endpoints.insert(ep);
            }
        }
    }

    let mut endpoint_rows = Vec::new();
    let mut dim_hits: HashMap<&'static str, Vec<Uuid>> = HashMap::new();
    let mut total_checks = 0usize;
    let mut clean_checks = 0usize;

    for ep in &endpoints {
        let cases = cases_by_endpoint
            .get(&ep.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut issues: Vec<String> = Vec::new();

        let has_negative = cases
            .iter()
            .any(|c| NEGATIVE_CATEGORIES.contains(&c.category.as_str()));
        let has_boundary = cases.iter().any(|c| c.category == BOUNDARY_CATEGORY);
        let has_response_assertion = cases.iter().any(case_has_response_assertion);
        let last_success = cases
            .iter()
            .any(|c| matches!(c.last_run_status, miyo_core::model::TestCaseStatus::Success));
        let write = is_write_method(ep.method);

        // 1 无用例
        total_checks += 1;
        if cases.is_empty() {
            issues.push("no_cases".into());
            dim_hits.entry("no_cases").or_default().push(ep.id);
        } else {
            clean_checks += 1;
        }
        // 2 无成功运行
        total_checks += 1;
        if !last_success && !successful_scenario_endpoints.contains(&ep.id) {
            issues.push("no_success_run".into());
            dim_hits.entry("no_success_run").or_default().push(ep.id);
        } else {
            clean_checks += 1;
        }
        // 3 无异常测试
        total_checks += 1;
        if !has_negative {
            issues.push("no_negative".into());
            dim_hits.entry("no_negative").or_default().push(ep.id);
        } else {
            clean_checks += 1;
        }
        // 4 无边界测试（仅当存在必填参数/必填 body 字段时有意义）
        let required_count = required_field_count(ep);
        total_checks += 1;
        if required_count > 0 && !has_boundary {
            issues.push("no_boundary".into());
            dim_hits.entry("no_boundary").or_default().push(ep.id);
        } else {
            clean_checks += 1;
        }
        // 5 缺响应 Schema 校验
        total_checks += 1;
        if !has_response_assertion {
            issues.push("no_response_assertion".into());
            dim_hits
                .entry("no_response_assertion")
                .or_default()
                .push(ep.id);
        } else {
            clean_checks += 1;
        }
        // 6 写接口缺幂等 / 重复请求测试
        if write {
            total_checks += 1;
            let has_idem = cases.iter().any(|c| {
                c.name.contains("幂等") || c.name.contains("重复") || case_has_idempotency_header(c)
            });
            if !has_idem {
                issues.push("write_no_idempotency".into());
                dim_hits
                    .entry("write_no_idempotency")
                    .or_default()
                    .push(ep.id);
            } else {
                clean_checks += 1;
            }
        }
        // 7 未被场景覆盖
        total_checks += 1;
        if !scenario_covered.contains(&ep.id) {
            issues.push("no_scenario".into());
            dim_hits.entry("no_scenario").or_default().push(ep.id);
        } else {
            clean_checks += 1;
        }

        let last_run_status = cases
            .iter()
            .filter_map(|c| match c.last_run_status {
                miyo_core::model::TestCaseStatus::Success => Some("success"),
                miyo_core::model::TestCaseStatus::Failed => Some("failed"),
                miyo_core::model::TestCaseStatus::Untested => None,
            })
            .next_back()
            .unwrap_or(if cases.is_empty() { "none" } else { "untested" })
            .to_string();

        endpoint_rows.push(EndpointCoverage {
            endpoint_id: ep.id,
            name: ep.name.clone(),
            method: ep.method.as_str().to_string(),
            path: ep.path.clone(),
            issues,
            case_count: cases.len(),
            last_run_status,
            scenario_covered: scenario_covered.contains(&ep.id),
        });
    }

    endpoint_rows.sort_by(|a, b| a.name.cmp(&b.name));
    // 空项目（0 检查点）视为满分。
    let score = u8::try_from(
        (clean_checks * 100)
            .checked_div(total_checks)
            .unwrap_or(100),
    )
    .unwrap_or(100);

    const DIM_LABELS: &[(&str, &str)] = &[
        ("no_cases", "没有测试用例"),
        ("no_success_run", "没有成功执行记录"),
        ("no_negative", "没有异常（负向/安全性）测试"),
        ("no_boundary", "有必填参数但没有边界值测试"),
        ("no_response_assertion", "缺少响应体断言校验"),
        ("write_no_idempotency", "写接口缺少幂等/重复请求测试"),
        ("no_scenario", "未被任何业务流程（场景）覆盖"),
    ];
    let dimensions: Vec<DimensionSummary> = DIM_LABELS
        .iter()
        .map(|(key, label)| DimensionSummary {
            key: key.to_string(),
            label: format!(
                "{label}（{} 个接口）",
                dim_hits.get(key).map(Vec::len).unwrap_or(0)
            ),
            endpoint_count: dim_hits.get(key).map(Vec::len).unwrap_or(0),
        })
        .collect();

    let summary = format!(
        "覆盖率得分 {score}/100：{} 个接口共 {} 项检查，通过 {} 项。最薄弱维度：{}。",
        endpoints.len(),
        total_checks,
        clean_checks,
        dimensions
            .iter()
            .filter(|d| d.endpoint_count > 0)
            .max_by_key(|d| d.endpoint_count)
            .map(|d| d.label.clone())
            .unwrap_or_else(|| "无".into())
    );

    Ok(CoverageReport {
        project_id,
        score,
        endpoint_total: endpoints.len(),
        dimensions,
        endpoints: endpoint_rows,
        summary,
    })
}

fn is_write_method(method: HttpMethod) -> bool {
    matches!(
        method,
        HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH | HttpMethod::DELETE
    )
}

/// 用例是否带响应体类断言（jsonpath / body / javascript 检查响应结构的都算）。
fn case_has_response_assertion(case: &TestCase) -> bool {
    serde_json::from_value::<Vec<serde_json::Value>>(case.assertions.clone())
        .map(|list| {
            list.iter().any(|a| {
                matches!(
                    a.get("type").and_then(|t| t.as_str()),
                    Some("jsonpath") | Some("body") | Some("graphql_errors")
                )
            })
        })
        .unwrap_or(false)
}

/// 用例是否显式带幂等键请求头。
fn case_has_idempotency_header(case: &TestCase) -> bool {
    case.headers
        .iter()
        .any(|h| h.key.eq_ignore_ascii_case("Idempotency-Key") && h.enabled)
}

/// 接口的必填参数计数（query / 路径变量 / JSON body 顶层必填键）。
fn required_field_count(ep: &Endpoint) -> usize {
    let spec = &ep.request;
    let mut count = spec
        .params
        .iter()
        .chain(&spec.path_variables)
        .filter(|kv| kv.enabled && kv.required)
        .count();
    if let BodySpec::Json { raw } = &spec.body {
        // body_docs 未标注时按「存在即必填」估计（演示链路等手工配置的保守口径）。
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw) {
            count += map.len();
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{
        BodySpec, EndpointStatus, KeyValue, RequestSpec, Scenario, ScenarioStep,
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

    async fn seed(db: &SqlitePool) -> (Uuid, Uuid, Uuid) {
        let now = chrono::Utc::now();
        let project = repo::create_project(db, "覆盖率", "").await.unwrap();
        let covered = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "有测试的接口".into(),
            method: HttpMethod::GET,
            path: "/ok".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec {
                params: vec![kv("page", "1")],
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        };
        let uncovered_write = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "裸写接口".into(),
            method: HttpMethod::POST,
            path: "/orders".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 1,
            request: RequestSpec {
                body: BodySpec::Json {
                    raw: r#"{"sku":"S1"}"#.into(),
                },
                ..Default::default()
            },
            created_at: now,
            updated_at: now,
        };
        repo::save_endpoint(db, &covered).await.unwrap();
        repo::save_endpoint(db, &uncovered_write).await.unwrap();
        // 有测试的接口：正向 + 边界值用例（带 jsonpath 断言、成功过）。
        let case = TestCase {
            id: Uuid::new_v4(),
            request_id: covered.id,
            name: "正向查询".into(),
            category: "正向".into(),
            folder: String::new(),
            method: HttpMethod::GET,
            url_path: "/ok".into(),
            path_variables: vec![],
            params: vec![kv("page", "1")],
            headers: vec![],
            body_type: "none".into(),
            body_content: String::new(),
            assertions: serde_json::json!([{ "type": "jsonpath", "path": "$.ok", "op": "eq", "expected": true }]),
            extractors: serde_json::Value::Null,
            auth: None,
            timeout_ms: None,
            disable_cookies: false,
            sort_order: 0,
            last_run_status: miyo_core::model::TestCaseStatus::Success,
            last_run_at: None,
            last_run_ok: Some(1),
            last_run_fail: Some(0),
            created_at: now,
        };
        repo::create_test_case(db, &case).await.unwrap();
        // 场景覆盖 covered 接口。
        let scenario = Scenario {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "查询链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: Some(covered.id),
                ..Default::default()
            }],
            script: None,
            setup: vec![],
            teardown: vec![],
            teardown_enabled: true,
            data_sets: vec![],
            created_at: now,
            updated_at: now,
        };
        repo::save_scenario(db, &scenario).await.unwrap();
        (project.id, covered.id, uncovered_write.id)
    }

    #[tokio::test]
    async fn coverage_flags_missing_dimensions() {
        let db = memory_pool().await.unwrap();
        let (project_id, covered, bare) = seed(&db).await;
        let report = analyze_project_test_coverage(&db, project_id)
            .await
            .unwrap();

        let good = report
            .endpoints
            .iter()
            .find(|e| e.endpoint_id == covered)
            .unwrap();
        // 正向+边界+响应断言+成功+场景覆盖，但无负向用例。
        assert!(good.issues.contains(&"no_negative".to_string()));
        assert!(!good.issues.contains(&"no_cases".to_string()));
        assert!(!good.issues.contains(&"no_success_run".to_string()));
        assert!(good.scenario_covered);

        let bare_row = report
            .endpoints
            .iter()
            .find(|e| e.endpoint_id == bare)
            .unwrap();
        for key in [
            "no_cases",
            "no_success_run",
            "no_negative",
            "no_boundary",
            "no_response_assertion",
            "write_no_idempotency",
            "no_scenario",
        ] {
            assert!(
                bare_row.issues.contains(&key.to_string()),
                "裸写接口应命中 {key}"
            );
        }

        assert_eq!(report.dimensions.len(), 7);
        let no_cases = report
            .dimensions
            .iter()
            .find(|d| d.key == "no_cases")
            .unwrap();
        assert_eq!(no_cases.endpoint_count, 1);
        assert!(report.score < 100);
        assert!(report.summary.contains("覆盖率得分"));
    }

    #[tokio::test]
    async fn empty_project_scores_full() {
        let db = memory_pool().await.unwrap();
        let project = repo::create_project(&db, "空项目", "").await.unwrap();
        let report = analyze_project_test_coverage(&db, project.id)
            .await
            .unwrap();
        assert_eq!(report.score, 100);
        assert_eq!(report.endpoints.len(), 0);
    }
}
