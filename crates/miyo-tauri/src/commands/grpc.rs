//! gRPC 调试 Command：服务目录（反射 / proto 编译）、unary 调用、
//! 服务端流生命周期与项目级 proto 文件管理。
//!
//! 事件约定：服务端流经 `miyo:grpc-event` 推送，载荷 =
//! `{stream_id, kind, ..}`（`kind` 为 message / end / failed，flatten 自
//! `GrpcEvent`）；关闭用 [`grpc_stream_close`]，或复用 `cancel_request`
//! （流取消令牌同时登记在 `request_cancels`，rid 机制与 HTTP 一致）。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use miyo_core::model::{KeyValue, ProtoFile, RequestHistory};
use miyo_core::VariableMap;
use miyo_grpc::{
    acquire_pool, spawn_server_stream, DescriptorPool, GrpcEvent, GrpcInvokeArgs as EngineArgs,
    GrpcResponse, GrpcSource, ProtoFileInput, ServiceCatalog,
};
use miyo_sync::redact::{is_sensitive_key, PLACEHOLDER};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 服务端流事件通道（前端 `listen("miyo:grpc-event")`）。
const GRPC_EVENT: &str = "miyo:grpc-event";

/// 服务目录入参（地址 / metadata 支持渲染前的 `{{变量}}` 模板）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcListServicesArgs {
    pub address: String,
    #[serde(default)]
    pub use_tls: bool,
    /// 反射请求附带的 metadata（认证服务器需要）。
    #[serde(default)]
    pub metadata: Vec<KeyValue>,
    /// 项目级 proto 文件 id（空 = 服务端反射）。
    #[serde(default)]
    pub proto_ids: Vec<String>,
    /// proto 文件归属项目（proto_ids 非空时必填）。
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub environment_id: Option<Uuid>,
    /// 强制重新编译 / 反射（忽略描述符池缓存）。
    #[serde(default)]
    pub force_reload: bool,
}

/// 列出服务目录：proto 文件编译（免连服务器）或服务端反射。
#[tauri::command(rename_all = "camelCase")]
pub async fn grpc_list_services(
    state: State<'_, AppState>,
    args: GrpcListServicesArgs,
) -> CommandResult<ServiceCatalog> {
    if args.address.trim().is_empty() {
        return Err(CommandError::validation("gRPC 地址不能为空"));
    }
    let vars = state.variables_for(args.environment_id).await?;
    let address = miyo_core::resolve_variables(&args.address, &vars);
    let metadata = render_metadata(&args.metadata, &vars);
    let pool = acquire_pool_for_args(
        &state,
        &address,
        args.use_tls,
        &metadata,
        &args.proto_ids,
        args.project_id,
        args.force_reload,
    )
    .await?;
    let source = if args.proto_ids.is_empty() {
        "reflection"
    } else {
        "proto"
    };
    Ok(ServiceCatalog::from_pool(&pool, source))
}

/// unary / 服务端流统一调用入口。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcInvokeRequest {
    /// gRPC 地址模板（host:port，可含 `{{变量}}` 与 scheme）。
    pub address: String,
    pub service: String,
    pub method: String,
    /// 请求消息 protobuf-JSON 文本（空 = 空消息）。
    #[serde(default)]
    pub message: String,
    /// metadata（即请求头，复用 HeadersPanel 数据）。
    #[serde(default)]
    pub metadata: Vec<KeyValue>,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default)]
    pub proto_ids: Vec<String>,
    /// proto 文件归属项目（proto_ids 非空时必填）。
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub environment_id: Option<Uuid>,
    /// unary 成功后历史归属（缺省不记历史）。
    #[serde(default)]
    pub endpoint_project_id: Option<Uuid>,
    #[serde(default)]
    pub endpoint_id: Option<Uuid>,
    /// unary / 流共用的取消标识（复用 `cancel_request`）。
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub force_reload: bool,
}

