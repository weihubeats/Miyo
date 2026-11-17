//! 场景引擎 Java 步骤执行器（tauri IPC 与 agent 控制面共用单源）。
//!
//! runner 不依赖存储层：执行器由调用方注入。两侧（`miyo-tauri`
//! `commands/java_debug.rs` 与 `server.rs` 的 `run_scenario`）都走本模块的
//! [`java_invoke_hook`]，持有 DB 连接池，按 java_project_id 加载配置 →
//! miyo-java 客户端调用 → 历史落库。
//!
//! 链路级失败（项目配置缺失 / Runtime 不可达）返回 `Err` 文本；方法业务
//! 失败（抛异常 / 超时）以 `Failed` 状态回传——场景侧映射 failed 步骤，
//! 不断链、可断言。

use std::sync::Arc;

use uuid::Uuid;

use miyo_core::model::JavaInvocationRecord;
use miyo_java::protocol::InvocationStatus;
use miyo_java::{JavaRuntimeClient, SecurityPolicy};
use miyo_storage::repository as repo;

/// 参数体积上限（固定 1 MiB；SDK 侧终检同值）。
const MAX_ARGS_BYTES: usize = 1024 * 1024;
/// 单方法参数个数上限。
const MAX_PARAMS: usize = 32;
/// 调用超时上限（与全局请求超时口径一致）。
const MAX_TIMEOUT_MS: u64 = 300_000;

/// 项目配置 → 调用安全策略。
pub fn security_policy_for(project: &miyo_core::model::JavaDebugProject) -> SecurityPolicy {
    SecurityPolicy {
        max_args_bytes: MAX_ARGS_BYTES,
        max_result_bytes: project.max_result_bytes,
        max_timeout_ms: MAX_TIMEOUT_MS,
        max_params: MAX_PARAMS,
        max_concurrent_invocations: project.max_concurrent,
    }
}

/// 归一参数：协议要求 JSON 数组；对象 / 标量包装为单元素，null = 空参。
pub fn normalize_arguments(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// `InvocationStatus` → 历史落库 / 场景步骤状态字符串（snake_case wire 值）。
pub fn status_str(status: InvocationStatus) -> &'static str {
    match status {
        InvocationStatus::Success => "success",
        InvocationStatus::Failed => "failed",
        InvocationStatus::Timeout => "timeout",
        InvocationStatus::Cancelled => "cancelled",
        InvocationStatus::RuntimeDisconnected => "runtime_disconnected",
    }
}

/// 项目配置 → 就绪客户端（地址归一 / token 注入 / 策略装载）。
pub fn client_for(
    project: &miyo_core::model::JavaDebugProject,
) -> Result<JavaRuntimeClient, miyo_core::AppError> {
    JavaRuntimeClient::new(
        &project.runtime_endpoint,
        project.auth_token.as_deref(),
        security_policy_for(project),
    )
}

/// 构造场景引擎的 Java 步骤执行器（两侧共用单源）。
pub fn java_invoke_hook(db: sqlx::SqlitePool) -> Arc<miyo_test::runner::JavaInvokeFn> {
    Arc::new(move |req| {
        let db = db.clone();
        Box::pin(async move {
            let project = repo::get_java_project(&db, req.java_project_id)
                .await
                .map_err(|e| format!("Java 项目配置加载失败：{}", e.user_message()))?;
            let client = client_for(&project).map_err(|e| e.to_string())?;
            let arguments = normalize_arguments(req.arguments.clone());
            let invocation_id = Uuid::new_v4();
            let resp = client
                .invoke(&req.method_id, arguments, req.timeout_ms, invocation_id)
                .await
                .map_err(|e| e.user_message())?;
            let record = JavaInvocationRecord {
                id: invocation_id,
                project_id: project.id,
                class_name: req.class_name.clone(),
                method_id: req.method_id.clone(),
                method_signature: req.method_id.clone(),
                arguments: req.arguments.clone(),
                status: status_str(resp.status).to_string(),
                result: resp.result.clone(),
                result_truncated: resp.result_truncated,
                exception: resp.exception.as_ref().map(|e| serde_json::json!(e)),
                duration_ms: resp.duration_ms,
                started_at: resp.started_at.clone(),
                finished_at: resp.finished_at.clone(),
                created_at: chrono::Utc::now(),
            };
            if let Err(e) = repo::insert_java_invocation(&db, &record).await {
                tracing::warn!("Java 调用历史落库失败：{e}");
            }
            Ok(miyo_test::runner::JavaStepResponse {
                status: status_str(resp.status).to_string(),
                result: resp.result,
                exception: resp.exception.map(|e| serde_json::json!(e)),
                duration_ms: resp.duration_ms,
            })
        })
    })
}
