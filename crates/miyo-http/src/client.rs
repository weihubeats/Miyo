//! HTTP 请求构建、发送、响应解析（SPEC §14）。

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::{Client, Method, Response};
use serde::{Deserialize, Serialize};
use url::Url;

use miyo_core::model::{
    ApiKeyLocation, AuthSpec, BodySpec, GraphQLError, GraphQLErrorLocation, GraphQLResponse,
    GraphQLSpec, HttpMethod, KeyValue, MultipartField, MultipartValueType, RequestSpec,
};
use miyo_core::variable::{resolve_variables, VariableMap};
use miyo_core::AppError;

use crate::signature::apply_signature;

/// 默认超时（毫秒），300 秒。
pub const DEFAULT_TIMEOUT_MS: u64 = 300_000;
/// 最大响应体大小（字节）。
pub const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

/// Cookie 数据。
#[derive(Debug, Clone, PartialEq)]
pub struct CookieData {
    pub name: String,
    pub value: String,
}

/// 请求体快照预览上限（64 KB）：够定位「实际发出去的是什么」，
/// 大表单 / 大文件体不随 IPC 回传。
const SNAPSHOT_BODY_BYTES: usize = 64 * 1024;

/// 本次请求实际发出形态的快照（变量渲染、全局参数注入、认证应用、
/// Cookie Jar 回放之后的最终值），供前端「请求」页签排查错误。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestSnapshot {
    pub method: String,
    /// 实际发送的完整 URL（含 query 参数）。
    pub url: String,
    /// 渲染前 URL 模板（含 `{{变量}}`）：由调用方填充，便于对照变量替换结果。
    pub url_template: Option<String>,
    /// 实际发送的请求头（含认证注入与自动 Content-Type / Content-Length；
    /// Cookie 单列，见 `cookies`）。
    pub headers: Vec<(String, String)>,
    /// Cookie Jar 自动回放的 `(name, value)`。
    pub cookies: Vec<(String, String)>,
    /// 请求体预览（文本）；流式 / 二进制体为摘要说明。
    pub body: String,
    /// `body` 是否非完整原文（截断或摘要）。
    pub body_omitted: bool,
}

/// 响应数据。
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponseData {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    /// 毫秒（浮点，保留亚毫秒精度）。
    pub duration_ms: f64,
    pub size_bytes: usize,
    pub cookies: Vec<CookieData>,
    pub truncated: bool,
    /// 本次请求实际发出的形态（见 [`RequestSnapshot`]）；
    /// 手工构造的响应（测试 / 脚本沙箱）为 `None`。
    pub request: Option<RequestSnapshot>,
}

impl HttpResponseData {
    /// 响应体按 UTF-8 解码（失败时回退到 lossy）。
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// 响应 Content-Type。
    pub fn content_type(&self) -> String {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

fn reqwest_method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::GET => Method::GET,
        HttpMethod::POST => Method::POST,
        HttpMethod::PUT => Method::PUT,
        HttpMethod::DELETE => Method::DELETE,
        HttpMethod::PATCH => Method::PATCH,
        HttpMethod::HEAD => Method::HEAD,
        HttpMethod::OPTIONS => Method::OPTIONS,
        // 不可达：gRPC 在 send_request_inner 入口已拦截；兜底按字面方法名
        HttpMethod::Grpc => Method::from_bytes(b"GRPC").expect("GRPC 为合法方法 token"),
    }
}

/// 将已渲染的 Query 参数拼接到 URL。
fn append_query(url: &mut Url, params: &[KeyValue]) {
    for kv in params {
        if !kv.enabled {
            continue;
        }
        let key = kv.key.trim();
        if key.is_empty() {
            continue;
        }
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair(key, &kv.value);
    }
}

/// 渲染后的请求载荷（body 与 content-type）。
///
/// `Stream` 为文件流式上传：磁盘文件不再全量读入内存，
/// 以 chunked 方式边读边发（总长度未知，不设 Content-Length）。
/// 注：`reqwest::Body` 未实现 `Debug`，故手写实现（不打印 body 内容）。
enum Payload {
    None,
    Bytes(Vec<u8>, Option<String>),
    Stream {
        body: reqwest::Body,
        content_type: Option<String>,
        /// 已知总长度时附带（如 binary 单文件），服务端可据此做进度与校验。
        content_length: Option<u64>,
        /// 快照正文预览：流式体发出后无法回读，构造阶段顺带记下可读形态
        /// （如 multipart 各字段）；`None` 时快照只给摘要。
        preview: Option<String>,
    },
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Payload::None => write!(f, "None"),
            Payload::Bytes(b, ct) => write!(f, "Bytes({} bytes, {:?})", b.len(), ct),
            Payload::Stream {
                content_type,
                content_length,
                ..
            } => write!(f, "Stream({:?}, {:?})", content_type, content_length),
        }
    }
}

fn upload_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("upload")
        .to_string()
}

/// 构建 multipart/form-data 请求体。
///
/// Text 字段直接作为文本 part；FilePath 字段以文件流作为 part，
/// 表单整体同样流式发送。content-type 由 `Form::boundary` 拼出（含 boundary）。
async fn build_multipart(fields: &[MultipartField]) -> Result<Payload, AppError> {
    let mut form = reqwest::multipart::Form::new();
    // 快照预览：文本 part 记真实值，文件 part 记字段名 + 路径。
    // 流式体发出后无法回读，可读形态只能在构造阶段留下。
    let mut preview = String::new();
    for field in fields {
        if !field.enabled {
            continue;
        }
        let key = field.key.trim();
        if key.is_empty() {
            continue;
        }
        match field.value_type {
            MultipartValueType::Text => {
                form = form.text(key.to_string(), field.value.clone());
                push_preview_line(&mut preview, &format!("{key}={}", field.value));
            }
            MultipartValueType::FilePath => {
                let path = Path::new(&field.value);
                let file = tokio::fs::File::open(path).await.map_err(|e| {
                    AppError::Validation(format!("读取文件 {} 失败：{e}", path.display()))
                })?;
                let stream = tokio_util::io::ReaderStream::new(file);
                let part = reqwest::multipart::Part::stream(reqwest::Body::wrap_stream(stream))
                    .file_name(upload_file_name(path))
                    .mime_str("application/octet-stream")
                    .map_err(|e| AppError::Validation(format!("multipart 构建失败：{e}")))?;
                form = form.part(key.to_string(), part);
                push_preview_line(&mut preview, &format!("{key}=[文件] {}", path.display()));
            }
        }
    }
    let content_type = format!("multipart/form-data; boundary={}", form.boundary());
    // 表单编码流直接作为请求体：文本 part 常驻内存不可避免，
    // 文件 part 边读边发，大文件不再物化。
    let stream = form.into_stream();
    Ok(Payload::Stream {
        body: reqwest::Body::wrap_stream(stream),
        content_type: Some(content_type),
        content_length: None,
        preview: Some(preview),
    })
}

async fn build_payload(spec: &RequestSpec) -> Result<Payload, AppError> {
    match &spec.body {
        // gRPC 在 send_request_inner 入口已拦截，此处兜底空载荷
        BodySpec::Grpc { .. } => Ok(Payload::None),
        BodySpec::None => Ok(Payload::None),
        BodySpec::Json { raw } => Ok(Payload::Bytes(
            raw.as_bytes().to_vec(),
            Some("application/json".into()),
        )),
        BodySpec::Text { raw } => Ok(Payload::Bytes(
            raw.as_bytes().to_vec(),
            Some("text/plain".into()),
        )),
        BodySpec::UrlEncoded { fields } => {
            let body: Vec<(String, String)> = fields
                .iter()
                .filter(|kv| kv.enabled)
                .map(|kv| (kv.key.clone(), kv.value.clone()))
                .collect();
            let body = serde_urlencoded::to_string(body).unwrap_or_default();
            Ok(Payload::Bytes(
                body.into_bytes(),
                Some("application/x-www-form-urlencoded".into()),
            ))
        }
        BodySpec::Multipart { fields } => build_multipart(fields).await,
        BodySpec::GraphQL { spec } => {
            let body = graphql_request_json(spec, &VariableMap::new())?;
            Ok(Payload::Bytes(
                body.into_bytes(),
                Some("application/json".into()),
            ))
        }
        // 二进制模式：文件流作为请求体（边读边发）；
        // Content-Length 取文件元信息（用户显式设置的请求头优先）。
        BodySpec::Binary { path } => {
            let fs_path = Path::new(path);
            let file = tokio::fs::File::open(fs_path)
                .await
                .map_err(|e| AppError::Validation(format!("读取文件 {path} 失败：{e}")))?;
            let len = file.metadata().await.map(|m| m.len()).unwrap_or_default();
            let stream = tokio_util::io::ReaderStream::new(file);
            Ok(Payload::Stream {
                body: reqwest::Body::wrap_stream(stream),
                content_type: Some("application/octet-stream".into()),
                content_length: if len > 0 { Some(len) } else { None },
                preview: None,
            })
        }
    }
}

