//! 场景修复草稿（`propose_scenario_fix` 共享实现，不落库）。
//!
//! 规则策略（保守优先，能自动改的才改）：
//! - **extract 修复**（自动）：失败步骤 URL / blocked 原因仍含 `{{var}}` →
//!   在上游步骤中回溯响应包含该字段的生产者，补充提取规则；
//! - **断言失败**（仅提示）：expected vs actual 差异照实列出，不自动改
//!   断言期望值（把 expected 改成 actual 会掩盖真实缺陷）；
//! - **网络 / 环境类**（仅提示）：给出环境 / Mock 排查建议，不改场景；
//! - 用户 `notes` 原样附带给 AI，作为二次裁量的上下文。
//!
//! 输出 `changes`（字段级对照）与 `draftSteps`（应用了自动修复后的完整
//! 步骤 JSON，可直接传给 update_scenario）。

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use uuid::Uuid;

use miyo_core::model::{Endpoint, ScenarioStep};
use miyo_test::runner::EndpointResult;

use super::failure::FailureAnalysis;
use super::scenario_gen::response_field_paths;

static VAR_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").unwrap());

/// 单条修改建议。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixChange {
    pub step_index: usize,
    pub step_name: String,
    /// extract / assertion_review / manual。
    pub kind: String,
    /// 涉及字段（如 `extract` / `assertions[0].expected` / `url`）。
    pub field: String,
    pub before: String,
    pub after: String,
    pub reason: String,
    /// low（自动可改）/ medium（需确认）/ high（写操作相关，必须确认）。
    pub risk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixProposal {
    pub run_id: Uuid,
    pub scenario_id: Uuid,
    /// 失败分析（内嵌，免得 AI 再调一次 analyze_run_failure）。
    pub analysis: FailureAnalysis,
    pub changes: Vec<FixChange>,
    /// 应用自动修复后的完整步骤（仅当存在 extract 类修改时给出；
    /// 形状与 ScenarioStep 一致，可直接作为 update_scenario 的 steps）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_steps: Option<serde_json::Value>,
    /// 用户附加的修复要求（原样回传）。
    pub notes: Option<String>,
    pub disclaimer: String,
}

/// 修复草稿入参（参数收敛）。
pub struct FixContext<'a> {
    pub run_id: Uuid,
    pub scenario_id: Uuid,
    pub analysis: &'a FailureAnalysis,
    pub steps: &'a [ScenarioStep],
    pub results: &'a [EndpointResult],
    /// 场景引用式步骤涉及接口的当前配置（响应字段回溯用）。
    pub endpoint_by_id: &'a HashMap<Uuid, &'a Endpoint>,
    pub response_examples: &'a HashMap<Uuid, Vec<miyo_core::model::ResponseExample>>,
    pub notes: Option<String>,
}

