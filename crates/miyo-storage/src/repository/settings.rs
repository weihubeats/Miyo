//! Settings（通用键值）。

use std::collections::HashMap;

use sqlx::{QueryBuilder, SqlitePool};

use miyo_core::Result;

pub async fn set_setting(db: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value_json) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get_setting(db: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value_json FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0))
}

/// 删除单键（恢复「未配置」语义：读取方走默认值兜底）。
pub async fn delete_setting(db: &SqlitePool, key: &str) -> Result<()> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(key)
        .execute(db)
        .await?;
    Ok(())
}

/// 批量读取多个键的设置（一次 `WHERE key IN (...)` 查询）。
///
/// 缺失的键不进返回表；SQLite 变量数上限按 500 分片。
/// 备份导出等需要连续读多个键的场景用它替代逐键 `get_setting`。
pub async fn get_settings(db: &SqlitePool, keys: &[&str]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for chunk in keys.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new("SELECT key, value_json FROM settings WHERE key IN (");
        let mut separated = qb.separated(", ");
        for key in chunk {
            separated.push_bind(*key);
        }
        separated.push_unseparated(")");
        let rows: Vec<(String, String)> = qb.build_query_as().fetch_all(db).await?;
        out.extend(rows);
    }
    Ok(out)
}
