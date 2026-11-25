//! Agent IPC 客户端（TCP 127.0.0.1 + 长度前缀 JSON 协议 v2）。
//!
//! 连接模型：每会话一条持久连接；请求带自增 id，响应按 id 关联——
//! invoke 在 Agent 侧 worker 池执行，期间 cancel / status 可并发发出。
//! 断线后按需重连（token 不变；Agent 端口由会话缓存）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};

use miyo_core::AppError;

use crate::AGENT_PROTOCOL_VERSION;

/// 单帧上限（与 Agent 一致 16 MiB）。
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;
/// 默认请求超时。
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// 连接任务命令。
enum ConnCmd {
    Request {
        id: String,
        op: String,
        params: Value,
        reply: oneshot::Sender<Result<Value, AgentClientError>>,
    },
}

/// Agent IPC 客户端（可克隆共享连接任务）。
#[derive(Clone)]
pub struct AgentClient {
    port: u16,
    token: Arc<String>,
    inner: Arc<Mutex<Option<ConnectionHandle>>>,
    request_seq: Arc<AtomicU64>,
}

struct ConnectionHandle {
    tx: mpsc::Sender<ConnCmd>,
    /// 连接任务句柄（disconnect 时 abort）。
    task: Arc<tokio::task::JoinHandle<()>>,
}

