//! 集成测试：内嵌 axum mock LLM 服务验证 SSE 聚合、工具循环与审批暂停。

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::{routing::post, Json, Router};
use miyo_ai::{
    AiError, AiProviderConfig, ChatMessage, LlmClient, LoopObserver, LoopStop, NoopObserver, Role,
    ToolCall, ToolExecutor, ToolSpec,
};
use tokio_util::sync::CancellationToken;

/// mock 服务：按请求顺序弹出预设响应。
#[derive(Default)]
struct MockState {
    requests: Mutex<Vec<serde_json::Value>>,
    /// 每项：{"sse": ["data: {…}", …]} 流式响应；或 {"json": {…}} 非流式响应。
    /// 队列语义：先写入的响应先被取走。
    responses: Mutex<std::collections::VecDeque<serde_json::Value>>,
}

async fn handler(
    axum::extract::State(state): axum::extract::State<Arc<MockState>>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    state.requests.lock().unwrap().push(body);
    let next = state.responses.lock().unwrap().pop_front();
    use axum::response::IntoResponse;
    match next {
        Some(v) if v.get("sse").is_some() => {
            let lines = v["sse"].as_array().unwrap();
            let mut body = String::new();
            for line in lines {
                body.push_str(line.as_str().unwrap());
                body.push_str("\n\n");
            }
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                body,
            )
                .into_response()
        }
        Some(v) if v.get("json").is_some() => {
            (StatusCode::OK, Json(v["json"].clone())).into_response()
        }
        Some(v) => (StatusCode::INTERNAL_SERVER_ERROR, v.to_string()).into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "no scripted response").into_response(),
    }
}

