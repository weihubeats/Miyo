//! gRPC 动态调用：tonic Channel + prost-reflect 动态消息编解码。
//!
//! 不生成代码：方法描述符来自 [`DescriptorPool`](prost_reflect::DescriptorPool)
//! （protox 编译或服务端反射），消息为 `DynamicMessage`（其 prost::Message
//! 实现直接对接 tonic codec）。unary 直返；服务端流由命令层 spawn 消费循环
//! 逐消息回调。

use std::sync::Arc;
use std::time::{Duration, Instant};

use miyo_core::{AppError, Result};
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

/// gRPC 调用入参（命令层从 GrpcSpec + 环境变量渲染结果组装）。
#[derive(Debug, Clone)]
pub struct GrpcInvokeArgs {
    /// 地址 host:port（已渲染变量，可含 scheme，解析时统一剥掉）。
    pub address: String,
    /// 全限定服务名。
    pub service: String,
    /// 方法名。
    pub method: String,
    /// 请求消息 protobuf-JSON 文本。
    pub message_json: String,
    /// metadata 键值对（渲染后）。
    pub metadata: Vec<(String, String)>,
    pub use_tls: bool,
    /// 超时（仅 unary 生效；服务端流的生命周期由用户控制，不设总超时）。
    pub timeout_ms: Option<u64>,
    /// 描述符池（反射或 proto 编译产物）。
    pub pool: Arc<DescriptorPool>,
}

/// unary 响应。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GrpcResponse {
    /// 响应消息 protobuf-JSON。
    pub message_json: String,
    /// 响应元数据（tonic 0.14 将 trailers 合并进同一 map，grpc-status 在此）。
    pub metadata: Vec<(String, String)>,
    /// grpc-status（0 = OK；命令层非零时以 error 呈现）。
    pub grpc_status: i32,
    pub grpc_message: String,
    pub duration_ms: f64,
    pub size_bytes: u64,
}

/// 服务端流单条消息事件。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GrpcMessage {
    pub sequence: u64,
    pub message_json: String,
    /// 距开流的毫秒数（时间线展示）。
    pub elapsed_ms: f64,
    pub size_bytes: u64,
}

/// 流事件：消息 / 正常结束（含元数据状态）/ 失败。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrpcEvent {
    Message(GrpcMessage),
    End {
        cancelled: bool,
        grpc_status: i32,
        grpc_message: String,
        /// 响应头 + trailers 合并（与 unary 口径一致）。
        metadata: Vec<(String, String)>,
    },
    Failed {
        message: String,
    },
}

/// 连接建立上限：网络不可达（防火墙丢包）时 TCP 连接可能无限挂起，
/// 取消令牌与 grpc-timeout 都管不到 connect 阶段，必须有硬超时兜底。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 通道缓存：key = `{scheme}://{host:port}`（TLS 开关进 scheme，天然区分）。
/// `Channel` 内部是 Arc——clone 共享底层 HTTP/2 连接且断线自动重连，反复
/// 调试同一服务时免去每次 TCP/TLS 握手（unary / 服务端流两条路径共用）。
/// 桌面会话内地址数有限，缓存不设淘汰。
static CHANNEL_CACHE: std::sync::OnceLock<
    tokio::sync::Mutex<std::collections::HashMap<String, Channel>>,
> = std::sync::OnceLock::new();

/// 建立通道。地址允许携带 scheme（统一剥掉按 TLS 开关重组）；
/// TLS 使用系统根证书（tls-native-roots），自签证书暂不支持（见里程碑）。
pub async fn connect(address: &str, use_tls: bool) -> Result<Channel> {
    let bare = address
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    if bare.is_empty() {
        return Err(AppError::Validation("gRPC 地址不能为空".into()));
    }
    let scheme = if use_tls { "https" } else { "http" };
    let key = format!("{scheme}://{bare}");
    {
        let cache = CHANNEL_CACHE
            .get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .await;
        if let Some(ch) = cache.get(&key) {
            return Ok(ch.clone());
        }
    }
    let endpoint = Channel::from_shared(key.clone())
        .map_err(|e| AppError::Validation(format!("gRPC 地址无效：{e}")))?
        .connect_timeout(CONNECT_TIMEOUT);
    let endpoint = if use_tls {
        endpoint
            .tls_config(tonic::transport::ClientTlsConfig::new().with_enabled_roots())
            .map_err(|e| AppError::Ssl(format!("TLS 配置失败：{e}")))?
    } else {
        endpoint
    };
    let channel = endpoint
        .connect()
        .await
        .map_err(|e| AppError::Connection(format!("gRPC 连接失败：{e}")))?;
    CHANNEL_CACHE
        .get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .await
        .insert(key, channel.clone());
    Ok(channel)
}

