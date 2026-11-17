//! Java 方法实时调试 Command：项目 CRUD、Runtime 连接探测、类/方法发现、
//! 方法调用（含历史落库）与场景引擎 Java 执行器注入。
//!
//! 安全要点：
//! - token 仅内存流转（落库走存储层 AES-GCM）；日志 / 错误信息不得带出；
//! - 调用前经过 [`SecurityPolicy`] 预检（体积 / 超时 / 黑名单），SDK 侧终检；
//! - 每次调用落 `java_invocations` 历史（审计 + 重放），滚动保留 200 条。

use std::sync::Arc;

use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

use miyo_core::model::{JavaDebugProject, JavaInvocationRecord};
use miyo_java::protocol::{InvocationStatus, InvokeResponse};
use miyo_java::{JavaRuntimeClient, SecurityPolicy};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 参数体积上限（固定 1 MiB；SDK 侧终检同值）。
const MAX_ARGS_BYTES: usize = 1024 * 1024;
/// 单方法参数个数上限。
const MAX_PARAMS: usize = 32;
/// 调用超时上限（与全局请求超时口径一致）。
const MAX_TIMEOUT_MS: u64 = 300_000;

/// 项目配置 → 调用安全策略。
fn security_policy(project: &JavaDebugProject) -> SecurityPolicy {
    SecurityPolicy {
        max_args_bytes: MAX_ARGS_BYTES,
        max_result_bytes: project.max_result_bytes,
        max_timeout_ms: MAX_TIMEOUT_MS,
        max_params: MAX_PARAMS,
        max_concurrent_invocations: project.max_concurrent,
    }
}

fn client_for(project: &JavaDebugProject) -> CommandResult<JavaRuntimeClient> {
    JavaRuntimeClient::new(
        &project.runtime_endpoint,
        project.auth_token.as_deref(),
        security_policy(project),
    )
    .map_err(CommandError::from)
}

// ---------------------------------------------------------------------------
// 项目 CRUD
// ---------------------------------------------------------------------------

/// 列出 Java 调试项目（可按关联的 Miyo 项目过滤）。
///
/// token 不回显（脱敏）：列表只用于展示与选择，前端编辑时 token 留空
/// 表示「保持现有值」（见 [`save_java_project`]）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_java_projects(
    state: State<'_, AppState>,
    project_id: Option<Uuid>,
) -> CommandResult<Vec<JavaDebugProject>> {
    let mut projects = repo::list_java_projects(&state.db, project_id).await?;
    for p in &mut projects {
        p.auth_token = None;
    }
    Ok(projects)
}

/// 保存 Java 调试项目（upsert；token 随模型 AES-GCM 加密落库）。
///
/// token 三态语义：
/// - `None`（字段缺省，前端编辑未动）→ 保留库中原 token；
/// - `Some("")`（前端显式清除）→ 清空；
/// - `Some(新值)` → 更新。
///
/// 返回值 token 一律脱敏（None），明文只在 Miyo 进程内流转。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_java_project(
    state: State<'_, AppState>,
    project: JavaDebugProject,
) -> CommandResult<JavaDebugProject> {
    let mut project = project;
    let name = project.name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::validation("项目名称不能为空"));
    }
    project.name = name;
    match project.auth_token.take() {
        // 字段缺省：已有项目保留原 token（新建时查不到 → 维持 None）。
        None => {
            if let Ok(existing) = repo::get_java_project(&state.db, project.id).await {
                project.auth_token = existing.auth_token;
            }
        }
        // 空串 = 显式清除。
        Some(t) if t.trim().is_empty() => project.auth_token = None,
        // 非空 = 更新。
        Some(t) => project.auth_token = Some(t),
    }
    // 地址形状校验（scheme / 尾斜杠）提前到保存期，失败即弹。
    JavaRuntimeClient::new(
        &project.runtime_endpoint,
        project.auth_token.as_deref(),
        SecurityPolicy::default(),
    )
    .map_err(CommandError::from)?;
    if let Some(pid) = project.project_id {
        repo::get_project(&state.db, pid).await?;
    }
    project.updated_at = chrono::Utc::now();
    repo::save_java_project(&state.db, &project).await?;
    project.auth_token = None;
    Ok(project)
}

