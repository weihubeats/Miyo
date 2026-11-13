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