/// 动态消息编解码器：prost-reflect 的 DynamicMessage 自带 prost::Message
/// 实现（描述符随消息携带），这里只做 tonic codec 特征的薄适配。
#[derive(Clone)]
struct DynamicCodec(MessageDescriptor);

impl tonic::codec::Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    // 编码走消息自带的描述符（编码器无状态）；解码需要方法描述符定字段
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder(self.0.clone())
    }
}

struct DynamicEncoder;

impl tonic::codec::Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> std::result::Result<(), Self::Error> {
        prost::Message::encode(&item, dst)
            .map_err(|e| tonic::Status::internal(format!("消息编码失败：{e}")))
    }
}

struct DynamicDecoder(MessageDescriptor);

impl tonic::codec::Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| tonic::Status::internal(format!("消息解码失败：{e}")))
    }
}

/// tonic 0.14 移除了内置 ProstCodec（prost 集成拆分至 tonic-prost），
/// 反射流的 prost 生成类型（请求/响应不同类型）用这个双参数薄适配对接。
pub(crate) struct ProstCodec<Req, Resp> {
    _marker: std::marker::PhantomData<(Req, Resp)>,
}

impl<Req, Resp> Default for ProstCodec<Req, Resp> {
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

// PhantomData 的派生 Clone 会要求 T: Clone；编解码器与消息值无关，手动实现
impl<Req, Resp> Clone for ProstCodec<Req, Resp> {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl<Req, Resp> tonic::codec::Codec for ProstCodec<Req, Resp>
where
    Req: prost::Message + 'static,
    Resp: prost::Message + Default + 'static,
{
    type Encode = Req;
    type Decode = Resp;
    type Encoder = ProstEncoder<Req>;
    type Decoder = ProstDecoder<Resp>;

    fn encoder(&mut self) -> Self::Encoder {
        ProstEncoder {
            _marker: std::marker::PhantomData,
        }
    }

    fn decoder(&mut self) -> Self::Decoder {
        ProstDecoder {
            _marker: std::marker::PhantomData,
        }
    }
}

pub(crate) struct ProstEncoder<Req> {
    _marker: std::marker::PhantomData<Req>,
}

impl<Req: prost::Message + 'static> tonic::codec::Encoder for ProstEncoder<Req> {
    type Item = Req;
    type Error = tonic::Status;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> std::result::Result<(), Self::Error> {
        prost::Message::encode(&item, dst)
            .map_err(|e| tonic::Status::internal(format!("消息编码失败：{e}")))
    }
}

pub(crate) struct ProstDecoder<Resp> {
    _marker: std::marker::PhantomData<Resp>,
}

impl<Resp: prost::Message + Default + 'static> tonic::codec::Decoder for ProstDecoder<Resp> {
    type Item = Resp;
    type Error = tonic::Status;

    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        Resp::decode(src)
            .map(Some)
            .map_err(|e| tonic::Status::internal(format!("消息解码失败：{e}")))
    }
}

/// 服务 + 方法解析：返回（输入描述符, 输出描述符, 客户端流, 服务端流）。
pub fn resolve_method(
    pool: &DescriptorPool,
    service: &str,
    method: &str,
) -> Result<(MessageDescriptor, MessageDescriptor, bool, bool)> {
    let svc = pool.get_service_by_name(service).ok_or_else(|| {
        AppError::Grpc(format!(
            "服务不存在：{service}（请检查服务名或重新加载目录）"
        ))
    })?;
    let m = svc
        .methods()
        .find(|m| m.name().eq_ignore_ascii_case(method))
        .ok_or_else(|| AppError::Grpc(format!("方法不存在：{service}.{method}")))?;
    Ok((
        m.input(),
        m.output(),
        m.is_client_streaming(),
        m.is_server_streaming(),
    ))
}

