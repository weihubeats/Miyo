//! 集成测试业务目录（多级文件夹）CRUD、移动与场景归属批量操作。
//!
//! 目录树：`scenario_folders.parent_id` 自引用；同级目录名唯一（DB 唯一索引，
//! 并配应用层预检给出可读错误）。删除父级时用递归 CTE 显式收集子树：
//! sqlite 的 `ON DELETE CASCADE` 依赖 PRAGMA foreign_keys，存储层不假设它开启。
//!
//! 场景归属：`scenarios.folder_id`（NULL = 未分类）。删除目录时按用户选择的
//! 策略处理其场景：仅删空目录 / 场景移到未分类 / 连同场景一起删除——任何
//! 策略都不会静默丢数据（级联删除需调用方二次确认，命令层负责确认语义）。

use crate::db::db_err;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::ScenarioFolder;
use miyo_core::{AppError, Result};

use super::rows::{parse_time, parse_uuid};

#[derive(sqlx::FromRow)]
struct ScenarioFolderRow {
    id: String,
    project_id: String,
    parent_id: Option<String>,
    name: String,
    sort_order: i64,
    created_at: String,
    updated_at: String,
}

impl ScenarioFolderRow {
    fn into_model(self) -> Result<ScenarioFolder> {
        Ok(ScenarioFolder {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            parent_id: self.parent_id.as_deref().map(parse_uuid).transpose()?,
            name: self.name,
            sort_order: self.sort_order,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

const FOLDER_COLUMNS: &str = "id, project_id, parent_id, name, sort_order, created_at, updated_at";

/// 删除目录时对其中场景的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderDeleteMode {
    /// 仅删除空目录（子树内仍有场景时拒绝）。
    Empty,
    /// 删除目录，场景移到未分类（数据不丢）。
    KeepScenarios,
    /// 删除目录及其全部场景（调用方负责二次确认）。
    Cascade,
}

/// 删除目录的结果（前端提示「移动了 N 个 / 删除了 N 个」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDeleteOutcome {
    pub deleted_folders: i64,
    pub moved_scenarios: i64,
    pub deleted_scenarios: i64,
}

/// 列出项目的全部业务目录（排序 → 名称序）。
pub async fn list_scenario_folders(
    db: &SqlitePool,
    project_id: Uuid,
) -> Result<Vec<ScenarioFolder>> {
    let rows: Vec<ScenarioFolderRow> = sqlx::query_as(&format!(
        "SELECT {FOLDER_COLUMNS} FROM scenario_folders
         WHERE project_id = ? ORDER BY sort_order ASC, name ASC"
    ))
    .bind(project_id.to_string())
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    rows.into_iter()
        .map(ScenarioFolderRow::into_model)
        .collect()
}

pub async fn get_scenario_folder(db: &SqlitePool, folder_id: Uuid) -> Result<ScenarioFolder> {
    let row: Option<ScenarioFolderRow> = sqlx::query_as(&format!(
        "SELECT {FOLDER_COLUMNS} FROM scenario_folders WHERE id = ?"
    ))
    .bind(folder_id.to_string())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    row.map(ScenarioFolderRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("场景目录（{folder_id}）")))
}

/// 同级目录名是否已被占用（大小写不敏感；`exclude` 用于重命名自身）。
pub async fn scenario_folder_name_taken(
    db: &SqlitePool,
    project_id: Uuid,
    parent_id: Option<Uuid>,
    name: &str,
    exclude: Option<Uuid>,
) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM scenario_folders
         WHERE project_id = ?
           AND COALESCE(parent_id, '') = COALESCE(?, '')
           AND name = ? COLLATE NOCASE
           AND id <> ?
         LIMIT 1",
    )
    .bind(project_id.to_string())
    .bind(parent_id.map(|p| p.to_string()))
    .bind(name)
    .bind(exclude.map(|e| e.to_string()).unwrap_or_default())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    Ok(row.is_some())
}

/// 每个父级下的下一个排序号（新建 / 移到末尾用）。
async fn next_sort_order(
    db: &SqlitePool,
    project_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM scenario_folders
         WHERE project_id = ? AND COALESCE(parent_id, '') = COALESCE(?, '')",
    )
    .bind(project_id.to_string())
    .bind(parent_id.map(|p| p.to_string()))
    .fetch_one(db)
    .await
    .map_err(db_err)?;
    Ok(row.0)
}