/// 删除 Java 调试项目（级联清历史）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_java_project(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    Ok(repo::delete_java_project(&state.db, id).await?)
}

// ---------------------------------------------------------------------------
// 本地 Runtime 发现
// ---------------------------------------------------------------------------

/// 发现本机正在运行的 Java Debug Runtime（注册目录扫描 + 默认端口段指纹探测）。
///
/// 探测为真实网络行为：对 SDK 端点发无鉴权 `GET /miyo/debug/info`，
/// 以「401 + not_authorized」指纹确认目标（详见 miyo_java::discover）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_discover_local() -> CommandResult<Vec<miyo_java::JavaDiscoveredRuntime>> {
    Ok(miyo_java::discover_local().await)
}

/// 端点连通性直连探测（保存前「测试连接」用：不落库，地址 + token 即可握手）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_probe_endpoint(
    endpoint: String,
    token: Option<String>,
) -> CommandResult<miyo_java::RuntimeInfo> {
    let client = JavaRuntimeClient::new(&endpoint, token.as_deref(), SecurityPolicy::default())
        .map_err(CommandError::from)?;
    Ok(client.info().await?)
}

// ---------------------------------------------------------------------------
// 连接与发现
// ---------------------------------------------------------------------------

/// Runtime 连接探测：握手校验协议版本，成功则刷新最近连接时间与信息缓存。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_test_connection(
    state: State<'_, AppState>,
    id: Uuid,
) -> CommandResult<miyo_java::RuntimeInfo> {
    let project = repo::get_java_project(&state.db, id).await?;
    let client = client_for(&project)?;
    let info = client.info().await?;
    repo::touch_java_project_connected(&state.db, id, &serde_json::json!(info).to_string()).await?;
    Ok(info)
}

/// 白名单类清单（`search` 按类名 / 包名 / Bean 名不区分大小写过滤）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_list_classes(
    state: State<'_, AppState>,
    id: Uuid,
    search: Option<String>,
) -> CommandResult<Vec<miyo_java::JavaClassInfo>> {
    let project = repo::get_java_project(&state.db, id).await?;
    let client = client_for(&project)?;
    let mut classes = client.classes().await?;
    if let Some(term) = search
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        classes.retain(|c| {
            c.class_name.to_lowercase().contains(&term)
                || c.simple_name.to_lowercase().contains(&term)
                || c.package_name.to_lowercase().contains(&term)
                || c.bean_name
                    .as_deref()
                    .map(|b| b.to_lowercase().contains(&term))
                    .unwrap_or(false)
        });
    }
    Ok(classes)
}

/// 某类的白名单方法清单（重载方法以完整 method_id 区分）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_list_methods(
    state: State<'_, AppState>,
    id: Uuid,
    class_name: String,
) -> CommandResult<Vec<miyo_java::JavaMethodInfo>> {
    let project = repo::get_java_project(&state.db, id).await?;
    let client = client_for(&project)?;
    Ok(client.methods(&class_name).await?)
}

// ---------------------------------------------------------------------------
// 方法调用与历史
// ---------------------------------------------------------------------------

/// 方法调用入参。
#[derive(Debug, Deserialize)]
pub struct JavaInvokeArgs {
    /// 目标方法稳定标识。
    pub method_id: String,
    /// 位置参数（JSON 数组）。
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// 本次调用超时毫秒（None 用项目默认）。
    pub timeout_ms: Option<u64>,
    /// 人类可读签名（历史展示快照）。
    #[serde(default)]
    pub method_signature: Option<String>,
}

/// 归一参数：协议要求 JSON 数组；对象包装为单元素，标量包装为单元素，null = 空参。
fn normalize_arguments(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn status_str(status: InvocationStatus) -> &'static str {
    match status {
        InvocationStatus::Success => "success",
        InvocationStatus::Failed => "failed",
        InvocationStatus::Timeout => "timeout",
        InvocationStatus::Cancelled => "cancelled",
        InvocationStatus::RuntimeDisconnected => "runtime_disconnected",
    }
}

