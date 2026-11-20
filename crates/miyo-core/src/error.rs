use uuid::Uuid;

/// 统一错误类型：所有用户可见错误都必须映射到该类型，UI 层统一转换为中文提示。
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    /// 数据库错误（载荷为 sqlx 错误的用户可见明细——由 miyo-storage 边界
    /// 显式映射；Domain 层不再依赖 sqlx）。
    #[error("database error: {0}")]
    Database(String),

    /// 库 schema 比当前程序新（用户回退到旧版本）：sqlx 的 `MigrateError::VersionMissing`
    /// 归此变体，给出可操作提示，而不是笼统的「数据库操作失败」。
    /// 载荷为库实际版本号（取最大值）。
    #[error("database schema version mismatch (db version {0})")]
    SchemaTooNew(i64),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// 未分类 HTTP 错误（载荷为 reqwest 错误的用户可见明细——分类由
    /// miyo-http 的 from_reqwest 完成；Domain 层不再依赖 reqwest）。
    #[error("http error: {0}")]
    Http(String),

    /// 网络超时（连接 / 读取 / 总请求超时）。
    #[error("network timeout: {0}")]
    NetworkTimeout(String),

    /// TLS / 证书错误（证书无效、已过期、不受信任）。
    #[error("ssl error: {0}")]
    Ssl(String),

    /// DNS 解析失败（域名不存在或网络不可用）。
    #[error("dns error: {0}")]
    Dns(String),

    /// 连接失败（目标拒绝连接或网络不可达）。
    #[error("connection error: {0}")]
    Connection(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    /// 被拒绝（工具停用 / 鉴权失败 / 权限不足）：区别于网络连接失败。
    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("openapi error: {0}")]
    OpenApi(String),

    #[error("mock error: {0}")]
    Mock(String),

    /// 服务端口绑定失败（Agent 控制面 / Mock 服务起不来）：
    /// 不再借用 Mock 变体（用户看到 MOCK 错误码却非 Mock 问题）。
    #[error("port bind error: {0}")]
    PortBind(String),

    #[error("test error: {0}")]
    Test(String),

    #[error("script error: {0}")]
    ScriptError(String),

    #[error("websocket error: {0}")]
    WebSocket(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// 环境变量解密失败（主密钥丢失 / 更换 / 密文损坏）。
    #[error("decryption failed: {0}")]
    Decryption(String),

    /// OAuth2 授权 / 刷新失败。
    #[error("oauth2 error: {0}")]
    OAuth2(String),

    /// 请求被用户主动取消（前端取消按钮）。
    #[error("request cancelled: {0}")]
    Cancelled(String),

    /// gRPC 调用失败（连接/编解码/远端状态；grpc-status 非零也归此）。
    #[error("grpc error: {0}")]
    Grpc(String),

    /// 团队同步传输失败（git/S3/db 远端网络、凭证、schema 版本不匹配）。
    /// 区别于笼统的 VALIDATION：前端可按 SYNC 码分支提示重试/检查凭证。
    #[error("sync error: {0}")]
    Sync(String),
}

impl AppError {
    /// 错误消息内的 URL query 脱敏（NetworkTimeout / Ssl / Dns / Connection /
    /// Validation 载荷统一处理）。
    pub fn sanitize_message_urls(self) -> AppError {
        match self {
            AppError::NetworkTimeout(m) => {
                AppError::NetworkTimeout(crate::sensitive::redact_message_urls(&m))
            }
            AppError::Ssl(m) => AppError::Ssl(crate::sensitive::redact_message_urls(&m)),
            AppError::Dns(m) => AppError::Dns(crate::sensitive::redact_message_urls(&m)),
            AppError::Connection(m) => {
                AppError::Connection(crate::sensitive::redact_message_urls(&m))
            }
            AppError::Validation(m) => {
                AppError::Validation(crate::sensitive::redact_message_urls(&m))
            }
            other => other,
        }
    }

