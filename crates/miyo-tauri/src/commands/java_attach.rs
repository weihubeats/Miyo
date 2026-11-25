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
use tauri::{Manager, State};
use uuid::Uuid;

use miyo_java::protocol::InvocationStatus;
use miyo_java_attach::model::{JavaProcessInfo, ProcessListSummary};
use miyo_java_attach::{AttachOptions, AttachSessionSnapshot};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

use crate::commands::java_gate::ensure_java_permission;
use miyo_java_attach::backend::{AgentOp, ClassesQuery};

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
///
/// 门禁：发现会 spawn launcher sidecar 子进程扫描本机——插件未安装 /
/// 未启用时一律拒绝（与「未装插件不启动 Sidecar」口径一致）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_discover_processes(
    state: State<'_, AppState>,
) -> CommandResult<ProcessListSummary> {
    ensure_java_permission(&state.db, "java.process.discover").await?;
    Ok(state.java_backend.discover_processes().await?)
}

/// 手动输入 pid 探测（手动 attach 前的可达性确认；探测同样 spawn launcher）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_probe_pid(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<JavaProcessInfo> {
    ensure_java_permission(&state.db, "java.process.discover").await?;
    state
        .java_backend
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
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    let options = AttachOptions::default();
    let snapshot = state
        .java_backend
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
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    state
        .java_backend
        .detach(pid)
        .await
        .map_err(CommandError::from)
}

/// 卸载 Agent（agent.stop：停服务 + 清 infoFile；已加载的类无法卸载——诚实语义）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_unload_agent(state: State<'_, AppState>, pid: i64) -> CommandResult<()> {
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    state
        .java_backend
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
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    Ok(state.java_backend.all_snapshots().await?)
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
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    state
        .java_backend
        .agent_call(pid, AgentOp::Status)
        .await
        .map_err(CommandError::from)
}

/// Spring 支持状态（上下文发现策略梯子 + 捕获状态，UI 分级展示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_spring_status(
    state: State<'_, AppState>,
    pid: i64,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    state
        .java_backend
        .agent_call(pid, AgentOp::SpringStatus)
        .await
        .map_err(CommandError::from)
}

/// 已加载类清单（分类 / 包前缀 / 搜索 / 分页）。
///
/// 默认只返回「业务类 + Spring Bean」：依赖与 JDK 需要显式
/// `includeDependencies` / `includeJdk` 才出现（分类由 agent 侧按 codeSource /
/// 启动信息 / Bean 角色判定，不靠包名黑名单）。
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn java_agent_classes(
    state: State<'_, AppState>,
    pid: i64,
    search: Option<String>,
    include_jdk: Option<bool>,
    limit: Option<u32>,
    offset: Option<u32>,
    category: Option<String>,
    package_prefix: Option<String>,
    include_dependencies: Option<bool>,
    business_packages: Option<String>,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    let mut query = ClassesQuery::new();
    query.search = search;
    query.category = category;
    query.package_prefix = package_prefix;
    query.include_dependencies = include_dependencies.unwrap_or(false);
    query.include_jdk = include_jdk.unwrap_or(false);
    query.business_packages = business_packages;
    query.limit = limit.unwrap_or(200);
    query.offset = offset.unwrap_or(0);
    state
        .java_backend
        .agent_call(pid, AgentOp::Classes(query))
        .await
        .map_err(CommandError::from)
}

/// 类分类统计（业务 / Bean / 依赖 / JDK / 未识别计数 + 分类证据摘要）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_classes_stats(
    state: State<'_, AppState>,
    pid: i64,
    business_packages: Option<String>,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    state
        .java_backend
        .agent_call(
            pid,
            AgentOp::ClassesStats {
                business_packages: business_packages.filter(|s| !s.trim().is_empty()),
            },
        )
        .await
        .map_err(CommandError::from)
}

