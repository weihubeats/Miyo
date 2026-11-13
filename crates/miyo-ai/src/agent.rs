//! 应用内 Agent 循环：模型 ↔ 工具多轮往返，直到模型给出最终回答。
//!
//! 职责边界：本模块只管循环编排与消息追加；工具的真正执行由调用方注入
//! （[`ToolExecutor`]，应用内走 miyo-agent 控制面），副作用审批由调用方经
//! [`LoopObserver::approve_tool`] 决定（应用内为聊天内确认弹卡）。

use tokio_util::sync::CancellationToken;

use crate::client::LlmClient;
use crate::error::AiError;
use crate::types::{ChatMessage, ToolCall, ToolSpec};

/// 工具执行器：`arguments_json` 为模型给出的参数 JSON 字符串。
/// `Err(String)` 的错误文案会原样作为工具结果回传给模型（模型可据此自纠）。
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, name: &str, arguments_json: &str) -> Result<serde_json::Value, String>;
}

/// 循环观察者：驱动 UI（流式增量 / 工具状态 / 审批）。全部带默认实现，
/// 调用方按需覆写。
#[async_trait::async_trait]
pub trait LoopObserver: Send + Sync {
    /// 助手正文增量（流式）。
    fn on_content_delta(&self, _delta: &str) {}

    /// 工具调用开始（已通过审批）。
    fn on_tool_start(&self, _call: &ToolCall) {}

    /// 工具调用结束（`ok=false` 时 `result` 为 `{"error": …}`）。
    fn on_tool_result(&self, _call: &ToolCall, _ok: bool, _result: &serde_json::Value) {}

    /// 副作用审批：返回 `true` 放行、`false` 拒绝（拒绝会以明确文案回传模型，
    /// 模型将转而向用户说明或改用只读路径）。
    async fn approve_tool(&self, _call: &ToolCall) -> bool {
        true
    }
}

/// 循环结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStop {
    /// 模型给出最终回答（无工具调用的收尾轮）。
    Completed,
    /// 达到迭代上限仍未收尾（防失控；最后一条 assistant 消息已入列）。
    MaxIterations(usize),
    /// 被取消令牌中止。
    Cancelled,
}

/// 空实现观察者（测试 / 无 UI 场景）。
pub struct NoopObserver;
#[async_trait::async_trait]
impl LoopObserver for NoopObserver {}

/// 一次性拒绝所有需审批工具的观察者（测试用）。
pub struct DenyAllObserver;
#[async_trait::async_trait]
impl LoopObserver for DenyAllObserver {
    async fn approve_tool(&self, _call: &ToolCall) -> bool {
        false
    }
}

/// 循环入参（参数收敛，避免 8 参函数）。
pub struct AgentLoopParams<'a> {
    pub client: &'a LlmClient,
    pub tools: &'a [ToolSpec],
    /// 返回 true 的工具需要审批（应用内：registry 元数据 side_effects=true）。
    pub needs_approval: &'a (dyn Fn(&str) -> bool + Send + Sync),
    pub executor: &'a dyn ToolExecutor,
    pub observer: &'a dyn LoopObserver,
    pub cancel: CancellationToken,
    /// 迭代上限（防失控）。
    pub max_iterations: usize,
}

/// 工具调用循环。
///
/// 每轮流式请求 → 聚合出工具调用 → 逐个审批+执行 → 结果以 tool 消息回传，
/// 直到模型输出纯文本回答（[`LoopStop::Completed`]）或触达 `max_iterations`。
pub async fn run_agent_loop(
    messages: &mut Vec<ChatMessage>,
    p: AgentLoopParams<'_>,
) -> Result<LoopStop, AiError> {
    let AgentLoopParams {
        client,
        tools,
        needs_approval,
        executor,
        observer,
        cancel,
        max_iterations,
    } = p;
    for _ in 0..max_iterations {
        if cancel.is_cancelled() {
            return Ok(LoopStop::Cancelled);
        }
        let round = {
            client
                .chat_stream(messages, tools, |ev| {
                    if let crate::types::StreamEvent::ContentDelta(delta) = ev {
                        observer.on_content_delta(&delta);
                    }
                })
                .await?
        };
        // 非流式兜底（个别兼容网关 stream=true 仍回整包 JSON 时内容为空）。
        let round = if round.content.is_empty()
            && round.tool_calls.is_empty()
            && round.finish_reason.as_deref() != Some("stop")
        {
            tracing::debug!("[ai] 流式响应为空，回退非流式请求");
            client.chat(messages, tools).await?
        } else {
            round
        };

        if round.tool_calls.is_empty() {
            messages.push(ChatMessage::assistant(round.content));
            return Ok(LoopStop::Completed);
        }

        messages.push(ChatMessage::assistant_with_tool_calls(
            if round.content.is_empty() {
                None
            } else {
                Some(round.content)
            },
            round.tool_calls.clone(),
        ));

        for call in round.tool_calls {
            if cancel.is_cancelled() {
                return Ok(LoopStop::Cancelled);
            }
            let approved = if needs_approval(&call.function.name) {
                observer.approve_tool(&call).await
            } else {
                true
            };
            if !approved {
                let denied = serde_json::json!({
                    "denied": true,
                    "message": "用户在确认弹卡中拒绝了该操作。请勿重复发起同一调用；请向用户说明该步骤的影响，或改用只读工具继续分析。"
                });
                observer.on_tool_result(&call, false, &denied);
                messages.push(ChatMessage::tool_result(&call.id, denied.to_string()));
                continue;
            }
            observer.on_tool_start(&call);
            let (ok, value) = match executor
                .execute(&call.function.name, &call.function.arguments)
                .await
            {
                Ok(v) => (true, v),
                Err(e) => (false, serde_json::json!({ "error": e })),
            };
            observer.on_tool_result(&call, ok, &value);
            messages.push(ChatMessage::tool_result(&call.id, value.to_string()));
        }
    }
    Ok(LoopStop::MaxIterations(max_iterations))
}
