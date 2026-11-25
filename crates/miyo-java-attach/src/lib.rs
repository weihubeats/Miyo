//! # miyo-java-attach — Java Debug 2.0 的 Attach 层
//!
//! 零源码侵入调试的 Rust 侧三件套：
//!
//! 1. **进程发现**（[`process`]）：扫描本机正在运行的 Java 进程
//!    （macOS `ps` / Linux `/proc` / Windows PowerShell），输出 pid / 命令行 /
//!    启动时间 / Java home / JDK-JRE 判定 / 敏感参数脱敏后的启动参数。
//! 2. **JDK 工具链发现**（[`jdk`]）：枚举本机 JDK（JAVA_HOME、macOS
//!    java_home、常见目录、PATH），按「版本 ≥ 目标、新版本优先」为每个目标
//!    进程挑选 launcher 运行时。
//! 3. **Attach 管理与 Agent 通信**（[`manager`] / [`client`]）：以短生命周期
//!    sidecar（`miyo-attach-launcher.jar`，经 `java -jar` 调用）执行
//!    attach → loadAgent → 轮询 infoFile；随后直连目标 JVM 内 Agent 的
//!    127.0.0.1 随机端口 IPC（长度前缀 JSON 协议 v2，一次性 token 鉴权）。
//!
//! ## 架构定位
//!
//! ```text
//! ┌────────────────────────┐ spawn 子进程   ┌──────────────────────────┐
//! │ Miyo（Rust / Tauri）    │ ────────────> │ miyo-attach-launcher.jar │
//! │ miyo-java-attach crate │               │ JDK Attach API（短进程）   │
//! └──────────┬─────────────┘               └────────────┬─────────────┘
//!            │  TCP 127.0.0.1:随机端口 + token           │ VirtualMachine.attach
//!            ▼                                          ▼ loadAgent(agent.jar)
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ 目标 JVM（零源码修改）内：miyo-java-agent（Instrumentation + IPC 服务）│
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## 安全模型（摘要）
//!
//! - Agent 仅绑定 `127.0.0.1`；鉴权 = Miyo 生成的一次性 256-bit token
//!   （经 launcher → loadAgent 参数传入 Agent，不落盘）；
//! - 连接信息经 infoFile（0600）回传（不含 token）；
//! - 进程身份绑定：expectedPid 校验 + vm_start_ms（重启识别）；
//! - 协议版本握手不匹配即拒绝；帧上限 / 并发上限 / 超时上限双端收敛。
//!
//! 超时 / 取消语义与协议 v1 一致：协作式中断（`terminated` 恒为 false），
//! UI 必须区分「已请求取消」与「实际终止」。

pub mod backend;
pub mod client;
pub mod jdk;
pub mod launcher;
pub mod manager;
pub mod model;
pub mod process;

/// sidecar 分发模式下 Supervisor 会话的插件 id（与内置插件 id 一致）。
pub const JAVA_DEBUG_SIDECAR_PLUGIN_ID: &str = "com.miyo.java-debug";

pub use backend::ClassesQuery;
pub use client::{AgentClient, AgentClientError};
pub use jdk::{JdkToolchain, ToolchainRegistry};
pub use launcher::{LauncherRunner, LauncherToolPaths};
pub use manager::{AttachManager, AttachOptions};
pub use model::{AgentHandshakeInfo, AttachSessionSnapshot, JavaProcessInfo, ProcessListSummary};

/// Agent 通信协议版本（与 java/miyo-java-agent 的 AgentVersion.PROTOCOL_VERSION 对齐）。
pub const AGENT_PROTOCOL_VERSION: u32 = 2;

/// Agent jar 产物名（`java/dist/` 与 Tauri resources/java/ 下同名）。
pub const AGENT_JAR_NAME: &str = "miyo-java-agent.jar";
/// Launcher jar 产物名。
pub const LAUNCHER_JAR_NAME: &str = "miyo-attach-launcher.jar";

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_version_alignment() {
        // 与 Java 侧 AgentVersion 对齐（协议漂移在此显式暴露）
        assert_eq!(super::AGENT_PROTOCOL_VERSION, 2);
    }
}
