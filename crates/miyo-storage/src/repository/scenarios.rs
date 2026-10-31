//! 集成测试场景 CRUD（项目级接口链路；步骤内嵌 steps_json）。

use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{Scenario, ScenarioStep};
use miyo_core::{AppError, Result};

#[derive(sqlx::FromRow)]
struct ScenarioRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    sort_order: i64,
    steps_json: String,
    script: Option<String>,
    created_at: String,
    updated_at: String,
}

impl ScenarioRow {
    fn into_model(self) -> Result<Scenario> {
        let steps: Vec<ScenarioStep> = serde_json::from_str(&self.steps_json).unwrap_or_default();
        Ok(Scenario {
            id: super::rows::parse_uuid(&self.id)?,
            project_id: super::rows::parse_uuid(&self.project_id)?,
            name: self.name,
            description: self.description,
            sort_order: self.sort_order,
            steps,
            // 空串归一为未自定义。
            script: self.script.filter(|s| !s.trim().is_empty()),
            created_at: super::rows::parse_time(&self.created_at)?,
            updated_at: super::rows::parse_time(&self.updated_at)?,
        })
    }
}

/// 列出项目的全部场景（编排序 → 名称序）。
pub async fn list_scenarios(db: &SqlitePool, project_id: Uuid) -> Result<Vec<Scenario>> {
    let rows: Vec<ScenarioRow> = sqlx::query_as(
        "SELECT id, project_id, name, description, sort_order, steps_json, script, created_at, updated_at
         FROM scenarios WHERE project_id = ? ORDER BY sort_order ASC, name ASC",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(ScenarioRow::into_model).collect()
}

pub async fn get_scenario(db: &SqlitePool, scenario_id: Uuid) -> Result<Scenario> {
    let row: Option<ScenarioRow> = sqlx::query_as(
        "SELECT id, project_id, name, description, sort_order, steps_json, script, created_at, updated_at
         FROM scenarios WHERE id = ?",
    )
    .bind(scenario_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(ScenarioRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("场景（{scenario_id}）")))
}

/// 保存场景（整体覆盖写，upsert；步骤序列化进 steps_json）。
pub async fn save_scenario(db: &SqlitePool, scenario: &Scenario) -> Result<()> {
    let steps_json = serde_json::to_string(&scenario.steps)
        .map_err(|e| AppError::Validation(format!("场景步骤序列化失败：{e}")))?;
    sqlx::query(
        "INSERT INTO scenarios (id, project_id, name, description, sort_order, steps_json, script, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           description = excluded.description,
           sort_order = excluded.sort_order,
           steps_json = excluded.steps_json,
           script = excluded.script,
           updated_at = excluded.updated_at",
    )
    .bind(scenario.id.to_string())
    .bind(scenario.project_id.to_string())
    .bind(&scenario.name)
    .bind(&scenario.description)
    .bind(scenario.sort_order)
    .bind(&steps_json)
    .bind(scenario.script.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(scenario.created_at.to_rfc3339())
    .bind(scenario.updated_at.to_rfc3339())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_scenario(db: &SqlitePool, scenario_id: Uuid) -> Result<()> {
    // 运行历史一并清理（scenario_runs 有 FK ON DELETE CASCADE，但 sqlite
    // 需 PRAGMA foreign_keys 才生效，显式删除不依赖连接参数）。
    sqlx::query("DELETE FROM scenario_runs WHERE scenario_id = ?")
        .bind(scenario_id.to_string())
        .execute(db)
        .await?;
    let result = sqlx::query("DELETE FROM scenarios WHERE id = ?")
        .bind(scenario_id.to_string())
        .execute(db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("场景（{scenario_id}）")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;
    use crate::repository::projects::create_project;
    use chrono::Utc;
    use serde_json::json;

    fn make_scenario(project_id: Uuid) -> Scenario {
        Scenario {
            id: Uuid::new_v4(),
            project_id,
            name: "登录后查订单".into(),
            description: "拿 token 查列表".into(),
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: Uuid::new_v4(),
                enabled: true,
                extract: json!([{ "name": "token", "from": "body", "path": "$.token" }]),
                assertions: json!([]),
            }],
            script: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn scenario_crud_roundtrip() {
        let db = memory_pool().await.expect("建库");
        let project = create_project(&db, "链路", "").await.unwrap();
        let scenario = make_scenario(project.id);

        save_scenario(&db, &scenario).await.unwrap();
        let listed = list_scenarios(&db, project.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "登录后查订单");
        assert_eq!(listed[0].steps.len(), 1);
        assert_eq!(
            listed[0].steps[0].endpoint_id,
            scenario.steps[0].endpoint_id
        );
        assert_eq!(
            listed[0].steps[0].extract,
            json!([{ "name": "token", "from": "body", "path": "$.token" }])
        );

        // 更新（upsert 覆盖步骤与自定义脚本）。
        let mut updated = scenario.clone();
        updated.name = "改名的场景".into();
        updated.steps.clear();
        updated.script = Some("const BASE = 'x';".into());
        save_scenario(&db, &updated).await.unwrap();
        let reloaded = get_scenario(&db, scenario.id).await.unwrap();
        assert_eq!(reloaded.name, "改名的场景");
        assert!(reloaded.steps.is_empty());
        assert_eq!(reloaded.script.as_deref(), Some("const BASE = 'x';"));

        delete_scenario(&db, scenario.id).await.unwrap();
        assert!(get_scenario(&db, scenario.id).await.is_err());
    }
}
