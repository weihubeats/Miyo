//! miyo-mcp：Miyo 的 Model Context Protocol 服务器。
//!
//! stdio transport（换行分隔 JSON-RPC 2.0），把 Agent 控制面暴露为 MCP 工具
//! 集合（清单与权限见 `miyo_agent::registry`），并实现：
//! - `tools/list` / `tools/call`（含 MCP 标准注解）；
//! - `resources/list` / `resources/read`（项目 / 接口 / 环境 / 场景 / 运行）；
//! - `prompts/list` / `prompts/get`（测试设计 / 场景编排 / 失败分析模板）；
//! - `notifications/tools/list_changed`（initialize 后轮询工具启停）；
//! - `notifications/progress`（run_scenario 长任务，progressToken 触发）。
//!
//! Claude Code 配置示例（`.mcp.json`）：
//!
//! ```json
//! { "mcpServers": { "miyo": { "command": "miyo-mcp" } } }
//! ```
//!
//! 前置条件：Miyo 桌面应用正在运行（自动拉起控制面，端口 4110 起）。

use std::io::{BufRead, Write};
use std::sync::mpsc;

mod tools;

#[tokio::main]
async fn main() {
    // stdout 单写线程：响应（stdin 循环）与通知（list_changed 轮询）共用
    // 一条 channel，避免交错写坏行协议。
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for line in rx {
            if writeln!(out, "{line}").is_err() {
                break; // 客户端已关闭管道
            }
            let _ = out.flush();
        }
    });

    let stdin = std::io::stdin();
    let mut initialized = false;
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        // 协议要求：通知只能在 initialize 响应之后发送——首次 initialize
        // 成功应答后启动工具启停轮询。
        if !initialized && line.contains("\"method\":\"initialize\"") {
            initialized = true;
            let notify_tx = tx.clone();
            tools::spawn_list_changed_poller(move |note| {
                let _ = notify_tx.send(note);
            });
        }
        // notification（无 id）不回复；解析失败静默丢弃（MCP 客户端会重试/报错）。
        for response in tools::handle_line_discover(&line).await {
            if tx.send(response).is_err() {
                return; // 客户端已关闭管道
            }
        }
    }
}
