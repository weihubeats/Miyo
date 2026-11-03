//! 同步 Command：远端 CRUD + 快照推/拉（预览后覆盖恢复）。
//!
//! 维度模型：远端绑定在**团队**上（一个团队一个路径，不同团队可复用同一
//! git 仓库）；推/拉按**项目**触发，同一仓库内按 `projects/{project_id}.json`
//! 分文件存放各项目快照。
//!
//! 快照装配复用 backup 链路（miyo-backup v4）；拉取恢复 = 项目级覆盖
//! （覆盖前自动全量快照兜底，见 `sync/backups/`），不做字段级合并。

use std::path::PathBuf;

use tauri::State;
use uuid::Uuid;

use miyo_backup::{build_backup, BackupFile, BackupInput};
use miyo_core::model::{EndpointStatus, SyncRemote, SyncRemoteKind};
use miyo_storage::repository as repo;
use miyo_sync::engine::{PullPreview, SyncEngine};
use miyo_sync::git::GitTransport;
use miyo_sync::redact::{redact_snapshot, restore_local_values};
use miyo_sync::{RemoteConfig, RemoteTransport, SyncMessage};

use crate::error::CommandResult;
use crate::state::AppState;

/// 备份设置白名单（与 backup.rs 保持一致）。
const BACKUP_SETTING_KEYS: &[&str] = &["http_proxy", "http_timeout_ms", "seq_counters"];

// ---------- 远端 CRUD（团队维度） ----------

