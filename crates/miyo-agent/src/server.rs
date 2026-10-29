//! Agent 控制面 HTTP 服务：axum 路由 + Bearer 令牌鉴权。
//!
//! 端口从 4110 起探测（避开 Mock 的 4010~4029 段），只绑定回环地址。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use miyo_core::model::{Endpoint, Project, RequestSpec, Scenario};
use miyo_storage::repository as repo;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::import::endpoint_from_curl;

/// 默认端口（4110 起探测；与 Mock 的 4010~4029 错开）。用户可在设置中改起始端口。
pub const DEFAULT_AGENT_PORT: u16 = 4110;

/// MCP 工具注册表：名称 → 读 / 写类别。设置页的「工具权限」与控制面、
/// miyo-mcp 三处共用此清单；停用按名存于 settings（[`MCP_DISABLED_TOOLS_KEY`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolDef {
    pub name: &'static str,
    /// read = 只读信息；write = 会落库。
    pub kind: &'static str,
}

pub const MCP_TOOLS: &[McpToolDef] = &[
    McpToolDef {
        name: "save_curl",
        kind: "write",
    },
    McpToolDef {
        name: "list_projects",
        kind: "read",
    },
    McpToolDef {
        name: "list_endpoints",
        kind: "read",
    },
    McpToolDef {
        name: "list_scenarios",
        kind: "read",
    },
    McpToolDef {
        name: "get_scenario",
        kind: "read",
    },
    McpToolDef {
        name: "save_scenario_script",
        kind: "write",
    },
    McpToolDef {
        name: "agent_info",
        kind: "read",
    },
];

/// settings 表中「已停用 MCP 工具」的键；值为 JSON 字符串数组。
pub const MCP_DISABLED_TOOLS_KEY: &str = "mcp_disabled_tools";

/// GET `/agent/tools` 列表项。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolState {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}
/// 起始端口被占用时最多尝试的次数（自定义端口同样顺延）。
pub const MAX_PORT_TRIES: u16 = 20;

/// 导入成功后广播给 UI 层的事件。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AgentEvent {
    /// 一个接口由 Agent 导入成功。
    EndpointImported {
        endpoint_id: uuid::Uuid,
        project_id: uuid::Uuid,
        name: String,
    },
}

/// 服务共享状态：连接池 + 鉴权令牌 + 事件广播。
#[derive(Clone)]
pub struct AgentState {
    pub db: SqlitePool,
    token: Arc<String>,
    events: broadcast::Sender<AgentEvent>,
}

impl AgentState {
    pub fn new(db: SqlitePool, token: impl Into<String>) -> Self {
        let (events, _) = broadcast::channel(64);
        AgentState {
            db,
            token: Arc::new(token.into()),
            events,
        }
    }

    /// 订阅事件（需在 `start` 前调用，避免漏掉最早的导入）。
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    /// 已停用的 MCP 工具名集合（每次实时读 settings：开关即时生效、免重启）。
    pub async fn disabled_tools(&self) -> Vec<String> {
        match repo::get_setting(&self.db, MCP_DISABLED_TOOLS_KEY).await {
            Ok(Some(raw)) => serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// 工具是否被停用（[`Self::disabled_tools`] 的单工具版）。
    pub async fn tool_disabled(&self, name: &str) -> bool {
        self.disabled_tools().await.iter().any(|n| n == name)
    }
}

// ---------- 请求 / 响应体 ----------

/// POST `/agent/curl` 请求体。字段同时接受 camelCase 与 snake_case。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCurlRequest {
    /// 完整 cURL 命令字符串。
    pub command: String,
    /// 目标项目 id；缺省时：仅一个项目 → 自动选中，零个项目 → 创建「Agent 导入」，
    /// 多个项目 → 返回错误并列出候选。
    #[serde(alias = "project_id")]
    pub project_id: Option<uuid::Uuid>,
    /// 接口名称；缺省从 URL 路径推导。
    #[serde(default)]
    pub name: Option<String>,
    /// 归属文件夹 id（可选）。
    #[serde(alias = "folder_id")]
    pub folder_id: Option<uuid::Uuid>,
}

/// POST `/agent/curl` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCurlResult {
    pub endpoint_id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    pub method: String,
    pub path: String,
    /// 项目变量 `base_url` 的最终值（可能因本次导入而新建）。
    pub base_url: String,
    /// 非致命提示（如项目已有不同 base_url），Agent 应转达用户。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

// ---------- 场景（脚本协作）DTO ----------

/// GET `/agent/scenarios` 列表项。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSummary {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    pub description: String,
    pub step_count: usize,
    /// 是否存在自定义脚本（scenario.script 非空）。
    pub has_custom_script: bool,
    pub updated_at: String,
}