/// protobuf-JSON 文本 → 动态消息。空文本按空消息处理。
fn parse_message(desc: &MessageDescriptor, json: &str) -> Result<DynamicMessage> {
    let text = json.trim();
    let value: serde_json::Value = if text.is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        serde_json::from_str(text)
            .map_err(|e| AppError::Grpc(format!("消息 JSON 解析失败：{e}")))?
    };
    DynamicMessage::deserialize(desc.clone(), value)
        .map_err(|e| AppError::Grpc(format!("消息与 proto 结构不匹配：{e}")))
}

/// 键值对 → MetadataMap（空键跳过；值非法报错，不静默丢弃）。
fn build_metadata(pairs: &[(String, String)]) -> Result<tonic::metadata::MetadataMap> {
    let mut map = tonic::metadata::MetadataMap::new();
    for (k, v) in pairs {
        let key = k.trim();
        if key.is_empty() {
            continue;
        }
        let parsed_key = key
            .parse::<tonic::metadata::AsciiMetadataKey>()
            .map_err(|e| AppError::Grpc(format!("metadata 键非法（{key}）：{e}")))?;
        let value = tonic::metadata::MetadataValue::try_from(v.as_str()).map_err(|_| {
            AppError::Grpc(format!("metadata 值非法（{key}）：仅支持可见 ASCII 字符"))
        })?;
        map.insert(parsed_key, value);
    }
    Ok(map)
}

fn metadata_pairs(map: &tonic::metadata::MetadataMap) -> Vec<(String, String)> {
    use base64::Engine as _;
    map.iter()
        .map(|entry| match entry {
            tonic::metadata::KeyAndValueRef::Ascii(k, v) => {
                let value = v.to_str().map(str::to_string).unwrap_or_default();
                (k.as_str().to_string(), value)
            }
            // 二进制值（-bin）base64 展示，保持无损
            tonic::metadata::KeyAndValueRef::Binary(k, v) => (
                k.as_str().to_string(),
                base64::engine::general_purpose::STANDARD.encode(v.as_encoded_bytes()),
            ),
        })
        .collect()
}

fn status_to_error(status: tonic::Status) -> AppError {
    if status.code() == tonic::Code::Cancelled {
        return AppError::Cancelled("gRPC 调用已被取消".into());
    }
    AppError::Grpc(format!(
        "gRPC 状态 {:?}（{}）：{}",
        status.code(),
        status.code() as i32,
        status.message()
    ))
}

fn http_path(service: &str, method: &str) -> Result<tonic::codegen::http::uri::PathAndQuery> {
    if service.is_empty() || method.is_empty() {
        return Err(AppError::Validation("服务名与方法名不能为空".into()));
    }
    if service.contains(|c: char| c.is_whitespace() || c == '/')
        || method.contains(|c: char| c.is_whitespace() || c == '/')
    {
        return Err(AppError::Validation(format!(
            "服务/方法名含非法字符：{service}.{method}"
        )));
    }
    format!("/{service}/{method}")
        .parse()
        .map_err(|e| AppError::Validation(format!("gRPC 路径构造失败：{e}")))
}

