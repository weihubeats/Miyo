//! 场景修复草稿（AI 修复 / 场景生成的留档；需求五「保留修改历史 + 允许撤销」）。
//!
//! - `base_json`：应用前的场景快照（撤销 = 回写此快照）；
//! - `draft_json`：草稿本体（应用 = 回写 draft_json）；
//! - `applied_at`：已应用时间（None = 待应用）。

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::{AppError, Result};

/// scenario_drafts 行元组（sqlx query_as 解包用）。
type DraftTuple = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

/// 场景草稿。
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioDraftRow {
    pub id: Uuid,
    pub scenario_id: Uuid,
    pub base_json: String,
    pub draft_json: String,
    pub summary: String,
    /// 来源（ai_fix / ai_generate / manual）。
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
}

/// 保存草稿（新草稿总是待应用状态）。
pub async fn save_scenario_draft(db: &SqlitePool, draft: &ScenarioDraftRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO scenario_drafts (id, scenario_id, base_json, draft_json, summary, source, created_at, applied_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(draft.id.to_string())
    .bind(draft.scenario_id.to_string())
    .bind(&draft.base_json)
    .bind(&draft.draft_json)
    .bind(&draft.summary)
    .bind(&draft.source)
    .bind(draft.created_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(AppError::from)?;
    Ok(())
}

/// 场景的草稿历史（新→旧，最多 limit 条）。
pub async fn list_scenario_drafts(
    db: &SqlitePool,
    scenario_id: Uuid,
    limit: i64,
) -> Result<Vec<ScenarioDraftRow>> {
    let rows: Vec<DraftTuple> = sqlx::query_as(
        "SELECT id, scenario_id, base_json, draft_json, summary, source, created_at, applied_at
             FROM scenario_drafts WHERE scenario_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(scenario_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await
    .map_err(AppError::from)?;
    rows.into_iter()
        .map(
            |(id, sid, base, draft, summary, source, created, applied)| {
                Ok(ScenarioDraftRow {
                    id: Uuid::parse_str(&id).map_err(uuid_err)?,
                    scenario_id: Uuid::parse_str(&sid).map_err(uuid_err)?,
                    base_json: base,
                    draft_json: draft,
                    summary,
                    source,
                    created_at: parse_ts(&created)?,
                    applied_at: applied.map(|a| parse_ts(&a)).transpose()?,
                })
            },
        )
        .collect()
}

/// 取单条草稿。
pub async fn get_scenario_draft(
    db: &SqlitePool,
    draft_id: Uuid,
) -> Result<Option<ScenarioDraftRow>> {
    let row: Option<DraftTuple> = sqlx::query_as(
        "SELECT id, scenario_id, base_json, draft_json, summary, source, created_at, applied_at
             FROM scenario_drafts WHERE id = ?",
    )
    .bind(draft_id.to_string())
    .fetch_optional(db)
    .await
    .map_err(AppError::from)?;
    row.map(
        |(id, sid, base, draft, summary, source, created, applied)| {
            Ok(ScenarioDraftRow {
                id: Uuid::parse_str(&id).map_err(uuid_err)?,
                scenario_id: Uuid::parse_str(&sid).map_err(uuid_err)?,
                base_json: base,
                draft_json: draft,
                summary,
                source,
                created_at: parse_ts(&created)?,
                applied_at: applied.map(|a| parse_ts(&a)).transpose()?,
            })
        },
    )
    .transpose()
}

/// 标记草稿已应用。
pub async fn mark_scenario_draft_applied(db: &SqlitePool, draft_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE scenario_drafts SET applied_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(draft_id.to_string())
        .execute(db)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// 删除草稿。
pub async fn delete_scenario_draft(db: &SqlitePool, draft_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM scenario_drafts WHERE id = ?")
        .bind(draft_id.to_string())
        .execute(db)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::Database(sqlx::Error::Protocol(format!("时间戳解析失败：{e}"))))
}

fn uuid_err(e: uuid::Error) -> AppError {
    AppError::Database(sqlx::Error::Protocol(format!("UUID 解析失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;

    fn draft(scenario_id: Uuid) -> ScenarioDraftRow {
        ScenarioDraftRow {
            id: Uuid::new_v4(),
            scenario_id,
            base_json: r#"{"name":"旧"}"#.into(),
            draft_json: r#"{"name":"新"}"#.into(),
            summary: "补 payToken 提取".into(),
            source: "ai_fix".into(),
            created_at: Utc::now(),
            applied_at: None,
        }
    }

    #[tokio::test]
    async fn draft_roundtrip_and_apply() {
        let db = memory_pool().await.unwrap();
        // 草稿外键指向真实场景：先建项目 + 场景。
        let project = crate::repository::create_project(&db, "草稿", "")
            .await
            .unwrap();
        let now = chrono::Utc::now();
        let scenario = miyo_core::model::Scenario {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "测试场景".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![],
            script: None,
            setup: vec![],
            teardown: vec![],
            teardown_enabled: true,
            data_sets: vec![],
            created_at: now,
            updated_at: now,
        };
        crate::repository::save_scenario(&db, &scenario)
            .await
            .unwrap();
        let sid = scenario.id;
        let d = draft(sid);
        save_scenario_draft(&db, &d).await.unwrap();

        let list = list_scenario_drafts(&db, sid, 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].summary, "补 payToken 提取");
        assert!(list[0].applied_at.is_none());

        mark_scenario_draft_applied(&db, d.id).await.unwrap();
        let got = get_scenario_draft(&db, d.id).await.unwrap().unwrap();
        assert!(got.applied_at.is_some());

        delete_scenario_draft(&db, d.id).await.unwrap();
        assert!(get_scenario_draft(&db, d.id).await.unwrap().is_none());
    }
}