impl AgentClient {
    /// 构造（不立即连接；首个请求时建立）。
    pub fn new(port: u16, token: String) -> Self {
        AgentClient {
            port,
            token: Arc::new(token),
            inner: Arc::new(Mutex::new(None)),
            request_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 发送请求并等待响应（timeout_ms；断线自动重连一次）。
    pub async fn request(
        &self,
        op: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, AgentClientError> {
        // 单次尝试
        match self.request_once(op, params.clone(), timeout_ms).await {
            Err(err) if err.is_connection_level() => {
                // 连接级失败：强制重连一次（Agent 可能刚重启 / Miyo 休眠唤醒）
                self.disconnect().await;
                self.request_once(op, params, timeout_ms).await
            }
            other => other,
        }
    }

    async fn request_once(
        &self,
        op: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, AgentClientError> {
        let handle = self.ensure_connection().await?;
        let id = format!("r-{}", self.request_seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        handle
            .tx
            .send(ConnCmd::Request {
                id,
                op: op.to_string(),
                params,
                reply: tx,
            })
            .await
            .map_err(|_| AgentClientError::Connection("连接任务已退出（Agent 断开）".into()))?;
        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AgentClientError::Connection("连接任务已退出".into())),
            Err(_) => Err(AgentClientError::Timeout(format!(
                "请求 {op} 超时（{timeout_ms}ms）"
            ))),
        }
    }

    async fn ensure_connection(&self) -> Result<ConnectionHandle, AgentClientError> {
        let mut guard = self.inner.lock().await;
        if let Some(handle) = guard.as_ref() {
            if !handle.task.is_finished() {
                return Ok(ConnectionHandle {
                    tx: handle.tx.clone(),
                    task: handle.task.clone(),
                });
            }
            *guard = None;
        }
        // 建连 + 握手（握手失败即断，不进入连接任务循环）
        let stream = TcpStream::connect(("127.0.0.1", self.port))
            .await
            .map_err(|e| {
                AgentClientError::Connection(format!(
                    "连接 Agent 失败（127.0.0.1:{}）：{e}",
                    self.port
                ))
            })?;
        let (handshake_task, handshake_stream) = handshake(stream, &self.token).await?;
        let (tx, rx) = mpsc::channel::<ConnCmd>(64);
        let task = tokio::spawn(connection_loop(handshake_stream, handshake_task, rx));
        let task = Arc::new(task);
        *guard = Some(ConnectionHandle {
            tx: tx.clone(),
            task: task.clone(),
        });
        Ok(ConnectionHandle { tx, task })
    }

    /// 主动断开（detach：关闭 TCP；Agent 服务继续运行）。
    pub async fn disconnect(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(handle) = guard.take() {
            handle.task.abort();
        }
    }

    // ---- 协议便捷方法 ----

    /// 握手信息（每个连接建立时已校验；此处仅回传缓存于响应）。
    pub async fn ping(&self) -> Result<Value, AgentClientError> {
        self.request("ping", serde_json::json!({}), 5_000).await
    }

    pub async fn status(&self) -> Result<Value, AgentClientError> {
        self.request("status", serde_json::json!({}), 10_000).await
    }

    /// 类清单（分类 / 包前缀 / 搜索 / 分页）。
    ///
    /// 参数对象经 `ClassesQuery::to_params` 序列化：全部字段带默认值，
    /// 老调用方（只传 include_jdk）语义不变。
    pub async fn classes(
        &self,
        query: &crate::backend::ClassesQuery,
    ) -> Result<Value, AgentClientError> {
        self.request("classes.list", query.to_params(), DEFAULT_TIMEOUT_MS)
            .await
    }

    /// 类分类统计（不重跑全量扫描：agent 侧走索引缓存）。
    pub async fn classes_stats(
        &self,
        business_packages: Option<&str>,
    ) -> Result<Value, AgentClientError> {
        self.request(
            "classes.stats",
            serde_json::json!({ "package_prefix": business_packages.unwrap_or("") }),
            DEFAULT_TIMEOUT_MS,
        )
        .await
    }

    /// 全量重建类索引（用户改业务包 / 点「重新扫描」）。
    pub async fn classes_reindex(
        &self,
        business_packages: Option<&str>,
    ) -> Result<Value, AgentClientError> {
        self.request(
            "classes.reindex",
            serde_json::json!({ "package_prefix": business_packages.unwrap_or("") }),
            // 全量重建是 O(类数) 的重操作：给 60s 余量
            60_000,
        )
        .await
    }

    pub async fn class_detail(&self, class: &str) -> Result<Value, AgentClientError> {
        self.request(
            "class.detail",
            serde_json::json!({ "class": class }),
            DEFAULT_TIMEOUT_MS,
        )
        .await
    }

    pub async fn methods(&self, class: &str) -> Result<Value, AgentClientError> {
        self.request(
            "methods.list",
            serde_json::json!({ "class": class }),
            DEFAULT_TIMEOUT_MS,
        )
        .await
    }

    pub async fn spring_status(&self) -> Result<Value, AgentClientError> {
        self.request("beans.contexts", serde_json::json!({}), DEFAULT_TIMEOUT_MS)
            .await
    }

    pub async fn beans(
        &self,
        search: &str,
        context_id: &str,
        limit: u32,
    ) -> Result<Value, AgentClientError> {
        self.request(
            "beans.list",
            serde_json::json!({
                "search": search,
                "context_id": context_id,
                "limit": limit,
            }),
            DEFAULT_TIMEOUT_MS,
        )
        .await
    }

    /// 方法调用（长超时由调用方给定；响应含 v1 形状的结果）。
    pub async fn invoke(&self, params: Value, timeout_ms: u64) -> Result<Value, AgentClientError> {
        // 网络/IPC 层余量 = 调用超时 + 10s
        self.request("invoke", params, timeout_ms + 10_000).await
    }

    pub async fn cancel(&self, invocation_id: &str) -> Result<Value, AgentClientError> {
        self.request(
            "cancel",
            serde_json::json!({ "invocation_id": invocation_id }),
            10_000,
        )
        .await
    }

    /// Agent 停机（卸载语义：服务停止 + infoFile 清理；类不可卸载——诚实边界）。
    pub async fn stop_agent(&self) -> Result<Value, AgentClientError> {
        self.request("agent.stop", serde_json::json!({}), 10_000)
            .await
    }
}

/// 连接任务的读侧协议（id → oneshot）。
struct PendingMap {
    pending: HashMap<String, oneshot::Sender<Result<Value, AgentClientError>>>,
}

/// 握手：发送 handshake 请求，校验响应 / 协议版本。返回连接任务用的
/// pending map（握手响应的 id 不在 map——独立通道）。
async fn handshake(
    mut stream: TcpStream,
    token: &str,
) -> Result<(PendingMap, TcpStream), AgentClientError> {
    let payload = serde_json::json!({
        "id": "handshake",
        "op": "handshake",
        "params": {
            "token": token,
            "client_protocol": AGENT_PROTOCOL_VERSION,
        },
    });
    write_frame(&mut stream, &payload).await.map_err(io_err)?;
    stream.set_nodelay(true).ok();
    let response = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream))
        .await
        .map_err(|_| AgentClientError::Timeout("握手超时（10s）".into()))??;
    if !response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let code = response
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or("unknown");
        let message = response
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("（无详情）");
        let err = if code == "not_authorized" {
            AgentClientError::Auth(format!("Agent 鉴权失败：{message}"))
        } else if code == "protocol_mismatch" {
            AgentClientError::Protocol(format!("协议版本不匹配：{message}"))
        } else {
            AgentClientError::Connection(format!("握手失败：{message}"))
        };
        return Err(err);
    }
    Ok((
        PendingMap {
            pending: HashMap::new(),
        },
        stream,
    ))
}

