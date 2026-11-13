//! 统一错误类型：`AppError` → `CommandError` → 前端 JS Error。
//!
//! Tauri Command 只能返回 `Result<T, E>` 且 `E: Serialize`。这里将
//! `miyo_core::AppError` 映射为 `{ code, message }`：
//! - `code`：程序化错误码（前端可用 `err.code` 分支处理）；
//! - `message`：中文用户提示（`AppError::user_message`）。
//!
//! 前端 `invoke()` 被 reject 时，收到的即为该对象（见 `useMiyoApi.ts` 的 `toFoxError`）。
//!
//! 错误码是 [`CommandErrorCode`] 枚举而非裸字符串：序列化产物（SCREAMING_SNAKE_CASE）
//! 与历史字符串完全一致；前端 union 类型由 `ts` feature 生成（types/generated/），
//! 新增变体若未同步前端会在 bindings job 红灯，不再可能漂移。

use miyo_core::AppError;

/// 程序化错误码。序列化为 SCREAMING_SNAKE_CASE 字符串（与历史前端约定一致）。
///
/// 变体覆盖：`From<AppError>` 的 22+1 个映射码 + Command 层手写码
///（INTERNAL / EXPORT / MOCK_RUNNING / AI_*）。新增变体时必须：
/// 1. 在 `as_str()` 补齐分支（编译器穷尽检查）；
/// 2. `tests/ts_bindings.rs` 导出后前端 union 自动获得新成员。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CommandErrorCode {
    Database,
    DatabaseSchemaTooNew,
    Io,
    Http,
    Timeout,
    Ssl,
    Dns,
    Connection,
    Validation,
    NotFound,
    Forbidden,
    /// serde 的 SCREAMING_SNAKE_CASE 会把 `OpenApi` 拆成 OPEN_API，显式钉住历史值。
    #[serde(rename = "OPENAPI")]
    OpenApi,
    Mock,
    PortBind,
    Test,
    Script,
    Websocket,
    Json,
    Decrypt,
    Oauth2,
    Cancelled,
    Grpc,
    /// 团队同步远端传输失败（原借道 VALIDATION）。
    Sync,
    /// Command 层内部失败（序列化 / 任务启动等，非领域错误）。
    Internal,
    /// 导出序列化失败。
    Export,
    /// Mock 服务已在运行。
    MockRunning,
    /// AI 供应商调用失败。
    AiProvider,
    /// AI 配置缺失。
    AiNotConfigured,
    /// 审批对象不在待确认状态。
    AiNoPending,
    /// 高风险操作需要用户确认（Agent 工具执行守卫）。
    HighRiskConfirmationRequired,
    /// Mock 服务未在运行。
    MockNotRunning,
    /// 系统剪贴板写入失败。
    Clipboard,
}

impl CommandErrorCode {
    /// 与 serde 序列化产物一致的字符串（Display / 日志用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            CommandErrorCode::Database => "DATABASE",
            CommandErrorCode::DatabaseSchemaTooNew => "DATABASE_SCHEMA_TOO_NEW",
            CommandErrorCode::Io => "IO",
            CommandErrorCode::Http => "HTTP",
            CommandErrorCode::Timeout => "TIMEOUT",
            CommandErrorCode::Ssl => "SSL",
            CommandErrorCode::Dns => "DNS",
            CommandErrorCode::Connection => "CONNECTION",
            CommandErrorCode::Validation => "VALIDATION",
            CommandErrorCode::NotFound => "NOT_FOUND",
            CommandErrorCode::Forbidden => "FORBIDDEN",
            CommandErrorCode::OpenApi => "OPENAPI",
            CommandErrorCode::Mock => "MOCK",
            CommandErrorCode::PortBind => "PORT_BIND",
            CommandErrorCode::Test => "TEST",
            CommandErrorCode::Script => "SCRIPT",
            CommandErrorCode::Websocket => "WEBSOCKET",
            CommandErrorCode::Json => "JSON",
            CommandErrorCode::Decrypt => "DECRYPT",
            CommandErrorCode::Oauth2 => "OAUTH2",
            CommandErrorCode::Cancelled => "CANCELLED",
            CommandErrorCode::Grpc => "GRPC",
            CommandErrorCode::Sync => "SYNC",
            CommandErrorCode::Internal => "INTERNAL",
            CommandErrorCode::Export => "EXPORT",
            CommandErrorCode::MockRunning => "MOCK_RUNNING",
            CommandErrorCode::AiProvider => "AI_PROVIDER",
            CommandErrorCode::AiNotConfigured => "AI_NOT_CONFIGURED",
            CommandErrorCode::AiNoPending => "AI_NO_PENDING",
            CommandErrorCode::HighRiskConfirmationRequired => "HIGH_RISK_CONFIRMATION_REQUIRED",
            CommandErrorCode::MockNotRunning => "MOCK_NOT_RUNNING",
            CommandErrorCode::Clipboard => "CLIPBOARD",
        }
    }
}

