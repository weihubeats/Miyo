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
use crate::error::{CommandError, CommandErrorCode, CommandResult};
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
    /// 步骤重试策略（显式配置才启用；None = 不重试）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<miyo_core::model::StepRetry>,
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
    /// Setup 步骤（生命周期；主步骤之前执行，提取产物流入主步骤）。
    #[serde(default)]
    pub setup: Vec<ScenarioStepArgs>,
    /// Teardown 步骤（生命周期；主步骤结束后执行，失败 / 取消时尽可能执行）。
    #[serde(default)]
    pub teardown: Vec<ScenarioStepArgs>,
    /// 自动清理开关（false = 不自动执行 Teardown）。
    #[serde(default = "default_step_enabled")]
    pub teardown_enabled: bool,
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

    // 物化步骤：单源走 miyo_test::materialize_step（URL / 规格保留模板；
    // 全局参数补缺、提取/断言与执行控制解析、wait_ms 收钳都在共享函数内）。
    // 禁用步骤照传（runner 落 skipped 行，结果面板完整呈现「为什么没跑」）。
    let materialize = |steps_args: Vec<ScenarioStepArgs>,
                       vars: &HashMap<String, String>|
     -> Result<Vec<ScenarioStepInput>, CommandError> {
        steps_args
            .into_iter()
            .enumerate()
            .map(|(idx, step)| {
                miyo_test::materialize::materialize_step(
                    miyo_test::materialize::StepMaterialInput {
                        endpoint: step.endpoint,
                        url_template: step.url,
                        extract_json: step.extract,
                        assertions_json: step.assertions,
                        condition_json: step.condition,
                        loop_json: step.r#loop,
                        retry: step.retry,
                        enabled: step.enabled,
                        wait_ms: step.wait_ms,
                        parallel: step.parallel,
                        overrides_json: None,
                        auto_idempotency_key: None,
                        step_label: format!("第 {} 步", idx + 1),
                    },
                    &global_params,
                    vars,
                )
                .map_err(CommandError::validation)
            })
            .collect()
    };
    let mut steps = materialize(args.steps, &vars)?;
    // 生命周期步骤：Setup 提取产物流入主步骤（同一执行上下文物化）。
    let setup_steps = materialize(args.setup, &vars)?;
    let teardown_steps = materialize(args.teardown, &vars)?;
    // 写操作 + 重试 → 步骤级幂等键（同一步骤全部尝试共用；跨步骤互异）。
    let idem_prefix = args
        .run_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    miyo_test::runner::apply_retry_idempotency(&mut steps, &idem_prefix);
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
    // 「从历史重跑」基于这份快照而非场景当前配置。快照在落库前脱敏：
    // 材料化已把 {{token}} 类变量代成真实值，明文落库 = 凭证进历史库并随
    // get_scenario_run 回传前端（与 results_json / variables_json 同口径）。
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
                // 执行阶段（main 缺省；setup / teardown = 生命周期实时进度）
                "phase": p.phase,
                "result": step,
            }),
        );
    };
    let options = ScenarioOptions {
        timeout_ms: Some(timeout_ms),
        cancel: cancel.as_ref().map(|(_, t)| t.clone()),
        stop_on_failure: args.stop_on_failure,
    };
    let lifecycle = miyo_test::runner::ScenarioLifecycle {
        setup: &setup_steps,
        teardown: &teardown_steps,
        teardown_enabled: args.teardown_enabled,
    };
    let mut result = miyo_test::runner::run_scenario_lifecycle(
        &lifecycle,
        steps,
        &vars,
        &options,
        Some(&progress),
    )
    .await;

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
        // 主步骤 + 生命周期行（setup / teardown）同口径截断；缓存键含阶段
        // 前缀，避免与主步骤位次冲突。
        let mut cap_all = |rows: &mut Vec<miyo_test::runner::EndpointResult>, prefix: &str| {
            for (i, step) in rows.iter_mut().enumerate() {
                let Some(resp) = step.response.as_mut() else {
                    continue;
                };
                let (preview, omitted) = cap_ipc_body(&resp.body);
                if omitted {
                    if !run_key.is_empty() {
                        cache.insert(format!("{run_key}:{prefix}{i}"), resp.body.clone());
                    }
                    resp.body = preview;
                    resp.body_omitted = true;
                }
            }
        };
        cap_all(&mut result.setup_results, "s:");
        cap_all(&mut result.results, "");
        cap_all(&mut result.teardown_results, "t:");
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
            None,
        )
        .await
        {
            tracing::warn!("[run_scenario] 保存运行历史失败：{e}");
        }
    }

    Ok(result)
}

