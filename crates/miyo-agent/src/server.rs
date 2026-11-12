//! Agent 控制面 HTTP 服务：axum 路由 + Bearer 令牌鉴权。
//!
//! 端口从 4110 起探测（避开 Mock 的 4010~4029 段），只绑定回环地址。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use jsonpath_rust::JsonPath;
use miyo_core::model::{BodySpec, Endpoint, Project, RequestSpec, Scenario, ScenarioStep};
use miyo_storage::repository as repo;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::import::endpoint_from_curl;

/// 默认端口（4110 起探测；与 Mock 的 4010~4029 错开）。用户可在设置中改起始端口。
pub const DEFAULT_AGENT_PORT: u16 = 4110;

/// MCP 工具注册表：单源在 [`crate::registry`]（元数据、Schema、权限等级与
/// 注解），re-export 供 miyo-tauri 设置页与 miyo-mcp 共用。工具停用
/// 按名存于 settings（见 [`MCP_DISABLED_TOOLS_KEY`]）。
pub use crate::registry::{all_tool_meta as mcp_tool_registry, tool_meta, PermissionLevel};

/// 兼容别名：旧调用方（miyo-tauri 设置页）用的注册表入口。
pub const MCP_TOOLS: &[crate::registry::ToolMeta] = crate::registry::TOOL_META;

/// settings 表中「已停用 MCP 工具」的键；值为 JSON 字符串数组。
pub const MCP_DISABLED_TOOLS_KEY: &str = "mcp_disabled_tools";

/// GET `/agent/tools` 列表项。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolState {
    pub name: String,
    /// 兼容字段：read_only → "read"，其余 → "write"。
    pub kind: String,
    /// 细分权限等级（read / write / execute / network_execute / destructive）。
    pub permission: String,
    /// 业务模块（设置页分组）。
    pub module: String,
    /// 短标题。
    pub title: String,
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

/// settings 表中「已启用 MCP 权限等级」的键；值为 JSON 字符串数组
///（PermissionLevel::as_str()）。缺省 = read + write + execute；
/// network_execute / destructive 为高风险等级，默认关闭。
pub const MCP_PERMISSIONS_KEY: &str = "mcp_permissions";

/// 服务共享状态：连接池 + 鉴权令牌 + 事件广播。
#[derive(Clone)]
pub struct AgentState {
    pub db: SqlitePool,
    token: Arc<String>,
    events: broadcast::Sender<AgentEvent>,
    /// 长任务进度（run_id → (done, total)）：wait=true 的运行在 HTTP 等待
    /// 期间，miyo-mcp 经 /agent/runs/{id}/progress 轮询并转发
    /// notifications/progress。运行结束即清除。
    pub progress: Arc<std::sync::Mutex<std::collections::HashMap<String, (u64, u64)>>>,
    /// 异步场景运行的内存注册表（run_id → 句柄）：wait=false 的运行在后台
    /// 执行，get_run_result 据此轮询；运行结束落 scenario_runs 后查询走 DB。
    pub runs: Arc<std::sync::Mutex<std::collections::HashMap<String, RunHandle>>>,
}

/// 一次异步运行的状态句柄（标准锁：持有期间不 await）。
#[derive(Clone)]
pub struct RunHandle {
    pub status: Arc<std::sync::Mutex<&'static str>>,
    pub result: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    /// 取消令牌：cancel_run 工具 / HTTP 端点触发，引擎在步间与在途请求处
    /// 响应（返回已完成步骤，cancelled=true，正常落库）。
    pub cancel: Arc<tokio_util::sync::CancellationToken>,
    /// 终态时刻（None = 运行中）；注册表 TTL 清理据此淘汰过期条目。
    pub finished_at: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    /// 任务 JoinHandle：停机超时后 abort 兜底（协作式取消无响应的任务
    /// 不能长期挂住进程退出）。
    pub join: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

/// wait=false 异步运行的最大并发数：超限时拒绝新运行（无上限的 tokio::spawn
/// 会让持有 token 的调用方把内存与出口连接打满）。
pub const MAX_CONCURRENT_RUNS: usize = 8;
/// 终态句柄在内存注册表的保留时长：期间轮询读内存（含步骤摘要形状），
/// 过期后经 get_run_result 走 DB 兜底（结果已落库，两形状关键字段同义）。
const RUN_HANDLE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// 终态句柄的内存驻留上限（结果 JSON 各持有数 KB~数 MB；条目数封顶防
/// 高频运行 × TTL 窗口把注册表撑爆）：清扫时按 finished_at 淘汰最老的。
const MAX_COMPLETED_RUNS: usize = 32;
/// 停机时等待在途运行落库的最长时间：超时后 abort（不能无限期挂住退出）。
const RUNS_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// 运行是否处于终态（completed / failed）。
pub(crate) fn run_status_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed")
}

impl AgentState {
    pub fn new(db: SqlitePool, token: impl Into<String>) -> Self {
        let (events, _) = broadcast::channel(64);
        AgentState {
            db,
            token: Arc::new(token.into()),
            events,
            progress: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            runs: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// 已启用的 MCP 权限等级（每次实时读 settings：开关即时生效）。
    /// 未配置或缺省时 = read + write + execute（高风险等级需显式启用）。
    pub async fn enabled_permissions(&self) -> Vec<crate::registry::PermissionLevel> {
        let raw = repo::get_setting(&self.db, MCP_PERMISSIONS_KEY)
            .await
            .ok()
            .flatten();
        crate::registry::parse_enabled_levels(raw.as_deref())
    }

    /// 某权限等级是否已启用（[`Self::enabled_permissions`] 的单等级版）。
    pub async fn permission_enabled(&self, level: crate::registry::PermissionLevel) -> bool {
        self.enabled_permissions().await.contains(&level)
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

    /// 内存注册表中 running 状态的条目数（并发上限判定）。
    fn running_count(runs: &std::collections::HashMap<String, RunHandle>) -> usize {
        runs.values()
            .filter(|h| *h.status.lock().unwrap_or_else(|p| p.into_inner()) == "running")
            .count()
    }

    /// 淘汰终态超过 [`RUN_HANDLE_TTL`] 的句柄（插入新运行前顺带清扫，
    /// 兜底延迟移除任务失效的极端情况）；终态条目数超 [`MAX_COMPLETED_RUNS`]
    /// 时按 finished_at 淘汰最老的（注册表内存封顶）。
    fn sweep_runs(&self) {
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.retain(|_, h| {
            if *h.status.lock().unwrap_or_else(|p| p.into_inner()) == "running" {
                return true;
            }
            let finished = h.finished_at.lock().unwrap_or_else(|p| p.into_inner());
            matches!(&*finished, Some(t) if t.elapsed() < RUN_HANDLE_TTL)
        });
        let mut completed: Vec<(String, Option<std::time::Instant>)> = runs
            .iter()
            .filter(|(_, h)| *h.status.lock().unwrap_or_else(|p| p.into_inner()) != "running")
            .map(|(id, h)| {
                (
                    id.clone(),
                    *h.finished_at.lock().unwrap_or_else(|p| p.into_inner()),
                )
            })
            .collect();
        if completed.len() > MAX_COMPLETED_RUNS {
            // finished_at 为 None（异常兜底路径）排最后；最老的先淘汰。
            completed
                .sort_by_key(|(_, t)| t.map(|i| i.elapsed()).unwrap_or(std::time::Duration::ZERO));
            let excess = completed.len() - MAX_COMPLETED_RUNS;
            for (id, _) in completed.into_iter().take(excess) {
                runs.remove(&id);
            }
        }
    }

    /// 并发闸 + 句柄插入（原子）：判定与插入在同一把锁内完成，
    /// 消除「检查通过 → 锁释放 → 并发请求同时插入」的超限窗口。
    /// 满员返回 false（调用方回 429 语义），成功返回 true。
    pub fn try_acquire_run_slot(&self, run_id: &str, handle: RunHandle) -> bool {
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        if AgentState::running_count(&runs) >= MAX_CONCURRENT_RUNS {
            return false;
        }
        runs.insert(run_id.to_string(), handle);
        true
    }

    /// 等待全部在途运行到达终态（停机落库窗口）；超 [`RUNS_SHUTDOWN_GRACE`]
    /// 后 abort 残留任务（协作式取消无响应的兜底），并清空注册表。
    pub async fn drain_runs(&self) {
        let deadline = std::time::Instant::now() + RUNS_SHUTDOWN_GRACE;
        loop {
            {
                let runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
                let in_flight = runs
                    .values()
                    .filter(|h| {
                        !run_status_terminal(*h.status.lock().unwrap_or_else(|p| p.into_inner()))
                    })
                    .count();
                if in_flight == 0 {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // 超时未完成的：abort 任务体（结果已不可靠，DB 由下次运行兜底）。
        let joins: Vec<tokio::task::JoinHandle<()>> = {
            let runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
            runs.values()
                .filter_map(|h| h.join.lock().unwrap_or_else(|p| p.into_inner()).take())
                .collect()
        };
        for j in joins {
            j.abort();
        }
    }

    /// 取消全部在途异步运行（应用关闭前调用：给后台任务一个落库窗口，
    /// 避免运行历史随 runtime drop 一起丢失）。
    pub fn cancel_all_runs(&self) {
        let runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        for h in runs.values() {
            if !run_status_terminal(*h.status.lock().unwrap_or_else(|p| p.into_inner())) {
                h.cancel.cancel();
            }
        }
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
    /// 执行控制（保存后原样读回——此前 get_scenario 不回显这些字段，
    /// AI 写完 waitMs / loop / condition 后读不到，误判为被静默丢弃，issue #156）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parallel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<serde_json::Value>,
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
    /// 分类标签（可选）。
    #[serde(default)]
    pub category: Option<String>,
    /// 按顺序作为步骤的接口 UUID 列表（可空 = 先建空场景再逐步加）。
    #[serde(default)]
    pub endpoint_ids: Vec<uuid::Uuid>,
    /// 完整步骤列表（引擎或 AI 方言；提供时优先于 endpointIds）。
    #[serde(default)]
    pub steps: Option<Vec<serde_json::Value>>,
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

/// 历史中每步响应正文的保留上限（与 Tauri 命令层同口径）。
const HISTORY_BODY_BYTES: usize = 64 * 1024;

/// 变量池脱敏（单源在 [`miyo_sync::redact::redact_vars`]）。
pub(crate) fn redact_vars(
    vars: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    miyo_sync::redact::redact_vars(vars)
}

/// URL query 脱敏（单源 [`miyo_sync::redact::redact_url_query`]）：日志 /
/// 历史落库 / MCP 返回共用；凭据在 query 的 URL 不落明文。
pub(crate) fn redact_url(url: &str) -> String {
    miyo_sync::redact::redact_url_query(url)
}

/// 请求快照脱敏（headers / cookies / url query）。
pub(crate) fn redact_snapshot(snapshot: &mut miyo_http::client::RequestSnapshot) {
    let mask = |(k, v): &(String, String)| -> (String, String) {
        if miyo_sync::redact::is_sensitive_key(k) && !v.is_empty() {
            (k.clone(), miyo_sync::redact::PLACEHOLDER.to_string())
        } else {
            (k.clone(), v.clone())
        }
    };
    snapshot.headers = snapshot.headers.iter().map(&mask).collect();
    snapshot.cookies = snapshot.cookies.iter().map(&mask).collect();
    snapshot.url = redact_url(&snapshot.url);
}

/// 运行结果全面脱敏（MCP/Agent 返回与历史落库共用单点）：
/// URL query、提取值、响应头、响应 body（JSON 保守策略）、请求快照、
/// 断言 expected/actual 逐一处理。旧历史行（脱敏前落库的明文数据）在
/// 读取端重过一遍本函数即可兜底。
pub(crate) fn redact_endpoint_result(r: &mut miyo_test::runner::EndpointResult) {
    if let Some(url) = &mut r.url {
        *url = redact_url(url);
    }
    r.extracted = redact_vars(&r.extracted);
    if let Some(resp) = r.response.as_mut() {
        miyo_sync::redact::redact_header_pairs(&mut resp.headers);
        let (masked, changed) = miyo_sync::redact::redact_json_text(&resp.body);
        if changed {
            resp.body = masked;
        }
        if let Some(snapshot) = resp.request.as_mut() {
            redact_snapshot(snapshot);
        }
    }
    for o in &mut r.outcomes {
        // 断言 expected/actual 是文本：URL 凭据脱敏 + JWT 形态判定
        //（JSON 形态文本交给 redact_json_text 保守处理）。
        for slot in [&mut o.expected, &mut o.actual] {
            let Some(text) = slot.as_mut() else { continue };
            let (masked, changed) = miyo_sync::redact::redact_json_text(text);
            if changed {
                *text = masked;
            }
            let redacted = miyo_sync::redact::redact_message_urls(text);
            if redacted != *text {
                *text = redacted;
            }
            if miyo_core::sensitive::looks_like_secret(text) {
                *text = miyo_sync::redact::PLACEHOLDER.to_string();
            }
        }
    }
}

/// TestCase 返回脱敏（单源 [`miyo_sync::redact::redact_test_case`]）。
pub(crate) fn redact_test_case(case: &mut miyo_core::model::TestCase) {
    miyo_sync::redact::redact_test_case(case);
}

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

/// 数据驱动的一行数据（MCP 方言：name 展示名 + vars 变量注入）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDataSet {
    pub name: String,
    #[serde(default)]
    pub vars: std::collections::HashMap<String, String>,
}

/// POST `/agent/scenarios/{id}/run` 请求体。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScenarioRequest {
    #[serde(alias = "environment_id", default)]
    pub environment_id: Option<uuid::Uuid>,
    /// 默认 true（失败即停），与应用内执行策略一致。
    #[serde(alias = "stop_on_failure", default = "default_true")]
    pub stop_on_failure: bool,
    /// 初始变量注入（优先级最高）。
    #[serde(alias = "variables", default)]
    pub variables: Option<std::collections::HashMap<String, String>>,
    /// 数据驱动：多组数据独立执行（每组变量隔离、独立落历史；
    /// 行变量覆盖 variables / 环境变量）。
    #[serde(alias = "dataSets", default, skip_serializing_if = "Option::is_none")]
    pub data_sets: Option<Vec<crate::server::McpDataSet>>,
    /// false = 异步执行：立即返回 runId，用 GET /agent/runs/{id} 轮询
    ///（长场景避免 MCP 请求超时）；缺省 true 同步等完整结果。
    #[serde(default = "default_true")]
    pub wait: bool,
    /// 运行标识（缺省生成 UUID；落库 id 与之相同——查询口径统一）。
    #[serde(default)]
    pub run_id: Option<String>,
    /// 幂等键自动注入：为写操作步骤（POST/PUT/PATCH/DELETE）补
    /// `Idempotency-Key: {runId}-{步骤位次}`（已有该头的步骤不覆盖）。
    /// 同一步骤循环重试共用一键——AI 重试不会重复下单。
    #[serde(default, rename = "autoIdempotency")]
    pub auto_idempotency: bool,
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
    /// 执行阶段（None = 主步骤；"setup" / "teardown"）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// 重试日志摘要（attempt:status/error 列表；无重试为空）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_log: Vec<String>,
}

/// POST `/agent/scenarios/{id}/run` 响应体。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunScenarioResult {
    /// 运行标识（落库 id 与之相同；get_run_result / get_step_result 按它查询）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub scenario_id: uuid::Uuid,
    pub scenario_name: String,
    pub passed: usize,
    pub failed: usize,
    /// 失败即停 / 取消时未执行的启用步骤数。
    pub skipped: usize,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    /// 最终变量池（链上全部提取结果；敏感名已脱敏为占位符，与落库 /
    /// 异步内存路径同口径）。
    pub variables: std::collections::HashMap<String, String>,
    pub steps: Vec<ScenarioStepRunResult>,
    /// Setup 结果摘要（生命周期；空 = 无 setup）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup_results: Vec<ScenarioStepRunResult>,
    /// Teardown 结果摘要（生命周期；空 = 无 teardown / 已关闭自动清理）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown_results: Vec<ScenarioStepRunResult>,
}

/// 统一错误响应体 `{ code, message }`。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ApiErrorBody {
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
            // 错误出口统一 URL 脱敏：reqwest / 引擎错误的 Display 可能内嵌
            // 带凭证 query 的完整 URL（不得进 MCP 客户端 / AI 上下文）。
            message: miyo_sync::redact::redact_message_urls(&err.user_message()),
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

/// Host 头合法性：控制面只服务本机调用。防 DNS rebinding 纵深——恶意网页
/// 把解析到 127.0.0.1 的自有域名发来时 Host 为攻击者域名，直接拒绝
///（本机 curl / Agent / MCP 不受影响；保留全部 localhost 测试能力）。
fn host_allowed(host_header: &str) -> bool {
    // 去端口：IPv6 形如 [::1]:4110，IPv4/域名形如 127.0.0.1:4110
    let bare = if let Some(rest) = host_header.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host_header
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_header)
    };
    matches!(
        bare.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

async fn auth(State(state): State<AgentState>, req: Request, next: Next) -> Response {
    // Host 校验先于鉴权（rebinding 下 Host 即攻击特征；无 Host 的 HTTP/1.0
    // 客户端按拒绝处理——本机集成方均为 HTTP/1.1+）。
    let host_ok = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(host_allowed)
        .unwrap_or(false);
    if !host_ok {
        tracing::warn!(
            "[agent] 非法 Host 请求：{} {}",
            req.method(),
            req.uri().path()
        );
        return (
            StatusCode::FORBIDDEN,
            Json(ApiErrorBody {
                code: "FORBIDDEN".into(),
                message: "控制面仅接受本机访问（Host 须为 127.0.0.1 / localhost / ::1）".into(),
            }),
        )
            .into_response();
    }
    // 存活探针公开（token 证明走 challenge 应答，见 `health`）——
    // 鉴权请求仍逐个校验 Bearer。
    if req.uri().path() == "/agent/health" {
        return next.run(req).await;
    }
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

/// 存活探针（公开端点：中间件跳过 token 校验，保留 Host 校验）。
///
/// 发现协议（防探测段 token 收割）：探针**不携带 Bearer**，改带
/// `?challenge=<hex>`；服务端回应 `proof = sha256(token || challenge)`——
/// 只有掌握 token 的真控制面能给出正确证明，客户端验证通过后才发送
/// 鉴权请求。4110~4129 段的恶意监听者收不到 token，也无法伪造证明。
async fn health(
    State(state): State<AgentState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    match q.get("challenge").filter(|c| !c.trim().is_empty()) {
        Some(challenge) => {
            let proof = miyo_sign::sha256_hex(
                format!("{}{}", state.token.as_str(), challenge)
                    .into_bytes()
                    .as_slice(),
            );
            Json(serde_json::json!({ "ok": true, "proof": proof }))
        }
        None => Json(serde_json::json!({ "ok": true })),
    }
}

/// 工具调用守卫（服务端权威校验，不依赖 MCP 层隐藏）：
/// ① 工具必须已注册（未知 → TOOL_NOT_FOUND）；② 未被用户停用（→
/// TOOL_DISABLED）；③ 所属权限等级已启用（→ PERMISSION_DENIED，高风险
/// 等级 network_execute / destructive 默认关闭，需用户明确确认）。
async fn ensure_tool_allowed(
    state: &AgentState,
    name: &str,
) -> Result<(), (StatusCode, Json<ApiErrorBody>)> {
    let Some(meta) = crate::registry::tool_meta(name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorBody {
                code: "TOOL_NOT_FOUND".into(),
                message: format!("未注册的 MCP 工具：{name}"),
            }),
        ));
    };
    if state.tool_disabled(name).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorBody {
                code: "TOOL_DISABLED".into(),
                message: format!("工具 {name} 已在 Miyo 设置 → MCP 服务中停用"),
            }),
        ));
    }
    if !state.permission_enabled(meta.permission).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorBody {
                code: "PERMISSION_DENIED".into(),
                message: format!(
                    "工具 {name} 需要「{}」权限（{}），当前未启用。请在 Miyo 设置 → MCP 服务 → 权限控制中确认启用",
                    meta.permission.label(),
                    meta.permission.as_str()
                ),
            }),
        ));
    }
    Ok(())
}

