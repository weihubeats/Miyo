//! 备份/恢复 Command:项目全量 JSON 导出(下载)与导入(新项目 UUID 重映射)。
//! 对应 Dioxus 版 M10 备份功能。

use tauri::State;
use uuid::Uuid;

use miyo_backup::{build_backup, restore_backup, BackupFile, BackupInput};
use miyo_core::model::EndpointStatus;
use miyo_storage::repository as repo;

use crate::error::CommandResult;
use crate::state::AppState;

/// 备份设置白名单（全局维度，随备份走；恢复时保守合并，见 `backup_restore`）。
const BACKUP_SETTING_KEYS: &[&str] = &["http_proxy", "http_timeout_ms", "seq_counters"];

/// 导出项目为备份 JSON 字符串（含项目 + 文件夹 + 接口 + 环境 + Mock 规则 +
/// 响应示例 + 请求用例 + 全局设置快照 + 全局变量/参数）。
#[tauri::command(rename_all = "camelCase")]
pub async fn backup_export(state: State<'_, AppState>, project_id: Uuid) -> CommandResult<String> {
    // 互不依赖的读（全部只按 project_id / 全局维度查）合并并发：
    // 8 次 SQLite 往返压成 1 次等待；设置键也并入，3 次 get_setting 合一条 IN 查询。
    let (
        project,
        folders,
        endpoints,
        environments,
        mock_rules,
        global_variables,
        global_params,
        settings,
        proto_files,
    ) = tokio::join!(
        repo::get_project(&state.db, project_id),
        repo::list_folders(&state.db, project_id),
        repo::list_endpoints(&state.db, project_id),
        repo::list_environments(&state.db, project_id),
        repo::list_mock_rules(&state.db, project_id),
        repo::get_global_variables(&state.db),
        repo::get_global_params(&state.db),
        repo::get_settings(&state.db, BACKUP_SETTING_KEYS),
        repo::list_proto_files(&state.db, project_id),
    );
    let project = project?;
    let folders = folders?;
    let endpoints = endpoints?;
    let environments = environments?;
    let mock_rules = mock_rules?;
    let global_variables = global_variables?;
    let global_params = global_params?;
    let settings = settings?;
    let proto_files = proto_files?;

    // 批量一次查询（去 N+1：E 个接口原来 2E 次查询 + 2E 次 JSON 反序列化）。
    let active_ids: Vec<Uuid> = endpoints
        .iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
        .map(|e| e.id)
        .collect();
    let (response_examples, request_examples) = tokio::join!(
        repo::list_response_examples_by_endpoints(&state.db, &active_ids),
        repo::list_request_examples_by_endpoints(&state.db, &active_ids),
    );
    let response_examples: Vec<_> = response_examples
        .unwrap_or_default()
        .into_values()
        .flatten()
        .collect();
    let request_examples: Vec<_> = request_examples
        .unwrap_or_default()
        .into_values()
        .flatten()
        .collect();

    // 团队元数据（v4）：项目归属团队随备份走，缺失（老项目）时为空。
    let team = match project.team_id {
        Some(tid) => repo::get_team(&state.db, tid).await.ok(),
        None => None,
    };

    let file = build_backup(&BackupInput {
        project: &project,
        folders: &folders,
        endpoints: &endpoints,
        environments: &environments,
        mock_rules: &mock_rules,
        response_examples: &response_examples,
        request_examples: &request_examples,
        proto_files: &proto_files,
        settings: &settings,
        global_variables: &global_variables,
        global_params: &global_params,
        team: &team,
    });
    file.serialize().map_err(Into::into)
}

