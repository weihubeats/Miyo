//! 测试套件与批量执行（Test Suite / Test Run Batch）持久化。
//!
//! 套件只存场景引用（排序 + 启用），不复制步骤；批次与明细独立落库，
//! 应用异常退出后由 [`recover_interrupted_batches`] 在启动时标记 interrupted。

use crate::db::db_err;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{
    TestBatchItemStatus, TestBatchMode, TestBatchStatus, TestFailurePolicy, TestRunBatch,
    TestRunBatchItem, TestSuite, TestSuiteItem,
};
use miyo_core::{AppError, Result};

use super::rows::{parse_time, parse_uuid};

const SUITE_COLUMNS: &str = "id, project_id, name, description, folder_id, created_at, updated_at";
const SUITE_ITEM_COLUMNS: &str = "id, suite_id, scenario_id, sort_order, enabled";
const BATCH_COLUMNS: &str = "id, project_id, suite_id, parent_batch_id, environment_id, status, \
     mode, concurrency, failure_policy, retry_count, skip_disabled, total_count, passed_count, \
     failed_count, skipped_count, cancelled_count, running_count, pending_count, started_at, \
     finished_at, duration_ms, created_at";
const BATCH_ITEM_COLUMNS: &str = "id, batch_id, scenario_id, scenario_name, status, sort_order, \
     attempt, duration_ms, error_message, execution_record_id, started_at, finished_at";