/// 连接主循环：读帧分发 + 写请求。连接级错误 → 标记 pending 全部失败 → 退出
///（后续请求触发 ensure_connection 重连）。
async fn connection_loop(
    mut stream: TcpStream,
    mut pending: PendingMap,
    mut rx: mpsc::Receiver<ConnCmd>,
) {
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return };
                let ConnCmd::Request { id, op, params, reply } = cmd;
                let payload = serde_json::json!({
                    "id": id,
                    "op": op,
                    "params": params,
                });
                if let Err(e) = write_frame(&mut stream, &payload).await {
                    let _ = reply.send(Err(AgentClientError::Connection(format!(
                        "请求写入失败：{e}"
                    ))));
                    return; // 连接已坏
                }
                pending.pending.insert(id, reply);
            }
            frame = read_frame(&mut stream) => {
                match frame {
                    Ok(value) => {
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if let Some(reply) = pending.pending.remove(&id) {
                            let result = if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                                Ok(value.get("data").cloned().unwrap_or(Value::Null))
                            } else {
                                let error = value.get("error").cloned().unwrap_or(Value::Null);
                                let code = error.get("code").and_then(|c| c.as_str()).unwrap_or("unknown").to_string();
                                let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("（无详情）").to_string();
                                Err(AgentClientError::Agent { code, message })
                            };
                            let _ = reply.send(result);
                        }
                        // 未知 id 的响应：丢弃（陈旧重连残留）
                    }
                    Err(e) => {
                        // 连接级错误：所有 pending 失败后退出
                        for (_, reply) in pending.pending.drain() {
                            let _ = reply.send(Err(e.clone()));
                        }
                        return;
                    }
                }
            }
        }
    }
}

pub(crate) async fn write_frame(stream: &mut TcpStream, payload: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(payload)?;
    let len = body.len() as u32;
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "帧超限",
        ));
    }
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await
}

pub(crate) async fn read_frame(stream: &mut TcpStream) -> Result<Value, AgentClientError> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.map_err(io_err)?;
    let len = u32::from_be_bytes(header);
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(AgentClientError::Connection(format!("帧长度非法：{len}")));
    }
    let mut body = vec![0u8; len as usize];
    stream.read_exact(&mut body).await.map_err(io_err)?;
    serde_json::from_slice(&body)
        .map_err(|e| AgentClientError::Connection(format!("响应不是合法 JSON：{e}")))
}

fn io_err(e: std::io::Error) -> AgentClientError {
    AgentClientError::Connection(format!("IPC 连接错误：{e}"))
}

/// 客户端错误（连接级错误触发重连；业务错误直接透传）。
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentClientError {
    /// 连接级失败（可重连）。
    #[error("连接错误：{0}")]
    Connection(String),
    /// Agent 鉴权失败（token 换发后旧客户端未重连等）。
    #[error("鉴权失败：{0}")]
    Auth(String),
    /// 协议版本不匹配。
    #[error("协议不匹配：{0}")]
    Protocol(String),
    /// 请求超时。
    #[error("请求超时：{0}")]
    Timeout(String),
    /// Agent 业务错误（error.code + message；不触发重连）。
    #[error("Agent 错误 [{code}]：{message}")]
    Agent { code: String, message: String },
}

