//! 测试用例 Command：列表（按接口）/ 保存 / 更新（名称·分组·请求内容·运行状态）/ 删除
//! / 拖拽排序 / 执行历史（list / get / delete；落库口径与 scenario_runs 同款）。

use tauri::State;
use uuid::Uuid;

use miyo_core::model::{HttpMethod, KeyValue, TestCase, TestCaseRun, TestCaseStatus};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 允许的用例分组。
const CATEGORIES: [&str; 5] = ["正向", "负向", "边界值", "安全性", "其他"];

/// 列出接口的全部测试用例（组内按 sort_order, created_at 稳定排序）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_test_cases(
    state: State<'_, AppState>,
    request_id: Uuid,
) -> CommandResult<Vec<TestCase>> {
    repo::list_test_cases(&state.db, request_id)
        .await
        .map_err(Into::into)
}

/// 取单条测试用例（抽屉深链 / 逻辑引用场景）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_test_case(
    state: State<'_, AppState>,
    case_id: Uuid,
) -> CommandResult<Option<TestCase>> {
    repo::get_test_case(&state.db, case_id)
        .await
        .map_err(Into::into)
}

/// 保存测试用例（新建）。名称必填、分组须在允许集合内。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_test_case(
    state: State<'_, AppState>,
    test_case: TestCase,
) -> CommandResult<TestCase> {
    if test_case.name.trim().is_empty() {
        return Err(CommandError::validation("用例名称不能为空"));
    }
    if !CATEGORIES.contains(&test_case.category.as_str()) {
        return Err(CommandError::validation(format!(
            "无效的用例分组：{}",
            test_case.category
        )));
    }
    repo::create_test_case(&state.db, &test_case).await?;
    Ok(test_case)
}

/// 更新用例名称 / 分组 / 文件夹（编辑入口）。文件夹为自由分组名，空 = 未分组。
#[tauri::command(rename_all = "camelCase")]
pub async fn update_test_case_meta(
    state: State<'_, AppState>,
    case_id: Uuid,
    name: String,
    category: String,
    folder: String,
) -> CommandResult<()> {
    if name.trim().is_empty() {
        return Err(CommandError::validation("用例名称不能为空"));
    }
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(CommandError::validation(format!(
            "无效的用例分组：{category}"
        )));
    }
    let folder = folder.trim().to_string();
    if folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::update_test_case_meta(&state.db, case_id, &name, &category, &folder)
        .await
        .map_err(Into::into)
}

/// 批量移动用例到文件夹（空 = 移出分组；行菜单「移动到分组」/ 跨组拖拽）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_test_case_folder(
    state: State<'_, AppState>,
    case_ids: Vec<Uuid>,
    folder: String,
) -> CommandResult<u64> {
    if case_ids.is_empty() {
        return Err(CommandError::validation("未选择用例"));
    }
    let folder = folder.trim().to_string();
    if folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::set_test_cases_folder(&state.db, &case_ids, &folder)
        .await
        .map_err(Into::into)
}

/// 重命名接口下的一个用例文件夹（整组改名；目标名已存在时两组合并）。
#[tauri::command(rename_all = "camelCase")]
pub async fn rename_test_case_folder(
    state: State<'_, AppState>,
    request_id: Uuid,
    old_folder: String,
    new_folder: String,
) -> CommandResult<u64> {
    let old_folder = old_folder.trim().to_string();
    let new_folder = new_folder.trim().to_string();
    if old_folder.is_empty() || new_folder.is_empty() {
        return Err(CommandError::validation("文件夹名称不能为空"));
    }
    if old_folder == new_folder {
        return Ok(0);
    }
    if new_folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::rename_test_case_folder(&state.db, request_id, &old_folder, &new_folder)
        .await
        .map_err(Into::into)
}

/// 拖拽排序：按前端给定的展示顺序写回组内 sort_order。
#[tauri::command(rename_all = "camelCase")]
pub async fn reorder_test_cases(
    state: State<'_, AppState>,
    case_ids: Vec<Uuid>,
) -> CommandResult<()> {
    if case_ids.is_empty() {
        return Ok(());
    }
    repo::reorder_test_cases(&state.db, &case_ids)
        .await
        .map_err(Into::into)
}

/// 更新用例运行状态（Success / Failed / Untested）。
#[tauri::command(rename_all = "camelCase")]
pub async fn update_test_case_status(
    state: State<'_, AppState>,
    case_id: Uuid,
    status: TestCaseStatus,
) -> CommandResult<()> {
    repo::update_test_case_status(&state.db, case_id, status)
        .await
        .map_err(Into::into)
}

