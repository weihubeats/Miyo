//! Project CRUD。

use std::collections::HashMap;

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{Project, DEFAULT_TEAM_ID};
use miyo_core::{AppError, Result};

use super::rows::ProjectRow;

pub async fn create_project(db: &SqlitePool, name: &str, description: &str) -> Result<Project> {
    let now = Utc::now();
    let model = Project {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: description.to_string(),
        variables: HashMap::new(),
        // 未显式指定团队的项目归入默认团队。
        team_id: Some(DEFAULT_TEAM_ID),
        created_at: now,
        updated_at: now,
    };
    // 新项目排在末尾（sort_order 取当前最大值 + 1）。
    let sort_order: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects")
            .fetch_one(db)
            .await?;
    let row = ProjectRow::from_model(&model)?;
    sqlx::query(
        "INSERT INTO projects (id, name, description, variables_json, created_at, updated_at, sort_order, team_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.name)
    .bind(&row.description)
    .bind(row.variables_json.clone())
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .bind(sort_order)
    .bind(row.team_id.clone())
    .execute(db)
    .await?;
    Ok(model)
}

pub async fn list_projects<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
) -> Result<Vec<Project>> {
    let rows: Vec<ProjectRow> = sqlx::query_as(
        "SELECT id, name, description, variables_json, team_id, created_at, updated_at
         FROM projects ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(ProjectRow::into_model).collect()
}

/// 项目级接口统计（仪表板用）：每个项目的接口总数 + 最近更新的一个接口。
///
/// 单条 SQL（窗口函数）替代「逐项目 list_endpoints 拉全量再在前端聚合」，
/// 接口含完整请求体时后者会把全部 body 传回前端，项目一多启动即卡。
/// scenario_count / request_count 同理：场景与请求历史的总数用相关子查询
/// 一并带出，供首页欢迎卡与项目卡片展示（零迁移，纯查询扩展）。
///
/// ok_count / fail_count 是 request_count 的拆分（2xx/3xx 成功，其余及
/// 无状态码的网络错误算失败），两者之和等于 request_count——首页请求数
/// 与成功/失败明细同源，不会出现「总数 0 却有失败记录」。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectEndpointStat {
    pub project_id: String,
    pub endpoint_count: i64,
    pub scenario_count: i64,
    pub request_count: i64,
    pub ok_count: i64,
    pub fail_count: i64,
    pub latest_method: Option<String>,
    pub latest_path: Option<String>,
}

pub async fn list_endpoint_stats(db: &SqlitePool) -> Result<Vec<ProjectEndpointStat>> {
    // 以 projects 驱动：零接口的项目也返回（count=0 / latest=NULL）。
    let stats: Vec<ProjectEndpointStat> = sqlx::query_as(
        "SELECT p.id AS project_id,
                (SELECT COUNT(*) FROM endpoints e WHERE e.project_id = p.id) AS endpoint_count,
                (SELECT COUNT(*) FROM scenarios s WHERE s.project_id = p.id) AS scenario_count,
                (SELECT COUNT(*) FROM request_histories rh WHERE rh.project_id = p.id)
                    AS request_count,
                (SELECT COUNT(*) FROM request_histories rh
                  WHERE rh.project_id = p.id AND rh.status IS NOT NULL AND rh.status < 400)
                    AS ok_count,
                (SELECT COUNT(*) FROM request_histories rh
                  WHERE rh.project_id = p.id AND (rh.status IS NULL OR rh.status >= 400))
                    AS fail_count,
                (SELECT e.method FROM endpoints e WHERE e.project_id = p.id
                  ORDER BY e.updated_at DESC LIMIT 1) AS latest_method,
                (SELECT e.path FROM endpoints e WHERE e.project_id = p.id
                  ORDER BY e.updated_at DESC LIMIT 1) AS latest_path
         FROM projects p",
    )
    .fetch_all(db)
    .await?;
    Ok(stats)
}

