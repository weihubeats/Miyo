//! reqwest 错误分类（自 miyo-core 迁出——Domain 层不再依赖 reqwest）。
//!
//! `from_reqwest` 是所有 reqwest 错误进 AppError 的唯一入口：分类顺序
//! 超时 → DNS → TLS → 连接 → 构建参数，兜底 `AppError::Http(String 明细)`。

use miyo_core::AppError;

/// 只做分类（借用错误，无所有权）。无法识别时返回 `None`，由调用方兜底。
pub fn classify(e: &reqwest::Error) -> Option<AppError> {
    let chain = source_chain(e);
    if e.is_timeout() {
        Some(AppError::NetworkTimeout(e.to_string()))
    } else if chain.contains("lookup")
        || chain.contains("name or service not known")
        || chain.contains("dns")
        || (chain.contains("resolve") && !chain.contains("certificate"))
    {
        Some(AppError::Dns(e.to_string()))
    } else if chain.contains("certificate") || chain.contains("tls") {
        Some(AppError::Ssl(e.to_string()))
    } else if e.is_connect() {
        Some(AppError::Connection(e.to_string()))
    } else if e.is_builder() {
        Some(AppError::Validation(
            "HTTP 请求参数非法，无法构建请求".to_string(),
        ))
    } else {
        None
    }
}

fn source_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string().to_lowercase()];
    let mut cur = e.source();
    while let Some(cause) = cur {
        parts.push(cause.to_string().to_lowercase());
        cur = cause.source();
    }
    parts.join(" | ")
}

/// reqwest 错误 → AppError 的唯一入口（原 miyo-core::AppError::from_reqwest）。
pub fn from_reqwest(e: reqwest::Error) -> AppError {
    // reqwest 错误 Display 携带完整请求 URL（`?token=…` 类 query 凭据会随
    // 错误消息进入日志 / AI 上下文 / 前端 toast）：统一在入口脱敏。
    classify(&e)
        .map(|a| a.sanitize_message_urls())
        .unwrap_or_else(|| AppError::Http(e.to_string()))
}
