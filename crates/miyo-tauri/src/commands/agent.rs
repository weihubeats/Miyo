//! Agent 控制面 Command：启动（幂等，随应用自动拉起）/ 停止 / 查询状态。
//!
//! 服务监听 `127.0.0.1`，Bearer 令牌存于 `{data_dir}/agent-token`，
//! 实际监听端口存于 `{data_dir}/agent-port`（供 miyo-mcp 发现）；
//! 导入事件经 broadcast 转发为前端事件 `miyo:agent-event`。

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use miyo_agent::server::{AgentState, MCP_DISABLED_TOOLS_KEY};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// settings 表中的 MCP（Agent 控制面）开关键；值为 JSON bool。
/// 缺失时视为 `true`，保持「随应用自动拉起」的历史默认行为。
const MCP_ENABLED_KEY: &str = "mcp_enabled";

/// MCP 工具启停状态（IPC 出参，snake_case 遵守序列化契约）。
#[derive(Debug, serde::Serialize)]
pub struct McpToolStatus {
    pub name: String,
    /// read = 只读信息；write = 会落库。
    pub kind: &'static str,
    pub enabled: bool,
}

/// settings 表中的 MCP 监听起始端口键；值为 JSON 数字。
/// 缺失时回退默认 4110（与 `miyo_agent::DEFAULT_AGENT_PORT` 一致）。
const MCP_PORT_KEY: &str = "mcp_port";

/// Agent 服务状态信息（字段 snake_case，与 IPC 响应命名惯例一致）。
#[derive(Debug, Serialize)]
pub struct AgentStatusInfo {
    pub running: bool,
    /// 监听地址（未运行时为 `None`）。
    pub address: Option<String>,
    /// 令牌文件路径（Agent 配置时读取）。
    pub token_path: String,
}

/// 确保 Agent 控制面已启动（幂等）。返回监听地址。
///
/// 从 settings 读起始端口；绑定成功后把**实际端口**写入端口文件。
/// 启动前先订阅事件通道，避免最早的导入事件丢失；事件转发任务在
/// channel 关闭（服务停止）后自然退出。
pub async fn ensure_started(app: &tauri::AppHandle) -> CommandResult<String> {
    let state: State<'_, AppState> = app.state();
    let mut guard = state.agent.write().await;
    if let Some(server) = guard.as_ref() {
        // 运行中也刷新端口文件，容忍外部误删
        let _ = miyo_agent::portfile::write_port(&miyo_storage::db::data_dir(), server.port);
        return Ok(server.address());
    }

    let preferred_port = read_mcp_port(&state.db).await;
    let token = miyo_agent::load_or_create_token(&miyo_storage::db::data_dir())
        .map_err(|e| CommandError::with_code("IO", format!("Agent 令牌文件创建失败：{e}")))?;
    let agent_state = AgentState::new(state.db.clone(), token);
    let mut events = agent_state.subscribe();

    let server = miyo_agent::server::start_at(agent_state, preferred_port).await?;
    let address = server.address();
    let _ = miyo_agent::portfile::write_port(&miyo_storage::db::data_dir(), server.port);
    *guard = Some(server);
    drop(guard);

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let _ = handle.emit("miyo:agent-event", &event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[agent] 事件转发滞后，丢弃 {n} 条");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(address)
}

/// 停止控制面并清除端口文件（幂等；须已持有写锁或独占 State）。
async fn stop_locked(guard: &mut Option<miyo_agent::AgentServer>) {
    if let Some(server) = guard.take() {
        server.stop().await;
        miyo_agent::portfile::clear_port(&miyo_storage::db::data_dir());
    }
}

/// 启动 Agent 控制面（幂等，返回监听地址）。
#[tauri::command(rename_all = "camelCase")]
pub async fn agent_start(app: tauri::AppHandle) -> CommandResult<String> {
    ensure_started(&app).await
}

/// 停止 Agent 控制面（幂等）。
#[tauri::command(rename_all = "camelCase")]
pub async fn agent_stop(state: State<'_, AppState>) -> CommandResult<()> {
    let mut guard = state.agent.write().await;
    stop_locked(&mut guard).await;
    Ok(())
}

/// 查询 Agent 控制面状态与令牌文件位置。
#[tauri::command(rename_all = "camelCase")]
pub async fn agent_status(state: State<'_, AppState>) -> CommandResult<AgentStatusInfo> {
    let guard = state.agent.read().await;
    Ok(AgentStatusInfo {
        running: guard.is_some(),
        address: guard.as_ref().map(|s| s.address()),
        token_path: miyo_agent::token::token_path(&miyo_storage::db::data_dir())
            .display()
            .to_string(),
    })
}

/// 读取 MCP 启用开关（缺键 / 解析失败时默认启用）。
pub async fn read_mcp_enabled(db: &sqlx::SqlitePool) -> bool {
    match repo::get_setting(db, MCP_ENABLED_KEY).await {
        Ok(Some(json)) => serde_json::from_str::<bool>(&json).unwrap_or(true),
        _ => true,
    }
}

/// 读取 MCP 监听起始端口（缺键 / 解析失败时默认 4110）。
pub async fn read_mcp_port(db: &sqlx::SqlitePool) -> u16 {
    match repo::get_setting(db, MCP_PORT_KEY).await {
        Ok(Some(json)) => serde_json::from_str::<u16>(&json)
            .ok()
            .filter(|p| *p >= 1)
            .unwrap_or(miyo_agent::server::DEFAULT_AGENT_PORT),
        _ => miyo_agent::server::DEFAULT_AGENT_PORT,
    }
}

/// 读取 MCP 启用开关（设置页展示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_mcp_enabled(state: State<'_, AppState>) -> CommandResult<bool> {
    Ok(read_mcp_enabled(&state.db).await)
}