/// 命令层错误。`#[derive(Serialize)]` 使其可直接作为 Tauri Command 的 Err 传输。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CommandError {
    /// 程序化错误码（如 `VALIDATION`、`NOT_FOUND`、`DATABASE`）。
    pub code: CommandErrorCode,
    /// 中文用户提示。
    pub message: String,
}

impl CommandError {
    /// 参数校验类错误。
    pub fn validation(msg: impl Into<String>) -> Self {
        CommandError {
            code: CommandErrorCode::Validation,
            message: msg.into(),
        }
    }

    /// 指定错误码的业务错误。
    pub fn with_code(code: CommandErrorCode, msg: impl Into<String>) -> Self {
        CommandError {
            code,
            message: msg.into(),
        }
    }

    /// gRPC 调用类错误（连接 / 编解码 / 远端非零状态）。
    pub fn grpc(msg: impl Into<String>) -> Self {
        CommandError {
            code: CommandErrorCode::Grpc,
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CommandError {}

/// `AppError` → `CommandError` 的统一映射。
impl From<AppError> for CommandError {
    fn from(err: AppError) -> Self {
        let code = match &err {
            AppError::Database(_) => CommandErrorCode::Database,
            AppError::SchemaTooNew(_) => CommandErrorCode::DatabaseSchemaTooNew,
            AppError::Io(_) => CommandErrorCode::Io,
            AppError::Http(_) => CommandErrorCode::Http,
            AppError::NetworkTimeout(_) => CommandErrorCode::Timeout,
            AppError::Ssl(_) => CommandErrorCode::Ssl,
            AppError::Dns(_) => CommandErrorCode::Dns,
            AppError::Connection(_) => CommandErrorCode::Connection,
            AppError::Validation(_) => CommandErrorCode::Validation,
            AppError::NotFound(_) => CommandErrorCode::NotFound,
            AppError::Forbidden(_) => CommandErrorCode::Forbidden,
            AppError::OpenApi(_) => CommandErrorCode::OpenApi,
            AppError::Mock(_) => CommandErrorCode::Mock,
            AppError::PortBind(_) => CommandErrorCode::PortBind,
            AppError::Test(_) => CommandErrorCode::Test,
            AppError::ScriptError(_) => CommandErrorCode::Script,
            AppError::WebSocket(_) => CommandErrorCode::Websocket,
            AppError::Json(_) => CommandErrorCode::Json,
            AppError::Decryption(_) => CommandErrorCode::Decrypt,
            AppError::OAuth2(_) => CommandErrorCode::Oauth2,
            AppError::Cancelled(_) => CommandErrorCode::Cancelled,
            AppError::Grpc(_) => CommandErrorCode::Grpc,
            AppError::Sync(_) => CommandErrorCode::Sync,
        };
        CommandError {
            code,
            message: err.user_message(),
        }
    }
}

/// Command 统一返回值。
pub type CommandResult<T> = std::result::Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// serde 序列化产物必须与历史字符串逐字一致（前端按码分支，破坏即回归）。
    #[test]
    fn serialization_matches_legacy_strings() {
        let cases = [
            (CommandErrorCode::Database, "DATABASE"),
            (CommandErrorCode::DatabaseSchemaTooNew, "DATABASE_SCHEMA_TOO_NEW"),
            (CommandErrorCode::Io, "IO"),
            (CommandErrorCode::Http, "HTTP"),
            (CommandErrorCode::Timeout, "TIMEOUT"),
            (CommandErrorCode::Ssl, "SSL"),
            (CommandErrorCode::Dns, "DNS"),
            (CommandErrorCode::Connection, "CONNECTION"),
            (CommandErrorCode::Validation, "VALIDATION"),
            (CommandErrorCode::NotFound, "NOT_FOUND"),
            (CommandErrorCode::Forbidden, "FORBIDDEN"),
            (CommandErrorCode::OpenApi, "OPENAPI"),
            (CommandErrorCode::Mock, "MOCK"),
            (CommandErrorCode::PortBind, "PORT_BIND"),
            (CommandErrorCode::Test, "TEST"),
            (CommandErrorCode::Script, "SCRIPT"),
            (CommandErrorCode::Websocket, "WEBSOCKET"),
            (CommandErrorCode::Json, "JSON"),
            (CommandErrorCode::Decrypt, "DECRYPT"),
            (CommandErrorCode::Oauth2, "OAUTH2"),
            (CommandErrorCode::Cancelled, "CANCELLED"),
            (CommandErrorCode::Grpc, "GRPC"),
            (CommandErrorCode::Sync, "SYNC"),
            (CommandErrorCode::Internal, "INTERNAL"),
            (CommandErrorCode::Export, "EXPORT"),
            (CommandErrorCode::MockRunning, "MOCK_RUNNING"),
            (CommandErrorCode::AiProvider, "AI_PROVIDER"),
            (CommandErrorCode::AiNotConfigured, "AI_NOT_CONFIGURED"),
            (CommandErrorCode::AiNoPending, "AI_NO_PENDING"),
        ];
        for (code, expected) in cases {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "code {expected} 序列化漂移");
            assert_eq!(code.as_str(), expected);
        }
    }

    /// serde 的 SCREAMING_SNAKE_CASE 与 `as_str()` 手写映射必须一致，
    /// 防止后续新增变体时两处口径分叉。
    #[test]
    fn as_str_matches_serde_output() {
        let codes = [
            CommandErrorCode::Database,
            CommandErrorCode::DatabaseSchemaTooNew,
            CommandErrorCode::Io,
            CommandErrorCode::Http,
            CommandErrorCode::Timeout,
            CommandErrorCode::Ssl,
            CommandErrorCode::Dns,
            CommandErrorCode::Connection,
            CommandErrorCode::Validation,
            CommandErrorCode::NotFound,
            CommandErrorCode::Forbidden,
            CommandErrorCode::OpenApi,
            CommandErrorCode::Mock,
            CommandErrorCode::PortBind,
            CommandErrorCode::Test,
            CommandErrorCode::Script,
            CommandErrorCode::Websocket,
            CommandErrorCode::Json,
            CommandErrorCode::Decrypt,
            CommandErrorCode::Oauth2,
            CommandErrorCode::Cancelled,
            CommandErrorCode::Grpc,
            CommandErrorCode::Sync,
            CommandErrorCode::Internal,
            CommandErrorCode::Export,
            CommandErrorCode::MockRunning,
            CommandErrorCode::AiProvider,
            CommandErrorCode::AiNotConfigured,
            CommandErrorCode::AiNoPending,
            CommandErrorCode::HighRiskConfirmationRequired,
            CommandErrorCode::MockNotRunning,
            CommandErrorCode::Clipboard,
        ];
        for code in codes {
            let serde_str = serde_json::to_string(&code).unwrap();
            let as_str = format!("\"{}\"", code.as_str());
            assert_eq!(serde_str, as_str, "{:?} 的 as_str 与 serde 产物不一致", code);
        }
    }
}
