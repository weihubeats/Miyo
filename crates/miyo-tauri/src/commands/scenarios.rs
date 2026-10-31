//! 集成测试场景 Command：项目级接口链路的 CRUD 与串行运行。
//!
//! 步骤为引用式（endpoint_id + 步骤级提取/断言），运行时用「环境静态变量 +
//! runtime 快照（前步提取值优先）」渲染每步的 URL / 规格模板——前步提取的
//! 变量即成为后续请求的入参（详见 miyo-test `run_scenario`）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use miyo_core::model::{Endpoint, Scenario, ScenarioRun};
use miyo_core::VariableMap;
use miyo_storage::repository as repo;
use miyo_test::runner::{ScenarioOptions, ScenarioStepInput};

use crate::commands::request::{cap_ipc_body, resolve_timeout_ms};
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

fn default_step_enabled() -> bool {
    true
}

fn validate_name(name: &str) -> CommandResult<()> {
    if name.trim().is_empty() {
        return Err(CommandError::validation("场景名称不能为空"));
    }
    Ok(())
}

/// 列出项目的全部场景（编排序）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_scenarios(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<Scenario>> {
    Ok(repo::list_scenarios(&state.db, project_id).await?)
}

/// 保存场景（整体覆盖写；新建时 created_at 由前端给定，更新时间后端刷新）。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_scenario(
    state: State<'_, AppState>,
    scenario: Scenario,
) -> CommandResult<Scenario> {
    validate_name(&scenario.name)?;
    // 归属校验：项目必须存在（防跨项目数据漂移）。
    repo::get_project(&state.db, scenario.project_id).await?;
    let mut scenario = scenario;
    scenario.name = scenario.name.trim().to_string();
    scenario.description = scenario.description.trim().to_string();
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario).await?;
    Ok(scenario)
}

/// 删除场景。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_scenario(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    Ok(repo::delete_scenario(&state.db, id).await?)
}

/// 场景运行入参步骤（前端物化：endpoint 用当前配置，url 保留 {{}} 模板）。
#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioStepArgs {
    pub endpoint: Endpoint,
    /// 渲染前 URL 模板（含 {{base_url}} / {{var}} 等未解析占位）。
    pub url: String,
    #[serde(default = "default_step_enabled")]
    pub enabled: bool,
    /// 提取规则（ExtractSpec 数组形状，JSON 透传）。
    #[serde(default)]
    pub extract: serde_json::Value,
    /// 断言规则（AssertionSpec 数组形状，JSON 透传）。
    #[serde(default)]
    pub assertions: serde_json::Value,
}

/// 场景运行入参。
#[derive(Debug, Deserialize)]
pub struct RunScenarioArgs {
    pub steps: Vec<ScenarioStepArgs>,
    /// 本次运行使用的环境（缺省用当前激活环境）。
    pub environment_id: Option<Uuid>,
    /// 失败即停：某步请求失败或断言不过时不再执行后续步骤。
    #[serde(default)]
    pub stop_on_failure: bool,
    /// 取消标识（前端生成；`cancel_scenario_run` 触发中止）。
    #[serde(default)]
    pub run_id: Option<String>,
    /// 归属场景（提供则运行完成后自动落运行历史；旧前端 / 裸步骤运行缺省不落）。
    #[serde(default)]
    pub scenario_id: Option<Uuid>,
    /// 初始变量（重跑失败步骤时传前次运行的 runtime 快照，重建链路上下文；
    /// 缺省用环境 / 项目 / 全局变量解析）。键值对覆盖默认解析结果。
    #[serde(default)]
    pub initial_vars: Option<HashMap<String, String>>,
}

