//! Miyo AI 大模型客户端。
//!
//! 面向「应用内 AI 测试中心」的 LLM 接入层：OpenAI 兼容协议
//! （`POST {base_url}/chat/completions`，覆盖 OpenAI / DeepSeek / Qwen / Kimi /
//! GLM / Ollama 等主流提供方），支持 SSE 流式输出与 function calling 工具调用。
//!
//! 分层约定：
//! - 本 crate 只懂协议，不懂业务：不依赖存储 / 控制面，工具执行由调用方
//!   注入（[`agent::ToolExecutor`]），副作用审批由调用方经
//!   [`agent::LoopObserver::approve_tool`] 决定；
//! - [`client::LlmClient`] 负责单轮流式请求与分片聚合；
//! - [`agent::run_agent_loop`] 负责多轮工具调用循环（模型 ↔ 工具往返，
//!   直到模型给出最终回答、达到迭代上限或被取消）。
//!
//! API Key 属敏感数据：由存储层（miyo-secret AES-256-GCM）加密落库，
//! 本 crate 只在内存中短暂持有明文。

pub mod agent;
pub mod client;
pub mod error;
pub mod types;

pub use agent::{
    run_agent_loop, AgentLoopParams, DenyAllObserver, LoopObserver, LoopStop, NoopObserver,
    ToolExecutor,
};
pub use client::LlmClient;
pub use error::AiError;
pub use types::{
    AiProviderConfig, ChatMessage, Role, RoundOutput, StreamEvent, ToolCall, ToolSpec,
};