// ---------- 数据驱动 / 环境矩阵（交付 §一 / §六） ----------

/// 矩阵一行入参：数据组（vars 注入）或环境（environmentId 指定，变量取该
/// 环境合并结果）。key 为稳定标识（数据集 id / 环境 id）。
#[derive(Debug, Deserialize)]
pub struct MatrixRowArgs {
    pub key: String,
    pub label: String,
    /// 环境行：变量取该环境的合并结果（每行独立解析——环境间零污染）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<Uuid>,
    /// 数据行：在基础环境变量之上覆盖该行变量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vars: Option<HashMap<String, String>>,
}

/// 矩阵运行入参：与单场景运行相同的步骤 + 多行执行。
#[derive(Debug, Deserialize)]
pub struct RunScenarioMatrixArgs {
    pub steps: Vec<ScenarioStepArgs>,
    #[serde(default)]
    pub setup: Vec<ScenarioStepArgs>,
    #[serde(default)]
    pub teardown: Vec<ScenarioStepArgs>,
    #[serde(default = "default_step_enabled")]
    pub teardown_enabled: bool,
    pub rows: Vec<MatrixRowArgs>,
    #[serde(default)]
    pub stop_on_failure: bool,
    /// 矩阵组 id（前端生成；历史按组聚合 + 取消令牌键）。
    pub group_id: String,
    /// 同时在途行数（1 = 串行；行间变量隔离与并行度无关）。
    #[serde(default)]
    pub concurrency: Option<usize>,
    /// 归属场景（落历史）。
    #[serde(default)]
    pub scenario_id: Option<Uuid>,
    /// 数据行的基础环境（行 vars 在其变量之上覆盖；缺省激活环境）。
    #[serde(default)]
    pub environment_id: Option<Uuid>,
}

/// 矩阵运行的单行结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRowOutcome {
    pub key: String,
    pub label: String,
    /// 该行运行历史 id（落库后可用 get_scenario_run 查详情）。
    pub run_id: String,
    pub ok: bool,
    pub ok_count: i64,
    pub fail_count: i64,
    pub skipped_count: i64,
    pub duration_ms: Option<f64>,
}

/// 矩阵运行汇总。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRunOutcome {
    pub group_id: String,
    pub total: usize,
    pub ok_rows: usize,
    pub rows: Vec<MatrixRowOutcome>,
}