impl ScenarioSummary {
    fn from_scenario(s: &Scenario) -> Self {
        ScenarioSummary {
            id: s.id,
            project_id: s.project_id,
            name: s.name.clone(),
            description: s.description.clone(),
            step_count: s.steps.len(),
            has_custom_script: s.script.as_deref().is_some_and(|v| !v.trim().is_empty()),
            updated_at: s.updated_at.to_rfc3339(),
        }
    }
}

/// 场景步骤 + 引用接口的当前配置（AI 写脚本的完整上下文）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioStepDetail {
    pub endpoint_id: uuid::Uuid,
    pub endpoint_name: String,
    pub method: String,
    /// 接口当前路径（含 {{var}} 占位，未做路径变量代入）。
    pub path: String,
    pub enabled: bool,
    pub extract: serde_json::Value,
    pub assertions: serde_json::Value,
    /// 接口当前请求规格（头 / Body / 认证形态），写脚本时照抄。
    pub request: RequestSpec,
}

/// GET `/agent/scenarios/{id}` 详情。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDetail {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    pub description: String,
    pub steps: Vec<ScenarioStepDetail>,
    /// 启用步骤将提取的变量名（链路变量，按步序去重）。
    pub chain_variables: Vec<String>,
    /// 项目变量（含 base_url——脚本里服务地址的兜底）。
    pub project_variables: std::collections::HashMap<String, String>,
    /// 当前自定义脚本；None = 未自定义（视图实时由步骤生成）。
    pub script: Option<String>,
}

/// PUT `/agent/scenarios/{id}/script` 请求体。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveScenarioScriptRequest {
    /// 自定义脚本全文；null / 空 = 清除自定义（回到实时生成）。
    #[serde(default)]
    pub script: Option<String>,
}

/// PUT `/agent/scenarios/{id}/script` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveScenarioScriptResult {
    pub id: uuid::Uuid,
    /// true = 已写入自定义脚本；false = 已清除。
    pub script_set: bool,
}

/// 统一错误响应体 `{ code, message }`。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiErrorBody>)>;

fn api_error(err: miyo_core::AppError) -> (StatusCode, Json<ApiErrorBody>) {
    let (status, code) = match &err {
        miyo_core::AppError::Validation(_) | miyo_core::AppError::Json(_) => {
            (StatusCode::BAD_REQUEST, "VALIDATION")
        }
        miyo_core::AppError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    };
    (
        status,
        Json(ApiErrorBody {
            code: code.to_string(),
            message: err.user_message(),
        }),
    )
}

// ---------- 中间件 ----------

/// 令牌常数时间比较：长度先行比较（长度本身非秘密），等长时逐字节异或
/// 累加后统一判零——与 `!=` 的早退比较不同，比较耗时不随匹配前缀变化，
/// 避免被计时侧信道逐字节猜出 token。手写实现，不引入新依赖。
fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    if provided.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in provided.as_bytes().iter().zip(expected.as_bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

async fn auth(State(state): State<AgentState>, req: Request, next: Next) -> Response {
    let provided = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| {
            req.headers()
                .get("x-agent-token")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        });
    // 不泄露 token 内容，只记录是否缺失。
    if !token_matches(provided.as_deref(), state.token.as_str()) {
        tracing::warn!("[agent] 未授权请求：{} {}", req.method(), req.uri().path());
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorBody {
                code: "UNAUTHORIZED".into(),
                message: "缺少或错误的令牌（Authorization: Bearer <token>）".into(),
            }),
        )
            .into_response();
    }
    next.run(req).await
}

