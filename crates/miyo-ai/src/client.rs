//! OpenAI 兼容协议客户端：SSE 流式 + 工具调用分片聚合。

use std::collections::BTreeMap;

use futures::StreamExt;
use serde_json::{json, Value};

use crate::error::AiError;
use crate::types::{
    AiProviderConfig, ChatMessage, Role, RoundOutput, StreamEvent, ToolCall, ToolSpec,
};

/// 单次流式读取的网络兜底超时：相邻 chunk 间最大间隔（整流不定长，
/// 不能用总超时；慢提供方逐字吐字属于正常）。
const CHUNK_IDLE_TIMEOUT_SECS: u64 = 180;

/// AI 大模型客户端（OpenAI 兼容 `/chat/completions`）。
pub struct LlmClient {
    http: reqwest::Client,
    cfg: AiProviderConfig,
}

impl LlmClient {
    pub fn new(cfg: AiProviderConfig) -> Result<Self, AiError> {
        if !cfg.is_complete() {
            return Err(AiError::Config(
                "Base URL 与模型名不能为空，且 Base URL 须以 http(s):// 开头".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| AiError::Http(e.to_string()))?;
        Ok(LlmClient { http, cfg })
    }

    pub fn config(&self) -> &AiProviderConfig {
        &self.cfg
    }

    /// chat/completions 完整地址。
    fn endpoint(&self) -> String {
        let base = self.cfg.base_url.trim().trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    /// 组装请求体（内部表示 → OpenAI 线上格式）。
    fn build_body(&self, messages: &[ChatMessage], tools: &[ToolSpec], stream: bool) -> Value {
        let wire_messages: Vec<Value> = messages
            .iter()
            .map(|m| match m.role {
                Role::Assistant => {
                    let mut obj = json!({
                        "role": "assistant",
                        // 带工具调用的 assistant 消息：显式 content=null（部分兼容实现要求字段存在）。
                        "content": m.content.clone(),
                    });
                    if let Some(calls) = &m.tool_calls {
                        obj["tool_calls"] = json!(calls);
                    }
                    obj
                }
                Role::Tool => json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content.clone().unwrap_or_default(),
                }),
                Role::System | Role::User => json!({
                    "role": m.role,
                    "content": m.content.clone().unwrap_or_default(),
                }),
            })
            .collect();

        let mut body = json!({
            "model": self.cfg.model,
            "messages": wire_messages,
            "stream": stream,
        });
        if let Some(t) = self.cfg.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(mt) = self.cfg.max_tokens {
            body["max_tokens"] = json!(mt);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools
                .iter()
                .map(|t| json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                }))
                .collect::<Vec<_>>());
            body["tool_choice"] = json!("auto");
        }
        body
    }

    /// 非流式请求（兜底路径；主路径用 [`chat_stream`]）。
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<RoundOutput, AiError> {
        if self.cfg.api_key.trim().is_empty() {
            return Err(AiError::Config("未配置 API Key".into()));
        }
        let resp = self
            .http
            .post(self.endpoint())
            .bearer_auth(self.cfg.api_key.trim())
            .json(&self.build_body(messages, tools, false))
            .send()
            .await
            .map_err(|e| AiError::Http(e.to_string()))?;
        let resp = Self::check_status(resp).await?;
        let value: Value = resp
            .json()
            .await
            .map_err(|e| AiError::Parse(e.to_string()))?;
        let choice = value.get("choices").and_then(|c| c.get(0)).ok_or_else(|| {
            AiError::Parse(format!(
                "响应缺少 choices：{}",
                Self::truncate(&value.to_string(), 400)
            ))
        })?;
        Ok(Self::round_from_message(
            choice.get("message").cloned().unwrap_or(Value::Null),
            choice
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        ))
    }

    /// 流式请求：逐事件回调 `on_event`，返回聚合结果。
    pub async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        mut on_event: impl FnMut(StreamEvent),
    ) -> Result<RoundOutput, AiError> {
        if self.cfg.api_key.trim().is_empty() {
            return Err(AiError::Config("未配置 API Key".into()));
        }
        let resp = self
            .http
            .post(self.endpoint())
            .bearer_auth(self.cfg.api_key.trim())
            .json(&self.build_body(messages, tools, true))
            .send()
            .await
            .map_err(|e| AiError::Http(e.to_string()))?;
        let resp = Self::check_status(resp).await?;

        let mut agg = RoundAggregation::default();
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        loop {
            let chunk = tokio::time::timeout(
                std::time::Duration::from_secs(CHUNK_IDLE_TIMEOUT_SECS),
                stream.next(),
            )
            .await
            .map_err(|_| AiError::Timeout)?;
            let bytes = match chunk {
                Some(b) => b.map_err(|e| AiError::Http(e.to_string()))?,
                None => break, // 连接结束
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            // SSE 以空行分隔事件；逐行消费，保留最后一段不完整行。
            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                let Some(data) = line.strip_prefix("data:") else {
                    continue; // event:/id:/注释行等，当前协议用不到
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(agg.finish());
                }
                if data.is_empty() {
                    continue;
                }
                agg.absorb_chunk(data, &mut on_event)?;
            }
        }
        Ok(agg.finish())
    }

    /// 连通性验活：发一条 1 token 的最小对话。
    pub async fn test_connection(&self) -> Result<(), AiError> {
        let msg = ChatMessage::user("ping");
        // 非流式最小请求即可验活；个别提供方不支持 max_tokens=1，放宽为不限制。
        let mut cfg = self.cfg.clone();
        cfg.max_tokens = Some(16);
        LlmClient::new(cfg)?.chat(&[msg], &[]).await?;
        Ok(())
    }

    async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, AiError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(AiError::Api {
            status: status.as_u16(),
            body: Self::truncate(&body, 600),
        })
    }

    fn truncate(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            s.to_string()
        } else {
            let cut: String = s.chars().take(max).collect();
            format!("{cut}…")
        }
    }

    /// 非流式 message → RoundOutput。
    fn round_from_message(message: Value, finish_reason: Option<String>) -> RoundOutput {
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let mut tool_calls = Vec::new();
        if let Some(list) = message.get("tool_calls").and_then(|v| v.as_array()) {
            for (i, c) in list.iter().enumerate() {
                if let Ok(call) = serde_json::from_value::<ToolCall>(c.clone()) {
                    tool_calls.push(ToolCall {
                        id: if call.id.is_empty() {
                            format!("call_{i}")
                        } else {
                            call.id
                        },
                        ..call
                    });
                }
            }
        }
        RoundOutput {
            content,
            tool_calls,
            finish_reason,
        }
    }
}

