//! TestCase（测试用例）仓储：新建 / 列表 / 删除 / 更新名称与分组 / 更新运行状态。

use sqlx::{QueryBuilder, SqlitePool};
use uuid::Uuid;

use miyo_core::model::{HttpMethod, KeyValue, TestCase, TestCaseStatus};
use miyo_core::Result;

/// 新建测试用例。
pub async fn create_test_case(db: &SqlitePool, case: &TestCase) -> Result<TestCase> {
    sqlx::query(
        "INSERT INTO test_cases
             (id, request_id, name, category, folder, method, url_path, path_variables, params, headers,
              body_type, body_content, last_run_status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(case.last_run_status.as_str())
    .bind(case.created_at.to_rfc3339())
    .execute(db)
    .await?;
    Ok(case.clone())
}

/// 列出接口的全部测试用例（按创建时间排序）。
pub async fn list_test_cases(db: &SqlitePool, request_id: Uuid) -> Result<Vec<TestCase>> {
    let rows: Vec<TestCaseRow> = sqlx::query_as(
        "SELECT id, request_id, name, category, folder, method, url_path, path_variables, params, headers,
                body_type, body_content, last_run_status, created_at
         FROM test_cases WHERE request_id = ? ORDER BY created_at",
    )
    .bind(request_id.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter().map(TestCaseRow::into_model).collect()
}

/// 批量列出多个接口的测试用例（一次查询，按接口分组交给调用方；
/// 冒烟文档导出等去 N+1 用）。
///
/// SQLite 变量数上限按 500 分片，避免超长 IN 列表；排序按
/// `request_id, created_at`，保证每个接口内部与 [`list_test_cases`] 一致。
pub async fn list_test_cases_by_endpoints(
    db: &SqlitePool,
    request_ids: &[Uuid],
) -> Result<Vec<TestCase>> {
    let mut out = Vec::new();
    for chunk in request_ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let mut qb = QueryBuilder::new(
            "SELECT id, request_id, name, category, folder, method, url_path, path_variables, params, headers,
                    body_type, body_content, last_run_status, created_at
             FROM test_cases WHERE request_id IN (",
        );
        let mut separated = qb.separated(", ");
        for id in chunk {
            separated.push_bind(id.to_string());
        }
        separated.push_unseparated(") ORDER BY request_id, created_at");
        let rows: Vec<TestCaseRow> = qb.build_query_as().fetch_all(db).await?;
        for row in rows {
            out.push(row.into_model()?);
        }
    }
    Ok(out)
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

/// 批量设置用例文件夹（行菜单「移动到分组」；空串 = 移出分组）。
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

/// 更新用例完整请求内容（方法 / 路径 / 路径变量 / Params / Headers / Body，抽屉「保存修改」）。
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
) -> Result<()> {
    sqlx::query(
        "UPDATE test_cases
         SET method = ?, url_path = ?, path_variables = ?, params = ?, headers = ?,
             body_type = ?, body_content = ?
         WHERE id = ?",
    )
    .bind(method.as_str())
    .bind(url_path)
    .bind(serde_json::to_string(path_variables).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(params).map_err(miyo_core::AppError::Json)?)
    .bind(serde_json::to_string(headers).map_err(miyo_core::AppError::Json)?)
    .bind(body_type)
    .bind(body_content)
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
    last_run_status: String,
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
            last_run_status: status,
            created_at: super::rows::parse_time(&self.created_at)?,
        })
    }
}
