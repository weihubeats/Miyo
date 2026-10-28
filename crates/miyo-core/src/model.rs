//! 领域模型：与 SPEC 第 8~12 节保持一致。

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;

/// 请求方法（HTTP 七种 + Grpc 独立协议维度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
    /// gRPC 调试端点（独立协议，不走 HTTP 引擎；不入 all() 方法下拉，
    /// 由前端方法下拉单独提供入口，历史/徽标展示走 all_with_grpc）。
    Grpc,
}

impl HttpMethod {
    /// HTTP 专用方法列表（方法下拉数据源；不含 Grpc——gRPC 是独立协议维度）。
    pub fn all() -> &'static [HttpMethod] {
        &[
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
        ]
    }

    /// 全量方法（含 Grpc；历史/徽标等展示场景用）。
    pub fn all_with_grpc() -> &'static [HttpMethod] {
        &[
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
            HttpMethod::Grpc,
        ]
    }

    /// 是否为 gRPC 端点（执行链路据此分流到 miyo-grpc）。
    pub fn is_grpc(&self) -> bool {
        matches!(self, HttpMethod::Grpc)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::Grpc => "GRPC",
        }
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HttpMethod {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(HttpMethod::GET),
            "POST" => Ok(HttpMethod::POST),
            "PUT" => Ok(HttpMethod::PUT),
            "DELETE" => Ok(HttpMethod::DELETE),
            "PATCH" => Ok(HttpMethod::PATCH),
            "HEAD" => Ok(HttpMethod::HEAD),
            "OPTIONS" => Ok(HttpMethod::OPTIONS),
            "GRPC" => Ok(HttpMethod::Grpc),
            other => Err(AppError::Validation(format!("不支持的 HTTP 方法：{other}"))),
        }
    }
}

/// 接口状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EndpointStatus {
    Designing,
    #[default]
    Developing,
    Testing,
    Released,
    Deprecated,
}

impl EndpointStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EndpointStatus::Designing => "designing",
            EndpointStatus::Developing => "developing",
            EndpointStatus::Testing => "testing",
            EndpointStatus::Released => "released",
            EndpointStatus::Deprecated => "deprecated",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            EndpointStatus::Designing => "设计中",
            EndpointStatus::Developing => "开发中",
            EndpointStatus::Testing => "测试中",
            EndpointStatus::Released => "已发布",
            EndpointStatus::Deprecated => "已废弃",
        }
    }
}

/// 参数字段类型（接口设计页的 Schema 标注；旧数据缺省为 String）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    String,
    Number,
    Boolean,
    Object,
}

/// 键值对：用于 Query / Header / Path 变量等。
///
/// `field_type` / `required` / `example` 为接口设计的参数元数据，
/// serde 带默认值：旧持久化 JSON 缺省这些字段时照常反序列化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub field_type: FieldType,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub example: String,
}

fn default_fault_status() -> u16 {
    500
}

fn default_true() -> bool {
    true
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        KeyValue {
            key: key.into(),
            value: value.into(),
            enabled: true,
            description: String::new(),
            field_type: FieldType::default(),
            required: true,
            example: String::new(),
        }
    }
}

/// API Key 放置位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// 签名算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
// 显式 rename：`rename_all = "snake_case"` 会把连续大写当作单词边界，
// 导致 `MD5`→`m_d5`、`SHA256`→`s_h_a256`、`HmacSHA256`→`hmac_s_h_a256`，
// 与前端传参 `md5` / `sha256` / `hmac_sha256` 不匹配（IPC 报 unknown variant）。
pub enum SignatureAlgorithm {
    #[default]
    #[serde(rename = "md5")]
    MD5,
    #[serde(rename = "sha256")]
    SHA256,
    /// HMAC-SHA256：`app_secret` 作为 HMAC 密钥，载荷为被签消息。
    #[serde(rename = "hmac_sha256")]
    HmacSHA256,
}

/// 摘要字节 → 传输字符串的编码方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SignatureEncoding {
    #[default]
    HexLower,
    HexUpper,
    Base64,
}