/// 拖拽排序持久化：按给定 id 顺序（事务）批量重写 sort_order。
/// 任一 id 不存在则整体回滚并报错，避免前端的脏顺序污染数据库。
pub async fn update_projects_order(db: &SqlitePool, project_ids: &[Uuid]) -> Result<()> {
    let mut tx = db.begin().await?;
    for (index, id) in project_ids.iter().enumerate() {
        let result = sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(index as i64)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!("项目（{id}）")));
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn get_project(db: &SqlitePool, project_id: Uuid) -> Result<Project> {
    let row: Option<ProjectRow> = sqlx::query_as(
        "SELECT id, name, description, variables_json, team_id, created_at, updated_at
         FROM projects WHERE id = ?",
    )
    .bind(project_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(ProjectRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("项目（{project_id}）")))
}

pub async fn update_project(db: &SqlitePool, project: &Project) -> Result<Project> {
    let mut updated = project.clone();
    updated.updated_at = Utc::now();
    let row = ProjectRow::from_model(&updated)?;
    let result = sqlx::query(
        "UPDATE projects SET name = ?, description = ?, variables_json = ?, updated_at = ?, team_id = ?
         WHERE id = ?",
    )
    .bind(&row.name)
    .bind(&row.description)
    .bind(row.variables_json.clone())
    .bind(row.updated_at.clone())
    .bind(&row.team_id)
    .bind(&row.id)
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("项目（{}）", project.id)));
    }
    Ok(updated)
}

