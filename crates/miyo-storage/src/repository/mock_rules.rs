//! Mock Rules（自定义 Mock 规则）。

use crate::db::db_err;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::MockRule;
use miyo_core::{AppError, Result};

use super::rows::MockRuleRow;

pub async fn create_mock_rule<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    project_id: Uuid,
    rule: &MockRule,
) -> Result<MockRule> {
    let row = MockRuleRow::from_model(rule);
    sqlx::query(
        "INSERT INTO mock_rules
         (id, project_id, endpoint_id, name, method, path, match_query_json, match_headers_json,
          response_status, response_headers_json, response_body_template, delay_ms,
          fault_rate_pct, fault_status, enabled, priority,
          created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            project_id = excluded.project_id,
            endpoint_id = excluded.endpoint_id,
            name = excluded.name,
            method = excluded.method,
            path = excluded.path,
            match_query_json = excluded.match_query_json,
            match_headers_json = excluded.match_headers_json,
            response_status = excluded.response_status,
            response_headers_json = excluded.response_headers_json,
            response_body_template = excluded.response_body_template,
            delay_ms = excluded.delay_ms,
            fault_rate_pct = excluded.fault_rate_pct,
            fault_status = excluded.fault_status,
            enabled = excluded.enabled,
            priority = excluded.priority,
            updated_at = excluded.updated_at",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.endpoint_id)
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.match_query_json)
    .bind(&row.match_headers_json)
    .bind(row.response_status)
    .bind(&row.response_headers_json)
    .bind(&row.response_body_template)
    .bind(row.delay_ms)
    .bind(row.fault_rate_pct)
    .bind(row.fault_status)
    .bind(row.enabled)
    .bind(row.priority)
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .execute(executor)
    .await
    .map_err(db_err)?;
    let _ = project_id;
    Ok(rule.clone())
}

/// 批量写入 Mock 规则（备份恢复用）：事务内每 200 行一条多值 INSERT，
/// 语义与逐条 [`create_mock_rule`] 的 upsert 一致（同 id 覆盖更新）。
pub async fn save_mock_rules_bulk(
    conn: &mut sqlx::SqliteConnection,
    rules: &[MockRule],
) -> Result<()> {
    if rules.is_empty() {
        return Ok(());
    }
    let rows: Vec<MockRuleRow> = rules.iter().map(MockRuleRow::from_model).collect();
    for chunk in rows.chunks(200) {
        let mut qb = sqlx::QueryBuilder::new(
            "INSERT INTO mock_rules (id, project_id, endpoint_id, name, method, path, match_query_json, match_headers_json,
                 response_status, response_headers_json, response_body_template, delay_ms,
                 fault_rate_pct, fault_status, enabled, priority, created_at, updated_at) ",
        );
        qb.push_values(chunk, |mut b, row| {
            b.push_bind(&row.id)
                .push_bind(&row.project_id)
                .push_bind(&row.endpoint_id)
                .push_bind(&row.name)
                .push_bind(&row.method)
                .push_bind(&row.path)
                .push_bind(&row.match_query_json)
                .push_bind(&row.match_headers_json)
                .push_bind(row.response_status)
                .push_bind(&row.response_headers_json)
                .push_bind(&row.response_body_template)
                .push_bind(row.delay_ms)
                .push_bind(row.fault_rate_pct)
                .push_bind(row.fault_status)
                .push_bind(row.enabled)
                .push_bind(row.priority)
                .push_bind(&row.created_at)
                .push_bind(&row.updated_at);
        });
        qb.push(
            " ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                endpoint_id = excluded.endpoint_id,
                name = excluded.name,
                method = excluded.method,
                path = excluded.path,
                match_query_json = excluded.match_query_json,
                match_headers_json = excluded.match_headers_json,
                response_status = excluded.response_status,
                response_headers_json = excluded.response_headers_json,
                response_body_template = excluded.response_body_template,
                delay_ms = excluded.delay_ms,
                fault_rate_pct = excluded.fault_rate_pct,
                fault_status = excluded.fault_status,
                enabled = excluded.enabled,
                priority = excluded.priority,
                updated_at = excluded.updated_at",
        );
        qb.build().execute(&mut *conn).await.map_err(db_err)?;
    }
    Ok(())
}

pub async fn list_mock_rules(db: &SqlitePool, project_id: Uuid) -> Result<Vec<MockRule>> {
    let rows: Vec<MockRuleRow> = sqlx::query_as(
        "SELECT id, project_id, endpoint_id, name, method, path, match_query_json, match_headers_json,
                response_status, response_headers_json, response_body_template, delay_ms,
                fault_rate_pct, fault_status, enabled, priority,
                created_at, updated_at
         FROM mock_rules WHERE project_id = ? ORDER BY priority DESC, created_at",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await.map_err(db_err)? ;
    rows.into_iter().map(MockRuleRow::into_model).collect()
}

pub async fn update_mock_rule(db: &SqlitePool, rule: &MockRule) -> Result<MockRule> {
    let row = MockRuleRow::from_model(rule);
    let result = sqlx::query(
        "UPDATE mock_rules SET name = ?, method = ?, path = ?, match_query_json = ?, match_headers_json = ?,
                response_status = ?, response_headers_json = ?, response_body_template = ?, delay_ms = ?,
                fault_rate_pct = ?, fault_status = ?,
                enabled = ?, priority = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&row.name)
    .bind(&row.method)
    .bind(&row.path)
    .bind(&row.match_query_json)
    .bind(&row.match_headers_json)
    .bind(row.response_status)
    .bind(&row.response_headers_json)
    .bind(&row.response_body_template)
    .bind(row.delay_ms)
    .bind(row.fault_rate_pct)
    .bind(row.fault_status)
    .bind(row.enabled)
    .bind(row.priority)
    .bind(row.updated_at.clone())
    .bind(&row.id)
    .execute(db)
    .await.map_err(db_err)? ;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Mock 规则（{}）", rule.id)));
    }
    Ok(rule.clone())
}

pub async fn delete_mock_rule(db: &SqlitePool, rule_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM mock_rules WHERE id = ?")
        .bind(rule_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// 带 id：原样写入 Mock 规则。
pub async fn save_mock_rule<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    rule: &MockRule,
) -> Result<()> {
    create_mock_rule(executor, rule.project_id, rule)
        .await
        .map(|_| ())
}