/// 动态签名鉴权配置（`App-Key` / `App-Secret` / `App-Timestamp` / `App-Sig`）。
///
/// - `app_secret` 属敏感数据：持久化前须经 AES-256-GCM 加密（见 miyo-secret）。
/// - `payload_template` 是签名载荷模板，`{{$key}}` / `{{$secret}}` / `{{$timestamp}}`
///   为内置占位符，由后端在发送前最后一刻替换（保证时间戳实时性）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicSignatureConfig {
    #[serde(default)]
    pub app_key: String,
    #[serde(default)]
    pub app_secret: String,
    /// App-Key 请求头名（默认 "App-Key"）。
    #[serde(default = "default_key_header")]
    pub key_header: String,
    /// App-Timestamp 请求头名（默认 "App-Timestamp"）。
    #[serde(default = "default_timestamp_header")]
    pub timestamp_header: String,
    /// App-Sig 请求头名（默认 "App-Sig"）。
    #[serde(default = "default_sig_header")]
    pub sig_header: String,
    #[serde(default)]
    pub algorithm: SignatureAlgorithm,
    #[serde(default)]
    pub encoding: SignatureEncoding,
    /// 签名载荷模板（默认 `{{$key}}{{$secret}}{{$timestamp}}`）。
    #[serde(default = "default_payload_template")]
    pub payload_template: String,
}

fn default_key_header() -> String {
    "App-Key".into()
}
fn default_timestamp_header() -> String {
    "App-Timestamp".into()
}
fn default_sig_header() -> String {
    "App-Sig".into()
}
fn default_payload_template() -> String {
    "{{$key}}{{$secret}}{{$timestamp}}".into()
}

impl Default for DynamicSignatureConfig {
    fn default() -> Self {
        Self {
            app_key: String::new(),
            app_secret: String::new(),
            key_header: default_key_header(),
            timestamp_header: default_timestamp_header(),
            sig_header: default_sig_header(),
            algorithm: SignatureAlgorithm::default(),
            encoding: SignatureEncoding::default(),
            payload_template: default_payload_template(),
        }
    }
}

/// OAuth2 授权模式：authorization_code（浏览器授权，默认）/ client_credentials /
/// password（非交互式，向 token_url 直接换取令牌）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuth2Grant {
    #[default]
    AuthorizationCode,
    ClientCredentials,
    Password,
}

/// 认证方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSpec {
    #[default]
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    #[serde(rename = "apikey")]
    ApiKey {
        key: String,
        value: String,
        #[serde(rename = "in")]
        location: ApiKeyLocation,
    },
    /// 动态签名鉴权（Key + Secret + Timestamp 拼接算签名，发送前最后一刻生成）。
    #[serde(rename = "dynamic_signature")]
    DynamicSignature {
        config: DynamicSignatureConfig,
    },
    /// OAuth2 授权码流（Authorization Code Grant）。
    ///
    /// 浏览器跳转 `auth_url` 完成授权 → 本地回调服务器（127.0.0.1:9090）收到 code →
    /// `token_url` 换取 access_token / refresh_token → 存入 `token`。
    /// 过期后由 `miyo-oauth` 用 refresh_token 静默刷新。
    #[serde(rename = "oauth2")]
    OAuth2 {
        client_id: String,
        client_secret: String,
        /// 授权页地址（GET，追加 response_type=code / state / redirect_uri / scope）。
        /// 仅 authorization_code 模式需要。
        auth_url: String,
        /// 换取与刷新 token 的地址（POST，form 编码）。
        token_url: String,
        scope: String,
        /// 回调地址，须与授权服务注册的一致（默认 http://127.0.0.1:9090/callback）。
        /// 仅 authorization_code 模式需要。
        redirect_uri: String,
        /// 授权模式（缺省 = authorization_code，旧数据零迁移）。
        #[serde(default)]
        grant: OAuth2Grant,
        /// password 模式：资源所有者用户名（其他模式忽略）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// password 模式：资源所有者密码（其他模式忽略）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        /// 已获取的令牌；授权成功后写入，刷新后由缓存回填。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<OAuth2Token>,
    },
    /// HTTP Digest 认证（RFC 7616）。
    ///
    /// 发送时先不带凭据；收到 `401 + WWW-Authenticate: Digest …` 后按质询
    /// 计算应答并自动重发一次（支持 MD5 / MD5-sess / SHA-256，qop=auth）。
    Digest {
        username: String,
        password: String,
    },
    /// Hawk 认证（Hawk 协议，HMAC-SHA-256）。
    ///
    /// 每次发送用递增时间戳 + 随机 nonce 计算 `mac`，有 body 时附带 payload `hash`。
    Hawk {
        /// 凭证标识（`id`）。
        key_id: String,
        /// 共享密钥（`key`）。
        key: String,
    },
    /// AWS Signature V4（兼容华为云 / 阿里云等 SigV4 风格网关）。
    ///
    /// 按区域 + 服务名做四步密钥派生，对方法 / 路径 / 排序后查询串 /
    /// `host;x-amz-date[;x-amz-security-token]` 头做规范签名。
    #[serde(rename = "awsv4")]
    AwsV4 {
        access_key: String,
        secret_key: String,
        region: String,
        service: String,
        /// 临时凭证配套；为空时不发送 `x-amz-security-token`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },
    /// 通用 HMAC AK-SK 加签（时间戳 + 随机数 + 方法 + 路径）。
    ///
    /// 发送 `X-Access-Key / X-Timestamp / X-Nonce / X-Signature` 四个头，
    /// `X-Signature = hex(HMAC-SHA256(secret, "ak\\nts\\nnonce\\nMETHOD\\npath_query"))`。
    #[serde(rename = "hmac")]
    Hmac {
        access_key: String,
        secret_key: String,
    },
}