/// 构建 GraphQL 请求体 JSON：`{"query":..., "variables":..., "operationName":...}`。
///
/// query / variables / operationName 中的 `{{name}}` 占位符会先经
/// `vars` 插值；variables 为空串或 `{}` 时省略该字段，operationName 为空时省略。
/// 请求始终为 POST + `application/json`（GraphQL over HTTP 约定）。
pub fn graphql_request_json(spec: &GraphQLSpec, vars: &VariableMap) -> Result<String, AppError> {
    let query = resolve_variables(&spec.query, vars);
    let variables = resolve_variables(&spec.variables, vars);
    let operation_name = resolve_variables(&spec.operation_name, vars);

    let mut payload = serde_json::Map::new();
    payload.insert("query".into(), serde_json::Value::String(query));
    let trimmed = variables.trim();
    if !trimmed.is_empty() && trimmed != "{}" {
        // 变量必须是合法 JSON 对象；非法时向用户报错而不是发坏请求。
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) if value.is_object() => {
                payload.insert("variables".into(), value);
            }
            Ok(_) => {
                return Err(AppError::Validation("GraphQL 变量必须是 JSON 对象".into()));
            }
            Err(e) => {
                return Err(AppError::Validation(format!("GraphQL 变量 JSON 无效：{e}")));
            }
        }
    }
    if !operation_name.trim().is_empty() {
        payload.insert(
            "operationName".into(),
            serde_json::Value::String(operation_name),
        );
    }
    serde_json::to_string(&serde_json::Value::Object(payload))
        .map_err(|e| AppError::Validation(format!("GraphQL 请求构建失败：{e}")))
}

/// 解析 GraphQL 响应，区分 `data` 与 `errors`。
///
/// 非 JSON 响应返回错误；JSON 合法但缺少 data / errors 时对应字段为空。
pub fn parse_graphql_response(body: &[u8]) -> Result<GraphQLResponse, AppError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| AppError::Validation(format!("GraphQL 响应不是合法 JSON：{e}")))?;
    let data = value.get("data").cloned();
    let errors = value
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let message = entry.get("message")?.as_str()?.to_string();
                    let locations = entry.get("locations").and_then(|l| l.as_array()).map(|ls| {
                        ls.iter()
                            .filter_map(|loc| {
                                Some(GraphQLErrorLocation {
                                    line: loc.get("line")?.as_u64()?,
                                    column: loc.get("column")?.as_u64()?,
                                })
                            })
                            .collect()
                    });
                    let path = entry.get("path").and_then(|p| p.as_array()).cloned();
                    Some(GraphQLError {
                        message,
                        locations,
                        path,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(GraphQLResponse { data, errors })
}

/// 是否已有指定头（不区分大小写）。
fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(k, _)| k.trim().eq_ignore_ascii_case(name))
}

/// 构造本次实际发出请求的快照（见 [`RequestSnapshot`]）。
///
/// 头与体按真实发送逻辑推导：spec 头（已含认证注入）+ 用户未显式设置时的
/// 自动 Content-Type / Content-Length；Cookie 取当前 URL 在 Jar 中的回放值。
/// Digest 认证首轮不带 Authorization（401 质询后才补发），快照取首轮。
fn build_request_snapshot(
    method: HttpMethod,
    url: &Url,
    headers: &[(String, String)],
    payload: &Payload,
    disable_cookies: bool,
) -> RequestSnapshot {
    let mut snapshot_headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .cloned()
        .collect();
    let mut body = String::new();
    let mut body_omitted = false;
    match payload {
        Payload::None => {}
        Payload::Bytes(bytes, content_type) => {
            push_auto_header(
                &mut snapshot_headers,
                headers,
                "Content-Type",
                content_type.as_deref(),
            );
            let text = String::from_utf8_lossy(bytes);
            let (preview, truncated) = truncate_text(&text, SNAPSHOT_BODY_BYTES);
            body = preview;
            body_omitted = truncated;
        }
        Payload::Stream {
            content_type,
            content_length,
            preview,
            ..
        } => {
            push_auto_header(
                &mut snapshot_headers,
                headers,
                "Content-Type",
                content_type.as_deref(),
            );
            if let Some(len) = content_length {
                push_auto_header(
                    &mut snapshot_headers,
                    headers,
                    "Content-Length",
                    Some(&len.to_string()),
                );
            }
            let kind = content_type.as_deref().unwrap_or("流式请求体");
            match preview {
                // multipart 等构造期留了可读形态：直接当正文展示（与 Bytes 同口径截断）。
                Some(text) => {
                    let (shown, cut) = truncate_text(text, SNAPSHOT_BODY_BYTES);
                    body = shown;
                    body_omitted = cut;
                }
                None => {
                    body = match content_length {
                        Some(len) => format!("[{kind}：边读边发，{len} 字节]"),
                        None => format!("[{kind}：边读边发，长度未知]"),
                    };
                    body_omitted = true;
                }
            }
        }
    }
    let cookies = if disable_cookies {
        Vec::new()
    } else {
        // trait 方法（与 `SharedJarRef` 同一实现）；禁用 Cookie 的请求走 EmptyJar。
        use reqwest::cookie::CookieStore as _;
        parse_cookie_pairs(crate::cookie::shared_jar().cookies(url))
    };
    RequestSnapshot {
        method: method.to_string(),
        url: url.to_string(),
        url_template: None,
        headers: snapshot_headers,
        cookies,
        body,
        body_omitted,
    }
}

/// 快照头补齐：只在用户未显式设置同名头时补自动值（与发送逻辑同口径）。
fn push_auto_header(
    out: &mut Vec<(String, String)>,
    existing: &[(String, String)],
    name: &str,
    value: Option<&str>,
) {
    if let Some(v) = value {
        if !has_header(existing, name) && !has_header(out, name) {
            out.push((name.to_string(), v.to_string()));
        }
    }
}

