//! 接口 Command：列表 / 详情 / 保存（含参数校验）/ 删除 / 复制。

use tauri::State;
use uuid::Uuid;

use miyo_core::model::{Endpoint, HttpMethod};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 列出项目下的全部接口。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_endpoints(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<Endpoint>> {
    repo::list_endpoints(&state.db, project_id)
        .await
        .map_err(Into::into)
}

/// 接口详情。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_endpoint(
    state: State<'_, AppState>,
    endpoint_id: Uuid,
) -> CommandResult<Endpoint> {
    repo::get_endpoint(&state.db, endpoint_id)
        .await
        .map_err(Into::into)
}

/// 保存接口（upsert）。
///
/// 参数校验：名称必填、路径必填且为 `/相对路径` 或绝对 URL。
///
/// 绝对 URL 分支：环境激活时 cURL 导入把 origin+path 整条存入 `path`
/// （见前端 `openCurlDraft`），发送走绝对地址直达导入主机——后端必须放行，
/// 口径与前端 `saveActiveDraft` 的 `/…` 或 `https?://…` 校验一致。
///
/// gRPC 分支：gRPC 端点的 `path` 存的是**服务端地址**（如 `grpcb.in:9000`，
/// 见 `GrpcInvokeRequest.address`），它不是 URI 路径，不符合 `/…` 或 URL 口径。
/// 跳过路径校验只放行「非空」——地址形态（host[:port] / scheme://）交给
/// gRPC 建连时判定并给出更准确的报错，否则拖拽排序等「无路径改动」的保存
/// 会被一条与用户操作无关的校验拦住（实测：gRPC 接口拖动即报「移动失败」）。
fn validate_endpoint_shape(method: &HttpMethod, name: &str, path: &str) -> CommandResult<()> {
    if name.trim().is_empty() {
        return Err(CommandError::validation("接口名称不能为空"));
    }
    if path.trim().is_empty() {
        if method.is_grpc() {
            return Err(CommandError::validation("gRPC 地址不能为空"));
        }
        return Err(CommandError::validation("接口路径不能为空"));
    }
    if method.is_grpc() {
        return Ok(());
    }
    let p = path.trim();
    let lower = p.to_ascii_lowercase();
    let absolute = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ws://")
        || lower.starts_with("wss://");
    if !p.starts_with('/') && !absolute {
        return Err(CommandError::validation(
            "接口路径必须以 / 开头或为完整 URL",
        ));
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_endpoint(
    state: State<'_, AppState>,
    endpoint: Endpoint,
) -> CommandResult<Endpoint> {
    validate_endpoint_shape(&endpoint.method, &endpoint.name, &endpoint.path)?;
    repo::save_endpoint(&state.db, &endpoint).await?;
    repo::touch_project(&state.db, &endpoint.project_id).await?;
    Ok(endpoint)
}

/// 删除接口。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_endpoint(state: State<'_, AppState>, endpoint_id: Uuid) -> CommandResult<()> {
    repo::delete_endpoint(&state.db, endpoint_id)
        .await
        .map_err(Into::into)
}

/// 复制接口（返回新接口）。
#[tauri::command(rename_all = "camelCase")]
pub async fn duplicate_endpoint(
    state: State<'_, AppState>,
    endpoint_id: Uuid,
) -> CommandResult<Endpoint> {
    repo::duplicate_endpoint(&state.db, endpoint_id)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径校验口径：`/相对路径` 与绝对 URL（cURL 导入环境激活分支）放行，其余拒绝。
    #[test]
    fn endpoint_path_validation_accepts_relative_and_absolute() {
        let get = HttpMethod::GET;
        // 相对路径
        assert!(validate_endpoint_shape(&get, "A", "/users").is_ok());
        // 绝对 URL（环境激活时 cURL 导入存整条 URL——回归：曾被后端拒绝无法保存）
        assert!(validate_endpoint_shape(&get, "A", "https://api.example.com/users").is_ok());
        assert!(validate_endpoint_shape(&get, "A", "HTTP://api.example.com/users").is_ok());
        assert!(validate_endpoint_shape(&get, "A", "wss://ws.example.com/sock").is_ok());
        // 拒绝：空名 / 空路径 / 既非斜杠也非绝对 URL
        assert!(validate_endpoint_shape(&get, "  ", "/x").is_err());
        assert!(validate_endpoint_shape(&get, "A", "   ").is_err());
        assert!(validate_endpoint_shape(&get, "A", "users").is_err());
    }

    /// gRPC 端点的 `path` 存的是服务端地址（`host:port`），不是 URI 路径。
    /// 回归：曾被 HTTP 路径口径拒绝，导致拖动排序（会触发保存）报「移动失败」。
    #[test]
    fn grpc_address_is_not_validated_as_uri_path() {
        let grpc = HttpMethod::Grpc;
        assert!(validate_endpoint_shape(&grpc, "A", "grpcb.in:9000").is_ok());
        assert!(validate_endpoint_shape(&grpc, "A", "https://grpc.example.com:443").is_ok());
        assert!(validate_endpoint_shape(&grpc, "A", "dns:///svc.local:50051").is_ok());
        // 名称与地址仍不得为空
        assert!(validate_endpoint_shape(&grpc, "A", "  ").is_err());
        assert!(validate_endpoint_shape(&grpc, "  ", "grpcb.in:9000").is_err());
    }
}
