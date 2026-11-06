//! 请求历史 Command：查询最近请求记录（execute_request 成功时自动落库）。

use serde::Serialize;
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

/// 仪表板最近活动（全局跨项目最近请求，单条 IPC 替代逐项目 N+1 查询）。
///
/// 字段保持 snake_case：前端 miyoApi.d.ts / ProjectList.vue 按蛇形读取
/// （与 ProjectStat 同一约定，序列化契约由 serialization_contract_test 兜底）。
/// created_at / url 直接透传存储层原始字符串（RFC3339），不经过模型重解析。
#[derive(Debug, Clone, Serialize)]
pub struct RecentActivity {
    pub id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub endpoint_name: Option<String>,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub created_at: String,
}

/// 查询全局最近请求活动（时间倒序，默认 20 条）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_recent_activities(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> CommandResult<Vec<RecentActivity>> {
    let limit = limit.filter(|l| *l > 0).unwrap_or(20);
    let rows = repo::list_recent_activities(&state.db, limit).await?;
    Ok(rows
        .into_iter()
        .map(|r| RecentActivity {
            id: Uuid::parse_str(&r.id).unwrap_or_else(|_| Uuid::nil()),
            project_id: Uuid::parse_str(&r.project_id).unwrap_or_else(|_| Uuid::nil()),
            project_name: r.project_name,
            endpoint_name: r.endpoint_name,
            method: r.method,
            url: r.url,
            status: r.status.map(|s| s as u16),
            duration_ms: r.duration_ms.map(|d| d as u64),
            created_at: r.created_at,
        })
        .collect())
}
