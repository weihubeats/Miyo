//! 实时调试 Command：WebSocket 连接管理 + SSE 订阅。
//!
//! 后端能力（`miyo-http::ws_client`）早已完备（自动重连/心跳/离线补发），
//! 但一直没有命令层暴露，前端无法使用。本模块补上：
//!
//! - WebSocket：`ws_connect` 建连（后台任务转发达事件）→ `ws_send` 发帧 →
//!   `ws_disconnect` 关闭；事件经 `miyo:ws-event` 推送；
//! - SSE：`sse_connect` 订阅（原始文本块转发，前端按帧解析）→
//!   `sse_disconnect` 中止；事件经 `miyo:sse-event` 推送。
//!
//! 中止统一走 `AppState::run_cancels` 注册表（与压测/集合测试共用语义）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::task::JoinHandle;

use miyo_http::ws_client::{WsClient, WsEvent, WsMessage, WsOptions};

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 一个 WS 会话：可克隆的客户端 + 事件转发任务句柄。
pub struct WsSession {
    pub client: WsClient,
    pub forward: JoinHandle<()>,
}

/// WS 入参。
#[derive(Debug, Clone, Deserialize)]
pub struct WsConnectArgs {
    /// 连接标识（前端生成；缺省由后端生成）。
    #[serde(default)]
    pub connection_id: Option<String>,
    /// `ws://` / `wss://` 地址（`{{变量}}` 由前端预渲染或后端按激活环境渲染？——
    /// 此处直接使用透传地址，变量渲染由前端在调用前完成，与 cURL 导入一致）。
    pub url: String,
    /// 自定义请求头。
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// 请求的子协议列表。
    #[serde(default)]
    pub subprotocols: Vec<String>,
    /// 断线自动重连（默认 true）。
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
}

fn default_true() -> bool {
    true
}

/// 推送给前端的 WS 事件（`miyo:ws-event` 载荷）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WsEventPayload {
    /// 连接状态变化（connecting/open/closed/error）。
    State {
        connection_id: String,
        state: String,
    },
    /// 收到的服務端消息（binary/ping 内容为 base64）。
    Message {
        connection_id: String,
        direction: &'static str,
        frame: &'static str,
        text: String,
    },
    /// 连接失败或异常断开。
    Failed {
        connection_id: String,
        message: String,
    },
}

/// 建立 WebSocket 连接：立即返回 id，后续事件经 `miyo:ws-event` 推送。
#[tauri::command(rename_all = "camelCase")]
pub async fn ws_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    args: WsConnectArgs,
) -> CommandResult<String> {
    if args.url.trim().is_empty() {
        return Err(CommandError::validation("WebSocket 地址不能为空"));
    }
    let id = args
        .connection_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if state.ws.read().await.contains_key(&id) {
        return Err(CommandError::validation("该连接已存在，请先断开"));
    }
    let options = WsOptions {
        auto_reconnect: args.auto_reconnect,
        ..WsOptions::default()
    };
    let client =
        WsClient::connect_with_options(args.url.clone(), args.headers, args.subprotocols, options)
            .await
            .map_err(CommandError::from)?;

    // 连接成功后以写锁复查：并发同 id 连接时后到者让位（丢弃新建连接）。
    // 检查与插入之间跨越了网络 await，单次快路径检查不足以关闭竞态窗口。
    let mut ws = state.ws.write().await;
    if ws.contains_key(&id) {
        return Err(CommandError::validation("该连接已存在，请先断开"));
    }

    // 事件转发：广播订阅 → Tauri 事件（任务随会话持有，断开时 abort）。
    let mut rx = client.subscribe();
    let app_handle = app.clone();
    let forward_id = id.clone();
    let forward = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let payload = match event {
                WsEvent::State(s) => WsEventPayload::State {
                    connection_id: forward_id.clone(),
                    state: format!("{s:?}").to_lowercase(),
                },
                WsEvent::Message(WsMessage::Text(t)) => WsEventPayload::Message {
                    connection_id: forward_id.clone(),
                    direction: "in",
                    frame: "text",
                    text: t,
                },
                WsEvent::Message(WsMessage::Binary(b)) => WsEventPayload::Message {
                    connection_id: forward_id.clone(),
                    direction: "in",
                    frame: "binary",
                    text: base64_text(&b),
                },
                WsEvent::Message(WsMessage::Ping(b)) => WsEventPayload::Message {
                    connection_id: forward_id.clone(),
                    direction: "in",
                    frame: "ping",
                    text: base64_text(&b),
                },
                WsEvent::Failed(msg) => WsEventPayload::Failed {
                    connection_id: forward_id.clone(),
                    message: msg,
                },
            };
            if app_handle.emit("miyo:ws-event", &payload).is_err() {
                break;
            }
        }
    });

    ws.insert(id.clone(), WsSession { client, forward });
    drop(ws);
    Ok(id)
}

fn base64_text(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// 发送帧入参（binary/ping 的 payload 为 base64）。
#[derive(Debug, Clone, Deserialize)]
pub struct WsSendArgs {
    pub connection_id: String,
    /// text | binary | ping。
    pub frame: String,
    /// 文本帧原文；binary/ping 为 base64。
    pub payload: String,
}

/// 发送 WebSocket 帧（连接未就绪时进内部缓冲，重连后自动补发）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ws_send(state: State<'_, AppState>, args: WsSendArgs) -> CommandResult<()> {
    // 先克隆 Arc 句柄再释放读锁：send_message 内部含 await（溢出落库等），
    // 持锁跨 await 会阻塞 ws_disconnect 的写锁与并发发送。
    let client = {
        let guard = state.ws.read().await;
        guard
            .get(&args.connection_id)
            .ok_or_else(|| CommandError::validation("连接不存在或已断开"))?
            .client
            .clone()
    };
    let message = match args.frame.as_str() {
        "text" => WsMessage::Text(args.payload),
        "binary" => WsMessage::Binary(decode_b64(&args.payload)?),
        "ping" => WsMessage::Ping(decode_b64(&args.payload)?),
        other => {
            return Err(CommandError::validation(format!(
                "不支持的帧类型：{other}（仅 text/binary/ping）"
            )))
        }
    };
    client
        .send_message(message)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

fn decode_b64(payload: &str) -> CommandResult<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| CommandError::validation(format!("base64 解码失败：{e}")))
}