/// OAuth2 令牌（access + refresh + 过期时刻）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// 过期时刻（UTC）。
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl OAuth2Token {
    /// 已过期（`expires_at` 早于当前时间）。
    pub fn is_expired(&self) -> bool {
        self.expires_at <= chrono::Utc::now()
    }

    /// 距过期不足 `threshold` 视为即将过期（用于提前静默刷新）。
    pub fn expires_within(&self, threshold: chrono::Duration) -> bool {
        self.expires_at - chrono::Utc::now() <= threshold
    }
}

/// OAuth2 授权状态（供 UI 状态指示器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuth2Status {
    /// 未授权（无 token 或无 refresh_token）。
    Unauthorized,
    /// 已授权，token 有效且未临近过期。
    Valid,
    /// 即将过期（≤ 60 秒），请求时将自动刷新。
    ExpiringSoon,
    /// 已过期（但仍有 refresh_token，请求时将自动刷新）。
    Expired,
}

impl AuthSpec {
    /// 若为 OAuth2，返回其授权状态。
    pub fn oauth2_status(&self) -> Option<OAuth2Status> {
        let AuthSpec::OAuth2 { token, .. } = self else {
            return None;
        };
        let Some(token) = token else {
            return Some(OAuth2Status::Unauthorized);
        };
        if token.is_expired() {
            return Some(OAuth2Status::Expired);
        }
        if token.expires_within(chrono::Duration::seconds(60)) {
            return Some(OAuth2Status::ExpiringSoon);
        }
        Some(OAuth2Status::Valid)
    }
}

/// Multipart 值类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultipartValueType {
    Text,
    FilePath,
}

/// Multipart 字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultipartField {
    pub key: String,
    pub value_type: MultipartValueType,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// GraphQL 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GraphQLSpec {
    pub query: String,
    /// 变量 JSON 文本（空串或 "{}" 表示无变量）。
    pub variables: String,
    pub operation_name: String,
}

/// gRPC 调用配置（BodySpec::Grpc 的载荷）。
///
/// 地址（host:port，可含 {{变量}}）存 `Endpoint.path`；metadata 复用
/// RequestSpec.headers（gRPC 元数据即 HTTP/2 头）；`{{变量}}` 由执行层
/// 与 HTTP 同口径渲染。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GrpcSpec {
    /// 全限定服务名，如 `user.v1.UserService`。
    pub service: String,
    /// 方法名，如 `GetUser`。
    pub method: String,
    /// 请求消息（protobuf-JSON 文本；空串或 "{}" 表示空消息）。
    pub message: String,
    /// 是否使用 TLS（false = 明文 h2c）。
    pub use_tls: bool,
    /// 引用的项目级 proto 文件 id（空 = 使用服务端反射）。
    pub proto_ids: Vec<String>,
}

