//! TestCase Run（测试用例执行历史）：每次运行落一行，append-only；
//! 修改用例不覆盖历史。保留策略镜像 scenario_runs（每用例 KEEP 条）。

use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{TestCaseRun, TestCaseRunSummary, TestCaseStatus};
use miyo_core::Result;

/// 每用例保留的执行历史条数（插入后修剪更旧的）。
pub const TEST_CASE_RUN_KEEP: i64 = 20;

/// 保存一次用例执行历史。
pub async fn save_test_case_run(db: &SqlitePool, run: &TestCaseRun) -> Result<()> {
    sqlx::query(
        "INSERT INTO test_case_runs
         (id, case_id, endpoint_id, environment_id, started_at, finished_at,
          duration_ms, status, status_code, ok_count, fail_count, error, results_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(run.id.to_string())
    .bind(run.case_id.to_string())
    .bind(run.endpoint_id.map(|v| v.to_string()))
    .bind(run.environment_id.map(|v| v.to_string()))
    .bind(run.started_at.to_rfc3339())
    .bind(run.finished_at.to_rfc3339())
    .bind(run.duration_ms)
    .bind(run.status.as_str())
    .bind(run.status_code.map(|v| v as i64))
    .bind(run.ok_count as i64)
    .bind(run.fail_count as i64)
    .bind(&run.error)
    .bind(&run.results_json)
    .execute(db)
    .await?;
    Ok(())
}

/// 列出某用例最近的执行历史摘要（新→旧；不含 results_json——每条最大
/// 64KB×步数，列表页不需要全文；详情走 [`get_test_case_run`] 按需取，
/// 与 scenario_runs 的 summary 范式一致）。
pub async fn list_test_case_run_summaries(
    db: &SqlitePool,
    case_id: Uuid,
    limit: i64,
) -> Result<Vec<TestCaseRunSummary>> {
    let rows: Vec<TestCaseRunSummaryRow> = sqlx::query_as(
        "SELECT id, case_id, endpoint_id, environment_id, started_at, finished_at,
                duration_ms, status, status_code, ok_count, fail_count, error
         FROM test_case_runs WHERE case_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(case_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(|r| r.into_model()).collect()
}

/// 列出某用例最近的执行历史（新→旧；含 results_json 全文）。
#[allow(dead_code)] // 保留全文变体供详情直读场景使用
pub async fn list_test_case_runs(
    db: &SqlitePool,
    case_id: Uuid,
    limit: i64,
) -> Result<Vec<TestCaseRun>> {
    let rows: Vec<TestCaseRunRow> = sqlx::query_as(
        "SELECT id, case_id, endpoint_id, environment_id, started_at, finished_at,
                duration_ms, status, status_code, ok_count, fail_count, error, results_json
         FROM test_case_runs WHERE case_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(case_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(|r| r.into_model()).collect()
}

/// 取单条执行历史（回看详情；不存在返回 None）。
pub async fn get_test_case_run(db: &SqlitePool, run_id: Uuid) -> Result<Option<TestCaseRun>> {
    let row: Option<TestCaseRunRow> = sqlx::query_as(
        "SELECT id, case_id, endpoint_id, environment_id, started_at, finished_at,
                duration_ms, status, status_code, ok_count, fail_count, error, results_json
         FROM test_case_runs WHERE id = ?",
    )
    .bind(run_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(|r| r.into_model()).transpose()
}

/// 删除单条执行历史（历史清理；记录存在返回 true）。
pub async fn delete_test_case_run(db: &SqlitePool, run_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM test_case_runs WHERE id = ?")
        .bind(run_id.to_string())
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 修剪历史：仅保留最近 `keep` 条。
pub async fn prune_test_case_runs(db: &SqlitePool, case_id: Uuid, keep: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM test_case_runs WHERE case_id = ? AND id NOT IN (
             SELECT id FROM test_case_runs WHERE case_id = ?
             ORDER BY started_at DESC LIMIT ?
         )",
    )
    .bind(case_id.to_string())
    .bind(case_id.to_string())
    .bind(keep)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TestCaseRunRow {
    id: String,
    case_id: String,
    endpoint_id: Option<String>,
    environment_id: Option<String>,
    started_at: String,
    finished_at: String,
    duration_ms: Option<f64>,
    status: String,
    status_code: Option<i64>,
    ok_count: i64,
    fail_count: i64,
    error: Option<String>,
    results_json: String,
}

impl TestCaseRunRow {
    fn into_model(self) -> Result<TestCaseRun> {
        Ok(TestCaseRun {
            id: super::rows::parse_uuid(&self.id)?,
            case_id: super::rows::parse_uuid(&self.case_id)?,
            endpoint_id: self
                .endpoint_id
                .as_deref()
                .map(super::rows::parse_uuid)
                .transpose()?,
            environment_id: self
                .environment_id
                .as_deref()
                .map(super::rows::parse_uuid)
                .transpose()?,
            started_at: super::rows::parse_time(&self.started_at)?,
            finished_at: super::rows::parse_time(&self.finished_at)?,
            duration_ms: self.duration_ms,
            status: TestCaseStatus::parse(&self.status).unwrap_or(TestCaseStatus::Failed),
            status_code: self.status_code.map(|v| v.clamp(0, u16::MAX as i64) as u16),
            ok_count: self.ok_count.clamp(0, u32::MAX as i64) as u32,
            fail_count: self.fail_count.clamp(0, u32::MAX as i64) as u32,
            error: self.error,
            results_json: self.results_json,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TestCaseRunSummaryRow {
    id: String,
    case_id: String,
    endpoint_id: Option<String>,
    environment_id: Option<String>,
    started_at: String,
    finished_at: String,
    duration_ms: Option<f64>,
    status: String,
    status_code: Option<i64>,
    ok_count: i64,
    fail_count: i64,
    error: Option<String>,
}

impl TestCaseRunSummaryRow {
    fn into_model(self) -> Result<TestCaseRunSummary> {
        Ok(TestCaseRunSummary {
            id: super::rows::parse_uuid(&self.id)?,
            case_id: super::rows::parse_uuid(&self.case_id)?,
            endpoint_id: self
                .endpoint_id
                .as_deref()
                .map(super::rows::parse_uuid)
                .transpose()?,
            environment_id: self
                .environment_id
                .as_deref()
                .map(super::rows::parse_uuid)
                .transpose()?,
            started_at: super::rows::parse_time(&self.started_at)?,
            finished_at: super::rows::parse_time(&self.finished_at)?,
            duration_ms: self.duration_ms,
            status: TestCaseStatus::parse(&self.status).unwrap_or(TestCaseStatus::Failed),
            status_code: self.status_code.map(|v| v.clamp(0, u16::MAX as i64) as u16),
            ok_count: self.ok_count.clamp(0, u32::MAX as i64) as u32,
            fail_count: self.fail_count.clamp(0, u32::MAX as i64) as u32,
            error: self.error,
        })
    }
}