/// 非工具端点（资源 / 进度）的权限守卫：按等级校验。
pub(crate) async fn ensure_permission(
    state: &AgentState,
    level: crate::registry::PermissionLevel,
) -> Result<(), (StatusCode, Json<ApiErrorBody>)> {
    if state.permission_enabled(level).await {
        return Ok(());
    }
    Err((
        StatusCode::FORBIDDEN,
        Json(ApiErrorBody {
            code: "PERMISSION_DENIED".into(),
            message: format!(
                "该操作需要「{}」权限（{}），当前未启用",
                level.label(),
                level.as_str()
            ),
        }),
    ))
}

/// GET `/agent/tools`：MCP 工具及启停状态（miyo-mcp 的 tools/list 过滤数据源）。
async fn list_tools_handler(State(state): State<AgentState>) -> ApiResult<Vec<McpToolState>> {
    let disabled = state.disabled_tools().await;
    Ok(Json(
        MCP_TOOLS
            .iter()
            .map(|t| McpToolState {
                name: t.name.to_string(),
                // kind 兼容旧设置页（read / write 二分）；细分等级见 permission。
                kind: if t.read_only {
                    "read".to_string()
                } else {
                    "write".to_string()
                },
                permission: t.permission.as_str().to_string(),
                module: t.module.as_str().to_string(),
                title: t.title.to_string(),
                enabled: !disabled.iter().any(|n| n == t.name),
            })
            .collect(),
    ))
}

async fn list_projects_handler(State(state): State<AgentState>) -> ApiResult<Vec<Project>> {
    ensure_tool_allowed(&state, "list_projects").await?;
    let mut projects = repo::list_projects(&state.db).await.map_err(api_error)?;
    // 项目变量可能存有 api_key 类全局凭据：与 get_environment 同口径脱敏。
    for p in &mut projects {
        p.variables = redact_vars(&p.variables);
    }
    Ok(Json(projects))
}

#[derive(Debug, serde::Deserialize)]
pub struct EndpointListQuery {
    /// 名称子串过滤（大小写不敏感）。
    #[serde(default)]
    pub name: Option<String>,
    /// HTTP 方法过滤（GET / POST / …，大小写不敏感）。
    #[serde(default)]
    pub method: Option<String>,
    /// 分组（文件夹）UUID 过滤。
    #[serde(rename = "folderId", default)]
    pub folder_id: Option<uuid::Uuid>,
    /// 分页：偏移与条数（缺省全量，兼容旧调用）。
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

async fn list_endpoints_handler(
    State(state): State<AgentState>,
    AxumPath(project_id): AxumPath<uuid::Uuid>,
    Query(q): Query<EndpointListQuery>,
) -> ApiResult<Vec<Endpoint>> {
    ensure_tool_allowed(&state, "list_endpoints").await?;
    let mut endpoints = repo::list_endpoints(&state.db, project_id)
        .await
        .map_err(api_error)?;
    // 筛选在内存完成（项目接口量级为几十到几百，SQLite 全量拉取后过滤足够）。
    if let Some(name) = &q.name {
        let needle = name.to_lowercase();
        endpoints.retain(|e| {
            e.name.to_lowercase().contains(&needle) || e.path.to_lowercase().contains(&needle)
        });
    }
    if let Some(m) = &q.method {
        let m = m.to_uppercase();
        endpoints.retain(|e| e.method.to_string() == m);
    }
    if let Some(fid) = q.folder_id {
        endpoints.retain(|e| e.folder_id == Some(fid));
    }
    let offset = q.offset.unwrap_or(0).min(endpoints.len());
    endpoints = endpoints.split_off(offset);
    if let Some(limit) = q.limit {
        endpoints.truncate(limit.max(1));
    }
    // 请求配置脱敏：auth / 敏感头 / body 里的凭证不进 AI 上下文（与
    // sync_push 的快照脱敏同口径；执行路径 send_endpoint 另行直读 DB 不受影响）。
    let mut log = miyo_sync::redact::RedactionLog::default();
    for ep in &mut endpoints {
        miyo_sync::redact::redact_request_spec(&mut ep.request, &mut log, &ep.name);
    }
    Ok(Json(endpoints))
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
    ensure_tool_allowed(&state, "create_scenario").await?;
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
    // 步骤来源：steps（完整定义，AI 方言或引擎形状）优先；否则 endpointIds 顺序引用。
    let steps: Vec<ScenarioStep> = match &body.steps {
        Some(raw_steps) => {
            let mut out = Vec::with_capacity(raw_steps.len());
            for (i, raw) in raw_steps.iter().enumerate() {
                let step = normalize_step_json(raw).map_err(|e| {
                    api_error(miyo_core::AppError::Validation(format!(
                        "第 {} 步无效：{e}",
                        i + 1
                    )))
                })?;
                if let Some(ep_id) = step.endpoint_id {
                    match repo::get_endpoint(&state.db, ep_id).await {
                        Ok(ep) if ep.project_id == project.id => {}
                        _ => {
                            return Err(api_error(miyo_core::AppError::Validation(format!(
                                "第 {} 步引用的接口 {ep_id} 不存在或不属于本项目",
                                i + 1
                            ))))
                        }
                    }
                }
                out.push(step);
            }
            out
        }
        None => body
            .endpoint_ids
            .iter()
            .map(|id| ScenarioStep::referenced(*id))
            .collect(),
    };
    // sort_order 接在该项目现有场景之后，列表里排在末尾（与手动新建一致）。
    let sort_order = repo::list_scenarios(&state.db, project.id)
        .await
        .map_err(api_error)?
        .len() as i64;
    let scenario = Scenario {
        setup: Vec::new(),
        teardown: Vec::new(),
        teardown_enabled: true,
        data_sets: Vec::new(),
        id: uuid::Uuid::new_v4(),
        project_id: project.id,
        name: name.to_string(),
        description: body.description.unwrap_or_default(),
        category: body.category.unwrap_or_default(),
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
    ensure_tool_allowed(&state, "list_scenarios").await?;
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
    ensure_tool_allowed(&state, "get_scenario").await?;
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
        // 生效请求配置脱敏：AI 生成脚本需要形状（方法 / 路径 / 头名），
        // 不需要真实凭据。
        let mut request = request;
        {
            let mut log = miyo_sync::redact::RedactionLog::default();
            miyo_sync::redact::redact_request_spec(&mut request, &mut log, &endpoint_name);
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
            condition: step.condition.clone(),
            r#loop: step.r#loop.clone(),
            wait_ms: step.wait_ms,
            parallel: step.parallel,
            overrides: step.overrides.clone(),
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
        project_variables: redact_vars(&project.variables),
        script: scenario.script,
    }))
}

async fn save_scenario_script_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Json(body): Json<SaveScenarioScriptRequest>,
) -> ApiResult<SaveScenarioScriptResult> {
    ensure_tool_allowed(&state, "save_scenario_script").await?;
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
    // 截断逻辑单源在 miyo-core::text（与 Tauri 命令层同口径）。
    let truncated = s.len() > max_bytes;
    (
        miyo_core::text::byte_truncate(s, max_bytes).to_string(),
        truncated,
    )
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
    ensure_tool_allowed(&state, "get_endpoint").await?;
    let mut ep = repo::get_endpoint(&state.db, id).await.map_err(api_error)?;
    let mut log = miyo_sync::redact::RedactionLog::default();
    miyo_sync::redact::redact_request_spec(&mut ep.request, &mut log, &ep.name);
    Ok(Json(ep))
}

/// POST `/agent/endpoints/{id}/execute`：按接口当前配置发送一次真实请求
/// （可覆盖 url/headers/query/body/变量），返回完整响应供 AI 提取链路字段。
async fn execute_endpoint_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    body: Option<Json<ExecuteEndpointRequest>>,
) -> ApiResult<ExecuteEndpointResult> {
    ensure_tool_allowed(&state, "send_endpoint").await?;
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
    // 日志记脱敏后的 URL：凭据在 query（?access_token=…）时不落明文日志。
    tracing::info!(
        "[agent] send_endpoint：{} {} → {}（{}ms）",
        ep.method.as_str(),
        redact_url(&url),
        resp.status,
        resp.duration_ms
    );
    // 返回体脱敏：URL query 与响应 body（JSON 敏感字段，如登录响应里的
    // token）不进 AI 上下文；非 JSON body 保守保留（无法可靠判定）。
    let (body, _) = miyo_sync::redact::redact_json_text(&body);
    Ok(Json(ExecuteEndpointResult {
        endpoint_id: ep.id,
        method: ep.method.as_str().to_string(),
        url: redact_url(&url),
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
) -> ApiResult<serde_json::Value> {
    ensure_tool_allowed(&state, "run_scenario").await?;
    let req = body.map(|Json(v)| v).unwrap_or_default();
    let run_id = req
        .run_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // ---- 异步模式：立即返回 runId，后台执行（长场景避免 MCP 超时）----
    if !req.wait {
        // 数据驱动矩阵要求同步等待（逐行汇总即响应；异步轮询语义与
        // 「每组独立 runId」组合会让轮询方无法区分行）。
        if req.data_sets.as_ref().is_some_and(|d| !d.is_empty()) {
            return Err(api_error(miyo_core::AppError::Validation(
                "dataSets（数据驱动）需要 wait=true：逐行汇总即为响应；长场景请拆分后按行调用"
                    .into(),
            )));
        }
        // 前置校验场景存在（fail fast，不建出注定失败的运行）。
        let _scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
        state.sweep_runs();
        let handle = RunHandle {
            status: Arc::new(std::sync::Mutex::new("running")),
            result: Arc::new(std::sync::Mutex::new(None)),
            cancel: Arc::new(tokio_util::sync::CancellationToken::new()),
            finished_at: Arc::new(std::sync::Mutex::new(None)),
            join: Arc::new(std::sync::Mutex::new(None)),
        };
        // 并发上限：判定与插入在同一把锁内（try_acquire_run_slot），超限即拒绝
        //（资源保护，非功能限制）。
        if !state.try_acquire_run_slot(&run_id, handle.clone()) {
            return Err(api_error(miyo_core::AppError::Validation(format!(
                "并发场景运行已达上限（{MAX_CONCURRENT_RUNS}），请等待现有运行完成或取消后再试"
            ))));
        }
        let state_c = state.clone();
        let run_id_c = run_id.clone();
        let join = tokio::spawn(async move {
            // panic 兜底：任务崩溃时置 failed（否则状态永久卡 running 且
            // 条目成为永久泄漏；AssertUnwindSafe 跨越 state/req 的所有权）。
            let outcome =
                futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(execute_scenario(
                    &state_c,
                    id,
                    req,
                    &run_id_c,
                    Some(handle.cancel.clone()),
                    None,
                )))
                .await;
            let mut st = handle.status.lock().unwrap_or_else(|p| p.into_inner());
            match outcome {
                Ok(Ok(result)) => {
                    // RunScenarioResult 构造时已统一脱敏（variables / steps 的
                    // extracted / url / body_preview），内存结果与同步响应同形状。
                    let v = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
                    *st = "completed";
                    *handle.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(v);
                }
                Ok(Err(e)) => {
                    *st = "failed";
                    *handle.result.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some(serde_json::json!({ "error": e.user_message() }));
                }
                Err(_panic) => {
                    *st = "failed";
                    *handle.result.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some(serde_json::json!({ "error": "运行任务异常终止（内部错误）" }));
                }
            }
            drop(st);
            *handle.finished_at.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(std::time::Instant::now());
            // TTL 后移除：注册表不随长会话无限增长（期间轮询读内存，
            // 过期后 get_run_result 走 DB 兜底）。
            let state_s = state_c.clone();
            let run_id_s = run_id_c.clone();
            tokio::spawn(async move {
                tokio::time::sleep(RUN_HANDLE_TTL).await;
                state_s
                    .runs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&run_id_s);
            });
        });
        // JoinHandle 挂到句柄上：停机超时可 abort（drain_runs 兜底）。
        *handle.join.lock().unwrap_or_else(|p| p.into_inner()) = Some(join);
        return Ok(Json(serde_json::json!({
            "runId": run_id,
            "scenarioId": id,
            "status": "running",
            "hint": "用 GET /agent/runs/{runId}（MCP: get_run_result）轮询结果",
        })));
    }

    // 数据驱动：多组数据独立执行（每组变量隔离、独立落历史、独立 runId）。
    if let Some(datasets) = req.data_sets.clone().filter(|d| !d.is_empty()) {
        // 组级取消令牌：total-timeout watchdog 不适用于矩阵，行间可被
        // cancel_run（对组 runId）取消——此处用本地令牌承载 watchdog 语义。
        let cancel = Arc::new(tokio_util::sync::CancellationToken::new());
        let total = datasets.len();
        let mut rows = Vec::with_capacity(total);
        for (idx, ds) in datasets.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let mut row_req = req.clone();
            row_req.data_sets = None;
            // 行变量覆盖初始变量（行优先级最高）。
            let mut vars = row_req.variables.clone().unwrap_or_default();
            for (k, v) in &ds.vars {
                vars.insert(k.clone(), v.clone());
            }
            row_req.variables = Some(vars);
            let row_run_id = uuid::Uuid::new_v4().to_string();
            let matrix = serde_json::json!({
                "kind": "data",
                "key": ds.name,
                "label": ds.name,
                "index": idx,
                "total": total,
                "groupId": run_id,
            });
            let result = execute_scenario(
                &state,
                id,
                row_req,
                &row_run_id,
                Some(cancel.clone()),
                Some(matrix),
            )
            .await
            .map_err(api_error)?;
            rows.push(serde_json::json!({
                "name": ds.name,
                "runId": result.run_id,
                "passed": result.passed,
                "failed": result.failed,
                "skipped": result.skipped,
                "ok": result.failed == 0,
                "steps": result.steps,
            }));
        }
        return Ok(Json(serde_json::json!({
            "runId": run_id,
            "scenarioId": id,
            "kind": "data-driven",
            "total": total,
            "okRows": rows.iter().filter(|r| r["ok"] == serde_json::json!(true)).count(),
            "rows": rows,
        })));
    }

    let result = execute_scenario(&state, id, req, &run_id, None, None)
        .await
        .map_err(api_error)?;
    Ok(Json(
        serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
    ))
}

