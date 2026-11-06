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
use miyo_core::model::{BodySpec, Endpoint, Project, RequestSpec, Scenario, ScenarioStep};
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
        name: "get_endpoint",
        kind: "read",
    },
    McpToolDef {
        name: "send_endpoint",
        kind: "write",
    },
    McpToolDef {
        name: "run_scenario",
        kind: "write",
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
        name: "create_scenario",
        kind: "write",
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

/// 场景步骤 + 生效请求配置（AI 写脚本的完整上下文）。
///
/// 引用式步骤 `endpointId` 有值、配置取自接口当前形态；内联步骤
/// `endpointId` 为 `None`、配置取自步骤内联请求。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioStepDetail {
    pub endpoint_id: Option<uuid::Uuid>,
    /// 步骤名（内联步骤用步骤名；引用步骤用接口名）。
    pub step_name: Option<String>,
    pub endpoint_name: String,
    pub method: String,
    /// 接口路径 / 内联 URL 模板（含 {{var}} 占位，未做路径变量代入）。
    pub path: String,
    pub enabled: bool,
    pub extract: serde_json::Value,
    pub assertions: serde_json::Value,
    /// 生效请求规格（头 / Body / 认证形态），写脚本时照抄。
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

/// POST `/agent/scenarios` 请求体。
///
/// 新建场景必须带 `name`；`endpointIds` 按数组顺序落为步骤（首个即链路起点的
/// 典型形态），每步默认启用、无提取 / 断言——后续用 `get_scenario` 读回、
/// 再 `save_scenario_script` 补脚本。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScenarioRequest {
    /// 场景名（去空白后不得为空）。
    pub name: String,
    /// 项目 id；缺省时：仅一个项目 → 自动选中，多项目 → 报错列候选。
    #[serde(default)]
    pub project_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub description: Option<String>,
    /// 按顺序作为步骤的接口 UUID 列表（可空 = 先建空场景再逐步加）。
    #[serde(default)]
    pub endpoint_ids: Vec<uuid::Uuid>,
}

/// POST `/agent/scenarios` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScenarioResult {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    pub step_count: usize,
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

// ---------- 执行类 DTO（send_endpoint / run_scenario / get_endpoint） ----------

/// MCP 返回体的单步 / 单请求正文截断上限（AI 只需要读出链路字段，超大响应
/// 全量进模型上下文既慢又贵；全文仍在应用内结果面板可查）。
const EXEC_BODY_LIMIT: usize = 64 * 1024;
/// run_scenario 每步正文预览上限（多步叠加，比单请求更紧）。
const STEP_BODY_LIMIT: usize = 4 * 1024;

/// POST `/agent/endpoints/{id}/execute` 请求体。全部字段可选——缺省即按
/// 接口当前配置原样发送（`{{var}}` 由变量池渲染）。
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteEndpointRequest {
    /// 覆盖完整 URL 模板（保留 `{{var}}` 占位，执行期渲染）。
    #[serde(default)]
    pub url: Option<String>,
    /// 追加 / 覆盖请求头（同名启用键大小写不敏感覆盖）。
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// 追加 / 覆盖 query 参数。
    #[serde(default)]
    pub query: Option<std::collections::HashMap<String, String>>,
    /// 覆盖请求体原文：Json/Text 类型原位替换；GraphQL 替换 query；
    /// 其余类型降级为 Text（需要特定 Content-Type 时在 headers 里给）。
    #[serde(default)]
    pub body: Option<String>,
    /// 额外变量注入（优先级最高，覆盖环境/项目/全局同名项；链式调用的核心传参通道）。
    #[serde(default)]
    pub variables: Option<std::collections::HashMap<String, String>>,
    /// 环境变量来源（缺省 = 全局 + 项目变量）。
    #[serde(alias = "environment_id", default)]
    pub environment_id: Option<uuid::Uuid>,
    /// 本次请求超时毫秒（缺省按 接口级 → 全局设置 → 300s 兜底）。
    #[serde(alias = "timeout_ms", default)]
    pub timeout_ms: Option<u64>,
}

/// POST `/agent/endpoints/{id}/execute` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteEndpointResult {
    pub endpoint_id: uuid::Uuid,
    pub method: String,
    /// 渲染变量后的实际请求 URL。
    pub url: String,
    pub status: u16,
    pub duration_ms: f64,
    pub content_type: String,
    pub size_bytes: usize,
    pub body: String,
    pub body_truncated: bool,
}