/// 项目级 proto 文件（gRPC 端点引用；protox 运行时编译的输入）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoFile {
    pub id: Uuid,
    pub project_id: Uuid,
    /// 文件名（import 路径相对名，如 `user/v1/user.proto`）。
    pub name: String,
    /// .proto 源文本。
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// GraphQL 错误位置（errors[].locations[]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQLErrorLocation {
    pub line: u64,
    pub column: u64,
}

/// GraphQL 错误条目（errors[]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphQLError {
    pub message: String,
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    pub path: Option<Vec<serde_json::Value>>,
}

/// 解析后的 GraphQL 响应：`data` 与 `errors` 并存或互斥。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphQLResponse {
    pub data: Option<serde_json::Value>,
    pub errors: Vec<GraphQLError>,
}

impl GraphQLResponse {
    /// 是否存在错误。
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// 请求 Body。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BodySpec {
    #[default]
    None,
    Json {
        raw: String,
    },
    Text {
        raw: String,
    },
    #[serde(rename = "urlencoded")]
    UrlEncoded {
        fields: Vec<KeyValue>,
    },
    Multipart {
        fields: Vec<MultipartField>,
    },
    /// 显式对齐前端的 `mode: "graphql"`：serde 对 `GraphQL` 的 snake_case
    /// 转换产物是 `graph_q_l`，会导致独立 GraphQL 视图 execute_request 报
    /// unknown variant（v0.0.26 起可复现）。
    #[serde(rename = "graphql")]
    GraphQL {
        spec: GraphQLSpec,
    },
    /// 二进制文件请求体：发送时由执行器读取 `path` 指向文件的原始字节，
    /// Content-Type 默认 application/octet-stream（用户显式设置的请求头优先）。
    Binary {
        path: String,
    },
    /// gRPC 调用（unary / 服务端流）：地址在 Endpoint.path，metadata 复用请求头。
    /// `Grpc` 的 snake_case 产物即 `grpc`，无 GraphQL 的重命名陷阱，仍显式声明固定 wire 值。
    #[serde(rename = "grpc")]
    Grpc {
        spec: GrpcSpec,
    },
}

impl BodySpec {
    /// 当前 body 模式的内部名称（用于 UI 下拉框）。
    pub fn mode_name(&self) -> &'static str {
        match self {
            BodySpec::None => "none",
            BodySpec::Json { .. } => "json",
            BodySpec::Text { .. } => "text",
            BodySpec::UrlEncoded { .. } => "urlencoded",
            BodySpec::Multipart { .. } => "multipart",
            BodySpec::GraphQL { .. } => "graphql",
            BodySpec::Binary { .. } => "binary",
            BodySpec::Grpc { .. } => "grpc",
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, BodySpec::None)
    }
}

/// JSON Body / 响应示例的字段文档（设计页结构化编辑器的注释 sidecar）。
///
/// key 为 RFC6901 JSON Pointer 路径（如 `/user/name`、数组样本元素用 `/0`），
/// 与前端 `jsonTree` 编辑器生成的 path 一一对应；发送 / Mock 均忽略此数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FieldDoc {
    /// 字段说明（展示用注释）。
    #[serde(default)]
    pub description: String,
    /// 是否必填（结构化编辑器勾选；样本推断默认为 true）。
    #[serde(default = "default_true")]
    pub required: bool,
}

/// 测试配置（pre_request / extract / assertions），存储为 JSON。
pub type TestConfig = serde_json::Value;

/// Pre / Post 请求脚本（Postman 风格 `pm.*` API，引擎见 miyo-http 脚本沙箱）。
/// 两段均为空时序列化整体省略（`Option::is_none`），旧数据零迁移。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScriptsSpec {
    /// 请求发送前执行：可改请求头（`pm.request.headers.*`）、读写变量。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pre_request: String,
    /// 响应到达后执行：可断言（`pm.test`）、读响应、读写变量（变量不落库，仅本次运行可见）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub post_response: String,
}

