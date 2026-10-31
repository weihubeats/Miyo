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
          cancelled, stopped_on_failure, results_json, variables_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                cancelled, stopped_on_failure, results_json, variables_json
         FROM scenario_runs WHERE scenario_id = ? ORDER BY finished_at DESC LIMIT ?",
    )
    .bind(scenario_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(|r| r.into_model()).collect()
}

/// 取单条运行历史（回看详情；不存在返回 None）。
pub async fn get_scenario_run(db: &SqlitePool, id: Uuid) -> Result<Option<ScenarioRun>> {
    let row: Option<ScenarioRunRow> = sqlx::query_as(
        "SELECT id, scenario_id, environment_id, finished_at, duration_ms,
                total_steps, ok_count, fail_count, skipped_count,
                cancelled, stopped_on_failure, results_json, variables_json
         FROM scenario_runs WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(|r| r.into_model()).transpose()
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
