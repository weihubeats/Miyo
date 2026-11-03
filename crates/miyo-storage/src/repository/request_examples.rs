//! RequestExample（请求用例）仓储：新建 / 列表 / 删除。

use std::collections::HashMap;

use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use miyo_core::model::RequestExample;
use miyo_core::Result;

/// 新建请求用例（保存当前请求快照）。
pub async fn create_request_example<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    example: &RequestExample,
) -> Result<RequestExample> {
    sqlx::query(
        "INSERT INTO request_examples
             (id, endpoint_id, name, request_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(example.id.to_string())
    .bind(example.endpoint_id.to_string())
    .bind(&example.name)
    // auth 密钥字段随快照加密（与 endpoints 同通道，见 rows::encrypt_request_secrets）。
    .bind(super::rows::encrypt_request_secrets(
        &serde_json::to_string(&example.request).map_err(miyo_core::AppError::Json)?,
    )?)
    .bind(example.created_at.to_rfc3339())
    .bind(example.updated_at.to_rfc3339())
    .execute(executor)
    .await?;
    Ok(example.clone())
}

/// 列出接口的全部请求用例（最新在前）。
pub async fn list_request_examples(
    db: &SqlitePool,
    endpoint_id: Uuid,
) -> Result<Vec<RequestExample>> {
    let rows: Vec<RequestExampleRow> = sqlx::query_as(
        "SELECT id, endpoint_id, name, request_json, created_at, updated_at
         FROM request_examples WHERE endpoint_id = ? ORDER BY created_at DESC",
    )
    .bind(endpoint_id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(RequestExampleRow::into_model)
        .collect()
}

/// 批量列出多个接口的请求用例（一次查询按 endpoint 分组；备份去 N+1 用）。
pub async fn list_request_examples_by_endpoints(
    db: &SqlitePool,
    endpoint_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<RequestExample>>> {
    let mut map: HashMap<Uuid, Vec<RequestExample>> = HashMap::with_capacity(endpoint_ids.len());
    for chunk in endpoint_ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new(
            "SELECT id, endpoint_id, name, request_json, created_at, updated_at
             FROM request_examples WHERE endpoint_id IN (",
        );
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(id.to_string());
        }
        separated.push_unseparated(") ORDER BY endpoint_id, created_at DESC");
        let rows: Vec<RequestExampleRow> = qb.build_query_as().fetch_all(db).await?;
        for row in rows {
            let model = row.into_model()?;
            map.entry(model.endpoint_id).or_default().push(model);
        }
    }
    Ok(map)
}

/// 删除单条请求用例。
pub async fn delete_request_example(db: &SqlitePool, example_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM request_examples WHERE id = ?")
        .bind(example_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 批量写入请求用例（备份恢复用）：事务内每 200 行一条多值 INSERT，
/// 与逐条 [`create_request_example`] 语义一致（普通 INSERT，不做 upsert）。
pub async fn save_request_examples_bulk(
    conn: &mut sqlx::SqliteConnection,
    examples: &[RequestExample],
) -> Result<()> {
    if examples.is_empty() {
        return Ok(());
    }
    let rows: Vec<RequestExampleRow> = examples
        .iter()
        .map(|example| {
            Ok(RequestExampleRow {
                id: example.id.to_string(),
                endpoint_id: example.endpoint_id.to_string(),
                name: example.name.clone(),
                request_json: super::rows::encrypt_request_secrets(
                    &serde_json::to_string(&example.request).map_err(miyo_core::AppError::Json)?,
                )?,
                created_at: example.created_at.to_rfc3339(),
                updated_at: example.updated_at.to_rfc3339(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for chunk in rows.chunks(200) {
        let mut qb = QueryBuilder::new(
            "INSERT INTO request_examples (id, endpoint_id, name, request_json, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.endpoint_id)
                .push_bind(&row.name)
                .push_bind(&row.request_json)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.build().execute(&mut *conn).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RequestExampleRow {
    id: String,
    endpoint_id: String,
    name: String,
    request_json: String,
    created_at: String,
    updated_at: String,
}

impl RequestExampleRow {
    fn into_model(self) -> Result<RequestExample> {
        Ok(RequestExample {
            id: super::rows::parse_uuid(&self.id)?,
            endpoint_id: super::rows::parse_uuid(&self.endpoint_id)?,
            name: self.name,
            request: serde_json::from_str(&super::rows::decrypt_request_secrets(
                &self.request_json,
            )?)
            .map_err(miyo_core::AppError::Json)?,
            created_at: super::rows::parse_time(&self.created_at)?,
            updated_at: super::rows::parse_time(&self.updated_at)?,
        })
    }
}