/// 执行 Java 方法：预检（黑名单 / 体积 / 超时）→ 调用 → 历史落库 → 返回结果。
///
/// 业务失败（方法抛异常）不算链路错误：结果原样回传（status = failed +
/// exception 详情）；Runtime 不可达 / 鉴权失败等链路错误返回 Err。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_invoke(
    state: State<'_, AppState>,
    id: Uuid,
    args: JavaInvokeArgs,
) -> CommandResult<InvokeResponse> {
    let project = repo::get_java_project(&state.db, id).await?;
    let client = client_for(&project)?;
    let raw_arguments = args.arguments.clone();
    let arguments = normalize_arguments(raw_arguments);
    let invocation_id = Uuid::new_v4();
    let signature = args.method_signature.clone();
    let resp = client
        .invoke(&args.method_id, arguments, args.timeout_ms, invocation_id)
        .await?;
    record_invocation(
        &state,
        &project,
        &args,
        &resp,
        invocation_id,
        signature.as_deref(),
    )
    .await;
    Ok(resp)
}

/// 落调用历史（审计 + 重放；失败只记日志，不阻塞调用返回）。
async fn record_invocation(
    state: &State<'_, AppState>,
    project: &JavaDebugProject,
    args: &JavaInvokeArgs,
    resp: &InvokeResponse,
    invocation_id: Uuid,
    signature_override: Option<&str>,
) {
    // 历史里的入参用「归一化前」的原值（模板重放时保持用户原始形状）。
    let record = JavaInvocationRecord {
        id: invocation_id,
        project_id: project.id,
        class_name: args
            .method_id
            .split_once('#')
            .map(|(c, _)| c.to_string())
            .unwrap_or_default(),
        method_id: args.method_id.clone(),
        method_signature: signature_override.unwrap_or_default().to_string(),
        arguments: args.arguments.clone(),
        status: status_str(resp.status).to_string(),
        result: resp.result.clone(),
        result_truncated: resp.result_truncated,
        exception: resp.exception.as_ref().map(|e| serde_json::json!(e)),
        duration_ms: resp.duration_ms,
        started_at: resp.started_at.clone(),
        finished_at: resp.finished_at.clone(),
        created_at: chrono::Utc::now(),
    };
    if let Err(e) = repo::insert_java_invocation(&state.db, &record).await {
        tracing::warn!("Java 调用历史落库失败：{e}");
    }
}

/// 取消在途调用（协作式中断：方法不响应中断时仍会执行完）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_cancel_invocation(
    state: State<'_, AppState>,
    id: Uuid,
    invocation_id: Uuid,
) -> CommandResult<bool> {
    let project = repo::get_java_project(&state.db, id).await?;
    let client = client_for(&project)?;
    Ok(client.cancel(invocation_id).await?)
}

/// 调用历史列表（时间倒序，`limit` 上限 500）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_java_history(
    state: State<'_, AppState>,
    id: Uuid,
    limit: Option<u32>,
) -> CommandResult<Vec<JavaInvocationRecord>> {
    Ok(repo::list_java_invocations(&state.db, id, limit.unwrap_or(100)).await?)
}

/// 单条调用历史详情。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_java_invocation(
    state: State<'_, AppState>,
    invocation_id: Uuid,
) -> CommandResult<JavaInvocationRecord> {
    Ok(repo::get_java_invocation(&state.db, invocation_id).await?)
}

/// 删除单条调用历史。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_java_invocation(
    state: State<'_, AppState>,
    invocation_id: Uuid,
) -> CommandResult<()> {
    Ok(repo::delete_java_invocation(&state.db, invocation_id).await?)
}

/// 清空项目调用历史。
#[tauri::command(rename_all = "camelCase")]
pub async fn clear_java_history(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    Ok(repo::clear_java_invocations(&state.db, id).await?)
}

// ---------------------------------------------------------------------------
// 场景引擎 Java 执行器（单源在 miyo-agent::java_hook，tauri 侧转发）
// ---------------------------------------------------------------------------

/// 构造场景引擎的 Java 步骤执行器（详见 miyo-agent::java_hook 模块文档）。
pub fn java_invoke_hook(db: sqlx::SqlitePool) -> Arc<miyo_test::runner::JavaInvokeFn> {
    miyo_agent::java_hook::java_invoke_hook(db)
}
