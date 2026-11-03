//! Environment CRUD（项目维度）。
//!
//! 环境按项目归属（`project_id` 外键，删项目级联删环境），
//! 每个环境持有单一前置 Base URL `base_url` 与一组结构化变量。

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::Environment;
use miyo_core::{AppError, Result};

use super::rows::{decrypt_env_json, EnvironmentRow};

pub async fn create_environment(
    db: &SqlitePool,
    project_id: Uuid,
    name: &str,
    base_url: &str,
    variables: &[miyo_core::model::EnvironmentVariable],
) -> Result<Environment> {
    let now = Utc::now();
    let model = Environment {
        id: Uuid::new_v4(),
        project_id,
        name: name.to_string(),
        base_url: base_url.to_string(),
        variables: variables.to_vec(),
        created_at: now,
        updated_at: now,
    };
    let row = EnvironmentRow::from_model(&model)?;
    sqlx::query(
        "INSERT INTO environments (id, project_id, name, base_url, variables_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.name)
    .bind(&row.base_url)
    .bind(row.variables_json.clone())
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(db)
    .await?;
    Ok(model)
}

pub async fn get_environment(db: &SqlitePool, environment_id: Uuid) -> Result<Environment> {
    let row: Option<EnvironmentRow> = sqlx::query_as(
        "SELECT id, project_id, name, base_url, variables_json, created_at, updated_at
         FROM environments WHERE id = ?",
    )
    .bind(environment_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(EnvironmentRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("环境（{environment_id}）")))
}

pub async fn update_environment(db: &SqlitePool, environment: &Environment) -> Result<Environment> {
    let mut updated = environment.clone();
    updated.updated_at = Utc::now();
    let row = EnvironmentRow::from_model(&updated)?;
    let result = sqlx::query(
        "UPDATE environments SET project_id = ?, name = ?, base_url = ?, variables_json = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&row.project_id)
    .bind(&row.name)
    .bind(&row.base_url)
    .bind(row.variables_json.clone())
    .bind(row.updated_at.clone())
    .bind(&row.id)
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("环境（{}）", environment.id)));
    }
    Ok(updated)
}

pub async fn delete_environment(db: &SqlitePool, environment_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM environments WHERE id = ?")
        .bind(environment_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 列出项目的全部环境（按创建时间排序）。
///
/// 单行密文损坏（主密钥更换 / 数据篡改）不毒化整个列表：
/// 该环境按空变量表返回，前端仍有全局 DECRYPT 去重提示兜底。
pub async fn list_environments(db: &SqlitePool, project_id: Uuid) -> Result<Vec<Environment>> {
    let rows: Vec<EnvironmentRow> = sqlx::query_as(
        "SELECT id, project_id, name, base_url, variables_json, created_at, updated_at
         FROM environments WHERE project_id = ? ORDER BY created_at",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        if decrypt_env_json(&row.variables_json).is_err() {
            tracing::warn!(env = %row.name, "环境变量解密失败，该环境变量表按空处理");
            row.variables_json = "[]".into();
        }
        out.push(row.into_model()?);
    }
    Ok(out)
}

/// 带 id：原样写入环境（upsert，同一 id 重复保存时更新而非报主键冲突）。
pub async fn save_environment(db: &SqlitePool, env: &Environment) -> Result<Environment> {
    let row = EnvironmentRow::from_model(env)?;
    sqlx::query(
        "INSERT INTO environments (id, project_id, name, base_url, variables_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
             project_id = excluded.project_id,
             name = excluded.name,
             base_url = excluded.base_url,
             variables_json = excluded.variables_json,
             updated_at = excluded.updated_at",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.name)
    .bind(&row.base_url)
    .bind(row.variables_json.clone())
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(db)
    .await?;
    Ok(env.clone())
}

/// 批量写入环境（备份恢复用）：事务内每 200 行一条多值 INSERT，
/// 语义与逐条 [`save_environment`] 一致（upsert；变量走 `encrypt_env_json` 加密）。
pub async fn save_environments_bulk(
    conn: &mut sqlx::SqliteConnection,
    environments: &[Environment],
) -> Result<()> {
    if environments.is_empty() {
        return Ok(());
    }
    // 逐条加密（变量整体加密），任一失败整体失败（不静默降级明文）。
    let rows: Vec<EnvironmentRow> = environments
        .iter()
        .map(EnvironmentRow::from_model)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for chunk in rows.chunks(200) {
        let mut qb = sqlx::QueryBuilder::new(
            "INSERT INTO environments (id, project_id, name, base_url, variables_json, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.project_id)
                .push_bind(&row.name)
                .push_bind(&row.base_url)
                .push_bind(&row.variables_json)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.push(
            " ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                name = excluded.name,
                base_url = excluded.base_url,
                variables_json = excluded.variables_json,
                updated_at = excluded.updated_at",
        );
        qb.build().execute(&mut *conn).await?;
    }
    Ok(())
}
