//! miyo-mcp：Miyo 的 Model Context Protocol 服务器。
//!
//! stdio transport（换行分隔 JSON-RPC 2.0），把 Agent 控制面暴露为 7 个工具：
//! `save_curl` / `list_projects` / `list_endpoints` / `list_scenarios` /
//! `get_scenario` / `save_scenario_script` / `agent_info`。
//!
//! Claude Code 配置示例（`.mcp.json`）：
//!
//! ```json
//! { "mcpServers": { "miyo": { "command": "miyo-mcp" } } }
//! ```
//!
//! 前置条件：Miyo 桌面应用正在运行（自动拉起控制面，端口 4110 起）。

use std::io::{BufRead, Write};

mod tools;

#[tokio::main]
async fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        // notification（无 id）不回复；解析失败静默丢弃（MCP 客户端会重试/报错）。
        let Some(response) = tools::handle_line_discover(&line).await else {
            continue;
        };
        if writeln!(out, "{response}").is_err() {
            break; // 客户端已关闭管道
        }
        let _ = out.flush();
    }
}