// ---------------------------------------------------------------------------
// 统一错误收敛：AgentClientError → AppError（attach 命令层经此单点转换，
// 不再各自手拼；Auth → Forbidden 保持「鉴权失败」语义）
// ---------------------------------------------------------------------------

impl From<AgentClientError> for miyo_core::AppError {
    fn from(err: AgentClientError) -> Self {
        match err {
            AgentClientError::Connection(msg) => miyo_core::AppError::Connection(msg),
            AgentClientError::Auth(msg) => miyo_core::AppError::Forbidden(msg),
            AgentClientError::Protocol(msg) => {
                miyo_core::AppError::Connection(format!("协议不匹配：{msg}"))
            }
            AgentClientError::Timeout(msg) => miyo_core::AppError::NetworkTimeout(msg),
            AgentClientError::Agent { code, message } => {
                miyo_core::AppError::Test(format!("Agent 错误 [{code}]：{message}"))
            }
        }
    }
}

impl AgentClientError {
    /// 连接级错误（重连可能恢复）。
    pub fn is_connection_level(&self) -> bool {
        matches!(
            self,
            AgentClientError::Connection(_) | AgentClientError::Timeout(_)
        )
    }

    /// → AppError（命令层透传语义）。
    pub fn to_app_error(&self) -> AppError {
        match self {
            AgentClientError::Connection(msg) => AppError::Connection(msg.clone()),
            AgentClientError::Auth(msg) => AppError::Forbidden(msg.clone()),
            AgentClientError::Protocol(msg) => AppError::Validation(msg.clone()),
            AgentClientError::Timeout(msg) => AppError::NetworkTimeout(msg.clone()),
            AgentClientError::Agent { code, message } => match code.as_str() {
                "not_allowed" => AppError::Forbidden(format!("目标受限：{message}")),
                "not_found" => AppError::NotFound(message.clone()),
                "bad_arguments" | "bad_request" => AppError::Validation(message.clone()),
                "rate_limited" => AppError::Forbidden(format!("触发限流：{message}")),
                "consent_required" => AppError::Forbidden(format!("需要用户确认：{message}")),
                "unsupported" => AppError::NotFound(format!("当前目标不支持：{message}")),
                _ => AppError::Connection(format!("Agent 错误 [{code}]：{message}")),
            },
        }
    }
}

/// Agent 响应 → 强类型（内部工具）。
pub fn parse_agent_data<T: DeserializeOwned>(value: Value) -> Result<T, AppError> {
    serde_json::from_value(value)
        .map_err(|e| AppError::Connection(format!("Agent 响应形状非法：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handshake_error_mapping() {
        // not_authorized → Auth；握手通过后进入请求阶段（mock 对 ping 无响应 → 超时）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    loop {
                        let Ok(frame) = read_frame(&mut socket).await else {
                            return;
                        };
                        let token = frame["params"]["token"].as_str().unwrap_or("");
                        let op = frame["op"].as_str().unwrap_or("");
                        let body = if op == "handshake" {
                            if token != "good" {
                                serde_json::json!({"id": "handshake", "ok": false,
                                    "error": {"code": "not_authorized", "message": "token 不匹配或缺失"}})
                            } else {
                                serde_json::json!({"id": "handshake", "ok": true,
                                    "data": {"protocol": 2, "pid": 1, "agent_version": "t"}})
                            }
                        } else if op == "ping" {
                            serde_json::json!({"id": frame["id"], "ok": true, "data": {"pong": 1}})
                        } else {
                            continue;
                        };
                        let _ = write_frame(&mut socket, &body).await;
                    }
                });
            }
        });
        let client = AgentClient::new(port, "bad".into());
        let err = client
            .request("ping", serde_json::json!({}), 3000)
            .await
            .unwrap_err();
        assert!(matches!(err, AgentClientError::Auth(_)), "{err:?}");

        let client = AgentClient::new(port, "good".into());
        let pong = client
            .request("ping", serde_json::json!({}), 3000)
            .await
            .unwrap();
        assert_eq!(pong["pong"], 1);
    }
}