/// 更新用例完整内容（抽屉「保存修改」：请求快照 + 断言 / 提取 / 执行控制）。
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn update_test_case_content(
    state: State<'_, AppState>,
    case_id: Uuid,
    method: HttpMethod,
    url_path: String,
    path_variables: Vec<KeyValue>,
    params: Vec<KeyValue>,
    headers: Vec<KeyValue>,
    body_type: String,
    body_content: String,
    assertions: serde_json::Value,
    extractors: serde_json::Value,
    auth: Option<miyo_core::model::AuthSpec>,
    timeout_ms: Option<u64>,
    disable_cookies: bool,
) -> CommandResult<()> {
    if url_path.trim().is_empty() {
        return Err(CommandError::validation("请求路径不能为空"));
    }
    if !assertions.is_array() {
        return Err(CommandError::validation("断言配置必须是数组"));
    }
    if !extractors.is_array() {
        return Err(CommandError::validation("提取配置必须是数组"));
    }
    if let Some(t) = timeout_ms {
        if t > miyo_test::config::StepLoop::MAX_INTERVAL_MS {
            return Err(CommandError::validation("超时最长 60000ms"));
        }
    }
    repo::update_test_case_content(
        &state.db,
        case_id,
        method,
        &url_path,
        &path_variables,
        &params,
        &headers,
        &body_type,
        &body_content,
        &assertions,
        &extractors,
        &auth,
        timeout_ms,
        disable_cookies,
    )
    .await
    .map_err(Into::into)
}

/// 删除测试用例。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_test_case(state: State<'_, AppState>, case_id: Uuid) -> CommandResult<()> {
    repo::delete_test_case(&state.db, case_id)
        .await
        .map_err(Into::into)
}

/// 列出用例执行历史（新→旧；limit 缺省 50）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_test_case_runs(
    state: State<'_, AppState>,
    case_id: Uuid,
    limit: Option<i64>,
) -> CommandResult<Vec<TestCaseRun>> {
    repo::list_test_case_runs(&state.db, case_id, limit.unwrap_or(50).clamp(1, 200))
        .await
        .map_err(Into::into)
}

/// 取单条执行历史（回看详情）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_test_case_run(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> CommandResult<Option<TestCaseRun>> {
    repo::get_test_case_run(&state.db, run_id)
        .await
        .map_err(Into::into)
}

/// 删除单条执行历史。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_test_case_run(state: State<'_, AppState>, run_id: Uuid) -> CommandResult<bool> {
    repo::delete_test_case_run(&state.db, run_id)
        .await
        .map_err(Into::into)
}

/// 历史中响应正文的保留上限（与 scenario_runs 的 HISTORY_BODY_BYTES 同口径）。
const HISTORY_BODY_BYTES: usize = 64 * 1024;

/// 运行完成后落用例执行历史 + 回写列表摘要（last_run_status/at/ok/fail）。
///
/// fail-open：历史落库失败只记日志，不影响运行结果返回（与场景历史一致）。
/// 请求快照与响应正文同口径处理：64KB 截断、敏感头 / 变量脱敏。
pub(crate) async fn persist_test_case_run(
    db: &sqlx::SqlitePool,
    case_id: Uuid,
    endpoint_id: Option<Uuid>,
    environment_id: Option<Uuid>,
    started_at: chrono::DateTime<chrono::Utc>,
    result: &miyo_test::runner::EndpointResult,
) {
    let status = if result.cancelled {
        TestCaseStatus::Failed
    } else if result.ok {
        TestCaseStatus::Success
    } else {
        TestCaseStatus::Failed
    };
    let ok_count = result.outcomes.iter().filter(|o| o.passed).count() as u32;
    let fail_count = result.outcomes.iter().filter(|o| !o.passed).count() as u32;

    // 快照处理：64KB 截断 + 请求快照脱敏（与场景历史同函数）。
    let mut snapshot = result.clone();
    if let Some(resp) = snapshot.response.as_mut() {
        if resp.body.len() > HISTORY_BODY_BYTES {
            resp.body =
                crate::commands::request::byte_truncate(&resp.body, HISTORY_BODY_BYTES).to_string();
            resp.history_truncated = true;
        }
        if let Some(req) = resp.request.as_mut() {
            crate::commands::scenarios::redact_request_snapshot(req);
        }
    }
    let results_json = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into());
    let record = TestCaseRun {
        id: Uuid::new_v4(),
        case_id,
        endpoint_id,
        environment_id,
        started_at,
        finished_at: chrono::Utc::now(),
        duration_ms: result.duration_ms,
        status,
        status_code: result.status,
        ok_count,
        fail_count,
        error: result
            .request_error
            .clone()
            .or_else(|| result.cancelled.then(|| "运行被取消".to_string())),
        results_json,
    };
    let ran_at = record.finished_at;
    let summary_status = record.status;
    let (ok, fail) = (record.ok_count, record.fail_count);
    if let Err(e) = async {
        repo::save_test_case_run(db, &record).await?;
        repo::prune_test_case_runs(db, case_id, repo::TEST_CASE_RUN_KEEP).await?;
        repo::update_test_case_run_summary(
            db,
            case_id,
            summary_status,
            ran_at,
            Some(ok),
            Some(fail),
        )
        .await
    }
    .await
    {
        // 历史链路 fail-open：DB 故障不打断运行结果返回。
        let _ = &e;
        tracing::warn!("[persist_test_case_run] 落历史失败：{e}");
        return;
    }
}
