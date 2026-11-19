//! 插件 Sidecar 命令：常驻型插件子进程的启动 / 停止 / 状态查询。
//!
//! 门禁（后端强制）：
//! - 插件已安装且启用（PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED）；
//! - granted 快照含 `runtime.process.start`（启动）/ `runtime.process.stop`
//!   （停止 / 状态查询）——权限细分与 java 族同口径；
//! - manifest.kind 必须是 ui-sidecar 且 sidecar.executable 在包内（files
//!   清单校验在安装 / 启用时完成）；当前平台必须在 manifest.platforms
//!   声明内（未声明 = 全平台）。
//!
//! 生命周期由 `miyo_sidecar::Supervisor` 承担（握手鉴权 / 心跳 / 崩溃恢复 /
//! 进程组清理）——本模块只做编排与门禁。

use std::sync::Arc;

use tauri::State;

use miyo_plugin::manifest::PluginKind;
use miyo_sidecar::{RestartPolicy, SidecarSpec, SidecarStatus, Supervisor};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 插件门禁：已安装 + 已启用 + 指定 runtime.* 权限。
async fn ensure_sidecar_permission(
    db: &sqlx::SqlitePool,
    id: &str,
    permission: &str,
) -> CommandResult<repo::PluginRow> {
    let row = repo::require_plugin(db, id).await?;
    if !repo::is_plugin_enabled(db, id).await? {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginDisabled,
            "插件未启用：sidecar 不可用",
        ));
    }
    let granted = row.granted_permissions();
    if !miyo_plugin::permission::granted_effective(&granted, permission) {
        return Err(CommandError::with_code(
            CommandErrorCode::Forbidden,
            format!("插件缺少权限「{permission}」：请在插件管理 → 权限中授权"),
        ));
    }
    Ok(row)
}

/// 从 manifest.sidecar 构造启动规格（含平台声明校验 + 可执行文件校验）。
fn build_spec(row: &repo::PluginRow) -> CommandResult<SidecarSpec> {
    let manifest = row
        .manifest()
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.user_message()))?;
    if manifest.kind != PluginKind::UiSidecar {
        return Err(CommandError::validation("该插件不是 ui-sidecar 形态"));
    }
    let sidecar = manifest
        .sidecar
        .as_ref()
        .ok_or_else(|| CommandError::validation("manifest 缺少 sidecar 配置"))?;
    // 平台声明校验（未声明 = 全平台）
    if let Some(platforms) = &manifest.platforms {
        let current = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "-arm64"
        } else {
            "-x64"
        };
        let ok = platforms
            .iter()
            .any(|p| p == current || p == &format!("{current}{arch}"));
        if !ok {
            return Err(CommandError::validation(format!(
                "当前平台（{current}{arch}）不在插件 platforms 声明内：{platforms:?}"
            )));
        }
    }
    let install_path = row.install_path.as_ref().ok_or_else(|| {
        CommandError::with_code(
            CommandErrorCode::NotFound,
            "内置插件无磁盘资源（sidecar 需包内可执行文件）",
        )
    })?;
    let executable = std::path::PathBuf::from(install_path).join(&sidecar.executable);
    // unix：可执行位（安装端不保留权限位，启动前兜底 chmod +x）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&executable) {
            if meta.is_file() {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o755);
                let _ = std::fs::set_permissions(&executable, perms);
            }
        }
    }
    Ok(SidecarSpec {
        plugin_id: row.id.clone(),
        executable,
        args: vec![],
        working_dir: Some(std::path::PathBuf::from(install_path)),
        max_memory_mb: sidecar.max_memory_mb,
        restart_policy: RestartPolicy::parse(sidecar.restart_policy.as_deref().unwrap_or("off")),
    })
}

/// 启动插件 sidecar（幂等替换；需 runtime.process.start 权限）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_sidecar_start(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<SidecarStatus> {
    let row = ensure_sidecar_permission(&state.db, &id, "runtime.process.start").await?;
    let spec = build_spec(&row)?;
    let supervisor = Arc::clone(&state.sidecar);
    supervisor.start(spec).await?;
    let status = supervisor
        .status()
        .await
        .into_iter()
        .find(|s| s.plugin_id == id)
        .ok_or_else(|| {
            CommandError::with_code(CommandErrorCode::Internal, "sidecar 启动后状态缺失")
        })?;
    audit(&state.db, &id, &format!("sidecar start pid={}", status.pid)).await;
    Ok(status)
}