/// 场景级总超时：到点自动触发取消令牌，runner 返回已完成的部分结果并按
/// cancelled 落库——防失控循环占住 worker（单步 300s × 循环轮数无上界时
/// 理论可跑数小时）。正常完成后撤销 watchdog。
const SCENARIO_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// 执行场景（同步核心）：物化步骤 → 串行运行（条件/循环/并行按配置）→
/// 落运行历史（响应正文 64KB 截断保留，run_id 即历史行 id）。
/// `cancel`：外部取消令牌（异步运行的注册表句柄；None = 内部新建，
/// 仅服务总超时 watchdog）。
async fn execute_scenario(
    state: &AgentState,
    scenario_id: uuid::Uuid,
    req: RunScenarioRequest,
    run_id: &str,
    cancel: Option<Arc<tokio_util::sync::CancellationToken>>,
    matrix: Option<serde_json::Value>,
) -> Result<RunScenarioResult, miyo_core::AppError> {
    let scenario = repo::get_scenario(&state.db, scenario_id).await?;
    let vars = merged_variables(
        &state.db,
        scenario.project_id,
        req.environment_id,
        req.variables.clone(),
    )
    .await?;
    let global_params = repo::get_global_params(&state.db).await?;

    // 物化步骤（与 Tauri 命令 run_scenario 同口径：全局参数补缺 +
    // TestSpec 解析；URL 模板保留 {{var}} 执行期渲染）。内联步骤用步骤
    // 自带请求合成运行端点；引用式步骤取接口当前配置。禁用步骤照传入
    //（runner 落 skipped 行）。
    let mut steps = Vec::new();
    for (idx, step) in scenario.steps.iter().enumerate() {
        let (ep, mut url_template, spec) = match (&step.request, step.endpoint_id) {
            (Some(inline), _) => {
                let method = inline
                    .method
                    .parse::<miyo_core::model::HttpMethod>()
                    .map_err(|_| {
                        miyo_core::AppError::Validation(format!(
                            "第 {} 步的 HTTP 方法无效（{}）",
                            idx + 1,
                            inline.method
                        ))
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
                        miyo_core::AppError::Validation(format!(
                            "第 {} 步引用的接口不存在（{endpoint_id}）",
                            idx + 1
                        ))
                    })?;
                let url = url_template_for(&ep);
                let spec = ep.request.clone();
                (ep, url, spec)
            }
            (None, None) => {
                return Err(miyo_core::AppError::Validation(format!(
                    "第 {} 步既无接口引用也无内联请求",
                    idx + 1
                )))
            }
        };
        // 幂等键自动注入：写操作步骤且未显式配置 Idempotency-Key 时补一键
        //（键 = runId-位次；循环轮次共用——同一逻辑操作重试不重复执行）。
        let is_write = {
            let m = match (&step.request, step.endpoint_id) {
                (Some(inline), _) => inline.method.clone(),
                (None, Some(ep_id)) => repo::get_endpoint(&state.db, ep_id)
                    .await
                    .map(|ep| ep.method.to_string())
                    .unwrap_or_default(),
                (None, None) => String::new(),
            };
            ["POST", "PUT", "PATCH", "DELETE"].contains(&m.to_uppercase().as_str())
        };
        let mut spec = spec;
        // 引用式步骤的请求参数覆盖（url 覆盖单独替换 url_template）。
        if let Some(ov) = &step.overrides {
            if let Some(u) = ov.get("url").and_then(|v| v.as_str()) {
                url_template = u.to_string();
            }
            miyo_core::apply_step_overrides(&mut spec, ov).map_err(|e| {
                miyo_core::AppError::Validation(format!("第 {} 步的参数覆盖无效：{e}", idx + 1))
            })?;
        }
        if req.auto_idempotency && is_write {
            let has_key = spec
                .headers
                .iter()
                .any(|h| h.enabled && h.key.eq_ignore_ascii_case("Idempotency-Key"));
            if !has_key {
                spec.headers.push(miyo_core::model::KeyValue::new(
                    "Idempotency-Key",
                    format!("{run_id}-{idx}"),
                ));
            }
        }
        miyo_core::apply_global_params(&mut spec, &global_params, &vars);
        let config = miyo_test::config::TestSpec::from_request_value(Some(&serde_json::json!({
            "extract": step.extract,
            "assertions": step.assertions,
        })))
        .map_err(|e| {
            miyo_core::AppError::Validation(format!("第 {} 步的提取/断言配置无效：{e}", idx + 1))
        })?;
        // 执行控制透传（条件 / 循环 / 等待 / 并行；配置无效 → 校验错误）。
        let condition = step
            .condition
            .as_ref()
            .map(|v| serde_json::from_value::<miyo_test::config::StepCondition>(v.clone()))
            .transpose()
            .map_err(|e| {
                miyo_core::AppError::Validation(format!("第 {} 步的执行条件无效：{e}", idx + 1))
            })?;
        let r#loop = step
            .r#loop
            .as_ref()
            .map(|v| serde_json::from_value::<miyo_test::config::StepLoop>(v.clone()))
            .transpose()
            .map_err(|e| {
                miyo_core::AppError::Validation(format!("第 {} 步的循环配置无效：{e}", idx + 1))
            })?;
        steps.push(miyo_test::runner::ScenarioStepInput {
            url_template,
            endpoint: ep,
            spec_template: spec,
            // 步骤重试策略（显式配置才启用；引擎执行期注入幂等键）。
            retry: step.retry.clone(),
            extract: config.extract,
            assertions: config.assertions,
            enabled: step.enabled,
            condition,
            r#loop,
            wait_ms: step
                .wait_ms
                .map(|w| w.min(miyo_test::config::StepLoop::MAX_INTERVAL_MS)),
            parallel: step.parallel,
        });
    }
    if steps.iter().all(|s| !s.enabled) {
        return Err(miyo_core::AppError::Validation(
            "场景没有启用中的步骤".into(),
        ));
    }
    // 生命周期步骤物化（与主步骤同口径：变量渲染 / 全局参数 / 断言解析）。
    async fn materialize_lifecycle(
        state: &AgentState,
        project_id: uuid::Uuid,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        rows: &[ScenarioStep],
        global_params: &[miyo_core::model::GlobalParam],
        vars: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<miyo_test::runner::ScenarioStepInput>, miyo_core::AppError> {
        let mut out = Vec::with_capacity(rows.len());
        for (idx, step) in rows.iter().enumerate() {
            let (ep, url_template, spec) = match (&step.request, step.endpoint_id) {
                (Some(inline), _) => {
                    let method = inline
                        .method
                        .parse::<miyo_core::model::HttpMethod>()
                        .map_err(|_| {
                            miyo_core::AppError::Validation(format!(
                                "setup/teardown 第 {} 步的 HTTP 方法无效（{}）",
                                idx + 1,
                                inline.method
                            ))
                        })?;
                    let name = step.display_name().unwrap_or("生命周期步骤").to_string();
                    let ep = Endpoint {
                        id: uuid::Uuid::new_v4(),
                        project_id,
                        folder_id: None,
                        name,
                        method,
                        path: inline.url.clone(),
                        description: String::new(),
                        status: miyo_core::model::EndpointStatus::Developing,
                        sort_order: idx as i64,
                        request: inline.spec.clone(),
                        created_at,
                        updated_at,
                    };
                    (ep, inline.url.clone(), inline.spec.clone())
                }
                (None, Some(endpoint_id)) => {
                    let ep = repo::get_endpoint(&state.db, endpoint_id).await?;
                    let url = url_template_for(&ep);
                    let spec = ep.request.clone();
                    (ep, url, spec)
                }
                (None, None) => {
                    return Err(miyo_core::AppError::Validation(format!(
                        "setup/teardown 第 {} 步既无接口引用也无内联请求",
                        idx + 1
                    )))
                }
            };
            let mut spec = spec;
            miyo_core::apply_global_params(&mut spec, global_params, vars);
            // 生命周期步骤的提取 / 断言与主步骤同引擎求值（setup 产物
            // 流入主步骤链路）。
            let config =
                miyo_test::config::TestSpec::from_request_value(Some(&serde_json::json!({
                    "extract": step.extract,
                    "assertions": step.assertions,
                })))
                .map_err(|e| {
                    miyo_core::AppError::Validation(format!(
                        "setup/teardown 第 {} 步的提取/断言配置无效：{e}",
                        idx + 1
                    ))
                })?;
            out.push(miyo_test::runner::ScenarioStepInput {
                endpoint: ep,
                url_template,
                spec_template: spec,
                retry: step.retry.clone(),
                extract: config.extract,
                assertions: config.assertions,
                enabled: step.enabled,
                condition: None,
                r#loop: None,
                wait_ms: None,
                parallel: false,
            });
        }
        Ok(out)
    }
    let mut setup_steps = materialize_lifecycle(
        state,
        scenario.project_id,
        scenario.created_at,
        scenario.updated_at,
        &scenario.setup,
        &global_params,
        &vars,
    )
    .await?;
    let mut teardown_steps = materialize_lifecycle(
        state,
        scenario.project_id,
        scenario.created_at,
        scenario.updated_at,
        &scenario.teardown,
        &global_params,
        &vars,
    )
    .await?;
    // 写操作 + 重试 → 步骤级幂等键（含 teardown：防重复清理副作用）。
    let idem_prefix = run_id.to_string();
    miyo_test::runner::apply_retry_idempotency(&mut steps, &idem_prefix);
    miyo_test::runner::apply_retry_idempotency(&mut setup_steps, &idem_prefix);
    miyo_test::runner::apply_retry_idempotency(&mut teardown_steps, &idem_prefix);

    let timeout_ms = steps[0]
        .spec_template
        .timeout_ms
        .or(global_timeout_ms(&state.db).await)
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS);
    let cancel_token =
        cancel.unwrap_or_else(|| Arc::new(tokio_util::sync::CancellationToken::new()));
    let options = miyo_test::runner::ScenarioOptions {
        timeout_ms: Some(timeout_ms),
        cancel: Some((*cancel_token).clone()),
        stop_on_failure: req.stop_on_failure,
    };
    // 总超时 watchdog：到点自动 cancel（runner 返回已完成步骤并落库）。
    // AbortOnDrop 守卫：正常完成 / 出错 / panic 三条路径都会撤销 watchdog
    //（原实现在 panic 路径泄漏 30 分钟任务）。
    struct AbortOnDrop(tokio::task::JoinHandle<()>);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let watchdog = tokio::spawn({
        let token = cancel_token.clone();
        async move {
            tokio::time::sleep(SCENARIO_TOTAL_TIMEOUT).await;
            token.cancel();
        }
    });
    let _watchdog_guard = AbortOnDrop(watchdog);
    // 进度记录：runner 回调写共享表（run_id → done/total），供
    // /agent/runs/{id}/progress 轮询（miyo-mcp 转发 notifications/progress）。
    let progress_run_id = run_id.to_string();
    let progress_cb = {
        let slot = state.progress.clone();
        let run_id = progress_run_id.clone();
        move |p: miyo_test::runner::ScenarioProgress<'_>| {
            slot.lock()
                .unwrap_or_else(|po| po.into_inner())
                .insert(run_id.clone(), (p.done as u64, p.total as u64));
        }
    };
    let lifecycle = miyo_test::runner::ScenarioLifecycle {
        setup: &setup_steps,
        teardown: &teardown_steps,
        teardown_enabled: scenario.teardown_enabled,
    };
    let result = miyo_test::runner::run_scenario_lifecycle(
        &lifecycle,
        steps,
        &vars,
        &options,
        Some(&progress_cb),
    )
    .await;
    state
        .progress
        .lock()
        .unwrap_or_else(|po| po.into_inner())
        .remove(run_id);
    drop(_watchdog_guard);

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
        // 返回体脱敏：URL query / 提取值（链路变量常含 token）/ body 预览
        // （JSON 敏感字段）不进 AI 上下文。
        let body_preview = body_preview.map(|b| miyo_sync::redact::redact_json_text(&b).0);
        step_results.push(ScenarioStepRunResult {
            index: r.step_index.unwrap_or_default(),
            endpoint_name: r.endpoint_name.clone(),
            method: r.method.clone(),
            url: r.url.as_ref().map(|u| redact_url(u)),
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
            extracted: redact_vars(&r.extracted),
            body_preview,
            body_truncated,
            error: r
                .request_error
                .as_deref()
                .map(miyo_sync::redact::redact_message_urls),
            phase: r.phase.clone(),
            retry_log: r.retry_log.iter().map(retry_log_summary).collect(),
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

    // 运行历史落库（与 Tauri 命令同库同表：正文 64KB 截断保留可回看，
    // 敏感头 / 变量池 / 响应 body 脱敏，保留最近 N 条）。
    // 结果行 = setup(phase) + 主步骤 + teardown(phase)：同一 results_json
    // 承载全生命周期；成败计数只看主步骤。脱敏含生命周期行。
    let mut history_results = result.setup_results.clone();
    history_results.extend(result.results.clone());
    history_results.extend(result.teardown_results.clone());
    for r in &mut history_results {
        if let Some(resp) = r.response.as_mut() {
            if resp.body.len() > HISTORY_BODY_BYTES {
                resp.body = truncate_utf8(&resp.body, HISTORY_BODY_BYTES).0;
                resp.history_truncated = true;
            }
        }
        // 全量脱敏（响应头 / body / 请求快照 / 提取值 / 断言值）：
        // 同一份 results_json 会被 get_run_result / get_step_result 回读给 AI。
        redact_endpoint_result(r);
    }
    let record = miyo_core::model::ScenarioRun {
        id: uuid::Uuid::parse_str(run_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
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
        // 成败计数只看主步骤（生命周期行 phase = setup/teardown 不计入）。
        total_steps: enabled_steps as i64,
        ok_count: passed as i64,
        fail_count: failed as i64,
        skipped_count: skipped as i64,
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        results_json: serde_json::to_string(&history_results).unwrap_or_else(|_| "[]".into()),
        variables_json: serde_json::to_string(&redact_vars(&result.variables))
            .unwrap_or_else(|_| "{}".into()),
        // 矩阵行标注（数据驱动 / 环境维度），包装为 {"matrix": {...}}——
        // 与 Tauri 侧 meta.matrix 同形状（前端 detail.meta.matrix 单一读取路径）；
        // 单场景运行为 None。
        meta_json: matrix.map(|m| serde_json::json!({ "matrix": m }).to_string()),
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

    Ok(RunScenarioResult {
        run_id: Some(run_id.to_string()),
        scenario_id: scenario.id,
        scenario_name: scenario.name,
        passed,
        failed,
        skipped,
        cancelled: result.cancelled,
        stopped_on_failure: result.stopped_on_failure,
        // 同步响应与落库 / 异步内存路径同口径：链路变量常含登录 token，
        // 明文返回即把凭据送进模型上下文。
        variables: redact_vars(&result.variables),
        steps: step_results,
        setup_results: phase_summaries(&result.setup_results),
        teardown_results: phase_summaries(&result.teardown_results),
    })
}

/// 单条重试日志摘要（attempt: 状态/错误；错误 URL 脱敏）。
fn retry_log_summary(a: &miyo_test::runner::RetryAttempt) -> String {
    format!(
        "#{}: {}{}",
        a.attempt,
        a.status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "网络错误".into()),
        a.error
            .as_deref()
            .map(miyo_sync::redact::redact_message_urls)
            .map(|e| format!(" {e}"))
            .unwrap_or_default()
    )
}

/// 生命周期结果摘要（setup / teardown 行：脱敏口径与主步骤一致）。
fn phase_summaries(rows: &[miyo_test::runner::EndpointResult]) -> Vec<ScenarioStepRunResult> {
    rows.iter()
        .map(|r| {
            let (body_preview, body_truncated) = match &r.response {
                Some(resp) => {
                    let (b, t) = truncate_utf8(&resp.body, STEP_BODY_LIMIT);
                    (Some(miyo_sync::redact::redact_json_text(&b).0), t)
                }
                None => (None, false),
            };
            ScenarioStepRunResult {
                index: r.step_index.unwrap_or_default(),
                endpoint_name: r.endpoint_name.clone(),
                method: r.method.clone(),
                url: r.url.as_ref().map(|u| redact_url(u)),
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
                extracted: redact_vars(&r.extracted),
                body_preview,
                body_truncated,
                error: r
                    .request_error
                    .as_deref()
                    .map(miyo_sync::redact::redact_message_urls),
                phase: r.phase.clone(),
                retry_log: r.retry_log.iter().map(retry_log_summary).collect(),
            }
        })
        .collect()
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
    ensure_tool_allowed(&state, "save_curl").await?;
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
        .route("/agent/environments", get(list_environments_handler))
        .route("/agent/environments/{id}", get(get_environment_handler))
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
        .route("/agent/scenarios/{id}", put(update_scenario_handler))
        .route("/agent/scenarios/{id}", delete(delete_scenario_handler))
        .route(
            "/agent/scenarios/{id}/steps",
            post(add_scenario_step_handler),
        )
        .route(
            "/agent/scenarios/{id}/steps/{index}",
            put(update_scenario_step_handler),
        )
        .route(
            "/agent/scenarios/{id}/validate",
            get(validate_scenario_handler),
        )
        .route("/agent/runs/{id}", get(get_run_result_handler))
        .route("/agent/runs/{id}/cancel", post(cancel_run_handler))
        .route("/agent/runs/{id}/progress", get(run_progress_handler))
        .route("/agent/context", get(context_handler))
        .route("/agent/runs/{id}/steps/{index}", get(get_run_step_handler))
        .route("/agent/runs/{id}/analysis", get(analyze_run_handler))
        .route(
            "/agent/scenarios/{id}/steps/{index}",
            delete(delete_scenario_step_handler),
        )
        .route(
            "/agent/scenarios/{id}/script",
            put(save_scenario_script_handler),
        )
        .route(
            "/agent/endpoints/{id}/test-cases",
            get(list_test_cases_handler),
        )
        .route("/agent/test-cases", post(create_test_case_handler))
        .route(
            "/agent/test-cases/{id}",
            get(get_test_case_handler)
                .put(update_test_case_handler)
                .delete(delete_test_case_handler),
        )
        .route("/agent/test-cases/{id}/run", post(run_test_case_handler))
        .route(
            "/agent/test-cases/{id}/runs",
            get(list_test_case_runs_handler),
        )
        .route("/agent/test-case-runs/{id}", get(get_test_case_run_handler))
        .route("/agent/tools", get(list_tools_handler))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

/// 正在运行的 Agent 服务句柄。
pub struct AgentServer {
    pub port: u16,
    /// 持有共享状态引用：停止时先取消全部在途异步运行，给后台任务一个
    /// 落库窗口（否则运行历史随 runtime drop 一起丢失）。
    state: AgentState,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AgentServer {
    pub fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 停止服务（幂等）：先取消全部在途运行并等待落库窗口（有界，超时
    /// abort），再关停 HTTP 服务——原实现只给 axum 优雅排空时间，长场景
    /// 的落库窗口不可靠。
    pub async fn stop(mut self) {
        self.state.cancel_all_runs();
        self.state.drain_runs().await;
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
            state,
            shutdown: tx,
            handle: Some(handle),
        });
    }
    Err(miyo_core::AppError::PortBind(format!(
        "端口 {}~{} 均被占用，无法启动 Agent 控制面",
        preferred_port,
        preferred_port + (max_tries - 1)
    )))
}

// ---------- MCP 集成测试升级（2026-11）：步骤归一化 + 环境 + 场景管理 ----------

/// 步骤 JSON 归一化：接受两种方言，产出引擎形状的 [`ScenarioStep`]。
///
/// 方言 A（引擎 / 前端 `steps_json` 原生形状）：
/// `{"endpointId" | "request": {"method","url","spec"}, "extract": [{"name","from","path"}],
///   "assertions": [{"type","path","op","expected"}], "condition"/"loop"/"waitMs"/"parallel"}`
///
/// 方言 B（AI 友好形状，自动翻译）：
/// `{"extractors": [{"name","source":"response.body"|"response.header","expression","type","required"}],
///   "assertions": [{"source":"response.status"|"response.body"|"response.header","expression","operator","expected"}]}`
/// operator 映射：equals→eq、notEquals→neq、contains、notContains→not_contains、
/// greaterThan→gt、greaterThanOrEquals→gte、lessThan→lt、lessThanOrEquals→lte、
/// exists、notExists→not_exists、matches（正则）。
pub(crate) fn normalize_step_json(v: &serde_json::Value) -> Result<ScenarioStep, String> {
    if !v.is_object() {
        return Err("步骤必须是对象".into());
    }
    let get = |k: &str| v.get(k);

    // ---- 名称 / 启用 ----
    let name = get("name").and_then(|x| x.as_str()).map(str::to_string);
    let enabled = get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);

    // ---- 提取：extract（引擎形状）或 extractors（AI 方言）；缺省 = 空数组
    //（引擎 TestSpec 要求序列形状，Null 会在运行期解析失败）----
    let extract = if let Some(arr) = get("extract").and_then(|x| x.as_array()) {
        // 引擎形状透传
        serde_json::Value::Array(arr.clone())
    } else if let Some(arr) = get("extractors").and_then(|x| x.as_array()) {
        let mut out = Vec::new();
        for e in arr {
            let name = e
                .get("name")
                .and_then(|x| x.as_str())
                .ok_or("extractor 缺少 name")?
                .to_string();
            let source = e
                .get("source")
                .and_then(|x| x.as_str())
                .unwrap_or("response.body");
            let expression = e
                .get("expression")
                .and_then(|x| x.as_str())
                .ok_or("extractor 缺少 expression")?
                .to_string();
            let from = match source {
                "response.body" => "body",
                "response.header" | "response.headers" => "header",
                "response.status" => {
                    return Err(format!(
                        "extractor「{name}」source=response.status 不支持提取——请改用断言（source=response.status）"
                    ))
                }
                other => return Err(format!("extractor source 不支持：{other}")),
            };
            let mut item = serde_json::json!({"name": name, "from": from, "path": expression});
            if e.get("required").and_then(|x| x.as_bool()).unwrap_or(false) {
                // required 透传为标记（校验与运行预检读取）。
                item["required"] = serde_json::Value::Bool(true);
            }
            out.push(item);
        }
        serde_json::Value::Array(out)
    } else {
        serde_json::Value::Array(Vec::new())
    };

    // ---- 断言：引擎形状或 AI 方言 ----
    let assertions = if let Some(arr) = get("assertions").and_then(|x| x.as_array()) {
        // 引擎形状：元素含 "type" 字段；AI 方言：元素含 "source"/"operator"。
        let mut out = Vec::new();
        for a in arr {
            let is_engine = a.get("type").is_some();
            if is_engine {
                out.push(a.clone());
                continue;
            }
            let source = a
                .get("source")
                .and_then(|x| x.as_str())
                .unwrap_or("response.status");
            let operator = a
                .get("operator")
                .and_then(|x| x.as_str())
                .unwrap_or("equals");
            let op = match operator {
                "equals" | "eq" => "eq",
                "notEquals" | "neq" => "neq",
                "contains" => "contains",
                "notContains" => "not_contains",
                "greaterThan" => "gt",
                "greaterThanOrEquals" => "gte",
                "lessThan" => "lt",
                "lessThanOrEquals" => "lte",
                "exists" => "exists",
                "notExists" => "not_exists",
                "matches" | "regex" => "matches",
                "empty" => "empty",
                "notEmpty" => "not_empty",
                other => return Err(format!("断言 operator 不支持：{other}")),
            };
            let (r#type, path) = match source {
                "response.status" => ("status", None),
                "response.body" => (
                    "jsonpath",
                    Some(
                        a.get("expression")
                            .and_then(|x| x.as_str())
                            .ok_or("断言 source=response.body 需要 expression（JSONPath）")?,
                    ),
                ),
                "response.header" | "response.headers" => (
                    "header",
                    Some(
                        a.get("expression")
                            .and_then(|x| x.as_str())
                            .ok_or("断言 source=response.header 需要 expression（Header 名）")?,
                    ),
                ),
                "performance" | "response.time" => ("response_time_ms", None),
                other => return Err(format!("断言 source 不支持：{other}")),
            };
            let mut item = serde_json::json!({"type": r#type, "op": op});
            if let Some(p) = path {
                item["path"] = serde_json::Value::String(p.to_string());
            }
            if let Some(exp) = a.get("expected") {
                if !exp.is_null() {
                    item["expected"] = exp.clone();
                }
            }
            if let Some(nm) = a.get("name").and_then(|x| x.as_str()) {
                if !nm.is_empty() {
                    item["name"] = serde_json::Value::String(nm.to_string());
                }
            }
            out.push(item);
        }
        serde_json::Value::Array(out)
    } else {
        serde_json::Value::Array(Vec::new())
    };

    // ---- 内联请求 / 接口引用 ----
    let request = match get("request") {
        Some(r) if !r.is_null() => {
            let method = r
                .get("method")
                .and_then(|x| x.as_str())
                .unwrap_or("GET")
                .to_uppercase();
            let url = r
                .get("url")
                .and_then(|x| x.as_str())
                .ok_or("内联请求缺少 url")?
                .to_string();
            let spec = r
                .get("spec")
                .cloned()
                .map(serde_json::from_value::<RequestSpec>)
                .transpose()
                .map_err(|e| format!("内联请求 spec 无效：{e}"))?
                .unwrap_or_default();
            Some(miyo_core::model::ScenarioInlineRequest { method, url, spec })
        }
        _ => None,
    };
    let endpoint_id = match get("endpointId").or_else(|| get("endpoint_id")) {
        Some(x) if !x.is_null() => Some(
            x.as_str()
                .ok_or("endpointId 必须是 UUID 字符串")?
                .parse::<uuid::Uuid>()
                .map_err(|_| "endpointId 不是合法 UUID")?,
        ),
        _ => None,
    };
    if request.is_none() && endpoint_id.is_none() {
        return Err("步骤既无 request（内联请求）也无 endpointId（接口引用）".into());
    }

    // ---- 执行控制（条件 / 循环 / 等待 / 并行）----
    let condition = match get("condition") {
        Some(c) if !c.is_null() => Some(
            serde_json::from_value::<miyo_test::config::StepCondition>(c.clone())
                .map_err(|e| format!("condition 无效：{e}"))?,
        ),
        _ => None,
    };
    let r#loop = match get("loop").or_else(|| get("repeat")) {
        Some(l) if !l.is_null() => Some(
            serde_json::from_value::<miyo_test::config::StepLoop>(l.clone())
                .map_err(|e| format!("loop 无效：{e}"))?,
        ),
        _ => None,
    };
    let wait_ms = get("waitMs")
        .or_else(|| get("wait_ms"))
        .and_then(|x| x.as_u64())
        .map(|w| w.min(miyo_test::config::StepLoop::MAX_INTERVAL_MS));
    let parallel = get("parallel").and_then(|x| x.as_bool()).unwrap_or(false);

    Ok(ScenarioStep {
        endpoint_id,
        enabled,
        extract,
        assertions,
        name,
        request,
        condition: condition.map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null)),
        r#loop: r#loop.map(|l| serde_json::to_value(l).unwrap_or(serde_json::Value::Null)),
        wait_ms,
        parallel,
        retry: None,
        overrides: get("overrides").cloned().filter(|v| v.is_object()),
    })
}