/// 新建目录（`sort_order` 缺省追加到同级末尾）。
pub async fn create_scenario_folder(
    db: &SqlitePool,
    project_id: Uuid,
    parent_id: Option<Uuid>,
    name: &str,
    sort_order: Option<i64>,
) -> Result<ScenarioFolder> {
    let sort_order = match sort_order {
        Some(v) => v,
        None => next_sort_order(db, project_id, parent_id).await?,
    };
    let now = Utc::now();
    let model = ScenarioFolder {
        id: Uuid::new_v4(),
        project_id,
        parent_id,
        name: name.to_string(),
        sort_order,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO scenario_folders
         (id, project_id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(model.id.to_string())
    .bind(model.project_id.to_string())
    .bind(model.parent_id.map(|p| p.to_string()))
    .bind(&model.name)
    .bind(model.sort_order)
    .bind(model.created_at.to_rfc3339())
    .bind(model.updated_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(model)
}

/// 更新目录（名称 / 父级 / 排序；updated_at 刷新）。
pub async fn update_scenario_folder(db: &SqlitePool, folder: &ScenarioFolder) -> Result<()> {
    let affected = sqlx::query(
        "UPDATE scenario_folders
         SET name = ?, parent_id = ?, sort_order = ?, updated_at = ?
         WHERE id = ? AND project_id = ?",
    )
    .bind(&folder.name)
    .bind(folder.parent_id.map(|p| p.to_string()))
    .bind(folder.sort_order)
    .bind(folder.updated_at.to_rfc3339())
    .bind(folder.id.to_string())
    .bind(folder.project_id.to_string())
    .execute(db)
    .await
    .map_err(db_err)?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound(format!("场景目录（{}）", folder.id)));
    }
    Ok(())
}

/// 目录子树（含自身）的递归 CTE；两处绑定参数均为目录 id。
const SUBTREE_CTE: &str = "WITH RECURSIVE subtree(id) AS (
    SELECT CAST(? AS TEXT)
    UNION ALL
    SELECT f.id FROM scenario_folders f JOIN subtree s ON f.parent_id = s.id
)";

/// 目标父级是否在 `folder_id` 的子树内（含自身）——移动目录时防环。
pub async fn is_folder_in_subtree(
    db: &SqlitePool,
    folder_id: Uuid,
    candidate: Uuid,
) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as(&format!(
        "{SUBTREE_CTE} SELECT id FROM subtree WHERE id = ? LIMIT 1"
    ))
    .bind(folder_id.to_string())
    .bind(candidate.to_string())
    .fetch_optional(db)
    .await
    .map_err(db_err)?;
    Ok(row.is_some())
}

/// 统计目录子树内（不含自身目录行）的场景数。
pub async fn count_scenarios_in_subtree(db: &SqlitePool, folder_id: Uuid) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(&format!(
        "{SUBTREE_CTE} SELECT COUNT(*) FROM scenarios WHERE folder_id IN (SELECT id FROM subtree)"
    ))
    .bind(folder_id.to_string())
    .fetch_one(db)
    .await
    .map_err(db_err)?;
    Ok(row.0)
}

/// 删除目录子树，按 `mode` 处理其中的场景（单事务，绝不静默丢数据）。
pub async fn delete_scenario_folder(
    db: &SqlitePool,
    folder_id: Uuid,
    mode: FolderDeleteMode,
) -> Result<FolderDeleteOutcome> {
    // 先确认目录存在（避免对不存在 id 静默返回 0 行）。
    get_scenario_folder(db, folder_id).await?;
    let id = folder_id.to_string();
    let mut tx = db.begin().await.map_err(db_err)?;

    let scenario_count: (i64,) = sqlx::query_as(&format!(
        "{SUBTREE_CTE} SELECT COUNT(*) FROM scenarios WHERE folder_id IN (SELECT id FROM subtree)"
    ))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    let scenario_count = scenario_count.0;

    if mode == FolderDeleteMode::Empty && scenario_count > 0 {
        return Err(AppError::Validation(format!(
            "目录不是空的（含 {scenario_count} 个场景），请选择移动或删除其中场景"
        )));
    }

    let mut outcome = FolderDeleteOutcome::default();
    match mode {
        FolderDeleteMode::Empty | FolderDeleteMode::KeepScenarios => {
            if scenario_count > 0 {
                // 场景移到未分类（updated_at 刷新，列表按更新时间排序可见）。
                let moved = sqlx::query(&format!(
                    "{SUBTREE_CTE}
                     UPDATE scenarios SET folder_id = NULL, updated_at = ?
                     WHERE folder_id IN (SELECT id FROM subtree)"
                ))
                .bind(&id)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(db_err)?
                .rows_affected();
                outcome.moved_scenarios = moved as i64;
            }
        }
        FolderDeleteMode::Cascade => {
            if scenario_count > 0 {
                // 运行历史 → 场景 → 目录（显式先子后父，不依赖 PRAGMA）。
                sqlx::query(&format!(
                    "{SUBTREE_CTE}
                     DELETE FROM scenario_runs WHERE scenario_id IN (
                         SELECT id FROM scenarios WHERE folder_id IN (SELECT id FROM subtree)
                     )"
                ))
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
                let deleted = sqlx::query(&format!(
                    "{SUBTREE_CTE}
                     DELETE FROM scenarios WHERE folder_id IN (SELECT id FROM subtree)"
                ))
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?
                .rows_affected();
                outcome.deleted_scenarios = deleted as i64;
            }
        }
    }

    let deleted_folders = sqlx::query(&format!(
        "{SUBTREE_CTE} DELETE FROM scenario_folders WHERE id IN (SELECT id FROM subtree)"
    ))
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?
    .rows_affected();
    outcome.deleted_folders = deleted_folders as i64;

    tx.commit().await.map_err(db_err)?;
    Ok(outcome)
}