#[tauri::command(rename_all = "camelCase")]
pub async fn sync_remote_list(
    state: State<'_, AppState>,
    team_id: Uuid,
) -> CommandResult<Vec<SyncRemote>> {
    Ok(repo::list_sync_remotes(&state.db, team_id).await?)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn sync_remote_create(
    state: State<'_, AppState>,
    team_id: Uuid,
    config: serde_json::Value,
) -> CommandResult<SyncRemote> {
    // kind 从 config 的 tag 推断（RemoteConfig 序列化格式：{"kind":"git",...}）。
    let kind = match config.get("kind").and_then(|v| v.as_str()) {
        Some("git") => SyncRemoteKind::Git,
        Some("s3") => SyncRemoteKind::S3,
        Some("db") => SyncRemoteKind::Db,
        other => {
            return Err(miyo_core::validation(format!("未知同步远端类型：{other:?}")).into());
        }
    };
    Ok(repo::create_sync_remote(&state.db, team_id, kind, &config).await?)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn sync_remote_delete(state: State<'_, AppState>, remote_id: Uuid) -> CommandResult<()> {
    Ok(repo::delete_sync_remote(&state.db, remote_id).await?)
}

// ---------- 推 / 拉（项目维度触发） ----------

/// 校验项目归属远端所属团队，返回项目与快照文件相对路径。
async fn project_for_remote(
    state: &State<'_, AppState>,
    remote: &SyncRemote,
    project_id: Uuid,
) -> Result<(miyo_core::model::Project, String), miyo_core::AppError> {
    let project = repo::get_project(&state.db, project_id).await?;
    let project_team = project.team_id.unwrap_or(miyo_core::model::DEFAULT_TEAM_ID);
    if project_team != remote.team_id {
        return Err(miyo_core::validation(
            "该项目不属于此同步远端所在的团队（请先将项目移入团队）",
        ));
    }
    Ok((project, snapshot_path(remote, project_id)))
}

/// 仓库内快照文件相对路径：`{config.path 或 projects}/{project_id}.json`。
fn snapshot_path(remote: &SyncRemote, project_id: Uuid) -> String {
    let dir = remote
        .config
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("projects");
    format!("{dir}/{project_id}.json")
}

/// 推送项目快照到团队远端。
///
/// 推送前**必做脱敏**：远端对全团队可见，凭证类字段替换为占位符。
/// 返回被脱敏的项清单，让用户明确知道哪些值没有离开本机。
#[tauri::command(rename_all = "camelCase")]
pub async fn sync_push(
    state: State<'_, AppState>,
    remote_id: Uuid,
    project_id: Uuid,
    message: Option<String>,
) -> CommandResult<serde_json::Value> {
    let remote = repo::get_sync_remote(&state.db, remote_id).await?;
    let (_, file_path) = project_for_remote(&state, &remote, project_id).await?;
    let transport = build_transport(&remote, &file_path)?;
    let snapshot = assemble_snapshot(&state, project_id).await?;

    // 脱敏：所有可能含凭证的字段替换为占位符。
    let report = redact_snapshot(&snapshot);

    let engine = SyncEngine::new(transport.as_ref());
    let result = engine
        .push(
            &report.snapshot,
            SyncMessage {
                summary: message.unwrap_or_else(|| {
                    format!(
                        "Miyo 同步：{} @ {}",
                        report.snapshot.project.name,
                        utc_now_short()
                    )
                }),
            },
        )
        .await
        .map_err(|e| miyo_core::validation(e.to_string()))?;

    repo::set_last_synced_revision(&state.db, remote.id, &result.revision).await?;
    Ok(serde_json::json!({
        "revision": result.revision,
        "noop": result.noop,
        "redacted": report.log.entries,
    }))
}

/// 拉取预览：返回差异与远端快照（不改本地，确认后走 sync_pull_apply）。
#[tauri::command(rename_all = "camelCase")]
pub async fn sync_pull_preview(
    state: State<'_, AppState>,
    remote_id: Uuid,
    project_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let remote = repo::get_sync_remote(&state.db, remote_id).await?;
    let (_, file_path) = project_for_remote(&state, &remote, project_id).await?;
    let transport = build_transport(&remote, &file_path)?;
    let engine = SyncEngine::new(transport.as_ref());

    let preview = engine
        .preview_pull(remote.last_synced_revision.as_deref())
        .await
        .map_err(|e| miyo_core::validation(e.to_string()))?;
    Ok(preview_json(&preview))
}

/// 应用拉取：本地自动快照兜底 → 远端快照覆盖恢复到现有项目。
#[tauri::command(rename_all = "camelCase")]
pub async fn sync_pull_apply(
    state: State<'_, AppState>,
    remote_id: Uuid,
    project_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let remote = repo::get_sync_remote(&state.db, remote_id).await?;
    let (_, file_path) = project_for_remote(&state, &remote, project_id).await?;
    let transport = build_transport(&remote, &file_path)?;
    let engine = SyncEngine::new(transport.as_ref());

    let preview = engine
        .preview_pull(remote.last_synced_revision.as_deref())
        .await
        .map_err(|e| miyo_core::validation(e.to_string()))?;

    // 覆盖前先取本地快照：① 还原占位符为本机凭证 ② 覆盖前状态留档。
    let local = assemble_snapshot(&state, project_id).await?;
    let mut incoming = preview.remote_snapshot;
    restore_local_values(&mut incoming, &local);

    // 覆盖前自动快照（JSON 留档；失败仅告警不阻断）。
    let backup_path = write_pre_pull_backup(project_id, &local).await.ok();

    apply_snapshot(&state, project_id, &incoming).await?;
    repo::set_last_synced_revision(&state.db, remote.id, &preview.revision).await?;

    Ok(serde_json::json!({
        "revision": preview.revision,
        "applied": true,
        "endpoints": incoming.endpoints.len(),
        "local_backup": backup_path,
    }))
}

/// 覆盖前把本地快照写入 `sync/backups/{project_id}-{时间戳}.json`。
///
/// 序列化 + 写盘为同步 IO 且快照可达数 MB：包 `spawn_blocking`，
/// 避免阻塞 tokio runtime worker（同 import_export.rs 的 tokio::fs 口径）。
async fn write_pre_pull_backup(
    project_id: Uuid,
    snapshot: &BackupFile,
) -> Result<String, miyo_core::AppError> {
    let dir = miyo_core::paths::default_data_dir()
        .join("sync")
        .join("backups");
    let serialized = tokio::task::spawn_blocking({
        let snapshot = snapshot.clone();
        move || snapshot.serialize()
    })
    .await
    .map_err(|e| miyo_core::AppError::Io(std::io::Error::other(format!("备份任务失败：{e}"))))??;
    let path = tokio::task::spawn_blocking(move || -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(&dir)?;
        let name = format!(
            "{}-{}.json",
            project_id,
            chrono::Utc::now().format("%Y%m%dT%H%M%S")
        );
        let path = dir.join(name);
        std::fs::write(&path, serialized.as_bytes())?;
        Ok(path)
    })
    .await
    .map_err(|e| miyo_core::AppError::Io(std::io::Error::other(format!("备份任务失败：{e}"))))?
    .map_err(miyo_core::AppError::Io)?;
    tracing::info!(path = %path.display(), "拉取覆盖前已留档本地快照");
    Ok(path.display().to_string())
}

// ---------- 内部装配 ----------

fn build_transport(
    remote: &SyncRemote,
    file_path: &str,
) -> CommandResult<Box<dyn RemoteTransport>> {
    let parsed: RemoteConfig = serde_json::from_value(remote.config.clone())
        .map_err(|e| miyo_core::validation(format!("远端配置非法：{e}")))?;
    match parsed {
        RemoteConfig::Git { url, branch, .. } => {
            // 每个远端独立的本地缓存仓库（团队维度）；快照文件路径按项目分文件。
            let workdir: PathBuf = miyo_core::paths::default_data_dir()
                .join("sync")
                .join(remote.id.to_string());
            Ok(Box::new(GitTransport::new(url, branch, file_path, workdir)))
        }
        RemoteConfig::S3 { .. } => Err(miyo_core::validation("S3 远端尚未实现").into()),
        RemoteConfig::Db { .. } => Err(miyo_core::validation("数据库远端尚未实现").into()),
    }
}

/// 装配项目快照（逻辑与 backup_export 一致，收敛为共享函数）。
async fn assemble_snapshot(
    state: &State<'_, AppState>,
    project_id: Uuid,
) -> Result<BackupFile, miyo_core::AppError> {
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
        scenarios,
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
        repo::list_scenarios(&state.db, project_id),
    );
    let (project, folders, endpoints, environments, mock_rules) =
        (project?, folders?, endpoints?, environments?, mock_rules?);
    let (global_variables, global_params, settings, proto_files, scenarios) = (
        global_variables?,
        global_params?,
        settings?,
        proto_files?,
        scenarios?,
    );

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

    let team = match project.team_id {
        Some(tid) => repo::get_team(&state.db, tid).await.ok(),
        None => None,
    };

    Ok(build_backup(&BackupInput {
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
        scenarios: &scenarios,
    }))
}

