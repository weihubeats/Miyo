//! 请求历史 Command：查询最近请求记录（execute_request 成功时自动落库）。

use tauri::State;
use uuid::Uuid;

use miyo_core::model::RequestHistory;
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 删除单条请求历史（不校验归属：id 为 UUID 主键，泄漏面为同机登录用户）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_request_history(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    repo::delete_request_history(&state.db, id)
        .await
        .map_err(CommandError::from)
}

/// 查询项目最近的请求历史（按时间倒序，默认 50 条）。
/// `endpoint_id` 提供时仅返回该接口的记录（前端「仅当前接口」过滤）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_request_histories(
    state: State<'_, AppState>,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    limit: Option<i64>,
) -> CommandResult<Vec<RequestHistory>> {
    let limit = limit.filter(|l| *l > 0).unwrap_or(50);
    repo::list_request_histories(&state.db, project_id, endpoint_id, limit)
        .await
        .map_err(CommandError::from)
}

/// 清空请求历史（`endpoint_id` 提供时仅清该接口），返回删除条数。
#[tauri::command(rename_all = "camelCase")]
pub async fn clear_request_histories(
    state: State<'_, AppState>,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
) -> CommandResult<u64> {
    repo::clear_request_histories(&state.db, project_id, endpoint_id)
        .await
        .map_err(CommandError::from)
}