/// 生成修复草稿。
pub fn propose_fix(ctx: FixContext<'_>) -> FixProposal {
    let FixContext {
        run_id,
        scenario_id,
        analysis,
        steps,
        results,
        endpoint_by_id,
        response_examples,
        notes,
    } = ctx;
    let mut changes: Vec<FixChange> = Vec::new();
    let mut draft_steps: Option<Vec<ScenarioStep>> = None;

    for failed in &analysis.failed_steps {
        let idx = failed.index;
        let Some(step) = steps.get(idx) else {
            continue;
        };
        let step_name = step.name.clone().unwrap_or_else(|| failed.name.clone());

        // ---- extract 修复：URL 仍含 {{var}} 且上游响应有该字段 ----
        let url = results
            .iter()
            .find(|r| r.step_index == Some(idx))
            .and_then(|r| r.url.clone())
            .unwrap_or_default();
        let missing: Vec<String> = VAR_REF_RE
            .captures_iter(&url)
            .map(|c| c[1].to_string())
            .collect();
        if failed.error_type == "VARIABLE_EXTRACT_FAILED" || !missing.is_empty() {
            for var in &missing {
                // 回溯：更早步骤的接口响应字段。
                let mut found: Option<(usize, String, String)> = None;
                for (producer_idx, producer_step) in steps.iter().enumerate().take(idx) {
                    let Some(pe) = producer_step
                        .endpoint_id
                        .and_then(|id| endpoint_by_id.get(&id).copied())
                    else {
                        continue;
                    };
                    if let Some((_, path)) =
                        response_field_paths(pe, response_examples, &HashMap::new())
                            .into_iter()
                            .find(|(f, _)| f == var)
                    {
                        found = Some((
                            producer_idx,
                            producer_step.name.clone().unwrap_or_default(),
                            path,
                        ));
                        break;
                    }
                }
                if let Some((producer_idx, producer_name, path)) = found {
                    let before = format!("步骤 {} 无 {} 提取规则", producer_idx + 1, var);
                    let after = format!("步骤 {} 新增提取 {var} ← {path}", producer_idx + 1);
                    changes.push(FixChange {
                        step_index: producer_idx,
                        step_name: producer_name,
                        kind: "extract".into(),
                        field: "extract".into(),
                        before,
                        after,
                        reason: format!(
                            "步骤 {}（{step_name}）因缺少变量 {var} 失败/被阻断",
                            idx + 1
                        ),
                        risk: "low".into(),
                    });
                    // 应用到草稿（去重：同一变量只补一次）。
                    let draft = draft_steps.get_or_insert_with(|| steps.to_vec());
                    let already = serde_json::to_string(&draft[producer_idx].extract)
                        .unwrap_or_default()
                        .contains(&format!("\"{var}\""));
                    if !already {
                        let rule =
                            serde_json::json!([{ "name": var, "from": "body", "path": path }]);
                        let mut list = draft[producer_idx]
                            .extract
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        list.extend(rule.as_array().cloned().unwrap_or_default());
                        draft[producer_idx].extract = serde_json::Value::Array(list);
                    }
                } else {
                    changes.push(FixChange {
                        step_index: idx,
                        step_name: step_name.clone(),
                        kind: "manual".into(),
                        field: "variables".into(),
                        before: format!("引用 {{{{{var}}}}} 无产出"),
                        after: "需用户配置：补提取规则 / 环境变量 / 前置登录步骤".into(),
                        reason: format!("步骤 {}（{step_name}）的请求仍含未解析占位符", idx + 1),
                        risk: "medium".into(),
                    });
                }
            }
        }

        // ---- 断言失败：仅列出差异，不自动改期望值 ----
        if failed.error_type == "ASSERTION_FAILED" {
            for a in &failed.assertion_failures {
                changes.push(FixChange {
                    step_index: idx,
                    step_name: step_name.clone(),
                    kind: "assertion_review".into(),
                    field: format!(
                        "assertions[{}]",
                        a.get("description").and_then(|d| d.as_str()).unwrap_or("?")
                    ),
                    before: format!(
                        "expected = {}",
                        a.get("expected").map(|v| v.to_string()).unwrap_or_default()
                    ),
                    after: format!(
                        "actual = {}",
                        a.get("actual").map(|v| v.to_string()).unwrap_or_default()
                    ),
                    reason: a
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or(
                            "断言期望与实际不符；请先确认是接口缺陷还是断言过时，再决定改哪边",
                        )
                        .to_string(),
                    risk: "medium".into(),
                });
            }
        }

        // ---- 网络 / 环境类：排查提示 ----
        if matches!(
            failed.error_type.as_str(),
            "NETWORK_UNREACHABLE" | "REQUEST_ERROR" | "URL_SCHEME_INVALID"
        ) {
            changes.push(FixChange {
                step_index: idx,
                step_name: step_name.clone(),
                kind: "manual".into(),
                field: "environment".into(),
                before: failed.error.clone().unwrap_or_default(),
                after: "确认目标服务可达（本地 Mock 是否启动）/ 环境 Base URL 是否正确后重跑"
                    .into(),
                reason: "网络类失败与场景配置通常无关".into(),
                risk: "low".into(),
            });
        }
    }

    let draft_steps_json = draft_steps.map(|s| serde_json::to_value(&s).unwrap_or_default());
    let has_auto = changes.iter().any(|c| c.kind == "extract");
    let disclaimer = if has_auto {
        "extract 类修改已应用到 draftSteps（确认后用 update_scenario 保存）；assertion_review / manual 项需人工判断。保存前建议先 validate_scenario 校验变量引用。"
    } else {
        "本次无自动可修项；请结合 changes 的 reason 与运行原始响应人工判断。"
    };

    FixProposal {
        run_id,
        scenario_id,
        analysis: analysis.clone(),
        changes,
        draft_steps: draft_steps_json,
        notes,
        disclaimer: disclaimer.into(),
    }
}

/// 数据库级编排：读运行 → 失败分析 → 场景与接口上下文 → 修复草稿
/// （server handler 与 Tauri 命令共用；草稿不落库）。
pub async fn propose_fix_for_run(
    db: &sqlx::SqlitePool,
    run_id: Uuid,
    notes: Option<String>,
) -> Result<FixProposal, miyo_core::AppError> {
    use miyo_storage::repository as repo;
    let analysis = super::failure::analyze_run(db, run_id).await?;
    let run = repo::get_scenario_run(db, run_id)
        .await?
        .ok_or_else(|| miyo_core::AppError::NotFound(format!("运行（{run_id}）不存在")))?;
    let scenario = repo::get_scenario(db, run.scenario_id).await?;

    let endpoint_ids: Vec<Uuid> = scenario
        .steps
        .iter()
        .filter_map(|s| s.endpoint_id)
        .collect();
    let endpoints = repo::list_endpoints(db, scenario.project_id).await?;
    let endpoint_by_id: HashMap<Uuid, &Endpoint> = endpoints
        .iter()
        .filter(|e| endpoint_ids.contains(&e.id))
        .map(|e| (e.id, e))
        .collect();
    let response_examples = repo::list_response_examples_by_endpoints(db, &endpoint_ids).await?;

    let mut results: Vec<EndpointResult> =
        serde_json::from_str(&run.results_json).unwrap_or_default();
    for r in &mut results {
        crate::redact::redact_endpoint_result(r);
    }

    let mut proposal = propose_fix(FixContext {
        run_id,
        scenario_id: scenario.id,
        analysis: &analysis,
        steps: &scenario.steps,
        results: &results,
        endpoint_by_id: &endpoint_by_id,
        response_examples: &response_examples,
        notes,
    });
    // draftSteps 保险脱敏（overrides 可能内嵌凭据模板值）。
    if let Some(steps) = &mut proposal.draft_steps {
        miyo_core::sensitive::redact_json_value(steps);
    }
    Ok(proposal)
}
