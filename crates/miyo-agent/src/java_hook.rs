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

/// 构造场景引擎的 Java 步骤执行器（两侧共用单源；按项目连接模式路由）。
///
/// `attach`：AttachManager（miyo-tauri AppState 持有 / Agent 控制面经
/// AgentState 注入）。项目 mode = attach 时必须提供——远程 / MCP 链路对
/// attach 项目经同一会话表路由（零侵入目标 + 历史同表落库）。
pub fn java_invoke_hook(
    db: sqlx::SqlitePool,
    attach: Option<Arc<miyo_java_attach::AttachManager>>,
) -> Arc<miyo_test::runner::JavaInvokeFn> {
    Arc::new(move |req| {
        let db = db.clone();
        let attach = attach.clone();
        Box::pin(async move {
            // 插件门禁：Java 调试插件未安装 / 未启用时拒绝执行
            // （tauri 命令层另有前置检查，这里是场景引擎 / Agent 控制面 / MCP 的兜底单源）。
            repo::ensure_plugin_enabled(&db, miyo_plugin::JAVA_DEBUG_PLUGIN_ID)
                .await
                .map_err(|e| format!("Java 调试插件不可用：{}", e.user_message()))?;
            let project = repo::get_java_project(&db, req.java_project_id)
                .await
                .map_err(|e| format!("Java 项目配置加载失败：{}", e.user_message()))?;
            // 按连接模式路由：attach（零侵入动态注入）/ runtime（SDK 端点，v1 兼容）。
            if project.mode == "attach" {
                let Some(manager) = attach else {
                    return Err("attach 模式项目需要本机 Attach 会话：请在 Java 调试工作台\
                        先 Attach 目标进程（远程 Java 应用不支持本地 Attach）"
                        .to_string());
                };
                return attach_invoke(&manager, &db, &project, req).await;
            }
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

/// attach 模式执行：经 AttachManager 会话（Agent IPC）调用，历史同表落库。
///
/// 目标方法经 `Class#method(T1,T2)` + param_types 定位（method_id 与场景步骤
/// 同键）；实例解析默认 bean（Spring Bean）/ Agent 侧 unsupported 显式失败。
/// attach 会话未建立时报错——场景引擎不隐式注入 JVM（入口在 UI 且需确认）。
async fn attach_invoke(
    manager: &Arc<miyo_java_attach::AttachManager>,
    db: &sqlx::SqlitePool,
    project: &miyo_core::model::JavaDebugProject,
    req: miyo_test::runner::JavaStepRequest,
) -> Result<miyo_test::runner::JavaStepResponse, String> {
    let pid = project.attach_pid.ok_or_else(|| {
        "attach 项目缺少目标 pid（项目行异常）：请重新 Attach 目标进程".to_string()
    })?;
    let client = manager
        .client(pid)
        .await
        .map_err(|_| "目标进程尚未 Attach：请在 Java 调试工作台连接后重试".to_string())?;
    let (class, method, param_types) = split_method_id(&req.method_id);
    // 黑名单预检（与 tauri 命令层 java_agent_invoke 同口径；Agent 侧 RiskPolicy
    // 终检兜底——纵深防御，不能只依赖目标 JVM 内的最后一道闸）
    if miyo_java::policy::blacklisted(class.trim()) {
        return Err(format!("{class} 属于受限类，禁止通过调试协议调用"));
    }
    let arguments = normalize_arguments(req.arguments.clone());
    let invocation_id = Uuid::new_v4();
    let params = serde_json::json!({
        "invocation_id": invocation_id.to_string(),
        "class": class,
        "method": method,
        "param_types": param_types,
        "args": arguments,
        "instance": { "kind": "bean" },
        "timeout_ms": req.timeout_ms.unwrap_or(30_000),
    });
    let timeout_ms = req.timeout_ms.unwrap_or(30_000).min(MAX_TIMEOUT_MS);
    let resp = client
        .invoke(params, timeout_ms)
        .await
        .map_err(|e| e.to_app_error().user_message())?;
    let status = resp
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("failed")
        .to_string();
    let record = JavaInvocationRecord {
        id: invocation_id,
        project_id: project.id,
        class_name: class.to_string(),
        method_id: req.method_id.clone(),
        method_signature: req.method_id.clone(),
        arguments: req.arguments.clone(),
        status,
        result: resp.get("result").cloned(),
        result_truncated: resp
            .get("result_truncated")
            .and_then(|t| t.as_bool())
            .unwrap_or(false),
        exception: resp.get("exception").cloned(),
        duration_ms: resp.get("duration_ms").and_then(|d| d.as_f64()),
        started_at: resp
            .get("started_at")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        finished_at: resp
            .get("finished_at")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        created_at: chrono::Utc::now(),
    };
    if let Err(e) = repo::insert_java_invocation(db, &record).await {
        tracing::warn!("attach 调用历史落库失败：{e}");
    }
    Ok(miyo_test::runner::JavaStepResponse {
        status: record.status.clone(),
        result: record.result.clone(),
        exception: record.exception.clone(),
        duration_ms: record.duration_ms,
    })
}

/// `Class#method(T1,T2)` → (class, method, [T1, T2])（缺参表 = 空表，要求唯一）。
fn split_method_id(method_id: &str) -> (&str, &str, Vec<String>) {
    let (class, rest) = method_id.split_once('#').unwrap_or((method_id, ""));
    let (method, params) = match rest.split_once('(') {
        Some((m, p)) => (m, p.trim_end_matches(')')),
        None => (rest, ""),
    };
    let types = params
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    (class, method, types)
}