/// 按字节上限截断文本（字符边界安全），返回预览与是否截断。
fn truncate_text(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// 追加一行 multipart 快照预览；累计超上限后不再追加（快照侧仍会截断标记）。
/// 单行本身可能超上限（超大文本字段），由 `truncate_text` 兜底。
fn push_preview_line(out: &mut String, line: &str) {
    if out.len() >= SNAPSHOT_BODY_BYTES {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
}

/// Cookie 头拆成 `(name, value)`（Jar 回放形如 `a=1; b=2`）。
fn parse_cookie_pairs(header: Option<reqwest::header::HeaderValue>) -> Vec<(String, String)> {
    let Some(Ok(text)) = header.map(|v| v.to_str().map(|s| s.to_string())) else {
        return Vec::new();
    };
    text.split(';')
        .filter_map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// 认证字段转 Header / Query。
/// 应用认证到请求头 / 查询参数。
///
/// OAuth2：取 access token（过期自动静默刷新），未授权时返回中文错误。
/// 签名类（Hawk / AWS SigV4 / HMAC）：按 `ctx`（方法 + 最终 URL + 载荷摘要）
/// 实时计算并写入请求头；字段值视为已做变量渲染。
/// Digest：首次不发送（等待服务端 401 质询，由发送循环自动重发）。
async fn apply_auth(
    headers: &mut Vec<(String, String)>,
    query: &mut Vec<KeyValue>,
    auth: &AuthSpec,
    ctx: &AuthSignCtx<'_>,
) -> Result<(), AppError> {
    match auth {
        AuthSpec::None => {}
        AuthSpec::Bearer { token } => {
            if !token.is_empty() {
                headers.push(("Authorization".into(), format!("Bearer {token}")));
            }
        }
        AuthSpec::Basic { username, password } => {
            let raw = format!("{username}:{password}");
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            headers.push(("Authorization".into(), format!("Basic {encoded}")));
        }
        AuthSpec::ApiKey {
            key,
            value,
            location,
        } => {
            if key.is_empty() {
                return Ok(());
            }
            match location {
                ApiKeyLocation::Header => {
                    headers.push((key.clone(), value.clone()));
                }
                ApiKeyLocation::Query => {
                    query.push(KeyValue::new(key.clone(), value.clone()));
                }
            }
        }
        AuthSpec::OAuth2 { .. } => {
            let access = miyo_oauth::access_token_for(auth)
                .await
                .map_err(|e| AppError::OAuth2(e.to_string()))?;
            headers.push(("Authorization".into(), format!("Bearer {access}")));
        }
        AuthSpec::Digest { .. } => {
            // 挑战-应答式：首轮不带凭据，401 后由发送循环补发（见 `digest_retry_header`）。
        }
        AuthSpec::Hawk { key_id, key } => {
            let ts: u64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let nonce = miyo_sign::random_nonce_hex(4);
            let header = miyo_sign::hawk_authorization(&miyo_sign::HawkParams {
                id: key_id,
                key,
                method: &ctx.method.to_string(),
                url: ctx.url.as_str(),
                body: ctx.body,
                ts,
                nonce: &nonce,
            })
            .map_err(|e| AppError::Validation(format!("Hawk 加签失败：{e}")))?;
            headers.push(("Authorization".into(), header));
        }
        AuthSpec::AwsV4 {
            access_key,
            secret_key,
            region,
            service,
            session_token,
        } => {
            let signed = miyo_sign::sign_aws_v4(&miyo_sign::AwsV4Params {
                access_key,
                secret_key,
                session_token: session_token.as_deref(),
                region,
                service,
                method: &ctx.method.to_string(),
                url: ctx.url.as_str(),
                payload_hash: ctx.payload_hash,
                amz_date: &miyo_sign::aws_amz_date_now(),
            })
            .map_err(|e| AppError::Validation(format!("AWS SigV4 加签失败：{e}")))?;
            headers.push(("x-amz-date".into(), signed.amz_date));
            if let Some(token) = session_token {
                if !token.trim().is_empty() {
                    headers.push(("x-amz-security-token".into(), token.clone()));
                }
            }
            headers.push(("Authorization".into(), signed.authorization));
        }
        AuthSpec::Hmac {
            access_key,
            secret_key,
        } => {
            let mut path_query = ctx.url.path().to_string();
            if let Some(q) = ctx.url.query() {
                path_query.push('?');
                path_query.push_str(q);
            }
            let timestamp = miyo_sign::utc_timestamp_secs();
            let nonce = miyo_sign::random_nonce_hex(4);
            let (signed, _) = miyo_sign::aksk_headers(&miyo_sign::AkSkParams {
                access_key,
                secret_key,
                method: &ctx.method.to_string(),
                path_query: &path_query,
                timestamp: &timestamp,
                nonce: &nonce,
            })
            .map_err(|e| AppError::Validation(format!("HMAC 加签失败：{e}")))?;
            headers.extend(signed);
        }
        // 动态签名（App-Key/Secret/Timestamp/Sig）：发送前最后一刻按模板计算。
        AuthSpec::DynamicSignature { config } => {
            apply_signature(headers, config)?;
        }
    }
    Ok(())
}

/// 加签上下文：签名类认证计算所需的请求快照。
struct AuthSignCtx<'a> {
    method: HttpMethod,
    /// 已拼接显式 params 的最终 URL（ApiKey-query 附加项除外，见下）。
    url: &'a Url,
    /// 载荷摘要（hex；流式 body 为 `UNSIGNED-PAYLOAD`）。
    payload_hash: &'a str,
    /// Hawk payload hash 用（content-type + 原始字节；无 body 为 `None`）。
    body: Option<(&'a str, &'a [u8])>,
}

/// Digest 重试：401 首轮响应携带合法 Digest 质询时，计算应答头（否则返回 `None` 不重试）。
fn digest_retry_header(
    resp: &Response,
    auth: &AuthSpec,
    method: HttpMethod,
    url: &Url,
) -> Option<String> {
    if resp.status().as_u16() != 401 {
        return None;
    }
    let AuthSpec::Digest { username, password } = auth else {
        return None;
    };
    let challenge = resp
        .headers()
        .get_all("www-authenticate")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(miyo_sign::parse_www_authenticate)?;
    let mut uri = url.path().to_string();
    if let Some(q) = url.query() {
        uri.push('?');
        uri.push_str(q);
    }
    miyo_sign::digest_authorization(&miyo_sign::DigestParams {
        username,
        password,
        method: &method.to_string(),
        uri: &uri,
        challenge: &challenge,
        nc: 1,
        cnonce: &miyo_sign::random_nonce_hex(4),
    })
    .ok()
}

/// 重定向策略是 Client 级配置，故按 `RequestSpec::follow_redirects`
/// 维护两个共享实例（跟随 = 默认最多 10 跳；不跟随 = `Policy::none`）。
///
/// 客户端对放在 `RwLock` 里：代理设置变更时整体重建（`Client` 内部是
/// Arc,克隆廉价）。默认启用 cookie 存储：同域后续请求自动回放
/// Set-Cookie（对标 Postman/Bruno 的默认行为）。
struct ClientPair {
    follow: Client,
    no_follow: Client,
    follow_no_cookies: Client,
    no_follow_no_cookies: Client,
}

fn build_pair(proxy: Option<&str>, tls: &TlsOptions) -> Result<ClientPair, String> {
    // 自管 jar：cookies=true 时自动回放 + 可查看/清理；
    // false（单请求禁用）时走空 jar，登录态不附带。
    // jar 全局共享：代理切换不再丢弃登录态（原来随旧客户端一起丢弃）。
    let build = |policy: reqwest::redirect::Policy, cookies: bool| -> Result<Client, String> {
        // 建连限速已按远程意见移除（7cf8cba），避免对慢服务误杀：总超时留给请求本身。
        let base = Client::builder().redirect(policy);
        // TLS：客户端证书（rustls 栈 → Identity::from_pem，PEM 文件含证书链
        // 与私钥，顺序不限）与自签放行。构建失败直接报错（配置错误不应
        // 静默降级为无证书）。
        let base = if tls.accept_invalid_certs {
            base.danger_accept_invalid_certs(true)
        } else {
            base
        };
        let base = match tls.identity_path.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                let pem =
                    std::fs::read(path.trim()).map_err(|e| format!("读取客户端证书失败：{e}"))?;
                let identity = reqwest::Identity::from_pem(&pem).map_err(|e| {
                    format!("客户端证书加载失败（PEM 需含证书链与私钥，私钥不支持密码加密）：{e}")
                })?;
                base.identity(identity)
            }
            _ => base,
        };
        let with_proxy = match proxy {
            Some(url) => {
                base.proxy(reqwest::Proxy::all(url).map_err(|e| format!("代理地址无效：{e}"))?)
            }
            // 无代理配置时禁用系统代理：本地开发（127.0.0.1）不受环境变量干扰。
            None => base.no_proxy(),
        };
        if cookies {
            with_proxy
                .cookie_provider(std::sync::Arc::new(SharedJarRef))
                .build()
                .map_err(|e| e.to_string())
        } else {
            with_proxy
                .cookie_provider(std::sync::Arc::new(EmptyJar))
                .build()
                .map_err(|e| e.to_string())
        }
    };
    Ok(ClientPair {
        follow: build(reqwest::redirect::Policy::default(), true)?,
        no_follow: build(reqwest::redirect::Policy::none(), true)?,
        follow_no_cookies: build(reqwest::redirect::Policy::default(), false)?,
        no_follow_no_cookies: build(reqwest::redirect::Policy::none(), false)?,
    })
}

/// 指向全局共享 jar 的 provider（`cookie_provider` 要求 `Arc<dyn CookieStore>`，
/// 不能直接传 `&'static ManagedJar`，故包一层）。
#[derive(Debug)]
struct SharedJarRef;

impl reqwest::cookie::CookieStore for SharedJarRef {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        url: &Url,
    ) {
        crate::cookie::shared_jar().set_cookies(cookie_headers, url)
    }

    fn cookies(&self, url: &Url) -> Option<reqwest::header::HeaderValue> {
        crate::cookie::shared_jar().cookies(url)
    }
}

/// 空 jar：禁用回放的请求专用（不收纳、不附带）。
#[derive(Debug, Default)]
struct EmptyJar;

impl reqwest::cookie::CookieStore for EmptyJar {
    fn set_cookies(
        &self,
        _cookie_headers: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        _url: &Url,
    ) {
    }

    fn cookies(&self, _url: &Url) -> Option<reqwest::header::HeaderValue> {
        None
    }
}

fn client_pair() -> &'static std::sync::RwLock<ClientPair> {
    static PAIR: OnceLock<std::sync::RwLock<ClientPair>> = OnceLock::new();
    PAIR.get_or_init(|| {
        let pair = build_pair(None, &TlsOptions::default()).expect("默认 HTTP 客户端构建不应失败");
        std::sync::RwLock::new(pair)
    })
}

/// 全局共享的 reqwest::Client（克隆廉价，内部为 Arc 连接池）。
fn shared_client(follow_redirects: bool, use_cookies: bool) -> Result<Client, AppError> {
    let pair = client_pair().read().unwrap_or_else(|e| e.into_inner());
    Ok(match (follow_redirects, use_cookies) {
        (true, true) => pair.follow.clone(),
        (false, true) => pair.no_follow.clone(),
        (true, false) => pair.follow_no_cookies.clone(),
        (false, false) => pair.no_follow_no_cookies.clone(),
    })
}

/// 校验代理地址格式（不实际建立连接），供设置入口提前反馈。
pub fn validate_proxy(url: &str) -> Result<(), AppError> {
    reqwest::Proxy::all(url)
        .map(|_| ())
        .map_err(|e| AppError::Validation(format!("代理地址无效：{e}")))
}