/// POST `/agent/scenarios/{id}/run` 请求体。
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScenarioRequest {
    #[serde(alias = "environment_id", default)]
    pub environment_id: Option<uuid::Uuid>,
    /// 默认 true（失败即停），与应用内执行策略一致。
    #[serde(alias = "stop_on_failure", default = "default_true")]
    pub stop_on_failure: bool,
    /// 初始变量注入（优先级最高）。
    #[serde(default)]
    pub variables: Option<std::collections::HashMap<String, String>>,
}

fn default_true() -> bool {
    true
}

/// run_scenario 单步摘要（AI 链式调用所需的全部信息：每步的提取值直接可读）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioStepRunResult {
    /// 启用步骤 0-based 位次（与结果数组下标一致）。
    pub index: usize,
    pub endpoint_name: String,
    pub method: String,
    /// 渲染变量后的实际 URL；发送失败时为 null。
    pub url: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<f64>,
    pub ok: bool,
    /// 失败断言明细（描述 + 原因）。
    pub assertion_failures: Vec<String>,
    /// 本步提取到的链路变量。
    pub extracted: std::collections::HashMap<String, String>,
    /// 响应正文预览（超 [`STEP_BODY_LIMIT`] 截断）。
    pub body_preview: Option<String>,
    pub body_truncated: bool,
    pub error: Option<String>,
}

/// POST `/agent/scenarios/{id}/run` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScenarioResult {
    pub scenario_id: uuid::Uuid,
    pub scenario_name: String,
    pub passed: usize,
    pub failed: usize,
    /// 失败即停 / 取消时未执行的启用步骤数。
    pub skipped: usize,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    /// 最终变量池（链上全部提取结果；敏感名照原值返回，由调用方自行注意）。
    pub variables: std::collections::HashMap<String, String>,
    pub steps: Vec<ScenarioStepRunResult>,
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

