//! 剪贴板写入 Command：把文本直接写入系统剪贴板。
//!
//! 前端 WebView 的 `navigator.clipboard` 与 `execCommand('copy')` 在
//! macOS WKWebView 等多个环境下不可靠（非安全上下文 / 权限被拒 / 无用户手势），
//! 这里用 `arboard` 走原生系统剪贴板作为最终可靠兜底。

use crate::error::{CommandError, CommandErrorCode, CommandResult};

/// 写入系统剪贴板（跨平台，基于 arboard）。
#[tauri::command(rename_all = "camelCase")]
pub fn clipboard_write_text(text: String) -> CommandResult<()> {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .map_err(|e| {
            CommandError::with_code(
                CommandErrorCode::Clipboard,
                format!("写入系统剪贴板失败: {e}"),
            )
        })
}