/// 快照覆盖恢复到现有项目 id（不重映射——同步语义是"覆盖"而非"新建"）。
///
/// 原子性：单事务；覆盖前 db.rs 的快照机制 + 本函数外层的调用先决条件
/// 保证可回滚（迁移快照在启动时已有，此处依赖 SQLite 事务）。
async fn apply_snapshot(
    state: &State<'_, AppState>,
    project_id: Uuid,
    snapshot: &BackupFile,
) -> Result<(), miyo_core::AppError> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(miyo_core::AppError::Database)?;

    // 先 upsert 团队（项目 team_id 外键）。
    if let Some(team) = &snapshot.team {
        repo::save_team(tx.as_mut(), team).await?;
    }

    // 覆盖项目本体（保留本地 id，字段以远端为准；team_id 缺失时保留本地归属）。
    let mut project = snapshot.project.clone();
    project.id = project_id;
    if project.team_id.is_none() {
        if let Ok(local) = repo::get_project(&state.db, project_id).await {
            project.team_id = local.team_id;
        }
    }
    repo::save_project(tx.as_mut(), &project).await?;

    // 重映射子对象 project_id 到本地项目 id。
    let remap = |mut f: miyo_core::model::Folder| {
        f.project_id = project_id;
        f
    };
    let folders: Vec<_> = snapshot.folders.iter().cloned().map(remap).collect();
    let endpoints: Vec<_> = {
        let mut list: Vec<_> = snapshot.endpoints.clone();
        for e in &mut list {
            e.project_id = project_id;
        }
        list
    };
    let environments: Vec<_> = {
        let mut list: Vec<_> = snapshot.environments.clone();
        for env in &mut list {
            env.project_id = project_id;
        }
        list
    };
    let mock_rules: Vec<_> = {
        let mut list: Vec<_> = snapshot.mock_rules.clone();
        for r in &mut list {
            r.project_id = project_id;
        }
        list
    };
    // 场景步骤是接口引用式：同步覆盖保留接口 id，endpoint_id 无需重映射。
    let scenarios: Vec<_> = {
        let mut list: Vec<_> = snapshot.scenarios.clone();
        for s in &mut list {
            s.project_id = project_id;
        }
        list
    };

    // 清空旧子对象（CASCADE 不走——这里显式删除保顺序）。
    repo::delete_project_children(tx.as_mut(), project_id).await?;

    repo::save_folders_bulk(tx.as_mut(), &folders).await?;
    repo::save_endpoints_bulk(tx.as_mut(), &endpoints).await?;
    repo::save_environments_bulk(tx.as_mut(), &environments).await?;
    repo::save_mock_rules_bulk(tx.as_mut(), &mock_rules).await?;
    repo::save_response_examples_bulk(tx.as_mut(), &snapshot.response_examples).await?;
    repo::save_request_examples_bulk(tx.as_mut(), &snapshot.request_examples).await?;
    repo::save_proto_files_bulk(tx.as_mut(), &snapshot.proto_files).await?;
    repo::save_scenarios_bulk(tx.as_mut(), &scenarios).await?;

    tx.commit().await.map_err(miyo_core::AppError::Database)?;
    Ok(())
}