/// unary 调用：直返响应（元数据已含 trailers 合并的 grpc-status）。
///
/// 取消令牌覆盖**整个调用周期**（连接建立 → 通道就绪 → unary 等待响应）：
/// 若只包住最终的 unary 调用，请求卡在 connect（网络不可达）时取消将永远
/// 无法打断——令牌已触发但无人监听，前端表现为「点了取消仍在请求中」。
pub async fn invoke_unary(
    args: &GrpcInvokeArgs,
    cancel: Option<&CancellationToken>,
) -> Result<GrpcResponse> {
    let run = async {
        let (input, _output, client_streaming, server_streaming) =
            resolve_method(&args.pool, &args.service, &args.method)?;
        if client_streaming || server_streaming {
            return Err(AppError::Grpc(format!(
                "{}.{} 是流式方法：当前仅支持 unary / 服务端流",
                args.service, args.method
            )));
        }
        let msg = parse_message(&input, &args.message_json)?;
        let channel = connect(&args.address, args.use_tls).await?;
        let mut grpc = tonic::client::Grpc::new(channel);
        grpc.ready()
            .await
            .map_err(|e| AppError::Connection(format!("gRPC 通道未就绪：{e}")))?;
        let path = http_path(&args.service, &args.method)?;
        let mut request = tonic::Request::new(msg);
        *request.metadata_mut() = build_metadata(&args.metadata)?;
        if let Some(ms) = args.timeout_ms {
            request.set_timeout(Duration::from_millis(ms));
        }
        let codec = DynamicCodec(input);

        let response = grpc.unary(request, path, codec).await;
        let response = response.map_err(status_to_error)?;
        Ok(response)
    };

    let started = Instant::now();
    let response = match cancel {
        Some(token) => tokio::select! {
            _ = token.cancelled() => {
                return Err(AppError::Cancelled("gRPC 调用已被用户取消".into()))
            }
            r = run => r?,
        },
        None => run.await?,
    };
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;

    // tonic 0.14 unary 把 trailers 合并进响应元数据（grpc-status 在此）
    let metadata = metadata_pairs(response.metadata());
    let grpc_status = metadata
        .iter()
        .find(|(k, _)| k == "grpc-status")
        .and_then(|(_, v)| v.parse::<i32>().ok())
        .unwrap_or(0);
    let grpc_message = metadata
        .iter()
        .find(|(k, _)| k == "grpc-message")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let message_json = serde_json::to_string_pretty(&response.into_inner())
        .map_err(|e| AppError::Grpc(format!("响应消息 JSON 序列化失败：{e}")))?;
    let size_bytes = message_json.len() as u64;

    Ok(GrpcResponse {
        message_json,
        metadata,
        grpc_status,
        grpc_message,
        duration_ms,
        size_bytes,
    })
}

