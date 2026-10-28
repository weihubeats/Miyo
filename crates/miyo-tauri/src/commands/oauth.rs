//! OAuth2 Command:授权(浏览器 + 本地回调 9090)/ 取 access token(带缓存与静默刷新)。

use tauri::State;

use miyo_core::model::{AuthSpec, OAuth2Token};
use miyo_oauth::client::{self, OAuth2Error};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 发起完整授权码流程：本地回调服务器 + 打开系统浏览器 + 换 token。
/// 返回的令牌由调用方写回 AuthSpec 持久化；进程内缓存由 miyo-oauth 维护。
#[tauri::command(rename_all = "camelCase")]
pub async fn oauth_authorize(
    _state: State<'_, AppState>,
    auth: AuthSpec,
) -> CommandResult<OAuth2Token> {
    client::authorize(&auth).await.map_err(CommandError::from)
}

/// 取 access token：缓存有效直接返回；临近过期静默刷新；无凭据报未授权。
#[tauri::command(rename_all = "camelCase")]
pub async fn oauth_access_token(
    _state: State<'_, AppState>,
    auth: AuthSpec,
) -> CommandResult<String> {
    client::access_token_for(&auth)
        .await
        .map_err(CommandError::from)
}

/// 非交互式获取令牌（client_credentials / password 模式）：直接向 token_url
/// 换取并返回（结果同时写入进程内缓存，发送时直接复用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn oauth_fetch_token(
    _state: State<'_, AppState>,
    auth: AuthSpec,
) -> CommandResult<OAuth2Token> {
    client::fetch_token(&auth).await.map_err(CommandError::from)
}

impl From<OAuth2Error> for CommandError {
    fn from(err: OAuth2Error) -> Self {
        CommandError {
            code: "OAUTH2",
            message: err.to_string(),
        }
    }
}
