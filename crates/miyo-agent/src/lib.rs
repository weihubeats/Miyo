//! # miyo-agent：Agent 控制面
//!
//! 在本机回环地址暴露带令牌鉴权的 HTTP API，让 AI Agent（Claude / Cursor /
//! 任意能发 HTTP 的工具）把 cURL 命令保存为 Miyo 接口，无需人工粘贴。
//!
//! ## 设计要点
//!
//! - **只绑 `127.0.0.1`**：不对外网暴露；
//! - **Bearer 令牌鉴权**：首次启动生成随机 token 写入 `{data_dir}/agent-token`
//!   （权限 0600），请求需携带 `Authorization: Bearer <token>` 或 `X-Agent-Token`；
//! - **可配置端口**：绑定端口持久化在 settings（起始端口可改），实际端口写入
//!   `{data_dir}/agent-port` 供 [`client::ControlClient::discover_default`] 直连；
//! - **单写入者**：HTTP 层只做解析 + 落库，全部写操作经由 App 持有的同一连接池，
//!   不引入第二个 SQLite 写入进程；
//! - **事件广播**：导入成功后通过 `broadcast` channel 发出 [`server::AgentEvent`]，
//!   由 Tauri 层转发为前端事件刷新 UI。
//!
//! ## API 一览
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET  | `/agent/health` | 存活探针（同样需要 token） |
//! | POST | `/agent/curl` | 导入 cURL 为接口 |
//! | GET  | `/agent/projects` | 项目列表（供 Agent 选择目标项目） |
//! | GET  | `/agent/endpoints/{project_id}` | 项目下的接口列表 |
//! | GET  | `/agent/scenarios?projectId=` | 场景列表（脚本协作） |
//! | GET  | `/agent/scenarios/{id}` | 场景详情（步骤 + 接口配置 + 链路变量 + 项目变量） |
//! | PUT  | `/agent/scenarios/{id}/script` | 写入 / 清除场景自定义脚本 |

pub mod client;
pub mod import;
pub mod portfile;
pub mod server;
pub mod token;

pub use client::{default_data_dir, ControlClient};
pub use server::{
    start, AgentEvent, AgentServer, AgentState, McpToolDef, McpToolState,
    SaveScenarioScriptRequest, SaveScenarioScriptResult, ScenarioDetail, ScenarioStepDetail,
    ScenarioSummary, MCP_DISABLED_TOOLS_KEY, MCP_TOOLS,
};
pub use token::load_or_create_token;
