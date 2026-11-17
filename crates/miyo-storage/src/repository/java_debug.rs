//! Java 调试项目与调用历史的 CRUD（0030_java_debug）。
//!
//! - token 明文只存在于模型内存；落库走 `encrypt_blob`（AES-256-GCM）进
//!   `auth_json`，读取时解密回 `auth_token`——日志与错误信息不得带出。
//! - `last_info_json` 是握手缓存（非权威），仅用于列表页快速展示。
//! - 调用历史按项目滚动保留（`prune_java_invocations`），插入方负责触发。

use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{JavaDebugProject, JavaInvocationRecord};
use miyo_core::{AppError, Result};

use super::rows::{encrypt_blob, parse_time, parse_uuid};

#[derive(sqlx::FromRow)]
struct JavaProjectRow {
    id: String,
    project_id: Option<String>,
    name: String,
    runtime_endpoint: String,
    environment_id: Option<String>,
    auth_json: String,
    timeout_ms: i64,
    max_result_bytes: i64,
    max_concurrent: i64,
    description: String,
    last_connected_at: Option<String>,
    last_info_json: Option<String>,
    created_at: String,
    updated_at: String,
}

impl JavaProjectRow {
    fn into_model(self) -> Result<JavaDebugProject> {
        // auth_json = {"token": "<密文>"}；解析失败按未配置兜底（不毒化列表）。
        let auth_token = parse_auth_token(&self.auth_json).unwrap_or(None);
        Ok(JavaDebugProject {
            id: parse_uuid(&self.id)?,
            project_id: self.project_id.map(|v| parse_uuid(&v)).transpose()?,
            name: self.name,
            runtime_endpoint: self.runtime_endpoint,
            environment_id: self.environment_id.map(|v| parse_uuid(&v)).transpose()?,
            auth_token,
            timeout_ms: self.timeout_ms.clamp(1, 300_000) as u64,
            max_result_bytes: self.max_result_bytes.max(1024) as usize,
            max_concurrent: self.max_concurrent.clamp(1, 64) as u32,
            description: self.description,
            last_connected_at: self
                .last_connected_at
                .as_deref()
                .map(parse_time)
                .transpose()?,
            last_info: self
                .last_info_json
                .and_then(|v| serde_json::from_str(&v).ok()),
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

fn parse_auth_token(auth_json: &str) -> Result<Option<String>> {
    if auth_json.trim().is_empty() || auth_json.trim() == "{}" {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(auth_json)
        .map_err(|e| AppError::Validation(format!("auth_json 非法：{e}")))?;
    let encrypted = value
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let Some(encrypted) = encrypted else {
        return Ok(None);
    };
    let plain = miyo_secret::ensure_master_key()
        .and_then(|k| miyo_secret::decrypt(&k, encrypted))
        .map_err(|e| AppError::Decryption(format!("Runtime token 解密失败：{e}")))?;
    Ok(Some(plain))
}

fn auth_json_for(token: Option<&str>) -> Result<String> {
    let encrypted = match token.filter(|t| !t.trim().is_empty()) {
        None => serde_json::Value::Null,
        Some(t) => serde_json::Value::String(encrypt_blob(t)?),
    };
    Ok(serde_json::json!({ "token": encrypted }).to_string())
}

const PROJECT_COLUMNS: &str = "id, project_id, name, runtime_endpoint, environment_id, auth_json, timeout_ms, max_result_bytes, max_concurrent, description, last_connected_at, last_info_json, created_at, updated_at";

/// 列出全部 Java 调试项目（更新时间倒序）；`project_id` 过滤关联项目。
pub async fn list_java_projects(
    db: &SqlitePool,
    project_id: Option<Uuid>,
) -> Result<Vec<JavaDebugProject>> {
    let rows: Vec<JavaProjectRow> = match project_id {
        Some(pid) => {
            sqlx::query_as(&format!(
                "SELECT {PROJECT_COLUMNS} FROM java_projects WHERE project_id = ? ORDER BY updated_at DESC"
            ))
            .bind(pid.to_string())
            .fetch_all(db)
            .await?
        }
        None => {
            sqlx::query_as(&format!(
                "SELECT {PROJECT_COLUMNS} FROM java_projects ORDER BY updated_at DESC"
            ))
            .fetch_all(db)
            .await?
        }
    };
    rows.into_iter().map(JavaProjectRow::into_model).collect()
}

pub async fn get_java_project(db: &SqlitePool, id: Uuid) -> Result<JavaDebugProject> {
    let row: Option<JavaProjectRow> = sqlx::query_as(&format!(
        "SELECT {PROJECT_COLUMNS} FROM java_projects WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(JavaProjectRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("Java 调试项目（{id}）")))
}

/// 保存（upsert 整体覆盖写）。
pub async fn save_java_project(db: &SqlitePool, project: &JavaDebugProject) -> Result<()> {
    let auth_json = auth_json_for(project.auth_token.as_deref())?;
    let last_info_json = project
        .last_info
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let result = sqlx::query(
        "INSERT INTO java_projects (id, project_id, name, runtime_endpoint, environment_id, auth_json, timeout_ms, max_result_bytes, max_concurrent, description, last_connected_at, last_info_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            project_id = excluded.project_id,
            name = excluded.name,
            runtime_endpoint = excluded.runtime_endpoint,
            environment_id = excluded.environment_id,
            auth_json = excluded.auth_json,
            timeout_ms = excluded.timeout_ms,
            max_result_bytes = excluded.max_result_bytes,
            max_concurrent = excluded.max_concurrent,
            description = excluded.description,
            last_info_json = excluded.last_info_json,
            updated_at = excluded.updated_at",
    )
    .bind(project.id.to_string())
    .bind(project.project_id.map(|v| v.to_string()))
    .bind(project.name.trim())
    .bind(project.runtime_endpoint.trim())
    .bind(project.environment_id.map(|v| v.to_string()))
    .bind(auth_json)
    .bind(project.timeout_ms as i64)
    .bind(project.max_result_bytes as i64)
    .bind(project.max_concurrent as i64)
    .bind(project.description.trim())
    .bind(project.last_connected_at.map(|t| t.to_rfc3339()))
    .bind(last_info_json)
    .bind(project.created_at.to_rfc3339())
    .bind(project.updated_at.to_rfc3339())
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Java 调试项目（{}）",
            project.id
        )));
    }
    Ok(())
}

/// 刷新连接状态（探测成功：最近连接时间 + 握手缓存）。
pub async fn touch_java_project_connected(
    db: &SqlitePool,
    id: Uuid,
    info_json: &str,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE java_projects SET last_connected_at = ?, last_info_json = ?, updated_at = ? WHERE id = ?",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(info_json)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(id.to_string())
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Java 调试项目（{id}）")));
    }
    Ok(())
}

pub async fn delete_java_project(db: &SqlitePool, id: Uuid) -> Result<()> {
    let result = sqlx::query("DELETE FROM java_projects WHERE id = ?")
        .bind(id.to_string())
        .execute(db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Java 调试项目（{id}）")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 调用历史
// ---------------------------------------------------------------------------

/// 每项目滚动保留的历史条数（插入方触发清理）。
pub const JAVA_INVOCATION_KEEP: u32 = 200;

#[derive(sqlx::FromRow)]
struct InvocationRow {
    id: String,
    project_id: String,
    class_name: String,
    method_id: String,
    method_signature: String,
    args_json: String,
    status: String,
    result_json: Option<String>,
    result_truncated: i64,
    exception_json: Option<String>,
    duration_ms: Option<f64>,
    started_at: Option<String>,
    finished_at: Option<String>,
    created_at: String,
}

impl InvocationRow {
    fn into_model(self) -> Result<JavaInvocationRecord> {
        Ok(JavaInvocationRecord {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            class_name: self.class_name,
            method_id: self.method_id,
            method_signature: self.method_signature,
            arguments: serde_json::from_str(&self.args_json).unwrap_or_default(),
            status: self.status,
            result: self.result_json.and_then(|v| serde_json::from_str(&v).ok()),
            result_truncated: self.result_truncated != 0,
            exception: self
                .exception_json
                .and_then(|v| serde_json::from_str(&v).ok()),
            duration_ms: self.duration_ms,
            started_at: self.started_at,
            finished_at: self.finished_at,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

/// 落一条调用历史并滚动清理（保留最近 `JAVA_INVOCATION_KEEP` 条）。
pub async fn insert_java_invocation(db: &SqlitePool, record: &JavaInvocationRecord) -> Result<()> {
    let args_json = serde_json::to_string(&record.arguments)
        .map_err(|e| AppError::Validation(format!("调用参数序列化失败：{e}")))?;
    let result_json = record
        .result
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let exception_json = record
        .exception
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    sqlx::query(
        "INSERT INTO java_invocations (id, project_id, class_name, method_id, method_signature, args_json, status, result_json, result_truncated, exception_json, duration_ms, started_at, finished_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(record.id.to_string())
    .bind(record.project_id.to_string())
    .bind(record.class_name.trim())
    .bind(record.method_id.trim())
    .bind(record.method_signature.trim())
    .bind(args_json)
    .bind(record.status.trim())
    .bind(if result_json.is_empty() { None } else { Some(result_json) })
    .bind(record.result_truncated as i64)
    .bind(if exception_json.is_empty() { None } else { Some(exception_json) })
    .bind(record.duration_ms)
    .bind(record.started_at.as_deref())
    .bind(record.finished_at.as_deref())
    .bind(record.created_at.to_rfc3339())
    .execute(db)
    .await?;
    prune_java_invocations(db, record.project_id, JAVA_INVOCATION_KEEP).await;
    Ok(())
}

/// 滚动清理：保留最近 `keep` 条（按 created_at DESC）。清理失败只记日志
///（历史非关键路径，不阻塞调用返回）。
pub async fn prune_java_invocations(db: &SqlitePool, project_id: Uuid, keep: u32) {
    let result = sqlx::query(
        "DELETE FROM java_invocations WHERE project_id = ? AND id NOT IN (
            SELECT id FROM java_invocations WHERE project_id = ? ORDER BY created_at DESC LIMIT ?
        )",
    )
    .bind(project_id.to_string())
    .bind(project_id.to_string())
    .bind(keep as i64)
    .execute(db)
    .await;
    if let Err(e) = result {
        tracing::warn!("Java 调用历史清理失败：{e}");
    }
}

/// 列出项目调用历史（时间倒序，`limit` 上限 500）。
pub async fn list_java_invocations(
    db: &SqlitePool,
    project_id: Uuid,
    limit: u32,
) -> Result<Vec<JavaInvocationRecord>> {
    let limit = limit.clamp(1, 500) as i64;
    let rows: Vec<InvocationRow> = sqlx::query_as(
        "SELECT id, project_id, class_name, method_id, method_signature, args_json, status, result_json, result_truncated, exception_json, duration_ms, started_at, finished_at, created_at
         FROM java_invocations WHERE project_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(InvocationRow::into_model).collect()
}

pub async fn get_java_invocation(db: &SqlitePool, id: Uuid) -> Result<JavaInvocationRecord> {
    let row: Option<InvocationRow> = sqlx::query_as(
        "SELECT id, project_id, class_name, method_id, method_signature, args_json, status, result_json, result_truncated, exception_json, duration_ms, started_at, finished_at, created_at
         FROM java_invocations WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(InvocationRow::into_model)
        .transpose()?
        .ok_or_else(|| AppError::NotFound(format!("调用历史（{id}）")))
}

pub async fn delete_java_invocation(db: &SqlitePool, id: Uuid) -> Result<()> {
    let result = sqlx::query("DELETE FROM java_invocations WHERE id = ?")
        .bind(id.to_string())
        .execute(db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("调用历史（{id}）")));
    }
    Ok(())
}

pub async fn clear_java_invocations(db: &SqlitePool, project_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM java_invocations WHERE project_id = ?")
        .bind(project_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> SqlitePool {
        crate::db::memory_pool().await.unwrap()
    }

    fn sample_project(name: &str) -> JavaDebugProject {
        JavaDebugProject {
            id: Uuid::new_v4(),
            project_id: None,
            name: name.into(),
            runtime_endpoint: "http://127.0.0.1:9918".into(),
            environment_id: None,
            auth_token: Some("s3cret".into()),
            timeout_ms: 30_000,
            max_result_bytes: 1_048_576,
            max_concurrent: 4,
            description: "订单服务调试".into(),
            last_connected_at: None,
            last_info: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn project_crud_with_encrypted_token() {
        let db = db().await;
        let mut p = sample_project("订单");
        save_java_project(&db, &p).await.unwrap();

        let got = get_java_project(&db, p.id).await.unwrap();
        assert_eq!(got.name, "订单");
        // token 解密回明文（仅内存），auth_json 落库为密文。
        assert_eq!(got.auth_token.as_deref(), Some("s3cret"));
        let raw: String = sqlx::query_scalar("SELECT auth_json FROM java_projects WHERE id = ?")
            .bind(p.id.to_string())
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(!raw.contains("s3cret"), "token 必须加密落库：{raw}");

        // 更新：改地址 + 清 token。
        p.runtime_endpoint = "http://127.0.0.1:9919".into();
        p.auth_token = None;
        save_java_project(&db, &p).await.unwrap();
        let got = get_java_project(&db, p.id).await.unwrap();
        assert_eq!(got.runtime_endpoint, "http://127.0.0.1:9919");
        assert_eq!(got.auth_token, None);

        delete_java_project(&db, p.id).await.unwrap();
        assert!(get_java_project(&db, p.id).await.is_err());
    }

    #[tokio::test]
    async fn connection_touch_updates_timestamp() {
        let db = db().await;
        let p = sample_project("健康检查");
        save_java_project(&db, &p).await.unwrap();
        assert!(get_java_project(&db, p.id)
            .await
            .unwrap()
            .last_connected_at
            .is_none());

        touch_java_project_connected(&db, p.id, r#"{"app_name":"demo"}"#)
            .await
            .unwrap();
        let got = get_java_project(&db, p.id).await.unwrap();
        assert!(got.last_connected_at.is_some());
        assert_eq!(got.last_info.unwrap()["app_name"], "demo");
    }

    #[tokio::test]
    async fn invocation_history_and_pruning() {
        let db = db().await;
        let p = sample_project("历史");
        save_java_project(&db, &p).await.unwrap();

        for i in 0..(JAVA_INVOCATION_KEEP + 10) {
            let record = JavaInvocationRecord {
                id: Uuid::new_v4(),
                project_id: p.id,
                class_name: "com.example.OrderService".into(),
                method_id: format!("com.example.OrderService#m{}(long)", i),
                method_signature: "m(long)".into(),
                arguments: serde_json::json!([i]),
                status: "success".into(),
                result: Some(serde_json::json!({"i": i})),
                result_truncated: false,
                exception: None,
                duration_ms: Some(3.5),
                started_at: None,
                finished_at: None,
                created_at: chrono::Utc::now() + chrono::Duration::milliseconds(i as i64),
            };
            insert_java_invocation(&db, &record).await.unwrap();
        }

        let list = list_java_invocations(&db, p.id, 500).await.unwrap();
        assert_eq!(list.len() as u32, JAVA_INVOCATION_KEEP);
        // 最新在前。
        assert!(list[0].method_id.contains("209"));
        assert_eq!(list[0].result.as_ref().unwrap()["i"], 209);

        let first_id = list[0].id;
        let got = get_java_invocation(&db, first_id).await.unwrap();
        assert_eq!(got.status, "success");

        delete_java_invocation(&db, first_id).await.unwrap();
        assert!(get_java_invocation(&db, first_id).await.is_err());

        clear_java_invocations(&db, p.id).await.unwrap();
        assert!(list_java_invocations(&db, p.id, 500)
            .await
            .unwrap()
            .is_empty());
    }
}