/// 读取 MCP 监听起始端口（设置页展示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_mcp_port(state: State<'_, AppState>) -> CommandResult<u16> {
    Ok(read_mcp_port(&state.db).await)
}

/// 设置 MCP 启用开关：true → 确保控制面已启动；false → 停止控制面。
///
/// 先应用后持久化——启动失败时不落盘，前端开关回滚并展示错误；
/// 关闭成功后下次应用启动将跳过自动拉起。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_mcp_enabled(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> CommandResult<()> {
    if enabled {
        ensure_started(&app).await?;
    } else {
        let mut guard = state.agent.write().await;
        stop_locked(&mut guard).await;
    }
    let json = serde_json::to_string(&enabled)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, MCP_ENABLED_KEY, &json).await?;
    Ok(())
}

/// MCP 工具清单及启停状态（设置页「工具权限」回显）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_mcp_tools(state: State<'_, AppState>) -> CommandResult<Vec<McpToolStatus>> {
    let disabled = match repo::get_setting(&state.db, MCP_DISABLED_TOOLS_KEY).await {
        Ok(Some(raw)) => serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(miyo_agent::server::MCP_TOOLS
        .iter()
        .map(|t| McpToolStatus {
            name: t.name.to_string(),
            kind: t.kind,
            enabled: !disabled.iter().any(|n| n == t.name),
        })
        .collect())
}

/// 设置单个 MCP 工具的启停（实时生效：控制面与 miyo-mcp 每次调用实时读 settings）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_mcp_tool_enabled(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> CommandResult<()> {
    if !miyo_agent::server::MCP_TOOLS.iter().any(|t| t.name == name) {
        return Err(CommandError::validation(format!("未知工具：{name}")));
    }
    let mut disabled: Vec<String> = match repo::get_setting(&state.db, MCP_DISABLED_TOOLS_KEY).await
    {
        Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
        _ => Vec::new(),
    };
    if enabled {
        disabled.retain(|n| n != &name);
    } else if !disabled.iter().any(|n| n == &name) {
        disabled.push(name);
    }
    let json = serde_json::to_string(&disabled)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, MCP_DISABLED_TOOLS_KEY, &json).await?;
    Ok(())
}

/// 设置 MCP 监听起始端口（1~65535）。运行中则先停再按新端口重启。
///
/// 端口未变化时直接返回（数字输入失焦会重复触发 change）。
/// 新端口启动失败时回滚旧设置并尽量恢复旧端口监听。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_mcp_port(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    port: u16,
) -> CommandResult<()> {
    if port < 1 {
        return Err(CommandError::validation("端口需在 1 ~ 65535 之间"));
    }
    let prev = read_mcp_port(&state.db).await;
    if port == prev {
        return Ok(());
    }
    let prev_json = serde_json::to_string(&prev)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    let next_json = serde_json::to_string(&port)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;

    let was_running = {
        let mut guard = state.agent.write().await;
        let running = guard.is_some();
        if running {
            stop_locked(&mut guard).await;
        }
        running
    };

    repo::set_setting(&state.db, MCP_PORT_KEY, &next_json).await?;
    if !was_running {
        return Ok(());
    }

    if let Err(e) = ensure_started(&app).await {
        // 新端口起不来：回滚设置并恢复旧端口监听
        let _ = repo::set_setting(&state.db, MCP_PORT_KEY, &prev_json).await;
        if let Err(recover) = ensure_started(&app).await {
            tracing::warn!("[agent] 端口回滚后恢复失败：{recover}");
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IPC 契约回归：AgentStatusInfo 按 snake_case 序列化
    /// （token_path 不得变成 tokenPath，与全仓响应命名惯例一致）。
    #[test]
    fn agent_status_serializes_snake_case() {
        let info = AgentStatusInfo {
            running: false,
            address: None,
            token_path: "/tmp/agent-token".into(),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json.get("token_path").is_some(), "缺少字段 token_path");
        assert!(json.get("tokenPath").is_none(), "不得输出 camelCase 字段");
        assert_eq!(json.get("running"), Some(&serde_json::Value::Bool(false)));
    }
}