/// 统一请求结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestSpec {
    #[serde(default)]
    pub params: Vec<KeyValue>,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub path_variables: Vec<KeyValue>,
    #[serde(default)]
    pub auth: AuthSpec,
    #[serde(default)]
    pub body: BodySpec,
    /// 编辑器配置 Tab（params/auth/headers/body/...）：用户最近一次停留位置。
    /// 为空时前端按 HTTP Method 智能默认（POST 系 → body，其余 → params）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<String>,
    /// 请求超时（毫秒）。`None` = 使用全局默认（设置中的请求超时）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
    #[serde(default)]
    pub tests: Option<TestConfig>,
    /// 禁用 Cookie 自动回放（默认 false = 携带 jar 中的同域 Cookie）。
    #[serde(default)]
    pub disable_cookies: bool,
    /// JSON Body 字段文档（JSON Pointer → 注释；设计页结构化编辑器用，发送时忽略）。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub body_docs: HashMap<String, FieldDoc>,
    /// Pre / Post 脚本（pm.* 沙箱执行；None / 空 = 不执行）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts: Option<ScriptsSpec>,
}

impl Default for RequestSpec {
    fn default() -> Self {
        RequestSpec {
            params: Vec::new(),
            headers: Vec::new(),
            path_variables: Vec::new(),
            auth: AuthSpec::None,
            body: BodySpec::None,
            active_tab: None,
            timeout_ms: None,
            follow_redirects: true,
            tests: None,
            disable_cookies: false,
            body_docs: HashMap::new(),
            scripts: None,
        }
    }
}

/// 默认团队 id：与迁移 0015 中插入的固定 id 一致，勿改。
pub const DEFAULT_TEAM_ID: Uuid = Uuid::from_u128(1);

/// 团队：现阶段仅作为项目的容器（schema 预留，后续扩展成员/权限）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 同步远端类型（存储层枚举，与 miyo-sync::RemoteKind 对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRemoteKind {
    Git,
    S3,
    Db,
}

/// 团队同步远端：团队绑定的快照仓库（git / S3 / db）。
///
/// 一个团队一个远端路径（不同团队可复用同一路径）；推/拉按项目触发，
/// 同一仓库内按 `projects/{project_id}.json` 分文件存放各项目快照。
/// `config` 为 RemoteConfig（miyo-sync）的 JSON（含 kind tag），存储层不解析内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRemote {
    pub id: Uuid,
    pub team_id: Uuid,
    pub kind: SyncRemoteKind,
    pub config: serde_json::Value,
    /// 上次成功同步的远端 revision。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_revision: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 项目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub variables: HashMap<String, String>,
    /// 所属团队：None 视为默认团队（兼容无团队信息的旧备份 v3）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 文件夹。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub project_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 接口。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: Uuid,
    pub project_id: Uuid,
    pub folder_id: Option<Uuid>,
    pub name: String,
    pub method: HttpMethod,
    pub path: String,
    pub description: String,
    pub status: EndpointStatus,
    pub sort_order: i64,
    pub request: RequestSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 环境变量（支持远程 / 本地双值：本地值优先覆盖远程值）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub key: String,
    /// 远程 / 公共值。
    pub remote_value: String,
    /// 本地私有覆盖值（非空时优先于 remote_value）。
    #[serde(default)]
    pub local_value: String,
    /// 是否参与注入（关闭后该变量不进入请求变量表）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 全局参数注入位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GlobalParamLocation {
    /// 拼入每个请求的 URL 查询参数。
    #[default]
    Query,
    /// 注入每个请求的请求头。
    Header,
}

/// 全局参数（注入制）：每个请求自动附加的 key/value。
///
/// 与「全局变量」的区别：
/// - 全局变量是「引用制」——请求里写 `{{name}}` 才按名替换；
/// - 全局参数是「注入制」——定义后每个请求自动并入 query / header，无需手动写。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalParam {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub location: GlobalParamLocation,
}

/// 环境（项目维度）。
///
/// 环境按项目归属（哪个项目就编辑哪个项目的环境），每个环境持有
/// 单一前置 Base URL `base_url`（可包含 `{{变量}}`）与一组结构化
/// 环境变量 `variables`（本地值覆盖远程值）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub id: Uuid,
    /// 归属项目（环境随项目删除级联清理）。
    pub project_id: Uuid,
    pub name: String,
    /// 前置 Base URL，如 `http://dev-test01.redotpay.inet:8092`（可包含 `{{变量}}`）。
    #[serde(default)]
    pub base_url: String,
    /// 结构化环境变量列表。
    #[serde(default)]
    pub variables: Vec<EnvironmentVariable>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EnvironmentVariable {
    /// 生效值：本地覆盖值非空时取本地，否则取远程 / 公共值。
    pub fn effective_value(&self) -> &str {
        let local = self.local_value.trim();
        if !local.is_empty() {
            local
        } else {
            self.remote_value.trim()
        }
    }

    pub fn is_effective(&self) -> bool {
        self.enabled && !self.effective_value().is_empty()
    }
}