/// POST `/agent/scenarios`：新建场景（可带初始步骤）。
///
/// 写路径：`projectId` 缺省时沿用「唯一项目自动选中」，多项目报错列候选，
/// 但**不**自动建项目（项目是用户资产，AI 随手创建会污染侧栏）。
async fn create_scenario_handler(
    State(state): State<AgentState>,
    Json(body): Json<CreateScenarioRequest>,
) -> ApiResult<CreateScenarioResult> {
    ensure_tool_enabled(&state, "create_scenario").await?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(api_error(miyo_core::AppError::Validation(
            "场景名不能为空".into(),
        )));
    }
    let project = resolve_project_read(&state.db, body.project_id)
        .await
        .map_err(api_error)?;
    // 步骤引用的接口必须真实存在于同一项目：先校验再落库，
    // 避免建出指向不存在接口的「空壳步骤」（执行期才会暴露）。
    for id in &body.endpoint_ids {
        match repo::get_endpoint(&state.db, *id).await {
            Ok(ep) if ep.project_id == project.id => {}
            Ok(_) => {
                return Err(api_error(miyo_core::AppError::Validation(format!(
                    "接口 {id} 不属于项目 {}",
                    project.id
                ))))
            }
            Err(_) => {
                return Err(api_error(miyo_core::AppError::NotFound(format!(
                    "接口（{id}）"
                ))))
            }
        }
    }
    let now = chrono::Utc::now();
    let steps: Vec<ScenarioStep> = body
        .endpoint_ids
        .iter()
        .map(|id| ScenarioStep::referenced(*id))
        .collect();
    // sort_order 接在该项目现有场景之后，列表里排在末尾（与手动新建一致）。
    let sort_order = repo::list_scenarios(&state.db, project.id)
        .await
        .map_err(api_error)?
        .len() as i64;
    let scenario = Scenario {
        id: uuid::Uuid::new_v4(),
        project_id: project.id,
        name: name.to_string(),
        description: body.description.unwrap_or_default(),
        category: String::new(),
        sort_order,
        steps,
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(CreateScenarioResult {
        id: scenario.id,
        project_id: scenario.project_id,
        name: scenario.name,
        step_count: scenario.steps.len(),
    }))
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
        // 生效请求配置：内联步骤用步骤自带请求；引用式步骤接口已删除时保留
        // 占位（AI 生成脚本可据此提示用户）。
        let (endpoint_name, method, path, request) = match (&step.request, step.endpoint_id) {
            (Some(inline), _) => (
                step.display_name().unwrap_or("内联请求").to_string(),
                inline.method.clone(),
                inline.url.clone(),
                inline.spec.clone(),
            ),
            (None, Some(endpoint_id)) => match repo::get_endpoint(&state.db, endpoint_id).await {
                Ok(ep) => (ep.name, ep.method.to_string(), ep.path, ep.request),
                Err(_) => (
                    "（接口已删除）".to_string(),
                    "GET".to_string(),
                    String::new(),
                    RequestSpec::default(),
                ),
            },
            (None, None) => (
                step.display_name().unwrap_or("（无效步骤）").to_string(),
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
            step_name: step.display_name().map(str::to_string),
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

// ---------- 执行类（send_endpoint / run_scenario / get_endpoint） ----------

/// UTF-8 安全截断（多字节字符按 char boundary 回退）。
fn truncate_utf8(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// 路径变量代入（镜像前端 `applyPathVars`）：长 key 先替换，`{{k}}` 与 `{k}`
/// 两种占位都支持，禁用/空行跳过。执行期 `{{var}}` 由变量池渲染，此处只
/// 处理接口自带的路径变量设计值。
fn apply_path_vars(path: &str, vars: &[miyo_core::model::KeyValue]) -> String {
    let mut rows: Vec<&miyo_core::model::KeyValue> = vars
        .iter()
        .filter(|r| r.enabled && !r.key.trim().is_empty() && !r.value.trim().is_empty())
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.key.chars().count()));
    let mut out = path.to_string();
    for row in rows {
        out = out
            .replace(&format!("{{{{{}}}}}", row.key), &row.value)
            .replace(&format!("{{{}}}", row.key), &row.value);
    }
    out
}

/// 接口当前配置 → URL 模板（与前端 `urlTemplateFor` 同构；控制面无会话
/// 域名输入，相对路径统一走 `{{base_url}}` 占位交由变量池渲染）。
fn url_template_for(ep: &Endpoint) -> String {
    let is_abs = ep.path.contains("://");
    let with_vars = apply_path_vars(&ep.path, &ep.request.path_variables);
    if is_abs {
        with_vars
    } else if with_vars.starts_with('/') {
        format!("{{{{base_url}}}}{with_vars}")
    } else {
        format!("{{{{base_url}}}}/{with_vars}")
    }
}

/// 同名启用键覆盖（大小写不敏感），不存在则追加。
fn upsert_kv(rows: &mut Vec<miyo_core::model::KeyValue>, key: &str, value: &str) {
    let lower = key.trim().to_ascii_lowercase();
    if let Some(kv) = rows
        .iter_mut()
        .find(|kv| kv.enabled && kv.key.trim().to_ascii_lowercase() == lower)
    {
        kv.value = value.to_string();
        return;
    }
    rows.push(miyo_core::model::KeyValue::new(key.trim(), value));
}

/// Body 覆盖：Json/Text 原位替换 raw；GraphQL 替换 query；字段表 / 文件 /
/// 二进制等结构化类型无法按原文承载，降级为 Text（Content-Type 由覆盖头决定）。
fn apply_body_override(spec: &mut RequestSpec, body: &str) {
    match &mut spec.body {
        BodySpec::Json { raw } | BodySpec::Text { raw } => *raw = body.to_string(),
        BodySpec::GraphQL { spec: gql } => gql.query = body.to_string(),
        _ => {
            spec.body = BodySpec::Text {
                raw: body.to_string(),
            }
        }
    }
}

/// 变量池合并（控制面版 `variables_for`）：全局（解密 + enabled）> 项目 >
/// 环境（显式传入才生效）> 调用方显式注入（最高优先，链式调用传参通道）。
/// 合并口径与 `AppState::variables_for` 一致（含 base_url 注入），去掉的是
/// 「激活项目 / 环境」UI 态——控制面按显式传参收敛。
async fn merged_variables(
    db: &SqlitePool,
    project_id: uuid::Uuid,
    environment_id: Option<uuid::Uuid>,
    extra: Option<std::collections::HashMap<String, String>>,
) -> Result<miyo_core::VariableMap, miyo_core::AppError> {
    let mut vars: miyo_core::VariableMap = repo::get_global_variables(db)
        .await?
        .into_iter()
        .filter(|v| v.enabled)
        // 先求值再 move key：`(v.key, v.effective_value()…)` 会因 key 先被
        // move 触发 E0382（与 state.rs 同款踩坑）。
        .map(|v| {
            let value = v.effective_value().to_string();
            (v.key, value)
        })
        .collect();
    let project = repo::get_project(db, project_id).await?;
    for (k, v) in project.variables {
        vars.insert(k, v);
    }
    if let Some(env_id) = environment_id {
        let env = repo::get_environment(db, env_id).await?;
        let mut env_vars = env.effective_variables();
        if !env_vars.contains_key("base_url") {
            let base = env.base_url.trim();
            if !base.is_empty() {
                env_vars.insert("base_url".into(), base.to_string());
            }
        }
        for (k, v) in env_vars {
            vars.insert(k, v);
        }
    }
    if let Some(extra) = extra {
        for (k, v) in extra {
            vars.insert(k, v);
        }
    }
    Ok(vars)
}

/// 全局超时设置（settings 表 `http_timeout_ms`，JSON `null` = 未配置）。
async fn global_timeout_ms(db: &SqlitePool) -> Option<u64> {
    repo::get_setting(db, "http_timeout_ms")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Option<u64>>(&raw).ok().flatten())
}