/// 环境摘要（列表用；不含变量明细）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentSummary {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub variable_count: usize,
}

/// 环境详情（敏感变量值脱敏——Token / 密钥不进入 MCP 返回）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentDetail {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub variables: Vec<EnvironmentVarLite>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentVarLite {
    pub key: String,
    /// 生效值（敏感变量为占位符）。
    pub value: String,
    pub sensitive: bool,
    pub enabled: bool,
    pub description: String,
}

async fn environment_detail(env: miyo_core::model::Environment) -> EnvironmentDetail {
    let variables = env
        .variables
        .iter()
        .filter(|v| v.enabled)
        .map(|v| {
            // 双保险：显式 sensitive 标记 或 键名命中敏感词根（与落库脱敏
            // 同口径）都视为敏感——用户漏勾选的真实凭据不明文回给 AI。
            let sensitive =
                v.sensitive.unwrap_or(false) || miyo_sync::redact::is_sensitive_key(&v.key);
            let value = if sensitive && !v.effective_value().is_empty() {
                miyo_sync::redact::PLACEHOLDER.to_string()
            } else {
                v.effective_value().to_string()
            };
            EnvironmentVarLite {
                key: v.key.clone(),
                value,
                sensitive,
                enabled: v.enabled,
                description: v.description.clone().unwrap_or_default(),
            }
        })
        .collect();
    EnvironmentDetail {
        id: env.id,
        project_id: env.project_id,
        name: env.name,
        base_url: env.base_url,
        variables,
    }
}

/// GET `/agent/environments?projectId=`：项目环境列表。
async fn list_environments_handler(
    State(state): State<AgentState>,
    Query(q): Query<EnvQuery>,
) -> ApiResult<Vec<EnvironmentSummary>> {
    ensure_tool_allowed(&state, "list_environments").await?;
    let project = resolve_project_read(&state.db, q.project_id)
        .await
        .map_err(api_error)?;
    let envs = repo::list_environments(&state.db, project.id)
        .await
        .map_err(api_error)?;
    Ok(Json(
        envs.into_iter()
            .map(|e| EnvironmentSummary {
                id: e.id,
                project_id: e.project_id,
                name: e.name,
                base_url: e.base_url,
                variable_count: e.variables.iter().filter(|v| v.enabled).count(),
            })
            .collect(),
    ))
}

#[derive(Debug, serde::Deserialize)]
pub struct EnvQuery {
    #[serde(rename = "projectId", default)]
    pub project_id: Option<uuid::Uuid>,
    /// validate_scenario 专用：运行期将经 run_scenario 的 variables 注入的
    /// 变量名（逗号分隔）——命中的引用不再报 VARIABLE_NOT_FOUND，
    /// 避免每次运行传唯一值（订单号 / 幂等键）的合法用法刷一屏 warning
    /// 淹没真问题（issue #157）。
    #[serde(rename = "runtimeVariables", default)]
    pub runtime_variables: String,
}

/// GET `/agent/environments/{id}`：环境详情（敏感值脱敏）。
async fn get_environment_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<EnvironmentDetail> {
    ensure_tool_allowed(&state, "get_environment").await?;
    let env = repo::get_environment(&state.db, id)
        .await
        .map_err(api_error)?;
    Ok(Json(environment_detail(env).await))
}

/// PUT `/agent/scenarios/{id}`：更新场景基本配置与步骤（整体覆盖语义，
/// 字段缺省保留原值；steps 提供时整体替换）。
#[derive(Debug, serde::Deserialize)]
pub struct UpdateScenarioRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// 步骤数组（引擎或 AI 方言，normalize_step_json 归一）；提供即整体替换。
    #[serde(default)]
    pub steps: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateScenarioResult {
    pub scenario_id: uuid::Uuid,
    pub step_count: usize,
}

async fn update_scenario_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Json(body): Json<UpdateScenarioRequest>,
) -> ApiResult<UpdateScenarioResult> {
    ensure_tool_allowed(&state, "update_scenario").await?;
    let mut scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    if let Some(n) = &body.name {
        let n = n.trim();
        if n.is_empty() {
            return Err(api_error(miyo_core::AppError::Validation(
                "场景名不能为空".into(),
            )));
        }
        scenario.name = n.to_string();
    }
    if let Some(d) = &body.description {
        scenario.description = d.clone();
    }
    if let Some(c) = &body.category {
        scenario.category = c.trim().to_string();
    }
    if let Some(steps) = &body.steps {
        let mut out = Vec::with_capacity(steps.len());
        for (i, raw) in steps.iter().enumerate() {
            let step = normalize_step_json(raw).map_err(|e| {
                api_error(miyo_core::AppError::Validation(format!(
                    "第 {} 步无效：{e}",
                    i + 1
                )))
            })?;
            // 引用式步骤校验归属。
            if let Some(ep_id) = step.endpoint_id {
                match repo::get_endpoint(&state.db, ep_id).await {
                    Ok(ep) if ep.project_id == scenario.project_id => {}
                    _ => {
                        return Err(api_error(miyo_core::AppError::Validation(format!(
                            "第 {} 步引用的接口 {ep_id} 不存在或不属于本项目",
                            i + 1
                        ))))
                    }
                }
            }
            out.push(step);
        }
        scenario.steps = out;
    }
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(UpdateScenarioResult {
        scenario_id: id,
        step_count: scenario.steps.len(),
    }))
}

/// DELETE `/agent/scenarios/{id}`：删除场景（运行历史随 FK 一并清理，
/// 响应中带被删历史条数供确认）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteScenarioResult {
    pub scenario_id: uuid::Uuid,
    pub deleted_runs: i64,
}

async fn delete_scenario_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<DeleteScenarioResult> {
    ensure_tool_allowed(&state, "delete_scenario").await?;
    // 先取历史条数（删除后不可再查）：COUNT(*) 免把全部历史 JSON 拉进内存。
    let deleted_runs = repo::count_scenario_runs(&state.db, id)
        .await
        .map_err(api_error)?;
    repo::delete_scenario(&state.db, id)
        .await
        .map_err(api_error)?;
    Ok(Json(DeleteScenarioResult {
        scenario_id: id,
        deleted_runs,
    }))
}

/// POST `/agent/scenarios/{id}/steps`：追加步骤（index 缺省 = 末尾）。
#[derive(Debug, serde::Deserialize)]
pub struct AddStepRequest {
    /// 步骤定义（引擎或 AI 方言）。
    pub step: serde_json::Value,
    /// 插入位次（0-based；缺省 = 末尾）。
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepMutationResult {
    pub scenario_id: uuid::Uuid,
    /// 操作后步骤总数。
    pub step_count: usize,
    /// 操作位次（0-based）。
    pub index: usize,
}

async fn add_scenario_step_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Json(body): Json<AddStepRequest>,
) -> ApiResult<StepMutationResult> {
    ensure_tool_allowed(&state, "add_scenario_step").await?;
    let mut scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    let step = normalize_step_json(&body.step)
        .map_err(|e| api_error(miyo_core::AppError::Validation(format!("步骤无效：{e}"))))?;
    if let Some(ep_id) = step.endpoint_id {
        match repo::get_endpoint(&state.db, ep_id).await {
            Ok(ep) if ep.project_id == scenario.project_id => {}
            _ => {
                return Err(api_error(miyo_core::AppError::Validation(format!(
                    "引用的接口 {ep_id} 不存在或不属于本项目"
                ))))
            }
        }
    }
    let index = body
        .index
        .unwrap_or(scenario.steps.len())
        .min(scenario.steps.len());
    scenario.steps.insert(index, step);
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(StepMutationResult {
        scenario_id: id,
        step_count: scenario.steps.len(),
        index,
    }))
}

async fn update_scenario_step_handler(
    State(state): State<AgentState>,
    AxumPath((id, index)): AxumPath<(uuid::Uuid, usize)>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<StepMutationResult> {
    ensure_tool_allowed(&state, "update_scenario_step").await?;
    let mut scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    if index >= scenario.steps.len() {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "步骤位次 {index} 越界（共 {} 步）",
            scenario.steps.len()
        ))));
    }
    let step = normalize_step_json(&body)
        .map_err(|e| api_error(miyo_core::AppError::Validation(format!("步骤无效：{e}"))))?;
    if let Some(ep_id) = step.endpoint_id {
        match repo::get_endpoint(&state.db, ep_id).await {
            Ok(ep) if ep.project_id == scenario.project_id => {}
            _ => {
                return Err(api_error(miyo_core::AppError::Validation(format!(
                    "引用的接口 {ep_id} 不存在或不属于本项目"
                ))))
            }
        }
    }
    scenario.steps[index] = step;
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(StepMutationResult {
        scenario_id: id,
        step_count: scenario.steps.len(),
        index,
    }))
}

async fn delete_scenario_step_handler(
    State(state): State<AgentState>,
    AxumPath((id, index)): AxumPath<(uuid::Uuid, usize)>,
) -> ApiResult<StepMutationResult> {
    ensure_tool_allowed(&state, "delete_scenario_step").await?;
    let mut scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    if index >= scenario.steps.len() {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "步骤位次 {index} 越界（共 {} 步）",
            scenario.steps.len()
        ))));
    }
    scenario.steps.remove(index);
    scenario.updated_at = chrono::Utc::now();
    repo::save_scenario(&state.db, &scenario)
        .await
        .map_err(api_error)?;
    Ok(Json(StepMutationResult {
        scenario_id: id,
        step_count: scenario.steps.len(),
        index,
    }))
}

// ---------- MCP 集成测试升级：校验 / 运行结果 / 失败分析 ----------

/// 校验问题项（可多条一次性返回）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateIssue {
    /// 稳定错误码（VARIABLE_NOT_FOUND / VARIABLE_USED_BEFORE_DEFINED /
    /// STEP_INVALID_URL / STEP_INVALID_ASSERT / STEP_INVALID_CONDITION /
    /// STEP_INVALID_LOOP / ENDPOINT_NOT_FOUND…）。
    pub code: String,
    pub message: String,
    /// 关联步骤位次（0-based；场景级问题为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_index: Option<usize>,
}

/// 校验报告。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateReport {
    pub valid: bool,
    pub errors: Vec<ValidateIssue>,
    pub warnings: Vec<ValidateIssue>,
    /// 写操作步骤（POST/PUT/PATCH/DELETE）——副作用提示。
    pub side_effect_steps: Vec<String>,
    /// 有副作用时的幂等键建议文本。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_hint: Option<String>,
}

