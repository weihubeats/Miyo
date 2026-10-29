//! 集成测试场景 Command：项目级接口链路的 CRUD 与串行运行。
//!
//! 步骤为引用式（endpoint_id + 步骤级提取/断言），运行时用「环境静态变量 +
//! runtime 快照（前步提取值优先）」渲染每步的 URL / 规格模板——前步提取的
//! 变量即成为后续请求的入参（详见 miyo-test `run_scenario`）。

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use miyo_core::model::{Endpoint, Scenario};
use miyo_core::VariableMap;
use miyo_storage::repository as repo;
use miyo_test::runner::{ScenarioOptions, ScenarioStepInput};

use crate::commands::request::resolve_timeout_ms;
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
    let result = miyo_test::runner::run_scenario(steps, &vars, &options, Some(&progress)).await;

    // 无论完成 / 取消，都从注册表移除，避免泄漏。
    if let Some((id, _)) = &cancel {
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    Ok(result)
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