/// 矩阵运行：同一场景 × N 行（数据组 / 环境）串行执行，**每行独立变量池**
/// （行间零污染），逐行落运行历史（meta.matrix 标注 + 共享 groupId），
/// 返回逐行汇总。步骤 / 生命周期与单场景运行共用同一物化与执行引擎。
#[tauri::command(rename_all = "camelCase")]
pub async fn run_scenario_matrix(
    app: AppHandle,
    state: State<'_, AppState>,
    args: RunScenarioMatrixArgs,
) -> CommandResult<MatrixRunOutcome> {
    if args.steps.is_empty() {
        return Err(CommandError::validation("场景没有可执行的步骤"));
    }
    if args.rows.is_empty() {
        return Err(CommandError::validation("矩阵至少需要一行数据 / 一个环境"));
    }

    let settings = state.global_settings().await?;
    let global_params = settings.global_params.clone();

    // 场景级并发守卫（矩阵运行同样只允许一个在途实例）。
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
        running.insert(scenario_id, args.group_id.clone());
    }
    // 取消令牌（组级：取消整个矩阵）。
    let cancel = tokio_util::sync::CancellationToken::new();
    state
        .run_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(args.group_id.clone(), cancel.clone());

    let options = ScenarioOptions {
        timeout_ms: None, // 每行用步骤级 / 全局超时（同单场景口径）
        cancel: Some(cancel.clone()),
        stop_on_failure: args.stop_on_failure,
    };
    let teardown_enabled = args.teardown_enabled;
    let concurrency = args.concurrency.unwrap_or(1);

    let total = args.rows.len();
    let mut outcomes: Vec<MatrixRowOutcome> = Vec::with_capacity(total);
    if concurrency > 1 {
        // 并行：引擎并发跑全部行（结果按行序回填），随后统一落库 + 汇总。
        let materialize_owned = |steps_args: &[ScenarioStepArgs],
                                 vars: &HashMap<String, String>|
         -> Result<Vec<ScenarioStepInput>, CommandError> {
            steps_args
                .iter()
                .enumerate()
                .map(|(i, step)| {
                    miyo_test::materialize::materialize_step(
                        miyo_test::materialize::StepMaterialInput {
                            endpoint: step.endpoint.clone(),
                            url_template: step.url.clone(),
                            extract_json: step.extract.clone(),
                            assertions_json: step.assertions.clone(),
                            condition_json: step.condition.clone(),
                            loop_json: step.r#loop.clone(),
                            retry: step.retry.clone(),
                            enabled: step.enabled,
                            wait_ms: step.wait_ms,
                            parallel: step.parallel,
                            overrides_json: None,
                            auto_idempotency_key: None,
                            step_label: format!("第 {} 步", i + 1),
                        },
                        &global_params,
                        vars,
                    )
                    .map_err(CommandError::validation)
                })
                .collect()
        };
        // 基础变量（数据行覆盖；环境行在 run rows 独立解析）——这里按行预解析。
        let mut row_vars: Vec<HashMap<String, String>> = Vec::with_capacity(args.rows.len());
        let mut row_timeout: Vec<u64> = Vec::with_capacity(args.rows.len());
        for row in &args.rows {
            let (vars, timeout_ms) = tokio::join!(
                state.variables_for(row.environment_id.or(args.environment_id)),
                async {
                    let first = args.steps.first().map(|s| s.endpoint.request.clone());
                    match first {
                        Some(spec) => resolve_timeout_ms(&spec, &state).await,
                        None => Ok(miyo_http::client::DEFAULT_TIMEOUT_MS),
                    }
                },
            );
            let mut vars = vars?;
            if let Some(row_vars_override) = &row.vars {
                for (k, v) in row_vars_override {
                    vars.insert(k.clone(), v.clone());
                }
            }
            row_timeout.push(timeout_ms?);
            row_vars.push(vars);
        }
        let engine_rows: Vec<miyo_test::runner::MatrixRow> = args
            .rows
            .iter()
            .zip(row_vars.iter())
            .map(|(row, vars)| miyo_test::runner::MatrixRow {
                key: row.key.clone(),
                label: row.label.clone(),
                vars: vars.clone(),
            })
            .collect();
        let lifecycle = miyo_test::runner::ScenarioLifecycle {
            setup: &[],
            teardown: &[],
            teardown_enabled,
        };
        // 并行行的单行超时取各自行解析值的首个（行级差异化超时随行 vars
        // 已在物化时生效——引擎 options 统一超时取全局兜底）。
        let options_par = ScenarioOptions {
            timeout_ms: row_timeout.first().copied(),
            cancel: Some(cancel.clone()),
            stop_on_failure: false,
        };
        let results = miyo_test::runner::run_scenario_matrix_concurrency(
            &|| materialize_owned(&args.steps, &row_vars[0]).unwrap_or_default(),
            &engine_rows,
            &lifecycle,
            &options_par,
            None,
            concurrency,
        )
        .await;
        for (idx, row_result) in results.into_iter().enumerate() {
            let row = &args.rows[idx];
            let row_run_id = Uuid::new_v4();
            // 每行按行 vars 重新物化（保持与串行路径一致的落库口径）。
            let steps = materialize_owned(&args.steps, &row_vars[idx])?;
            let steps_snapshot = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string());
            let row_ok = row_result
                .result
                .results
                .iter()
                .all(|r| r.ok || r.cancelled)
                && !row_result.result.results.is_empty();
            if let Some(scenario_id) = args.scenario_id {
                let result_ref = &row_result.result;
                if let Err(e) = persist_scenario_run(
                    &state,
                    scenario_id,
                    row.environment_id.or(args.environment_id),
                    Some(&row_run_id.to_string()),
                    chrono::Utc::now(),
                    args.stop_on_failure,
                    &steps_snapshot,
                    &row_vars[idx],
                    result_ref.results.len(),
                    result_ref,
                    Some(serde_json::json!({
                        "kind": if row.environment_id.is_some() { "environment" } else { "data" },
                        "key": row.key,
                        "label": row.label,
                        "index": idx,
                        "total": total,
                        "groupId": args.group_id,
                    })),
                )
                .await
                {
                    tracing::warn!("[run_scenario_matrix] 第 {} 行落历史失败：{e}", idx + 1);
                }
            }
            let _ = app.emit(
                "miyo:matrix-progress",
                serde_json::json!({
                    "groupId": args.group_id,
                    "index": idx + 1,
                    "total": total,
                    "key": row.key,
                    "label": row.label,
                    "ok": row_ok,
                    "runId": row_run_id.to_string(),
                }),
            );
            outcomes.push(MatrixRowOutcome {
                key: row.key.clone(),
                label: row.label.clone(),
                run_id: row_run_id.to_string(),
                ok: row_ok,
                ok_count: row_result.result.results.iter().filter(|r| r.ok).count() as i64,
                fail_count: row_result
                    .result
                    .results
                    .iter()
                    .filter(|r| !r.ok && !r.cancelled)
                    .count() as i64,
                skipped_count: row_result.result.results.len() as i64
                    - row_result.result.results.iter().filter(|r| r.ok).count() as i64
                    - row_result
                        .result
                        .results
                        .iter()
                        .filter(|r| !r.ok && !r.cancelled)
                        .count() as i64,
                duration_ms: row_result
                    .result
                    .results
                    .iter()
                    .filter_map(|r| r.duration_ms)
                    .sum::<f64>()
                    .into(),
            });
        }
    } else {
        for (idx, row) in args.rows.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            // 每行独立变量池：环境行重新解析合并变量；数据行在基础变量上覆盖。
            let (vars, timeout_ms) = tokio::join!(
                state.variables_for(row.environment_id.or(args.environment_id)),
                async {
                    let first = args.steps.first().map(|s| s.endpoint.request.clone());
                    match first {
                        Some(spec) => resolve_timeout_ms(&spec, &state).await,
                        None => Ok(miyo_http::client::DEFAULT_TIMEOUT_MS),
                    }
                },
            );
            let mut vars: VariableMap = vars?;
            let mut timeout_ms = timeout_ms?;
            if let Some(row_vars) = &row.vars {
                for (k, v) in row_vars {
                    vars.insert(k.clone(), v.clone());
                }
            }
            let _ = &mut timeout_ms;

            // 物化（行变量参与全局参数注入——每行独立）+ 幂等键（行级前缀）。
            let materialize = |steps_args: &[ScenarioStepArgs],
                               vars: &HashMap<String, String>|
             -> Result<Vec<ScenarioStepInput>, CommandError> {
                steps_args
                    .iter()
                    .enumerate()
                    .map(|(i, step)| {
                        miyo_test::materialize::materialize_step(
                            miyo_test::materialize::StepMaterialInput {
                                endpoint: step.endpoint.clone(),
                                url_template: step.url.clone(),
                                extract_json: step.extract.clone(),
                                assertions_json: step.assertions.clone(),
                                condition_json: step.condition.clone(),
                                loop_json: step.r#loop.clone(),
                                retry: step.retry.clone(),
                                enabled: step.enabled,
                                wait_ms: step.wait_ms,
                                parallel: step.parallel,
                                overrides_json: None,
                                auto_idempotency_key: None,
                                step_label: format!("第 {} 步", i + 1),
                            },
                            &global_params,
                            vars,
                        )
                        .map_err(CommandError::validation)
                    })
                    .collect()
            };
            let mut steps = materialize(&args.steps, &vars)?;
            let setup_steps = materialize(&args.setup, &vars)?;
            let teardown_steps = materialize(&args.teardown, &vars)?;
            let row_run_id = Uuid::new_v4();
            miyo_test::runner::apply_retry_idempotency(&mut steps, &row_run_id.to_string());
            // 行配置快照（脱敏后随 meta 落库，供该行失败后的恢复重跑）。
            let steps_snapshot = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string());

            let lifecycle_row = miyo_test::runner::ScenarioLifecycle {
                setup: &setup_steps,
                teardown: &teardown_steps,
                teardown_enabled,
            };
            let result = miyo_test::runner::run_scenario_lifecycle(
                &lifecycle_row,
                steps,
                &vars,
                &options,
                None,
            )
            .await;

            // 逐行落历史（groupId 关联同组行；meta.matrix 标注行信息）。
            let row_ok =
                result.results.iter().all(|r| r.ok || r.cancelled) && !result.results.is_empty();
            if let Some(scenario_id) = args.scenario_id {
                if let Err(e) = persist_scenario_run(
                    &state,
                    scenario_id,
                    row.environment_id.or(args.environment_id),
                    Some(&row_run_id.to_string()),
                    chrono::Utc::now(),
                    args.stop_on_failure,
                    &steps_snapshot,
                    &vars,
                    result.results.len(),
                    &result,
                    Some(serde_json::json!({
                        "kind": if row.environment_id.is_some() { "environment" } else { "data" },
                        "key": row.key,
                        "label": row.label,
                        "index": idx,
                        "total": total,
                        "groupId": args.group_id,
                    })),
                )
                .await
                {
                    tracing::warn!("[run_scenario_matrix] 第 {} 行落历史失败：{e}", idx + 1);
                }
            }

            let outcome = MatrixRowOutcome {
                key: row.key.clone(),
                label: row.label.clone(),
                run_id: row_run_id.to_string(),
                ok: row_ok,
                ok_count: result.results.iter().filter(|r| r.ok).count() as i64,
                fail_count: result
                    .results
                    .iter()
                    .filter(|r| !r.ok && !r.cancelled)
                    .count() as i64,
                skipped_count: result.results.len() as i64
                    - result.results.iter().filter(|r| r.ok).count() as i64
                    - result
                        .results
                        .iter()
                        .filter(|r| !r.ok && !r.cancelled)
                        .count() as i64,
                duration_ms: result
                    .results
                    .iter()
                    .filter_map(|r| r.duration_ms)
                    .sum::<f64>()
                    .into(),
            };
            let _ = app.emit(
                "miyo:matrix-progress",
                serde_json::json!({
                    "groupId": args.group_id,
                    "index": idx + 1,
                    "total": total,
                    "key": row.key,
                    "label": row.label,
                    "ok": outcome.ok,
                    "runId": outcome.run_id,
                }),
            );
            outcomes.push(outcome);
        }

        // 释放守卫 / 取消令牌。
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&args.group_id);
        if let Some(scenario_id) = args.scenario_id {
            state
                .scenario_running
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&scenario_id);
        }
    }
    let ok_rows = outcomes.iter().filter(|o| o.ok).count();
    Ok(MatrixRunOutcome {
        group_id: args.group_id,
        total: args.rows.len(),
        ok_rows,
        rows: outcomes,
    })
}