/// GET `/agent/endpoints/{id}/detail`：单个接口完整定义（大项目避免全量拉取）。
async fn get_endpoint_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<Endpoint> {
    ensure_tool_enabled(&state, "get_endpoint").await?;
    repo::get_endpoint(&state.db, id)
        .await
        .map(Json)
        .map_err(api_error)
}

/// POST `/agent/endpoints/{id}/execute`：按接口当前配置发送一次真实请求
/// （可覆盖 url/headers/query/body/变量），返回完整响应供 AI 提取链路字段。
async fn execute_endpoint_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    body: Option<Json<ExecuteEndpointRequest>>,
) -> ApiResult<ExecuteEndpointResult> {
    ensure_tool_enabled(&state, "send_endpoint").await?;
    let req = body.map(|Json(v)| v).unwrap_or_default();
    let ep = repo::get_endpoint(&state.db, id).await.map_err(api_error)?;
    let vars = merged_variables(
        &state.db,
        ep.project_id,
        req.environment_id,
        req.variables.clone(),
    )
    .await
    .map_err(api_error)?;

    let mut spec = ep.request.clone();
    if let Some(headers) = &req.headers {
        for (k, v) in headers {
            upsert_kv(&mut spec.headers, k, v);
        }
    }
    if let Some(query) = &req.query {
        for (k, v) in query {
            upsert_kv(&mut spec.params, k, v);
        }
    }
    if let Some(body) = &req.body {
        apply_body_override(&mut spec, body);
    }
    let global_params = repo::get_global_params(&state.db)
        .await
        .map_err(api_error)?;
    miyo_core::apply_global_params(&mut spec, &global_params, &vars);
    // 变量渲染（与 Tauri execute_request 同口径）：URL 模板 + 规格全文
    // （headers/params/auth/body 的 {{var}} 占位在此替换）。
    let spec = miyo_core::render_spec(&spec, &vars);

    let url_template = req.url.clone().unwrap_or_else(|| url_template_for(&ep));
    let url = miyo_core::resolve_variables(&url_template, &vars);
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(api_error(miyo_core::AppError::Validation(format!(
            "解析后的 URL 非法：{url}。项目缺少 base_url 变量？可在 variables 中显式传入 base_url 或用 url 参数覆盖完整地址。"
        ))));
    }

    let timeout_ms = req
        .timeout_ms
        .or(spec.timeout_ms)
        .or(global_timeout_ms(&state.db).await)
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS);
    let resp = miyo_http::client::send_request(ep.method, &url, &spec, Some(timeout_ms))
        .await
        .map_err(api_error)?;
    let content_type = resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let (body, truncated) = truncate_utf8(&resp.body_text(), EXEC_BODY_LIMIT);
    tracing::info!(
        "[agent] send_endpoint：{} {url} → {}（{}ms）",
        ep.method.as_str(),
        resp.status,
        resp.duration_ms
    );
    Ok(Json(ExecuteEndpointResult {
        endpoint_id: ep.id,
        method: ep.method.as_str().to_string(),
        url,
        status: resp.status,
        duration_ms: resp.duration_ms,
        content_type,
        size_bytes: resp.size_bytes,
        body,
        body_truncated: truncated,
    }))
}

