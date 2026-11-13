//! AI 测试中心的分析 / 生成服务。
//!
//! 纯启发式与模板规则（不调 LLM）：同一份实现同时服务两条路径——
//! ① MCP 工具（外部 AI Agent 经控制面调用，见 [`crate::server`]）；
//! ② 应用内直连（Tauri 命令，左侧面板的依赖图 / 覆盖率摘要等，无需 LLM）。
//!
//! 所有输出均为 camelCase JSON（与控制面既有 handler 口径一致），
//! 敏感字段在返回前由 server 层统一脱敏。

pub mod coverage;
pub mod dependencies;
pub mod failure;
pub mod fix;
pub mod scenario_gen;
pub mod testcase_gen;
