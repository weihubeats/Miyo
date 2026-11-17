//! Java Debug 2.0 Attach 命令：本地 JVM 进程发现 / Attach / Agent 通信
//! （类 / 方法 / Spring Bean 发现 + 受控调用）。
//!
//! 与 v1（commands/java_debug.rs，Runtime SDK 模式）的关系：
//! - v1 命令原样保留（旧项目兼容连接方式）；
//! - attach 会话复用 `java_projects` 行（mode='attach'），历史 / 审计 /
//!   场景引擎 / MCP 链路全部零改动复用；
//! - 插件门禁与 v1 同源（`ensure_java_plugin`）；
//! - attach / unload 属高危动作：写 audit 级插件日志。

use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

use miyo_java::protocol::InvocationStatus;
use miyo_java_attach::model::{JavaProcessInfo, ProcessListSummary};
use miyo_java_attach::{AttachOptions, AttachSessionSnapshot};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 插件门禁（与 v1 同源：未安装 / 未启用 → PLUGIN_* 错误码）。
async fn ensure_java_plugin(db: &sqlx::SqlitePool) -> CommandResult<()> {
    if !repo::is_plugin_installed(db, miyo_plugin::JAVA_DEBUG_PLUGIN_ID).await? {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginNotInstalled,
            "Java 调试插件未安装：请在插件管理中安装后使用",
        ));
    }
    if !repo::is_plugin_enabled(db, miyo_plugin::JAVA_DEBUG_PLUGIN_ID).await? {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginDisabled,
            "Java 调试插件已停用：请在插件管理中启用",
        ));
    }
    Ok(())
}

/// 高危动作审计（attach / unload / invoke）。
async fn audit(db: &sqlx::SqlitePool, message: &str) {
    if let Err(e) = repo::insert_plugin_log(
        db,
        miyo_plugin::JAVA_DEBUG_PLUGIN_ID,
        "audit",
        message,
        None,
    )
    .await
    {
        tracing::warn!("Java attach 审计写入失败：{e}");
    }
}

/// attach 会话的项目行（历史 / 审计归属；首次 attach 自动建行）。
async fn attach_project_id(
    db: &sqlx::SqlitePool,
    snapshot: &AttachSessionSnapshot,
) -> CommandResult<Uuid> {
    let display = format!(
        "PID {}（{}）",
        snapshot.pid, snapshot.handshake.java_version
    );
    Ok(repo::upsert_attach_project(db, &display, snapshot.pid, snapshot.vm_start_ms).await?)
}

// ---------------------------------------------------------------------------
// 进程发现
// ---------------------------------------------------------------------------

/// 发现本机 Java 进程（OS 扫描 + Attach listener 交叉 + 版本富化 + 脱敏）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_discover_processes(
    state: State<'_, AppState>,
) -> CommandResult<ProcessListSummary> {
    Ok(state.java_attach.discover_processes().await)
}

/// 手动输入 pid 探测（手动 attach 前的可达性确认）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_probe_pid(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<JavaProcessInfo> {
    state
        .java_attach
        .probe_pid(pid)
        .await
        .map_err(CommandError::from)
}

// ---------------------------------------------------------------------------
// Attach / Detach / Unload
// ---------------------------------------------------------------------------

/// attach 目标进程（高危：动态注入 Agent——需前端二次确认后调用）。
///
/// 流程：launcher sidecar attach → loadAgent → infoFile 轮询 → IPC 握手
/// （token / 协议 / pid 身份校验）→ 会话注册 → java_projects(mode=attach)
/// 行落库（历史归属）。幂等：已 attach 且健康 → 直接返回快照。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_attach_process(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<AttachSessionSnapshot> {
    ensure_java_plugin(&state.db).await?;
    let options = AttachOptions::default();
    let snapshot = state
        .java_attach
        .attach(pid, &options)
        .await
        .map_err(CommandError::from)?;
    attach_project_id(&state.db, &snapshot).await?;
    audit(
        &state.db,
        &format!(
            "attach pid={} vm_start={} agent={} ({})",
            snapshot.pid,
            snapshot.vm_start_ms,
            snapshot.agent_version,
            if snapshot.reattached {
                "重连"
            } else {
                "注入"
            }
        ),
    )
    .await;
    Ok(snapshot)
}