// ---------- Handlers ----------

async fn health() -> &'static str {
    "ok"
}

/// 工具停用守卫：停用后控制面路由直接 403（MCP 层的 tools/list 过滤是
/// 第一层，这里是绕过 MCP 直连 HTTP 的兜底）。
async fn ensure_tool_enabled(
    state: &AgentState,
    name: &str,
) -> Result<(), (StatusCode, Json<ApiErrorBody>)> {
    if state.tool_disabled(name).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorBody {
                code: "TOOL_DISABLED".into(),
                message: format!("工具 {name} 已在 Miyo 设置 → MCP 服务中停用"),
            }),
        ));
    }
    Ok(())
}

/// GET `/agent/tools`：MCP 工具及启停状态（miyo-mcp 的 tools/list 过滤数据源）。
async fn list_tools_handler(State(state): State<AgentState>) -> ApiResult<Vec<McpToolState>> {
    let disabled = state.disabled_tools().await;
    Ok(Json(
        MCP_TOOLS
            .iter()
            .map(|t| McpToolState {
                name: t.name.to_string(),
                kind: t.kind.to_string(),
                enabled: !disabled.iter().any(|n| n == t.name),
            })
            .collect(),
    ))
}

async fn list_projects_handler(State(state): State<AgentState>) -> ApiResult<Vec<Project>> {
    ensure_tool_enabled(&state, "list_projects").await?;
    repo::list_projects(&state.db)
        .await
        .map(Json)
        .map_err(api_error)
}

async fn list_endpoints_handler(
    State(state): State<AgentState>,
    AxumPath(project_id): AxumPath<uuid::Uuid>,
) -> ApiResult<Vec<Endpoint>> {
    ensure_tool_enabled(&state, "list_endpoints").await?;
    repo::list_endpoints(&state.db, project_id)
        .await
        .map(Json)
        .map_err(api_error)
}

// ---------- 场景（脚本协作）Handlers ----------

