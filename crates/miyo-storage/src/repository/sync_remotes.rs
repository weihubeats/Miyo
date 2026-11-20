//! SyncRemote CRUD：团队绑定的同步远端（git / S3 / db 快照仓库）。
//!
//! 团队维度：一个团队绑定远端路径（不同团队可复用同一路径）；
//! 推/拉按项目触发，仓库内按 `projects/{project_id}.json` 分文件存快照。

use crate::db::db_err;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{SyncRemote, SyncRemoteKind};
use miyo_core::{AppError, Result};

use super::rows::parse_time;
use super::rows::parse_uuid;

#[derive(sqlx::FromRow)]
struct SyncRemoteRow {
    id: String,
    team_id: String,
    kind: String,
    config_json: String,
    last_synced_revision: Option<String>,
    created_at: String,
    updated_at: String,
}

impl SyncRemoteRow {
    fn into_model(self) -> Result<SyncRemote> {
        Ok(SyncRemote {
            id: parse_uuid(&self.id)?,
            team_id: parse_uuid(&self.team_id)?,
            kind: match self.kind.as_str() {
                "git" => SyncRemoteKind::Git,
                "s3" => SyncRemoteKind::S3,
                "db" => SyncRemoteKind::Db,
                other => {
                    return Err(AppError::Validation(format!("未知同步远端类型：{other}")));
                }
            },
            config: serde_json::from_str(&self.config_json).map_err(AppError::Json)?,
            last_synced_revision: self.last_synced_revision,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

pub async fn create_sync_remote(
    db: &SqlitePool,
    team_id: Uuid,
    kind: SyncRemoteKind,
    config: &serde_json::Value,
) -> Result<SyncRemote> {
    // 团队须存在（默认团队由迁移 0015 保证）。
    super::teams::get_team(db, team_id).await?;
    let now = Utc::now();
    let model = SyncRemote {
        id: Uuid::new_v4(),
        team_id,
        kind,
        config: config.clone(),
        last_synced_revision: None,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO sync_remotes (id, team_id, kind, config_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(model.id.to_string())
    .bind(model.team_id.to_string())
    .bind(kind_str(kind))
    .bind(serde_json::to_string(&model.config).map_err(AppError::Json)?)
    .bind(model.created_at.to_rfc3339())
    .bind(model.updated_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(model)
}

/// 团队的同步远端列表（通常一个）。
pub async fn list_sync_remotes(db: &SqlitePool, team_id: Uuid) -> Result<Vec<SyncRemote>> {
    let rows: Vec<SyncRemoteRow> = sqlx::query_as(
        "SELECT id, team_id, kind, config_json, last_synced_revision, created_at, updated_at
         FROM sync_remotes WHERE team_id = ? ORDER BY created_at ASC",
    )
    .bind(team_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(SyncRemoteRow::into_model).collect()
}

pub async fn get_sync_remote(db: &SqlitePool, remote_id: Uuid) -> Result<SyncRemote> {
    let row: Option<SyncRemoteRow> = sqlx::query_as(
        "SELECT id, team_id, kind, config_json, last_synced_revision, created_at, updated_at
         FROM sync_remotes WHERE id = ?",
    )
    .bind(remote_id.to_string())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    row.map(SyncRemoteRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("同步远端（{remote_id}）")))
}

/// 记录同步完成的远端 revision。
pub async fn set_last_synced_revision(
    db: &SqlitePool,
    remote_id: Uuid,
    revision: &str,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE sync_remotes SET last_synced_revision = ?, updated_at = ? WHERE id = ?",
    )
    .bind(revision)
    .bind(Utc::now().to_rfc3339())
    .bind(remote_id.to_string())
    .execute(db)
    .await
    .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("同步远端（{remote_id}）")));
    }
    Ok(())
}

pub async fn delete_sync_remote(db: &SqlitePool, remote_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM sync_remotes WHERE id = ?")
        .bind(remote_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

fn kind_str(kind: SyncRemoteKind) -> &'static str {
    match kind {
        SyncRemoteKind::Git => "git",
        SyncRemoteKind::S3 => "s3",
        SyncRemoteKind::Db => "db",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;

    #[tokio::test]
    async fn sync_remote_crud_and_revision_tracking() {
        let db = memory_pool().await.expect("建库");
        // 默认团队（迁移 0015）。
        let team = super::super::teams::list_teams(&db)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.id == miyo_core::model::DEFAULT_TEAM_ID)
            .expect("默认团队存在");
        let config = serde_json::json!({
            "kind": "git",
            "url": "git@example.com:team/api.git",
            "branch": "main",
            "path": "projects",
        });

        let remote = create_sync_remote(&db, team.id, SyncRemoteKind::Git, &config)
            .await
            .unwrap();
        assert_eq!(remote.last_synced_revision, None);

        let listed = list_sync_remotes(&db, team.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        // config 原样往返（miyo-sync::RemoteConfig 解析的真源）。
        assert_eq!(listed[0].config, config);

        set_last_synced_revision(&db, remote.id, "abc123")
            .await
            .unwrap();
        let got = get_sync_remote(&db, remote.id).await.unwrap();
        assert_eq!(got.last_synced_revision.as_deref(), Some("abc123"));

        delete_sync_remote(&db, remote.id).await.unwrap();
        assert!(list_sync_remotes(&db, team.id).await.unwrap().is_empty());
    }
}