/// 断开连接（Agent 服务继续运行；重连无需重新注入）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_detach_process(state: State<'_, AppState>, pid: i64) -> CommandResult<()> {
    state
        .java_attach
        .detach(pid)
        .await
        .map_err(CommandError::from)
}

/// 卸载 Agent（agent.stop：停服务 + 清 infoFile；已加载的类无法卸载——诚实语义）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_unload_agent(state: State<'_, AppState>, pid: i64) -> CommandResult<()> {
    ensure_java_plugin(&state.db).await?;
    state
        .java_attach
        .unload_agent(pid)
        .await
        .map_err(CommandError::from)?;
    audit(&state.db, &format!("unload-agent pid={pid}")).await;
    Ok(())
}

/// 当前全部 attach 会话快照。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_attach_sessions(
    state: State<'_, AppState>,
) -> CommandResult<Vec<AttachSessionSnapshot>> {
    Ok(state.java_attach.all_snapshots().await)
}

// ---------------------------------------------------------------------------
// Agent 通信（类 / 方法 / Bean 发现）
// ---------------------------------------------------------------------------

/// Agent 状态（在途 / 统计 / 存活）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_status(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .status()
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// Spring 支持状态（上下文发现策略梯子 + 捕获状态，UI 分级展示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_spring_status(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .spring_status()
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// 已加载类清单（过滤 / 搜索 / 分页；JDK 内部类默认排除）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_classes(
    state: State<'_, AppState>,
    pid: i64,
    search: Option<String>,
    include_jdk: Option<bool>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .classes(
            search.as_deref(),
            None,
            None,
            include_jdk.unwrap_or(false),
            limit.unwrap_or(200),
            offset.unwrap_or(0),
        )
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// 类详情（父类 / 接口 / loader / 修饰符）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_class_detail(
    state: State<'_, AppState>,
    pid: i64,
    class: String,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .class_detail(&class)
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// 类方法清单（重载 / 泛型 / 风险分级 / 实例解析提示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_methods(
    state: State<'_, AppState>,
    pid: i64,
    class: String,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .methods(&class)
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// Spring Bean 清单（定义信息 + 已实例化单例的实际类型；不触发惰性实例化）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_beans(
    state: State<'_, AppState>,
    pid: i64,
    search: Option<String>,
    context_id: Option<String>,
    limit: Option<u32>,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .beans(
            search.as_deref().unwrap_or(""),
            context_id.as_deref().unwrap_or(""),
            limit.unwrap_or(500),
        )
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

// ---------------------------------------------------------------------------
// 受控调用（历史 / 审计复用 v1 链路）
// ---------------------------------------------------------------------------

/// attach 调用入参。
#[derive(Debug, Deserialize)]
pub struct JavaAgentInvokeArgs {
    /// 方法稳定标识 `Class#method(Types)`（历史 / 重放键）。
    pub method_id: String,
    /// 目标类全限定名。
    pub class: String,
    /// 方法名。
    pub method: String,
    /// 擦除参数类型表（重载定位；缺省要求方法名唯一）。
    #[serde(default)]
    pub param_types: Vec<String>,
    /// 位置参数（JSON 数组）。
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// 实例解析：{kind: static|bean|new, bean_name?, consent?}。
    #[serde(default)]
    pub instance: serde_json::Value,
    /// 调用超时毫秒。
    pub timeout_ms: Option<u64>,
    /// 人类可读签名（历史快照）。
    #[serde(default)]
    pub method_signature: Option<String>,
}

/// 归一参数（与 v1 同语义：数组原样 / 标量包装单元素 / null = 空参）。
fn normalize_arguments(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn status_str(status: &str) -> &'static str {
    match status {
        "success" => "success",
        "failed" => "failed",
        "timeout" => "timeout",
        "cancelled" => "cancelled",
        _ => "runtime_disconnected",
    }
}