/// 全局 TLS 选项：mTLS 客户端证书（rustls 栈 → 单个 PEM 文件含证书链 +
/// 私钥）与自签 / 无效证书放行（内网调试）。随代理一起触发共享客户端整体重建。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsOptions {
    /// 客户端证书 PEM 文件路径（证书链 + 私钥同文件；空 = 不使用客户端证书）。
    pub identity_path: Option<String>,
    /// 放行自签名 / 无效证书（内网自签服务调试）。
    pub accept_invalid_certs: bool,
}

/// 共享客户端当前配置（代理 + TLS）：`set_proxy` / `set_tls_options`
/// 读取对方现值整体重建，避免互相覆盖。
#[derive(Debug, Clone, Default)]
struct SharedClientConfig {
    proxy: Option<String>,
    tls: TlsOptions,
}

fn shared_config() -> &'static std::sync::RwLock<SharedClientConfig> {
    static CFG: OnceLock<std::sync::RwLock<SharedClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| std::sync::RwLock::new(SharedClientConfig::default()))
}

/// 设置 / 更换全局 HTTP 代理（`http://host:port` / `socks5://host:port`）。
///
/// `None` 表示直连（并忽略系统代理）。立即生效于后续请求；已建立的
/// 连接池随旧客户端一起丢弃。
pub fn set_proxy(proxy: Option<&str>) -> Result<(), AppError> {
    let mut cfg = shared_config().write().unwrap_or_else(|e| e.into_inner());
    cfg.proxy = proxy.map(String::from);
    let pair = build_pair(cfg.proxy.as_deref(), &cfg.tls)
        .map_err(|e| AppError::Validation(format!("HTTP 客户端初始化失败：{e}")))?;
    *client_pair().write().unwrap_or_else(|e| e.into_inner()) = pair;
    Ok(())
}

/// 设置 / 更换全局 TLS 选项（客户端证书 + 自签放行）。立即生效于后续请求。
pub fn set_tls_options(tls: &TlsOptions) -> Result<(), AppError> {
    let mut cfg = shared_config().write().unwrap_or_else(|e| e.into_inner());
    cfg.tls = tls.clone();
    let pair = build_pair(cfg.proxy.as_deref(), &cfg.tls)
        .map_err(|e| AppError::Validation(format!("HTTP 客户端初始化失败：{e}")))?;
    *client_pair().write().unwrap_or_else(|e| e.into_inner()) = pair;
    Ok(())
}

/// 读取当前 TLS 选项（设置页回显）。
pub fn tls_options() -> TlsOptions {
    shared_config()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .tls
        .clone()
}

/// 发送 HTTP 请求。
///
/// - `url` 应为已渲染（含变量替换与路径变量）的完整地址。
/// - `timeout_ms` 为超时毫秒数；None 时使用默认 300 秒。
///
/// 复用 [`shared_client`] 全局连接池；超时按请求设置，各请求互不影响，
/// 并发调用是安全的。
pub async fn send_request(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    timeout_ms: Option<u64>,
) -> Result<HttpResponseData, AppError> {
    send_request_inner(method, url, spec, timeout_ms, None, MAX_BODY_BYTES).await
}

///  capped 发送：响应体超过 `body_cap_bytes` 即停止读取并标记截断。
///
/// 压测等只关心状态码 / 耗时的场景用小上限（如 64KB），避免 500 并发 ×
/// 20MB 的内存爆炸；注意耗时口径变为"到截断为止"，调用方应在结果中说明。
pub async fn send_request_capped(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    timeout_ms: Option<u64>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    body_cap_bytes: usize,
) -> Result<HttpResponseData, AppError> {
    send_request_inner(
        method,
        url,
        spec,
        timeout_ms,
        cancel,
        body_cap_bytes.max(1024),
    )
    .await
}

/// 带取消能力的发送（用户点击「取消请求」时返回 [`AppError::Cancelled`]）。
///
/// 取消令牌由调用方持有（如 Tauri Command 层的请求注册表）；等待连接 /
/// 响应 / 读取响应体期间任一时刻取消，都会中止底层 reqwest 请求并返回
/// [`AppError::Cancelled`]。不可取消场景请使用 [`send_request`]。
pub async fn send_request_cancel(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    timeout_ms: Option<u64>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<HttpResponseData, AppError> {
    send_request_inner(method, url, spec, timeout_ms, Some(cancel), MAX_BODY_BYTES).await
}

async fn send_request_inner(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    timeout_ms: Option<u64>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    body_cap: usize,
) -> Result<HttpResponseData, AppError> {
    // gRPC 端点走 miyo-grpc 引擎（命令层分流），HTTP 引擎不受理
    if method.is_grpc() {
        return Err(AppError::Validation(
            "gRPC 端点不支持 HTTP 发送（应经 gRPC 引擎调用）".into(),
        ));
    }
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let client = shared_client(spec.follow_redirects, !spec.disable_cookies)?;

    let mut url = Url::parse(url).map_err(|e| AppError::Validation(format!("URL 无效：{e}")))?;

    // Query：先拼显式 params（签名类认证按最终 URL 计算，须在加签前就位）；
    // ApiKey(query) 附加项在加签后拼接（与 AWS 联用时不计入签名，属极小众组合）。
    append_query(&mut url, &spec.params);

    let payload = build_payload(spec).await?;
    // 加签上下文：Bytes 计算真实摘要；流式 body（multipart / binary / 大表单）
    // 无法预读，按 AWS S3 风格记 `UNSIGNED-PAYLOAD`；Hawk 无 body 时不带 hash。
    let (payload_hash, hawk_body) = match &payload {
        Payload::None => (miyo_sign::sha256_hex(&[]), None),
        Payload::Bytes(body, content_type) => (
            miyo_sign::sha256_hex(body),
            Some((
                content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
                body.as_slice(),
            )),
        ),
        Payload::Stream { .. } => ("UNSIGNED-PAYLOAD".to_string(), None),
    };

    let mut query_extra: Vec<KeyValue> = Vec::new();
    let mut headers: Vec<(String, String)> = spec
        .headers
        .iter()
        .filter(|kv| kv.enabled)
        .map(|kv| (kv.key.trim().to_string(), kv.value.clone()))
        .collect();
    let sign_ctx = AuthSignCtx {
        method,
        url: &url,
        payload_hash: &payload_hash,
        body: hawk_body,
    };
    apply_auth(&mut headers, &mut query_extra, &spec.auth, &sign_ctx).await?;
    append_query(&mut url, &query_extra);

    // 实际发出形态快照（前端「请求」页签）：URL / 头 / Cookie / 体均已定型。
    let request = build_request_snapshot(method, &url, &headers, &payload, spec.disable_cookies);

    // 签名阶段建好的 payload 复用为首轮请求体（原来 Digest 循环内再建一次：
    // 重复序列化 / 重复开文件）；仅 Digest 401 重发时才重建——流式 body
    //（文件流 / multipart）一次性不可复用，重建 = 重新打开，语义一致。
    let mut pending_payload = Some(payload);
    // 只有 Digest 认证可能重发（见下方 `digest_retry_header`）：非 Digest
    // 路径发完即结束，把 url move 进请求，省掉一次 Url 克隆。
    let is_digest = matches!(spec.auth, AuthSpec::Digest { .. });
    let mut url_slot: Option<Url> = Some(url);
    let start = std::time::Instant::now();
    // Digest 挑战-应答：首轮无凭据 → 401 携质询 → 补 Authorization 重发一次。
    let mut digest_authz: Option<String> = None;
    let resp: Response = loop {
        // 首轮 payload 直接取签名阶段那一份；重发轮（pending_payload 已被取走）
        // 才重新构建。
        let payload = match pending_payload.take() {
            Some(p) => p,
            None => build_payload(spec).await?,
        };
        // Digest 首轮发完还要用 url 算质询，只能克隆；非 Digest（或已重发过）
        // 是最后一轮，url 直接 move 进请求。
        let retry_possible = is_digest && digest_authz.is_none();
        let req_url = if retry_possible {
            url_slot.as_ref().expect("Digest 首轮保留 url").clone()
        } else {
            url_slot.take().expect("最后一轮 move url")
        };
        let mut req = client
            .request(reqwest_method(method), req_url)
            .timeout(Duration::from_millis(timeout_ms));

        for (k, v) in &headers {
            if k.is_empty() {
                continue;
            }
            req = req.header(k, v);
        }
        if let Some(authz) = &digest_authz {
            req = req.header("Authorization", authz);
        }
        // payload 之后不再使用，按值解构直接 move body，避免整包克隆
        match payload {
            Payload::None => {}
            Payload::Bytes(body, content_type) => {
                if let Some(ct) = &content_type {
                    if !has_header(&headers, "content-type") {
                        req = req.header("content-type", ct.as_str());
                    }
                }
                req = req.body(body);
            }
            Payload::Stream {
                body,
                content_type,
                content_length,
                ..
            } => {
                if let Some(ct) = &content_type {
                    if !has_header(&headers, "content-type") {
                        req = req.header("content-type", ct.as_str());
                    }
                }
                if let Some(len) = content_length {
                    if !has_header(&headers, "content-length") {
                        req = req.header("content-length", len);
                    }
                }
                req = req.body(body);
            }
        }

        let one: Response = match cancel {
            Some(token) => {
                let fut = req.send();
                tokio::select! {
                    // 取消分支胜出即丢弃请求 future：底层 hyper 连接随之中止
                    //（hyper 官方文档：丢弃返回的 future 是取消在途请求的支持方式）。
                    _ = token.cancelled() => {
                        return Err(AppError::Cancelled("request aborted by user".to_string()));
                    }
                    r = fut => r.map_err(AppError::from_reqwest)?,
                }
            }
            None => req.send().await.map_err(AppError::from_reqwest)?,
        };
        // 仅 Digest 首轮 401 且质询可解析时重试；算法不支持 / 密码错误等情况
        // 直接返回服务端响应（用户可见真实 401，而不是客户端报错）。
        // 非 Digest 路径 url 已 move 进请求，此处取不到 url 同样 break——
        // 与 digest_retry_header 对非 Digest 恒返回 None 等价。
        if digest_authz.is_none() {
            if let Some(authz) = url_slot
                .as_ref()
                .and_then(|u| digest_retry_header(&one, &spec.auth, method, u))
            {
                digest_authz = Some(authz);
                continue;
            }
        }
        break one;
    };

    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let cookies: Vec<CookieData> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| {
            let value = v.to_str().unwrap_or_default();
            let first = value.split(';').next()?.trim();
            let (name, value) = first.split_once('=')?;
            Some(CookieData {
                name: name.to_string(),
                value: value.to_string(),
            })
        })
        .collect();

    // 读取响应体，超过上限截断并标记后即丢弃后续字节（连接不复用，
    // 由调用方按场景选上限）；期间可被取消。
    let mut body: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut chunks = resp.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        if cancel.is_some_and(|t| t.is_cancelled()) {
            return Err(AppError::Cancelled("request aborted by user".to_string()));
        }
        let chunk = chunk.map_err(AppError::from_reqwest)?;
        if body.len() + chunk.len() > body_cap {
            let remaining = body_cap - body.len();
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }

    let size_bytes = body.len();
    // 耗时口径：含响应体下载（与 Postman 一致）——大文件场景只计到响应头
    // 会把 30s 的下载显示成 80ms，严重误导。
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(HttpResponseData {
        status,
        headers,
        body: Bytes::from(body),
        duration_ms,
        size_bytes,
        cookies,
        truncated,
        request: Some(request),
    })
}

