//! 错误类型。

/// AI 客户端错误（用户可见消息，直接展示于聊天面板）。
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("配置不完整：{0}")]
    Config(String),
    #[error("网络请求失败：{0}")]
    Http(String),
    #[error("AI 服务返回 {status}：{body}")]
    Api { status: u16, body: String },
    #[error("响应解析失败：{0}")]
    Parse(String),
    #[error("连接 AI 服务超时")]
    Timeout,
    #[error("已取消")]
    Cancelled,
}

impl AiError {
    /// 供上层判断是否为用户可自行修复的配置类错误（缺 Key / URL 不合法）。
    pub fn is_config(&self) -> bool {
        matches!(self, AiError::Config(_))
    }
}

// ---------------------------------------------------------------------------
// 统一错误收敛：AiError → AppError（单一映射点，命令层不再手拼错误码）
// ---------------------------------------------------------------------------

impl From<AiError> for miyo_core::AppError {
    fn from(err: AiError) -> Self {
        match err {
            // 配置类：命令层按 is_config() 分型为 AI_PROVIDER / AI_NOT_CONFIGURED
            AiError::Config(msg) => {
                miyo_core::AppError::Validation(format!("AI 配置不完整：{msg}"))
            }
            AiError::Http(msg) => {
                miyo_core::AppError::Connection(format!("AI 网络请求失败：{msg}"))
            }
            AiError::Api { status, body } => {
                miyo_core::AppError::Test(format!("AI 服务返回 {status}：{body}"))
            }
            AiError::Parse(msg) => miyo_core::AppError::Json(serde_json::Error::io(
                std::io::Error::other(format!("AI 响应解析失败：{msg}")),
            )),
            AiError::Timeout => miyo_core::AppError::NetworkTimeout("连接 AI 服务超时".into()),
            AiError::Cancelled => miyo_core::AppError::Cancelled("AI 任务已取消".into()),
        }
    }
}