/// GET `/agent/scenarios/{id}/validate?environmentId=`：运行前校验。
///
/// 检查：项目 / 环境存在性、步骤配置完整性（内联 URL / 方法 / 断言 /
/// 条件 / 循环）、变量引用分析（缺失 / 先引用后定义）、写操作清单。
async fn validate_scenario_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Query(q): Query<EnvQuery>,
) -> ApiResult<ValidateReport> {
    ensure_tool_allowed(&state, "validate_scenario").await?;
    let scenario = repo::get_scenario(&state.db, id).await.map_err(api_error)?;
    let project = repo::get_project(&state.db, scenario.project_id)
        .await
        .map_err(api_error)?;

    let mut errors: Vec<ValidateIssue> = Vec::new();
    let mut warnings: Vec<ValidateIssue> = Vec::new();

    // ---- 环境变量池（合并口径：环境 > 项目；base_url 注入）----
    let env = match q.project_id_env() {
        Some(env_id) => match repo::get_environment(&state.db, env_id).await {
            Ok(e) if e.project_id == scenario.project_id => Some(e),
            Ok(_) => {
                errors.push(ValidateIssue {
                    code: "ENVIRONMENT_NOT_FOUND".into(),
                    message: format!("环境 {env_id} 不属于本项目"),
                    step_index: None,
                });
                None
            }
            Err(_) => {
                errors.push(ValidateIssue {
                    code: "ENVIRONMENT_NOT_FOUND".into(),
                    message: format!("环境（{env_id}）不存在"),
                    step_index: None,
                });
                None
            }
        },
        None => None,
    };
    let mut available: std::collections::HashSet<String> =
        project.variables.keys().cloned().collect();
    if let Some(e) = &env {
        for v in e.variables.iter().filter(|v| v.enabled) {
            available.insert(v.key.clone());
        }
        if !e.base_url.trim().is_empty() {
            available.insert("base_url".into());
        }
    } else if !project.variables.contains_key("base_url") {
        // 无环境时 base_url 可能来自全局变量；不在则引用会字面量发送。
    }

    // ---- 逐步检查 ----
    let mut produced_later: Vec<std::collections::HashSet<String>> =
        vec![Default::default(); scenario.steps.len() + 1];
    for (i, s) in scenario.steps.iter().enumerate() {
        if !s.enabled {
            continue;
        }
        if let Some(rules) = s.extract.as_array() {
            for r in rules {
                if let Some(n) = r.get("name").and_then(|v| v.as_str()) {
                    if !n.trim().is_empty() {
                        // 记录「第 i 步产出」——先引用后定义判定用。
                        produced_later[i].insert(n.trim().to_string());
                    }
                }
            }
        }
    }

    for (i, s) in scenario.steps.iter().enumerate() {
        if !s.enabled {
            continue;
        }
        // ---- 步骤配置完整性 ----
        if let Some(inline) = &s.request {
            if inline.url.trim().is_empty() {
                errors.push(ValidateIssue {
                    code: "STEP_INVALID_URL".into(),
                    message: "内联请求 URL 为空".into(),
                    step_index: Some(i),
                });
            } else {
                let lower = inline.url.to_lowercase();
                let scheme_ok = lower.starts_with("http://")
                    || lower.starts_with("https://")
                    || lower.starts_with("{{");
                if !scheme_ok {
                    errors.push(ValidateIssue {
                        code: "STEP_INVALID_URL".into(),
                        message: format!(
                            "URL 需以 http(s):// 或 {{base_url}} 开头（当前：{}）",
                            inline.url
                        ),
                        step_index: Some(i),
                    });
                }
            }
            if let Err(e) = inline.method.parse::<miyo_core::model::HttpMethod>() {
                errors.push(ValidateIssue {
                    code: "STEP_INVALID_METHOD".into(),
                    message: format!("HTTP 方法无效：{e}"),
                    step_index: Some(i),
                });
            }
        }
        // 提取规则：body 提取的 JSONPath 可解析性。
        if let Some(rules) = s.extract.as_array() {
            for r in rules {
                let from = r.get("from").and_then(|v| v.as_str()).unwrap_or("body");
                let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.trim().is_empty() {
                    errors.push(ValidateIssue {
                        code: "STEP_INVALID_EXTRACT".into(),
                        message: "提取规则 path 为空".into(),
                        step_index: Some(i),
                    });
                    continue;
                }
                if from == "body" {
                    // 可解析性检查：JSONPath 语法错误在运行期表现为「无匹配」，
                    // 校验期提前拦截（合法但无匹配是正常业务情形，不报错）。
                    // 非法路径在 query 时返回 Err（合法但无匹配返回 Ok(空)）。
                    if serde_json::json!({}).query(path).is_err() {
                        errors.push(ValidateIssue {
                            code: "STEP_INVALID_EXTRACT".into(),
                            message: format!("JSONPath「{path}」无效"),
                            step_index: Some(i),
                        });
                    }
                }
            }
        }
        // 断言：类型 / 算子 / JSONPath。
        if let Some(rules) = s.assertions.as_array() {
            for a in rules {
                let r#type = a.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let op = a.get("op").and_then(|v| v.as_str()).unwrap_or("eq");
                const TYPES: &[&str] = &[
                    "status",
                    "header",
                    "body",
                    "jsonpath",
                    "response_time_ms",
                    "graphql_errors",
                    "length",
                ];
                if !TYPES.contains(&r#type) {
                    errors.push(ValidateIssue {
                        code: "STEP_INVALID_ASSERT".into(),
                        message: format!("断言类型不支持：{type}"),
                        step_index: Some(i),
                    });
                    continue;
                }
                if matches!(r#type, "jsonpath" | "header" | "length")
                    && a.get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .is_empty()
                {
                    errors.push(ValidateIssue {
                        code: "STEP_INVALID_ASSERT".into(),
                        message: format!("断言类型 {type} 需要 path"),
                        step_index: Some(i),
                    });
                    continue;
                }
                if r#type == "jsonpath" {
                    let p = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    if serde_json::json!({}).query(p).is_err() {
                        errors.push(ValidateIssue {
                            code: "STEP_INVALID_ASSERT".into(),
                            message: format!("断言 JSONPath「{p}」无效"),
                            step_index: Some(i),
                        });
                    }
                }
                let _ = op;
            }
        }
        // 条件 / 循环合法性。
        if let Some(c) = &s.condition {
            let parsed = serde_json::from_value::<miyo_test::config::StepCondition>(c.clone());
            match parsed {
                Ok(c) if c.is_valid() => {}
                _ => errors.push(ValidateIssue {
                    code: "STEP_INVALID_CONDITION".into(),
                    message: "执行条件配置无效（source/name/op 组合不合法）".into(),
                    step_index: Some(i),
                }),
            }
        }
        if let Some(l) = &s.r#loop {
            let parsed = serde_json::from_value::<miyo_test::config::StepLoop>(l.clone());
            match parsed {
                Ok(l) if l.is_valid() => {}
                Ok(l) => {
                    // while 的 source / expression 用法错误报真实原因（issue #156）
                    let message = l.while_problem().unwrap_or_else(|| {
                        "循环配置无效（mode/count/while_condition 组合不合法）".into()
                    });
                    errors.push(ValidateIssue {
                        code: "STEP_INVALID_LOOP".into(),
                        message,
                        step_index: Some(i),
                    });
                }
                Err(e) => errors.push(ValidateIssue {
                    code: "STEP_INVALID_LOOP".into(),
                    message: format!("循环配置无效：{e}"),
                    step_index: Some(i),
                }),
            }
        }
        // ---- 变量引用分析 ----
        let mut refs: Vec<String> = Vec::new();
        match (&s.request, s.endpoint_id) {
            (Some(inline), _) => {
                refs.extend(miyo_core::collect_var_refs(&inline.url));
                for kv in inline
                    .spec
                    .params
                    .iter()
                    .chain(inline.spec.headers.iter())
                    .chain(inline.spec.path_variables.iter())
                {
                    refs.extend(miyo_core::collect_var_refs(&kv.value));
                }
                match &inline.spec.body {
                    miyo_core::model::BodySpec::Json { raw }
                    | miyo_core::model::BodySpec::Text { raw } => {
                        refs.extend(miyo_core::collect_var_refs(raw));
                    }
                    miyo_core::model::BodySpec::UrlEncoded { fields } => {
                        for f in fields {
                            refs.extend(miyo_core::collect_var_refs(&f.value));
                        }
                    }
                    _ => {}
                }
            }
            (None, Some(ep_id)) => {
                if let Ok(ep) = repo::get_endpoint(&state.db, ep_id).await {
                    refs.extend(miyo_core::collect_var_refs(&ep.path));
                    for kv in ep
                        .request
                        .params
                        .iter()
                        .chain(ep.request.headers.iter())
                        .chain(ep.request.path_variables.iter())
                    {
                        refs.extend(miyo_core::collect_var_refs(&kv.value));
                    }
                } else {
                    errors.push(ValidateIssue {
                        code: "ENDPOINT_NOT_FOUND".into(),
                        message: format!("引用的接口（{ep_id}）不存在"),
                        step_index: Some(i),
                    });
                }
            }
            (None, None) => {}
        }
        for a in s.assertions.as_array().into_iter().flatten() {
            if let Some(v) = a.get("expected").and_then(|v| v.as_str()) {
                refs.extend(miyo_core::collect_var_refs(v));
            }
        }
        if let Some(c) = &s.condition {
            if c.get("source").and_then(|v| v.as_str()) != Some("prevStatus") {
                if let Some(n) = c.get("name").and_then(|v| v.as_str()) {
                    refs.push(n.to_string());
                }
            }
        }
        refs.sort();
        refs.dedup();

        // 运行期注入变量（runtimeVariables）：运行开始即存在，任何步骤引用
        // 都合法——不再报 VARIABLE_NOT_FOUND / VARIABLE_USED_BEFORE_DEFINED。
        let runtime_vars: std::collections::HashSet<&str> = q
            .runtime_variables
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        // 可用集合：静态变量 + 本步之前的提取产出。
        let mut avail_now = available.clone();
        for earlier in produced_later.iter().take(i) {
            avail_now.extend(earlier.iter().cloned());
        }
        for r in refs {
            if avail_now.contains(&r) || r == "base_url" {
                continue;
            }
            if runtime_vars.contains(r.as_str()) {
                continue;
            }
            if produced_later
                .iter()
                .skip(i + 1)
                .any(|set| set.contains(&r))
            {
                errors.push(ValidateIssue {
                    code: "VARIABLE_USED_BEFORE_DEFINED".into(),
                    message: format!("变量 {{{{{r}}}}} 由后续步骤才提取——执行到本步时它还不存在",),
                    step_index: Some(i),
                });
            } else {
                warnings.push(ValidateIssue {
                    code: "VARIABLE_NOT_FOUND".into(),
                    message: format!(
                        "变量 {{{{{r}}}}} 未在任何步骤提取、也不在环境/项目变量中——将按字面量发送"
                    ),
                    step_index: Some(i),
                });
            }
        }
    }

    // ---- 写操作清单（副作用提示）----
    let write_methods = ["POST", "PUT", "PATCH", "DELETE"];
    let mut side_effect_steps: Vec<String> = Vec::new();
    for (i, s) in scenario.steps.iter().enumerate() {
        if !s.enabled {
            continue;
        }
        let method = match &s.request {
            Some(inline) => inline.method.clone(),
            None => match s.endpoint_id {
                Some(ep_id) => repo::get_endpoint(&state.db, ep_id)
                    .await
                    .map(|ep| ep.method.to_string())
                    .unwrap_or_default(),
                None => String::new(),
            },
        };
        if write_methods.contains(&method.to_uppercase().as_str()) {
            let name = s.display_name().unwrap_or("（未命名）");
            side_effect_steps.push(format!("第 {} 步 {method} {name}", i + 1));
        }
    }
    let idempotency_hint = if side_effect_steps.is_empty() {
        None
    } else {
        Some(
            "场景包含写操作：重复执行可能重复下单/支付。建议为写操作步骤配置幂等键（如 Idempotency-Key 请求头），并在执行前与用户确认目标环境。"
                .to_string(),
        )
    };

    Ok(Json(ValidateReport {
        valid: errors.is_empty(),
        errors,
        warnings,
        side_effect_steps,
        idempotency_hint,
    }))
}

impl EnvQuery {
    fn project_id_env(&self) -> Option<uuid::Uuid> {
        self.project_id
    }
}

// ---------- 测试用例（issue #154 相关能力：AI/MCP 可创建 / 查询 / 执行用例） ----------

/// 用例快照 → RequestSpec（镜像前端 restoreBody：body_type/body_content 展开为 BodySpec）。
fn case_spec(case: &miyo_core::model::TestCase) -> miyo_core::model::RequestSpec {
    use miyo_core::model::{BodySpec, KeyValue, RequestSpec};
    let body: BodySpec = match case.body_type.as_str() {
        "json" => BodySpec::Json {
            raw: case.body_content.clone(),
        },
        "raw" | "text" => BodySpec::Text {
            raw: case.body_content.clone(),
        },
        "urlencoded" => BodySpec::UrlEncoded {
            fields: serde_json::from_str::<Vec<KeyValue>>(&case.body_content).unwrap_or_default(),
        },
        "form-data" | "multipart" => BodySpec::Multipart {
            fields: serde_json::from_str::<Vec<miyo_core::model::MultipartField>>(
                &case.body_content,
            )
            .unwrap_or_default(),
        },
        "graphql" => BodySpec::GraphQL {
            spec: serde_json::from_str(&case.body_content).unwrap_or_default(),
        },
        "binary" => BodySpec::Binary {
            path: case.body_content.clone(),
        },
        _ => BodySpec::None,
    };
    RequestSpec {
        params: case.params.clone(),
        headers: case.headers.clone(),
        path_variables: case.path_variables.clone(),
        auth: case
            .auth
            .clone()
            .unwrap_or(miyo_core::model::AuthSpec::None),
        body,
        active_tab: None,
        timeout_ms: case.timeout_ms,
        follow_redirects: true,
        tests: None,
        disable_cookies: case.disable_cookies,
        body_docs: Default::default(),
        scripts: None,
    }
}

/// GET /agent/endpoints/{id}/test-cases：接口的用例列表。
async fn list_test_cases_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<Vec<miyo_core::model::TestCase>> {
    ensure_tool_allowed(&state, "list_test_cases").await?;
    let mut cases = repo::list_test_cases(&state.db, id)
        .await
        .map_err(api_error)?;
    for c in &mut cases {
        redact_test_case(c);
    }
    Ok(Json(cases))
}

/// POST /agent/test-cases：新建用例（body = TestCase JSON；id 缺省自动生成）。
async fn create_test_case_handler(
    State(state): State<AgentState>,
    Json(mut case): Json<miyo_core::model::TestCase>,
) -> ApiResult<miyo_core::model::TestCase> {
    ensure_tool_allowed(&state, "create_test_case").await?;
    if case.name.trim().is_empty() {
        return Err(api_error(miyo_core::AppError::Validation(
            "用例名称不能为空".into(),
        )));
    }
    if case.id.is_nil() {
        case.id = uuid::Uuid::new_v4();
    }
    if case.created_at == chrono::DateTime::<chrono::Utc>::UNIX_EPOCH {
        case.created_at = chrono::Utc::now();
    }
    let created = repo::create_test_case(&state.db, &case)
        .await
        .map_err(api_error)?;
    tracing::info!(
        "[agent] create_test_case：{}（{} {}）",
        created.name,
        created.method.as_str(),
        created.url_path
    );
    // 返回给 AI 的定义脱敏（落库保持原样；UI 明文可见）。
    let mut out = created;
    redact_test_case(&mut out);
    Ok(Json(out))
}

/// GET /agent/test-cases/{id}：用例详情。
async fn get_test_case_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<miyo_core::model::TestCase> {
    ensure_tool_allowed(&state, "get_test_case").await?;
    let mut case = repo::get_test_case(&state.db, id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| {
            api_error(miyo_core::AppError::NotFound(format!(
                "测试用例 {id} 不存在"
            )))
        })?;
    redact_test_case(&mut case);
    Ok(Json(case))
}

/// PUT /agent/test-cases/{id}：整体更新（全字段覆盖语义）。
async fn update_test_case_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Json(case): Json<miyo_core::model::TestCase>,
) -> ApiResult<miyo_core::model::TestCase> {
    ensure_tool_allowed(&state, "update_test_case").await?;
    let existing = repo::get_test_case(&state.db, id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| {
            api_error(miyo_core::AppError::NotFound(format!(
                "测试用例 {id} 不存在"
            )))
        })?;
    repo::update_test_case_meta(
        &state.db,
        id,
        case.name.trim(),
        &case.category,
        &case.folder,
    )
    .await
    .map_err(api_error)?;
    repo::update_test_case_content(
        &state.db,
        id,
        case.method,
        &case.url_path,
        &case.path_variables,
        &case.params,
        &case.headers,
        &case.body_type,
        &case.body_content,
        &case.assertions,
        &case.extractors,
        &case.auth,
        case.timeout_ms,
        case.disable_cookies,
    )
    .await
    .map_err(api_error)?;
    let updated = repo::get_test_case(&state.db, id)
        .await
        .map_err(api_error)?
        .unwrap_or(existing);
    let mut out = updated;
    redact_test_case(&mut out);
    Ok(Json(out))
}

/// DELETE /agent/test-cases/{id}。
async fn delete_test_case_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<serde_json::Value> {
    ensure_tool_allowed(&state, "delete_test_case").await?;
    repo::delete_test_case(&state.db, id)
        .await
        .map_err(api_error)?;
    Ok(Json(serde_json::json!({ "deleted": true, "id": id })))
}

/// POST /agent/test-cases/{id}/run：执行用例（同一 miyo-test 引擎，强制发送；
/// 落执行历史并回写摘要；64KB 截断 + 脱敏与场景运行同口径）。
async fn run_test_case_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    body: Option<Json<RunTestCaseRequest>>,
) -> ApiResult<serde_json::Value> {
    use miyo_core::{apply_global_params, render_spec, resolve_variables};
    ensure_tool_allowed(&state, "run_test_case").await?;
    let req = body.map(|Json(v)| v).unwrap_or_default();
    let case = repo::get_test_case(&state.db, id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| {
            api_error(miyo_core::AppError::NotFound(format!(
                "测试用例 {id} 不存在"
            )))
        })?;
    let ep = repo::get_endpoint(&state.db, case.request_id)
        .await
        .map_err(api_error)?;
    let vars = merged_variables(&state.db, ep.project_id, req.environment_id, None)
        .await
        .map_err(api_error)?;

    // 运行用例的合成 Endpoint：请求快照来自用例，tests 挂断言 / 提取（引擎求值）。
    let mut run_ep = ep.clone();
    run_ep.name = case.name.clone();
    run_ep.method = case.method;
    run_ep.request.tests = Some(serde_json::json!({
        "assertions": case.assertions,
        "extract": case.extractors,
    }));
    let spec = case_spec(&case);
    let global_params = repo::get_global_params(&state.db)
        .await
        .map_err(api_error)?;
    let mut spec = render_spec(&spec, &vars);
    apply_global_params(&mut spec, &global_params, &vars);

    // URL：绝对直用；否则 {{base_url}} 前缀 + 路径变量代入（url_template_for 同构）。
    let is_abs = case.url_path.contains("://");
    let with_vars = apply_path_vars(&case.url_path, &case.path_variables);
    let url_template = if is_abs {
        with_vars
    } else if with_vars.starts_with('/') {
        format!("{{{{base_url}}}}{with_vars}")
    } else {
        format!("{{{{base_url}}}}/{with_vars}")
    };
    let url = resolve_variables(&url_template, &vars);
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(api_error(miyo_core::AppError::Validation(format!(
            "解析后的 URL 非法：{url}。环境缺少 base_url？"
        ))));
    }

    let timeout_ms = case
        .timeout_ms
        .or(spec.timeout_ms)
        .or(global_timeout_ms(&state.db).await)
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS);

    let started_at = chrono::Utc::now();
    let mut runtime = std::collections::HashMap::new();
    let (result, _) = miyo_test::runner::run_endpoint_with_cancel(
        &run_ep,
        &url,
        &spec,
        &mut runtime,
        Some(timeout_ms),
        None,
        true,
    )
    .await;

    // 落执行历史（fail-open）+ 摘要回写：与 Tauri 命令层同口径。
    let run_id = uuid::Uuid::new_v4();
    let ok_count = result.outcomes.iter().filter(|o| o.passed).count() as i64;
    let fail_count = result.outcomes.iter().filter(|o| !o.passed).count() as i64;
    let status = if result.cancelled {
        miyo_core::model::TestCaseStatus::Failed
    } else if result.ok {
        miyo_core::model::TestCaseStatus::Success
    } else {
        miyo_core::model::TestCaseStatus::Failed
    };
    let mut snapshot = result.clone();
    if let Some(resp) = snapshot.response.as_mut() {
        if resp.body.len() > HISTORY_BODY_BYTES {
            let (body, _) = truncate_utf8(&resp.body, HISTORY_BODY_BYTES);
            resp.body = body;
            resp.history_truncated = true;
        }
    }
    // 全量脱敏（请求快照 / 响应头 / body / 提取值 / 断言值）：同一份
    // results_json 会随 list_test_case_runs / get_test_case_run 回读。
    redact_endpoint_result(&mut snapshot);
    let record = miyo_core::model::TestCaseRun {
        id: run_id,
        case_id: case.id,
        endpoint_id: Some(ep.id),
        environment_id: req.environment_id,
        started_at,
        finished_at: chrono::Utc::now(),
        duration_ms: result.duration_ms,
        status,
        status_code: result.status,
        ok_count: ok_count.max(0) as u32,
        fail_count: fail_count.max(0) as u32,
        error: result.request_error.clone(),
        results_json: serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string()),
    };
    let ran_at = record.finished_at;
    if let Err(e) = async {
        repo::save_test_case_run(&state.db, &record).await?;
        repo::prune_test_case_runs(&state.db, case.id, repo::TEST_CASE_RUN_KEEP).await?;
        repo::update_test_case_run_summary(
            &state.db,
            case.id,
            status,
            ran_at,
            Some(record.ok_count),
            Some(record.fail_count),
        )
        .await
    }
    .await
    {
        tracing::warn!("[agent] run_test_case：落历史失败 {e}");
    }
    tracing::info!(
        "[agent] run_test_case：{} {} {} → {}（{}ms，断言 {}/{}）",
        case.name,
        case.method.as_str(),
        redact_url(&url),
        result
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into()),
        result.duration_ms.unwrap_or(0.0) as u64,
        ok_count,
        ok_count + fail_count,
    );
    Ok(Json(serde_json::json!({
        "runId": run_id,
        "caseId": case.id,
        "result": snapshot,
    })))
}

/// GET /agent/test-cases/{id}/runs：用例执行历史（新→旧）。
async fn list_test_case_runs_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Vec<miyo_core::model::TestCaseRun>> {
    ensure_tool_allowed(&state, "list_test_case_runs").await?;
    let mut runs = repo::list_test_case_runs(&state.db, id, q.limit.unwrap_or(50).clamp(1, 200))
        .await
        .map_err(api_error)?;
    // 读取端重脱敏：旧历史行（results_json 内响应 body / 提取值）不漏明文。
    for run in &mut runs {
        if let Ok(mut snap) =
            serde_json::from_str::<miyo_test::runner::EndpointResult>(&run.results_json)
        {
            redact_endpoint_result(&mut snap);
            if let Ok(json) = serde_json::to_string(&snap) {
                run.results_json = json;
            }
        }
    }
    Ok(Json(runs))
}