    /// 用户可见的中文错误消息。
    pub fn user_message(&self) -> String {
        match self {
            AppError::Database(_) => "数据库操作失败".to_string(),
            AppError::SchemaTooNew(v) => format!(
                "数据库由更新版本的 Miyo 创建（库版本 {v}），当前版本无法识别。请在设置中改用新的数据目录，或安装最新版 Miyo"
            ),
            AppError::Io(_) => format!("文件操作失败：{self}"),
            AppError::Http(_) => "网络请求失败".to_string(),
            AppError::NetworkTimeout(_) => "连接超时：服务器未在限定时间内响应".to_string(),
            AppError::Ssl(_) => "TLS 证书错误：证书无效、已过期或不受信任".to_string(),
            AppError::Dns(_) => "DNS 解析失败：域名不存在或网络不可用".to_string(),
            AppError::Connection(_) => "连接失败：目标拒绝连接或网络不可达".to_string(),
            AppError::Validation(msg) => msg.clone(),
            AppError::NotFound(name) => format!("未找到：{name}"),
            AppError::Forbidden(msg) => msg.clone(),
            AppError::PortBind(msg) => msg.clone(),
            AppError::OpenApi(msg) => msg.clone(),
            AppError::Mock(msg) => msg.clone(),
            AppError::Test(msg) => msg.clone(),
            AppError::ScriptError(msg) => msg.clone(),
            AppError::WebSocket(msg) => msg.clone(),
            AppError::Json(_) => "JSON 解析失败".to_string(),
            AppError::Decryption(msg) => msg.clone(),
            AppError::OAuth2(msg) => msg.clone(),
            AppError::Cancelled(_) => "请求已取消".to_string(),
            AppError::Grpc(msg) => msg.clone(),
            AppError::Sync(msg) => msg.clone(),
        }
    }
}

/// 便捷的未找到错误构造。
pub fn not_found(what: &str, id: Uuid) -> AppError {
    AppError::NotFound(format!("{what}（{id}）"))
}

pub fn validation(msg: impl Into<String>) -> AppError {
    AppError::Validation(msg.into())
}

pub fn openapi_error(msg: impl Into<String>) -> AppError {
    AppError::OpenApi(msg.into())
}

pub fn script_error(msg: impl Into<String>) -> AppError {
    AppError::ScriptError(msg.into())
}

pub fn ws_error(msg: impl Into<String>) -> AppError {
    AppError::WebSocket(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_messages_cover_all_network_variants() {
        assert!(AppError::NetworkTimeout("".into())
            .user_message()
            .contains("超时"));
        assert!(AppError::Dns("".into()).user_message().contains("DNS"));
        assert!(AppError::Ssl("".into()).user_message().contains("证书"));
        assert!(AppError::Connection("".into())
            .user_message()
            .contains("连接失败"));
        assert!(AppError::OAuth2("需要重新授权".into())
            .user_message()
            .contains("重新授权"));
    }

    #[test]
    fn sanitize_message_urls_redacts_url_credentials_in_payload() {
        // 错误载荷包含带凭证 query 的 URL → 脱敏后不得出现明文值。
        //（NetworkTimeout 的 user_message 是固定文案，载荷经 Debug / 日志 /
        // api_error 通道外发，同样必须脱敏。）
        let err = AppError::NetworkTimeout(
            "error sending request for url (https://api.example.com/login?access_token=sec&x=1)"
                .into(),
        )
        .sanitize_message_urls();
        match err {
            AppError::NetworkTimeout(msg) => {
                assert!(msg.contains("__MIYO_REDACTED__"), "应脱敏：{msg}");
                assert!(!msg.contains("sec&"), "不得残留明文凭据：{msg}");
                assert!(msg.contains("x=1"), "非敏感参数保留：{msg}");
            }
            other => panic!("变体应保持：{other:?}"),
        }
    }

    #[test]
    fn error_codes_are_distinct() {
        let msgs = [
            AppError::NetworkTimeout("t".into()),
            AppError::Dns("d".into()),
            AppError::Ssl("s".into()),
            AppError::Connection("c".into()),
        ];
        assert!(
            msgs.iter()
                .map(|m| m.user_message())
                .collect::<std::collections::HashSet<_>>()
                .len()
                == 4
        );
    }
}