/// POST `/agent/scenarios/{id}/run`：运行整个场景（串行 + 链路变量传递），
/// 返回分步摘要与最终变量池，供 AI 一次性执行预编排的接口链路。
async fn run_scenario_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    body: Option<Json<RunScenarioRequest>>,
) -> ApiResult<RunScenarioResult> {
    ensure_tool_enabled(&state, "run_scenario").await?;
    let req = body.map(|Json(v)| v).unwrap_or_default();
    let scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    let vars = merged_variables(
        &state.db,
        scenario.project_id,
        req.environment_id,
        req.variables.clone(),
    )
    .await
    .map_err(api_error)?;
    let global_params = repo::get_global_params(&state.db)
        .await
        .map_err(api_error)?;

    // 物化步骤（与 Tauri 命令 run_scenario 同口径：全局参数补缺 +
    // TestSpec 解析；URL 模板保留 {{var}} 执行期渲染）。内联步骤用步骤
    // 自带请求合成运行端点；引用式步骤取接口当前配置。禁用步骤照传入
    //（runner 落 skipped 行）。
    let mut steps = Vec::new();
    for (idx, step) in scenario.steps.iter().enumerate() {
        let (ep, url_template, spec) = match (&step.request, step.endpoint_id) {
            (Some(inline), _) => {
                let method = inline
                    .method
                    .parse::<miyo_core::model::HttpMethod>()
                    .map_err(|_| {
                        api_error(miyo_core::AppError::Validation(format!(
                            "第 {} 步的 HTTP 方法无效（{}）",
                            idx + 1,
                            inline.method
                        )))
                    })?;
                let name = step.display_name().unwrap_or("内联步骤").to_string();
                let path = inline.url.clone();
                (
                    Endpoint {
                        id: uuid::Uuid::new_v4(),
                        project_id: scenario.project_id,
                        folder_id: None,
                        name,
                        method,
                        path: path.clone(),
                        description: String::new(),
                        status: miyo_core::model::EndpointStatus::Developing,
                        sort_order: idx as i64,
                        request: inline.spec.clone(),
                        created_at: scenario.created_at,
                        updated_at: scenario.updated_at,
                    },
                    path,
                    inline.spec.clone(),
                )
            }
            (None, Some(endpoint_id)) => {
                let ep = repo::get_endpoint(&state.db, endpoint_id)
                    .await
                    .map_err(|_| {
                        api_error(miyo_core::AppError::Validation(format!(
                            "第 {} 步引用的接口不存在（{endpoint_id}）",
                            idx + 1
                        )))
                    })?;
                let url = url_template_for(&ep);
                let spec = ep.request.clone();
                (ep, url, spec)
            }
            (None, None) => {
                return Err(api_error(miyo_core::AppError::Validation(format!(
                    "第 {} 步既无接口引用也无内联请求",
                    idx + 1
                ))))
            }
        };
        let mut spec = spec;
        miyo_core::apply_global_params(&mut spec, &global_params, &vars);
        let config = miyo_test::config::TestSpec::from_request_value(Some(&serde_json::json!({
            "extract": step.extract,
            "assertions": step.assertions,
        })))
        .map_err(|e| {
            api_error(miyo_core::AppError::Validation(format!(
                "第 {} 步的提取/断言配置无效：{e}",
                idx + 1
            )))
        })?;
        steps.push(miyo_test::runner::ScenarioStepInput {
            url_template,
            endpoint: ep,
            spec_template: spec,
            extract: config.extract,
            assertions: config.assertions,
            enabled: step.enabled,
        });
    }
    if steps.iter().all(|s| !s.enabled) {
        return Err(api_error(miyo_core::AppError::Validation(
            "场景没有启用中的步骤".into(),
        )));
    }

    let timeout_ms = steps[0]
        .spec_template
        .timeout_ms
        .or(global_timeout_ms(&state.db).await)
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS);
    let options = miyo_test::runner::ScenarioOptions {
        timeout_ms: Some(timeout_ms),
        cancel: None,
        stop_on_failure: req.stop_on_failure,
    };
    let result = miyo_test::runner::run_scenario(steps, &vars, &options, None).await;

    let enabled_steps = scenario.steps.iter().filter(|s| s.enabled).count();
    let mut step_results = Vec::with_capacity(result.results.len());
    for r in &result.results {
        let (body_preview, body_truncated) = match &r.response {
            Some(resp) => {
                let (b, t) = truncate_utf8(&resp.body, STEP_BODY_LIMIT);
                (Some(b), t)
            }
            None => (None, false),
        };
        step_results.push(ScenarioStepRunResult {
            index: r.step_index.unwrap_or_default(),
            endpoint_name: r.endpoint_name.clone(),
            method: r.method.clone(),
            url: r.url.clone(),
            status: r.status,
            duration_ms: r.duration_ms,
            ok: r.ok,
            assertion_failures: r
                .outcomes
                .iter()
                .filter(|o| !o.passed)
                .map(|o| match &o.reason {
                    Some(reason) => format!("{}（{reason}）", o.description),
                    None => o.description.clone(),
                })
                .collect(),
            extracted: r.extracted.clone(),
            body_preview,
            body_truncated,
            error: r.request_error.clone(),
        });
    }
    // 计数按执行状态（results 含 skipped / blocked 行，不再用差值推算）。
    let passed = result
        .results
        .iter()
        .filter(|r| r.effective_status() == "success")
        .count();
    let failed = result
        .results
        .iter()
        .filter(|r| r.effective_status() == "failed" || r.effective_status() == "cancelled")
        .count();
    let skipped = result
        .results
        .iter()
        .filter(|r| r.effective_status() == "skipped" || r.effective_status() == "blocked")
        .count();

    // 运行历史落库（与 Tauri 命令同库同表：剥正文、统计摘要、保留最近 N 条）。
    let mut history_results = result.results.clone();
    for r in &mut history_results {
        r.response = None;
    }
    let record = miyo_core::model::ScenarioRun {
        id: uuid::Uuid::new_v4(),
        scenario_id: scenario.id,
        environment_id: req.environment_id,
        finished_at: chrono::Utc::now(),
        duration_ms: Some(
            result
                .results
                .iter()
                .filter_map(|r| r.duration_ms)
                .sum::<f64>(),
        ),
        total_steps: enabled_steps as i64,
        ok_count: passed as i64,
        fail_count: failed as i64,
        skipped_count: skipped as i64,
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        results_json: serde_json::to_string(&history_results).unwrap_or_else(|_| "[]".into()),
        variables_json: serde_json::to_string(&result.variables).unwrap_or_else(|_| "{}".into()),
        meta_json: None,
    };
    if let Err(e) = repo::save_scenario_run(&state.db, &record).await {
        tracing::warn!("[agent] run_scenario 历史落库失败：{e}");
    } else if let Err(e) =
        repo::prune_scenario_runs(&state.db, scenario.id, repo::SCENARIO_RUN_KEEP).await
    {
        tracing::warn!("[agent] run_scenario 历史清理失败：{e}");
    }
    tracing::info!(
        "[agent] run_scenario：场景「{}」{} 步，通过 {passed} 失败 {failed}",
        scenario.name,
        enabled_steps
    );

    Ok(Json(RunScenarioResult {
        scenario_id: scenario.id,
        scenario_name: scenario.name,
        passed,
        failed,
        skipped,
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        variables: result.variables,
        steps: step_results,
    }))
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
        .route("/agent/endpoints/{id}/detail", get(get_endpoint_handler))
        .route(
            "/agent/endpoints/{id}/execute",
            post(execute_endpoint_handler),
        )
        .route("/agent/scenarios/{id}/run", post(run_scenario_handler))
        .route("/agent/curl", post(import_curl_handler))
        .route(
            "/agent/scenarios",
            get(list_scenarios_handler).post(create_scenario_handler),
        )
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

    // ---------- 执行类路由测试（本地 echo 上游） ----------

    /// 本地回显上游：/orders 返回固定 JSON（供提取），/pay 回显收到的 JSON
    /// body，/echo 回显 Authorization 头与原文 body（供覆盖项断言）。
    async fn spawn_upstream() -> String {
        use axum::routing::post;
        let app = axum::Router::new()
            .route(
                "/orders",
                get(|| async { Json(serde_json::json!({ "orderId": "A1024" })) }),
            )
            .route(
                "/pay",
                post(|body: String| async move {
                    Json(serde_json::json!({ "paid": true, "got": body }))
                }),
            )
            .route(
                "/echo",
                post(|headers: axum::http::HeaderMap, body: String| async move {
                    Json(serde_json::json!({
                        "auth": headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or(""),
                        "body": body,
                    }))
                }),
            )
            .route(
                "/users/{id}",
                get(|AxumPath(id): AxumPath<String>| async move {
                    Json(serde_json::json!({ "userId": id }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    fn make_endpoint(project_id: uuid::Uuid, name: &str, method: &str, path: &str) -> Endpoint {
        Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id,
            folder_id: None,
            name: name.into(),
            method: method.parse().unwrap(),
            path: path.into(),
            description: String::new(),
            status: miyo_core::model::EndpointStatus::Developing,
            sort_order: 0,
            request: RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn execute_endpoint_sends_with_overrides() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "执行", "").await.unwrap();
        let ep = make_endpoint(project.id, "回显", "POST", &format!("{upstream}/echo"));
        repo::save_endpoint(&db, &ep).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/endpoints/{}/execute", ep.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({
                "headers": { "Authorization": "Bearer test-token" },
                "body": "hello-{{greeting}}",
                "variables": { "greeting": "你好" }
            }))
            .send()
            .await
            .unwrap();
        let status = res.status();
        let raw = res.text().await.unwrap_or_default();
        assert_eq!(status, StatusCode::OK, "执行应成功，响应：{raw}");
        let result: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(result["status"], 200);
        assert_eq!(result["method"], "POST");
        // 覆盖头与变量渲染（{{greeting}} → 你好）都在上游回显中可见
        let echoed: serde_json::Value =
            serde_json::from_str(result["body"].as_str().unwrap()).unwrap();
        assert_eq!(echoed["auth"], "Bearer test-token");
        assert!(echoed["body"].as_str().unwrap().contains("hello-你好"));
        assert_eq!(result["bodyTruncated"], false);
        db.close().await;
    }

    #[tokio::test]
    async fn execute_endpoint_relative_path_uses_base_url_variable() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "相对路径", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();
        let ep = make_endpoint(project.id, "订单", "GET", "/orders");
        repo::save_endpoint(&db, &ep).await.unwrap();

        // 相对路径 → {{base_url}} 占位由项目变量渲染
        let res = client()
            .post(format!("{url}/agent/endpoints/{}/execute", ep.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["status"], 200);
        assert_eq!(result["url"], format!("{upstream}/orders"));
        assert!(result["body"].as_str().unwrap().contains("A1024"));
        db.close().await;
    }

    #[tokio::test]
    async fn execute_endpoint_without_base_url_is_validation_error() {
        let (url, token, db, _state) = spawn_test_server().await;
        let project = repo::create_project(&db, "无 base", "").await.unwrap();
        let ep = make_endpoint(project.id, "订单", "GET", "/orders");
        repo::save_endpoint(&db, &ep).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/endpoints/{}/execute", ep.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["code"], "VALIDATION");
        db.close().await;
    }

    #[tokio::test]
    async fn run_scenario_chains_variables_and_persists_history() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "链路执行", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();

        // 步骤 1 下单（提取 orderId）→ 步骤 2 支付（body 引用 {{orderId}}）
        let order = make_endpoint(project.id, "下单", "GET", "/orders");
        let mut pay = make_endpoint(project.id, "支付", "POST", "/pay");
        pay.request.body = miyo_core::model::BodySpec::Json {
            raw: r#"{"orderId":"{{orderId}}"}"#.into(),
        };
        repo::save_endpoint(&db, &order).await.unwrap();
        repo::save_endpoint(&db, &pay).await.unwrap();
        let scenario = Scenario {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "下单支付链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(order.id),
                    enabled: true,
                    extract: serde_json::json!([{ "name": "orderId", "from": "body", "path": "$.orderId" }]),
                    assertions: serde_json::json!([{ "type": "status", "op": "eq", "expected": "200" }]),
                    name: None,
                    request: None,
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(pay.id),
                    enabled: true,
                    extract: serde_json::json!([]),
                    assertions: serde_json::json!([]),
                    name: None,
                    request: None,
                },
            ],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "stopOnFailure": true }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["passed"], 2, "两步都应通过：{result}");
        assert_eq!(result["failed"], 0);
        // 链路变量：步 1 提取 orderId → 步 2 body 渲染 + 最终变量池可读
        assert_eq!(result["variables"]["orderId"], "A1024");
        assert_eq!(result["steps"][0]["extracted"]["orderId"], "A1024");
        let pay_body = result["steps"][1]["bodyPreview"].as_str().unwrap();
        assert!(
            pay_body.contains("A1024"),
            "支付步应收到链路变量：{pay_body}"
        );
        assert_eq!(result["steps"][0]["endpointName"], "下单");

        // 运行历史已落库（与 Tauri 命令同表）
        let runs = repo::list_scenario_runs(&db, scenario.id, 5).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].ok_count, 2);
        db.close().await;
    }

    #[tokio::test]
    async fn run_scenario_stop_on_failure_halts() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "失败即停", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();
        let a = make_endpoint(project.id, "会失败", "GET", "/orders");
        let b = make_endpoint(project.id, "不应执行", "GET", "/users/1");
        repo::save_endpoint(&db, &a).await.unwrap();
        repo::save_endpoint(&db, &b).await.unwrap();
        let scenario = Scenario {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "停".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(a.id),
                    enabled: true,
                    extract: serde_json::json!([]),
                    // 断言 500 但上游返回 200 → 失败
                    assertions: serde_json::json!([{ "type": "status", "op": "eq", "expected": "500" }]),
                    name: None,
                    request: None,
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(b.id),
                    enabled: true,
                    extract: serde_json::json!([]),
                    assertions: serde_json::json!([]),
                    name: None,
                    request: None,
                },
            ],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "stopOnFailure": true }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["stoppedOnFailure"], true);
        assert_eq!(result["failed"], 1);
        assert_eq!(result["skipped"], 1, "第二步应被跳过：{result}");
        // 失败步 + skipped 占位行（禁用 / 早停语义对齐 runner）。
        assert_eq!(result["steps"].as_array().unwrap().len(), 2);
        assert_eq!(result["steps"][1]["ok"], false, "skipped 行非成功态");
        assert!(
            !result["steps"][0]["assertionFailures"]
                .as_array()
                .unwrap()
                .is_empty(),
            "失败断言明细应在"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn get_endpoint_detail_and_missing() {
        let (url, token, db, _state) = spawn_test_server().await;
        let project = repo::create_project(&db, "详情", "").await.unwrap();
        let ep = make_endpoint(project.id, "详情接口", "GET", "/detail");
        repo::save_endpoint(&db, &ep).await.unwrap();
        let auth = format!("Bearer {token}");

        let res = client()
            .get(format!("{url}/agent/endpoints/{}/detail", ep.id))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let detail: serde_json::Value = res.json().await.unwrap();
        assert_eq!(detail["name"], "详情接口");
        assert_eq!(detail["path"], "/detail");

        let res = client()
            .get(format!(
                "{url}/agent/endpoints/{}/detail",
                uuid::Uuid::new_v4()
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        db.close().await;
    }

    #[tokio::test]
    async fn disabled_execute_tools_are_403() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "停用", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();
        let ep = make_endpoint(project.id, "订单", "GET", "/orders");
        repo::save_endpoint(&db, &ep).await.unwrap();
        let scenario = Scenario {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "停用场景".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();
        repo::set_setting(
            &db,
            MCP_DISABLED_TOOLS_KEY,
            &serde_json::json!(["send_endpoint", "run_scenario"]).to_string(),
        )
        .await
        .unwrap();
        let auth = format!("Bearer {token}");

        let res = client()
            .post(format!("{url}/agent/endpoints/{}/execute", ep.id))
            .header("Authorization", &auth)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &auth)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        db.close().await;
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
    async fn create_scenario_endpoint_semantics() {
        use serde_json::json;

        let (url, token, db, _state) = spawn_test_server().await;
        let project = repo::create_project(&db, "下单项目", "").await.unwrap();
        let other = repo::create_project(&db, "另一个项目", "").await.unwrap();
        let make_ep = |pid: uuid::Uuid, name: &str| miyo_core::model::Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id: pid,
            folder_id: None,
            name: name.into(),
            method: miyo_core::model::HttpMethod::POST,
            path: "/orders".into(),
            description: String::new(),
            status: miyo_core::model::EndpointStatus::Developing,
            sort_order: 0,
            request: miyo_core::model::RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let ep = make_ep(project.id, "下单");
        let foreign = make_ep(other.id, "别家的接口");
        repo::save_endpoint(&db, &ep).await.unwrap();
        repo::save_endpoint(&db, &foreign).await.unwrap();
        let auth = format!("Bearer {token}");

        // 指定命名项目 + 一步 → 201/200 + 落库 + sort_order 追加到尾
        let res = client()
            .post(format!("{url}/agent/scenarios"))
            .header("Authorization", &auth)
            .json(&json!({
                "name": "收单链路",
                "projectId": project.id,
                "endpointIds": [ep.id],
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        let new_id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(body["stepCount"], 1);
        let saved = repo::get_scenario(&db, new_id).await.unwrap();
        assert_eq!(saved.steps.len(), 1);
        assert_eq!(saved.steps[0].endpoint_id, Some(ep.id));

        // 空名 → 400（Validation）
        let res = client()
            .post(format!("{url}/agent/scenarios"))
            .header("Authorization", &auth)
            .json(&json!({ "name": "   ", "projectId": project.id }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 引用别的项目的接口 → 400，且不落库
        let res = client()
            .post(format!("{url}/agent/scenarios"))
            .header("Authorization", &auth)
            .json(&json!({
                "name": "跨项目",
                "projectId": project.id,
                "endpointIds": [foreign.id],
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            repo::list_scenarios(&db, project.id).await.unwrap().len(),
            1
        );

        // 工具停用 → 403 TOOL_DISABLED
        repo::set_setting(
            &db,
            MCP_DISABLED_TOOLS_KEY,
            &json!(["create_scenario"]).to_string(),
        )
        .await
        .unwrap();
        let res = client()
            .post(format!("{url}/agent/scenarios"))
            .header("Authorization", &auth)
            .json(&json!({ "name": "停用后", "projectId": project.id }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
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
            category: String::new(),
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: Some(ep.id),
                enabled: true,
                extract: json!([{ "name": "userId", "from": "body", "path": "$.userId" }]),
                assertions: json!([]),
                name: None,
                request: None,
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