impl Environment {
    /// 生效环境变量扁平表（仅 enabled；本地值优先）。
    pub fn effective_variables(&self) -> HashMap<String, String> {
        self.variables
            .iter()
            .filter(|v| v.is_effective())
            .map(|v| (v.key.trim().to_string(), v.effective_value().to_string()))
            .collect()
    }
}

/// 响应示例。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseExample {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub name: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub content_type: String,
    /// 响应字段文档（JSON Pointer → 注释；设计页结构化编辑器用）。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub docs: HashMap<String, FieldDoc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 请求用例（请求快照）。
///
/// 保存接口某次请求的完整 `RequestSpec`（参数 / 认证 / 请求头 / Body / 超时等），
/// 可一键回填编辑器复用，避免反复手工拼装。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestExample {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub name: String,
    pub request: RequestSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 测试用例运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TestCaseStatus {
    Success,
    Failed,
    Untested,
}

impl TestCaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestCaseStatus::Success => "Success",
            TestCaseStatus::Failed => "Failed",
            TestCaseStatus::Untested => "Untested",
        }
    }

    pub fn parse(s: &str) -> Option<TestCaseStatus> {
        match s {
            "Success" => Some(TestCaseStatus::Success),
            "Failed" => Some(TestCaseStatus::Failed),
            "Untested" => Some(TestCaseStatus::Untested),
            _ => None,
        }
    }
}

/// 测试用例（Apifox 风格用例管理）。
///
/// 每个用例保存一次调用的独立快照：URL 路径 + 路径变量 + Query 参数 + 请求头 +
/// Body，可单独运行、一键回填调试页重新调用。`request_id` 关联主接口（`endpoints`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestCase {
    pub id: Uuid,
    /// 关联的主接口 ID（endpoints.id）。
    pub request_id: Uuid,
    pub name: String,
    /// 用例分组：正向 / 负向 / 边界值 / 安全性 / 其他。
    pub category: String,
    pub method: HttpMethod,
    pub url_path: String,
    /// URL 路径变量取值（`/users/{id}` 里的 id）。缺了它单跑会把字面量
    /// `{id}` 发出去，故与 params 同级随用例快照保存。
    #[serde(default)]
    pub path_variables: Vec<KeyValue>,
    pub params: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    /// body 类型标识：json / form-data / raw / urlencoded / graphql / binary / none。
    pub body_type: String,
    pub body_content: String,
    pub last_run_status: TestCaseStatus,
    pub created_at: DateTime<Utc>,
}

/// Mock 匹配条件项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockMatchItem {
    pub key: String,
    pub value: String,
}

/// 自定义 Mock 规则。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockRule {
    pub id: Uuid,
    pub project_id: Uuid,
    pub endpoint_id: Option<Uuid>,
    pub name: String,
    pub method: HttpMethod,
    pub path: String,
    pub match_query: Vec<MockMatchItem>,
    pub match_headers: Vec<MockMatchItem>,
    pub response_status: u16,
    pub response_headers: HashMap<String, String>,
    pub response_body_template: String,
    pub delay_ms: u64,
    /// 故障注入：百分之多少的命中请求返回 fault_status（0 = 关闭）。
    #[serde(default)]
    pub fault_rate_pct: u8,
    /// 故障注入时的状态码（默认 500）。
    #[serde(default = "default_fault_status")]
    pub fault_status: u16,
    pub enabled: bool,
    pub priority: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 单次测试运行结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub environment_id: Option<Uuid>,
    pub name: String,
    pub result_json: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// 请求历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestHistory {
    pub id: Uuid,
    pub project_id: Uuid,
    pub endpoint_id: Option<Uuid>,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub request_summary_json: String,
    pub response_summary_json: String,
    pub created_at: DateTime<Utc>,
}

/// 自增序列（`{{$seq:key}}`）；`value` 为下一次输出值。key 为空字符串表示全局 `{{$seq}}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqCounter {
    pub key: String,
    pub value: u64,
}