/// 调用结果：unary 直返响应；服务端流转 stream_id（消息走事件推送）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrpcInvokeResult {
    Unary { response: GrpcResponse },
    Stream { stream_id: String },
}

/// 执行 gRPC 调用：变量渲染 → 描述符池 → 按方法类型分流。
///
/// - unary：同步等待响应（成功且 grpc_status = 0 时落历史）；
/// - 服务端流：注册 stream_id 后台消费，逐消息 `miyo:grpc-event` 推送，
///   结束 / 失败事件后自动从注册表摘除。
#[tauri::command(rename_all = "camelCase")]
pub async fn grpc_invoke(
    app: AppHandle,
    state: State<'_, AppState>,
    args: GrpcInvokeRequest,
) -> CommandResult<GrpcInvokeResult> {
    if args.address.trim().is_empty() {
        return Err(CommandError::validation("gRPC 地址不能为空"));
    }
    if args.service.trim().is_empty() || args.method.trim().is_empty() {
        return Err(CommandError::validation("服务名与方法名不能为空"));
    }

    // 1. 变量加载与渲染（环境 > 项目 > 全局，与 HTTP 同口径）。
    let vars = state.variables_for(args.environment_id).await?;
    let address = miyo_core::resolve_variables(&args.address, &vars);
    let service = miyo_core::resolve_variables(&args.service, &vars);
    let method = miyo_core::resolve_variables(&args.method, &vars);
    let message = miyo_core::resolve_variables(&args.message, &vars);
    let metadata = render_metadata(&args.metadata, &vars);

    // 2. 描述符池（proto 编译或反射；地址只用于反射路径）。
    let pool = acquire_pool_for_args(
        &state,
        &address,
        args.use_tls,
        &metadata,
        &args.proto_ids,
        args.project_id,
        args.force_reload,
    )
    .await?;

    // 3. 解析方法（存在性 + 流式判定），组装引擎入参。
    let (_input, _output, client_streaming, server_streaming) =
        miyo_grpc::resolve_method(&pool, &service, &method)?;
    if client_streaming {
        return Err(CommandError::grpc(format!(
            "{service}.{method} 为客户端流方法：暂不支持（仅 unary / 服务端流）"
        )));
    }
    let timeout_ms = match args.timeout_ms {
        Some(ms) => Some(ms),
        // 服务端流不设总超时（生命周期由用户控制）；unary 兜底全局设置
        None if !server_streaming => Some(
            super::settings::read_http_timeout_ms(&state.db)
                .await?
                .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS),
        ),
        None => None,
    };
    let engine_args = EngineArgs {
        address,
        service: service.clone(),
        method: method.clone(),
        message_json: message,
        metadata,
        use_tls: args.use_tls,
        timeout_ms,
        pool,
    };

    if server_streaming {
        Ok(GrpcInvokeResult::Stream {
            stream_id: start_stream(&app, &state, engine_args, &args).await?,
        })
    } else {
        Ok(GrpcInvokeResult::Unary {
            response: invoke_unary_with_history(&state, engine_args, &args).await?,
        })
    }
}

/// unary 调用 + 历史落库（grpc_status = 0 时；尽力而为不阻断返回）。
async fn invoke_unary_with_history(
    state: &AppState,
    engine_args: EngineArgs,
    args: &GrpcInvokeRequest,
) -> CommandResult<GrpcResponse> {
    // 取消令牌：复用 request_cancels 注册表（cancel_request 直接可用）
    let token = args.request_id.as_ref().map(|id| {
        let token = CancellationToken::new();
        state
            .request_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), token.clone());
        (id.clone(), token)
    });

    let result = miyo_grpc::invoke_unary(&engine_args, token.as_ref().map(|(_, t)| t)).await;

    if let Some((id, _)) = &token {
        state
            .request_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    let response = result?;

    if response.grpc_status == 0 {
        if let Some(project_id) = args.endpoint_project_id {
            let history = build_grpc_history(project_id, args.endpoint_id, &engine_args, &response);
            let db = state.db.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = miyo_storage::repository::save_request_history(&db, &history).await
                {
                    tracing::warn!("[grpc_invoke] 保存历史失败：{}", e.user_message());
                }
            });
        }
    }
    Ok(response)
}

