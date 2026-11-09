//! TestCase（测试用例）仓储：CRUD / 分组 / 排序 / 断言提取配置 / 运行状态与摘要。

use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use miyo_core::model::{HttpMethod, KeyValue, TestCase, TestCaseStatus};
use miyo_core::Result;

/// 新建测试用例。
pub async fn create_test_case(db: &SqlitePool, case: &TestCase) -> Result<TestCase> {
    sqlx::query(
        "INSERT INTO test_cases
             (id, request_id, name, category, folder, method, url_path, path_variables, params, headers,
              body_type, body_content, assertions, extractors, auth, timeout_ms, disable_cookies,
              sort_order, last_run_status, last_run_at, last_run_ok, last_run_fail, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(case.id.to_string())
    .bind(case.request_id.to_string())
    .bind(&case.name)
    .bind(&case.category)
    .bind(&case.folder)
    .bind(case.method.as_str())
    .bind(&case.url_path)
    .bind(serde_json::to_string(&case.path_variables).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(&case.params).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(&case.headers).map_err(miyo_core::AppError::Json)?)
    .bind(&case.body_type)
    .bind(&case.body_content)
    .bind(case.assertions.to_string())
    .bind(case.extractors.to_string())
    .bind(opt_json(&case.auth)?)
    .bind(case.timeout_ms.map(|v| v as i64))
    .bind(case.disable_cookies)
    .bind(case.sort_order)
    .bind(case.last_run_status.as_str())
    .bind(case.last_run_at.map(|t| t.to_rfc3339()))
    .bind(case.last_run_ok.map(|v| v as i64))
    .bind(case.last_run_fail.map(|v| v as i64))
    .bind(case.created_at.to_rfc3339())
    .execute(db)
    .await?;
    Ok(case.clone())
}

/// Option<AuthSpec> → JSON 字符串（None 存 NULL）。
fn opt_json<T: serde::Serialize>(v: &Option<T>) -> Result<Option<String>> {
    match v {
        Some(v) => Ok(Some(
            serde_json::to_string(v).map_err(miyo_core::AppError::Json)?,
        )),
        None => Ok(None),
    }
}

const CASE_COLUMNS: &str =
    "id, request_id, name, category, folder, method, url_path, path_variables, params, headers,
                body_type, body_content, assertions, extractors, auth, timeout_ms, disable_cookies,
                sort_order, last_run_status, last_run_at, last_run_ok, last_run_fail, created_at";

/// 列出接口的全部测试用例（组内按 sort_order, created_at 稳定排序）。
pub async fn list_test_cases(db: &SqlitePool, request_id: Uuid) -> Result<Vec<TestCase>> {
    let rows: Vec<TestCaseRow> = sqlx::query_as(&format!(
        "SELECT {CASE_COLUMNS}
             FROM test_cases WHERE request_id = ? ORDER BY sort_order, created_at"
    ))
    .bind(request_id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(TestCaseRow::into_model).collect()
}

/// 批量列出多个接口的测试用例（一次查询，按接口分组交给调用方；
/// 冒烟文档导出等去 N+1 用）。
///
/// SQLite 变量数上限按 500 分片，避免超长 IN 列表；排序按
/// `request_id, sort_order, created_at`，保证每个接口内部与 [`list_test_cases`] 一致。
pub async fn list_test_cases_by_endpoints(
    db: &SqlitePool,
    request_ids: &[Uuid],
) -> Result<Vec<TestCase>> {
    let mut out = Vec::new();
    for chunk in request_ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new(&format!(
            "SELECT {CASE_COLUMNS} FROM test_cases WHERE request_id IN ("
        ));
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(id.to_string());
        }
        separated.push_unseparated(") ORDER BY request_id, sort_order, created_at");
        let rows: Vec<TestCaseRow> = qb.build_query_as().fetch_all(db).await?;
        for row in rows {
            out.push(row.into_model()?);
        }
    }
    Ok(out)
}

/// 取单条测试用例。
pub async fn get_test_case(db: &SqlitePool, case_id: Uuid) -> Result<Option<TestCase>> {
    let row: Option<TestCaseRow> = sqlx::query_as(&format!(
        "SELECT {CASE_COLUMNS} FROM test_cases WHERE id = ?"
    ))
    .bind(case_id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(|r| r.into_model()).transpose()
}

/// 删除单条测试用例。
pub async fn delete_test_case(db: &SqlitePool, case_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM test_cases WHERE id = ?")
        .bind(case_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 更新用例名称 / 分组 / 文件夹（「编辑」入口）。
pub async fn update_test_case_meta(
    db: &SqlitePool,
    case_id: Uuid,
    name: &str,
    category: &str,
    folder: &str,
) -> Result<()> {
    sqlx::query("UPDATE test_cases SET name = ?, category = ?, folder = ? WHERE id = ?")
        .bind(name)
        .bind(category)
        .bind(folder)
        .bind(case_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 重命名接口下的一个用例文件夹（整体改名，未命中无副作用）。
pub async fn rename_test_case_folder(
    db: &SqlitePool,
    request_id: Uuid,
    old_folder: &str,
    new_folder: &str,
) -> Result<u64> {
    let result =
        sqlx::query("UPDATE test_cases SET folder = ? WHERE request_id = ? AND folder = ?")
            .bind(new_folder)
            .bind(request_id.to_string())
            .bind(old_folder)
            .execute(db)
            .await?;
    Ok(result.rows_affected())
}

/// 批量设置用例文件夹（行菜单「移动到分组」/ 拖拽跨组；空串 = 移出分组）。
pub async fn set_test_cases_folder(
    db: &SqlitePool,
    case_ids: &[Uuid],
    folder: &str,
) -> Result<u64> {
    let mut affected = 0u64;
    for chunk in case_ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new("UPDATE test_cases SET folder = ");
        qb.push_bind(folder.to_string());
        qb.push(" WHERE id IN (");
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(id.to_string());
        }
        separated.push_unseparated(")");
        affected += qb.build().execute(db).await?.rows_affected();
    }
    Ok(affected)
}

/// 拖拽排序：按传入顺序写回组内 sort_order（0,1,2…；未列出的用例保持原值）。
pub async fn reorder_test_cases(db: &SqlitePool, ordered_ids: &[Uuid]) -> Result<()> {
    for (idx, id) in ordered_ids.iter().enumerate() {
        sqlx::query("UPDATE test_cases SET sort_order = ? WHERE id = ?")
            .bind(idx as i64)
            .bind(id.to_string())
            .execute(db)
            .await?;
    }
    Ok(())
}

/// 更新用例运行状态（直接运行 / 全部运行后回写）。
pub async fn update_test_case_status(
    db: &SqlitePool,
    case_id: Uuid,
    status: TestCaseStatus,
) -> Result<()> {
    sqlx::query("UPDATE test_cases SET last_run_status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(case_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// 落执行历史后同步回写列表摘要：状态 + 最近执行时间 + 断言通过 / 失败数。
pub async fn update_test_case_run_summary(
    db: &SqlitePool,
    case_id: Uuid,
    status: TestCaseStatus,
    ran_at: chrono::DateTime<chrono::Utc>,
    ok_count: Option<u32>,
    fail_count: Option<u32>,
) -> Result<()> {
    sqlx::query(
        "UPDATE test_cases
         SET last_run_status = ?, last_run_at = ?, last_run_ok = ?, last_run_fail = ?
         WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(ran_at.to_rfc3339())
    .bind(ok_count.map(|v| v as i64))
    .bind(fail_count.map(|v| v as i64))
    .bind(case_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

/// 更新用例完整内容（抽屉「保存修改」：请求快照 + 断言 / 提取 / 执行控制）。
#[allow(clippy::too_many_arguments)]
pub async fn update_test_case_content(
    db: &SqlitePool,
    case_id: Uuid,
    method: HttpMethod,
    url_path: &str,
    path_variables: &[KeyValue],
    params: &[KeyValue],
    headers: &[KeyValue],
    body_type: &str,
    body_content: &str,
    assertions: &serde_json::Value,
    extractors: &serde_json::Value,
    auth: &Option<miyo_core::model::AuthSpec>,
    timeout_ms: Option<u64>,
    disable_cookies: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE test_cases
         SET method = ?, url_path = ?, path_variables = ?, params = ?, headers = ?,
             body_type = ?, body_content = ?, assertions = ?, extractors = ?,
             auth = ?, timeout_ms = ?, disable_cookies = ?
         WHERE id = ?",
    )
    .bind(method.as_str())
    .bind(url_path)
    .bind(serde_json::to_string(path_variables).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(params).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(headers).map_err(miyo_core::AppError::Json)?)
    .bind(body_type)
    .bind(body_content)
    .bind(assertions.to_string())
    .bind(extractors.to_string())
    .bind(opt_json(auth)?)
    .bind(timeout_ms.map(|v| v as i64))
    .bind(disable_cookies)
    .bind(case_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TestCaseRow {
    id: String,
    request_id: String,
    name: String,
    category: String,
    folder: Option<String>,
    method: String,
    url_path: String,
    path_variables: Option<String>,
    params: Option<String>,
    headers: Option<String>,
    body_type: String,
    body_content: String,
    assertions: Option<String>,
    extractors: Option<String>,
    auth: Option<String>,
    timeout_ms: Option<i64>,
    disable_cookies: Option<i64>,
    sort_order: Option<i64>,
    last_run_status: String,
    last_run_at: Option<String>,
    last_run_ok: Option<i64>,
    last_run_fail: Option<i64>,
    created_at: String,
}

impl TestCaseRow {
    fn into_model(self) -> Result<TestCase> {
        let status =
            TestCaseStatus::parse(&self.last_run_status).unwrap_or(TestCaseStatus::Untested);
        Ok(TestCase {
            id: super::rows::parse_uuid(&self.id)?,
            request_id: super::rows::parse_uuid(&self.request_id)?,
            name: self.name,
            category: self.category,
            folder: self.folder.unwrap_or_default(),
            method: self.method.parse()?,
            url_path: self.url_path,
            path_variables: serde_json::from_str(self.path_variables.as_deref().unwrap_or("[]"))
                .map_err(miyo_core::AppError::Json)?,
            params: serde_json::from_str(self.params.as_deref().unwrap_or("[]"))
                .map_err(miyo_core::AppError::Json)?,
            headers: serde_json::from_str(self.headers.as_deref().unwrap_or("[]"))
                .map_err(miyo_core::AppError::Json)?,
            body_type: self.body_type,
            body_content: self.body_content,
            assertions: serde_json::from_str(self.assertions.as_deref().unwrap_or("[]"))
                .map_err(miyo_core::AppError::Json)?,
            extractors: serde_json::from_str(self.extractors.as_deref().unwrap_or("[]"))
                .map_err(miyo_core::AppError::Json)?,
            auth: match self.auth.as_deref() {
                Some(s) => Some(serde_json::from_str(s).map_err(miyo_core::AppError::Json)?),
                None => None,
            },
            timeout_ms: self.timeout_ms.map(|v| v.max(0) as u64),
            disable_cookies: self.disable_cookies.unwrap_or(0) != 0,
            sort_order: self.sort_order.unwrap_or(0),
            last_run_status: status,
            last_run_at: self
                .last_run_at
                .as_deref()
                .map(super::rows::parse_time)
                .transpose()?,
            last_run_ok: self.last_run_ok.map(|v| v.clamp(0, u32::MAX as i64) as u32),
            last_run_fail: self
                .last_run_fail
                .map(|v| v.clamp(0, u32::MAX as i64) as u32),
            created_at: super::rows::parse_time(&self.created_at)?,
        })
    }
}
