//! 自增序列 Command：查看 / 设置 / 删除 `{{$seq:key}}` 计数器（持久化到 settings 表）。

use std::collections::HashMap;

use tauri::State;

use miyo_core::model::SeqCounter;
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// settings 表中自增序列的存储键（JSON map：key → 下一次输出值）。
const SEQ_STORAGE_KEY: &str = "seq_counters";

/// 列出全部自增序列（含全局 `$seq`，其 key 为空字符串）。
#[tauri::command(rename_all = "camelCase")]
pub fn list_seq_counters() -> Vec<SeqCounter> {
    miyo_core::variable::list_seq_counters()
        .into_iter()
        .map(|(key, value)| SeqCounter { key, value })
        .collect()
}

/// 设置自增序列的下一次输出值（key 为空 = 全局 `$seq`），并持久化到磁盘。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_seq_counter(
    state: State<'_, AppState>,
    key: String,
    value: u64,
) -> CommandResult<()> {
    if value == 0 {
        return Err(CommandError::validation("自增起始值需 ≥ 1"));
    }
    miyo_core::variable::set_seq_counter(&key, value);
    sync_seq_counters(&state.db).await?;
    Ok(())
}

/// 删除自增序列（key 为空 = 全局 `$seq`；删除后再使用从 1 重新开始）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_seq_counter(state: State<'_, AppState>, key: String) -> CommandResult<()> {
    miyo_core::variable::delete_seq_counter(&key);
    sync_seq_counters(&state.db).await?;
    Ok(())
}

/// 将内存计数落盘（请求执行后调用，保证「UI 看到的值」重启后一致）。
pub async fn sync_seq_counters(db: &sqlx::SqlitePool) -> CommandResult<()> {
    let map = miyo_core::variable::dump_seq_counters();
    let json = serde_json::to_string(&map).map_err(|e| {
        CommandError::with_code(CommandErrorCode::Internal, format!("序列化失败：{e}"))
    })?;
    repo::set_setting(db, SEQ_STORAGE_KEY, &json).await?;
    Ok(())
}

/// 脏检查版落盘：计数器未推进时跳过 settings 查询 + 写入。
/// 请求 / 测试 / 压测热路径用此函数（`{{$seq}}` 未使用时零开销）。
pub async fn sync_seq_counters_if_dirty(db: &sqlx::SqlitePool) -> CommandResult<()> {
    if !miyo_core::variable::take_seq_dirty() {
        return Ok(());
    }
    sync_seq_counters(db).await
}

/// 启动时从磁盘恢复自增序列（加载失败静默，从默认状态开始）。
pub async fn apply_saved_seq_counters(db: &sqlx::SqlitePool) {
    let raw = match repo::get_setting(db, SEQ_STORAGE_KEY).await {
        Ok(Some(json)) => json,
        _ => return,
    };
    if let Ok(map) = serde_json::from_str::<HashMap<String, u64>>(&raw) {
        miyo_core::variable::load_seq_counters(map);
    }
}
