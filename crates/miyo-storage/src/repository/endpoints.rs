//! Endpoint CRUD。

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, RequestSpec};
use miyo_core::{AppError, Result};

use super::rows::EndpointRow;

/// 触摸父项目更新时间：接口增删改视为项目活跃（仪表板最近活动排序 / 卡片更新时间）。
pub async fn touch_project(db: &SqlitePool, project_id: &Uuid) -> Result<()> {
    sqlx::query("UPDATE projects SET updated_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(project_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn create_endpoint(
    db: &SqlitePool,
    project_id: Uuid,
    folder_id: Option<Uuid>,
    name: &str,
) -> Result<Endpoint> {
    let now = Utc::now();
    let model = Endpoint {
        id: Uuid::new_v4(),
        project_id,
        folder_id,
        name: name.to_string(),
        method: HttpMethod::GET,
        path: "/".to_string(),
        description: String::new(),
        status: EndpointStatus::Developing,
        sort_order: 0,
        request: RequestSpec::default(),
        created_at: now,
        updated_at: now,
    };
    let row = EndpointRow::from_model(&model)?;
    sqlx::query(
        "INSERT INTO endpoints
         (id, project_id, folder_id, name, method, path, description, status, sort_order, request_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.folder_id)
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.description)
    .bind(&row.status)
    .bind(row.sort_order)
    .bind(&row.request_json)
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(db)
    .await?;
    touch_project(db, &model.project_id).await?;
    Ok(model)
}

pub async fn get_endpoint(db: &SqlitePool, endpoint_id: Uuid) -> Result<Endpoint> {
    let row: Option<EndpointRow> = sqlx::query_as(
        "SELECT id, project_id, folder_id, name, method, path, description, status,
                sort_order, request_json, created_at, updated_at
         FROM endpoints WHERE id = ?",
    )
    .bind(endpoint_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(EndpointRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("接口（{endpoint_id}）")))
}

pub async fn update_endpoint(db: &SqlitePool, endpoint: &Endpoint) -> Result<Endpoint> {
    let mut updated = endpoint.clone();
    updated.updated_at = Utc::now();
    let row = EndpointRow::from_model(&updated)?;
    let result = sqlx::query(
        "UPDATE endpoints SET folder_id = ?, name = ?, method = ?, path = ?, description = ?,
                status = ?, sort_order = ?, request_json = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&row.folder_id)
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.description)
    .bind(&row.status)
    .bind(row.sort_order)
    .bind(&row.request_json)
    .bind(row.updated_at.clone())
    .bind(&row.id)
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("接口（{}）", endpoint.id)));
    }
    touch_project(db, &updated.project_id).await?;
    Ok(updated)
}

pub async fn delete_endpoint(db: &SqlitePool, endpoint_id: Uuid) -> Result<()> {
    // 先触摸（删除后即查不到归属项目）
    sqlx::query(
        "UPDATE projects SET updated_at = ? WHERE id = (SELECT project_id FROM endpoints WHERE id = ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(endpoint_id.to_string())
    .execute(db)
    .await?;
    sqlx::query("DELETE FROM endpoints WHERE id = ?")
        .bind(endpoint_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn duplicate_endpoint(db: &SqlitePool, endpoint_id: Uuid) -> Result<Endpoint> {
    let source = get_endpoint(db, endpoint_id).await?;
    let now = Utc::now();
    let duplicate = Endpoint {
        id: Uuid::new_v4(),
        project_id: source.project_id,
        folder_id: source.folder_id,
        name: format!("{}（副本）", source.name),
        method: source.method,
        path: source.path,
        description: source.description,
        status: source.status,
        sort_order: source.sort_order + 1,
        request: source.request,
        created_at: now,
        updated_at: now,
    };
    let row = EndpointRow::from_model(&duplicate)?;
    sqlx::query(
        "INSERT INTO endpoints
         (id, project_id, folder_id, name, method, path, description, status, sort_order, request_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.folder_id)
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.description)
    .bind(&row.status)
    .bind(row.sort_order)
    .bind(&row.request_json)
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(db)
    .await?;
    touch_project(db, &duplicate.project_id).await?;
    Ok(duplicate)
}

pub async fn list_endpoints(db: &SqlitePool, project_id: Uuid) -> Result<Vec<Endpoint>> {
    let rows: Vec<EndpointRow> = sqlx::query_as(
        "SELECT id, project_id, folder_id, name, method, path, description, status,
                sort_order, request_json, created_at, updated_at
         FROM endpoints WHERE project_id = ? ORDER BY sort_order, created_at",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(EndpointRow::into_model).collect()
}

/// 带 id：原样写入接口（upsert，同一 id 重复保存时更新而非报主键冲突）。
pub async fn save_endpoint<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    endpoint: &Endpoint,
) -> Result<()> {
    let row = EndpointRow::from_model(endpoint)?;
    sqlx::query(
        "INSERT INTO endpoints (id, project_id, folder_id, name, method, path, description, status, sort_order, request_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            project_id = excluded.project_id,
            folder_id = excluded.folder_id,
            name = excluded.name,
            method = excluded.method,
            path = excluded.path,
            description = excluded.description,
            status = excluded.status,
            sort_order = excluded.sort_order,
            request_json = excluded.request_json,
            updated_at = excluded.updated_at",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.folder_id)
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.description)
    .bind(&row.status)
    .bind(row.sort_order)
    .bind(&row.request_json)
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(executor)
    .await?;
    Ok(())
}

/// 批量写入接口（备份恢复用）：事务内每 200 行一条多值 INSERT，
/// 语义与逐条 [`save_endpoint`] 的 upsert 一致（同 id 覆盖更新，
/// 含 `app_secret` 加密，见 [`EndpointRow::from_model`]）。
pub async fn save_endpoints_bulk(
    conn: &mut sqlx::SqliteConnection,
    endpoints: &[Endpoint],
) -> Result<()> {
    if endpoints.is_empty() {
        return Ok(());
    }
    // 逐条加密（auth 密钥字段），任一失败整体失败（不静默降级明文）。
    let rows: Vec<EndpointRow> = endpoints
        .iter()
        .map(EndpointRow::from_model)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for chunk in rows.chunks(200) {
        let mut qb = sqlx::QueryBuilder::new(
            "INSERT INTO endpoints (id, project_id, folder_id, name, method, path, description, status, sort_order, request_json, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.project_id)
                .push_bind(&row.folder_id)
                .push_bind(&row.name)
                .push_bind(&row.method)
                .push_bind(&row.path)
                .push_bind(&row.description)
                .push_bind(&row.status)
                .push_bind(row.sort_order)
                .push_bind(&row.request_json)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.push(
            " ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                folder_id = excluded.folder_id,
                name = excluded.name,
                method = excluded.method,
                path = excluded.path,
                description = excluded.description,
                status = excluded.status,
                sort_order = excluded.sort_order,
                request_json = excluded.request_json,
                updated_at = excluded.updated_at",
        );
        qb.build().execute(&mut *conn).await?;
    }
    Ok(())
}
