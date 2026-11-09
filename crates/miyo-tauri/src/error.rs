//! 统一错误类型：`AppError` → `CommandError` → 前端 JS Error。
//!
//! Tauri Command 只能返回 `Result<T, E>` 且 `E: Serialize`。这里将
//! `miyo_core::AppError` 映射为 `{ code, message }`：
//! - `code`：程序化错误码（前端可用 `err.code` 分支处理）；
//! - `message`：中文用户提示（`AppError::user_message`）。
//!
//! 前端 `invoke()` 被 reject 时，收到的即为该对象（见 `useMiyoApi.ts` 的 `toFoxError`）。

use miyo_core::AppError;

/// 命令层错误。`#[derive(Serialize)]` 使其可直接作为 Tauri Command 的 Err 传输。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CommandError {
    /// 程序化错误码（如 `VALIDATION`、`NOT_FOUND`、`DATABASE`）。
    pub code: &'static str,
    /// 中文用户提示。
    pub message: String,
}

impl CommandError {
    /// 参数校验类错误。
    pub fn validation(msg: impl Into<String>) -> Self {
        CommandError {
            code: "VALIDATION",
            message: msg.into(),
        }
    }

    /// 业务错误（保留原 code）。
    pub fn with_code(code: &'static str, msg: impl Into<String>) -> Self {
        CommandError {
            code,
            message: msg.into(),
        }
    }

    /// gRPC 调用类错误（连接 / 编解码 / 远端非零状态）。
    pub fn grpc(msg: impl Into<String>) -> Self {
        CommandError {
            code: "GRPC",
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

/// `AppError` → `CommandError` 的统一映射。
impl From<AppError> for CommandError {
    fn from(err: AppError) -> Self {
        let code = match &err {
            AppError::Database(_) => "DATABASE",
            AppError::SchemaTooNew(_) => "DATABASE_SCHEMA_TOO_NEW",
            AppError::Io(_) => "IO",
            AppError::Http(_) => "HTTP",
            AppError::NetworkTimeout(_) => "TIMEOUT",
            AppError::Ssl(_) => "SSL",
            AppError::Dns(_) => "DNS",
            AppError::Connection(_) => "CONNECTION",
            AppError::Validation(_) => "VALIDATION",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::OpenApi(_) => "OPENAPI",
            AppError::Mock(_) => "MOCK",
            AppError::Test(_) => "TEST",
            AppError::ScriptError(_) => "SCRIPT",
            AppError::WebSocket(_) => "WEBSOCKET",
            AppError::Json(_) => "JSON",
            AppError::Decryption(_) => "DECRYPT",
            AppError::OAuth2(_) => "OAUTH2",
            AppError::Cancelled(_) => "CANCELLED",
            AppError::Grpc(_) => "GRPC",
        };
        CommandError {
            code,
            message: err.user_message(),
        }
    }
}

/// Command 统一返回值。
pub type CommandResult<T> = std::result::Result<T, CommandError>;