/// 全量重建类索引（用户修改业务包前缀后，让分类按新配置重算）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_classes_reindex(
    state: State<'_, AppState>,
    pid: i64,
    business_packages: Option<String>,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    state
        .java_backend
        .agent_call(
            pid,
            AgentOp::ClassesReindex {
                business_packages: business_packages.filter(|s| !s.trim().is_empty()),
            },
        )
        .await
        .map_err(CommandError::from)
}

/// 类详情（父类 / 接口 / loader / 修饰符）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_class_detail(
    state: State<'_, AppState>,
    pid: i64,
    class: String,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    state
        .java_backend
        .agent_call(pid, AgentOp::ClassDetail { class })
        .await
        .map_err(CommandError::from)
}

/// 类方法清单（重载 / 泛型 / 风险分级 / 实例解析提示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_agent_methods(
    state: State<'_, AppState>,
    pid: i64,
    class: String,
) -> CommandResult<serde_json::Value> {
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    state
        .java_backend
        .agent_call(pid, AgentOp::Methods { class })
        .await
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
    ensure_java_permission(&state.db, "java.class.inspect").await?;
    state
        .java_backend
        .agent_call(
            pid,
            AgentOp::Beans {
                search: search.unwrap_or_default(),
                context_id: context_id.unwrap_or_default(),
                limit: limit.unwrap_or(500),
            },
        )
        .await
        .map_err(CommandError::from)
}

// ---------------------------------------------------------------------------
// 受控调用（历史 / 审计复用 v1 链路）
// ---------------------------------------------------------------------------

/// attach 调用入参。
///
/// 字段用 camelCase 反序列化：前端 `useMiyoApi.javaAgentInvoke` 传的是
/// `methodId` / `paramTypes` / `timeoutMs`，而命令上的
/// `#[tauri::command(rename_all = "camelCase")]` **只作用于顶层命令参数**
/// （`pid`、`args`），不会下渗到结构体字段。缺这行时 serde 只认
/// `method_id`，前端调用直接报
/// `invalid args 'args' for command 'java_agent_invoke': missing field 'method_id'`
/// （真实用户调用 `isDebtFullyCleared` 时踩到）。
///
/// 同时保留 snake_case 别名：历史回放 / 外部脚本可能按协议原文传参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaAgentInvokeArgs {
    /// 方法稳定标识 `Class#method(Types)`（历史 / 重放键）。
    #[serde(alias = "method_id")]
    pub method_id: String,
    /// 目标类全限定名。
    pub class: String,
    /// 方法名。
    pub method: String,
    /// 擦除参数类型表（重载定位；缺省要求方法名唯一）。
    #[serde(default, alias = "param_types")]
    pub param_types: Vec<String>,
    /// 位置参数（JSON 数组）。
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// 实例解析：{kind: static|bean|new, bean_name?, consent?}。
    #[serde(default)]
    pub instance: serde_json::Value,
    /// 调用超时毫秒。
    #[serde(alias = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    /// 人类可读签名（历史快照）。
    #[serde(default, alias = "method_signature")]
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
    ensure_java_permission(&state.db, "java.method.invoke").await?;
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

    let response = state
        .java_backend
        .agent_call(pid, AgentOp::Invoke { params, timeout_ms })
        .await
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
    ensure_java_permission(&state.db, "java.runtime.attach").await?;
    state
        .java_backend
        .agent_call(pid, AgentOp::Cancel { invocation_id })
        .await
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

/// 应用退出清理（frontend/src-tauri main.rs 在 `RunEvent::ExitRequested` 调用）：
/// 尽力停止全部目标 JVM 内 Agent 服务（停 IPC + 清 infoFile）并移除会话。
/// Agent 已加载类不可卸载（诚实语义）；目标 JVM 继续运行。
pub async fn shutdown_attach_on_exit(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.java_attach.shutdown_all().await;
    }
}

#[cfg(test)]
mod invoke_args_tests {
    use super::JavaAgentInvokeArgs;