/// 断开连接（不存在时返回 false；优雅停止 + 中止转发任务）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ws_disconnect(
    state: State<'_, AppState>,
    connection_id: String,
) -> CommandResult<bool> {
    let session = state.ws.write().await.remove(&connection_id);
    let Some(session) = session else {
        return Ok(false);
    };
    session.forward.abort();
    session.client.stop().await.map_err(CommandError::from)?;
    Ok(true)
}

// ---------- SSE ----------

/// SSE 订阅入参。
#[derive(Debug, Clone, Deserialize)]
pub struct SseConnectArgs {
    #[serde(default)]
    pub connection_id: Option<String>,
    /// `http(s)://` 地址。
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// 断线重连时续传（`Last-Event-ID`）。
    #[serde(default)]
    pub last_event_id: Option<String>,
}

/// SSE 转发载荷（`miyo:sse-event`；`chunk` 为原始文本块，前端按帧解析）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SseEventPayload {
    Open {
        connection_id: String,
    },
    Chunk {
        connection_id: String,
        chunk: String,
    },
    Error {
        connection_id: String,
        message: String,
    },
    Closed {
        connection_id: String,
    },
}

/// 订阅 SSE：以后台任务拉流并转发原始文本块；断开用 `sse_disconnect`。
#[tauri::command(rename_all = "camelCase")]
pub async fn sse_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    args: SseConnectArgs,
) -> CommandResult<String> {
    if args.url.trim().is_empty() {
        return Err(CommandError::validation("SSE 地址不能为空"));
    }
    let parsed = url::Url::parse(&args.url)
        .map_err(|e| CommandError::validation(format!("URL 无效：{e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CommandError::validation(format!(
                "不支持的协议：{other}（仅支持 http/https）"
            )))
        }
    }
    let id = args
        .connection_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // 校验 / 占位 / 建客户端 / 启动转发任务在同一把写锁内完成，
    // 关闭 contains_key 与 insert 之间的 check-then-act 竞态窗口。
    let mut sse = state.sse.write().await;
    if sse.contains_key(&id) {
        return Err(CommandError::validation("该订阅已存在，请先断开"));
    }

    let client = reqwest::Client::builder()
        // SSE 是永不结束的长连接：只设连接超时。总超时（timeout）会把每条
        // 订阅在超时点无条件掐断（此前 300s 总超时导致 5 分钟必断）。
        .connect_timeout(std::time::Duration::from_secs(15))
        .no_proxy()
        .build()
        .map_err(|e| {
            CommandError::with_code(CommandErrorCode::Io, format!("HTTP 客户端初始化失败：{e}"))
        })?;
    let mut req = client
        .get(&args.url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache");
    for (k, v) in &args.headers {
        if !k.trim().is_empty() {
            req = req.header(k, v);
        }
    }
    if let Some(last) = args.last_event_id.filter(|s| !s.trim().is_empty()) {
        req = req.header("Last-Event-ID", last);
    }

    let forward_id = id.clone();
    let app_handle = app.clone();
    let task = tokio::spawn(async move {
        let emit = |payload: SseEventPayload| {
            let _ = app_handle.emit("miyo:sse-event", &payload);
        };
        async {
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    emit(SseEventPayload::Error {
                        connection_id: forward_id.clone(),
                        message: format!("SSE 连接失败：{e}"),
                    });
                    return;
                }
            };
            if !resp.status().is_success() {
                emit(SseEventPayload::Error {
                    connection_id: forward_id.clone(),
                    message: format!("SSE 服务端返回 HTTP {}", resp.status()),
                });
                return;
            }
            emit(SseEventPayload::Open {
                connection_id: forward_id.clone(),
            });
            use futures::StreamExt;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        if bytes.is_empty() {
                            continue;
                        }
                        emit(SseEventPayload::Chunk {
                            connection_id: forward_id.clone(),
                            chunk: String::from_utf8_lossy(&bytes).into_owned(),
                        });
                    }
                    Err(e) => {
                        emit(SseEventPayload::Error {
                            connection_id: forward_id.clone(),
                            message: format!("SSE 读取中断：{e}"),
                        });
                        break;
                    }
                }
            }
            emit(SseEventPayload::Closed {
                connection_id: forward_id.clone(),
            });
        }
        .await;
        // 任务自然结束（连接失败 / 服务端关流 / 读中断）统一清理自身条目：
        // 防止句柄泄漏与同 id 重连被拒；abort 路径由 sse_disconnect 清理。
        app_handle
            .state::<AppState>()
            .sse
            .write()
            .await
            .remove(&forward_id);
    });

    sse.insert(id.clone(), task);
    drop(sse);
    Ok(id)
}

/// 取消 SSE 订阅（不存在时返回 false）。
#[tauri::command(rename_all = "camelCase")]
pub async fn sse_disconnect(
    state: State<'_, AppState>,
    connection_id: String,
) -> CommandResult<bool> {
    let task = state.sse.write().await.remove(&connection_id);
    if let Some(task) = task {
        task.abort();
        Ok(true)
    } else {
        Ok(false)
    }
}