/// 读路径的项目解析：projectId 缺省时唯一项目自动选中（多项目报错列候选；
/// 不像导入路径那样自动建项目——GET 不应有写副作用）。
async fn resolve_project_read(
    db: &SqlitePool,
    project_id: Option<uuid::Uuid>,
) -> Result<Project, miyo_core::AppError> {
    if let Some(id) = project_id {
        return repo::get_project(db, id).await;
    }
    let projects = repo::list_projects(db).await?;
    match projects.len() {
        1 => Ok(projects.into_iter().next().unwrap()),
        n => Err(miyo_core::AppError::Validation(format!(
            "存在 {n} 个项目且未指定 projectId，请先 GET /agent/projects 后在请求中指定。候选：{}",
            projects
                .iter()
                .take(5)
                .map(|p| format!("{}({})", p.name, p.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

async fn list_scenarios_handler(
    State(state): State<AgentState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Vec<ScenarioSummary>> {
    ensure_tool_enabled(&state, "list_scenarios").await?;
    let project_id = match params.get("projectId").filter(|s| !s.is_empty()) {
        Some(raw) => Some(raw.parse::<uuid::Uuid>().map_err(|_| {
            api_error(miyo_core::AppError::Validation(
                "projectId 不是合法 UUID".into(),
            ))
        })?),
        None => None,
    };
    let project = resolve_project_read(&state.db, project_id)
        .await
        .map_err(api_error)?;
    let scenarios = repo::list_scenarios(&state.db, project.id)
        .await
        .map_err(api_error)?;
    Ok(Json(
        scenarios
            .iter()
            .map(ScenarioSummary::from_scenario)
            .collect(),
    ))
}

async fn get_scenario_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<ScenarioDetail> {
    ensure_tool_enabled(&state, "get_scenario").await?;
    let scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    let mut steps = Vec::with_capacity(scenario.steps.len());
    let mut chain_variables: Vec<String> = Vec::new();
    for step in &scenario.steps {
        // 引用式步骤：接口已删除时保留占位（AI 生成脚本可据此提示用户）。
        let (endpoint_name, method, path, request) =
            match repo::get_endpoint(&state.db, step.endpoint_id).await {
                Ok(ep) => (ep.name, ep.method.to_string(), ep.path, ep.request),
                Err(_) => (
                    "（接口已删除）".to_string(),
                    "GET".to_string(),
                    String::new(),
                    RequestSpec::default(),
                ),
            };
        if step.enabled {
            if let Some(rules) = step.extract.as_array() {
                for rule in rules {
                    if let Some(n) = rule.get("name").and_then(|v| v.as_str()) {
                        if !n.trim().is_empty() && !chain_variables.iter().any(|v| v == n) {
                            chain_variables.push(n.to_string());
                        }
                    }
                }
            }
        }
        steps.push(ScenarioStepDetail {
            endpoint_id: step.endpoint_id,
            endpoint_name,
            method,
            path,
            enabled: step.enabled,
            extract: step.extract.clone(),
            assertions: step.assertions.clone(),
            request,
        });
    }
    let project = repo::get_project(&state.db, scenario.project_id)
        .await
        .map_err(api_error)?;
    Ok(Json(ScenarioDetail {
        id: scenario.id,
        project_id: scenario.project_id,
        name: scenario.name,
        description: scenario.description,
        steps,
        chain_variables,
        project_variables: project.variables,
        script: scenario.script,
    }))
}

async fn save_scenario_script_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Json(body): Json<SaveScenarioScriptRequest>,
) -> ApiResult<SaveScenarioScriptResult> {
    ensure_tool_enabled(&state, "save_scenario_script").await?;
    let mut scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    let trimmed = body
        .script
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let script_set = trimmed.is_some();
    scenario.script = trimmed.map(String::from);
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(SaveScenarioScriptResult { id, script_set }))
}

/// 解析目标项目：显式 id 校验存在性；缺省时按「唯一项目自动选中 /
/// 零项目自动创建 / 多项目报错列出候选」的规则收敛。
async fn resolve_project(
    db: &SqlitePool,
    project_id: Option<uuid::Uuid>,
) -> Result<Project, miyo_core::AppError> {
    if let Some(id) = project_id {
        return repo::get_project(db, id).await;
    }
    let projects = repo::list_projects(db).await?;
    match projects.len() {
        0 => {
            let now = chrono::Utc::now();
            let project = Project {
                id: uuid::Uuid::new_v4(),
                name: "Agent 导入".into(),
                description: "由 AI Agent 自动创建".into(),
                variables: Default::default(),
                team_id: Some(miyo_core::model::DEFAULT_TEAM_ID),
                created_at: now,
                updated_at: now,
            };
            repo::save_project(db, &project).await?;
            Ok(project)
        }
        1 => Ok(projects.into_iter().next().unwrap()),
        n => Err(miyo_core::AppError::Validation(format!(
            "存在 {n} 个项目且未指定 projectId，请先 GET /agent/projects 后在请求中指定。候选：{}",
            projects
                .iter()
                .take(5)
                .map(|p| format!("{}({})", p.name, p.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// 维护项目变量 `base_url`：
/// - 缺失 → 写入本次导入的 origin（返回新值）；
/// - 已有且一致 → 原样返回；
/// - 已有但不一致 → 不覆盖用户配置，返回 warning 由 Agent 转达。
async fn ensure_base_url(
    db: &SqlitePool,
    project: &Project,
    origin: &str,
) -> Result<(String, Option<String>), miyo_core::AppError> {
    match project.variables.get("base_url") {
        Some(existing) if existing == origin => Ok((origin.to_string(), None)),
        Some(existing) => Ok((
            existing.clone(),
            Some(format!(
                "项目「{}」的 base_url 为 {existing}，与导入 URL 的来源 {origin} 不一致；接口已按相对路径保存，发送前请确认 base_url。",
                project.name
            )),
        )),
        None => {
            let mut updated = project.clone();
            updated.variables.insert("base_url".into(), origin.to_string());
            updated.updated_at = chrono::Utc::now();
            repo::save_project(db, &updated).await?;
            Ok((origin.to_string(), None))
        }
    }
}

async fn import_curl_handler(
    State(state): State<AgentState>,
    Json(req): Json<ImportCurlRequest>,
) -> ApiResult<ImportCurlResult> {
    ensure_tool_enabled(&state, "save_curl").await?;
    let parsed = miyo_core::parse_curl(&req.command).map_err(api_error)?;
    let project = resolve_project(&state.db, req.project_id)
        .await
        .map_err(api_error)?;
    let (mut endpoint, origin) =
        endpoint_from_curl(parsed, project.id, req.folder_id, req.name).map_err(api_error)?;
    let (base_url, warning) = ensure_base_url(&state.db, &project, &origin)
        .await
        .map_err(api_error)?;

    repo::save_endpoint(&state.db, &endpoint)
        .await
        .map_err(api_error)?;
    repo::touch_project(&state.db, &endpoint.project_id)
        .await
        .map_err(api_error)?;

    let _ = state.events.send(AgentEvent::EndpointImported {
        endpoint_id: endpoint.id,
        project_id: endpoint.project_id,
        name: endpoint.name.clone(),
    });
    tracing::info!(
        "[agent] cURL 已导入为接口 {} {}（项目 {}）",
        endpoint.method.as_str(),
        endpoint.path,
        project.name
    );

    Ok(Json(ImportCurlResult {
        endpoint_id: endpoint.id,
        project_id: endpoint.project_id,
        name: std::mem::take(&mut endpoint.name),
        method: endpoint.method.as_str().to_string(),
        path: std::mem::take(&mut endpoint.path),
        base_url,
        warning,
    }))
}

/// 构建路由（pub 供 miyo-mcp 集成测试复用）。
pub fn build_router(state: AgentState) -> Router {
    Router::new()
        .route("/agent/health", get(health))
        .route("/agent/projects", get(list_projects_handler))
        .route("/agent/endpoints/{project_id}", get(list_endpoints_handler))
        .route("/agent/curl", post(import_curl_handler))
        .route("/agent/scenarios", get(list_scenarios_handler))
        .route("/agent/scenarios/{id}", get(get_scenario_handler))
        .route(
            "/agent/scenarios/{id}/script",
            put(save_scenario_script_handler),
        )
        .route("/agent/tools", get(list_tools_handler))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

/// 正在运行的 Agent 服务句柄。
pub struct AgentServer {
    pub port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AgentServer {
    pub fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 停止服务（幂等）。
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(());
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }
}

/// 启动服务：默认从 4110 起依次尝试（测试与未配置端口时的入口）。
pub async fn start(state: AgentState) -> Result<AgentServer, miyo_core::AppError> {
    start_at(state, DEFAULT_AGENT_PORT).await
}

/// 启动服务：从 `preferred_port` 起依次尝试，最多 [`MAX_PORT_TRIES`] 个端口
/// （贴近 `u16::MAX` 时提前截止）。绑定成功即返回，调用方应把**实际端口**
/// 写入 `{data_dir}/agent-port` 供 `miyo-mcp` 发现。
pub async fn start_at(
    state: AgentState,
    preferred_port: u16,
) -> Result<AgentServer, miyo_core::AppError> {
    let max_tries =
        u32::from(MAX_PORT_TRIES).min(u32::from(u16::MAX) - u32::from(preferred_port) + 1) as u16;
    for offset in 0..max_tries {
        let port = preferred_port + offset;
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let Ok(listener) = tokio::net::TcpListener::bind(addr).await else {
            continue;
        };
        let router = build_router(state.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await;
        });
        tracing::info!("[agent] Agent 控制面已启动：http://127.0.0.1:{port}");
        return Ok(AgentServer {
            port,
            shutdown: tx,
            handle: Some(handle),
        });
    }
    Err(miyo_core::AppError::Mock(format!(
        "端口 {}~{} 均被占用，无法启动 Agent 控制面",
        preferred_port,
        preferred_port + (max_tries - 1)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::load_or_create_token;

    async fn spawn_test_server() -> (String, String, SqlitePool, AgentState) {
        let dir = std::env::temp_dir().join(format!("miyo-agent-srv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let db = miyo_storage::db::init_db(&db_path).await.expect("建库");
        let token = load_or_create_token(&dir).expect("token");
        let state = AgentState::new(db.clone(), token.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = build_router(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let url = format!("http://{addr}");
        (url, token, db, state)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn disabled_tool_blocks_route_and_reflects_in_tools_endpoint() {
        let (url, token, db, _state) = spawn_test_server().await;
        let auth = format!("Bearer {token}");

        // 停用 list_projects
        repo::set_setting(
            &db,
            MCP_DISABLED_TOOLS_KEY,
            &serde_json::json!(["list_projects"]).to_string(),
        )
        .await
        .unwrap();

        // /agent/tools 如实反映
        let res = client()
            .get(format!("{url}/agent/tools"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        let tools: Vec<serde_json::Value> = res.json().await.unwrap();
        let lp = tools.iter().find(|t| t["name"] == "list_projects").unwrap();
        assert_eq!(lp["enabled"], false);
        assert_eq!(lp["kind"], "read");
        let sc = tools.iter().find(|t| t["name"] == "save_curl").unwrap();
        assert_eq!(sc["enabled"], true);

        // 路由 403（TOOL_DISABLED）
        let res = client()
            .get(format!("{url}/agent/projects"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["code"], "TOOL_DISABLED");

        // 其他工具不受影响（health 免检）
        let res = client()
            .get(format!("{url}/agent/health"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // 重新启用 → 路由恢复
        repo::set_setting(
            &db,
            MCP_DISABLED_TOOLS_KEY,
            &serde_json::json!([]).to_string(),
        )
        .await
        .unwrap();
        let res = client()
            .get(format!("{url}/agent/projects"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn scenario_script_collaboration_flow() {
        use miyo_core::model::{Scenario, ScenarioStep};
        use serde_json::json;

        let (url, token, db, _state) = spawn_test_server().await;
        let project = repo::create_project(&db, "链路", "").await.unwrap();
        let ep = miyo_core::model::Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "文章详情".into(),
            method: miyo_core::model::HttpMethod::GET,
            path: "/posts/1".into(),
            description: String::new(),
            status: miyo_core::model::EndpointStatus::Developing,
            sort_order: 0,
            request: miyo_core::model::RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_endpoint(&db, &ep).await.unwrap();
        let scenario = Scenario {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "链路演示".into(),
            description: String::new(),
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: ep.id,
                enabled: true,
                extract: json!([{ "name": "userId", "from": "body", "path": "$.userId" }]),
                assertions: json!([]),
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let auth = format!("Bearer {token}");

        // 列表（projectId 缺省 → 唯一项目自动选中）
        let res = client()
            .get(format!("{url}/agent/scenarios"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let list: Vec<serde_json::Value> = res.json().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["stepCount"], 1);
        assert_eq!(list[0]["hasCustomScript"], false);

        // 详情：链路变量 + 接口配置内嵌
        let res = client()
            .get(format!("{url}/agent/scenarios/{}", scenario.id))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let detail: serde_json::Value = res.json().await.unwrap();
        assert_eq!(detail["chainVariables"], json!(["userId"]));
        assert_eq!(detail["steps"][0]["endpointName"], "文章详情");
        assert_eq!(detail["steps"][0]["path"], "/posts/1");
        assert!(detail["script"].is_null());

        // 写脚本 → 持久化；再传空串 → 清除
        let res = client()
            .put(format!("{url}/agent/scenarios/{}/script", scenario.id))
            .header("Authorization", &auth)
            .json(&json!({ "script": "const BASE = 'https://x';" }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["scriptSet"], true);
        let reloaded = repo::get_scenario(&db, scenario.id).await.unwrap();
        assert_eq!(
            reloaded.script.as_deref(),
            Some("const BASE = 'https://x';")
        );

        let res = client()
            .put(format!("{url}/agent/scenarios/{}/script", scenario.id))
            .header("Authorization", &auth)
            .json(&json!({ "script": "  " }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["scriptSet"], false);
        let reloaded = repo::get_scenario(&db, scenario.id).await.unwrap();
        assert!(reloaded.script.is_none());

        // 不存在的场景 → 404
        let res = client()
            .get(format!("{url}/agent/scenarios/{}", uuid::Uuid::new_v4()))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_missing_and_wrong_token() {
        let (url, token, _db, _state) = spawn_test_server().await;
        // 无 token → 401
        let res = client()
            .get(format!("{url}/agent/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // 错误 token → 401
        let res = client()
            .get(format!("{url}/agent/health"))
            .header("Authorization", "Bearer wrong")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // 正确 token → 200
        let res = client()
            .get(format!("{url}/agent/health"))
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // X-Agent-Token 头同样可用
        let res = client()
            .get(format!("{url}/agent/health"))
            .header("X-Agent-Token", &token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn import_curl_creates_project_and_endpoint() {
        let (url, token, db, state) = spawn_test_server().await;
        let mut rx = state.subscribe();

        let command = r#"curl -X POST -H "Content-Type: application/json" -d '{"sku":"A1"}' https://api.example.com/orders?channel=app"#;
        let res: ImportCurlResult = client()
            .post(format!("{url}/agent/curl"))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "command": command }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(res.method, "POST");
        assert_eq!(res.path, "/orders");
        assert_eq!(res.base_url, "https://api.example.com");
        assert!(res.warning.is_none());

        // 事件已广播
        let event = rx.recv().await.unwrap();
        assert!(matches!(event, AgentEvent::EndpointImported { .. }));

        // 落库可查：自动创建了「Agent 导入」项目 + base_url 变量
        let projects = repo::list_projects(&db).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].variables.get("base_url").map(String::as_str),
            Some("https://api.example.com")
        );
        let endpoints = repo::list_endpoints(&db, projects[0].id).await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].path, "/orders");
        assert_eq!(endpoints[0].request.params[0].key, "channel");

        // 列表端点可用
        let list: Vec<serde_json::Value> = client()
            .get(format!("{url}/agent/endpoints/{}", projects[0].id))
            .header("X-Agent-Token", &token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        db.close().await;
    }

    #[tokio::test]
    async fn ambiguous_project_without_id_returns_candidates() {
        let (url, token, db, _state) = spawn_test_server().await;
        for name in ["A", "B"] {
            let now = chrono::Utc::now();
            repo::save_project(
                &db,
                &Project {
                    id: uuid::Uuid::new_v4(),
                    name: name.into(),
                    description: String::new(),
                    variables: Default::default(),
                    team_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        }
        let res = client()
            .post(format!("{url}/agent/curl"))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "command": "curl https://x.com/ping" }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: ApiErrorBody = res.json().await.unwrap();
        assert_eq!(body.code, "VALIDATION");
        assert!(body.message.contains("GET /agent/projects"));
        db.close().await;
    }

    #[tokio::test]
    async fn base_url_conflict_yields_warning_not_overwrite() {
        let (url, token, db, _state) = spawn_test_server().await;
        let now = chrono::Utc::now();
        let project = Project {
            id: uuid::Uuid::new_v4(),
            name: "已有项目".into(),
            description: String::new(),
            variables: [(
                "base_url".to_string(),
                "https://prod.example.com".to_string(),
            )]
            .into_iter()
            .collect(),
            team_id: None,
            created_at: now,
            updated_at: now,
        };
        repo::save_project(&db, &project).await.unwrap();

        let res: ImportCurlResult = client()
            .post(format!("{url}/agent/curl"))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({
                "command": "curl https://other.example.com/items",
                "projectId": project.id
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(
            res.base_url, "https://prod.example.com",
            "不得覆盖既有 base_url"
        );
        let warn = res.warning.expect("应包含不一致警告");
        assert!(warn.contains("https://other.example.com"));

        // 用户数据未被篡改
        let reloaded = repo::get_project(&db, project.id).await.unwrap();
        assert_eq!(
            reloaded.variables.get("base_url").map(String::as_str),
            Some("https://prod.example.com")
        );
        db.close().await;
    }

    #[tokio::test]
    async fn malformed_command_is_400() {
        let (url, token, _db, _state) = spawn_test_server().await;
        let res = client()
            .post(format!("{url}/agent/curl"))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "command": "curl -X GET -H 'a: 1'" }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
