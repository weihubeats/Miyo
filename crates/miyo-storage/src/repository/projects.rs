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

/// 项目级运行质量聚合（首页项目卡「测试概览」）：场景与用例的最近运行结果。
///
/// 口径（只统计真实落库数据，无运行记录返回 0/NULL，前端显示「未运行」）：
/// - 场景：每场景最近一次运行（窗口函数 rn=1，与 scenario_runs 的
///   `latest_run_summaries_for_project` 同范式），成功 = 未取消且零失败且有通过
///   步骤；取消或有失败计失败。
///   全 skipped 的运行不计入任一侧（既不算通过也不算失败）。
/// - 用例：`test_cases.last_run_status` 汇总（'Untested'/'Success'/'Failed'，
///   由 update_test_case_run_summary 回写），最近失败时间取 Failed 行的
///   last_run_at 最大值。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectQualityStatRow {
    pub project_id: String,
    pub scenarios_total: i64,
    /// 有过至少一次运行记录的场景数。
    pub scenarios_ran: i64,
    /// 最近一次运行通过的场景数。
    pub scenario_last_ok: i64,
    /// 最近一次运行失败/被取消的场景数。
    pub scenario_last_fail: i64,
    /// 全项目最近一次场景运行失败的时刻（RFC3339；无失败为 NULL）。
    pub scenario_last_failed_at: Option<String>,
    pub cases_total: i64,
    pub cases_success: i64,
    pub cases_failed: i64,
    pub cases_untested: i64,
    /// 全项目最近一次用例运行失败的时刻（RFC3339；无失败为 NULL）。
    pub case_last_failed_at: Option<String>,
}