/// 停止插件 sidecar（需 runtime.process.stop 权限）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_sidecar_stop(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    ensure_sidecar_permission(&state.db, &id, "runtime.process.stop").await?;
    let supervisor = Arc::clone(&state.sidecar);
    supervisor.stop(&id).await?;
    audit(&state.db, &id, "sidecar stop").await;
    Ok(())
}

/// 插件 → 自己 sidecar 的通用 RPC 通道（req/res 透传；需
/// runtime.process.start 权限——启动面即调用面，能力一致才可调用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_sidecar_request(
    state: State<'_, AppState>,
    id: String,
    method: String,
    params: Option<serde_json::Value>,
) -> CommandResult<serde_json::Value> {
    ensure_sidecar_permission(&state.db, &id, "runtime.process.start").await?;
    // 方法名白名单字符校验（防控制字符注入帧协议）
    if method.is_empty()
        || method.len() > 64
        || !method
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(CommandError::validation(format!(
            "sidecar 方法名非法：{method}"
        )));
    }
    let supervisor = Arc::clone(&state.sidecar);
    supervisor
        .request(
            &id,
            &method,
            params.unwrap_or(serde_json::Value::Null),
            miyo_sidecar::supervisor::REQUEST_TIMEOUT,
        )
        .await
        .map_err(command_err_sidecar)
}

fn command_err_sidecar(e: miyo_core::AppError) -> CommandError {
    CommandError::from(e)
}

/// 全部 sidecar 运行状态（宿主管理中心视图；不经 SDK 白名单暴露给插件，
/// 插件仅能通过 start / stop / request 管理自己的进程）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_sidecar_status(
    state: State<'_, AppState>,
) -> CommandResult<Vec<SidecarStatus>> {
    Ok(state.sidecar.status().await)
}

async fn audit(db: &sqlx::SqlitePool, plugin_id: &str, message: &str) {
    if let Err(e) = repo::insert_plugin_log(db, plugin_id, "audit", message, None).await {
        tracing::warn!("sidecar 审计日志写入失败（{plugin_id}）：{e}");
    }
}

/// Supervisor 装配（AppState::new 调用）：日志经 sink 落 plugin_logs；
/// 崩溃重启门禁接「插件启用态」查询（停用 / 卸载即终止重启链）。
pub fn build_supervisor(db: sqlx::SqlitePool) -> Arc<Supervisor> {
    struct LogSink(sqlx::SqlitePool);
    impl miyo_sidecar::SidecarSink for LogSink {
        fn log(&self, plugin_id: &str, level: &str, message: &str) {
            let pool = self.0.clone();
            let plugin_id = plugin_id.to_string();
            let level = level.to_string();
            let message = message.chars().take(2000).collect::<String>();
            tokio::spawn(async move {
                if let Err(e) =
                    repo::insert_plugin_log(&pool, &plugin_id, &level, &message, None).await
                {
                    tracing::warn!("sidecar 日志落库失败：{e}");
                }
            });
        }
        fn event(&self, plugin_id: &str, event: &str, payload: serde_json::Value) {
            // 事件总线前端转发为路线图（tauri emit 注入）；当前如实落日志
            tracing::debug!("sidecar 事件（{plugin_id}）：{event} {}", payload);
        }
    }
    let supervisor = Arc::new(Supervisor::new(Arc::new(LogSink(db.clone()))));
    // 重启门禁：重启前复查插件启用态。DB 不可达按禁用处理（fail-closed：
    // 宁可少一次自动重启，不可把已停用插件的进程复活）。
    supervisor.set_restart_gate(Arc::new(move |plugin_id: &str| {
        let pool = db.clone();
        let id = plugin_id.to_string();
        Box::pin(async move { matches!(repo::is_plugin_enabled(&pool, &id).await, Ok(true)) })
    }));
    supervisor
}