/// 把 reqwest 错误翻译为面向用户的中文提示（DNS / 超时 / TLS / 连接失败）。
///
/// 分类逻辑统一收敛在 [`AppError::classify`]，此处仅取其用户消息。
pub fn describe_http_error(e: &reqwest::Error) -> String {
    AppError::classify(e)
        .map(|err| err.user_message())
        .unwrap_or_else(|| "请求失败：服务端未返回有效响应".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::RequestSpec;

    /// 快照必须反映「实际发出」的形态：自动 Content-Type、渲染后的 URL、体预览。
    #[test]
    fn request_snapshot_reflects_effective_request() {
        let url = Url::parse("https://api.example.com/users?page=2").unwrap();
        let headers = vec![("X-Token".to_string(), "tok".to_string())];
        let payload = Payload::Bytes(b"{\"a\":1}".to_vec(), Some("application/json".into()));
        let snap = build_request_snapshot(HttpMethod::POST, &url, &headers, &payload, true);
        assert_eq!(snap.method, "POST");
        assert_eq!(snap.url, "https://api.example.com/users?page=2");
        assert!(snap
            .headers
            .contains(&("X-Token".to_string(), "tok".to_string())));
        assert!(snap
            .headers
            .contains(&("Content-Type".to_string(), "application/json".to_string())));
        // 自动值不得重复：同名头只出现一次
        assert_eq!(
            snap.headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .count(),
            1
        );
        assert!(snap.cookies.is_empty()); // disable_cookies 分支：不读 Cookie Jar
        assert_eq!(snap.body, "{\"a\":1}");
        assert!(!snap.body_omitted);
        assert!(snap.url_template.is_none()); // 模板由调用方（Tauri 层）填入
    }

    /// 超长体只留 64 KB 预览并标记截断；流式体给摘要，不落地整段内容。
    #[test]
    fn request_snapshot_truncates_body_and_summarizes_stream() {
        let url = Url::parse("https://api.example.com/upload").unwrap();
        let big = vec![b'x'; SNAPSHOT_BODY_BYTES + 32];
        let snap = build_request_snapshot(
            HttpMethod::POST,
            &url,
            &[],
            &Payload::Bytes(big, None),
            true,
        );
        assert!(snap.body_omitted);
        assert_eq!(snap.body.len(), SNAPSHOT_BODY_BYTES);

        let stream = Payload::Stream {
            body: reqwest::Body::from("abc".to_string()),
            content_type: Some("multipart/form-data".into()),
            content_length: Some(1234),
            preview: None,
        };
        let snap = build_request_snapshot(HttpMethod::POST, &url, &[], &stream, true);
        assert!(snap.body_omitted);
        assert!(snap.body.contains("1234"));
        assert!(snap
            .headers
            .contains(&("Content-Length".to_string(), "1234".to_string())));
    }

    /// 无文件的 form-data（纯文本字段）必须能看清实际参数，而不是流式摘要：
    /// multipart 体在构造阶段就把可读形态存进快照。
    #[tokio::test]
    async fn multipart_snapshot_lists_text_fields() {
        let spec = RequestSpec {
            body: BodySpec::Multipart {
                fields: vec![
                    MultipartField {
                        key: "id".into(),
                        value_type: MultipartValueType::Text,
                        value: "88880034".into(),
                        enabled: true,
                    },
                    MultipartField {
                        key: "status".into(),
                        value_type: MultipartValueType::Text,
                        value: "2".into(),
                        enabled: true,
                    },
                    // 禁用字段既不发送也不入快照
                    MultipartField {
                        key: "off".into(),
                        value_type: MultipartValueType::Text,
                        value: "skip".into(),
                        enabled: false,
                    },
                ],
            },
            ..Default::default()
        };
        let payload = build_payload(&spec).await.unwrap();
        let url = Url::parse("https://cloud.example.cn/index.php?c=book&a=togglestatus").unwrap();
        let snap = build_request_snapshot(HttpMethod::POST, &url, &[], &payload, true);
        assert_eq!(snap.body, "id=88880034\nstatus=2", "实际字段应逐个可见");
        assert!(!snap.body_omitted);
        assert!(
            !snap.body.contains("边读边发"),
            "文本表单不应退化为流式摘要"
        );
        // boundary 仍在 Content-Type 上（服务端需要），但不再当成正文主体
        assert!(snap
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v.starts_with("multipart/form-data; boundary=")));
    }

    /// 极简本地 HTTP 服务。
    fn start_server(
        handler: impl Fn(&str, &str) -> (u16, String, String) + Send + 'static,
    ) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                // 流式上传分多次写：循环读到完整请求（chunked 读到终止块，
                // 定长读到声明长度），单次 read 在流式场景下必然截断。
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut raw: Vec<u8> = Vec::new();
                let mut buf = [0u8; 8192];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(hend) = text.find("\r\n\r\n") {
                        let head = text[..hend].to_ascii_lowercase();
                        if head.contains("transfer-encoding: chunked") {
                            if text.contains("\r\n0\r\n\r\n") {
                                break;
                            }
                        } else if let Some(len) = head.lines().find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            if k.trim() == "content-length" {
                                v.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        }) {
                            if raw.len() >= hend + 4 + len {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    if raw.len() > 4 * 1024 * 1024 {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&raw).to_string();
                let head = request.split("\r\n").next().unwrap_or("").to_string();
                let (status, ctype, body) = handler(&head, &request);
                // Digest 端到端用例：401 自动附带质询头（其余测试只用 200/201，不受影响）。
                let challenge = if status == 401 {
                    "WWW-Authenticate: Digest realm=\"test\", nonce=\"abc123\", qop=\"auth\", algorithm=MD5\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\n{challenge}Content-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    use std::io::{Read, Write};

    /// 还原请求载荷原文供断言：chunked 传输去分帧，
    /// 定长 body（Content-Length）按长度截取（忽略 8KB 读缓冲的零填充）。
    fn dechunk(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        let head_end = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let head = &raw[..head_end];
        let mut rest = &raw[head_end..];
        if !head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            // 定长：按声明长度截取（若缺失则整体返回）。
            let len = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("content-length") {
                        v.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(rest.len());
            // from_utf8_lossy 下多字节字符不会被长度截断劈裂到 panic：
            // 按字符边界安全截取。
            let mut acc = 0usize;
            for (i, ch) in rest.char_indices() {
                if acc + ch.len_utf8() > len {
                    return rest[..i].to_string();
                }
                acc += ch.len_utf8();
            }
            return rest.to_string();
        }
        loop {
            let Some(line_end) = rest.find("\r\n") else {
                out.push_str(rest);
                break;
            };
            let size = usize::from_str_radix(rest[..line_end].trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            rest = &rest[line_end + 2..];
            let take = size.min(rest.len());
            out.push_str(&rest[..take]);
            rest = &rest[take..];
            rest = rest.strip_prefix("\r\n").unwrap_or(rest);
        }
        out
    }

    #[tokio::test]
    async fn send_get_with_query() {
        let base = start_server(|head, _| {
            assert!(head.starts_with("GET /echo?a=1&b=hello"));
            (200, "text/plain".to_string(), "ok".to_string())
        });
        let spec = RequestSpec {
            params: vec![KeyValue::new("a", "1"), KeyValue::new("b", "hello"), {
                let mut kv = KeyValue::new("off", "x");
                kv.enabled = false;
                kv
            }],
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/echo"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body_text(), "ok");
        assert_eq!(resp.size_bytes, 2);
    }

    #[tokio::test]
    async fn send_post_json() {
        let base = start_server(|head, request| {
            assert!(head.starts_with("POST /data"));
            assert!(request.contains("content-type: application/json"));
            assert!(request.contains("{\"a\":1}"));
            (
                201,
                "application/json".to_string(),
                "{\"ok\":true}".to_string(),
            )
        });
        let spec = RequestSpec {
            body: BodySpec::Json {
                raw: "{\"a\":1}".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::POST, &format!("{base}/data"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.content_type(), "application/json");
    }

    // ---------- OAuth2：Bearer 头 + 过期静默刷新 ----------

    /// 本地 OAuth2 服务：/token 返回新令牌；其余路径断言 Authorization == 期望值。
    fn oauth_server(
        expected_bearer: &'static str,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let refresh_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = refresh_count.clone();
        let base = start_server(move |head, request| {
            if head.contains("/token") {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert!(head.starts_with("POST /token"), "HEAD: {head}");
                assert!(request.contains("grant_type=refresh_token"));
                assert!(request.contains("refresh_token=rt-1"));
                return (
                    200,
                    "application/json".to_string(),
                    "{\"access_token\":\"refreshed-1\",\"token_type\":\"Bearer\",\
                     \"refresh_token\":\"rt-2\",\"expires_in\":3600}"
                        .to_string(),
                );
            }
            let auth = request
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                .map(|l| l.to_string())
                .unwrap_or_default();
            assert!(
                auth.contains(expected_bearer),
                "期望 Authorization 含 {expected_bearer}，实际：{auth}"
            );
            (200, "text/plain".to_string(), "ok".to_string())
        });
        (base, refresh_count)
    }

    fn oauth_spec(expires_in_secs: i64, refresh_token: Option<&str>) -> RequestSpec {
        RequestSpec {
            auth: AuthSpec::OAuth2 {
                client_id: "e2e-client".into(),
                client_secret: "e2e-secret".into(),
                auth_url: "https://idp.example/authorize".into(),
                token_url: String::new(), // 测试中动态填入
                scope: "read".into(),
                redirect_uri: "http://127.0.0.1:9090/callback".into(),
                grant: miyo_core::model::OAuth2Grant::default(),
                username: None,
                password: None,
                token: Some(miyo_core::model::OAuth2Token {
                    access_token: "stale-token".into(),
                    token_type: "Bearer".into(),
                    refresh_token: refresh_token.map(String::from),
                    expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_in_secs),
                }),
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn oauth2_expired_token_auto_refreshes() {
        let (base, counter) = oauth_server("Bearer refreshed-1");
        let mut spec = oauth_spec(-10, Some("rt-1"));
        if let AuthSpec::OAuth2 { token_url, .. } = &mut spec.auth {
            *token_url = format!("{base}/token");
        }
        let resp = send_request(HttpMethod::GET, &format!("{base}/echo"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "过期 token 应触发一次刷新"
        );
        // 刷新后的 token 已入缓存 → 第二次请求不再刷新。
        let resp2 = send_request(HttpMethod::GET, &format!("{base}/echo"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp2.status, 200);
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "有效 token 不应重复刷新"
        );
    }

    #[tokio::test]
    async fn oauth2_valid_token_uses_bearer_without_refresh() {
        let (base, counter) = oauth_server("Bearer stale-token");
        let mut spec = oauth_spec(3600, Some("rt-1"));
        if let AuthSpec::OAuth2 { token_url, .. } = &mut spec.auth {
            *token_url = format!("{base}/token");
        }
        let resp = send_request(HttpMethod::GET, &format!("{base}/echo"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "有效 token 不应触发刷新"
        );
    }

    #[tokio::test]
    async fn oauth2_unauthorized_reports_error() {
        let (base, _counter) = oauth_server("Bearer unused");
        let spec = RequestSpec {
            auth: AuthSpec::OAuth2 {
                client_id: "u-client".into(),
                client_secret: "s".into(),
                auth_url: "https://idp.example/authorize".into(),
                token_url: format!("{base}/token"),
                scope: String::new(),
                redirect_uri: String::new(),
                grant: miyo_core::model::OAuth2Grant::default(),
                username: None,
                password: None,
                token: None,
            },
            ..Default::default()
        };
        let err = send_request(HttpMethod::GET, &format!("{base}/echo"), &spec, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::OAuth2(_)),
            "应映射为 OAuth2 错误：{err}"
        );
        assert!(err.user_message().contains("立即授权"));
    }

    #[tokio::test]
    async fn send_basic_auth() {
        let base = start_server(|_, request| {
            let expect = base64::engine::general_purpose::STANDARD.encode("u:p");
            let line = request
                .lines()
                .find(|l| l.to_lowercase().starts_with("authorization"))
                .unwrap_or_default()
                .to_string();
            assert_eq!(
                line.to_lowercase(),
                format!("authorization: basic {}", expect.to_lowercase())
            );
            (200, "text/plain".to_string(), "auth-ok".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::Basic {
                username: "u".into(),
                password: "p".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    // ---------- 签名类认证：Hawk / AWS SigV4 / HMAC / Digest ----------

    /// 取请求原文中指定头的值（hyper 发出的头名为小写，此处大小写不敏感）。
    fn auth_header_of(request: &str, name: &str) -> String {
        request
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn send_hawk_auth_header() {
        let base = start_server(|_, request| {
            let value = auth_header_of(request, "authorization");
            assert!(value.starts_with("Hawk "), "意外：{value}");
            assert!(value.contains("id=\"kid\""), "缺 id：{value}");
            assert!(value.contains("mac=\""), "缺 mac：{value}");
            (200, "text/plain".to_string(), "hawk-ok".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::Hawk {
                key_id: "kid".into(),
                key: "k".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/r"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn send_aws_v4_auth_headers() {
        let base = start_server(|_, request| {
            let auth = auth_header_of(request, "authorization");
            assert!(
                auth.starts_with("AWS4-HMAC-SHA256 Credential=AK/20"),
                "意外：{auth}"
            );
            assert!(
                auth.contains("SignedHeaders=host;x-amz-date"),
                "缺 SignedHeaders：{auth}"
            );
            assert!(auth.contains("Signature="), "缺 Signature：{auth}");
            let date = auth_header_of(request, "x-amz-date");
            assert!(!date.is_empty(), "缺 x-amz-date");
            (200, "text/plain".to_string(), "aws-ok".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::AwsV4 {
                access_key: "AK".into(),
                secret_key: "SK".into(),
                region: "us-east-1".into(),
                service: "iam".into(),
                session_token: None,
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/?A=1"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn send_hmac_aksk_headers() {
        let base = start_server(|_, request| {
            let lower = request.to_ascii_lowercase();
            assert!(lower.contains("x-access-key: ak-1"), "缺 X-Access-Key");
            assert!(lower.contains("x-timestamp: "), "缺 X-Timestamp");
            assert!(lower.contains("x-nonce: "), "缺 X-Nonce");
            let sig = auth_header_of(request, "x-signature");
            assert_eq!(sig.len(), 64, "签名应为 64 位 hex：{sig}");
            assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
            (200, "text/plain".to_string(), "hmac-ok".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::Hmac {
                access_key: "ak-1".into(),
                secret_key: "sk-1".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::POST, &format!("{base}/sign"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn digest_auth_retries_after_challenge() {
        let unauthorized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = unauthorized.clone();
        let base = start_server(move |_, request| {
            let value = auth_header_of(request, "authorization");
            if value.starts_with("Digest ") {
                assert!(value.contains("username=\"u\""), "缺 username：{value}");
                assert!(
                    value.contains("nonce=\"abc123\""),
                    "应答质询 nonce：{value}"
                );
                assert!(value.contains("response=\""), "缺 response：{value}");
                assert!(value.contains("nc=00000001"), "缺 nc：{value}");
                (200, "text/plain".to_string(), "digest-ok".to_string())
            } else {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (401, "text/plain".to_string(), "unauthorized".to_string())
            }
        });
        let spec = RequestSpec {
            auth: AuthSpec::Digest {
                username: "u".into(),
                password: "p".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/private"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body_text(), "digest-ok");
        assert_eq!(
            unauthorized.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "应恰好经历一次 401 质询"
        );
    }

    #[tokio::test]
    async fn digest_wrong_password_returns_401() {
        // 质询不可复用固定 response：服务端校验失败 → 第二次仍 401，直接返回响应。
        let base = start_server(|_, request| {
            let value = auth_header_of(request, "authorization");
            if value.starts_with("Digest ") {
                // 故意不认可任何应答（模拟密码错误）。
                return (401, "text/plain".to_string(), "bad".to_string());
            }
            (401, "text/plain".to_string(), "unauthorized".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::Digest {
                username: "u".into(),
                password: "wrong".into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/private"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 401);
    }

    #[tokio::test]
    async fn send_dynamic_signature() {
        // 验证签名三头注入：Key / Timestamp / Sig，且 Sig = MD5(Key+Secret+Timestamp)。
        let base = start_server(|_, request| {
            let line = |prefix: &str| {
                request
                    .lines()
                    .find(|l| l.to_lowercase().starts_with(&format!("{prefix}: ")))
                    .unwrap_or_default()
                    .to_string()
            };
            let key = line("app-key").replacen("app-key: ", "", 1);
            let ts = line("app-timestamp").replacen("app-timestamp: ", "", 1);
            let sig = line("app-sig").replacen("app-sig: ", "", 1);
            assert_eq!(key, "app-123");
            // 时间戳为当前毫秒（发送前最后一刻生成）。
            let diff = chrono::Utc::now().timestamp_millis() - ts.parse::<i64>().unwrap();
            assert!(diff.abs() < 5_000, "时间戳应实时生成，偏差 {diff}ms");
            // 签名 = MD5(Key+Secret+Timestamp) 的 hex 小写。
            let mut h = md5::Md5::new();
            use sha2::Digest as _;
            h.update(format!("{key}sec-456{ts}").as_bytes());
            let expected: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(sig, expected, "Sig 必须等于 MD5(Key+Secret+Timestamp)");
            (200, "text/plain".to_string(), "sig-ok".to_string())
        });
        let spec = RequestSpec {
            auth: AuthSpec::DynamicSignature {
                config: miyo_core::model::DynamicSignatureConfig {
                    app_key: "app-123".into(),
                    app_secret: "sec-456".into(),
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::GET, &format!("{base}/"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn send_urlencoded() {
        let base = start_server(|_, request| {
            assert!(request.contains("content-type: application/x-www-form-urlencoded"));
            assert!(request.contains("k=v"));
            (200, "text/plain".to_string(), "form".to_string())
        });
        let spec = RequestSpec {
            body: BodySpec::UrlEncoded {
                fields: vec![KeyValue::new("k", "v")],
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::PUT, &format!("{base}/form"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn connection_refused_reports_error() {
        let spec = RequestSpec::default();
        let err = send_request(HttpMethod::GET, "http://127.0.0.1:1/", &spec, Some(3000)).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn connection_refused_mapped_to_chinese_hint() {
        let spec = RequestSpec::default();
        let err = send_request(HttpMethod::GET, "http://127.0.0.1:1/", &spec, Some(3000))
            .await
            .unwrap_err();
        match err {
            AppError::Connection(_) => (),
            other => panic!("应映射为 Connection 错误：{other}"),
        }
        assert!(err.user_message().contains("连接失败"));
    }

    #[tokio::test]
    async fn dns_failure_mapped_to_dns_variant() {
        let spec = RequestSpec::default();
        // .invalid 顶级域按 RFC 2606 保证不解析。
        let err = send_request(
            HttpMethod::GET,
            "http://miyo-nonexistent.invalid/",
            &spec,
            Some(3000),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Dns(_)), "应映射为 DNS 错误：{err}");
        assert!(err.user_message().contains("DNS 解析失败"));
    }

    #[tokio::test]
    async fn invalid_url_mapped_to_validation_variant() {
        let spec = RequestSpec::default();
        let err = send_request(HttpMethod::GET, "not a url", &spec, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Validation(_)),
            "应映射为 Validation：{err}"
        );
    }

    #[tokio::test]
    async fn timeout_is_applied() {
        // 服务端不响应：读请求后挂起。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                // 不写响应，挂起直到客户端超时断开。
                std::thread::sleep(std::time::Duration::from_secs(5));
                let _ = stream.write_all(b"");
            }
        });
        let spec = RequestSpec::default();
        let start = std::time::Instant::now();
        let err = send_request(
            HttpMethod::GET,
            &format!("http://{addr}/slow"),
            &spec,
            Some(500),
        )
        .await;
        assert!(err.is_err());
        assert!(start.elapsed().as_millis() < 3000);
        if let Err(err) = err {
            assert!(
                matches!(err, AppError::NetworkTimeout(_)),
                "应映射为超时错误：{err}"
            );
            assert!(err.user_message().contains("超时"));
        }
    }

    #[tokio::test]
    async fn body_truncated_at_limit() {
        let big = "x".repeat(MAX_BODY_BYTES + 100);
        let base = start_server(move |_, _| (200, "text/plain".to_string(), big.clone()));
        let spec = RequestSpec::default();
        let resp = send_request(HttpMethod::GET, &format!("{base}/big"), &spec, None)
            .await
            .unwrap();
        assert!(resp.truncated);
        assert_eq!(resp.size_bytes, MAX_BODY_BYTES);
    }

    #[tokio::test]
    async fn concurrent_requests_share_client() {
        let base = start_server(|_, _| (200, "text/plain".to_string(), "ok".to_string()));
        let spec = RequestSpec::default();
        let url = format!("{base}/concurrent");
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let spec = spec.clone();
            let url = url.clone();
            tasks.push(tokio::spawn(async move {
                let resp = send_request(HttpMethod::GET, &url, &spec, None)
                    .await
                    .unwrap();
                assert_eq!(resp.status, 200);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
    }

    #[tokio::test]
    async fn payload_builds_correctly() {
        let spec = RequestSpec {
            body: BodySpec::UrlEncoded {
                fields: vec![KeyValue::new("a", "1"), KeyValue::new("b", "x y"), {
                    let mut kv = KeyValue::new("off", "z");
                    kv.enabled = false;
                    kv
                }],
            },
            ..Default::default()
        };
        let payload = build_payload(&spec).await.unwrap();
        match payload {
            Payload::Bytes(body, ct) => {
                assert_eq!(ct.as_deref(), Some("application/x-www-form-urlencoded"));
                assert_eq!(String::from_utf8(body).unwrap(), "a=1&b=x+y");
            }
            _ => panic!("期望 UrlEncoded payload"),
        }
    }

    #[test]
    fn graphql_json_builds_post_payload() {
        let spec = GraphQLSpec {
            query: "query Hero($id: ID!) { hero(id: $id) { name } }".into(),
            variables: "{\"id\":\"42\"}".into(),
            operation_name: "Hero".into(),
        };
        let json = graphql_request_json(&spec, &VariableMap::new()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["query"],
            "query Hero($id: ID!) { hero(id: $id) { name } }"
        );
        assert_eq!(parsed["variables"]["id"], "42");
        assert_eq!(parsed["operationName"], "Hero");
    }

    #[test]
    fn graphql_json_omits_empty_variables_and_operation() {
        let spec = GraphQLSpec::default();
        let json = graphql_request_json(&spec, &VariableMap::new()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("variables").is_none());
        assert!(parsed.get("operationName").is_none());
        assert_eq!(parsed["query"], "");
    }

    #[test]
    fn graphql_json_interpolates_variables() {
        let spec = GraphQLSpec {
            query: "query { hero(id: {{id}}) { name } }".into(),
            variables: "{\"id\":\"{{hero_id}}\"}".into(),
            operation_name: String::new(),
        };
        let mut vars = VariableMap::new();
        vars.insert("id".into(), "7".into());
        vars.insert("hero_id".into(), "99".into());
        let json = graphql_request_json(&spec, &vars).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["query"].as_str().unwrap().contains("hero(id: 7)"));
        assert_eq!(parsed["variables"]["id"], "99");
    }

    #[test]
    fn graphql_json_rejects_invalid_variables() {
        let spec = GraphQLSpec {
            query: "query { a }".into(),
            variables: "not-json".into(),
            operation_name: String::new(),
        };
        let err = graphql_request_json(&spec, &VariableMap::new()).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn graphql_response_parses_data_and_errors() {
        let raw = br#"{"data":{"hero":{"name":"R2-D2"}},"errors":[{"message":"oops","locations":[{"line":1,"column":3}],"path":["hero","name"]}]}"#;
        let resp = parse_graphql_response(raw).unwrap();
        assert_eq!(resp.data.as_ref().unwrap()["hero"]["name"], "R2-D2");
        assert!(resp.has_errors());
        assert_eq!(resp.errors[0].message, "oops");
        assert_eq!(resp.errors[0].locations.as_ref().unwrap()[0].line, 1);
        assert_eq!(resp.errors[0].path.as_ref().unwrap()[0], "hero");
    }

    #[test]
    fn graphql_response_without_errors_has_empty_list() {
        let raw = br#"{"data":{"ok":true}}"#;
        let resp = parse_graphql_response(raw).unwrap();
        assert!(!resp.has_errors());
        assert_eq!(resp.data.as_ref().unwrap()["ok"], true);
    }

    #[test]
    fn graphql_response_rejects_non_json() {
        let err = parse_graphql_response(b"<html>error</html>").unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn multipart_text_payload_has_boundary() {
        let spec = RequestSpec {
            body: BodySpec::Multipart {
                fields: vec![
                    MultipartField {
                        key: "note".into(),
                        value_type: MultipartValueType::Text,
                        value: "hello".into(),
                        enabled: true,
                    },
                    MultipartField {
                        key: "off".into(),
                        value_type: MultipartValueType::Text,
                        value: "skip".into(),
                        enabled: false,
                    },
                ],
            },
            ..Default::default()
        };
        // 纯文本表单：流式载荷（文件 part 边读边发，大表单不物化），
        // content-type 仍携带 boundary；正文内容由端到端用例校验。
        let payload = build_payload(&spec).await.unwrap();
        match payload {
            Payload::Stream {
                content_type,
                content_length,
                ..
            } => {
                let ct = content_type.unwrap();
                assert!(
                    ct.starts_with("multipart/form-data; boundary="),
                    "意外 content-type：{ct}"
                );
                assert_eq!(content_length, None, "流式表单总长度未知");
            }
            _ => panic!("期望 Multipart 流式 payload"),
        }
    }

    #[tokio::test]
    async fn send_multipart_with_file_upload() {
        let dir = std::env::temp_dir();
        let file_path = dir.join("miyo_http_upload_test.txt");
        tokio::fs::write(&file_path, b"hello multipart file")
            .await
            .unwrap();
        let file_path_str = file_path.to_str().unwrap().to_string();

        let base = start_server(|_, request| {
            assert!(request.contains("content-type: multipart/form-data; boundary="));
            // 流式上传走 chunked：去分帧后再断言载荷原文。
            let body = dechunk(request);
            assert!(body.contains("name=\"note\""), "缺文本字段：{body}");
            assert!(body.contains("hello from text"), "缺文本内容");
            assert!(body.contains("name=\"file\""), "缺文件字段");
            assert!(
                body.contains("filename=\"miyo_http_upload_test.txt\""),
                "缺文件名"
            );
            assert!(body.contains("hello multipart file"), "缺文件内容");
            (200, "text/plain".to_string(), "uploaded".to_string())
        });
        let spec = RequestSpec {
            body: BodySpec::Multipart {
                fields: vec![
                    MultipartField {
                        key: "note".into(),
                        value_type: MultipartValueType::Text,
                        value: "hello from text".into(),
                        enabled: true,
                    },
                    MultipartField {
                        key: "file".into(),
                        value_type: MultipartValueType::FilePath,
                        value: file_path_str,
                        enabled: true,
                    },
                ],
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::POST, &format!("{base}/upload"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body_text(), "uploaded");
        tokio::fs::remove_file(&file_path).await.unwrap();
    }

    #[tokio::test]
    async fn multipart_missing_file_reports_error() {
        let spec = RequestSpec {
            body: BodySpec::Multipart {
                fields: vec![MultipartField {
                    key: "file".into(),
                    value_type: MultipartValueType::FilePath,
                    value: "/nonexistent/miyo_http_missing.txt".into(),
                    enabled: true,
                }],
            },
            ..Default::default()
        };
        let err = build_payload(&spec).await.unwrap_err();
        match err {
            AppError::Validation(msg) => assert!(msg.contains("读取文件"), "意外提示：{msg}"),
            other => panic!("非 Validation 错误：{other}"),
        }
    }

    // ---------- Binary：文件原始字节作为请求体 ----------

    #[tokio::test]
    async fn binary_payload_reads_file_bytes() {
        let dir = std::env::temp_dir();
        let file_path = dir.join("miyo_http_binary_test.bin");
        tokio::fs::write(&file_path, b"\x00\x01raw-bytes\xff")
            .await
            .unwrap();
        let spec = RequestSpec {
            body: BodySpec::Binary {
                path: file_path.to_str().unwrap().into(),
            },
            ..Default::default()
        };
        // binary 走文件流：不断言字节内容（端到端用例覆盖），
        // 只校验流式变体 + 已知长度 + 默认 content-type。
        let payload = build_payload(&spec).await.unwrap();
        match payload {
            Payload::Stream {
                content_type,
                content_length,
                ..
            } => {
                assert_eq!(content_type.as_deref(), Some("application/octet-stream"));
                assert_eq!(content_length, Some(12), "应为文件实际长度");
            }
            _ => panic!("期望 Binary 流式 payload"),
        }
        tokio::fs::remove_file(&file_path).await.unwrap();
    }

    // ---------- Cookie Jar：自动回放 + 单请求禁用 ----------

    #[tokio::test]
    async fn cookie_jar_replays_and_disable_cookies_skips() {
        use std::sync::{Arc, Mutex};
        // 首个请求下发 Set-Cookie；第二个请求应自动携带 Cookie。
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                let request = String::from_utf8_lossy(&buf).to_string();
                seen2.lock().unwrap().push(request.clone());
                let cookie_line = request
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("cookie:"))
                    .unwrap_or("")
                    .to_string();
                let body = format!("got:{cookie_line}");
                let response = format!(
                    "HTTP/1.1 200\r\nContent-Type: text/plain\r\nSet-Cookie: sid=abc123; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let spec = RequestSpec::default();
        let url = format!("http://{addr}/login");
        send_request(HttpMethod::GET, &url, &spec, None)
            .await
            .unwrap();
        let second = send_request(HttpMethod::GET, &url, &spec, None)
            .await
            .unwrap();
        assert!(
            second.body_text().contains("sid=abc123"),
            "第二次请求应自动回放 Cookie，实际：{}",
            second.body_text()
        );

        // 单请求禁用：jar 不附带（服务端看到无 Cookie 头）。
        let disabled = RequestSpec {
            disable_cookies: true,
            ..Default::default()
        };
        let third = send_request(HttpMethod::GET, &url, &disabled, None)
            .await
            .unwrap();
        assert!(
            third.body_text() == "got:",
            "禁用后不应附带 Cookie，实际：{}",
            third.body_text()
        );
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 3);
    }

    #[tokio::test]
    async fn send_binary_streams_file_with_length() {
        let dir = std::env::temp_dir();
        let file_path = dir.join("miyo_http_binary_e2e.bin");
        tokio::fs::write(&file_path, b"\x00\x01raw-bytes\xff")
            .await
            .unwrap();
        let base = start_server(|_, request| {
            // binary 单文件已知长度：应带 Content-Length（非 chunked）。
            assert!(
                request.contains("content-length: 12"),
                "应声明长度：{request}"
            );
            let body = dechunk(request);
            assert!(body.contains("raw-bytes"), "文件字节应原样送达");
            (200, "text/plain".to_string(), "ok".to_string())
        });
        let spec = RequestSpec {
            body: BodySpec::Binary {
                path: file_path.to_str().unwrap().into(),
            },
            ..Default::default()
        };
        let resp = send_request(HttpMethod::POST, &format!("{base}/bin"), &spec, None)
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        tokio::fs::remove_file(&file_path).await.unwrap();
    }

    #[tokio::test]
    async fn binary_missing_file_reports_error() {
        let spec = RequestSpec {
            body: BodySpec::Binary {
                path: "/nonexistent/miyo_http_binary_missing.bin".into(),
            },
            ..Default::default()
        };
        let err = build_payload(&spec).await.unwrap_err();
        match err {
            AppError::Validation(msg) => assert!(msg.contains("读取文件"), "意外提示：{msg}"),
            other => panic!("非 Validation 错误：{other}"),
        }
    }

    #[tokio::test]
    async fn auth_api_key_query_appends() {
        let mut headers = Vec::new();
        let mut query = Vec::new();
        let url = Url::parse("https://api.example.com/v1").unwrap();
        let empty_hash = miyo_sign::sha256_hex(&[]);
        let ctx = AuthSignCtx {
            method: HttpMethod::GET,
            url: &url,
            payload_hash: &empty_hash,
            body: None,
        };
        apply_auth(
            &mut headers,
            &mut query,
            &AuthSpec::ApiKey {
                key: "apikey".into(),
                value: "secret".into(),
                location: ApiKeyLocation::Query,
            },
            &ctx,
        )
        .await
        .unwrap();
        assert!(headers.is_empty());
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].key, "apikey");
        assert_eq!(query[0].value, "secret");
    }
}