pub async fn list_project_quality(db: &SqlitePool) -> Result<Vec<ProjectQualityStatRow>> {
    let rows: Vec<ProjectQualityStatRow> = sqlx::query_as(
        "SELECT p.id AS project_id,
                COALESCE(s.scenarios_total, 0) AS scenarios_total,
                COALESCE(s.scenarios_ran, 0) AS scenarios_ran,
                COALESCE(s.last_ok, 0) AS scenario_last_ok,
                COALESCE(s.last_fail, 0) AS scenario_last_fail,
                s.last_failed_at AS scenario_last_failed_at,
                COALESCE(c.cases_total, 0) AS cases_total,
                COALESCE(c.cases_success, 0) AS cases_success,
                COALESCE(c.cases_failed, 0) AS cases_failed,
                COALESCE(c.cases_untested, 0) AS cases_untested,
                c.last_failed_at AS case_last_failed_at
         FROM projects p
         LEFT JOIN (
             SELECT sc.project_id,
                    COUNT(*) AS scenarios_total,
                    COUNT(r.scenario_id) AS scenarios_ran,
                    SUM(CASE WHEN r.cancelled = 0 AND r.fail_count = 0 AND r.ok_count > 0
                             THEN 1 ELSE 0 END) AS last_ok,
                    SUM(CASE WHEN r.cancelled = 1 OR r.fail_count > 0
                             THEN 1 ELSE 0 END) AS last_fail,
                    MAX(CASE WHEN r.cancelled = 1 OR r.fail_count > 0
                             THEN r.finished_at END) AS last_failed_at
             FROM scenarios sc
             LEFT JOIN (
                 SELECT sr.scenario_id, sr.ok_count, sr.fail_count, sr.cancelled,
                        sr.finished_at,
                        ROW_NUMBER() OVER (
                            PARTITION BY sr.scenario_id ORDER BY sr.finished_at DESC
                        ) AS rn
                 FROM scenario_runs sr
             ) r ON r.scenario_id = sc.id AND r.rn = 1
             GROUP BY sc.project_id
         ) s ON s.project_id = p.id
         LEFT JOIN (
             SELECT e.project_id,
                    COUNT(*) AS cases_total,
                    SUM(CASE WHEN tc.last_run_status = 'Success' THEN 1 ELSE 0 END)
                        AS cases_success,
                    SUM(CASE WHEN tc.last_run_status = 'Failed' THEN 1 ELSE 0 END)
                        AS cases_failed,
                    SUM(CASE WHEN tc.last_run_status = 'Untested' THEN 1 ELSE 0 END)
                        AS cases_untested,
                    MAX(CASE WHEN tc.last_run_status = 'Failed' THEN tc.last_run_at END)
                        AS last_failed_at
             FROM test_cases tc
             JOIN endpoints e ON e.id = tc.request_id
             GROUP BY e.project_id
         ) c ON c.project_id = p.id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
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
                    setup: Vec::new(),
                    teardown: Vec::new(),
                    teardown_enabled: true,
                    data_sets: Vec::new(),
                    id: Uuid::new_v4(),
                    project_id: busy.id,
                    name: name.into(),
                    description: String::new(),
                    category: String::new(),
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

    /// 质量聚合：场景按「每场景最近一次运行」判定通过/失败（旧运行不参与），
    /// 用例按 last_run_status 分布；无运行记录显示为未运行而非 0 成功率。
    #[tokio::test]
    async fn list_project_quality_aggregates_latest_runs() {
        use crate::repository::scenario_runs::save_scenario_run;
        use crate::repository::test_case_runs::save_test_case_run;
        use crate::repository::test_cases::{create_test_case, update_test_case_run_summary};
        use miyo_core::model::{ScenarioRun, TestCase, TestCaseStatus};

        let db = memory_pool().await.expect("建库");
        let quiet = create_project(&db, "安静项目", "").await.unwrap();
        let active = create_project(&db, "活跃项目", "").await.unwrap();

        // 活跃项目：场景 A 最近失败、场景 B 最近通过（各跑两次，只看最新一次）。
        let scenario = |name: &str| Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: Uuid::new_v4(),
            project_id: active.id,
            name: name.into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: Vec::new(),
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let sa = scenario("场景A");
        let sb = scenario("场景B");
        save_scenario(&db, &sa).await.unwrap();
        save_scenario(&db, &sb).await.unwrap();

        let run = |scenario_id: Uuid, ok: i64, fail: i64, cancelled: bool| ScenarioRun {
            id: Uuid::new_v4(),
            scenario_id,
            environment_id: None,
            finished_at: chrono::Utc::now(),
            duration_ms: Some(10.0),
            total_steps: ok + fail,
            ok_count: ok,
            fail_count: fail,
            skipped_count: 0,
            cancelled,
            stopped_on_failure: false,
            results_json: "{}".into(),
            variables_json: "{}".into(),
            meta_json: None,
        };
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        save_scenario_run(&db, &run(sa.id, 3, 0, false))
            .await
            .unwrap(); // 旧：通过
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        save_scenario_run(&db, &run(sa.id, 1, 2, false))
            .await
            .unwrap(); // 新：失败
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        save_scenario_run(&db, &run(sb.id, 2, 0, false))
            .await
            .unwrap(); // 通过

        // 活跃项目：用例 1 通过、用例 2 失败、用例 3 未运行；安静项目无用例。
        let endpoint = endpoint_for(active.id, "被测接口", HttpMethod::GET, "/case");
        save_endpoint(&db, &endpoint).await.unwrap();
        let mk_case = |name: &str| TestCase {
            id: Uuid::new_v4(),
            request_id: endpoint.id,
            name: name.into(),
            category: "正向".into(),
            folder: String::new(),
            method: HttpMethod::GET,
            url_path: "/case".into(),
            path_variables: Vec::new(),
            params: Vec::new(),
            headers: Vec::new(),
            body_type: "none".into(),
            body_content: String::new(),
            assertions: serde_json::json!([]),
            extractors: serde_json::json!([]),
            auth: None,
            timeout_ms: None,
            disable_cookies: false,
            sort_order: 0,
            last_run_status: TestCaseStatus::Untested,
            last_run_at: None,
            last_run_ok: None,
            last_run_fail: None,
            created_at: chrono::Utc::now(),
        };
        let c1 = mk_case("用例1");
        let c2 = mk_case("用例2");
        let c3 = mk_case("用例3");
        for c in [&c1, &c2, &c3] {
            create_test_case(&db, c).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        update_test_case_run_summary(
            &db,
            c1.id,
            TestCaseStatus::Success,
            chrono::Utc::now(),
            Some(2),
            Some(0),
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let failed_at = chrono::Utc::now();
        update_test_case_run_summary(
            &db,
            c2.id,
            TestCaseStatus::Failed,
            failed_at,
            Some(0),
            Some(1),
        )
        .await
        .unwrap();
        save_test_case_run(
            &db,
            &miyo_core::model::TestCaseRun {
                id: Uuid::new_v4(),
                case_id: c2.id,
                endpoint_id: Some(endpoint.id),
                environment_id: None,
                started_at: failed_at,
                finished_at: failed_at,
                duration_ms: Some(5.0),
                status: TestCaseStatus::Failed,
                status_code: Some(500),
                ok_count: 0,
                fail_count: 1,
                error: None,
                results_json: "{}".into(),
            },
        )
        .await
        .unwrap();

        let rows = list_project_quality(&db).await.unwrap();
        assert_eq!(rows.len(), 2);

        let quiet_row = rows
            .iter()
            .find(|r| r.project_id == quiet.id.to_string())
            .unwrap();
        assert_eq!(quiet_row.scenarios_total, 0);
        assert_eq!(quiet_row.scenarios_ran, 0);
        assert_eq!(quiet_row.scenario_last_ok, 0);
        assert_eq!(quiet_row.scenario_last_fail, 0);
        assert!(quiet_row.scenario_last_failed_at.is_none());
        assert_eq!(quiet_row.cases_total, 0);
        assert!(quiet_row.case_last_failed_at.is_none());

        let active_row = rows
            .iter()
            .find(|r| r.project_id == active.id.to_string())
            .unwrap();
        assert_eq!(active_row.scenarios_total, 2);
        assert_eq!(active_row.scenarios_ran, 2);
        // 场景 A 只看最新一次（失败），场景 B 通过 → 1 通过 / 1 失败。
        assert_eq!(active_row.scenario_last_ok, 1);
        assert_eq!(active_row.scenario_last_fail, 1);
        assert!(active_row.scenario_last_failed_at.is_some());
        assert_eq!(active_row.cases_total, 3);
        assert_eq!(active_row.cases_success, 1);
        assert_eq!(active_row.cases_failed, 1);
        assert_eq!(active_row.cases_untested, 1);
        assert!(active_row.case_last_failed_at.is_some());
    }
}
