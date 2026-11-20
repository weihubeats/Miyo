//! WebSocket 离线消息（ws_messages）。

use crate::db::db_err;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{WsMessageRecord, WsMessageType};
use miyo_core::Result;

use super::rows::WsMessageRow;

/// 持久化一条待发消息（发送队列溢出 / 连接不可达时保活）。
pub async fn enqueue_ws_message(
    db: &SqlitePool,
    target: &str,
    message_type: WsMessageType,
    payload: &str,
) -> Result<WsMessageRecord> {
    let record = WsMessageRecord {
        id: Uuid::new_v4(),
        message_type,
        payload: payload.to_string(),
        created_at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO ws_messages (id, target, message_type, payload, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(record.id.to_string())
    .bind(target)
    .bind(message_type.as_str())
    .bind(&record.payload)
    .bind(record.created_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(record)
}

/// 查询指定连接的目标地址尚未发送的消息（重连成功后按序补发）。
///
/// `max` 为单次补发的条数上限：取**最新**的 `max` 条、按 created_at 升序返回，
/// 更旧的积压放弃补发（TTL 清理兜底）——防止断线期间的无界积压在单次
/// 重连时被一次性全量载入内存。调用方以 `返回条数 == max` 判断可能截断。
pub async fn list_pending_ws_messages(
    db: &SqlitePool,
    target: &str,
    max: i64,
) -> Result<Vec<WsMessageRecord>> {
    let rows: Vec<WsMessageRow> = sqlx::query_as(
        "SELECT id, message_type, payload, created_at
         FROM ws_messages WHERE target = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(target)
    .bind(max)
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    let mut records: Vec<WsMessageRecord> = rows
        .into_iter()
        .map(WsMessageRow::into_model)
        .collect::<Result<_>>()?;
    records.reverse();
    Ok(records)
}

/// 删除已发送的持久化消息。
pub async fn delete_ws_messages(db: &SqlitePool, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    // 单条 IN 批量删除，避免逐条 DELETE 的往返开销
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("DELETE FROM ws_messages WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id.to_string());
    }
    query.execute(db).await.map_err(db_err)?;
    Ok(())
}

/// 清理超过保留期的待发消息（过期丢弃），返回清理条数。
pub async fn purge_expired_ws_messages(
    db: &SqlitePool,
    target: &str,
    retention: chrono::Duration,
) -> Result<u64> {
    let cutoff = (Utc::now() - retention).to_rfc3339();
    let result = sqlx::query("DELETE FROM ws_messages WHERE target = ? AND created_at < ?")
        .bind(target)
        .bind(cutoff)
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(result.rows_affected())
}