/// `miyo:grpc-event` 载荷：stream_id + 流事件（flatten 平铺，与 WS/SSE 口径一致）。
#[derive(Debug, Clone, Serialize)]
struct GrpcStreamEventPayload {
    stream_id: String,
    #[serde(flatten)]
    event: GrpcEvent,
}

/// 服务端流：注册取消令牌（request_cancels + grpc_streams）并 spawn 消费任务。
async fn start_stream(
    app: &AppHandle,
    state: &AppState,
    engine_args: EngineArgs,
    args: &GrpcInvokeRequest,
) -> CommandResult<String> {
    let stream_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    if let Some(rid) = &args.request_id {
        state
            .request_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(rid.clone(), cancel.clone());
    }

    let app_handle = app.clone();
    let event_stream_id = stream_id.clone();
    let cleanup_state = app.clone();
    let cleanup_rid = args.request_id.clone();
    let on_event = Arc::new(move |event: GrpcEvent| {
        let is_terminal = matches!(event, GrpcEvent::End { .. } | GrpcEvent::Failed { .. });
        let _ = app_handle.emit(
            GRPC_EVENT,
            &GrpcStreamEventPayload {
                stream_id: event_stream_id.clone(),
                event,
            },
        );
        // 结束 / 失败后自摘除注册表（回调在 tokio 任务内但为同步上下文，
        // 经 AppHandle 取托管状态；锁不跨 await，无死锁面）
        if is_terminal {
            if let Some(s) = cleanup_state.try_state::<AppState>() {
                s.grpc_streams
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&event_stream_id);
                if let Some(rid) = &cleanup_rid {
                    s.request_cancels
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(rid);
                }
            }
        }
    });
    let task = spawn_server_stream(engine_args, stream_id.clone(), cancel.clone(), on_event);

    state
        .grpc_streams
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(stream_id.clone(), (cancel, task));
    Ok(stream_id)
}

/// 关闭服务端流：触发取消令牌 + 中止消费任务（不存在时返回 false）。
#[tauri::command(rename_all = "camelCase")]
pub fn grpc_stream_close(state: State<'_, AppState>, stream_id: String) -> CommandResult<bool> {
    let entry = state
        .grpc_streams
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&stream_id);
    let Some((cancel, task)) = entry else {
        return Ok(false);
    };
    cancel.cancel();
    task.abort();
    Ok(true)
}

// ---------- proto 文件管理（项目级） ----------