/// GET /agent/test-case-runs/{id}：单条执行历史详情。
async fn get_test_case_run_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<miyo_core::model::TestCaseRun> {
    ensure_tool_allowed(&state, "get_test_case_run").await?;
    let mut run = repo::get_test_case_run(&state.db, id)
        .await
        .map_err(api_error)?
        .ok_or_else(|| {
            api_error(miyo_core::AppError::NotFound(format!(
                "执行历史 {id} 不存在"
            )))
        })?;
    if let Ok(mut snap) =
        serde_json::from_str::<miyo_test::runner::EndpointResult>(&run.results_json)
    {
        redact_endpoint_result(&mut snap);
        if let Ok(json) = serde_json::to_string(&snap) {
            run.results_json = json;
        }
    }
    Ok(Json(run))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTestCaseRequest {
    pub environment_id: Option<uuid::Uuid>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

// ---------- 运行结果查询 ----------

/// POST `/agent/runs/{id}/cancel`：取消在途异步运行（cancel_run 工具的
/// HTTP 路径）。引擎在步间与在途请求处响应取消，返回已完成步骤并按
/// cancelled=true 落库；已结束 / 不存在的运行给出明确状态（幂等友好）。
/// GET `/agent/runs/{id}/progress`：长任务进度（done/total）。
///
/// wait=true 的场景运行在 HTTP 等待期间，miyo-mcp 据此向客户端转发
/// `notifications/progress`；运行结束即从共享表清除（404 = 无在途进度）。
async fn run_progress_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<serde_json::Value> {
    ensure_permission(&state, PermissionLevel::Read).await?;
    let slot = state
        .progress
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&id.to_string())
        .copied();
    match slot {
        Some((done, total)) => Ok(Json(serde_json::json!({
            "runId": id.to_string(),
            "done": done,
            "total": total,
        }))),
        None => Err(api_error(miyo_core::AppError::NotFound(format!(
            "运行（{id}）没有在途进度（已结束或不存在）"
        )))),
    }
}

/// GET `/agent/context`：Agent 上下文快照（get_context 工具的数据源）。
///
/// 只回**摘要**，不回完整请求规格——AI 需要详情时用 get_endpoint 等工具
/// 按需拉取；变量池经统一脱敏（redact_vars / environment_detail 同口径）。
async fn context_handler(State(state): State<AgentState>) -> ApiResult<serde_json::Value> {
    use miyo_core::sensitive;
    ensure_permission(&state, PermissionLevel::Read).await?;

    // 当前项目：settings 激活项；未激活时为 null（不替用户猜）。
    let active_project_id: Option<uuid::Uuid> = repo::get_setting(&state.db, "active_project_id")
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Option<String>>(&raw).ok().flatten())
        .and_then(|s| uuid::Uuid::parse_str(&s).ok());
    let project = match active_project_id {
        Some(id) => repo::get_project(&state.db, id).await.ok(),
        None => None,
    };
    let Some(project) = project else {
        return Ok(Json(serde_json::json!({
            "currentProject": null,
            "hint": "尚未激活项目（或激活项目不存在）。用 list_projects 查看全部项目后，请用户在应用中选择激活。",
        })));
    };

    // 当前环境：按项目记忆的激活环境。
    let env_key = format!("active_environment_id:{}", project.id);
    let active_env_id: Option<uuid::Uuid> = repo::get_setting(&state.db, &env_key)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Option<String>>(&raw).ok().flatten())
        .and_then(|s| uuid::Uuid::parse_str(&s).ok());
    let current_environment = match active_env_id {
        Some(id) => match repo::get_environment(&state.db, id).await {
            Ok(env) => {
                let detail = environment_detail(env).await;
                Some(serde_json::json!({
                    "id": detail.id,
                    "name": detail.name,
                    "baseUrl": detail.base_url,
                    "variables": detail.variables,
                }))
            }
            Err(_) => None,
        },
        None => None,
    };

    // 项目资产摘要（不含请求规格 / Body —— 详情按需经工具拉取）。
    let endpoints: Vec<serde_json::Value> = repo::list_endpoints(&state.db, project.id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id, "name": e.name,
                "method": e.method.as_str(), "path": e.path,
            })
        })
        .collect();
    let environments: Vec<serde_json::Value> = repo::list_environments(&state.db, project.id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|env| {
            serde_json::json!({
                "id": env.id, "name": env.name, "baseUrl": env.base_url,
                "variableCount": env.variables.iter().filter(|v| v.enabled).count(),
            })
        })
        .collect();
    let scenarios: Vec<serde_json::Value> = repo::list_scenarios(&state.db, project.id)
        .await
        .unwrap_or_default()
        .iter()
        .map(ScenarioSummary::from_scenario)
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();
    // 用例总数（批量查一次，聚合计数即可）。
    let endpoint_ids: Vec<uuid::Uuid> = repo::list_endpoints(&state.db, project.id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|e| e.id)
        .collect();
    let test_case_count = repo::list_test_cases_by_endpoints(&state.db, &endpoint_ids)
        .await
        .map(|cases| cases.len())
        .unwrap_or(0);

    // 最近一次场景运行（全项目最新一条摘要）。
    let last_run = repo::latest_run_summaries_for_project(&state.db, project.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .max_by(|a, b| a.finished_at.cmp(&b.finished_at))
        .map(|r| {
            serde_json::json!({
                "runId": r.id,
                "scenarioId": r.scenario_id,
                "finishedAt": r.finished_at,
                "okCount": r.ok_count,
                "failCount": r.fail_count,
                "skippedCount": r.skipped_count,
            })
        });

    // 项目变量脱敏（与 list_projects / get_scenario 同口径）。
    let variables: std::collections::HashMap<String, String> = project
        .variables
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                if sensitive::is_sensitive_key(k) && !v.is_empty() {
                    sensitive::PLACEHOLDER.to_string()
                } else {
                    v.clone()
                },
            )
        })
        .collect();

    Ok(Json(serde_json::json!({
        "currentProject": {
            "id": project.id,
            "name": project.name,
            "description": project.description,
            "variables": variables,
        },
        "currentEnvironment": current_environment,
        "endpoints": endpoints,
        "environments": environments,
        "scenarios": scenarios,
        "testCaseCount": test_case_count,
        "lastRun": last_run,
    })))
}

/// POST `/agent/runs/{id}/cancel` 的前一节：取消在途异步运行（cancel_run 工具的
/// HTTP 路径）。引擎在步间与在途请求处响应取消，返回已完成步骤并按
/// cancelled=true 落库；已结束 / 不存在的运行给出明确状态（幂等友好）。
async fn cancel_run_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<serde_json::Value> {
    ensure_tool_allowed(&state, "cancel_run").await?;
    let handle = {
        let runs = state.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.get(&id.to_string()).cloned()
    };
    let Some(h) = handle else {
        // 不在内存：已落库 / 已过期 / 未知。查 DB 给出可判定状态。
        let exists = repo::get_scenario_run(&state.db, id)
            .await
            .ok()
            .flatten()
            .is_some();
        if exists {
            return Ok(Json(serde_json::json!({
                "runId": id.to_string(),
                "status": "already-finished",
                "message": "该运行已结束，无需取消",
            })));
        }
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "运行（{id}）不存在"
        ))));
    };
    let status = *h.status.lock().unwrap_or_else(|p| p.into_inner());
    if run_status_terminal(status) {
        return Ok(Json(serde_json::json!({
            "runId": id.to_string(),
            "status": status,
            "message": "该运行已结束，无需取消",
        })));
    }
    h.cancel.cancel();
    // 注册表同步进入 cancelling：轮询方（get_run_result）可见中间态，
    // 引擎实际返回终态后由运行任务覆盖。
    *h.status.lock().unwrap_or_else(|p| p.into_inner()) = "cancelling";
    tracing::info!("[agent] run {id} 取消请求已受理");
    Ok(Json(serde_json::json!({
        "runId": id.to_string(),
        "status": "cancelling",
        "hint": "引擎将在当前请求中止后返回已完成步骤（cancelled=true），可用 GET /agent/runs/{runId} 轮询终态",
    })))
}

/// GET `/agent/runs/{id}`：运行状态与摘要（内存注册表优先，落库后走 DB）。
///
/// completed 响应 = RunScenarioResult 同形状（runId/scenarioId/汇总/每步摘要，
/// 正文只带 4KB 预览）；running 响应 = {runId, status}。
async fn get_run_result_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<serde_json::Value> {
    ensure_tool_allowed(&state, "get_run_result").await?;
    // 内存：在途运行。
    {
        let runs = state.runs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(h) = runs.get(&id.to_string()) {
            let status = *h.status.lock().unwrap_or_else(|p| p.into_inner());
            // running / cancelling 都是在途：终态才返回结果体。
            if !run_status_terminal(status) {
                return Ok(Json(
                    serde_json::json!({ "runId": id.to_string(), "status": status }),
                ));
            }
            if let Some(mut v) = h.result.lock().unwrap_or_else(|p| p.into_inner()).clone() {
                // 统一 status 字段（同步响应形状无 status——轮询方以此判定终态）。
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("status".into(), serde_json::json!(status));
                }
                return Ok(Json(v));
            }
        }
    }
    // DB：已完成（重启后仍可查）。
    let Some(run) = repo::get_scenario_run(&state.db, id)
        .await
        .map_err(api_error)?
    else {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "运行（{id}）不存在或尚未开始"
        ))));
    };
    let mut results: Vec<miyo_test::runner::EndpointResult> =
        serde_json::from_str(&run.results_json).unwrap_or_else(|e| {
            tracing::warn!("[get_run_result] results_json 解析失败：{e}");
            Vec::new()
        });
    // 读取端重脱敏：旧历史行（脱敏启用前落库的明文数据）不漏明文凭据。
    for r in &mut results {
        redact_endpoint_result(r);
    }
    let variables: std::collections::HashMap<String, String> =
        serde_json::from_str(&run.variables_json).unwrap_or_default();
    let step_summaries: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "index": r.step_index.unwrap_or_default(),
                "name": r.endpoint_name,
                "method": r.method,
                "url": r.url,
                "status": r.status,
                "stepStatus": r.effective_status(),
                "durationMs": r.duration_ms,
                "ok": r.ok,
                "assertionFailures": r.outcomes.iter().filter(|o| !o.passed).count(),
                "assertionsTotal": r.outcomes.len(),
                "extracted": r.extracted,
                "error": r.request_error,
                "blockedReason": r.blocked_reason,
                "loopRuns": r.loop_runs,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "runId": run.id.to_string(),
        "status": if run.cancelled { "cancelled" } else { "completed" },
        "scenarioId": run.scenario_id,
        "durationMs": run.duration_ms,
        "totalSteps": run.total_steps,
        "passed": run.ok_count,
        "failed": run.fail_count,
        "skipped": run.skipped_count,
        "cancelled": run.cancelled,
        "stoppedOnFailure": run.stopped_on_failure,
        "variables": redact_vars(&variables),
        "steps": step_summaries,
    })))
}

/// GET `/agent/runs/{id}/steps/{index}`：单步完整详情（请求快照 + 64KB 内
/// 响应正文 + 提取 + 逐条断言）。
async fn get_run_step_handler(
    State(state): State<AgentState>,
    AxumPath((id, index)): AxumPath<(uuid::Uuid, usize)>,
) -> ApiResult<serde_json::Value> {
    ensure_tool_allowed(&state, "get_step_result").await?;
    let Some(run) = repo::get_scenario_run(&state.db, id)
        .await
        .map_err(api_error)?
    else {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "运行（{id}）不存在"
        ))));
    };
    let mut results: Vec<miyo_test::runner::EndpointResult> =
        serde_json::from_str(&run.results_json).unwrap_or_default();
    // 读取端重脱敏：兜底旧历史行的明文凭据。
    for r in &mut results {
        redact_endpoint_result(r);
    }
    let Some(r) = results.get(index) else {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "步骤位次 {index} 越界（该运行共 {} 步结果）",
            results.len()
        ))));
    };
    let outcomes: Vec<serde_json::Value> = r
        .outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "description": o.description,
                "passed": o.passed,
                "reason": o.reason,
                "expected": o.expected,
                "actual": o.actual,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "runId": run.id.to_string(),
        "index": index,
        "name": r.endpoint_name,
        "method": r.method,
        "ok": r.ok,
        "stepStatus": r.effective_status(),
        "status": r.status,
        "durationMs": r.duration_ms,
        "loopRuns": r.loop_runs,
        "error": r.request_error,
        "blockedReason": r.blocked_reason,
        "request": r.response.as_ref().and_then(|resp| resp.request.clone()),
        "response": r.response.as_ref().map(|resp| serde_json::json!({
            "status": resp.status,
            "headers": resp.headers,
            "body": resp.body,
            "contentType": resp.content_type,
            "durationMs": resp.duration_ms,
            "sizeBytes": resp.size_bytes,
            "truncated": resp.truncated,
            "historyTruncated": resp.history_truncated,
        })),
        "extracted": r.extracted,
        "assertions": outcomes,
    })))
}

/// 同一断言连续失败的历史对比信息。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringFailure {
    /// 含本次在内连续失败的运行次数。
    pub consecutive_runs: i64,
    /// 最近一次失败的时间（RFC3339）。
    pub last_seen_at: String,
}

/// GET `/agent/runs/{id}/analysis`：失败原因分析（启发式规则，标注为推测）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedStepAnalysis {
    pub index: usize,
    pub name: String,
    pub method: String,
    /// 历史对比：同一断言在最近 N 次运行中连续失败（环境漂移 / 接口行为
    /// 变更的强信号）。None = 首次失败或无历史可比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring: Option<RecurringFailure>,
    /// 错误类型（REQUEST_ERROR / ASSERTION_FAILED / DEPENDENCY_BLOCKED /
    /// RUN_CANCELLED / LOOP_TIMEOUT…）。
    pub error_type: String,
    pub error: Option<String>,
    pub status: Option<u16>,
    /// 可能原因（推测，非确定结论）。
    pub possible_causes: Vec<String>,
    /// 建议处理方式。
    pub suggestions: Vec<String>,
    /// 失败断言明细。
    pub assertion_failures: Vec<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAnalysis {
    pub run_id: uuid::Uuid,
    pub ok: bool,
    pub total_steps: i64,
    pub passed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    pub failed_steps: Vec<FailedStepAnalysis>,
    /// 提示：分析为规则推测，修复前请用 get_step_result 核对原始响应。
    pub disclaimer: String,
}