/// 运行场景：严格按步骤顺序串行执行，返回每步结果与最终变量池。
#[tauri::command(rename_all = "camelCase")]
pub async fn run_scenario(
    app: AppHandle,
    state: State<'_, AppState>,
    args: RunScenarioArgs,
) -> CommandResult<miyo_test::runner::ScenarioRunResult> {
    if args.steps.is_empty() {
        return Err(CommandError::validation("场景没有可执行的步骤"));
    }

    // 变量（环境 > 项目 > 全局）与超时同波次解析；全局参数按步补缺。
    let (vars, timeout_ms) = tokio::join!(state.variables_for(args.environment_id), async {
        // 超时兜底用第一个步骤的规格（接口级 timeout_ms 优先）。
        let first = args.steps.first().map(|s| s.endpoint.request.clone());
        match first {
            Some(spec) => resolve_timeout_ms(&spec, &state).await,
            None => Ok(miyo_http::client::DEFAULT_TIMEOUT_MS),
        }
    },);
    let vars: VariableMap = vars?;
    let timeout_ms = timeout_ms?;
    // 重跑失败步骤：以前次运行的 runtime 快照为底（已含环境静态变量与提取值），
    // 全局参数按步补缺仍走同一物化路径。
    let vars: VariableMap = match args.initial_vars {
        Some(initial) if !initial.is_empty() => initial,
        _ => vars,
    };

    let settings = state.global_settings().await?;
    let global_params = settings.global_params.clone();

    // 物化步骤：URL / 规格保留模板；步骤级提取/断言解析为 TestSpec 形状。
    let mut steps = Vec::with_capacity(args.steps.len());
    for (idx, step) in args.steps.into_iter().enumerate() {
        if !step.enabled {
            continue;
        }
        let mut spec = step.endpoint.request.clone();
        crate::commands::request::apply_global_params(&mut spec, &global_params, &vars);
        let config = miyo_test::config::TestSpec::from_request_value(Some(&serde_json::json!({
            "extract": step.extract,
            "assertions": step.assertions,
        })))
        .map_err(|e| {
            CommandError::validation(format!("第 {} 步的提取/断言配置无效：{e}", idx + 1))
        })?;
        steps.push(ScenarioStepInput {
            endpoint: step.endpoint,
            url_template: step.url,
            spec_template: spec,
            extract: config.extract,
            assertions: config.assertions,
        });
    }
    if steps.is_empty() {
        return Err(CommandError::validation("场景没有启用中的步骤"));
    }

    // 取消令牌注册（「停止运行」→ 中止）。
    let cancel = args.run_id.as_ref().map(|id| {
        let token = tokio_util::sync::CancellationToken::new();
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), token.clone());
        (id.clone(), token)
    });

    // 进度事件（带 run_id：同前端多编辑器实例可过滤只认自己的那次运行）。
    let total = steps.len();
    let total_enabled = total;
    let event_run_id = args.run_id.clone().unwrap_or_default();
    let progress = move |done: usize, _total: usize| {
        let _ = app.emit(
            "miyo:scenario-progress",
            serde_json::json!({ "runId": event_run_id, "done": done, "total": total }),
        );
    };
    let options = ScenarioOptions {
        timeout_ms: Some(timeout_ms),
        cancel: cancel.as_ref().map(|(_, t)| t.clone()),
        stop_on_failure: args.stop_on_failure,
    };
    let mut result = miyo_test::runner::run_scenario(steps, &vars, &options, Some(&progress)).await;

    // 无论完成 / 取消，都从注册表移除，避免泄漏。
    if let Some((id, _)) = &cancel {
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    // 正文 IPC 预览口径与单请求流程一致：超 2MB 截断，全文入缓存
    // （键 `{run_id}:{结果位次}`），前端经 get_response_body 按需取回；
    // 无 run_id 的调用不缓存，前端对 body_omitted 降级为预览提示。
    let run_key = args.run_id.clone().unwrap_or_default();
    {
        let mut cache = state
            .response_bodies
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for (i, step) in result.results.iter_mut().enumerate() {
            let Some(resp) = step.response.as_mut() else {
                continue;
            };
            let (preview, omitted) = cap_ipc_body(&resp.body);
            if omitted {
                if !run_key.is_empty() {
                    cache.insert(format!("{run_key}:{i}"), resp.body.clone());
                }
                resp.body = preview;
                resp.body_omitted = true;
            }
        }
    }

    // 运行历史落库（fail-open：结果照常返回，历史失败仅告警）。
    if let Some(scenario_id) = args.scenario_id {
        if let Err(e) = persist_scenario_run(
            &state,
            scenario_id,
            args.environment_id,
            args.run_id.as_deref(),
            total_enabled,
            &result,
        )
        .await
        {
            tracing::warn!("[run_scenario] 保存运行历史失败：{e}");
        }
    }

    Ok(result)
}

/// 场景运行历史条目（IPC 摘要；results_json / variables_json 不下发）。
#[derive(Debug, Serialize)]
pub struct ScenarioRunSummary {
    pub id: String,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: Option<f64>,
    pub total_steps: i64,
    pub ok_count: i64,
    pub fail_count: i64,
    pub skipped_count: i64,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
}