// ---------- 测试报告导出（交付 §八） ----------

/// 报告导出格式。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioReportFormat {
    /// 自包含 HTML（统计 + 环境对比 + 步骤明细）。
    Html,
    /// JUnit XML（CI 消费）。
    JunitXml,
    /// 统计 JSON（前端自定义渲染）。
    Json,
}

/// 导出结果：内容 + 建议文件名。
#[derive(Debug, Serialize)]
pub struct ScenarioReportExport {
    pub content: String,
    pub suggested_name: String,
}

/// 导出场景测试报告：按 runIds 取运行历史 → 聚合统计 → HTML / JUnit XML /
/// 统计 JSON。环境 / 数据组维度取 meta.matrix.label（环境行）与
/// meta.environmentName（单环境历史运行），缺省「默认」。
#[tauri::command(rename_all = "camelCase")]
pub async fn export_scenario_report(
    state: State<'_, AppState>,
    _scenario_id: Uuid,
    scenario_name: String,
    run_ids: Vec<String>,
    format: ScenarioReportFormat,
) -> CommandResult<ScenarioReportExport> {
    if run_ids.is_empty() {
        return Err(CommandError::validation("请选择要纳入报告的运行历史"));
    }
    let mut runs = Vec::with_capacity(run_ids.len());
    for id in &run_ids {
        let uuid = Uuid::parse_str(id)
            .map_err(|_| CommandError::validation(format!("运行历史 id 无效：{id}")))?;
        let Some(run) = repo::get_scenario_run(&state.db, uuid).await? else {
            continue;
        };
        let mut results: Vec<miyo_test::runner::EndpointResult> =
            serde_json::from_str(&run.results_json).unwrap_or_default();
        // 报告复用历史脱敏（响应头 / 请求快照 / 提取值）。
        for r in &mut results {
            if let Some(resp) = r.response.as_mut() {
                for (k, v) in resp.headers.iter_mut() {
                    if is_sensitive_key(k) && !v.is_empty() {
                        *v = PLACEHOLDER.to_string();
                    }
                }
                if let Some(snapshot) = resp.request.as_mut() {
                    redact_request_snapshot(snapshot);
                }
            }
            for kv in r.extracted.iter_mut() {
                if is_sensitive_key(kv.0) && !kv.1.is_empty() {
                    *kv.1 = PLACEHOLDER.to_string();
                }
            }
        }
        let meta: serde_json::Value = run
            .meta_json
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or(serde_json::Value::Null);
        let matrix = meta.get("matrix");
        let environment = matrix
            .filter(|m| m["kind"] == "environment")
            .and_then(|m| m["label"].as_str())
            .map(str::to_string);
        let data_set = matrix
            .filter(|m| m["kind"] == "data")
            .and_then(|m| m["label"].as_str())
            .map(str::to_string);
        // 非矩阵运行的失败恢复统计维度：无。
        let _ = &meta;
        let ok = run.fail_count == 0 && !run.cancelled;
        runs.push(miyo_test::report::ReportRun {
            run_id: run.id.to_string(),
            finished_at: run.finished_at.to_rfc3339(),
            environment,
            data_set,
            duration_ms: run.duration_ms,
            ok,
            results,
        });
    }
    let stats = miyo_test::report::compute_stats(&scenario_name, &runs);
    let (content, ext) = match format {
        ScenarioReportFormat::Html => (
            miyo_test::report::html_report(&scenario_name, &stats, &runs),
            "html",
        ),
        ScenarioReportFormat::JunitXml => {
            (miyo_test::report::junit_xml(&scenario_name, &runs), "xml")
        }
        ScenarioReportFormat::Json => (
            serde_json::to_string_pretty(&stats).map_err(|e| {
                CommandError::with_code(CommandErrorCode::Internal, format!("统计序列化失败：{e}"))
            })?,
            "json",
        ),
    };
    let date = chrono::Local::now().format("%Y-%m-%d");
    Ok(ScenarioReportExport {
        content,
        suggested_name: format!("report-{scenario_name}-{date}.{ext}"),
    })
}

