//! Test Run（测试运行结果）。

use crate::db::db_err;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::TestRun;
use miyo_core::Result;

use super::rows::TestRunRow;

/// 保存一次测试运行结果。
/// 每项目保留的运行结果上限（与 request_histories / scenario_runs 同口径的
/// 滚动清理——此前 test_runs 无任何清理，长期使用无限增长）。
const TEST_RUNS_KEEP_PER_PROJECT: i64 = 50;

pub async fn save_test_run(db: &SqlitePool, run: &TestRun) -> Result<()> {
    let row = TestRunRow::from_model(run);
    sqlx::query(
        "INSERT INTO test_runs (id, project_id, environment_id, name, result_json, started_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.environment_id)
    .bind(&row.name)
    .bind(&row.result_json)
    .bind(row.started_at.clone())
    .bind(&row.finished_at)
    .execute(db)
    .await.map_err(db_err)? ;
    prune_test_runs(db, run.project_id, TEST_RUNS_KEEP_PER_PROJECT).await?;
    Ok(())
}

/// 修剪：仅保留每项目最近 N 条（按 started_at 倒序）。
async fn prune_test_runs(db: &SqlitePool, project_id: Uuid, keep: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM test_runs WHERE project_id = ? AND id NOT IN (
             SELECT id FROM test_runs WHERE project_id = ?
             ORDER BY started_at DESC LIMIT ?
         )",
    )
    .bind(project_id.to_string())
    .bind(project_id.to_string())
    .bind(keep)
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// 删除单条测试运行（M11 历史清理）。
pub async fn delete_test_run(db: &SqlitePool, run_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM test_runs WHERE id = ?")
        .bind(run_id.to_string())
        .execute(db)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// 列出最近的测试运行。
pub async fn list_test_runs(db: &SqlitePool, project_id: Uuid, limit: i64) -> Result<Vec<TestRun>> {
    let rows: Vec<TestRunRow> = sqlx::query_as(
        "SELECT id, project_id, environment_id, name, result_json, started_at, finished_at
         FROM test_runs WHERE project_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter().map(|r| r.into_model()).collect()
}