/// 场景运行历史详情：摘要 + 回读的步骤结果与变量池。
#[derive(Debug, Serialize)]
pub struct ScenarioRunDetail {
    #[serde(flatten)]
    pub summary: ScenarioRunSummary,
    pub results: Vec<miyo_test::runner::EndpointResult>,
    pub variables: HashMap<String, String>,
}

/// 运行完成后落历史：剥离 response 正文（体积），统计摘要，保留最近 N 条。
async fn persist_scenario_run(
    state: &State<'_, AppState>,
    scenario_id: Uuid,
    environment_id: Option<Uuid>,
    run_id: Option<&str>,
    enabled_steps: usize,
    result: &miyo_test::runner::ScenarioRunResult,
) -> CommandResult<()> {
    // 结果序列化前剥离 response（正文预览最大 2MB，历史不需要；
    // 状态码 / 断言明细 / 提取值 / 实际 URL 均在结果字段里保留）。
    let mut results = result.results.clone();
    for r in &mut results {
        r.response = None;
    }
    let executed = results.len() as i64;
    let ok_count = results.iter().filter(|r| r.ok).count() as i64;
    let record = ScenarioRun {
        // id 即前端生成的 run_id（同一次重试覆盖旧行，不膨胀）。
        id: run_id
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4),
        scenario_id,
        environment_id,
        finished_at: chrono::Utc::now(),
        duration_ms: Some(results.iter().filter_map(|r| r.duration_ms).sum()),
        total_steps: enabled_steps as i64,
        ok_count,
        fail_count: executed - ok_count,
        skipped_count: (enabled_steps as i64 - executed).max(0),
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        results_json: serde_json::to_string(&results)
            .map_err(|e| CommandError::validation(format!("运行结果序列化失败：{e}")))?,
        variables_json: serde_json::to_string(&result.variables)
            .map_err(|e| CommandError::validation(format!("变量池序列化失败：{e}")))?,
    };
    repo::save_scenario_run(&state.db, &record).await?;
    repo::prune_scenario_runs(&state.db, scenario_id, repo::SCENARIO_RUN_KEEP).await?;
    Ok(())
}

/// 列出场景最近的运行历史（新→旧，默认 20 条）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_scenario_runs(
    state: State<'_, AppState>,
    scenario_id: Uuid,
    limit: Option<i64>,
) -> CommandResult<Vec<ScenarioRunSummary>> {
    let runs = repo::list_scenario_runs(&state.db, scenario_id, limit.unwrap_or(20)).await?;
    Ok(runs
        .into_iter()
        .map(|r| ScenarioRunSummary {
            id: r.id.to_string(),
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            total_steps: r.total_steps,
            ok_count: r.ok_count,
            fail_count: r.fail_count,
            skipped_count: r.skipped_count,
            cancelled: r.cancelled,
            stopped_on_failure: r.stopped_on_failure,
        })
        .collect())
}

/// 取单条运行历史详情（回看；不存在返回 `null`）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_scenario_run(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Option<ScenarioRunDetail>> {
    let uuid = Uuid::parse_str(&id)
        .map_err(|_| CommandError::validation(format!("运行历史 id 无效：{id}")))?;
    let Some(run) = repo::get_scenario_run(&state.db, uuid).await? else {
        return Ok(None);
    };
    let results: Vec<miyo_test::runner::EndpointResult> =
        serde_json::from_str(&run.results_json)
            .map_err(|e| CommandError::validation(format!("运行历史结果解析失败：{e}")))?;
    let variables: HashMap<String, String> = serde_json::from_str(&run.variables_json)
        .map_err(|e| CommandError::validation(format!("运行历史变量解析失败：{e}")))?;
    Ok(Some(ScenarioRunDetail {
        summary: ScenarioRunSummary {
            id: run.id.to_string(),
            finished_at: run.finished_at,
            duration_ms: run.duration_ms,
            total_steps: run.total_steps,
            ok_count: run.ok_count,
            fail_count: run.fail_count,
            skipped_count: run.skipped_count,
            cancelled: run.cancelled,
            stopped_on_failure: run.stopped_on_failure,
        },
        results,
        variables,
    }))
}

/// 取消一个在途场景运行（run_id 不存在或已完成时返回 `false`）。
#[tauri::command(rename_all = "camelCase")]
pub fn cancel_scenario_run(state: State<'_, AppState>, run_id: String) -> CommandResult<bool> {
    let token = state
        .run_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&run_id);
    if let Some(token) = token {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}
