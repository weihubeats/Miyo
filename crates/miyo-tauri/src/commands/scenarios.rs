//! 集成测试场景 Command：项目级接口链路的 CRUD 与串行运行。
//!
//! 步骤为引用式（endpoint_id + 步骤级提取/断言），运行时用「环境静态变量 +
//! runtime 快照（前步提取值优先）」渲染每步的 URL / 规格模板——前步提取的
//! 变量即成为后续请求的入参（详见 miyo-test `run_scenario`）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use miyo_core::model::{Endpoint, HttpMethod, Scenario, ScenarioRun};
use miyo_core::VariableMap;
use miyo_storage::repository as repo;
use miyo_sync::redact::{is_sensitive_key, PLACEHOLDER};
use miyo_test::runner::{ScenarioOptions, ScenarioProgress, ScenarioStepInput};

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
    scenario.category = scenario.category.trim().to_string();
    // 内联步骤校验：URL 非空、方法合法（引用式步骤跳过——配置在接口上）。
    for (idx, step) in scenario.steps.iter().enumerate() {
        let Some(req) = &step.request else {
            continue;
        };
        if req.url.trim().is_empty() {
            return Err(CommandError::validation(format!(
                "第 {} 步的内联请求 URL 不能为空",
                idx + 1
            )));
        }
        if req.method.parse::<HttpMethod>().is_err() {
            return Err(CommandError::validation(format!(
                "第 {} 步的 HTTP 方法无效：{}",
                idx + 1,
                req.method
            )));
        }
    }
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
    /// 执行条件（StepCondition 形状，JSON 透传；None = 无条件）。
    #[serde(default)]
    pub condition: Option<serde_json::Value>,
    /// 循环配置（StepLoop 形状，JSON 透传；None = 单次）。
    #[serde(default)]
    pub r#loop: Option<serde_json::Value>,
    /// 执行前等待毫秒。
    #[serde(default)]
    pub wait_ms: Option<u64>,
    /// 与相邻 parallel 步骤成批并发。
    #[serde(default)]
    pub parallel: bool,
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
    // 禁用步骤照传（runner 落 skipped 行，结果面板完整呈现「为什么没跑」）。
    let mut steps = Vec::with_capacity(args.steps.len());
    for (idx, step) in args.steps.into_iter().enumerate() {
        let mut spec = step.endpoint.request.clone();
        crate::commands::request::apply_global_params(&mut spec, &global_params, &vars);
        let config = miyo_test::config::TestSpec::from_request_value(Some(&serde_json::json!({
            "extract": step.extract,
            "assertions": step.assertions,
        })))
        .map_err(|e| {
            CommandError::validation(format!("第 {} 步的提取/断言配置无效：{e}", idx + 1))
        })?;
        let condition = step
            .condition
            .as_ref()
            .map(|v| serde_json::from_value::<miyo_test::config::StepCondition>(v.clone()))
            .transpose()
            .map_err(|e| {
                CommandError::validation(format!("第 {} 步的执行条件无效：{e}", idx + 1))
            })?;
        let r#loop = step
            .r#loop
            .as_ref()
            .map(|v| serde_json::from_value::<miyo_test::config::StepLoop>(v.clone()))
            .transpose()
            .map_err(|e| {
                CommandError::validation(format!("第 {} 步的循环配置无效：{e}", idx + 1))
            })?;
        steps.push(ScenarioStepInput {
            endpoint: step.endpoint,
            url_template: step.url,
            spec_template: spec,
            extract: config.extract,
            assertions: config.assertions,
            enabled: step.enabled,
            condition,
            r#loop,
            wait_ms: step
                .wait_ms
                .map(|w| w.min(miyo_test::config::StepLoop::MAX_INTERVAL_MS)),
            parallel: step.parallel,
        });
    }
    if steps.iter().all(|s| !s.enabled) {
        return Err(CommandError::validation("场景没有启用中的步骤"));
    }

    // 执行并发限制：同一场景同时只允许一个运行（不同场景互不影响）。
    // 运行上下文本就按 run 隔离（独立 runtime），这里挡的是重复触发。
    if let Some(scenario_id) = args.scenario_id {
        let mut running = state
            .scenario_running
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if running.contains_key(&scenario_id) {
            return Err(CommandError::validation(
                "该场景正在运行中，请等待当前运行结束",
            ));
        }
        if let Some(run_id) = &args.run_id {
            running.insert(scenario_id, run_id.clone());
        }
    }
    let started_at = chrono::Utc::now();
    // 当时配置快照（材料化后的步骤，含全局参数注入结果）：历史 meta 落库，
    // 「从历史重跑」基于这份快照而非场景当前配置。
    let steps_snapshot_json = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string());

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
    //
    // 每个步骤发两次：进入本步（result 缺省 + url，前端显示「执行中」与
    // 实际请求地址）、本步结束（result 带整段结果，前端立刻就能展开响应 /
    // 断言 / 提取值——不必等整轮跑完）。正文与最终结果同口径做 IPC 预览截断，
    // 避免逐步回传大响应；全文仍由末尾的 get_response_body 缓存路径兜底。
    let total = steps.len();
    let total_enabled = total;
    let event_run_id = args.run_id.clone().unwrap_or_default();
    let progress = move |p: ScenarioProgress<'_>| {
        let step = p.step.map(|r| {
            let mut r = r.clone();
            if let Some(resp) = r.response.as_mut() {
                let (preview, omitted) = cap_ipc_body(&resp.body);
                if omitted {
                    resp.body = preview;
                    resp.body_omitted = true;
                }
            }
            r
        });
        let _ = app.emit(
            "miyo:scenario-progress",
            serde_json::json!({
                "runId": event_run_id,
                "done": p.done,
                "total": p.total,
                "url": p.url,
                "result": step,
            }),
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
    // 释放场景级并发守卫。
    if let Some(scenario_id) = args.scenario_id {
        state
            .scenario_running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&scenario_id);
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
            started_at,
            args.stop_on_failure,
            &steps_snapshot_json,
            &vars,
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

impl ScenarioRunSummary {
    fn from_record(r: miyo_core::model::ScenarioRun) -> Self {
        ScenarioRunSummary {
            id: r.id.to_string(),
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            total_steps: r.total_steps,
            ok_count: r.ok_count,
            fail_count: r.fail_count,
            skipped_count: r.skipped_count,
            cancelled: r.cancelled,
            stopped_on_failure: r.stopped_on_failure,
        }
    }
}

/// 场景库列表项：场景 + 最近一次运行摘要（单次 IPC 供列表页渲染，
/// 免去前端逐场景 N+1 查询——与 dashboard list_project_stats 同思路）。
#[derive(Debug, Serialize)]
pub struct ScenarioLibraryItem {
    #[serde(flatten)]
    pub scenario: Scenario,
    pub last_run: Option<ScenarioRunSummary>,
}

/// 列出项目的场景库条目（编排序 → 名称序；每项带最近一次运行摘要）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_scenario_summaries(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<ScenarioLibraryItem>> {
    let scenarios = repo::list_scenarios(&state.db, project_id).await?;
    // 一次 SQL 批量取每场景最近一次运行摘要（替代逐场景 N+1 全量 JSON 查询）。
    let latest: HashMap<Uuid, repo::ScenarioRunSummaryRow> =
        repo::latest_run_summaries_for_project(&state.db, project_id)
            .await?
            .into_iter()
            .filter_map(|r| {
                let sid = Uuid::parse_str(&r.scenario_id).ok()?;
                Some((sid, r))
            })
            .collect();
    let items = scenarios
        .into_iter()
        .map(|scenario| {
            let last_run = latest.get(&scenario.id).map(|r| ScenarioRunSummary {
                id: r.id.clone(),
                finished_at: r.finished_at,
                duration_ms: r.duration_ms,
                total_steps: r.total_steps,
                ok_count: r.ok_count,
                fail_count: r.fail_count,
                skipped_count: r.skipped_count,
                cancelled: r.cancelled,
                stopped_on_failure: r.stopped_on_failure,
            });
            ScenarioLibraryItem { scenario, last_run }
        })
        .collect();
    Ok(items)
}