/// 流式分片聚合器：content 增量拼接；tool_calls 按 index 聚合
/// （OpenAI 流式把一次调用拆成 id/name 首片 + arguments 多片）。
#[derive(Default)]
struct RoundAggregation {
    content: String,
    /// index → (id, name, arguments)。
    calls: BTreeMap<usize, (String, String, String)>,
    finish_reason: Option<String>,
    /// 顺序恢复：BTreeMap 有序，但 id 首片出现顺序与 index 不保证一致时
    /// 以首次出现序为准。
    order: Vec<usize>,
}

impl RoundAggregation {
    fn absorb_chunk(
        &mut self,
        data: &str,
        on_event: &mut impl FnMut(StreamEvent),
    ) -> Result<(), AiError> {
        let value: Value = serde_json::from_str(data)
            .map_err(|e| AiError::Parse(format!("SSE 分片非法 JSON：{e}")))?;
        let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
            return Ok(()); // usage 尾包 / 心跳等无 choices 的分片
        };
        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            self.finish_reason = Some(reason.to_string());
            on_event(StreamEvent::Finish {
                reason: Some(reason.to_string()),
            });
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(());
        };
        if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                self.content.push_str(text);
                on_event(StreamEvent::ContentDelta(text.to_string()));
            }
        }
        if let Some(list) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for piece in list {
                let index = piece.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let id = piece.get("id").and_then(|v| v.as_str()).map(str::to_string);
                let name = piece
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let args = piece
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !self.calls.contains_key(&index) {
                    self.order.push(index);
                    self.calls
                        .insert(index, (String::new(), String::new(), String::new()));
                }
                let entry = self.calls.get_mut(&index).expect("刚插入");
                if let Some(i) = &id {
                    entry.0.push_str(i);
                }
                if let Some(n) = &name {
                    entry.1.push_str(n);
                }
                entry.2.push_str(&args);
                if !args.is_empty() {
                    on_event(StreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta: args,
                    });
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> RoundOutput {
        let tool_calls = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(i, idx)| {
                let (id, name, arguments) = self.calls.get(idx)?;
                Some(ToolCall {
                    id: if id.is_empty() {
                        format!("call_{i}")
                    } else {
                        id.clone()
                    },
                    kind: "function".into(),
                    function: crate::types::ToolCallFunction {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                })
            })
            .collect();
        RoundOutput {
            content: self.content,
            tool_calls,
            finish_reason: self.finish_reason,
        }
    }
}