/// 从备份 JSON 恢复：校验格式 → 全量重映射 UUID → 落库为全新项目。
/// 返回 `{ id, name, counts }` 摘要。
///
/// 原子性：全部写入在单个 SQLite 事务内（`BEGIN IMMEDIATE`），任一步失败
/// 自动回滚——用户视角真正「要么全部成功，要么什么都没发生」，且中间态
/// 对其他连接不可见（原来是逐条串行 + 事后补偿删除）。
#[tauri::command(rename_all = "camelCase")]
pub async fn backup_restore(
    state: State<'_, AppState>,
    text: String,
) -> CommandResult<serde_json::Value> {
    let file = BackupFile::parse(&text)?;
    let restored = restore_backup(&file);

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(miyo_core::AppError::Database)?;
    let result: miyo_core::Result<()> = async {
        // v4 备份带团队元数据：先 upsert 团队（项目 team_id 外键引用它）。
        if let Some(team) = &file.team {
            repo::save_team(tx.as_mut(), team).await?;
        }
        repo::save_project(tx.as_mut(), &restored.project).await?;
        // 每类实体一条多值 INSERT（每批 200 行，事务内），替代原来逐行
        // INSERT：N 行从 N 次往返降到 ceil(N/200) 次，原子性不变。
        repo::save_folders_bulk(tx.as_mut(), &restored.folders).await?;
        repo::save_endpoints_bulk(tx.as_mut(), &restored.endpoints).await?;
        // 环境随备份项目重映射归属后整体写入。
        repo::save_environments_bulk(tx.as_mut(), &restored.environments).await?;
        repo::save_mock_rules_bulk(tx.as_mut(), &restored.mock_rules).await?;
        repo::save_response_examples_bulk(tx.as_mut(), &restored.response_examples).await?;
        repo::save_request_examples_bulk(tx.as_mut(), &restored.request_examples).await?;
        repo::save_proto_files_bulk(tx.as_mut(), &restored.proto_files).await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        // tx 未 commit，drop 即回滚；显式回滚失败也仅记录，不掩盖原始错误。
        if let Err(rb) = tx.rollback().await {
            tracing::error!(
                project = %restored.project.id,
                error = %rb,
                "backup_restore 事务回滚失败"
            );
        }
        return Err(e.into());
    }
    tx.commit().await.map_err(miyo_core::AppError::Database)?;

    // 全局维度合并（commit 之后、尽力而为，失败仅告警不回滚项目数据）：
    // 全部为保守合并（缺失才补 / 取 max），幂等可重试，故放在事务外，
    // 避免把加密 blob 编解码塞进原子单元。
    let globals = restore_globals(&state.db, &file).await.unwrap_or_else(|e| {
        tracing::warn!("备份全局维度合并失败（项目数据已落库）：{e}");
        GlobalRestoreSummary::default()
    });
    // 恢复可能合入全局变量 / 参数 / 超时：作废进程内缓存，下一次请求重新查库。
    state.invalidate_global_settings();

    Ok(serde_json::json!({
        "id": restored.project.id,
        "name": restored.project.name,
        "folders": restored.folders.len(),
        "endpoints": restored.endpoints.len(),
        "environments": restored.environments.len(),
        "mock_rules": restored.mock_rules.len(),
        "response_examples": restored.response_examples.len(),
        "request_examples": restored.request_examples.len(),
        "proto_files": restored.proto_files.len(),
        "settings_applied": globals.settings_applied,
        "settings_skipped": globals.settings_skipped,
        "global_variables_merged": globals.global_variables_merged,
        "global_params_merged": globals.global_params_merged,
    }))
}

/// 全局维度恢复摘要（合并计数，供前端展示"恢复了什么"）。
#[derive(Debug, Default)]
struct GlobalRestoreSummary {
    settings_applied: Vec<String>,
    settings_skipped: Vec<String>,
    global_variables_merged: usize,
    global_params_merged: usize,
}

/// 全局维度保守合并：
///
/// - 全局变量 / 参数：按 key 补缺，不覆盖现有同名项；
/// - `http_proxy` / `http_timeout_ms`：仅当前未配置时应用，不覆盖用户现有全局配置；
/// - `seq_counters`：按 key 取 max 合并，并刷新内存计数（重启一致）。
async fn restore_globals(
    db: &sqlx::SqlitePool,
    file: &BackupFile,
) -> miyo_core::Result<GlobalRestoreSummary> {
    let mut summary = GlobalRestoreSummary::default();

    if !file.global_variables.is_empty() {
        let mut current = repo::get_global_variables(db).await?;
        let mut keys: std::collections::HashSet<String> =
            current.iter().map(|v| v.key.clone()).collect();
        for v in &file.global_variables {
            if keys.insert(v.key.clone()) {
                current.push(v.clone());
                summary.global_variables_merged += 1;
            }
        }
        if summary.global_variables_merged > 0 {
            repo::save_global_variables(db, &current).await?;
        }
    }

    if !file.global_params.is_empty() {
        let mut current = repo::get_global_params(db).await?;
        let keys: std::collections::HashSet<String> = current
            .iter()
            .map(|p| format!("{:?}:{}", p.location, p.key.to_lowercase()))
            .collect();
        for p in &file.global_params {
            let key = format!("{:?}:{}", p.location, p.key.to_lowercase());
            if !keys.contains(&key) {
                current.push(p.clone());
                summary.global_params_merged += 1;
            }
        }
        if summary.global_params_merged > 0 {
            repo::save_global_params(db, &current).await?;
        }
    }

    for key in ["http_proxy", "http_timeout_ms"] {
        let Some(value) = file.settings.get(key) else {
            continue;
        };
        if repo::get_setting(db, key).await?.is_some() {
            summary.settings_skipped.push(key.to_string());
        } else {
            repo::set_setting(db, key, value).await?;
            summary.settings_applied.push(key.to_string());
        }
    }

    if let Some(seq_json) = file.settings.get("seq_counters") {
        let incoming: std::collections::HashMap<String, u64> =
            serde_json::from_str(seq_json).unwrap_or_default();
        if !incoming.is_empty() {
            let mut merged = miyo_core::variable::dump_seq_counters();
            for (k, v) in incoming {
                let entry = merged.entry(k).or_insert(v);
                if *entry < v {
                    *entry = v;
                }
            }
            let json = serde_json::to_string(&merged).map_err(miyo_core::AppError::Json)?;
            repo::set_setting(db, "seq_counters", &json).await?;
            miyo_core::variable::load_seq_counters(merged);
            summary.settings_applied.push("seq_counters".to_string());
        }
    }

    Ok(summary)
}