/// 场景运行历史详情：摘要 + 回读的步骤结果与变量池 + 运行元信息。
#[derive(Debug, Serialize)]
pub struct ScenarioRunDetail {
    #[serde(flatten)]
    pub summary: ScenarioRunSummary,
    pub results: Vec<miyo_test::runner::EndpointResult>,
    pub variables: HashMap<String, String>,
    /// 运行元信息（meta_json 解析：startedAt / 当时配置快照 steps / 初始变量
    /// initialVars（脱敏）/ stopOnFailure / warnings）；旧行为 None。
    pub meta: Option<serde_json::Value>,
}

/// 历史中每步响应正文的保留上限（超限截断并标记 `history_truncated`）。
/// 与请求快照体预览（SNAPSHOT_BODY_BYTES = 64KB）同口径。
const HISTORY_BODY_BYTES: usize = 64 * 1024;

/// 请求快照脱敏：headers / cookies 中敏感键值替换为占位符，URL query 同样
/// 处理（凭据不落历史库；与请求历史的 redact_history_spec 同口径）。
pub(crate) fn redact_request_snapshot(snapshot: &mut miyo_http::client::RequestSnapshot) {
    snapshot.headers = snapshot
        .headers
        .iter()
        .map(|(k, v)| {
            if is_sensitive_key(k) && !v.is_empty() {
                (k.clone(), PLACEHOLDER.to_string())
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    snapshot.cookies = snapshot
        .cookies
        .iter()
        .map(|(k, v)| {
            if is_sensitive_key(k) && !v.is_empty() {
                (k.clone(), PLACEHOLDER.to_string())
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    snapshot.url = crate::commands::request::redact_url_query(&snapshot.url);
}

/// 变量池脱敏（单源在 [`miyo_sync::redact::redact_vars`]；历史 / meta 中的
/// 变量快照不落明文凭据）。
pub(crate) fn redact_vars(vars: &HashMap<String, String>) -> HashMap<String, String> {
    miyo_sync::redact::redact_vars(vars)
}

/// 运行完成后落历史：响应正文 64KB 截断保留（可回看），请求快照与变量池
/// 脱敏，meta_json 存开始时间 / 当时配置快照 / 初始变量（脱敏）/ 策略 /
/// 预检警告；计数按执行状态；保留最近 N 条。
#[allow(clippy::too_many_arguments)]
async fn persist_scenario_run(
    state: &State<'_, AppState>,
    scenario_id: Uuid,
    environment_id: Option<Uuid>,
    run_id: Option<&str>,
    started_at: chrono::DateTime<chrono::Utc>,
    stop_on_failure: bool,
    steps_snapshot_json: &str,
    base_vars: &HashMap<String, String>,
    total_steps: usize,
    result: &miyo_test::runner::ScenarioRunResult,
) -> CommandResult<()> {
    let mut results = result.results.clone();
    for r in &mut results {
        if let Some(resp) = r.response.as_mut() {
            if resp.body.len() > HISTORY_BODY_BYTES {
                resp.body = crate::commands::request::byte_truncate(&resp.body, HISTORY_BODY_BYTES)
                    .to_string();
                resp.history_truncated = true;
            }
            if let Some(snapshot) = resp.request.as_mut() {
                redact_request_snapshot(snapshot);
            }
        }
    }
    // 计数按执行状态（results 含 skipped / blocked 行，不再用差值推算）。
    let count = |statuses: &[&str]| {
        results
            .iter()
            .filter(|r| statuses.contains(&r.effective_status()))
            .count() as i64
    };
    let ok_count = count(&["success"]);
    let fail_count = count(&["failed", "cancelled"]);
    let skipped_count = count(&["skipped", "blocked"]);
    let meta = serde_json::json!({
        "version": 1,
        "startedAt": started_at.to_rfc3339(),
        "stopOnFailure": stop_on_failure,
        // 当时配置快照（材料化后的步骤）——历史重跑基于它而非当前场景。
        "steps": serde_json::from_str::<serde_json::Value>(steps_snapshot_json)
            .unwrap_or(serde_json::Value::Null),
        // 初始变量（环境 / 项目 / 全局合并结果），敏感值脱敏。
        "initialVars": redact_vars(base_vars),
        "warnings": result.warnings,
    });
    let record = ScenarioRun {
        // id 即前端生成的 run_id（同一次重试覆盖旧行，不膨胀）。
        id: run_id
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4),
        scenario_id,
        environment_id,
        finished_at: chrono::Utc::now(),
        duration_ms: Some(results.iter().filter_map(|r| r.duration_ms).sum()),
        total_steps: total_steps as i64,
        ok_count,
        fail_count,
        skipped_count,
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        results_json: serde_json::to_string(&results)
            .map_err(|e| CommandError::validation(format!("运行结果序列化失败：{e}")))?,
        variables_json: serde_json::to_string(&redact_vars(&result.variables))
            .map_err(|e| CommandError::validation(format!("变量池序列化失败：{e}")))?,
        meta_json: Some(meta.to_string()),
    };
    repo::save_scenario_run(&state.db, &record).await?;
    repo::prune_scenario_runs(&state.db, scenario_id, repo::SCENARIO_RUN_KEEP).await?;
    Ok(())
}

/// 删除单条运行历史。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_scenario_run(
    state: State<'_, AppState>,
    run_id: String,
) -> CommandResult<bool> {
    let uuid = Uuid::parse_str(&run_id)
        .map_err(|_| CommandError::validation(format!("运行历史 id 无效：{run_id}")))?;
    Ok(repo::delete_scenario_run(&state.db, uuid).await?)
}

/// 清空场景的全部运行历史（返回删除条数）。
#[tauri::command(rename_all = "camelCase")]
pub async fn clear_scenario_runs(
    state: State<'_, AppState>,
    scenario_id: Uuid,
) -> CommandResult<u64> {
    Ok(repo::delete_scenario_runs_all(&state.db, scenario_id).await?)
}

/// 列出场景最近的运行历史（新→旧，默认 20 条）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_scenario_runs(
    state: State<'_, AppState>,
    scenario_id: Uuid,
    limit: Option<i64>,
) -> CommandResult<Vec<ScenarioRunSummary>> {
    // 摘要查询：列表 IPC 只回计数，不读大 JSON 列（详情走 get_scenario_run）。
    let runs =
        repo::list_scenario_run_summaries(&state.db, scenario_id, limit.unwrap_or(20)).await?;
    Ok(runs
        .into_iter()
        .map(|r| ScenarioRunSummary {
            id: r.id,
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
    let meta = run
        .meta_json
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok());
    Ok(Some(ScenarioRunDetail {
        summary: ScenarioRunSummary::from_record(run),
        results,
        variables,
        meta,
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

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_http::client::RequestSnapshot;
    use std::collections::HashMap;

    #[test]
    fn redact_vars_masks_sensitive_keys() {
        let mut vars = HashMap::new();
        vars.insert("orderNo".to_string(), "SO-1".to_string());
        vars.insert("password".to_string(), "明文密码".to_string());
        vars.insert("accessToken".to_string(), "tok".to_string());
        let out = redact_vars(&vars);
        assert_eq!(out.get("orderNo").map(String::as_str), Some("SO-1"));
        assert_eq!(out.get("password").map(String::as_str), Some(PLACEHOLDER));
        assert_eq!(
            out.get("accessToken").map(String::as_str),
            Some(PLACEHOLDER)
        );
    }

    #[test]
    fn redact_vars_keeps_empty_values() {
        let mut vars = HashMap::new();
        vars.insert("token".to_string(), String::new());
        let out = redact_vars(&vars);
        // 空值无泄露风险，保持原样（占位符反而误导「有值被隐藏」）。
        assert_eq!(out.get("token").map(String::as_str), Some(""));
    }

    #[test]
    fn redact_request_snapshot_masks_headers_cookies_and_query() {
        let mut snapshot = RequestSnapshot {
            method: "POST".into(),
            url: "http://x/api?api_key=secret&page=2".into(),
            url_template: Some("http://x/api?api_key={{key}}&page=2".into()),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer real".to_string()),
            ],
            cookies: vec![("sessionid".to_string(), "sid-9".to_string())],
            body: r#"{"password": "kept-in-body"}"#.into(),
            body_omitted: false,
        };
        redact_request_snapshot(&mut snapshot);
        let auth = snapshot
            .headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .unwrap();
        assert_eq!(auth.1, PLACEHOLDER);
        assert_eq!(
            snapshot
                .headers
                .iter()
                .find(|(k, _)| k == "Content-Type")
                .unwrap()
                .1,
            "application/json"
        );
        assert_eq!(snapshot.cookies[0].1, PLACEHOLDER);
        assert!(snapshot.url.contains("api_key=") && !snapshot.url.contains("secret"));
        // url_template 保留模板语义（渲染前），不参与脱敏。
        assert!(snapshot
            .url_template
            .as_deref()
            .unwrap()
            .contains("{{key}}"));
    }
}