/// proto 文件保存载荷（upsert 式：新增/修改一并提交，删除单独调）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtoFileInputDto {
    /// 已有文件传原 id；新文件留空由后端生成。
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub content: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn grpc_list_proto_files(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<ProtoFile>> {
    Ok(miyo_storage::repository::list_proto_files(&state.db, project_id).await?)
}

/// 批量保存项目级 proto 文件（upsert；id 缺省生成），返回保存后的完整列表。
#[tauri::command(rename_all = "camelCase")]
pub async fn grpc_save_proto_files(
    state: State<'_, AppState>,
    project_id: Uuid,
    files: Vec<ProtoFileInputDto>,
) -> CommandResult<Vec<ProtoFile>> {
    if !files.is_empty() {
        let now = chrono::Utc::now();
        let mut models = Vec::with_capacity(files.len());
        for f in files {
            let name = f.name.trim().to_string();
            if name.is_empty() {
                return Err(CommandError::validation("proto 文件名不能为空"));
            }
            if f.content.trim().is_empty() {
                return Err(CommandError::validation(format!(
                    "proto 文件内容不能为空（{name}）"
                )));
            }
            let id = f.id.unwrap_or_else(Uuid::new_v4);
            models.push(ProtoFile {
                id,
                project_id,
                name,
                content: f.content,
                created_at: now,
                updated_at: now,
            });
        }
        miyo_storage::repository::save_proto_files(&state.db, &models).await?;
    }
    Ok(miyo_storage::repository::list_proto_files(&state.db, project_id).await?)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn grpc_delete_proto_file(
    state: State<'_, AppState>,
    proto_id: Uuid,
) -> CommandResult<()> {
    Ok(miyo_storage::repository::delete_proto_file(&state.db, proto_id).await?)
}

// ---------- 内部工具 ----------

/// 按入参获取描述符池：proto_ids 非空走 protox 编译（从库加载文件集），
/// 否则走服务端反射。地址与 metadata 应已按环境渲染（两个调用方共用）。
async fn acquire_pool_for_args(
    state: &AppState,
    address: &str,
    use_tls: bool,
    metadata: &[(String, String)],
    proto_ids: &[String],
    project_id: Option<Uuid>,
    force_reload: bool,
) -> CommandResult<Arc<DescriptorPool>> {
    let source = if proto_ids.is_empty() {
        GrpcSource::Reflection
    } else {
        let project_id = project_id
            .ok_or_else(|| CommandError::validation("使用 proto 文件时必须指定所属项目"))?;
        let files = miyo_storage::repository::list_proto_files(&state.db, project_id).await?;
        let selected: Vec<ProtoFileInput> = proto_ids
            .iter()
            .filter_map(|id| {
                Uuid::parse_str(id).ok().and_then(|pid| {
                    files.iter().find(|f| f.id == pid).map(|f| ProtoFileInput {
                        id: f.id.to_string(),
                        name: f.name.clone(),
                        content: f.content.clone(),
                    })
                })
            })
            .collect();
        if selected.is_empty() {
            return Err(CommandError::grpc(
                "未找到所选 proto 文件（可能已被删除），请重新选择或改用服务端反射",
            ));
        }
        GrpcSource::Files(selected)
    };
    let pool = acquire_pool(source, address, use_tls, metadata, force_reload)
        .await
        .map_err(CommandError::from)?;
    Ok(pool)
}

/// 渲染 metadata（启用项；key/value 支持 `{{变量}}`）。
fn render_metadata(items: &[KeyValue], vars: &VariableMap) -> Vec<(String, String)> {
    items
        .iter()
        .filter(|kv| kv.enabled)
        .map(|kv| {
            (
                miyo_core::resolve_variables(&kv.key, vars),
                miyo_core::resolve_variables(&kv.value, vars),
            )
        })
        .collect()
}

/// gRPC unary 历史记录（复用 request_histories 表，method = "GRPC"）。
///
/// - `url` 存渲染后地址拼 `pkg.Service/Method`（列表展示 / 搜索口径与
///   HTTP 一致，service/method 一眼可辨）；
/// - `request_summary_json.spec.grpc` 保存完整调用配置（渲染后），
///   `url` 字段存地址模板（恢复编辑器保留变量语义）。
fn build_grpc_history(
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    args: &EngineArgs,
    response: &GrpcResponse,
) -> RequestHistory {
    // 响应预览按字节截断（字符边界安全），口径与 HTTP 历史一致
    let body_preview: &str = byte_truncate(&response.message_json, 2000);
    // metadata 即 gRPC 的请求头：敏感键（authorization / token / *-key…）
    // 的值脱敏后落历史库，口径与 HTTP 历史 redact_history_spec 一致。
    let metadata_redacted: Vec<serde_json::Value> = args
        .metadata
        .iter()
        .map(|(key, value)| {
            serde_json::json!({
                "key": key,
                "value": if !value.is_empty() && is_sensitive_key(key) {
                    PLACEHOLDER.to_string()
                } else {
                    value.clone()
                },
            })
        })
        .collect();
    let request_summary = serde_json::json!({
        "method": "GRPC",
        "url": args.address,
        "spec": {
            "grpc": {
                "service": args.service,
                "method": args.method,
                "message": args.message_json,
                "use_tls": args.use_tls,
                "metadata": metadata_redacted,
                "timeout_ms": args.timeout_ms,
            }
        },
    });
    let response_summary = serde_json::json!({
        "grpc_status": response.grpc_status,
        "grpc_message": response.grpc_message,
        "duration_ms": response.duration_ms,
        "size_bytes": response.size_bytes,
        "body": body_preview,
    });
    RequestHistory {
        id: Uuid::new_v4(),
        project_id,
        endpoint_id,
        method: "GRPC".into(),
        url: format!("{}/{}/{}", args.address, args.service, args.method),
        status: None,
        duration_ms: Some(response.duration_ms.round() as u64),
        request_summary_json: request_summary.to_string(),
        response_summary_json: response_summary.to_string(),
        created_at: chrono::Utc::now(),
    }
}

/// 按字节上限截断（保证字符边界），口径同 `commands::request::byte_truncate`。
fn byte_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_args(service: &str, method: &str) -> EngineArgs {
        EngineArgs {
            address: "127.0.0.1:50051".into(),
            service: service.into(),
            method: method.into(),
            message_json: "{}".into(),
            metadata: vec![("x-token".into(), "tok".into())],
            use_tls: false,
            timeout_ms: Some(3000),
            pool: Arc::new(DescriptorPool::new()),
        }
    }

    /// 历史记录：method=GRPC、url 拼接 service/method、summary 含 grpc 配置。
    #[test]
    fn grpc_history_shape() {
        let args = engine_args("user.v1.UserService", "GetUser");
        let response = GrpcResponse {
            message_json: r#"{"id":"1"}"#.into(),
            metadata: vec![("grpc-status".into(), "0".into())],
            grpc_status: 0,
            grpc_message: String::new(),
            duration_ms: 12.5,
            size_bytes: 8,
        };
        let history = build_grpc_history(Uuid::new_v4(), None, &args, &response);
        assert_eq!(history.method, "GRPC");
        assert_eq!(history.url, "127.0.0.1:50051/user.v1.UserService/GetUser");
        assert_eq!(history.status, None);
        assert_eq!(history.duration_ms, Some(13));
        let summary: serde_json::Value =
            serde_json::from_str(&history.request_summary_json).unwrap();
        assert_eq!(summary["method"], "GRPC");
        assert_eq!(summary["url"], "127.0.0.1:50051");
        assert_eq!(summary["spec"]["grpc"]["service"], "user.v1.UserService");
        assert_eq!(summary["spec"]["grpc"]["metadata"][0][0], "x-token");
        let resp_summary: serde_json::Value =
            serde_json::from_str(&history.response_summary_json).unwrap();
        assert_eq!(resp_summary["grpc_status"], 0);
        assert_eq!(resp_summary["body"], r#"{"id":"1"}"#);
    }

    /// 响应体按字节截断入历史（字符边界安全）。
    #[test]
    fn grpc_history_truncates_long_body() {
        let args = engine_args("svc", "M");
        let long = "汉".repeat(3000);
        let response = GrpcResponse {
            message_json: long.clone(),
            metadata: Vec::new(),
            grpc_status: 0,
            grpc_message: String::new(),
            duration_ms: 1.0,
            size_bytes: long.len() as u64,
        };
        let history = build_grpc_history(Uuid::new_v4(), None, &args, &response);
        let summary: serde_json::Value =
            serde_json::from_str(&history.response_summary_json).unwrap();
        let body = summary["body"].as_str().unwrap();
        assert!(body.len() <= 2000);
        assert!(body.chars().all(|c| c == '汉'), "截断不得撕裂字符");
    }
}