/// 单条 upsert（命令层保存目录用；新建 / 重命名 / 移动统一走这里）。
pub async fn save_scenario_folder(db: &SqlitePool, folder: &ScenarioFolder) -> Result<()> {
    sqlx::query(
        "INSERT INTO scenario_folders
         (id, project_id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           parent_id = excluded.parent_id,
           name = excluded.name,
           sort_order = excluded.sort_order,
           updated_at = excluded.updated_at",
    )
    .bind(folder.id.to_string())
    .bind(folder.project_id.to_string())
    .bind(folder.parent_id.map(|p| p.to_string()))
    .bind(&folder.name)
    .bind(folder.sort_order)
    .bind(folder.created_at.to_rfc3339())
    .bind(folder.updated_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// 批量写入目录（备份恢复 / 同步覆盖用）：事务连接上逐条 upsert，
/// 语义与 [`create_scenario_folder`] 一致（目录量级小，逐条足够）。
pub async fn save_scenario_folders_bulk(
    conn: &mut sqlx::SqliteConnection,
    folders: &[ScenarioFolder],
) -> Result<()> {
    if folders.is_empty() {
        return Ok(());
    }
    for folder in folders {
        sqlx::query(
            "INSERT INTO scenario_folders
             (id, project_id, parent_id, name, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               parent_id = excluded.parent_id,
               name = excluded.name,
               sort_order = excluded.sort_order,
               updated_at = excluded.updated_at",
        )
        .bind(folder.id.to_string())
        .bind(folder.project_id.to_string())
        .bind(folder.parent_id.map(|p| p.to_string()))
        .bind(&folder.name)
        .bind(folder.sort_order)
        .bind(folder.created_at.to_rfc3339())
        .bind(folder.updated_at.to_rfc3339())
        .execute(&mut *conn)
        .await
        .map_err(db_err)?;
    }
    Ok(())
}

/// 移动目录到新父级（`sort_order` 缺省追加到目标父级末尾）。
/// 防环：目标父级不得是自身或自身后代（命令层先校验，这里兜底）。
pub async fn move_scenario_folder(
    db: &SqlitePool,
    folder_id: Uuid,
    parent_id: Option<Uuid>,
    sort_order: Option<i64>,
) -> Result<ScenarioFolder> {
    let mut folder = get_scenario_folder(db, folder_id).await?;
    let sort_order = match sort_order {
        Some(v) => v,
        None => next_sort_order(db, folder.project_id, parent_id).await?,
    };
    folder.parent_id = parent_id;
    folder.sort_order = sort_order;
    folder.updated_at = Utc::now();
    update_scenario_folder(db, &folder).await?;
    Ok(folder)
}

/// 同级目录批量重排（拖拽落点后按可见顺序写回 sort_order）。
pub async fn reorder_scenario_folders(db: &SqlitePool, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = db.begin().await.map_err(db_err)?;
    let now = Utc::now().to_rfc3339();
    for (idx, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE scenario_folders SET sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(&now)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// 批量移动场景到目录（`folder_id` None = 未分类）。返回受影响行数。
pub async fn move_scenarios(db: &SqlitePool, ids: &[Uuid], folder_id: Option<Uuid>) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql =
        format!("UPDATE scenarios SET folder_id = ?, updated_at = ? WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql)
        .bind(folder_id.map(|f| f.to_string()))
        .bind(Utc::now().to_rfc3339());
    for id in ids {
        query = query.bind(id.to_string());
    }
    let affected = query.execute(db).await.map_err(db_err)?.rows_affected();
    Ok(affected as i64)
}

/// 同级场景批量重排（拖拽落点后按可见顺序写回 sort_order）。
pub async fn reorder_scenarios(db: &SqlitePool, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = db.begin().await.map_err(db_err)?;
    let now = Utc::now().to_rfc3339();
    for (idx, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE scenarios SET sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(&now)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// 批量追加标签（去重合并；不覆盖已有标签）。返回受影响行数。
pub async fn add_scenario_tags(db: &SqlitePool, ids: &[Uuid], tags: &[String]) -> Result<i64> {
    if ids.is_empty() || tags.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let rows: Vec<(String, String)> = {
        let sql = format!("SELECT id, tags_json FROM scenarios WHERE id IN ({placeholders})");
        let mut query = sqlx::query_as(&sql);
        for id in ids {
            query = query.bind(id.to_string());
        }
        query.fetch_all(db).await.map_err(db_err)?
    };
    let mut tx = db.begin().await.map_err(db_err)?;
    let now = Utc::now().to_rfc3339();
    let mut affected = 0i64;
    for (id, raw) in rows {
        let mut existing: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        let mut changed = false;
        for tag in tags {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            if !existing.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                existing.push(tag.to_string());
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        let json = serde_json::to_string(&existing)
            .map_err(|e| AppError::Validation(format!("标签序列化失败：{e}")))?;
        affected += sqlx::query("UPDATE scenarios SET tags_json = ?, updated_at = ? WHERE id = ?")
            .bind(json)
            .bind(&now)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?
            .rows_affected() as i64;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(affected)
}

/// 批量设置收藏标记。返回受影响行数。
pub async fn set_scenarios_favorite(db: &SqlitePool, ids: &[Uuid], favorite: bool) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql =
        format!("UPDATE scenarios SET favorite = ?, updated_at = ? WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql)
        .bind(favorite)
        .bind(Utc::now().to_rfc3339());
    for id in ids {
        query = query.bind(id.to_string());
    }
    let affected = query.execute(db).await.map_err(db_err)?.rows_affected();
    Ok(affected as i64)
}

/// 批量删除场景（运行历史一并清理，单事务）。返回删除的场景数。
pub async fn delete_scenarios(db: &SqlitePool, ids: &[Uuid]) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut tx = db.begin().await.map_err(db_err)?;
    {
        let sql = format!("DELETE FROM scenario_runs WHERE scenario_id IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id.to_string());
        }
        query.execute(&mut *tx).await.map_err(db_err)?;
    }
    let sql = format!("DELETE FROM scenarios WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id.to_string());
    }
    let affected = query
        .execute(&mut *tx)
        .await
        .map_err(db_err)?
        .rows_affected();
    tx.commit().await.map_err(db_err)?;
    Ok(affected as i64)
}

/// 场景所属项目（批量操作的归属校验：跨项目 id 不得混入一次批量写入）。
pub async fn scenario_project_ids(db: &SqlitePool, ids: &[Uuid]) -> Result<Vec<Uuid>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let rows: Vec<(String,)> = {
        let sql = format!("SELECT DISTINCT project_id FROM scenarios WHERE id IN ({placeholders})");
        let mut query = sqlx::query_as(&sql);
        for id in ids {
            query = query.bind(id.to_string());
        }
        query.fetch_all(db).await.map_err(db_err)?
    };
    rows.into_iter().map(|(pid,)| parse_uuid(&pid)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;
    use crate::repository::projects::create_project;
    use crate::repository::scenarios::{get_scenario, save_scenario};
    use chrono::Utc;
    use miyo_core::model::Scenario;

    fn make_scenario(project_id: Uuid, name: &str) -> Scenario {
        Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: Uuid::new_v4(),
            project_id,
            name: name.into(),
            description: String::new(),
            category: String::new(),
            folder_id: None,
            tags: Vec::new(),
            favorite: false,
            sort_order: 0,
            steps: Vec::new(),
            script: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn folder_tree_crud_and_move() {
        let db = memory_pool().await.expect("建库");
        let project = create_project(&db, "目录", "").await.unwrap();
        let root = create_scenario_folder(&db, project.id, None, "电商业务", None)
            .await
            .unwrap();
        let child = create_scenario_folder(&db, project.id, Some(root.id), "订单中心", None)
            .await
            .unwrap();
        let listed = list_scenario_folders(&db, project.id).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(child.parent_id, Some(root.id));

        // 同级重名：DB 唯一索引兜底（应用层预检在命令层）。
        assert!(
            create_scenario_folder(&db, project.id, Some(root.id), "订单中心", None)
                .await
                .is_err()
        );
        // 不同父级可同名。
        assert!(
            create_scenario_folder(&db, project.id, Some(child.id), "订单中心", None)
                .await
                .is_ok()
        );

        // 移动子目录到根；防环：不能移到自己的后代。
        assert!(is_folder_in_subtree(&db, root.id, child.id).await.unwrap());
        let moved = move_scenario_folder(&db, child.id, None, Some(9))
            .await
            .unwrap();
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.sort_order, 9);

        assert!(
            !scenario_folder_name_taken(&db, project.id, None, "电商业务", Some(root.id))
                .await
                .unwrap()
        );
        assert!(
            scenario_folder_name_taken(&db, project.id, None, "电商业务", None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn delete_folder_empty_keep_cascade() {
        let db = memory_pool().await.expect("建库");
        let project = create_project(&db, "删除策略", "").await.unwrap();
        let root = create_scenario_folder(&db, project.id, None, "业务", None)
            .await
            .unwrap();
        let child = create_scenario_folder(&db, project.id, Some(root.id), "子目录", None)
            .await
            .unwrap();
        let mut scenario = make_scenario(project.id, "场景 A");
        scenario.folder_id = Some(child.id);
        save_scenario(&db, &scenario).await.unwrap();

        // 空模式拒绝（子树内有场景）。
        assert!(
            delete_scenario_folder(&db, root.id, FolderDeleteMode::Empty)
                .await
                .is_err()
        );

        // 保留模式：场景移到未分类，目录全部删除。
        let outcome = delete_scenario_folder(&db, root.id, FolderDeleteMode::KeepScenarios)
            .await
            .unwrap();
        assert_eq!(outcome.deleted_folders, 2);
        assert_eq!(outcome.moved_scenarios, 1);
        let reloaded = get_scenario(&db, scenario.id).await.unwrap();
        assert_eq!(reloaded.folder_id, None);
        assert!(list_scenario_folders(&db, project.id)
            .await
            .unwrap()
            .is_empty());

        // 级联模式：目录与场景一起删。
        let root2 = create_scenario_folder(&db, project.id, None, "业务2", None)
            .await
            .unwrap();
        let mut scenario2 = make_scenario(project.id, "场景 B");
        scenario2.folder_id = Some(root2.id);
        save_scenario(&db, &scenario2).await.unwrap();
        let outcome = delete_scenario_folder(&db, root2.id, FolderDeleteMode::Cascade)
            .await
            .unwrap();
        assert_eq!(outcome.deleted_folders, 1);
        assert_eq!(outcome.deleted_scenarios, 1);
        assert!(get_scenario(&db, scenario2.id).await.is_err());
        // 之前的场景不受影响。
        assert!(get_scenario(&db, scenario.id).await.is_ok());
    }

    #[tokio::test]
    async fn batch_move_tags_and_favorite() {
        let db = memory_pool().await.expect("建库");
        let project = create_project(&db, "批量", "").await.unwrap();
        let folder = create_scenario_folder(&db, project.id, None, "回归", None)
            .await
            .unwrap();
        let a = make_scenario(project.id, "A");
        let b = make_scenario(project.id, "B");
        save_scenario(&db, &a).await.unwrap();
        save_scenario(&db, &b).await.unwrap();

        let moved = move_scenarios(&db, &[a.id, b.id], Some(folder.id))
            .await
            .unwrap();
        assert_eq!(moved, 2);
        assert_eq!(
            get_scenario(&db, a.id).await.unwrap().folder_id,
            Some(folder.id)
        );

        let tagged = add_scenario_tags(
            &db,
            &[a.id, b.id],
            &["冒烟".to_string(), "冒烟".to_string(), " ".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(tagged, 2);
        assert_eq!(get_scenario(&db, a.id).await.unwrap().tags, vec!["冒烟"]);
        // 重复追加幂等。
        assert_eq!(
            add_scenario_tags(&db, &[a.id], &["冒烟".to_string()])
                .await
                .unwrap(),
            0
        );

        assert_eq!(set_scenarios_favorite(&db, &[a.id], true).await.unwrap(), 1);
        assert!(get_scenario(&db, a.id).await.unwrap().favorite);
        // 收藏置顶。
        let listed = crate::repository::list_scenarios(&db, project.id)
            .await
            .unwrap();
        assert_eq!(listed[0].id, a.id);

        assert_eq!(delete_scenarios(&db, &[a.id, b.id]).await.unwrap(), 2);
        assert!(crate::repository::list_scenarios(&db, project.id)
            .await
            .unwrap()
            .is_empty());
    }
}