/// 场景运行历史条目（IPC 摘要；results_json / variables_json 不下发）。
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    matrix: Option<serde_json::Value>,
) -> CommandResult<()> {
    // 结果行 = setup(phase=setup) + 主步骤 + teardown(phase=teardown)：
    // 同一 results_json 承载全生命周期（旧读取端按 Vec<EndpointResult> 兼容）；
    // 成败计数只看主步骤（phase=None）。
    let cap_rows =
        |rows: &Vec<miyo_test::runner::EndpointResult>| -> Vec<miyo_test::runner::EndpointResult> {
            rows.iter()
                .map(|r| {
                    let mut r = r.clone();
                    if let Some(resp) = r.response.as_mut() {
                        if resp.body.len() > HISTORY_BODY_BYTES {
                            resp.body = crate::commands::request::byte_truncate(
                                &resp.body,
                                HISTORY_BODY_BYTES,
                            )
                            .to_string();
                            resp.history_truncated = true;
                        }
                        if let Some(snapshot) = resp.request.as_mut() {
                            redact_request_snapshot(snapshot);
                        }
                    }
                    r
                })
                .collect()
        };
    let mut results = cap_rows(&result.setup_results);
    results.extend(cap_rows(&result.results));
    results.extend(cap_rows(&result.teardown_results));
    // 计数按执行状态（只看主步骤；results 含 skipped / blocked 行）。
    let count = |statuses: &[&str]| {
        results
            .iter()
            .filter(|r| r.phase.is_none() && statuses.contains(&r.effective_status()))
            .count() as i64
    };
    let ok_count = count(&["success"]);
    let fail_count = count(&["failed", "cancelled"]);
    let skipped_count = count(&["skipped", "blocked"]);
    // 材料化步骤快照脱敏（auth / 敏感头 / body / URL query）：历史重跑
    // 需要「当时的配置形状」，不需要当时的真实凭证（initialVars 同口径）。
    let mut steps_value = serde_json::from_str::<serde_json::Value>(steps_snapshot_json)
        .unwrap_or(serde_json::Value::Null);
    miyo_core::sensitive::redact_request_shape_json(&mut steps_value);
    // 失败恢复：每步执行前的 runtime 快照（敏感值脱敏后落库）——历史重跑
    // 失败步骤以此为初始上下文（此前仅实时运行可用）。
    let snapshots: Vec<serde_json::Value> = result
        .vars_snapshots
        .iter()
        .map(|snap| {
            serde_json::to_value(miyo_sync::redact::redact_vars(snap))
                .unwrap_or(serde_json::Value::Null)
        })
        .collect();
    let mut meta = serde_json::json!({
        "version": 2,
        "startedAt": started_at.to_rfc3339(),
        "stopOnFailure": stop_on_failure,
        // 当时配置快照（材料化后的步骤，敏感值已脱敏）——历史重跑基于它
        // 而非当前场景；脱敏字段需用户在环境中补值。
        "steps": steps_value,
        // 失败恢复上下文（v2 新增；旧行缺省 = 无快照，重跑回退 initialVars）。
        "varsSnapshots": snapshots,
        "lifecycle": {
            "setupCount": result.setup_results.len(),
            "teardownCount": result.teardown_results.len(),
            "teardownOk": result.teardown_results.iter().all(|r| r.ok),
        },
        // 初始变量（环境 / 项目 / 全局合并结果），敏感值脱敏。
        "initialVars": redact_vars(base_vars),
        "warnings": result.warnings,
    });
    // 矩阵行标注（数据驱动 / 环境矩阵）：历史列表按组聚合展示。
    if let Some(m) = matrix {
        meta["matrix"] = m;
    }
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
