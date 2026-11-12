//! 代码生成 Command:把接口请求渲染为 curl / python / js / go / java / php / rust 片段。

use miyo_codegen::{GenRequest, Lang};
use miyo_core::model::{AuthSpec, BodySpec, HttpMethod, KeyValue};
use tauri::State;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 渲染请求为指定语言代码。
///
/// `sanitize`（缺省 true）= 导出脱敏：敏感头 / auth / URL query / JSON body
/// 凭证替换为环境变量引用（`${MIYO_AUTHORIZATION}` 等），生成的代码可安全
/// 离开本机（提交仓库 / 发给同事）。false 仅限应用内预览明文。
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)] // lang/method/url/headers/body/auth/sanitize 均为独立入参
pub async fn codegen_render(
    _state: State<'_, AppState>,
    lang: String,
    method: HttpMethod,
    url: String,
    headers: Vec<KeyValue>,
    body: BodySpec,
    auth: AuthSpec,
    sanitize: Option<bool>,
) -> CommandResult<String> {
    let lang = Lang::from_str_cn(&lang).ok_or_else(|| CommandError::validation("未知代码语言"))?;
    let req = GenRequest {
        method: &method,
        url: &url,
        headers: &headers,
        body: &body,
        auth: &auth,
    };
    Ok(if sanitize.unwrap_or(true) {
        miyo_codegen::render_export(lang, &req)
    } else {
        miyo_codegen::render(lang, &req)
    })
}
