//! Request History。

use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::RequestHistory;
use miyo_core::Result;

use super::rows::HistoryRow;

/// 每个项目保留的历史条数上限（超出按最旧淘汰）。
///
/// 历史每条含请求/响应摘要 JSON，无限增长会持续膨胀数据库并拖慢
/// 列表查询与启动迁移，故写入时顺带按项目裁剪。
pub const HISTORY_RETENTION_PER_PROJECT: i64 = 500;

/// 裁剪节流：每项目计数，该项目每 N 次写入做一次保留策略裁剪，而非逐次双写。
/// 逐次裁剪意味着请求热路径每次 2 次写（INSERT + 全表 DELETE…NOT IN）。
///
/// 原来是全局 `AtomicU64`，但裁剪只针对当前写入的项目——多项目交替写入时
/// 计数被摊薄，任何单个项目都可能长期攒不满阈值，保留上限形同虚设。
/// 改为按项目 id 分别计数，各项目独立达到阈值即各自裁剪。
const TRIM_EVERY_INSERTS: u64 = 20;

/// 各项目已写入次数（key = project_id 文本，与库中列一致）。
fn project_insert_ticks() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    use std::sync::OnceLock;
    static TICKS: OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        OnceLock::new();
    TICKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 项目写入计数 +1，返回该计数（用于判断是否到达裁剪阈值）。
fn bump_project_tick(project_id: &str) -> u64 {
    let mut map = project_insert_ticks()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(project_id.to_string()).or_insert(0);
    *entry += 1;
    *entry
}

pub async fn save_request_history(db: &SqlitePool, model: &RequestHistory) -> Result<()> {
    let row = HistoryRow::from_model(model);
    sqlx::query(
        "INSERT INTO request_histories
         (id, project_id, endpoint_id, method, url, status, duration_ms, request_summary_json, response_summary_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.endpoint_id)
    .bind(&row.method)
    .bind(&row.url)
    .bind(row.status)
    .bind(row.duration_ms)
    .bind(&row.request_summary_json)
    .bind(&row.response_summary_json)
    .bind(row.created_at.clone())
    .execute(db)
    .await?;
    // 保留策略节流：该项目超额最多延迟 N 条被清理，换热路径少一次写。
    // `tick` 为 1 起的写入序号，取 `tick % N == 1` 让首轮写入就裁一次
    //（清掉历史遗留的超额行），此后每 N 条一轮，与原全局节流节奏一致。
    let tick = bump_project_tick(&row.project_id);
    if tick % TRIM_EVERY_INSERTS == 1 {
        trim_request_history(db, &row.project_id).await?;
    }
    Ok(())
}

/// 保留策略：淘汰指定项目最旧的超额历史（`HISTORY_RETENTION_PER_PROJECT` 之外）。
pub async fn trim_request_history(db: &SqlitePool, project_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM request_histories
         WHERE project_id = ? AND id NOT IN (
             SELECT id FROM request_histories WHERE project_id = ?
             ORDER BY created_at DESC LIMIT ?
         )",
    )
    .bind(project_id)
    .bind(project_id)
    .bind(HISTORY_RETENTION_PER_PROJECT)
    .execute(db)
    .await?;
    Ok(())
}

/// 查询项目请求历史（时间倒序）；`endpoint_id` 为 Some 时仅返回该接口的记录。
///
/// 按有无 endpoint 条件拆两条 SQL：`(? IS NULL OR endpoint_id = ?)` 的 OR
/// 会让 endpoint 过滤走不上索引，全项目历史一多按接口过滤就全表扫描。
pub async fn list_request_histories(
    db: &SqlitePool,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<RequestHistory>> {
    const BASE: &str = "SELECT id, project_id, endpoint_id, method, url, status, duration_ms,
                request_summary_json, response_summary_json, created_at
         FROM request_histories";
    let rows: Vec<HistoryRow> = match endpoint_id {
        Some(ep_id) => {
            sqlx::query_as(&format!(
                "{BASE} WHERE project_id = ? AND endpoint_id = ? ORDER BY created_at DESC LIMIT ?"
            ))
            .bind(project_id.to_string())
            .bind(ep_id.to_string())
            .bind(limit)
            .fetch_all(db)
            .await?
        }
        None => {
            sqlx::query_as(&format!(
                "{BASE} WHERE project_id = ? ORDER BY created_at DESC LIMIT ?"
            ))
            .bind(project_id.to_string())
            .bind(limit)
            .fetch_all(db)
            .await?
        }
    };
    rows.into_iter().map(HistoryRow::into_model).collect()
}

/// 清空项目请求历史；`endpoint_id` 为 Some 时仅清该接口的记录。返回删除条数。
///
/// 按有无 endpoint 条件拆两条 SQL（同 [`list_request_histories`]）：
/// `(? IS NULL OR endpoint_id = ?)` 会让 DELETE 走不上 endpoint 索引，
/// 按接口清空时全表扫描。
pub async fn clear_request_histories(
    db: &SqlitePool,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
) -> Result<u64> {
    let result = match endpoint_id {
        Some(ep_id) => {
            sqlx::query("DELETE FROM request_histories WHERE project_id = ? AND endpoint_id = ?")
                .bind(project_id.to_string())
                .bind(ep_id.to_string())
                .execute(db)
                .await?
        }
        None => {
            sqlx::query("DELETE FROM request_histories WHERE project_id = ?")
                .bind(project_id.to_string())
                .execute(db)
                .await?
        }
    };
    Ok(result.rows_affected())
}

/// 删除单条请求历史（敏感记录即时清除；审计 #6：原先只能整页清空）。
pub async fn delete_request_history(db: &SqlitePool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM request_histories WHERE id = ?")
        .bind(id.to_string())
        .execute(db)
        .await?;
    Ok(())
}
