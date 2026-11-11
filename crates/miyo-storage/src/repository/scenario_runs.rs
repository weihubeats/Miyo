//! Scenario Run（场景运行历史）：run_scenario 完成落一行，回看与重开恢复。

use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::ScenarioRun;
use miyo_core::Result;

use super::rows::ScenarioRunRow;

/// 每场景保留的运行历史条数（插入后修剪更旧的）。
pub const SCENARIO_RUN_KEEP: i64 = 20;

/// 保存一次场景运行历史（同 id 重复落库覆盖——取消重跑同 run_id 不膨胀）。
pub async fn save_scenario_run(db: &SqlitePool, run: &ScenarioRun) -> Result<()> {
    let row = ScenarioRunRow::from_model(run);
    sqlx::query(
        "INSERT OR REPLACE INTO scenario_runs
         (id, scenario_id, environment_id, finished_at, duration_ms,
          total_steps, ok_count, fail_count, skipped_count,
          cancelled, stopped_on_failure, results_json, variables_json, meta_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.scenario_id)
    .bind(&row.environment_id)
    .bind(&row.finished_at)
    .bind(row.duration_ms)
    .bind(row.total_steps)
    .bind(row.ok_count)
    .bind(row.fail_count)
    .bind(row.skipped_count)
    .bind(row.cancelled)
    .bind(row.stopped_on_failure)
    .bind(&row.results_json)
    .bind(&row.variables_json)
    .bind(&row.meta_json)
    .execute(db)
    .await?;
    Ok(())
}

/// 列出某场景最近的运行历史（新→旧）。
pub async fn list_scenario_runs(
    db: &SqlitePool,
    scenario_id: Uuid,
    limit: i64,
) -> Result<Vec<ScenarioRun>> {
    let rows: Vec<ScenarioRunRow> = sqlx::query_as(
        "SELECT id, scenario_id, environment_id, finished_at, duration_ms,
                total_steps, ok_count, fail_count, skipped_count,
                cancelled, stopped_on_failure, results_json, variables_json, meta_json
         FROM scenario_runs WHERE scenario_id = ? ORDER BY finished_at DESC LIMIT ?",
    )
    .bind(scenario_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(|r| r.into_model()).collect()
}

/// 每场景最近一次运行的摘要（不含 results_json / variables_json / meta_json
/// 三个大 JSON 列——场景库列表页只需要计数，全量读出再丢弃是纯浪费）。
#[derive(Debug, sqlx::FromRow)]
pub struct ScenarioRunSummaryRow {
    pub id: String,
    pub scenario_id: String,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: Option<f64>,
    pub total_steps: i64,
    pub ok_count: i64,
    pub fail_count: i64,
    pub skipped_count: i64,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
}

/// 批量取项目内每个场景最近一次运行的摘要（一次 SQL 替代逐场景 N+1 查询，
/// 且不读大 JSON 列）。窗口函数按 finished_at 新→旧取 rn=1。
pub async fn latest_run_summaries_for_project(
    db: &SqlitePool,
    project_id: Uuid,
) -> Result<Vec<ScenarioRunSummaryRow>> {
    let rows: Vec<ScenarioRunSummaryRow> = sqlx::query_as(
        "SELECT id, scenario_id, finished_at, duration_ms, total_steps,
                ok_count, fail_count, skipped_count, cancelled, stopped_on_failure
         FROM (
             SELECT id, scenario_id, finished_at, duration_ms, total_steps,
                    ok_count, fail_count, skipped_count, cancelled, stopped_on_failure,
                    ROW_NUMBER() OVER (
                        PARTITION BY scenario_id ORDER BY finished_at DESC
                    ) AS rn
             FROM scenario_runs
             WHERE scenario_id IN (SELECT id FROM scenarios WHERE project_id = ?)
         )
         WHERE rn = 1",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// 列出某场景最近的运行摘要（新→旧；不读 results_json / variables_json /
/// meta_json 三个大列——历史列表只展示计数，全量读出解析再丢弃是纯 IO 浪费，
/// 每条 results_json 最大 64KB × 步数）。详情走 [`get_scenario_run`]。
pub async fn list_scenario_run_summaries(
    db: &SqlitePool,
    scenario_id: Uuid,
    limit: i64,
) -> Result<Vec<ScenarioRunSummaryRow>> {
    let rows: Vec<ScenarioRunSummaryRow> = sqlx::query_as(
        "SELECT id, scenario_id, finished_at, duration_ms, total_steps,
                ok_count, fail_count, skipped_count, cancelled, stopped_on_failure
         FROM scenario_runs WHERE scenario_id = ? ORDER BY finished_at DESC LIMIT ?",
    )
    .bind(scenario_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// 取单条运行历史（回看详情；不存在返回 None）。
pub async fn get_scenario_run(db: &SqlitePool, id: Uuid) -> Result<Option<ScenarioRun>> {
    let row: Option<ScenarioRunRow> = sqlx::query_as(
        "SELECT id, scenario_id, environment_id, finished_at, duration_ms,
                total_steps, ok_count, fail_count, skipped_count,
                cancelled, stopped_on_failure, results_json, variables_json, meta_json
         FROM scenario_runs WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(|r| r.into_model()).transpose()
}

/// 删除单条运行历史（历史清理；记录存在返回 true）。
pub async fn delete_scenario_run(db: &SqlitePool, run_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM scenario_runs WHERE id = ?")
        .bind(run_id.to_string())
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 清空场景的全部运行历史（保留场景本身），返回删除条数。
pub async fn delete_scenario_runs_all(db: &SqlitePool, scenario_id: Uuid) -> Result<u64> {
    let result = sqlx::query("DELETE FROM scenario_runs WHERE scenario_id = ?")
        .bind(scenario_id.to_string())
        .execute(db)
        .await?;
    Ok(result.rows_affected())
}

/// 统计场景的运行历史条数（删除场景时回执用；COUNT(*) 免拉全量 JSON）。
pub async fn count_scenario_runs(db: &SqlitePool, scenario_id: Uuid) -> Result<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scenario_runs WHERE scenario_id = ?")
        .bind(scenario_id.to_string())
        .fetch_one(db)
        .await?;
    Ok(n)
}

/// 修剪历史：仅保留最近 `keep` 条。
pub async fn prune_scenario_runs(db: &SqlitePool, scenario_id: Uuid, keep: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM scenario_runs WHERE scenario_id = ? AND id NOT IN (
             SELECT id FROM scenario_runs WHERE scenario_id = ?
             ORDER BY finished_at DESC LIMIT ?
         )",
    )
    .bind(scenario_id.to_string())
    .bind(scenario_id.to_string())
    .bind(keep)
    .execute(db)
    .await?;
    Ok(())
}