async fn spawn_mock(scripted: Vec<serde_json::Value>) -> (String, Arc<MockState>) {
    let state = Arc::new(MockState {
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(scripted.into_iter().collect()),
    });
    let app = Router::new()
        .route("/v1/chat/completions", post(handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1"), state)
}

fn sse(chunks: &[serde_json::Value]) -> serde_json::Value {
    let mut lines: Vec<serde_json::Value> = chunks
        .iter()
        .map(|c| serde_json::json!(format!("data: {c}")))
        .collect();
    lines.push(serde_json::json!("data: [DONE]"));
    serde_json::json!({ "sse": lines })
}

fn content_chunk(text: &str) -> serde_json::Value {
    serde_json::json!({ "choices": [{ "delta": { "content": text } }] })
}

fn call_chunk(index: u64, id: Option<&str>, name: Option<&str>, args: &str) -> serde_json::Value {
    let mut function = serde_json::json!({ "arguments": args });
    if let Some(n) = name {
        function["name"] = serde_json::json!(n);
    }
    let mut delta = serde_json::json!({ "tool_calls": [{ "index": index, "function": function }] });
    if let Some(i) = id {
        delta["tool_calls"][0]["id"] = serde_json::json!(i);
        delta["tool_calls"][0]["type"] = serde_json::json!("function");
    }
    serde_json::json!({ "choices": [{ "delta": delta }] })
}

fn finish_chunk(reason: &str) -> serde_json::Value {
    serde_json::json!({ "choices": [{ "delta": {}, "finish_reason": reason }] })
}

fn client(base_url: &str) -> LlmClient {
    LlmClient::new(AiProviderConfig {
        base_url: base_url.into(),
        api_key: "sk-test".into(),
        model: "mock-model".into(),
        temperature: None,
        max_tokens: None,
    })
    .unwrap()
}

#[tokio::test]
async fn stream_aggregates_content_and_tool_calls() {
    let (url, _state) = spawn_mock(vec![sse(&[
        content_chunk("我来查看"),
        content_chunk("接口列表。"),
        call_chunk(0, Some("call_a"), Some("list_endpoints"), ""),
        call_chunk(0, None, None, "{\"projectId\":"),
        call_chunk(0, None, None, "\"p-1\"}"),
        call_chunk(
            1,
            Some("call_b"),
            Some("get_endpoint"),
            "{\"endpointId\":\"e-9\"}",
        ),
        finish_chunk("tool_calls"),
    ])])
    .await;

    let mut deltas = Vec::new();
    let out = client(&url)
        .chat_stream(&[ChatMessage::user("hi")], &[], |ev| {
            if let miyo_ai::StreamEvent::ContentDelta(d) = ev {
                deltas.push(d)
            }
        })
        .await
        .unwrap();

    assert_eq!(out.content, "我来查看接口列表。");
    assert_eq!(deltas.join(""), "我来查看接口列表。");
    assert_eq!(out.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(out.tool_calls.len(), 2);
    assert_eq!(out.tool_calls[0].id, "call_a");
    assert_eq!(out.tool_calls[0].function.name, "list_endpoints");
    assert_eq!(
        out.tool_calls[0].function.arguments,
        "{\"projectId\":\"p-1\"}"
    );
    assert_eq!(
        out.tool_calls[1].function.arguments,
        "{\"endpointId\":\"e-9\"}"
    );
    assert!(!out.is_plain_answer());
}

#[tokio::test]
async fn non_stream_response_maps_message() {
    let (url, _state) = spawn_mock(vec![serde_json::json!({ "json": {
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "最终回答",
                "tool_calls": [{
                    "id": "call_x", "type": "function",
                    "function": { "name": "noop", "arguments": "{}" }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    } })])
    .await;

    let out = client(&url)
        .chat(&[ChatMessage::user("hi")], &[])
        .await
        .unwrap();
    assert_eq!(out.content, "最终回答");
    assert_eq!(out.tool_calls.len(), 1);
}

/// 记录回调的观察者（可选拒绝指定工具）。
struct RecordingObserver {
    started: Mutex<Vec<String>>,
    results: Mutex<Vec<(String, bool)>>,
    deny: Option<String>,
}

impl RecordingObserver {
    fn new(deny: Option<&str>) -> Self {
        RecordingObserver {
            started: Mutex::new(Vec::new()),
            results: Mutex::new(Vec::new()),
            deny: deny.map(str::to_string),
        }
    }
}

#[async_trait::async_trait]
impl LoopObserver for RecordingObserver {
    fn on_tool_start(&self, call: &ToolCall) {
        self.started
            .lock()
            .unwrap()
            .push(call.function.name.clone());
    }
    fn on_tool_result(&self, call: &ToolCall, ok: bool, _result: &serde_json::Value) {
        self.results.lock().unwrap().push((call.id.clone(), ok));
    }
    async fn approve_tool(&self, call: &ToolCall) -> bool {
        self.deny.as_deref() != Some(call.function.name.as_str())
    }
}

struct EchoExecutor;
#[async_trait::async_trait]
impl ToolExecutor for EchoExecutor {
    async fn execute(&self, name: &str, arguments_json: &str) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "tool": name, "echo": arguments_json }))
    }
}

#[tokio::test]
async fn agent_loop_runs_tool_rounds_then_answers() {
    let (url, state) = spawn_mock(vec![
        // 第 1 轮：发起工具调用。
        sse(&[
            call_chunk(
                0,
                Some("call_1"),
                Some("list_endpoints"),
                "{\"projectId\":\"p-1\"}",
            ),
            finish_chunk("tool_calls"),
        ]),
        // 第 2 轮：纯文本回答。
        sse(&[content_chunk("分析完成。"), finish_chunk("stop")]),
    ])
    .await;

    let tools = vec![ToolSpec::new(
        "list_endpoints",
        "列出接口",
        serde_json::json!({}),
    )];
    let mut messages = vec![
        ChatMessage::system("你是测试助手"),
        ChatMessage::user("跑一下"),
    ];
    let observer = RecordingObserver::new(None);
    let stop = miyo_ai::run_agent_loop(
        &mut messages,
        miyo_ai::AgentLoopParams {
            client: &client(&url),
            tools: &tools,
            needs_approval: &|_| false,
            executor: &EchoExecutor,
            observer: &observer,
            cancel: CancellationToken::new(),
            max_iterations: 5,
        },
    )
    .await
    .unwrap();

    assert_eq!(stop, LoopStop::Completed);
    // 请求了 2 轮。
    assert_eq!(state.requests.lock().unwrap().len(), 2);
    // 消息序列：system / user / assistant(工具调用) / tool / assistant(回答)。
    let roles: Vec<Role> = messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            Role::System,
            Role::User,
            Role::Assistant,
            Role::Tool,
            Role::Assistant
        ]
    );
    assert_eq!(messages[2].tool_calls.as_ref().unwrap()[0].id, "call_1");
    assert!(messages[3].tool_call_id.as_deref() == Some("call_1"));
    assert!(messages[3]
        .content
        .as_deref()
        .unwrap()
        .contains("list_endpoints"));
    assert_eq!(messages[4].content.as_deref(), Some("分析完成。"));
    assert_eq!(
        observer.results.lock().unwrap().clone(),
        vec![("call_1".into(), true)]
    );
}