/// WebSocket 待发消息类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WsMessageType {
    Text,
    Binary,
    Ping,
}

impl WsMessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WsMessageType::Text => "text",
            WsMessageType::Binary => "binary",
            WsMessageType::Ping => "ping",
        }
    }
}

/// 持久化的 WebSocket 待发消息（断线 / 队列溢出时落库）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsMessageRecord {
    pub id: Uuid,
    pub message_type: WsMessageType,
    /// Text 为原文；Binary / Ping 为 base64 编码。
    pub payload: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_method_roundtrip() {
        for m in HttpMethod::all() {
            let s = serde_json::to_string(m).unwrap();
            assert_eq!(s, format!("\"{}\"", m.as_str()));
            let back: HttpMethod = serde_json::from_str(&s).unwrap();
            assert_eq!(back, *m);
            let parsed: HttpMethod = m.as_str().parse().unwrap();
            assert_eq!(parsed, *m);
        }
        assert!("TRACE".parse::<HttpMethod>().is_err());
    }

    #[test]
    fn status_serde() {
        let s = serde_json::to_string(&EndpointStatus::Developing).unwrap();
        assert_eq!(s, "\"developing\"");
        let back: EndpointStatus = serde_json::from_str("\"released\"").unwrap();
        assert_eq!(back, EndpointStatus::Released);
    }

    #[test]
    fn request_spec_default_json_shape() {
        let spec = RequestSpec::default();
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["params"], serde_json::json!([]));
        assert_eq!(json["auth"]["type"], "none");
        assert_eq!(json["body"]["mode"], "none");
        assert_eq!(json["timeout_ms"], serde_json::Value::Null);
        assert_eq!(json["follow_redirects"], true);
    }

    #[test]
    fn keyvalue_design_metadata_roundtrip() {
        let kv = KeyValue {
            key: "userId".into(),
            value: "1".into(),
            enabled: true,
            description: "用户 ID".into(),
            field_type: FieldType::Number,
            required: false,
            example: "42".into(),
        };
        let json = serde_json::to_value(&kv).unwrap();
        assert_eq!(json["field_type"], "number");
        assert_eq!(json["required"], false);
        assert_eq!(json["example"], "42");
        let back: KeyValue = serde_json::from_value(json).unwrap();
        assert_eq!(back, kv);
    }

    /// 旧数据兼容：缺省 field_type / required / example 时按默认值反序列化。
    #[test]
    fn keyvalue_legacy_json_defaults() {
        let legacy = serde_json::json!({
            "key": "page", "value": "1", "enabled": true, "description": ""
        });
        let kv: KeyValue = serde_json::from_value(legacy).unwrap();
        assert_eq!(kv.field_type, FieldType::String);
        assert!(kv.required);
        assert_eq!(kv.example, "");
        // 新字段序列化后仍可被旧读取方忽略（未知字段不报错由 serde 默认保证）。
        let full = serde_json::to_string(&KeyValue::new("a", "1")).unwrap();
        assert!(full.contains("\"field_type\":\"string\""));
        assert!(full.contains("\"required\":true"));
    }

    #[test]
    fn body_spec_binary_json_shape() {
        let body = BodySpec::Binary {
            path: "/tmp/a.png".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["mode"], "binary");
        assert_eq!(json["path"], "/tmp/a.png");
        let back: BodySpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn auth_bearer_json_shape() {
        let auth = AuthSpec::Bearer {
            token: "{{token}}".into(),
        };
        let json = serde_json::to_value(&auth).unwrap();
        assert_eq!(json["type"], "bearer");
        assert_eq!(json["token"], "{{token}}");
        let back: AuthSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, auth);
    }

    #[test]
    fn auth_apikey_json_shape() {
        let auth = AuthSpec::ApiKey {
            key: "X-API-KEY".into(),
            value: "{{api_key}}".into(),
            location: ApiKeyLocation::Header,
        };
        let json = serde_json::to_value(&auth).unwrap();
        assert_eq!(json["type"], "apikey");
        assert_eq!(json["in"], "header");
        let back: AuthSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, auth);
    }

    #[test]
    fn auth_signing_types_json_shape_and_roundtrip() {
        // tag 命名与前端 `AuthSpec` 联合类型严格对应。
        let cases: Vec<(AuthSpec, &str)> = vec![
            (
                AuthSpec::Digest {
                    username: "u".into(),
                    password: "p".into(),
                },
                "digest",
            ),
            (
                AuthSpec::Hawk {
                    key_id: "id-1".into(),
                    key: "k".into(),
                },
                "hawk",
            ),
            (
                AuthSpec::AwsV4 {
                    access_key: "AK".into(),
                    secret_key: "SK".into(),
                    region: "cn-north-1".into(),
                    service: "s3".into(),
                    session_token: None,
                },
                "awsv4",
            ),
            (
                AuthSpec::Hmac {
                    access_key: "ak".into(),
                    secret_key: "sk".into(),
                },
                "hmac",
            ),
        ];
        for (auth, tag) in cases {
            let json = serde_json::to_value(&auth).unwrap();
            assert_eq!(json["type"], tag);
            let back: AuthSpec = serde_json::from_value(json).unwrap();
            assert_eq!(back, auth);
        }
        // session_token 为空时不序列化（紧凑存储）。
        let json = serde_json::to_value(&AuthSpec::AwsV4 {
            access_key: String::new(),
            secret_key: String::new(),
            region: String::new(),
            service: String::new(),
            session_token: None,
        })
        .unwrap();
        assert!(json.get("session_token").is_none());
    }

    #[test]
    fn body_urlencoded_json_shape() {
        let body = BodySpec::UrlEncoded {
            fields: vec![KeyValue::new("a", "1")],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["mode"], "urlencoded");
        assert_eq!(json["fields"][0]["key"], "a");
        let back: BodySpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn oauth2_json_shape_and_roundtrip() {
        let auth = AuthSpec::OAuth2 {
            client_id: "my-client".into(),
            client_secret: "s3cret".into(),
            auth_url: "https://idp.example.com/authorize".into(),
            token_url: "https://idp.example.com/token".into(),
            scope: "openid profile".into(),
            redirect_uri: "http://127.0.0.1:9090/callback".into(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: Some(OAuth2Token {
                access_token: "at-1".into(),
                token_type: "Bearer".into(),
                refresh_token: Some("rt-1".into()),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            }),
        };
        let json = serde_json::to_value(&auth).unwrap();
        assert_eq!(json["type"], "oauth2");
        assert_eq!(json["client_id"], "my-client");
        assert_eq!(json["token"]["access_token"], "at-1");
        let back: AuthSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, auth);
    }

    #[test]
    fn oauth2_without_token_serializes_compact() {
        let auth = AuthSpec::OAuth2 {
            client_id: "c".into(),
            client_secret: "s".into(),
            auth_url: String::new(),
            token_url: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: None,
        };
        let json = serde_json::to_value(&auth).unwrap();
        assert!(json.get("token").is_none());
    }

    #[test]
    fn oauth2_token_status_transitions() {
        let make = |hours: i64| AuthSpec::OAuth2 {
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: String::new(),
            token_url: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: Some(OAuth2Token {
                access_token: String::new(),
                token_type: String::new(),
                refresh_token: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(hours),
            }),
        };
        assert_eq!(make(1).oauth2_status(), Some(OAuth2Status::Valid));
        // 30 秒后过期 → 即将过期
        let soon = AuthSpec::OAuth2 {
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: String::new(),
            token_url: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: Some(OAuth2Token {
                access_token: String::new(),
                token_type: String::new(),
                refresh_token: None,
                expires_at: chrono::Utc::now() + chrono::Duration::seconds(30),
            }),
        };
        assert_eq!(soon.oauth2_status(), Some(OAuth2Status::ExpiringSoon));
        // 已过期
        let expired = AuthSpec::OAuth2 {
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: String::new(),
            token_url: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: Some(OAuth2Token {
                access_token: String::new(),
                token_type: String::new(),
                refresh_token: None,
                expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
            }),
        };
        assert_eq!(expired.oauth2_status(), Some(OAuth2Status::Expired));
        // 无 token
        let none = AuthSpec::OAuth2 {
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: String::new(),
            token_url: String::new(),
            scope: String::new(),
            redirect_uri: String::new(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: None,
        };
        assert_eq!(none.oauth2_status(), Some(OAuth2Status::Unauthorized));
        assert_eq!(AuthSpec::None.oauth2_status(), None);
    }
}