#[derive(sqlx::FromRow)]
struct TestSuiteRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    folder_id: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TestSuiteRow {
    fn into_model(self) -> Result<TestSuite> {
        Ok(TestSuite {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            name: self.name,
            description: self.description,
            folder_id: self.folder_id.as_deref().map(parse_uuid).transpose()?,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TestSuiteItemRow {
    id: String,
    suite_id: String,
    scenario_id: String,
    sort_order: i64,
    enabled: bool,
}

impl TestSuiteItemRow {
    fn into_model(self) -> Result<TestSuiteItem> {
        Ok(TestSuiteItem {
            id: parse_uuid(&self.id)?,
            suite_id: parse_uuid(&self.suite_id)?,
            scenario_id: parse_uuid(&self.scenario_id)?,
            sort_order: self.sort_order,
            enabled: self.enabled,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TestRunBatchRow {
    id: String,
    project_id: String,
    suite_id: Option<String>,
    parent_batch_id: Option<String>,
    environment_id: Option<String>,
    status: String,
    mode: String,
    concurrency: i64,
    failure_policy: String,
    retry_count: i64,
    skip_disabled: bool,
    total_count: i64,
    passed_count: i64,
    failed_count: i64,
    skipped_count: i64,
    cancelled_count: i64,
    running_count: i64,
    pending_count: i64,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_ms: Option<f64>,
    created_at: String,
}

impl TestRunBatchRow {
    fn into_model(self) -> Result<TestRunBatch> {
        Ok(TestRunBatch {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            suite_id: self.suite_id.as_deref().map(parse_uuid).transpose()?,
            parent_batch_id: self
                .parent_batch_id
                .as_deref()
                .map(parse_uuid)
                .transpose()?,
            environment_id: self.environment_id.as_deref().map(parse_uuid).transpose()?,
            status: TestBatchStatus::parse(&self.status)
                .ok_or_else(|| AppError::Validation(format!("未知批次状态：{}", self.status)))?,
            mode: TestBatchMode::parse(&self.mode)
                .ok_or_else(|| AppError::Validation(format!("未知执行模式：{}", self.mode)))?,
            concurrency: self.concurrency,
            failure_policy: TestFailurePolicy::parse(&self.failure_policy).ok_or_else(|| {
                AppError::Validation(format!("未知失败策略：{}", self.failure_policy))
            })?,
            retry_count: self.retry_count,
            skip_disabled: self.skip_disabled,
            total_count: self.total_count,
            passed_count: self.passed_count,
            failed_count: self.failed_count,
            skipped_count: self.skipped_count,
            cancelled_count: self.cancelled_count,
            running_count: self.running_count,
            pending_count: self.pending_count,
            started_at: self.started_at.as_deref().map(parse_time).transpose()?,
            finished_at: self.finished_at.as_deref().map(parse_time).transpose()?,
            duration_ms: self.duration_ms,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TestRunBatchItemRow {
    id: String,
    batch_id: String,
    scenario_id: String,
    scenario_name: String,
    status: String,
    sort_order: i64,
    attempt: i64,
    duration_ms: Option<f64>,
    error_message: Option<String>,
    execution_record_id: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl TestRunBatchItemRow {
    fn into_model(self) -> Result<TestRunBatchItem> {
        Ok(TestRunBatchItem {
            id: parse_uuid(&self.id)?,
            batch_id: parse_uuid(&self.batch_id)?,
            scenario_id: parse_uuid(&self.scenario_id)?,
            scenario_name: self.scenario_name,
            status: TestBatchItemStatus::parse(&self.status).ok_or_else(|| {
                AppError::Validation(format!("未知批次明细状态：{}", self.status))
            })?,
            sort_order: self.sort_order,
            attempt: self.attempt,
            duration_ms: self.duration_ms,
            error_message: self.error_message,
            execution_record_id: self
                .execution_record_id
                .as_deref()
                .map(parse_uuid)
                .transpose()?,
            started_at: self.started_at.as_deref().map(parse_time).transpose()?,
            finished_at: self.finished_at.as_deref().map(parse_time).transpose()?,
        })
    }
}

// ---------- 套件 ----------

/// 列出项目的全部套件（名称序）。
pub async fn list_test_suites(db: &SqlitePool, project_id: Uuid) -> Result<Vec<TestSuite>> {
    let rows: Vec<TestSuiteRow> = sqlx::query_as(&format!(
        "SELECT {SUITE_COLUMNS} FROM test_suites WHERE project_id = ? ORDER BY name ASC"
    ))
    .bind(project_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(TestSuiteRow::into_model).collect()
}

/// 取单个套件（不存在返回 NotFound）。
pub async fn get_test_suite(db: &SqlitePool, id: Uuid) -> Result<TestSuite> {
    let row: Option<TestSuiteRow> = sqlx::query_as(&format!(
        "SELECT {SUITE_COLUMNS} FROM test_suites WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    row.map(TestSuiteRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("测试套件（{id}）")))
}

/// 套件成员（编排序）。
pub async fn list_test_suite_items(db: &SqlitePool, suite_id: Uuid) -> Result<Vec<TestSuiteItem>> {
    let rows: Vec<TestSuiteItemRow> = sqlx::query_as(&format!(
        "SELECT {SUITE_ITEM_COLUMNS} FROM test_suite_items WHERE suite_id = ? ORDER BY sort_order ASC"
    ))
    .bind(suite_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(TestSuiteItemRow::into_model).collect()
}

/// 项目内全部套件成员（一次查询供列表页组装套件树，避免逐套件 N+1）。
pub async fn list_test_suite_items_for_project(
    db: &SqlitePool,
    project_id: Uuid,
) -> Result<Vec<TestSuiteItem>> {
    let rows: Vec<TestSuiteItemRow> = sqlx::query_as(
        "SELECT i.id, i.suite_id, i.scenario_id, i.sort_order, i.enabled
         FROM test_suite_items i
         JOIN test_suites s ON s.id = i.suite_id
         WHERE s.project_id = ?
         ORDER BY i.sort_order ASC",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(TestSuiteItemRow::into_model).collect()
}

/// 保存套件 + 成员（整体覆盖写；事务内替换成员列表）。
pub async fn save_test_suite(
    db: &SqlitePool,
    suite: &TestSuite,
    items: &[TestSuiteItem],
) -> Result<()> {
    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO test_suites (id, project_id, name, description, folder_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           description = excluded.description,
           folder_id = excluded.folder_id,
           updated_at = excluded.updated_at",
    )
    .bind(suite.id.to_string())
    .bind(suite.project_id.to_string())
    .bind(&suite.name)
    .bind(&suite.description)
    .bind(suite.folder_id.map(|f| f.to_string()))
    .bind(suite.created_at.to_rfc3339())
    .bind(suite.updated_at.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query("DELETE FROM test_suite_items WHERE suite_id = ?")
        .bind(suite.id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    for (idx, item) in items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO test_suite_items (id, suite_id, scenario_id, sort_order, enabled)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(suite.id.to_string())
        .bind(item.scenario_id.to_string())
        .bind(idx as i64)
        .bind(item.enabled)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// 删除套件（成员随外键 / 显式删除清理；批次历史保留）。
pub async fn delete_test_suite(db: &SqlitePool, id: Uuid) -> Result<()> {
    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM test_suite_items WHERE suite_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let result = sqlx::query("DELETE FROM test_suites WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("测试套件（{id}）")));
    }
    Ok(())
}

/// 复制套件（新 id / 新成员 id；名称追加「副本」，重名时追加序号）。
pub async fn duplicate_test_suite(db: &SqlitePool, id: Uuid) -> Result<TestSuite> {
    let source = get_test_suite(db, id).await?;
    let items = list_test_suite_items(db, id).await?;
    let now = chrono::Utc::now();
    let existing = list_test_suites(db, source.project_id).await?;
    let mut name = format!("{} 副本", source.name);
    let mut n = 2;
    while existing.iter().any(|s| s.name == name) {
        name = format!("{} 副本 {}", source.name, n);
        n += 1;
    }
    let copy = TestSuite {
        id: Uuid::new_v4(),
        project_id: source.project_id,
        name,
        description: source.description,
        folder_id: source.folder_id,
        created_at: now,
        updated_at: now,
    };
    let copied_items: Vec<TestSuiteItem> = items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| TestSuiteItem {
            id: Uuid::new_v4(),
            suite_id: copy.id,
            scenario_id: item.scenario_id,
            sort_order: idx as i64,
            enabled: item.enabled,
        })
        .collect();
    save_test_suite(db, &copy, &copied_items).await?;
    Ok(copy)
}

/// 启用 / 禁用套件成员。
pub async fn set_test_suite_item_enabled(
    db: &SqlitePool,
    item_id: Uuid,
    enabled: bool,
) -> Result<()> {
    let result = sqlx::query("UPDATE test_suite_items SET enabled = ? WHERE id = ?")
        .bind(enabled)
        .bind(item_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("套件成员（{item_id}）")));
    }
    Ok(())
}

/// 套件内重排（按传入 id 顺序写回 sort_order；不属于该套件的 id 拒绝）。
pub async fn reorder_test_suite_items(db: &SqlitePool, suite_id: Uuid, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let owned: std::collections::HashSet<Uuid> = list_test_suite_items(db, suite_id)
        .await?
        .into_iter()
        .map(|i| i.id)
        .collect();
    if ids.iter().any(|id| !owned.contains(id)) {
        return Err(AppError::Validation("存在不属于该套件的成员".into()));
    }
    let mut tx = db.begin().await.map_err(db_err)?;
    for (idx, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE test_suite_items SET sort_order = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

// ---------- 批次 ----------

/// 新建批次 + 明细（事务；明细初始状态由调用方给定）。
pub async fn create_test_batch(
    db: &SqlitePool,
    batch: &TestRunBatch,
    items: &[TestRunBatchItem],
) -> Result<()> {
    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO test_run_batches
         (id, project_id, suite_id, parent_batch_id, environment_id, status, mode, concurrency,
          failure_policy, retry_count, skip_disabled, total_count, passed_count, failed_count,
          skipped_count, cancelled_count, running_count, pending_count, started_at, finished_at,
          duration_ms, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(batch.id.to_string())
    .bind(batch.project_id.to_string())
    .bind(batch.suite_id.map(|v| v.to_string()))
    .bind(batch.parent_batch_id.map(|v| v.to_string()))
    .bind(batch.environment_id.map(|v| v.to_string()))
    .bind(batch.status.as_str())
    .bind(batch.mode.as_str())
    .bind(batch.concurrency)
    .bind(batch.failure_policy.as_str())
    .bind(batch.retry_count)
    .bind(batch.skip_disabled)
    .bind(batch.total_count)
    .bind(batch.passed_count)
    .bind(batch.failed_count)
    .bind(batch.skipped_count)
    .bind(batch.cancelled_count)
    .bind(batch.running_count)
    .bind(batch.pending_count)
    .bind(batch.started_at.map(|v| v.to_rfc3339()))
    .bind(batch.finished_at.map(|v| v.to_rfc3339()))
    .bind(batch.duration_ms)
    .bind(batch.created_at.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    for item in items {
        sqlx::query(
            "INSERT INTO test_run_batch_items
             (id, batch_id, scenario_id, scenario_name, status, sort_order, attempt, duration_ms,
              error_message, execution_record_id, started_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(batch.id.to_string())
        .bind(item.scenario_id.to_string())
        .bind(&item.scenario_name)
        .bind(item.status.as_str())
        .bind(item.sort_order)
        .bind(item.attempt)
        .bind(item.duration_ms)
        .bind(item.error_message.as_deref())
        .bind(item.execution_record_id.map(|v| v.to_string()))
        .bind(item.started_at.map(|v| v.to_rfc3339()))
        .bind(item.finished_at.map(|v| v.to_rfc3339()))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// 取单个批次（不存在返回 None）。
pub async fn get_test_batch(db: &SqlitePool, id: Uuid) -> Result<Option<TestRunBatch>> {
    let row: Option<TestRunBatchRow> = sqlx::query_as(&format!(
        "SELECT {BATCH_COLUMNS} FROM test_run_batches WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    row.map(TestRunBatchRow::into_model).transpose()
}

/// 项目最近批次（新→旧；列表页）。
pub async fn list_test_batches(
    db: &SqlitePool,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<TestRunBatch>> {
    let rows: Vec<TestRunBatchRow> = sqlx::query_as(&format!(
        "SELECT {BATCH_COLUMNS} FROM test_run_batches
         WHERE project_id = ? ORDER BY created_at DESC LIMIT ?"
    ))
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(TestRunBatchRow::into_model).collect()
}

/// 批次明细（编排序）。
pub async fn list_test_batch_items(
    db: &SqlitePool,
    batch_id: Uuid,
) -> Result<Vec<TestRunBatchItem>> {
    let rows: Vec<TestRunBatchItemRow> = sqlx::query_as(&format!(
        "SELECT {BATCH_ITEM_COLUMNS} FROM test_run_batch_items
         WHERE batch_id = ? ORDER BY sort_order ASC"
    ))
    .bind(batch_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter()
        .map(TestRunBatchItemRow::into_model)
        .collect()
}

/// 覆盖写批次（状态 / 计数 / 起止时间）。
pub async fn update_test_batch(db: &SqlitePool, batch: &TestRunBatch) -> Result<()> {
    sqlx::query(
        "UPDATE test_run_batches SET
           environment_id = ?, status = ?, mode = ?, concurrency = ?, failure_policy = ?,
           retry_count = ?, skip_disabled = ?, total_count = ?, passed_count = ?,
           failed_count = ?, skipped_count = ?, cancelled_count = ?, running_count = ?,
           pending_count = ?, started_at = ?, finished_at = ?, duration_ms = ?
         WHERE id = ?",
    )
    .bind(batch.environment_id.map(|v| v.to_string()))
    .bind(batch.status.as_str())
    .bind(batch.mode.as_str())
    .bind(batch.concurrency)
    .bind(batch.failure_policy.as_str())
    .bind(batch.retry_count)
    .bind(batch.skip_disabled)
    .bind(batch.total_count)
    .bind(batch.passed_count)
    .bind(batch.failed_count)
    .bind(batch.skipped_count)
    .bind(batch.cancelled_count)
    .bind(batch.running_count)
    .bind(batch.pending_count)
    .bind(batch.started_at.map(|v| v.to_rfc3339()))
    .bind(batch.finished_at.map(|v| v.to_rfc3339()))
    .bind(batch.duration_ms)
    .bind(batch.id.to_string())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// 覆盖写批次明细。
pub async fn update_test_batch_item(db: &SqlitePool, item: &TestRunBatchItem) -> Result<()> {
    sqlx::query(
        "UPDATE test_run_batch_items SET
           scenario_name = ?, status = ?, sort_order = ?, attempt = ?, duration_ms = ?,
           error_message = ?, execution_record_id = ?, started_at = ?, finished_at = ?
         WHERE id = ?",
    )
    .bind(&item.scenario_name)
    .bind(item.status.as_str())
    .bind(item.sort_order)
    .bind(item.attempt)
    .bind(item.duration_ms)
    .bind(item.error_message.as_deref())
    .bind(item.execution_record_id.map(|v| v.to_string()))
    .bind(item.started_at.map(|v| v.to_rfc3339()))
    .bind(item.finished_at.map(|v| v.to_rfc3339()))
    .bind(item.id.to_string())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// 删除批次（明细随外键 / 显式删除清理；单场景执行历史保留）。
pub async fn delete_test_batch(db: &SqlitePool, id: Uuid) -> Result<()> {
    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM test_run_batch_items WHERE batch_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM test_run_batches WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// 启动恢复：把上次进程遗留的未完成批次标记为 interrupted。
///
/// * 只处理 pending / running / paused 三种「未终态」批次；
/// * 明细里 pending / running 的行同样标记 interrupted（保留已完成的通过 /
///   失败行，报告可继续展示已完成部分）；
/// * 返回被标记的批次数。
///
/// 必须在应用启动、且没有任何批次在跑时调用（lib.rs setup 单点）。
pub async fn recover_interrupted_batches(db: &SqlitePool) -> Result<u64> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query(
        "UPDATE test_run_batch_items SET status = 'interrupted', finished_at = COALESCE(finished_at, ?)
         WHERE status IN ('pending', 'running')
           AND batch_id IN (
             SELECT id FROM test_run_batches WHERE status IN ('pending', 'running', 'paused')
           )",
    )
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    let result = sqlx::query(
        "UPDATE test_run_batches SET status = 'interrupted', finished_at = ?,
                running_count = 0, pending_count = 0
         WHERE status IN ('pending', 'running', 'paused')",
    )
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;
    use crate::repository::projects::create_project;
    use chrono::Utc;

    fn suite(project_id: Uuid, name: &str) -> TestSuite {
        TestSuite {
            id: Uuid::new_v4(),
            project_id,
            name: name.into(),
            description: "回归".into(),
            folder_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn item(suite_id: Uuid, scenario_id: Uuid, order: i64, enabled: bool) -> TestSuiteItem {
        TestSuiteItem {
            id: Uuid::new_v4(),
            suite_id,
            scenario_id,
            sort_order: order,
            enabled,
        }
    }

    fn batch(project_id: Uuid, status: TestBatchStatus) -> TestRunBatch {
        TestRunBatch {
            id: Uuid::new_v4(),
            project_id,
            suite_id: None,
            parent_batch_id: None,
            environment_id: None,
            status,
            mode: TestBatchMode::Serial,
            concurrency: 1,
            failure_policy: TestFailurePolicy::Continue,
            retry_count: 0,
            skip_disabled: true,
            total_count: 0,
            passed_count: 0,
            failed_count: 0,
            skipped_count: 0,
            cancelled_count: 0,
            running_count: 0,
            pending_count: 0,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn suite_crud_and_duplicate() {
        let db = memory_pool().await.unwrap();
        let project = create_project(&db, "套件", "").await.unwrap();
        let s = suite(project.id, "订单回归");
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let items = vec![item(s.id, a, 0, true), item(s.id, b, 1, false)];
        save_test_suite(&db, &s, &items).await.unwrap();

        let listed = list_test_suites(&db, project.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        let loaded = list_test_suite_items(&db, s.id).await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].scenario_id, b);
        assert!(!loaded[1].enabled);

        // 重排 + 启用切换。
        reorder_test_suite_items(&db, s.id, &[loaded[1].id, loaded[0].id])
            .await
            .unwrap();
        let reordered = list_test_suite_items(&db, s.id).await.unwrap();
        assert_eq!(reordered[0].scenario_id, b);
        set_test_suite_item_enabled(&db, loaded[1].id, true)
            .await
            .unwrap();
        let reloaded = list_test_suite_items(&db, s.id).await.unwrap();
        assert!(reloaded.iter().all(|i| i.enabled));

        // 复制：新套件 + 独立成员 id。
        let copy = duplicate_test_suite(&db, s.id).await.unwrap();
        assert_eq!(copy.name, "订单回归 副本");
        let copied = list_test_suite_items(&db, copy.id).await.unwrap();
        assert_eq!(copied.len(), 2);
        assert!(copied.iter().all(|i| i.id != loaded[0].id));

        delete_test_suite(&db, copy.id).await.unwrap();
        assert!(list_test_suite_items(&db, copy.id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(list_test_suites(&db, project.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rerun_creates_new_batch_and_keeps_history() {
        let db = memory_pool().await.unwrap();
        let project = create_project(&db, "重跑", "").await.unwrap();
        let s = suite(project.id, "回归");
        let scenario_id = Uuid::new_v4();
        save_test_suite(&db, &s, &[item(s.id, scenario_id, 0, true)])
            .await
            .unwrap();

        // 第一次执行（环境 A）。
        let mut b1 = batch(project.id, TestBatchStatus::Completed);
        b1.suite_id = Some(s.id);
        b1.environment_id = Some(Uuid::new_v4());
        let item1 = TestRunBatchItem {
            id: Uuid::new_v4(),
            batch_id: b1.id,
            scenario_id,
            scenario_name: "登录".into(),
            status: TestBatchItemStatus::Passed,
            sort_order: 0,
            attempt: 1,
            duration_ms: Some(8.0),
            error_message: None,
            execution_record_id: Some(Uuid::new_v4()),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
        };
        create_test_batch(&db, &b1, &[item1]).await.unwrap();

        // 换环境后重跑：新批次（parent 指向原批次），历史保留两条。
        let mut b2 = batch(project.id, TestBatchStatus::Completed);
        b2.suite_id = Some(s.id);
        b2.parent_batch_id = Some(b1.id);
        b2.environment_id = Some(Uuid::new_v4());
        let item2 = TestRunBatchItem {
            id: Uuid::new_v4(),
            batch_id: b2.id,
            scenario_id,
            scenario_name: "登录".into(),
            status: TestBatchItemStatus::Passed,
            sort_order: 0,
            attempt: 1,
            duration_ms: Some(9.0),
            error_message: None,
            execution_record_id: None,
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
        };
        create_test_batch(&db, &b2, &[item2]).await.unwrap();

        let all = list_test_batches(&db, project.id, 10).await.unwrap();
        assert_eq!(all.len(), 2);
        let reloaded = get_test_batch(&db, b2.id).await.unwrap().unwrap();
        assert_eq!(reloaded.parent_batch_id, Some(b1.id));
        assert_ne!(reloaded.environment_id, b1.environment_id);

        // 删除套件：批次历史与场景名快照仍在（无外键级联）。
        delete_test_suite(&db, s.id).await.unwrap();
        let kept = get_test_batch(&db, b1.id).await.unwrap().unwrap();
        assert_eq!(kept.suite_id, Some(s.id));
        let kept_items = list_test_batch_items(&db, b1.id).await.unwrap();
        assert_eq!(kept_items[0].scenario_name, "登录");
    }

    #[tokio::test]
    async fn batch_persists_items_and_recovers_interrupted() {
        let db = memory_pool().await.unwrap();
        let project = create_project(&db, "批次", "").await.unwrap();
        let b = batch(project.id, TestBatchStatus::Running);
        let scenario_id = Uuid::new_v4();
        let items = vec![
            TestRunBatchItem {
                id: Uuid::new_v4(),
                batch_id: b.id,
                scenario_id,
                scenario_name: "登录".into(),
                status: TestBatchItemStatus::Passed,
                sort_order: 0,
                attempt: 1,
                duration_ms: Some(12.0),
                error_message: None,
                execution_record_id: Some(Uuid::new_v4()),
                started_at: Some(Utc::now()),
                finished_at: Some(Utc::now()),
            },
            TestRunBatchItem {
                id: Uuid::new_v4(),
                batch_id: b.id,
                scenario_id: Uuid::new_v4(),
                scenario_name: "下单".into(),
                status: TestBatchItemStatus::Running,
                sort_order: 1,
                attempt: 1,
                duration_ms: None,
                error_message: None,
                execution_record_id: None,
                started_at: Some(Utc::now()),
                finished_at: None,
            },
        ];
        create_test_batch(&db, &b, &items).await.unwrap();
        assert_eq!(list_test_batch_items(&db, b.id).await.unwrap().len(), 2);

        let marked = recover_interrupted_batches(&db).await.unwrap();
        assert_eq!(marked, 1);
        let reloaded = get_test_batch(&db, b.id).await.unwrap().unwrap();
        assert_eq!(reloaded.status, TestBatchStatus::Interrupted);
        assert_eq!(reloaded.running_count, 0);
        assert_eq!(reloaded.pending_count, 0);
        let rows = list_test_batch_items(&db, b.id).await.unwrap();
        // 已完成行保留，未完成行标记 interrupted。
        assert_eq!(rows[0].status, TestBatchItemStatus::Passed);
        assert_eq!(rows[1].status, TestBatchItemStatus::Interrupted);
        assert!(rows[1].finished_at.is_some());

        // 再次恢复幂等（没有未终态批次）。
        assert_eq!(recover_interrupted_batches(&db).await.unwrap(), 0);
    }
}