pub async fn delete_project(db: &SqlitePool, project_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(project_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 同步拉取覆盖：删除项目全部子对象（文件夹树 CASCADE 一并清掉），
/// 供快照整体重写前清场。项目本体保留。
pub async fn delete_project_children(
    executor: &mut sqlx::SqliteConnection,
    project_id: Uuid,
) -> Result<()> {
    let pid = project_id.to_string();
    // 文件夹级联（子文件夹 + 其下接口）由外键 ON DELETE CASCADE 完成；
    // 这里显式删各顶层表，覆盖未走 CASCADE 的历史边角。
    for table in [
        "environments",
        "mock_rules",
        "proto_files",
        "request_histories",
        "test_runs",
        // 运行历史在场景之前删（scenario_runs → scenarios 外键，先子后父）。
        "scenario_runs",
        "scenarios",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE project_id = ?"))
            .bind(&pid)
            .execute(&mut *executor)
            .await?;
    }
    // 接口及其从属（示例/用例）：先按 project_id 删接口，从属表 CASCADE。
    sqlx::query("DELETE FROM endpoints WHERE project_id = ?")
        .bind(&pid)
        .execute(&mut *executor)
        .await?;
    // 文件夹最后删（接口已随 endpoints 表删净）。
    sqlx::query("DELETE FROM folders WHERE project_id = ?")
        .bind(&pid)
        .execute(&mut *executor)
        .await?;
    Ok(())
}

/// 备份恢复：按给定 id 原样写入项目。
/// 带 id：原样写入项目（upsert，同一 id 重复保存时更新而非报主键冲突）。
pub async fn save_project<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    project: &Project,
) -> Result<()> {
    let row = ProjectRow::from_model(project)?;
    sqlx::query(
        "INSERT INTO projects (id, name, description, variables_json, created_at, updated_at, team_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            variables_json = excluded.variables_json,
            updated_at = excluded.updated_at,
            team_id = excluded.team_id",
    )
    .bind(&row.id)
    .bind(&row.name)
    .bind(&row.description)
    .bind(&row.variables_json)
    .bind(row.created_at.clone())
    .bind(row.updated_at.clone())
    .bind(&row.team_id)
    .execute(executor)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;
    use crate::repository::endpoints::save_endpoint;
    use crate::repository::history::save_request_history;
    use crate::repository::scenarios::save_scenario;
    use miyo_core::model::{
        Endpoint, EndpointStatus, HttpMethod, RequestHistory, RequestSpec, Scenario,
    };

    fn endpoint_for(project_id: Uuid, name: &str, method: HttpMethod, path: &str) -> Endpoint {
        Endpoint {
            id: Uuid::new_v4(),
            project_id,
            folder_id: None,
            name: name.into(),
            method,
            path: path.into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// 仪表板统计：零接口项目 count=0/latest=NULL；多接口项目取最近更新的一条。
    /// 场景数 / 请求历史数一并带出（欢迎卡与项目卡片的数据源）。
    #[tokio::test]
    async fn list_endpoint_stats_counts_and_latest() {
        let db = memory_pool().await.expect("建库");
        let empty = create_project(&db, "空项目", "").await.unwrap();
        let busy = create_project(&db, "活跃项目", "").await.unwrap();

        save_endpoint(
            &db,
            &endpoint_for(busy.id, "旧接口", HttpMethod::GET, "/old"),
        )
        .await
        .unwrap();
        // 保证 updated_at 严格更大（RFC3339 精度依赖时钟推进）
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // save_endpoint 对带 id 的记录做 upsert，updated_at 刷新为当前时间
        let newer = endpoint_for(busy.id, "新接口", HttpMethod::POST, "/latest");
        save_endpoint(&db, &newer).await.unwrap();

        // 活跃项目补 2 个场景 + 1 条请求历史；空项目保持全零
        for name in ["下单链路", "支付链路"] {
            save_scenario(
                &db,
                &Scenario {
                    id: Uuid::new_v4(),
                    project_id: busy.id,
                    name: name.into(),
                    description: String::new(),
                    sort_order: 0,
                    steps: Vec::new(),
                    script: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        }
        save_request_history(
            &db,
            &RequestHistory {
                id: Uuid::new_v4(),
                project_id: busy.id,
                endpoint_id: Some(newer.id),
                method: "POST".into(),
                url: "https://api.example.com/latest".into(),
                status: Some(200),
                duration_ms: Some(12),
                request_summary_json: "{}".into(),
                response_summary_json: "{}".into(),
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();

        let stats = list_endpoint_stats(&db).await.unwrap();
        assert_eq!(stats.len(), 2);

        let empty_stat = stats
            .iter()
            .find(|s| s.project_id == empty.id.to_string())
            .unwrap();
        assert_eq!(empty_stat.endpoint_count, 0);
        assert_eq!(empty_stat.scenario_count, 0);
        assert_eq!(empty_stat.request_count, 0);
        assert_eq!(empty_stat.ok_count, 0);
        assert_eq!(empty_stat.fail_count, 0);
        assert!(empty_stat.latest_method.is_none());
        assert!(empty_stat.latest_path.is_none());

        let busy_stat = stats
            .iter()
            .find(|s| s.project_id == busy.id.to_string())
            .unwrap();
        assert_eq!(busy_stat.endpoint_count, 2);
        assert_eq!(busy_stat.scenario_count, 2);
        assert_eq!(busy_stat.request_count, 1);
        // 成功/失败是 request_count 的拆分：200 → ok，两者相加等于总数
        assert_eq!(busy_stat.ok_count, 1);
        assert_eq!(busy_stat.fail_count, 0);
        assert_eq!(
            busy_stat.ok_count + busy_stat.fail_count,
            busy_stat.request_count
        );
        assert_eq!(busy_stat.latest_method.as_deref(), Some("POST"));
        assert_eq!(busy_stat.latest_path.as_deref(), Some("/latest"));
    }

    #[tokio::test]
    async fn update_projects_order_persists_and_orders_list() {
        let db = memory_pool().await.expect("建库");
        let a = create_project(&db, "A", "").await.unwrap();
        let b = create_project(&db, "B", "").await.unwrap();
        let c = create_project(&db, "C", "").await.unwrap();

        // 新项目排末尾：A, B, C。
        let ids: Vec<String> = list_projects(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id.to_string())
            .collect();
        assert_eq!(ids, [a.id.to_string(), b.id.to_string(), c.id.to_string()]);

        // 拖拽成 C, A, B → 持久化并保持新序。
        update_projects_order(&db, &[c.id, a.id, b.id])
            .await
            .unwrap();
        let ids: Vec<String> = list_projects(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id.to_string())
            .collect();
        assert_eq!(ids, [c.id.to_string(), a.id.to_string(), b.id.to_string()]);

        // 不存在的 id → 整体回滚，原顺序不变。
        let missing = Uuid::new_v4();
        assert!(update_projects_order(&db, &[missing, a.id]).await.is_err());
        let ids: Vec<String> = list_projects(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id.to_string())
            .collect();
        assert_eq!(ids, [c.id.to_string(), a.id.to_string(), b.id.to_string()]);
    }
}