async fn analyze_run_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<uuid::Uuid>,
) -> ApiResult<FailureAnalysis> {
    ensure_tool_allowed(&state, "analyze_run_failure").await?;
    let Some(run) = repo::get_scenario_run(&state.db, id)
        .await
        .map_err(api_error)?
    else {
        return Err(api_error(miyo_core::AppError::NotFound(format!(
            "运行（{id}）不存在"
        ))));
    };
    let mut results: Vec<miyo_test::runner::EndpointResult> =
        serde_json::from_str(&run.results_json).unwrap_or_default();
    // 断言 expected/actual 等原值脱敏（读取端重脱敏兜底旧数据）。
    for r in &mut results {
        redact_endpoint_result(r);
    }

    // ---- 历史对比：该场景最近几次运行（不含本次）的失败断言集合 ----
    // 同一断言（步位次 + 断言描述）连续失败 → 环境漂移 / 接口行为变更信号。
    let previous_runs = repo::list_scenario_runs(&state.db, run.scenario_id, 10)
        .await
        .unwrap_or_default();
    let mut history: Vec<(String, std::collections::HashSet<String>)> = Vec::new(); // (finished_at, failedAssertionKeys)
    for prev in previous_runs.iter().filter(|r| r.id != run.id) {
        let prev_results: Vec<miyo_test::runner::EndpointResult> =
            serde_json::from_str(&prev.results_json).unwrap_or_default();
        let keys: std::collections::HashSet<String> = prev_results
            .iter()
            .filter(|pr| !pr.ok && !pr.cancelled)
            .flat_map(|pr| {
                let idx = pr.step_index.unwrap_or_default();
                pr.outcomes
                    .iter()
                    .filter(|o| !o.passed)
                    .map(move |o| format!("{idx}::{}", o.description))
            })
            .collect();
        history.push((prev.finished_at.to_rfc3339(), keys));
    }
    // 历史按 finished_at 倒序（repo 排序），取连续失败计数：从最近一次往前，
    // 断言键集合包含当前失败键则计数 +1，首次不含即停。
    let history_tail: Vec<(String, std::collections::HashSet<String>)> =
        history.into_iter().take(9).collect();

    let mut failed_steps = Vec::new();
    for r in &results {
        let status = r.effective_status();
        if matches!(status, "success") {
            continue;
        }
        #[allow(unused_assignments)]
        let mut error_type = String::new();
        let mut causes: Vec<String> = Vec::new();
        let mut suggestions: Vec<String> = Vec::new();
        let mut error = r.request_error.clone();

        match status {
            "blocked" => {
                error_type = "DEPENDENCY_BLOCKED".into();
                causes.push(
                    r.blocked_reason
                        .clone()
                        .unwrap_or_else(|| "前序步骤失败，本步依赖的变量未产出".into()),
                );
                suggestions.push("先修复失败的前序步骤，再整体重跑（或用从该步重跑能力）".into());
            }
            "cancelled" => {
                error_type = "RUN_CANCELLED".into();
                causes.push("运行被用户或系统取消".into());
                suggestions.push("如需继续请重新运行场景".into());
            }
            _ if r
                .request_error
                .as_deref()
                .is_some_and(|e| e.contains("循环条件")) =>
            {
                error_type = "LOOP_TIMEOUT".into();
                causes.push("循环条件在最大轮数内未满足（轮询超时）".into());
                suggestions
                    .push("检查轮询目标服务的状态变更速度；适当调大 maxLoops / intervalMs".into());
            }
            _ if r
                .request_error
                .as_deref()
                .is_some_and(|e| e.contains("http/https")) =>
            {
                error_type = "URL_SCHEME_INVALID".into();
                causes
                    .push("URL 协议非法（仅允许 http/https）——可能因变量替换产生了意外地址".into());
                suggestions.push("检查 base_url 与 URL 模板的变量替换结果".into());
            }
            _ if r.request_error.is_some() => {
                error_type = "REQUEST_ERROR".into();
                let msg = r.request_error.clone().unwrap_or_default();
                causes.push(format!("请求未成功发送或连接失败：{msg}"));
                causes.push("目标服务不可达 / 域名或端口错误 / 环境选择错误".into());
                suggestions
                    .push("用 send_endpoint 单独探测该接口；确认环境的 Base URL 是否正确".into());
            }
            _ => {
                error_type = "ASSERTION_FAILED".into();
                let st = r.status.unwrap_or(0);
                match st {
                    401 | 403 => {
                        causes.push("认证失败：Token 缺失 / 过期 / 未被变量替换".into());
                        suggestions
                            .push("检查认证配置与 {{token}} 类变量是否来自前序登录步骤".into());
                    }
                    404 | 405 => {
                        causes.push("路径或方法不存在：URL 拼接 / 环境指向错误".into());
                        suggestions.push("核对实际请求 URL 与接口定义；确认环境 Base URL".into());
                    }
                    500..=599 => {
                        causes.push("服务端错误：可能由请求参数（变量值）不合法引发".into());
                        suggestions.push("检查请求 Body / Query 中变量替换后的实际值".into());
                    }
                    _ => {}
                }
                error = Some(format!(
                    "HTTP {st}：{} 条断言未通过",
                    r.outcomes.iter().filter(|o| !o.passed).count()
                ));
                suggestions
                    .push("对比断言的预期值与实际值；确认上游返回结构是否与提取假设一致".into());
            }
        }
        // 变量残留检查：URL / 提取值中含未解析 {{ }} → 变量未定义。
        if let Some(url) = &r.url {
            if url.contains("{{") {
                causes.push("请求 URL 仍含 {{占位}}——变量未定义或前序提取失败".into());
                suggestions
                    .push("运行前用 validate_scenario 做变量分析；补齐提取规则或环境变量".into());
            }
        }
        let assertion_failures: Vec<serde_json::Value> = r
            .outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| {
                serde_json::json!({
                    "description": o.description,
                    "expected": o.expected,
                    "actual": o.actual,
                    "reason": o.reason,
                })
            })
            .collect();
        // 连续失败计数：按当前失败断言键在前史中逐次回溯。
        let idx = r.step_index.unwrap_or_default();
        let current_keys: std::collections::HashSet<String> = r
            .outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| format!("{idx}::{}", o.description))
            .collect();
        let mut consecutive = 1i64;
        let mut last_seen = run.finished_at.to_rfc3339();
        for (at, keys) in &history_tail {
            if current_keys.is_subset(keys) && !keys.is_empty() {
                consecutive += 1;
                last_seen = at.clone();
            } else {
                break;
            }
        }
        let recurring = if consecutive >= 2 && !assertion_failures.is_empty() {
            Some(RecurringFailure {
                consecutive_runs: consecutive,
                last_seen_at: last_seen,
            })
        } else {
            None
        };
        if let Some(rec) = &recurring {
            suggestions.push(format!(
                "该断言已连续 {} 次运行失败（最近 {}）——优先怀疑环境漂移或接口行为变更，而非脚本问题",
                rec.consecutive_runs, rec.last_seen_at
            ));
        }
        failed_steps.push(FailedStepAnalysis {
            index: r.step_index.unwrap_or_default(),
            name: r.endpoint_name.clone(),
            method: r.method.clone(),
            recurring,
            error_type,
            error,
            status: r.status,
            possible_causes: causes,
            suggestions,
            assertion_failures,
        });
    }

    Ok(Json(FailureAnalysis {
        run_id: run.id,
        ok: run.fail_count == 0 && !run.cancelled,
        total_steps: run.total_steps,
        passed: run.ok_count,
        failed: run.fail_count,
        skipped: run.skipped_count,
        cancelled: run.cancelled,
        stopped_on_failure: run.stopped_on_failure,
        failed_steps,
        disclaimer:
            "以上原因为规则推测，非确定结论；修复前请用 get_step_result 核对原始请求与响应。".into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_result_roundtrip_with_response() {
        let r = miyo_test::runner::EndpointResult {
            endpoint_id: uuid::Uuid::new_v4(),
            endpoint_name: "登录".into(),
            method: "POST".into(),
            path: "/login".into(),
            ok: true,
            status: Some(200),
            duration_ms: Some(12.0),
            request_error: None,
            outcomes: vec![],
            extracted: Default::default(),
            url: Some("http://x".into()),
            step_index: Some(0),
            cancelled: false,
            response: Some(miyo_test::runner::StepResponse {
                status: 200,
                headers: vec![],
                body: "{\"ok\":true}".into(),
                content_type: "application/json".into(),
                duration_ms: 12.0,
                size_bytes: 9,
                truncated: false,
                body_omitted: false,
                history_truncated: true,
                request: Some(miyo_http::client::RequestSnapshot {
                    method: "POST".into(),
                    url: "http://x".into(),
                    url_template: None,
                    headers: vec![],
                    cookies: vec![],
                    body: String::new(),
                    body_omitted: false,
                }),
            }),
            step_status: Some("success".into()),
            blocked_reason: None,
            loop_runs: Some(2),
            phase: None,
            started_at: None,
            retry_log: Vec::new(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: miyo_test::runner::EndpointResult =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("回读失败：{e}｜{json}"));
        assert_eq!(back.endpoint_name, "登录");
        assert_eq!(back.loop_runs, Some(2));
    }

    #[test]
    fn normalize_step_ai_dialect() {
        // AI 方言：extractors + source/operator 断言 → 引擎形状。
        let raw = serde_json::json!({
            "name": "创建订单",
            "request": {"method": "POST", "url": "{{base_url}}/shop/orders", "spec": {}},
            "extractors": [
                {"name": "orderNo", "source": "response.body", "expression": "$.data.orderNo", "type": "jsonpath", "required": true},
                {"name": "sign", "source": "response.header", "expression": "x-sign"}
            ],
            "assertions": [
                {"source": "response.status", "operator": "equals", "expected": 200},
                {"source": "response.body", "expression": "$.data.status", "operator": "equals", "expected": "SUCCESS"},
                {"source": "response.body", "expression": "$.data.amount", "operator": "greaterThan", "expected": 0},
                {"source": "response.header", "expression": "x-req-id", "operator": "exists"},
                {"source": "performance", "operator": "lessThan", "expected": 2000}
            ]
        });
        let step = normalize_step_json(&raw).expect("归一化");
        assert_eq!(step.request.as_ref().unwrap().method, "POST");
        // 提取：body/header 两源。
        assert_eq!(step.extract.as_array().unwrap().len(), 2);
        assert_eq!(step.extract[0]["from"], "body");
        assert_eq!(step.extract[0]["path"], "$.data.orderNo");
        assert_eq!(step.extract[1]["from"], "header");
        // 断言：status/jsonpath×2/header/response_time_ms 五条。
        let arr = step.assertions.as_array().unwrap();
        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0]["type"], "status");
        assert_eq!(arr[0]["op"], "eq");
        assert_eq!(arr[1]["type"], "jsonpath");
        assert_eq!(arr[2]["op"], "gt");
        assert_eq!(arr[3]["type"], "header");
        assert_eq!(arr[4]["type"], "response_time_ms");
        assert_eq!(arr[4]["op"], "lt");
    }

    #[test]
    fn normalize_step_rejects_bad_shapes() {
        // 缺 request 与 endpointId。
        assert!(normalize_step_json(&serde_json::json!({"name": "x"})).is_err());
        // 不支持的 extractor source。
        assert!(normalize_step_json(&serde_json::json!({
            "endpointId": uuid::Uuid::new_v4(),
            "extractors": [{"name": "a", "source": "response.status", "expression": "$"}]
        }))
        .is_err());
        // 不支持的 operator。
        assert!(normalize_step_json(&serde_json::json!({
            "endpointId": uuid::Uuid::new_v4(),
            "assertions": [{"source": "response.status", "operator": "powerEquals", "expected": 1}]
        }))
        .is_err());
    }

    use crate::token::load_or_create_token;

    async fn spawn_test_server() -> (String, String, SqlitePool, AgentState) {
        let dir = std::env::temp_dir().join(format!("miyo-agent-srv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let db = miyo_storage::db::init_db(&db_path).await.expect("建库");
        // 测试默认放开全部权限等级（默认集会拒 network_execute/destructive，
        // 干扰行为测试；权限专项测试自行收窄等级）。
        repo::set_setting(
            &db,
            MCP_PERMISSIONS_KEY,
            &serde_json::json!(["read", "write", "execute", "network_execute", "destructive"])
                .to_string(),
        )
        .await
        .unwrap();
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
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
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
                    retry: None,
                    ..Default::default()
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(pay.id),
                    enabled: true,
                    extract: serde_json::json!([]),
                    assertions: serde_json::json!([]),
                    retry: None,
                    ..Default::default()
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
    async fn get_environment_masks_unmarked_sensitive_keys() {
        let (url, token, db, _state) = spawn_test_server().await;
        let project = repo::create_project(&db, "环境脱敏", "").await.unwrap();
        let env = miyo_core::model::Environment {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "prod".into(),
            base_url: "http://x".into(),
            variables: vec![
                // 未勾选 sensitive，但键名命中敏感词根 → 也必须脱敏
                miyo_core::model::EnvironmentVariable {
                    key: "api_key".into(),
                    remote_value: "sk-plain-secret".into(),
                    local_value: String::new(),
                    enabled: true,
                    local_override: None,
                    sensitive: None,
                    description: None,
                },
                // 显式标记 sensitive → 脱敏
                miyo_core::model::EnvironmentVariable {
                    key: "pass".into(),
                    remote_value: "hunter2".into(),
                    local_value: String::new(),
                    enabled: true,
                    local_override: None,
                    sensitive: Some(true),
                    description: None,
                },
                // 普通变量 → 原样
                miyo_core::model::EnvironmentVariable {
                    key: "region".into(),
                    remote_value: "cn-north".into(),
                    local_value: String::new(),
                    enabled: true,
                    local_override: None,
                    sensitive: None,
                    description: None,
                },
            ],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_environment(&db, &env).await.unwrap();

        let res = client()
            .get(format!("{url}/agent/environments/{}", env.id))
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        let vars = body["variables"].as_array().unwrap();
        let find = |k: &str| {
            vars.iter()
                .find(|v| v["key"] == k)
                .unwrap_or_else(|| panic!("缺变量 {k}"))
                .clone()
        };
        let api_key = find("api_key");
        assert_eq!(
            api_key["value"],
            miyo_sync::redact::PLACEHOLDER,
            "键名命中敏感词根（未标记）也必须脱敏"
        );
        assert_eq!(api_key["sensitive"], true);
        assert_eq!(find("pass")["value"], miyo_sync::redact::PLACEHOLDER);
        assert_eq!(find("region")["value"], "cn-north", "普通变量保留原值");
        db.close().await;
    }

    #[tokio::test]
    async fn auth_rejects_non_local_host_header() {
        let (url, token, _db, _state) = spawn_test_server().await;
        let client = client();
        let base = |host: String| {
            client
                .get(format!("{url}/agent/health"))
                .header("Authorization", &format!("Bearer {token}"))
                .header("Host", host)
        };
        // 合法 Host：放行
        for ok in [
            "127.0.0.1",
            "localhost",
            "[::1]",
            "127.0.0.1:4010",
            "LOCALHOST",
        ] {
            let res = base(ok.to_string()).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "Host {ok} 应放行");
        }
        // 攻击者域名（DNS rebinding 特征）：拒绝
        for bad in ["evil.example.com", "internal.corp:80"] {
            let res = base(bad.to_string()).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "Host {bad} 应拒绝");
            let body: serde_json::Value = res.json().await.unwrap();
            assert_eq!(body["code"], "FORBIDDEN");
        }
        _db.close().await;
    }

    // ---------- P0 生命周期：并发闸原子性 / 注册表封顶 / cancelling 中间态 ----------

    /// 注册表 / 并发闸测试用的最小状态（无 HTTP 服务）。
    async fn registry_state() -> (AgentState, SqlitePool) {
        let dir = std::env::temp_dir().join(format!("miyo-agent-reg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = miyo_storage::db::init_db(&dir.join("t.db")).await.unwrap();
        (AgentState::new(db.clone(), "tok"), db)
    }

    #[tokio::test]
    async fn concurrency_gate_is_atomic_and_rejects_over_limit() {
        let (state, db) = registry_state().await;
        // 直接以运行句柄压满并发闸：第 9 个必须被拒（判定与插入同锁）。
        for i in 0..MAX_CONCURRENT_RUNS {
            assert!(
                state.try_acquire_run_slot(&format!("run-{i}"), running_handle()),
                "前 {MAX_CONCURRENT_RUNS} 个应成功"
            );
        }
        assert!(
            !state.try_acquire_run_slot("run-extra", running_handle()),
            "超限必须被拒绝"
        );
        // 释放一个（置终态）后可再进。
        if let Some(h) = state.runs.lock().unwrap().get("run-0") {
            *h.status.lock().unwrap() = "completed";
        }
        assert!(state.try_acquire_run_slot("run-extra", running_handle()));
        db.close().await;
    }

    #[tokio::test]
    async fn sweep_runs_caps_completed_entries() {
        let (state, db) = registry_state().await;
        for i in 0..(MAX_COMPLETED_RUNS + 10) {
            let h = running_handle();
            *h.status.lock().unwrap() = "completed";
            *h.finished_at.lock().unwrap() = Some(std::time::Instant::now());
            state.runs.lock().unwrap().insert(format!("done-{i}"), h);
        }
        state.sweep_runs();
        let completed = {
            let runs = state.runs.lock().unwrap();
            runs.values()
                .filter(|h| *h.status.lock().unwrap() == "completed")
                .count()
        };
        assert!(
            completed <= MAX_COMPLETED_RUNS,
            "终态条目必须封顶（{completed} > {MAX_COMPLETED_RUNS}）"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn drain_runs_aborts_stragglers_within_grace() {
        let (state, db) = registry_state().await;
        let handle = running_handle();
        // 模拟协作式取消无响应的任务：永挂 JoinHandle。
        let join = tokio::spawn(async {
            // 模拟取消令牌失效的长任务：sleep 一个远超 grace 的时长。
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        *handle.join.lock().unwrap() = Some(join);
        // 引擎侧不响应（status 保持 running）。
        state
            .runs
            .lock()
            .unwrap()
            .insert("stuck-run".into(), handle);
        let started = std::time::Instant::now();
        state.drain_runs().await;
        // 必须在 grace + 余量内返回（不能等 3600s）。
        assert!(
            started.elapsed() < RUNS_SHUTDOWN_GRACE + std::time::Duration::from_secs(3),
            "drain_runs 必须有界返回：{:?}",
            started.elapsed()
        );
        db.close().await;
    }

    #[test]
    fn terminal_status_helper() {
        assert!(run_status_terminal("completed"));
        assert!(run_status_terminal("failed"));
        assert!(!run_status_terminal("running"));
        assert!(!run_status_terminal("cancelling"));
    }

    // ---------- P0 脱敏：运行结果 / 用例定义 ----------

    #[test]
    fn redact_endpoint_result_masks_all_sensitive_outputs() {
        use miyo_sync::redact::PLACEHOLDER;
        use miyo_test::runner::{EndpointResult, StepResponse};
        let mut r = EndpointResult {
            endpoint_id: uuid::Uuid::new_v4(),
            endpoint_name: "login".into(),
            method: "POST".into(),
            path: "/login".into(),
            ok: true,
            status: Some(200),
            duration_ms: Some(1.0),
            request_error: Some(
                "error sending request for url (https://a.com/login?access_token=sec)".into(),
            ),
            outcomes: vec![miyo_test::assert::Outcome {
                description: "body.token eq".into(),
                passed: false,
                reason: None,
                expected: Some("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U".into()),
                actual: Some("https://a.com/real?password=pw-1".into()),
            }],
            extracted: std::collections::HashMap::from([(
                "access_token".to_string(),
                "tok-1".to_string(),
            )]),
            url: Some("https://a.com/login?access_token=sec".into()),
            step_index: Some(0),
            cancelled: false,
            response: Some(StepResponse {
                status: 200,
                headers: vec![("Set-Cookie".to_string(), "sid=secret-session".to_string())],
                body: r#"{"token":"bt-1","keep":1}"#.into(),
                content_type: "application/json".into(),
                duration_ms: 1.0,
                size_bytes: 22,
                truncated: false,
                body_omitted: false,
                history_truncated: false,
                request: Some(miyo_http::client::RequestSnapshot {
                    method: "POST".into(),
                    url: "https://a.com/login?access_token=sec".into(),
                    url_template: None,
                    headers: vec![("Authorization".into(), "Bearer t".into())],
                    cookies: Vec::new(),
                    body: "{}".into(),
                    body_omitted: false,
                }),
            }),
            step_status: Some("success".into()),
            blocked_reason: None,
            loop_runs: None,
            phase: None,
            started_at: None,
            retry_log: Vec::new(),
        };
        redact_endpoint_result(&mut r);
        assert!(r.url.as_deref().unwrap().contains(PLACEHOLDER));
        assert_eq!(
            r.extracted.get("access_token").map(String::as_str),
            Some(PLACEHOLDER)
        );
        let resp = r.response.as_ref().unwrap();
        assert_eq!(
            resp.headers
                .iter()
                .find(|(k, _)| k == "Set-Cookie")
                .unwrap()
                .1,
            PLACEHOLDER,
            "Set-Cookie 必须脱敏"
        );
        assert!(
            !resp.body.contains("bt-1") && resp.body.contains(PLACEHOLDER),
            "JSON body 敏感字段必须脱敏：{}",
            resp.body
        );
        assert!(resp.body.contains("\"keep\":1"), "非敏感字段保留");
        let snap = resp.request.as_ref().unwrap();
        assert_eq!(
            snap.headers
                .iter()
                .find(|(k, _)| k == "Authorization")
                .unwrap()
                .1,
            PLACEHOLDER
        );
        assert!(snap.url.contains(PLACEHOLDER));
        // 断言值：JWT 形态 → 占位符；URL 凭据 → query 脱敏。
        let o = &r.outcomes[0];
        assert_eq!(o.expected.as_deref(), Some(PLACEHOLDER));
        assert!(
            !o.actual.as_deref().unwrap().contains("pw-1"),
            "actual 里的 URL 凭据必须脱敏：{:?}",
            o.actual
        );
        // request_error 里的 URL 凭据脱敏由 redact_message_urls 在映射层兜底。
    }

    #[test]
    fn redact_test_case_masks_auth_headers_and_body() {
        use miyo_core::model::{ApiKeyLocation, AuthSpec, KeyValue, TestCase, TestCaseStatus};
        use miyo_sync::redact::PLACEHOLDER;
        let mut case = TestCase {
            id: uuid::Uuid::new_v4(),
            request_id: uuid::Uuid::new_v4(),
            name: "登录".into(),
            category: "正向".into(),
            folder: String::new(),
            method: miyo_core::model::HttpMethod::POST,
            url_path: "https://a.com/login?api_key=sec".into(),
            path_variables: Vec::new(),
            params: vec![KeyValue::new("next", "2")],
            headers: vec![KeyValue::new("Authorization", "Bearer t-1")],
            body_type: "json".into(),
            body_content: r#"{"user":"u","password":"pw-1"}"#.into(),
            assertions: serde_json::json!([]),
            extractors: serde_json::json!([]),
            auth: Some(AuthSpec::ApiKey {
                key: "X-Key".into(),
                value: "kv-1".into(),
                location: ApiKeyLocation::Header,
            }),
            timeout_ms: None,
            disable_cookies: false,
            sort_order: 0,
            last_run_status: TestCaseStatus::Untested,
            last_run_at: None,
            last_run_ok: None,
            last_run_fail: None,
            created_at: chrono::Utc::now(),
        };
        redact_test_case(&mut case);
        assert_eq!(case.headers[0].value, PLACEHOLDER);
        assert_eq!(case.params[0].value, "2", "非敏感参数保留");
        assert!(
            !case.body_content.contains("pw-1"),
            "JSON body 密码必须脱敏：{}",
            case.body_content
        );
        assert!(case.body_content.contains("u"), "非敏感字段保留");
        assert!(
            matches!(&case.auth, Some(AuthSpec::ApiKey { value, .. }) if value == PLACEHOLDER),
            "auth 凭据必须脱敏"
        );
    }

    #[test]
    fn redact_url_masks_sensitive_query() {
        use miyo_sync::redact::PLACEHOLDER;
        assert_eq!(
            redact_url("https://x.com/api?access_token=abc&page=2"),
            format!("https://x.com/api?access_token={PLACEHOLDER}&page=2")
        );
        // 无 query 原样返回；非敏感参数不动
        assert_eq!(redact_url("https://x.com/api"), "https://x.com/api");
        assert_eq!(
            redact_url("https://x.com/api?page=2&size=10"),
            "https://x.com/api?page=2&size=10"
        );
    }

    /// 手工构造注册表句柄（取消 / 并发上限测试用）。
    fn running_handle() -> RunHandle {
        RunHandle {
            status: Arc::new(std::sync::Mutex::new("running")),
            result: Arc::new(std::sync::Mutex::new(None)),
            cancel: Arc::new(tokio_util::sync::CancellationToken::new()),
            finished_at: Arc::new(std::sync::Mutex::new(None)),
            join: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    #[tokio::test]
    async fn cancel_run_endpoint_cancels_and_reports_states() {
        let (url, token, db, state) = spawn_test_server().await;
        // 在途运行：取消受理 + 令牌触发
        let handle = running_handle();
        let run_id = uuid::Uuid::new_v4();
        state
            .runs
            .lock()
            .unwrap()
            .insert(run_id.to_string(), handle.clone());
        let res = client()
            .post(format!("{url}/agent/runs/{run_id}/cancel"))
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["status"], "cancelling");
        assert!(handle.cancel.is_cancelled());

        // 已结束的运行：幂等返回 already-finished（令牌不再触发也无妨）
        *handle.status.lock().unwrap() = "completed";
        let res = client()
            .post(format!("{url}/agent/runs/{run_id}/cancel"))
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["status"], "completed");

        // 未知运行：404
        let res = client()
            .post(format!("{url}/agent/runs/{}/cancel", uuid::Uuid::new_v4()))
            .header("Authorization", &format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        db.close().await;
    }

    /// 数据驱动（MCP dataSets）：每组独立执行 / 独立落历史（meta.matrix）/
    /// 响应为逐行汇总；行间变量零污染。
    #[tokio::test]
    async fn run_scenario_data_sets_runs_rows_isolated() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "数据驱动", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();

        let users = make_endpoint(project.id, "用户", "GET", "/users/{{uid}}");
        repo::save_endpoint(&db, &users).await.unwrap();
        let scenario = Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "多组用户查询".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![miyo_core::model::ScenarioStep {
                endpoint_id: Some(users.id),
                enabled: true,
                extract: serde_json::json!([{ "name": "userId", "from": "body", "path": "$.userId" }]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({
                "dataSets": [
                    { "name": "组1", "vars": { "uid": "1" } },
                    { "name": "组2", "vars": { "uid": "2" } }
                ]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["kind"], "data-driven", "{result}");
        assert_eq!(result["total"], 2);
        assert_eq!(result["okRows"], 2);
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        // 行间隔离：每组提取自己的 userId。
        assert_eq!(rows[0]["steps"][0]["extracted"]["userId"], "1");
        assert_eq!(rows[1]["steps"][0]["extracted"]["userId"], "2");
        // 逐行独立落历史（meta.matrix 标注行信息；行 runId 即历史行 id，
        // get_run_result 可按响应 runId 查询该行）。
        let runs = repo::list_scenario_runs(&db, scenario.id, 10)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2, "两组各落一条历史");
        for row in rows {
            let run_id = row["runId"].as_str().unwrap();
            assert!(
                runs.iter().any(|r| r.id.to_string() == run_id),
                "响应 runId {run_id} 应对应历史行；runs={:?}",
                runs.iter()
                    .map(|r| (r.id.to_string(), r.meta_json.clone()))
                    .collect::<Vec<_>>()
            );
        }
        let meta1: serde_json::Value = serde_json::from_str(
            runs.iter()
                .find(|r| {
                    let meta: serde_json::Value =
                        serde_json::from_str(r.meta_json.as_deref().unwrap_or("null")).unwrap();
                    meta["matrix"]["label"] == "组1"
                })
                .unwrap()
                .meta_json
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(meta1["matrix"]["kind"], "data");
        assert_eq!(meta1["matrix"]["label"], "组1");
        assert_eq!(
            meta1["matrix"]["groupId"], result["runId"],
            "groupId 关联同组行"
        );
        db.close().await;
    }

    /// 生命周期（agent 引擎）：setup 提取 → 主步骤引用 → teardown 清理；
    /// 响应带 setupResults / teardownResults（phase 标注），历史行含 phase 行。
    #[tokio::test]
    async fn run_scenario_lifecycle_setup_teardown_via_agent() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "生命周期", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();

        let login = make_endpoint(project.id, "登录", "POST", "/echo");
        let mut users = make_endpoint(project.id, "用户", "GET", "/users/{{uid}}");
        users.path = "/users/{{uid}}".into();
        let mut del = make_endpoint(project.id, "清理", "DELETE", "/orders/x");
        del.path = "/orders/x".into();
        repo::save_endpoint(&db, &login).await.unwrap();
        repo::save_endpoint(&db, &users).await.unwrap();
        repo::save_endpoint(&db, &del).await.unwrap();

        // setup / teardown 为内联步骤（agent 引擎按内联物化）。
        let scenario = Scenario {
            setup: vec![miyo_core::model::ScenarioStep {
                endpoint_id: None,
                enabled: true,
                request: Some(miyo_core::model::ScenarioInlineRequest {
                    method: "POST".into(),
                    url: "{{base_url}}/echo".into(),
                    spec: miyo_core::model::RequestSpec {
                        body: miyo_core::model::BodySpec::Text { raw: "42".into() },
                        ..Default::default()
                    },
                }),
                extract: serde_json::json!([{ "name": "uid", "from": "body", "path": "$.body" }]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            teardown: vec![miyo_core::model::ScenarioStep {
                endpoint_id: Some(del.id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "生命周期链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![miyo_core::model::ScenarioStep {
                endpoint_id: Some(users.id),
                enabled: true,
                extract: serde_json::json!([{ "name": "userId", "from": "body", "path": "$.userId" }]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let result: serde_json::Value = res.json().await.unwrap();
        assert_eq!(result["passed"], 1, "{result}");
        // 响应带生命周期摘要（phase 标注）。
        assert_eq!(result["setupResults"][0]["phase"], "setup");
        assert_eq!(result["teardownResults"][0]["phase"], "teardown");
        // setup 的提取（uid）流入主步骤 URL：/users/42 → userId=42。
        assert_eq!(result["steps"][0]["url"], format!("{upstream}/users/42"));
        assert_eq!(result["steps"][0]["extracted"]["userId"], "42");

        // 历史：results_json = setup(phase) + 主步骤 + teardown(phase)。
        let runs = repo::list_scenario_runs(&db, scenario.id, 5).await.unwrap();
        assert_eq!(runs.len(), 1);
        let history: Vec<miyo_test::runner::EndpointResult> =
            serde_json::from_str(&runs[0].results_json).unwrap();
        assert_eq!(history.len(), 3, "setup + 主 + teardown 三行");
        assert_eq!(history[0].phase.as_deref(), Some("setup"));
        assert_eq!(history[1].phase, None);
        assert_eq!(history[2].phase.as_deref(), Some("teardown"));
        // 成败计数只看主步骤。
        assert_eq!(runs[0].ok_count, 1);
        db.close().await;
    }

    #[tokio::test]
    async fn run_scenario_async_rejects_when_concurrent_cap_reached() {
        let (url, token, db, state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "并发上限", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();
        let order = make_endpoint(project.id, "下单", "GET", "/orders");
        repo::save_endpoint(&db, &order).await.unwrap();
        let scenario = Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "占位场景".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![miyo_core::model::ScenarioStep {
                endpoint_id: Some(order.id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        // 占满并发额度
        for i in 0..MAX_CONCURRENT_RUNS {
            state
                .runs
                .lock()
                .unwrap()
                .insert(format!("busy-{i}"), running_handle());
        }
        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({ "wait": false }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["code"], "VALIDATION");
        assert!(body["message"].as_str().unwrap().contains("并发"));
        db.close().await;
    }

    #[tokio::test]
    async fn run_scenario_async_redacts_memory_result_variables() {
        let (url, token, db, _state) = spawn_test_server().await;
        let upstream = spawn_upstream().await;
        let project = repo::create_project(&db, "内存脱敏", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();
        let order = make_endpoint(project.id, "下单", "GET", "/orders");
        repo::save_endpoint(&db, &order).await.unwrap();
        let scenario = Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "异步脱敏".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![miyo_core::model::ScenarioStep {
                endpoint_id: Some(order.id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([]),
                retry: None,
                ..Default::default()
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        // wait=false 异步运行：注入敏感变量，完成后内存结果必须脱敏
        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &format!("Bearer {token}"))
            .json(&serde_json::json!({
                "wait": false,
                "variables": { "apiToken": "sk-secret-123", "plainNote": "可读" }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let started: serde_json::Value = res.json().await.unwrap();
        let run_id = started["runId"].as_str().unwrap().to_string();

        // 轮询到终态（最长 ~10s；回显上游即时响应，通常一两轮内完成）
        let mut final_body: Option<serde_json::Value> = None;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let poll = client()
                .get(format!("{url}/agent/runs/{run_id}"))
                .header("Authorization", &format!("Bearer {token}"))
                .send()
                .await
                .unwrap();
            assert_eq!(poll.status(), StatusCode::OK);
            let body: serde_json::Value = poll.json().await.unwrap();
            if body["status"] != "running" {
                final_body = Some(body);
                break;
            }
        }
        let body = final_body.expect("异步运行应在超时前完成");
        assert_eq!(body["status"], "completed", "最终态：{body}");
        assert_eq!(body["passed"], 1);
        assert_eq!(
            body["variables"]["apiToken"],
            miyo_sync::redact::PLACEHOLDER,
            "内存结果的敏感变量必须脱敏"
        );
        assert_eq!(body["variables"]["plainNote"], "可读", "非敏感变量保留");
        db.close().await;
    }

    /// 验收主链路（电商下单 → 支付预览 → 支付）：DB 场景 → 控制面运行 →
    /// 变量链路自动传递 → 断言 → 运行历史落库。
    #[tokio::test]
    async fn run_scenario_ecommerce_full_chain() {
        use axum::routing::get as axum_get;
        let (url, token, db, _state) = spawn_test_server().await;
        let auth = format!("Bearer {token}");

        // 三步上游：校验链路传参（缺参 400）+ 金额数字类型。
        let app = axum::Router::new()
            .route(
                "/shop/orders",
                axum::routing::post(|| async {
                    Json(serde_json::json!({
                        "orderNo": "SO-1",
                        "payToken": "pt-9"
                    }))
                }),
            )
            .route(
                "/shop/pay-preview",
                axum_get(|Query(q): Query<std::collections::HashMap<String, String>>| async move {
                    let ok = q.get("orderNo").map(|v| v == "SO-1").unwrap_or(false)
                        && q.get("payToken").map(|v| v == "pt-9").unwrap_or(false);
                    if ok {
                        (axum::http::StatusCode::OK, Json(serde_json::json!({"payableAmount": 388.2})))
                    } else {
                        (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "x"})))
                    }
                }),
            )
            .route(
                "/shop/pay",
                axum::routing::post(|body: String| async move {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let number_amount = matches!(v.get("amount"), Some(serde_json::Value::Number(_)));
                    if number_amount && v.get("payToken").and_then(|t| t.as_str()) == Some("pt-9") {
                        // 回显金额类型：类型保留渲染的验证信号（数字 ≠ 字符串）。
                        (axum::http::StatusCode::OK, Json(serde_json::json!({"status": "PAID", "tradeNo": "ALI-1", "amountWasNumber": number_amount})))
                    } else {
                        (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "x"})))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // 项目变量 base_url → 上游。
        let project = repo::create_project(&db, "电商验收", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), upstream.clone());
        repo::save_project(&db, &with_base).await.unwrap();

        let order = make_endpoint(project.id, "创建订单", "POST", "/shop/orders");
        // 预览：Query 带第 1 步提取的 orderNo / payToken（种子场景同款）。
        let mut preview = make_endpoint(project.id, "支付预览", "GET", "/shop/pay-preview");
        preview.request.params = vec![
            miyo_core::model::KeyValue::new("orderNo", "{{orderNo}}"),
            miyo_core::model::KeyValue::new("payToken", "{{payToken}}"),
        ];
        let mut pay = make_endpoint(project.id, "确认支付", "POST", "/shop/pay");
        pay.request.body = miyo_core::model::BodySpec::Json {
            raw: r#"{"amount": "{{payableAmount}}", "payToken": "{{payToken}}"}"#.into(),
        };
        for ep in [&order, &preview, &pay] {
            repo::save_endpoint(&db, ep).await.unwrap();
        }
        let scenario = Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            name: "电商主流程".into(),
            description: String::new(),
            category: "电商".into(),
            sort_order: 0,
            steps: vec![
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(order.id),
                    enabled: true,
                    extract: serde_json::json!([
                        {"name": "orderNo", "from": "body", "path": "$.orderNo"},
                        {"name": "payToken", "from": "body", "path": "$.payToken"}
                    ]),
                    assertions: serde_json::json!([{"type": "status", "op": "eq", "expected": 200}]),
                    retry: None,
                    ..Default::default()
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(preview.id),
                    enabled: true,
                    extract: serde_json::json!([
                        {"name": "payableAmount", "from": "body", "path": "$.payableAmount"}
                    ]),
                    assertions: serde_json::json!([{"type": "status", "op": "eq", "expected": 200}]),
                    retry: None,
                    ..Default::default()
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(pay.id),
                    enabled: true,
                    extract: serde_json::json!([
                        {"name": "tradeNo", "from": "body", "path": "$.tradeNo"}
                    ]),
                    assertions: serde_json::json!([
                        {"type": "jsonpath", "path": "$.status", "op": "eq", "expected": "PAID"}
                    ]),
                    retry: None,
                    ..Default::default()
                },
            ],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        // 控制面运行。
        let res = client()
            .post(format!("{url}/agent/scenarios/{}/run", scenario.id))
            .header("Authorization", &auth)
            .json(&serde_json::json!({ "stopOnFailure": false }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["passed"], 3, "三步全过：{body}");
        assert_eq!(body["failed"], 0);
        // 变量链路：无需手动复制任何返回值。
        assert_eq!(body["variables"]["orderNo"], "SO-1");
        assert_eq!(body["variables"]["payableAmount"], "388.2");
        assert_eq!(body["variables"]["tradeNo"], "ALI-1");
        // 第三步：金额保持数字（类型保留）——上游校验通过且回显 amountWasNumber=true。
        let steps = body["steps"].as_array().unwrap();
        let pay_body = steps[2]["bodyPreview"].as_str().unwrap_or("");
        let resp: serde_json::Value = serde_json::from_str(pay_body).unwrap_or_default();
        assert_eq!(
            resp["amountWasNumber"],
            serde_json::json!(true),
            "金额应为数字类型：{pay_body}"
        );

        // 运行历史已落库（可回看）。
        let runs = repo::list_scenario_runs(&db, scenario.id, 5).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].ok_count, 3);
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
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
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
                    retry: None,
                    ..Default::default()
                },
                miyo_core::model::ScenarioStep {
                    endpoint_id: Some(b.id),
                    enabled: true,
                    extract: serde_json::json!([]),
                    assertions: serde_json::json!([]),
                    retry: None,
                    ..Default::default()
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
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
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
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
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
                retry: None,
                ..Default::default()
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
        // 无 token → 401（health 现为公开探针——challenge-proof 见
        // health_challenge_proof_verifies_token 测试；鉴权面用 /agent/projects）
        let res = client()
            .get(format!("{url}/agent/projects"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // 错误 token → 401
        let res = client()
            .get(format!("{url}/agent/projects"))
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

    // ---------- P0 MCP 权限模型：服务端权威校验（不依赖 MCP 层隐藏） ----------

    /// 把权限等级写入 settings（测试辅助）。
    async fn set_permissions(db: &SqlitePool, levels: &[&str]) {
        repo::set_setting(
            db,
            MCP_PERMISSIONS_KEY,
            &serde_json::json!(levels).to_string(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn network_execute_is_independent_from_execute_and_read() {
        let (url, token, db, _state) = spawn_test_server().await;
        let http = client();
        // 仅启用 execute：send_endpoint（network_execute）必须被拒，
        // 场景运行（execute）不被权限拦截。
        set_permissions(&db, &["execute"]).await;

        // send_endpoint：权限在参数校验之前（路由第一行），任意参数都 403。
        let resp = http
            .post(format!(
                "{url}/agent/endpoints/{}/execute",
                uuid::Uuid::new_v4()
            ))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            403,
            "network_execute 关闭时 send_endpoint 必须 403"
        );
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], "PERMISSION_DENIED", "{body}");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("network_execute"),
            "拒绝消息必须包含等级标识：{body}"
        );

        // run_scenario（execute）：路由可达（403 不会出现；参数错误 / 场景缺失才是预期）。
        let resp = http
            .post(format!(
                "{url}/agent/scenarios/{}/run",
                uuid::Uuid::new_v4()
            ))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_ne!(
            resp.status(),
            403,
            "execute 启用时 run_scenario 不得被权限拦截"
        );

        // list_projects（read）：read 未启用 → 403。
        let resp = http
            .get(format!("{url}/agent/projects"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403, "read 关闭时 list_projects 必须 403");

        db.close().await;
    }

    #[tokio::test]
    async fn destructive_permission_gates_delete_endpoints() {
        let (url, token, db, _state) = spawn_test_server().await;
        let http = client();
        // spawn 预置了全开：先复位为默认缺省（删除键）——destructive 默认关闭。
        repo::delete_setting(&db, MCP_PERMISSIONS_KEY)
            .await
            .unwrap();
        // 默认权限（缺省 = read/write/execute）：delete_scenario（destructive）必须 403。
        let resp = http
            .delete(format!("{url}/agent/scenarios/{}", uuid::Uuid::new_v4()))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403, "destructive 默认关闭：删除场景必须 403");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], "PERMISSION_DENIED");

        // 显式启用 destructive → 路由放行（404 = 场景不存在，不再是权限错误）。
        set_permissions(&db, &["read", "write", "execute", "destructive"]).await;
        let resp = http
            .delete(format!("{url}/agent/scenarios/{}", uuid::Uuid::new_v4()))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            404,
            "destructive 启用后应到达业务层（场景不存在 404）"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn default_permissions_exclude_high_risk() {
        // 手工建库（不经 spawn_test_server——它会预置全开权限）。
        let dir = std::env::temp_dir().join(format!("miyo-agent-def-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = miyo_storage::db::init_db(&dir.join("t.db")).await.unwrap();
        let state = AgentState::new(db.clone(), "tok");
        // 全新库（未配置 mcp_permissions）：默认 = read/write/execute。
        let enabled = state.enabled_permissions().await;
        assert!(enabled.contains(&PermissionLevel::Read));
        assert!(enabled.contains(&PermissionLevel::Write));
        assert!(enabled.contains(&PermissionLevel::Execute));
        assert!(!enabled.contains(&PermissionLevel::NetworkExecute));
        assert!(!enabled.contains(&PermissionLevel::Destructive));
        // 空数组 = 用户全关（显式配置优先于默认集）。
        set_permissions(&db, &[]).await;
        assert!(state.enabled_permissions().await.is_empty());
        db.close().await;
    }

    /// 发现协议：无凭据探针换 challenge 证明——正确 token 的客户端通过，
    /// 错误 token 的客户端证明不匹配被拒（恶意监听者无法收割 token）。
    #[tokio::test]
    async fn health_challenge_proof_verifies_token() {
        use crate::client::ControlClient;
        let (url, token, _db, _state) = spawn_test_server().await;
        // 正确 token：证明匹配。
        let good = ControlClient::new(url.clone(), token);
        assert!(good.health().await.is_ok(), "真控制面证明应通过");
        // 错误 token：无法验证证明（防伪装）。
        let rogue = ControlClient::new(url.clone(), "wrong-token");
        assert!(rogue.health().await.is_err(), "伪装客户端证明必须失败");
        // 无凭据探针确实没有发送 Bearer（握手期零凭据暴露由协议保证，
        // 此处验证公开探针不要求鉴权头）。
        let res = client()
            .get(format!("{url}/agent/health?challenge=abc"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body["proof"].as_str().is_some());
    }

    #[tokio::test]
    async fn unknown_tool_returns_tool_not_found_code() {
        // 工具守卫对注册表外的名字返回 TOOL_NOT_FOUND（注册完整性兜底）。
        let dir = std::env::temp_dir().join(format!("miyo-agent-reg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = miyo_storage::db::init_db(&dir.join("t.db")).await.unwrap();
        let state = AgentState::new(db.clone(), "tok");
        let err = ensure_tool_allowed(&state, "not_a_real_tool").await;
        let (status, body) = err.unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.code, "TOOL_NOT_FOUND");
        db.close().await;
    }
}