// ---------- 远端项目清单 / 导入（首页入口：发现并导入远端新增项目） ----------

/// 列出远端仓库内的项目快照清单，附带「本地是否已存在」标记。
#[tauri::command(rename_all = "camelCase")]
pub async fn sync_remote_list_projects(
    state: State<'_, AppState>,
    remote_id: Uuid,
) -> CommandResult<Vec<serde_json::Value>> {
    let remote = repo::get_sync_remote(&state.db, remote_id).await?;
    // list_projects 只扫描 path 的父目录，文件路径用不到——借 nil uuid 占位。
    let transport = build_transport(&remote, &snapshot_path(&remote, Uuid::nil()))?;
    let entries = transport
        .list_projects()
        .await
        .map_err(|e| miyo_core::validation(e.to_string()))?;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let exists = match Uuid::parse_str(&e.project_id) {
            Ok(id) => repo::get_project(&state.db, id).await.is_ok(),
            Err(_) => false,
        };
        out.push(serde_json::json!({
            "project_id": e.project_id,
            "name": e.name,
            "endpoint_count": e.endpoint_count,
            "updated_at": e.updated_at,
            "exists_locally": exists,
        }));
    }
    Ok(out)
}

/// 导入远端项目快照为本地新项目（远端有、本地无的场景）。
///
/// - 沿用远端 project_id（后续推/拉与远端文件对齐）；
/// - 归属绑定到该远端所在的本地团队（远端携带的团队元数据不落库）；
/// - 本地已存在同 id 项目时拒绝（进入项目内「拉取」覆盖即可）。
#[tauri::command(rename_all = "camelCase")]
pub async fn sync_import_project(
    state: State<'_, AppState>,
    remote_id: Uuid,
    project_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let remote = repo::get_sync_remote(&state.db, remote_id).await?;
    let file_path = snapshot_path(&remote, project_id);
    let transport = build_transport(&remote, &file_path)?;
    let engine = SyncEngine::new(transport.as_ref());
    let preview = engine
        .preview_pull(None)
        .await
        .map_err(|e| miyo_core::validation(e.to_string()))?;
    let mut incoming = preview.remote_snapshot;

    if repo::get_project(&state.db, project_id).await.is_ok() {
        return Err(miyo_core::validation("该项目已存在于本地").into());
    }

    // 绑定本地团队；远端团队元数据（发送方 UUID/名称）不落库，防覆盖本地团队。
    incoming.project.id = project_id;
    incoming.project.team_id = Some(remote.team_id);
    incoming.team = None;
    apply_snapshot(&state, project_id, &incoming).await?;

    Ok(serde_json::json!({
        "projectId": project_id,
        "name": incoming.project.name,
        "endpoints": incoming.endpoints.len(),
    }))
}

fn preview_json(preview: &PullPreview) -> serde_json::Value {
    serde_json::json!({
        "revision": preview.revision,
        "diff": {
            "added": preview.diff.added,
            "removed": preview.diff.removed,
            "modified": preview.diff.modified,
        },
        "snapshot_summary": {
            "project": preview.remote_snapshot.project.name,
            "endpoints": preview.remote_snapshot.endpoints.len(),
            "folders": preview.remote_snapshot.folders.len(),
            "environments": preview.remote_snapshot.environments.len(),
            "exported_at": preview.remote_snapshot.exported_at,
        },
    })
}

fn utc_now_short() -> String {
    chrono::Utc::now().format("%m-%d %H:%M").to_string()
}