    /// 前端 `useMiyoApi.javaAgentInvoke` 传的是驼峰字段。
    ///
    /// 回归（真实用户调用 `isDebtFullyCleared` 报
    /// `invalid args 'args' for command 'java_agent_invoke': missing field 'method_id'`）：
    /// 命令上的 `#[tauri::command(rename_all = "camelCase")]` 只作用于顶层命令
    /// 参数，不会下渗到结构体字段；结构体本身缺 `rename_all` 时 serde 只认
    /// snake_case。
    #[test]
    fn deserializes_camel_case_payload_from_frontend() {
        let json = serde_json::json!({
            "methodId": "com.redotpay.CardTransactionBillService#isDebtFullyCleared(long)",
            "class": "com.redotpay.CardTransactionBillService",
            "method": "isDebtFullyCleared",
            "paramTypes": ["long"],
            "arguments": [1],
            "instance": { "kind": "bean" },
            "timeoutMs": 30000,
            "methodSignature": "boolean isDebtFullyCleared(Long)"
        });
        let args: JavaAgentInvokeArgs = serde_json::from_value(json).unwrap();
        assert_eq!(
            args.method_id,
            "com.redotpay.CardTransactionBillService#isDebtFullyCleared(long)"
        );
        assert_eq!(args.param_types, vec!["long".to_string()]);
        assert_eq!(args.timeout_ms, Some(30000));
        assert_eq!(
            args.method_signature.as_deref(),
            Some("boolean isDebtFullyCleared(Long)")
        );
    }

    /// 兼容协议原文（snake_case）：历史回放 / 外部脚本可能直接传 `method_id`。
    #[test]
    fn deserializes_snake_case_payload_for_replay() {
        let json = serde_json::json!({
            "method_id": "a.B#m()",
            "class": "a.B",
            "method": "m",
            "param_types": [],
            "arguments": [],
            "timeout_ms": 5000
        });
        let args: JavaAgentInvokeArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.method_id, "a.B#m()");
        assert_eq!(args.timeout_ms, Some(5000));
    }

    /// 缺 `method_id` 仍应报错（不能因为兼容而放宽必填项）。
    #[test]
    fn still_requires_method_id() {
        let json = serde_json::json!({
            "class": "a.B",
            "method": "m"
        });
        let err = serde_json::from_value::<JavaAgentInvokeArgs>(json).unwrap_err();
        // rename_all 生效后 serde 报的是规范名（methodId）；snake_case 是 alias，
        // 不在错误信息里。两种命名都算通过。
        let msg = err.to_string();
        assert!(
            msg.contains("methodId") || msg.contains("method_id"),
            "{msg}"
        );
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use crate::commands::java_gate::ensure_java_plugin;

    /// 门禁分型：未安装 → PLUGIN_NOT_INSTALLED；已安装未启用 → PLUGIN_DISABLED；
    /// 启用 → 放行。attach 命令族 14/14 均经此门禁（Phase 1 收口）。
    #[tokio::test]
    async fn gate_rejects_uninstalled_disabled_and_passes_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let db = miyo_storage::db::init_db(&tmp.path().join("gate.db"))
            .await
            .unwrap();
        let id = miyo_plugin::JAVA_DEBUG_PLUGIN_ID;

        // 未安装
        let err = ensure_java_plugin(&db).await.unwrap_err();
        assert_eq!(err.code, CommandErrorCode::PluginNotInstalled);

        // 已安装（granted 含 java.debug）未启用
        let manifest_bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../miyo-plugin/builtins/java-debug/manifest.json"),
        )
        .unwrap();
        let manifest = miyo_plugin::PluginManifest::parse(&manifest_bytes).unwrap();
        repo::insert_plugin(
            &db,
            id,
            &manifest,
            "builtin",
            None,
            false,
            &manifest.permissions,
            None,
        )
        .await
        .unwrap();
        let err = ensure_java_plugin(&db).await.unwrap_err();
        assert_eq!(err.code, CommandErrorCode::PluginDisabled);

        // 启用 → 放行
        repo::set_plugin_state(&db, id, repo::STATE_ENABLED, None)
            .await
            .unwrap();
        ensure_java_plugin(&db).await.unwrap();
    }
}
