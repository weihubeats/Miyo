//! 类型定义：提供方配置、对话消息、工具规格、流式事件。

use serde::{Deserialize, Serialize};

/// AI 提供方配置（OpenAI 兼容协议）。
///
/// `api_key` 由存储层加密落库；本结构在内存中持有明文，禁止日志输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    /// API Base URL（如 `https://api.deepseek.com/v1`；客户端自动拼
    /// `/chat/completions`，容忍尾斜杠与 `/v1` 缺省——缺省时不自动补全，
    /// 各家路径习惯不一，由用户/预设填全）。
    pub base_url: String,
    /// API Key。
    #[serde(default)]
    pub api_key: String,
    /// 模型名（如 `deepseek-chat`）。
    pub model: String,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// 单次回复最大 token（None = 服务端默认）。
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl AiProviderConfig {
    /// 配置是否足够发起请求。
    pub fn is_complete(&self) -> bool {
        !self.base_url.trim().is_empty()
            && !self.model.trim().is_empty()
            && (self.base_url.starts_with("http://") || self.base_url.starts_with("https://"))
    }
}

/// 对话角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// 工具结果消息（OpenAI 协议的 tool role）。
    Tool,
}

/// 一次工具调用（模型发起；arguments 为 JSON 字符串）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_tool_kind")]
    pub kind: String,
    pub function: ToolCallFunction,
}

fn default_tool_kind() -> String {
    "function".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

/// 对话消息（内部表示；线上格式由 [`crate::client::LlmClient`] 组装）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    /// 文本内容（工具结果消息为 JSON 字符串；assistant 带工具调用时可为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// assistant 消息携带的工具调用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// tool 消息对应的调用 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::System,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::User,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// assistant 文本回复（无工具调用）。
    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Assistant,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// assistant 工具调用消息（content 序列化为 null，兼容要求 content 字段的提供方）。
    pub fn assistant_with_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        ChatMessage {
            role: Role::Assistant,
            content,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    /// tool 结果消息。
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// 暴露给模型的工具规格（JSON Schema）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// 参数 JSON Schema（顶层应为 object）。
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// 流式事件（客户端聚合前的原始增量）。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 助手正文增量。
    ContentDelta(String),
    /// 工具调用增量分片（同一 index 的 arguments 分片按序拼接）。
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// 本轮流结束（finish_reason：stop / tool_calls / length / …）。
    Finish { reason: Option<String> },
}

/// 单轮流式/非流式请求的聚合结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundOutput {
    /// 完整正文（增量拼接）。
    pub content: String,
    /// 聚合后的完整工具调用（arguments 分片拼接）。
    pub tool_calls: Vec<ToolCall>,
    /// finish_reason（stop / tool_calls / length / …）。
    pub finish_reason: Option<String>,
}

impl RoundOutput {
    /// 是否仅文本回复（循环据此判定对话结束）。
    pub fn is_plain_answer(&self) -> bool {
        self.tool_calls.is_empty()
    }
}