#[tokio::test]
async fn agent_loop_denied_tool_returns_denied_result() {
    let (url, _state) = spawn_mock(vec![
        // 第 1 轮：发起副作用工具。
        sse(&[
            call_chunk(0, Some("call_d"), Some("run_scenario"), "{}"),
            finish_chunk("tool_calls"),
        ]),
        // 第 2 轮：模型收到拒绝后收尾。
        sse(&[content_chunk("好的，已跳过。"), finish_chunk("stop")]),
    ])
    .await;

    let mut messages = vec![ChatMessage::user("run")];
    let observer = RecordingObserver::new(Some("run_scenario"));
    let stop = miyo_ai::run_agent_loop(
        &mut messages,
        miyo_ai::AgentLoopParams {
            client: &client(&url),
            tools: &[],
            needs_approval: &|name| name == "run_scenario",
            executor: &EchoExecutor,
            observer: &observer,
            cancel: CancellationToken::new(),
            max_iterations: 5,
        },
    )
    .await
    .unwrap();

    assert_eq!(stop, LoopStop::Completed);
    let tool_msg = &messages[2];
    assert!(tool_msg
        .content
        .as_deref()
        .unwrap()
        .contains("\"denied\":true"));
    assert!(!observer
        .started
        .lock()
        .unwrap()
        .iter()
        .any(|n| n == "run_scenario"));
    assert_eq!(
        observer.results.lock().unwrap().clone(),
        vec![("call_d".into(), false)]
    );
}

#[tokio::test]
async fn agent_loop_executor_error_feeds_back_to_model() {
    struct FailExecutor;
    #[async_trait::async_trait]
    impl ToolExecutor for FailExecutor {
        async fn execute(&self, _name: &str, _args: &str) -> Result<serde_json::Value, String> {
            Err("工具炸了".into())
        }
    }

    let (url, _state) = spawn_mock(vec![
        sse(&[
            call_chunk(0, Some("call_e"), Some("boom"), "{}"),
            finish_chunk("tool_calls"),
        ]),
        sse(&[content_chunk("收到错误，已停止。"), finish_chunk("stop")]),
    ])
    .await;

    let mut messages = vec![ChatMessage::user("go")];
    let stop = miyo_ai::run_agent_loop(
        &mut messages,
        miyo_ai::AgentLoopParams {
            client: &client(&url),
            tools: &[],
            needs_approval: &|_| false,
            executor: &FailExecutor,
            observer: &NoopObserver,
            cancel: CancellationToken::new(),
            max_iterations: 5,
        },
    )
    .await
    .unwrap();
    assert_eq!(stop, LoopStop::Completed);
    assert!(messages[2].content.as_deref().unwrap().contains("工具炸了"));
}

#[tokio::test]
async fn agent_loop_respects_max_iterations() {
    // 每轮都发起工具调用，永不收尾。
    let scripted: Vec<serde_json::Value> = (0..8)
        .map(|_| {
            sse(&[
                call_chunk(0, Some("call_loop"), Some("noop"), "{}"),
                finish_chunk("tool_calls"),
            ])
        })
        .collect();
    let (url, _state) = spawn_mock(scripted).await;

    let mut messages = vec![ChatMessage::user("loop")];
    let stop = miyo_ai::run_agent_loop(
        &mut messages,
        miyo_ai::AgentLoopParams {
            client: &client(&url),
            tools: &[],
            needs_approval: &|_| false,
            executor: &EchoExecutor,
            observer: &NoopObserver,
            cancel: CancellationToken::new(),
            max_iterations: 3,
        },
    )
    .await
    .unwrap();
    assert_eq!(stop, LoopStop::MaxIterations(3));
}

#[tokio::test]
async fn api_error_maps_status_and_body() {
    let (url, _state) = spawn_mock(vec![serde_json::json!("boom")]).await;
    let err = client(&url)
        .chat(&[ChatMessage::user("hi")], &[])
        .await
        .unwrap_err();
    assert!(matches!(err, AiError::Api { status: 500, .. }));
}

#[tokio::test]
async fn empty_stream_falls_back_to_non_stream() {
    // 流式无任何有效分片（兼容网关伪流式把整包 JSON 当普通文本返回）→ 循环自动回退非流式。
    let (url, state) = spawn_mock(vec![
        sse(&[]),
        serde_json::json!({ "json": { "choices": [{ "message": { "role": "assistant", "content": "非流式回答" }, "finish_reason": "stop" }] } }),
    ])
    .await;

    let mut messages = vec![ChatMessage::user("hi")];
    let stop = miyo_ai::run_agent_loop(
        &mut messages,
        miyo_ai::AgentLoopParams {
            client: &client(&url),
            tools: &[],
            needs_approval: &|_| false,
            executor: &EchoExecutor,
            observer: &NoopObserver,
            cancel: CancellationToken::new(),
            max_iterations: 3,
        },
    )
    .await
    .unwrap();
    assert_eq!(stop, LoopStop::Completed);
    assert_eq!(state.requests.lock().unwrap().len(), 2);
    assert_eq!(messages[1].content.as_deref(), Some("非流式回答"));
}