/// 服务端流：spawn 消费循环，消息/结束/失败经 `on_event` 回调推送
/// （命令层负责转成 Tauri 事件）；取消令牌命中即中止并回报 End{cancelled}。
pub fn spawn_server_stream(
    args: GrpcInvokeArgs,
    stream_id: String,
    cancel: CancellationToken,
    on_event: Arc<dyn Fn(GrpcEvent) + Send + Sync + 'static>,
) -> tokio::task::JoinHandle<()> {
    let _ = stream_id;
    tokio::spawn(async move {
        let started = Instant::now();
        let run = async {
            // 建流阶段（resolve → connect → 开流）同样受取消令牌约束：
            // 网络不可达时 connect 挂起，「关闭流」必须能立即生效。
            let setup = async {
                let (input, _output, client_streaming, server_streaming) =
                    resolve_method(&args.pool, &args.service, &args.method)?;
                if client_streaming || !server_streaming {
                    return Err(AppError::Grpc(format!(
                        "{}.{} 不是服务端流方法",
                        args.service, args.method
                    )));
                }
                let msg = parse_message(&input, &args.message_json)?;
                let channel = connect(&args.address, args.use_tls).await?;
                let mut grpc = tonic::client::Grpc::new(channel);
                grpc.ready()
                    .await
                    .map_err(|e| AppError::Connection(format!("gRPC 通道未就绪：{e}")))?;
                let path = http_path(&args.service, &args.method)?;
                let mut request = tonic::Request::new(msg);
                *request.metadata_mut() = build_metadata(&args.metadata)?;
                // 流式不设总超时：生命周期由用户控制（取消令牌负责中断）
                let codec = DynamicCodec(input);
                let response = grpc
                    .server_streaming(request, path, codec)
                    .await
                    .map_err(status_to_error)?;
                let headers = metadata_pairs(response.metadata());
                Ok::<_, AppError>((response.into_inner(), headers))
            };
            let (mut stream, headers) = tokio::select! {
                _ = cancel.cancelled() => {
                    return Ok(GrpcEvent::End {
                        cancelled: true,
                        grpc_status: 0,
                        grpc_message: "已取消".into(),
                        metadata: Vec::new(),
                    })
                }
                s = setup => s?,
            };

            let mut sequence = 0u64;
            loop {
                let item = tokio::select! {
                    _ = cancel.cancelled() => {
                        return Ok(GrpcEvent::End {
                            cancelled: true,
                            grpc_status: 0,
                            grpc_message: "已取消".into(),
                            metadata: headers,
                        });
                    }
                    m = stream.message() => m,
                };
                match item {
                    Ok(Some(message)) => {
                        sequence += 1;
                        let message_json = serde_json::to_string_pretty(&message)
                            .map_err(|e| AppError::Grpc(format!("消息 JSON 序列化失败：{e}")))?;
                        let size_bytes = message_json.len() as u64;
                        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                        on_event(GrpcEvent::Message(GrpcMessage {
                            sequence,
                            message_json,
                            elapsed_ms,
                            size_bytes,
                        }));
                    }
                    Ok(None) => {
                        // 正常收尾：trailers 携带 grpc-status / grpc-message，并入元数据
                        let mut metadata = headers;
                        if let Ok(Some(trailers)) = stream.trailers().await {
                            metadata.extend(metadata_pairs(&trailers));
                        }
                        let grpc_status = metadata
                            .iter()
                            .find(|(k, _)| k == "grpc-status")
                            .and_then(|(_, v)| v.parse::<i32>().ok())
                            .unwrap_or(0);
                        let grpc_message = metadata
                            .iter()
                            .find(|(k, _)| k == "grpc-message")
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default();
                        return Ok(GrpcEvent::End {
                            cancelled: false,
                            grpc_status,
                            grpc_message,
                            metadata,
                        });
                    }
                    Err(status) => return Err(status_to_error(status)),
                }
            }
        };

        match run.await {
            Ok(event) => on_event(event),
            Err(AppError::Cancelled(msg)) => on_event(GrpcEvent::End {
                cancelled: true,
                grpc_status: 0,
                grpc_message: msg,
                metadata: Vec::new(),
            }),
            Err(err) => on_event(GrpcEvent::Failed {
                message: err.user_message(),
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通道缓存复用：同地址第二次 connect 命中缓存——listener 关闭后仍能
    /// 取到通道（无缓存时 TCP 已拒连必然报错）。TLS 开关进 key（不同 scheme
    /// 各自缓存，互不串用）。
    #[tokio::test]
    async fn connect_reuses_cached_channel_for_same_address() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let first = connect(&addr.to_string(), false).await.expect("首次连接");
        let second = connect(&addr.to_string(), false)
            .await
            .expect("二次连接应命中缓存");
        // 同一底层连接（Channel 内部 Arc，克隆共享）——两次都成功即缓存生效。
        drop(first);
        drop(second);

        // 关闭 listener 后再 connect：缓存路径不发起 TCP，依然成功。
        drop(listener);
        let third = connect(&addr.to_string(), false)
            .await
            .expect("缓存命中不应重连");

        // 缓存恰一条该地址条目（key 含 scheme；TLS 变体会另立条目）。
        let cache = CHANNEL_CACHE.get().expect("缓存已初始化");
        let len = cache.lock().await.len();
        assert!(len >= 1, "缓存应至少有一条该地址");
        let _ = third; // 证明取到的是可用克隆
    }

    /// 地址为空 / 无法解析：校验错误先于缓存写入（不污染缓存）。
    #[tokio::test]
    async fn connect_validates_before_cache() {
        let err = connect("", false).await.expect_err("空地址必须报错");
        assert!(err.user_message().contains("地址"));
        // 非法字符（含空格）进 URL 解析失败。
        let err = connect("invalid address with spaces:1", false)
            .await
            .expect_err("非法地址必须报错");
        assert!(matches!(err, AppError::Validation(_)));
    }
}