/// Agent 方法调用（预检 → IPC invoke → 历史落库 → 审计）。
///
/// 响应为 Agent 协议 v1 形状（invocation_id / method_id / status / result /
/// exception / duration_ms…），额外字段（instance_kind / void / interrupt_sent）
/// 原样透传给前端。业务失败（异常 / 超时）不算链路错误。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_invoke(
    state: State<'_, AppState>,
    pid: i64,
    args: JavaAgentInvokeArgs,
) -> CommandResult<serde_json::Value> {
    ensure_java_plugin(&state.db).await?;
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    let invocation_id = Uuid::new_v4();
    let arguments = normalize_arguments(args.arguments.clone());

    // 黑名单预检（Agent 侧终检：名字前缀 + 可赋值性双闸）
    let class = args.class.trim();
    if miyo_java::policy::blacklisted(class) {
        return Err(CommandError::with_code(
            CommandErrorCode::Forbidden,
            format!("{class} 属于受限类，禁止通过调试协议调用"),
        ));
    }

    let mut params = serde_json::json!({
        "invocation_id": invocation_id.to_string(),
        "class": class,
        "method": args.method.trim(),
        "param_types": args.param_types,
        "args": arguments,
        "instance": args.instance,
    });
    if let Some(timeout_ms) = args.timeout_ms {
        params["timeout_ms"] = serde_json::json!(timeout_ms);
    }
    // 调用超时上限收敛（与 v1 口径一致）
    let timeout_ms = args
        .timeout_ms
        .unwrap_or(150_000)
        .min(miyo_java::SecurityPolicy::default().max_timeout_ms);

    let response = client
        .invoke(params, timeout_ms)
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)?;
    let status = response
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("failed")
        .to_string();
    let duration = response
        .get("duration_ms")
        .and_then(|d| d.as_f64())
        .unwrap_or(0.0);

    // 历史落库（与 v1 同表同链路；大字段截断由存储层策略一致）
    let record = miyo_core::model::JavaInvocationRecord {
        id: invocation_id,
        project_id: repo::find_attach_project_by_pid(&state.db, pid)
            .await
            .ok()
            .flatten()
            .map(|p| p.id)
            .unwrap_or_else(Uuid::nil),
        class_name: class.to_string(),
        method_id: args.method_id.clone(),
        method_signature: args.method_signature.clone().unwrap_or_default(),
        arguments: args.arguments.clone(),
        status: status_str(&status).to_string(),
        result: response.get("result").cloned(),
        result_truncated: response
            .get("result_truncated")
            .and_then(|t| t.as_bool())
            .unwrap_or(false),
        exception: response.get("exception").cloned(),
        duration_ms: Some(duration),
        started_at: response
            .get("started_at")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        finished_at: response
            .get("finished_at")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        created_at: chrono::Utc::now(),
    };
    if let Err(e) = repo::insert_java_invocation(&state.db, &record).await {
        tracing::warn!("attach 调用历史落库失败：{e}");
    }
    audit(
        &state.db,
        &format!(
            "attach-invoke {pid} {} status={status} duration={duration:.0}ms",
            args.method_id
        ),
    )
    .await;
    Ok(response)
}

/// 取消在途调用（协作式中断——UI 必须区分「已请求取消」与「实际终止」）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_cancel(
    state: State<'_, AppState>,
    pid: i64,
    invocation_id: String,
) -> CommandResult<serde_json::Value> {
    let client = state
        .java_attach
        .client(pid)
        .await
        .map_err(CommandError::from)?;
    client
        .cancel(&invocation_id)
        .await
        .map_err(|e| e.to_app_error())
        .map_err(CommandError::from)
}

/// status 字符串 → 旧枚举（场景引擎 / MCP 复用工具函数）。
pub fn invocation_status_of(status: &str) -> InvocationStatus {
    match status {
        "success" => InvocationStatus::Success,
        "failed" => InvocationStatus::Failed,
        "timeout" => InvocationStatus::Timeout,
        "cancelled" => InvocationStatus::Cancelled,
        _ => InvocationStatus::RuntimeDisconnected,
    }
}
