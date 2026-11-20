//! Team CRUD（现阶段：项目的容器，schema 预留成员/权限扩展）。

use crate::db::db_err;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::Team;
use miyo_core::{AppError, Result};

#[derive(sqlx::FromRow)]
struct TeamRow {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
}

impl TeamRow {
    fn into_model(self) -> Result<Team> {
        Ok(Team {
            id: super::rows::parse_uuid(&self.id)?,
            name: self.name,
            created_at: super::rows::parse_time(&self.created_at)?,
            updated_at: super::rows::parse_time(&self.updated_at)?,
        })
    }
}

pub async fn create_team(db: &SqlitePool, name: &str) -> Result<Team> {
    let now = Utc::now();
    let model = Team {
        id: Uuid::new_v4(),
        name: name.to_string(),
        created_at: now,
        updated_at: now,
    };
    sqlx::query("INSERT INTO teams (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(model.id.to_string())
        .bind(&model.name)
        .bind(model.created_at.to_rfc3339())
        .bind(model.updated_at.to_rfc3339())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(model)
}

pub async fn list_teams(db: &SqlitePool) -> Result<Vec<Team>> {
    let rows: Vec<TeamRow> = sqlx::query_as(
        "SELECT id, name, created_at, updated_at FROM teams ORDER BY created_at ASC",
    )
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(TeamRow::into_model).collect()
}

pub async fn get_team(db: &SqlitePool, team_id: Uuid) -> Result<Team> {
    let row: Option<TeamRow> =
        sqlx::query_as("SELECT id, name, created_at, updated_at FROM teams WHERE id = ?")
            .bind(team_id.to_string())
            .fetch_optional(db)
            .await
            .map_err(db_err)?;
    row.map(TeamRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("团队（{team_id}）")))
}

/// 备份恢复：按给定 id 原样落库团队（upsert）。
pub async fn save_team<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    team: &Team,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO teams (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, updated_at = excluded.updated_at",
    )
    .bind(team.id.to_string())
    .bind(&team.name)
    .bind(team.created_at.to_rfc3339())
    .bind(team.updated_at.to_rfc3339())
    .execute(executor)
    .await
    .map_err(db_err)?;
    Ok(())
}

pub async fn update_team(db: &SqlitePool, team: &Team) -> Result<Team> {
    let mut updated = team.clone();
    updated.updated_at = Utc::now();
    let result = sqlx::query("UPDATE teams SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&updated.name)
        .bind(updated.updated_at.to_rfc3339())
        .bind(updated.id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("团队（{}）", team.id)));
    }
    Ok(updated)
}

/// 删除团队：有下属项目时拒绝（先迁移项目到其他团队）。
pub async fn delete_team(db: &SqlitePool, team_id: Uuid) -> Result<()> {
    let project_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE team_id = ?")
        .bind(team_id.to_string())
        .fetch_one(db)
        .await
        .map_err(db_err)?;
    if project_count > 0 {
        return Err(AppError::Validation(format!(
            "团队下仍有 {project_count} 个项目，请先迁移或删除"
        )));
    }
    let result = sqlx::query("DELETE FROM teams WHERE id = ?")
        .bind(team_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("团队（{team_id}）")));
    }
    Ok(())
}

/// 项目移入团队（团队须存在）。
pub async fn move_project_to_team(db: &SqlitePool, project_id: Uuid, team_id: Uuid) -> Result<()> {
    get_team(db, team_id).await?;
    let result = sqlx::query("UPDATE projects SET team_id = ? WHERE id = ?")
        .bind(team_id.to_string())
        .bind(project_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("项目（{project_id}）")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;
    use crate::repository::projects::{create_project, list_projects};

    #[tokio::test]
    async fn team_crud_and_project_move() {
        let db = memory_pool().await.expect("建库");
        let team = create_team(&db, "支付组").await.unwrap();
        let project = create_project(&db, "网关", "").await.unwrap();

        move_project_to_team(&db, project.id, team.id)
            .await
            .unwrap();
        let projects = list_projects(&db).await.unwrap();
        assert_eq!(projects[0].team_id, Some(team.id));

        // 有下属项目时不可删除。
        assert!(delete_team(&db, team.id).await.is_err());
        // 迁走后可删除。
        let default = crate::repository::teams::list_teams(&db)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.id == miyo_core::model::DEFAULT_TEAM_ID)
            .expect("默认团队存在");
        move_project_to_team(&db, project.id, default.id)
            .await
            .unwrap();
        delete_team(&db, team.id).await.unwrap();
        assert!(get_team(&db, team.id).await.is_err());
    }
}
