//! ResponseExample（响应示例）。

use crate::db::db_err;
use std::collections::HashMap;

use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use miyo_core::model::ResponseExample;
use miyo_core::Result;

use super::rows::ResponseExampleRow;

/// 新建或按 id 覆盖写入响应示例（upsert：同 id 重复保存时更新而非主键冲突）。
pub async fn create_response_example<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    endpoint_id: Uuid,
    example: &ResponseExample,
) -> Result<ResponseExample> {
    let row = ResponseExampleRow::from_model(example);
    sqlx::query(
        "INSERT INTO response_examples
             (id, endpoint_id, name, status, headers_json, body, content_type, docs_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            endpoint_id = excluded.endpoint_id,
            name = excluded.name,
            status = excluded.status,
            headers_json = excluded.headers_json,
            body = excluded.body,
            content_type = excluded.content_type,
            docs_json = excluded.docs_json,
            updated_at = excluded.updated_at",
    )
    .bind(&row.id)
    .bind(&row.endpoint_id)
    .bind(&row.name)
    .bind(row.status)
    .bind(row.headers_json.clone())
    .bind(&row.body)
    .bind(&row.content_type)
    .bind(row.docs_json.clone())
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(executor)
    .await.map_err(db_err)? ;
    let _ = endpoint_id;
    Ok(example.clone())
}

pub async fn list_response_examples(
    db: &SqlitePool,
    endpoint_id: Uuid,
) -> Result<Vec<ResponseExample>> {
    let rows: Vec<ResponseExampleRow> = sqlx::query_as(
        "SELECT id, endpoint_id, name, status, headers_json, body, content_type, docs_json, created_at, updated_at
         FROM response_examples WHERE endpoint_id = ? ORDER BY created_at",
    )
    .bind(endpoint_id.to_string())
    .fetch_all(db)
    .await.map_err(db_err)? ;
    rows.into_iter()
        .map(ResponseExampleRow::into_model)
        .collect()
}

/// 批量列出多个接口的响应示例（一次查询按 endpoint 分组；导出/备份/Mock 去 N+1 用）。
///
/// SQLite 变量数上限按 500 分片，避免超长 IN 列表。
pub async fn list_response_examples_by_endpoints(
    db: &SqlitePool,
    endpoint_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<ResponseExample>>> {
    let mut map: HashMap<Uuid, Vec<ResponseExample>> = HashMap::with_capacity(endpoint_ids.len());
    for chunk in endpoint_ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new(
            "SELECT id, endpoint_id, name, status, headers_json, body, content_type, docs_json, created_at, updated_at
             FROM response_examples WHERE endpoint_id IN (",
        );
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(id.to_string());
        }
        separated.push_unseparated(") ORDER BY endpoint_id, created_at");
        let rows: Vec<ResponseExampleRow> =
            qb.build_query_as().fetch_all(db).await.map_err(db_err)?;
        for row in rows {
            let model = row.into_model()?;
            map.entry(model.endpoint_id).or_default().push(model);
        }
    }
    Ok(map)
}

/// 删除单条响应示例（M10 示例管理）。
pub async fn delete_response_example(db: &SqlitePool, example_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM response_examples WHERE id = ?")
        .bind(example_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// 删除某接口的全部响应示例（导入覆盖时使用）。
pub async fn delete_response_examples(db: &SqlitePool, endpoint_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM response_examples WHERE endpoint_id = ?")
        .bind(endpoint_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// 带 id：原样写入响应示例（upsert，同 id 覆盖更新，见 [`create_response_example`]）。
pub async fn save_response_example<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    example: &ResponseExample,
) -> Result<()> {
    create_response_example(executor, example.endpoint_id, example)
        .await
        .map(|_| ())
}

/// 批量写入响应示例（备份恢复用）：事务内每 200 行一条多值 INSERT，
/// 语义与逐条 [`save_response_example`] 的 upsert 一致。
pub async fn save_response_examples_bulk(
    conn: &mut sqlx::SqliteConnection,
    examples: &[ResponseExample],
) -> Result<()> {
    if examples.is_empty() {
        return Ok(());
    }
    let rows: Vec<ResponseExampleRow> = examples
        .iter()
        .map(ResponseExampleRow::from_model)
        .collect();
    for chunk in rows.chunks(200) {
        let mut qb = QueryBuilder::new(
            "INSERT INTO response_examples (id, endpoint_id, name, status, headers_json, body, content_type, docs_json, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.endpoint_id)
                .push_bind(&row.name)
                .push_bind(row.status)
                .push_bind(&row.headers_json)
                .push_bind(&row.body)
                .push_bind(&row.content_type)
                .push_bind(&row.docs_json)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.push(
            " ON CONFLICT(id) DO UPDATE SET
                endpoint_id = excluded.endpoint_id,
                name = excluded.name,
                status = excluded.status,
                headers_json = excluded.headers_json,
                body = excluded.body,
                content_type = excluded.content_type,
                docs_json = excluded.docs_json,
                updated_at = excluded.updated_at",
        );
        qb.build().execute(&mut *conn).await.map_err(db_err)?;
    }
    Ok(())
}
