//! MCP 协议处理：JSON-RPC 分发 + 工具定义 + 工具调用 + Resources + Prompts。
//!
//! 实现的 server 方法集（协议版本 2025-06-18，回显客户端请求值保证兼容）：
//! - `initialize` / `ping` / `tools/list` / `tools/call`
//! - `resources/list` / `resources/read`（项目 / 接口 / 环境 / 场景 / 运行历史）
//! - `prompts/list` / `prompts/get`（接口测试 / 场景编排 / 失败分析模板）
//! - 通知：`notifications/tools/list_changed`（工具启停轮询）、
//!   `notifications/progress`（长任务进度，客户端 progressToken 触发）
//!
//! 工具定义单源在 `miyo_agent::registry`（元数据 + Schema + 权限 + 注解）；
//! 本文件只做协议分发，不再持有工具清单。

use miyo_agent::ControlClient;
use miyo_core::AppError;
use serde_json::{json, Value};
use uuid::Uuid;

/// 默认协议版本（客户端未携带时使用）。
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

// ---------- 入口 ----------

/// 进程级缓存的控制面客户端：MCP 生命周期内复用连接池与端口发现结果，
/// 免去每条 JSON-RPC 重建 reqwest Client / 重扫 4110~4129 端口段的开销。
/// 健康检查失败（桌面端重启 / 换端口）时丢弃并重新发现。
static CLIENT_CACHE: tokio::sync::Mutex<Option<ControlClient>> =
    tokio::sync::Mutex::const_new(None);

async fn cached_client() -> Result<ControlClient, AppError> {
    let mut cached = CLIENT_CACHE.lock().await;
    if let Some(client) = cached.as_ref() {
        if client.health().await.is_ok() {
            return Ok(client.clone());
        }
        eprintln!("miyo-mcp: 缓存的控制面连接失效，重新发现");
    }
    let client = ControlClient::discover_default().await?;
    *cached = Some(client.clone());
    Ok(client)
}

/// 处理一行请求：复用进程级缓存的控制面连接（失效时自动重新发现）。
/// 返回全部输出行（响应 + 通知）。
pub async fn handle_line_discover(line: &str) -> Vec<String> {
    handle_line_with(line, || Box::pin(cached_client())).await
}

/// 处理一行请求：返回**全部输出行**（响应 + 处理期间产生的通知，如
/// notifications/progress；顺序：通知在前、响应在最后）。
pub(crate) async fn handle_line_with(
    line: &str,
    connect: impl FnOnce() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
    >,
) -> Vec<String> {
    // 期间收集的通知（progress 等）；响应始终是最后一行。
    let mut extra: Vec<String> = Vec::new();
    let response = handle_one(line, connect, &mut extra).await;
    if let Some(resp) = response {
        extra.push(resp);
    }
    extra
}

async fn handle_one(
    line: &str,
    connect: impl FnOnce() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
    >,
    extra: &mut Vec<String>,
) -> Option<String> {
    let msg: Value = serde_json::from_str(line).ok()?;
    let id = msg.get("id").cloned();
    let method = msg.get("method")?.as_str()?.to_string();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    let payload: Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => {
            // 停用的工具对 AI 完全不可见；控制面不可达时全量列出（调用会在连接层失败）。
            let defs = match connect().await {
                Err(_) => tool_definitions(),
                Ok(client) => {
                    let states = client.list_tool_states().await.unwrap_or_default();
                    tool_definitions()
                        .into_iter()
                        .filter(|d| {
                            states
                                .iter()
                                .find(|s| Some(s.name.as_str()) == d["name"].as_str())
                                .map(|s| s.enabled)
                                .unwrap_or(true)
                        })
                        .collect::<Vec<_>>()
                }
            };
            Ok(json!({ "tools": defs }))
        }
        "resources/list" => match connect().await {
            Err(e) => Err(tool_error(&e.user_message())),
            Ok(_) => Ok(resources_list()),
        },
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap_or_default().to_string();
            if uri.is_empty() {
                Err((-32602, "缺少必填参数 uri".to_string()))
            } else {
                match connect().await {
                    Err(e) => Err(tool_error(&e.user_message())),
                    Ok(client) => match resource_read(&client, &uri).await {
                        Ok(content) => Ok(json!({ "contents": [content] })),
                        Err(f) => Err(jsonrpc_error(&f.code, &f.message)),
                    },
                }
            }
        }
        "prompts/list" => Ok(prompts_list()),
        "prompts/get" => match prompts_get(&params) {
            Ok(p) => Ok(p),
            Err(message) => Err((-32602, message)),
        },
        // 先校验工具与参数，全部通过才建立控制面连接——缺参 / 未知工具
        // 这类注定失败的调用不该付一次连接往返（也不该把连接错误盖过
        // 更贴近调用方的 -32602 参数错误）。
        "tools/call" => match validate_tool_call(&params) {
            Err(e) => Err(e),
            // 执行失败按 MCP 约定返回 isError: true 的正常结果（信封在 content 里，
            // AI 可解析 {success:false, error:{code,message,details}}）。
            Ok(()) => match connect().await {
                Err(e) => Err(tool_error(&e.user_message())),
                Ok(client) => match call_tool(&client, &params, extra).await {
                    Ok(result) => Ok(result),
                    Err(f) => Ok(tool_result(
                        fail_envelope(&f.code, &f.message, f.details),
                        true,
                    )),
                },
            },
        },
        // 未知 notification：静默忽略；未知 request：协议错误
        _ if id.is_none() => return None,
        _ => Err((-32601, format!("Method not found: {method}"))),
    };

    let mut response = json!({ "jsonrpc": "2.0", "id": id });
    match payload {
        Ok(result) => response["result"] = result,
        Err((code, message)) => {
            response["error"] = json!({ "code": code, "message": message });
        }
    }
    Some(response.to_string())
}

fn initialize_result(params: &Value) -> Value {
    json!({
        "protocolVersion": params["protocolVersion"].as_str().unwrap_or(DEFAULT_PROTOCOL_VERSION),
        // 协议能力（2025-06-18 核心集）：工具列表变更通知 + Resources + Prompts。
        // 进度通知无需声明能力（客户端以 _meta.progressToken 逐请求触发）。
        "capabilities": {
            "tools": { "listChanged": true },
            "resources": {},
            "prompts": {}
        },
        "serverInfo": { "name": "miyo-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// 连接级失败（控制面不可达等）走 JSON-RPC 协议错误。
fn tool_error(message: &str) -> (i64, String) {
    (-32000, message.to_string())
}

/// 统一返回信封：成功 `{success:true, data, error:null}`；
/// 失败 `{success:false, data:null, error:{code,message,details}}`。
fn ok_envelope(data: Value) -> Value {
    json!({ "success": true, "data": data, "error": null })
}

fn fail_envelope(code: &str, message: &str, details: Value) -> Value {
    json!({ "success": false, "data": null, "error": { "code": code, "message": message, "details": details } })
}

/// 工具业务失败（区别于 -32602 参数错误）：带上下文错误码。
#[derive(Debug)]
struct ToolFailure {
    code: String,
    message: String,
    details: Value,
}

impl ToolFailure {
    fn new(code: &str, message: impl Into<String>) -> Self {
        ToolFailure {
            code: code.to_string(),
            message: message.into(),
            details: Value::Null,
        }
    }
}

impl From<ToolFailure> for (i64, String) {
    fn from(f: ToolFailure) -> Self {
        (-32602, f.message)
    }
}

impl From<(i64, String)> for ToolFailure {
    fn from(v: (i64, String)) -> Self {
        ToolFailure {
            code: "INVALID_ARGUMENT".into(),
            message: v.1,
            details: Value::Null,
        }
    }
}

/// AppError → 上下文错误码：`ctx` 为实体前缀（PROJECT / ENDPOINT /
/// ENVIRONMENT / SCENARIO / RUN），NotFound → `{ctx}_NOT_FOUND`，
/// Validation → `{ctx}_VALIDATION_FAILED`，Cancelled → RUN_CANCELLED。
fn tool_fail(ctx: &str, e: &AppError) -> ToolFailure {
    // 实体细化：NotFound 的消息指向接口/环境时优先用更精确的实体码
    //（场景操作里引用了不存在的接口 → ENDPOINT_NOT_FOUND 而非 SCENARIO_NOT_FOUND）。
    let refine = |code: String, msg: &str| -> String {
        if code.ends_with("_NOT_FOUND") {
            if msg.contains("接口") {
                return "ENDPOINT_NOT_FOUND".into();
            }
            if msg.contains("环境") {
                return "ENVIRONMENT_NOT_FOUND".into();
            }
            if msg.contains("运行") || msg.contains("run") {
                return "RUN_NOT_FOUND".into();
            }
        }
        code
    };
    let code = match e {
        AppError::NotFound(_) => refine(format!("{ctx}_NOT_FOUND"), &e.user_message()),
        AppError::Validation(_) => refine(format!("{ctx}_VALIDATION_FAILED"), &e.user_message()),
        AppError::Cancelled(_) => "RUN_CANCELLED".to_string(),
        // 工具停用 / 鉴权失败：统一权限语义（HTTP 层 TOOL_DISABLED 经
        // client decode 映射为 Forbidden；此前退化为 INTERNAL）。
        AppError::Forbidden(_) => "PERMISSION_DENIED".to_string(),
        _ => "INTERNAL".to_string(),
    };
    ToolFailure {
        code,
        message: e.user_message(),
        details: Value::Null,
    }
}

// ---------- 工具定义 ----------

/// tools/list 的完整定义。单源：`miyo_agent::registry`（元数据 + 描述 +
/// Schema + 注解），本函数只做透传——新增工具无需改这里。
fn tool_definitions() -> Vec<Value> {
    miyo_agent::registry::tool_definitions_json()
}

/// 标准 JSON-RPC 错误（协议层失败：工具停用 / 权限拒绝在 content 信封里，
/// 这里承载资源不存在等请求级失败）。
fn jsonrpc_error(code: &str, message: &str) -> (i64, String) {
    let rpc_code = match code {
        "NOT_FOUND" | "TOOL_NOT_FOUND" => -32602,
        "PERMISSION_DENIED" | "TOOL_DISABLED" => -32000,
        _ => -32000,
    };
    (rpc_code, format!("[{code}] {message}"))
}

// ---------- Resources（项目 / 接口 / 环境 / 场景 / 运行历史） ----------

/// 资源清单：具体资源（projects / context）+ URI 模板（项目子资源 / 环境 /
/// 运行）。读取走既有控制面端点——权限（read）与数据脱敏由服务端统一负责。
fn resources_list() -> Value {
    json!({
        "resources": [
            {
                "uri": "miyo://context",
                "name": "Agent 上下文快照",
                "description": "当前项目 / 当前环境 / 接口与环境清单摘要 / 场景清单 / 最近一次运行（变量与敏感值已脱敏；不含请求规格明细）",
                "mimeType": "application/json"
            },
            {
                "uri": "miyo://projects",
                "name": "全部项目",
                "description": "Miyo 中全部项目（id / 名称 / 描述 / 变量，敏感变量已脱敏）",
                "mimeType": "application/json"
            }
        ],
        "resourceTemplates": [
            {
                "uriTemplate": "miyo://projects/{projectId}/endpoints",
                "name": "项目接口清单",
                "description": "指定项目下的接口（方法 / 路径 / 名称 / 请求规格，认证与敏感头已脱敏）",
                "mimeType": "application/json"
            },
            {
                "uriTemplate": "miyo://projects/{projectId}/environments",
                "name": "项目环境清单",
                "description": "指定项目的执行环境（名称 / Base URL / 启用变量数）",
                "mimeType": "application/json"
            },
            {
                "uriTemplate": "miyo://projects/{projectId}/scenarios",
                "name": "项目场景清单",
                "description": "指定项目的集成测试场景摘要（步骤数 / 最近运行）",
                "mimeType": "application/json"
            },
            {
                "uriTemplate": "miyo://environments/{environmentId}",
                "name": "环境详情",
                "description": "环境 Base URL 与启用变量（敏感值已脱敏为占位符）",
                "mimeType": "application/json"
            },
            {
                "uriTemplate": "miyo://runs/{runId}",
                "name": "场景运行结果",
                "description": "一次场景运行的结果摘要与分步状态（响应正文不含全文）",
                "mimeType": "application/json"
            }
        ]
    })
}

/// 资源读取：URI → 控制面端点。未知 URI → NOT_FOUND（资源不在服务端
/// 白名单内一律拒绝，不做自由路径拼接）。
async fn resource_read(client: &ControlClient, uri: &str) -> Result<Value, ToolFailure> {
    let payload = match uri.strip_prefix("miyo://") {
        Some("context") => client
            .get_context()
            .await
            .map_err(|e| tool_fail("CONTEXT", &e))?,
        Some("projects") => serde_json::to_value(
            client
                .list_projects()
                .await
                .map_err(|e| tool_fail("PROJECT", &e))?,
        )
        .unwrap_or(Value::Null),
        Some(rest) if rest.starts_with("projects/") => {
            let rest = &rest["projects/".len()..];
            let (project_id, sub) = rest
                .split_once('/')
                .ok_or_else(|| ToolFailure::new("NOT_FOUND", format!("资源 URI 不完整：{uri}")))?;
            let project_id: Uuid = project_id.parse().map_err(|_| {
                ToolFailure::new("VALIDATION_FAILED", "projectId 不是合法 UUID".to_string())
            })?;
            match sub {
                "endpoints" => serde_json::to_value(
                    client
                        .list_endpoints(project_id)
                        .await
                        .map_err(|e| tool_fail("ENDPOINT", &e))?,
                )
                .unwrap_or(Value::Null),
                "environments" => serde_json::to_value(
                    client
                        .list_environments(Some(project_id))
                        .await
                        .map_err(|e| tool_fail("ENVIRONMENT", &e))?,
                )
                .unwrap_or(Value::Null),
                "scenarios" => serde_json::to_value(
                    client
                        .list_scenarios(Some(project_id))
                        .await
                        .map_err(|e| tool_fail("SCENARIO", &e))?,
                )
                .unwrap_or(Value::Null),
                _ => {
                    return Err(ToolFailure::new(
                        "NOT_FOUND",
                        format!("未知资源子路径：{sub}"),
                    ))
                }
            }
        }
        Some(rest) if rest.starts_with("environments/") => {
            let env_id: Uuid = rest["environments/".len()..].parse().map_err(|_| {
                ToolFailure::new(
                    "VALIDATION_FAILED",
                    "environmentId 不是合法 UUID".to_string(),
                )
            })?;
            serde_json::to_value(
                client
                    .get_environment(env_id)
                    .await
                    .map_err(|e| tool_fail("ENVIRONMENT", &e))?,
            )
            .unwrap_or(Value::Null)
        }
        Some(rest) if rest.starts_with("runs/") => {
            let run_id: Uuid = rest["runs/".len()..].parse().map_err(|_| {
                ToolFailure::new("VALIDATION_FAILED", "runId 不是合法 UUID".to_string())
            })?;
            client
                .get_run_result(run_id)
                .await
                .map_err(|e| tool_fail("RUN", &e))?
        }
        _ => {
            return Err(ToolFailure::new(
                "NOT_FOUND",
                format!("未知资源：{uri}（可用清单见 resources/list）"),
            ))
        }
    };
    Ok(json!({
        "uri": uri,
        "mimeType": "application/json",
        "text": serde_json::to_string_pretty(&payload).unwrap_or_default(),
    }))
}

// ---------- Prompts（标准化测试 / 编排 / 分析模板） ----------

/// Prompt 模板清单（纯文本生成，不读项目数据——无数据泄露面）。
fn prompts_list() -> Value {
    json!({
        "prompts": [
            {
                "name": "design-api-tests",
                "description": "为指定接口设计测试用例（正向 / 负向 / 边界值 / 安全性），并可直接创建为 Miyo 测试用例",
                "arguments": [
                    { "name": "endpointId", "description": "接口 UUID（list_endpoints 获取）", "required": true }
                ]
            },
            {
                "name": "author-scenario",
                "description": "编排一个集成测试场景（接口链路 + 变量提取 + 断言），创建后先校验再运行",
                "arguments": [
                    { "name": "goal", "description": "业务目标描述（如「下单后用订单号支付」）", "required": true },
                    { "name": "projectId", "description": "可选，项目 UUID（缺省用当前唯一 / 激活项目）", "required": false }
                ]
            },
            {
                "name": "analyze-failure",
                "description": "分析一次场景运行的失败原因，给出可执行的修复建议",
                "arguments": [
                    { "name": "runId", "description": "运行 UUID（run_scenario / get_run_result 返回）", "required": true }
                ]
            }
        ]
    })
}

/// prompts/get：模板展开为 user 消息。参数缺失 / 未知 prompt → -32602。
fn prompts_get(params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().unwrap_or_default();
    let arg = |k: &str| {
        params["arguments"][k]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let text = match name {
        "design-api-tests" => {
            let endpoint_id = arg("endpointId");
            if endpoint_id.is_empty() {
                return Err("缺少必填参数 endpointId".to_string());
            }
            format!(
                "请为 Miyo 中的接口（endpointId={endpoint_id}）设计测试用例。步骤：\n                 1. 用 get_endpoint 读取接口定义（方法 / 路径 / 参数 / Body 形态）；\n                 2. 用 list_test_cases 查看已有用例，避免重复；\n                 3. 设计覆盖：正向（合法输入 → 预期 2xx 与关键字段断言）、负向（缺参 / 非法值 → 预期 4xx）、边界值（空串 / 超长 / 极值）、安全性（未授权 → 预期 401/403）；\n                 4. 每条用例给出：name、category（正向/负向/边界值/安全性/其他）、request 快照与断言；\n                 5. 与我确认后用 create_test_case 逐条创建（创建是落库操作，未经确认不要执行）。\n                 不要编造接口没有的字段；信息不足先提问。"
            )
        }
        "author-scenario" => {
            let goal = arg("goal");
            if goal.is_empty() {
                return Err("缺少必填参数 goal".to_string());
            }
            let project_id = arg("projectId");
            let project_hint = if project_id.is_empty() {
                "用 list_projects / get_context 确认目标项目".to_string()
            } else {
                format!("目标项目 projectId={project_id}")
            };
            format!(
                "请把以下业务目标编排为 Miyo 集成测试场景：{goal}\n\n                 步骤：\n                 1. {project_hint}；\n                 2. 用 list_endpoints 列出可用接口，选出链路所需的接口并逐个 get_endpoint 了解参数与响应形态；\n                 3. 用 create_scenario 一次创建完整 steps：每步含内联请求或接口引用、变量提取（extractors）、断言（source/operator 形状）；前一步提取的值在后续步骤用 {{{{变量名}}}} 引用；\n                 4. 用 validate_scenario 校验（变量引用 / 断言 / 写操作清单），不通过则修复；\n                 5. 运行前向我确认目标环境（生产环境必须显式确认）；写操作场景建议开 autoIdempotency。"
            )
        }
        "analyze-failure" => {
            let run_id = arg("runId");
            if run_id.is_empty() {
                return Err("缺少必填参数 runId".to_string());
            }
            format!(
                "请分析 Miyo 场景运行（runId={run_id}）的失败原因。步骤：\n                 1. 用 get_run_result 获取整体结果（哪几步失败 / cancelled / blocked）；\n                 2. 用 analyze_run_failure 得到规则化归因（错误类型 / 可能原因 / 建议）；\n                 3. 对关键失败步用 get_step_result 核对原始请求与响应（断言 expected vs actual）；\n                 4. 输出：结论（根因推断）+ 证据（具体步骤与字段）+ 可执行修复建议（区分「场景配置问题」与「被测服务问题」）。\n                 analyze_run_failure 的输出是启发式推测，必须用原始响应验证后再下结论。"
            )
        }
        other => return Err(format!("未知 prompt：{other}")),
    };
    Ok(json!({
        "description": "Miyo 标准化工作流模板",
        "messages": [
            { "role": "user", "content": { "type": "text", "text": text } }
        ]
    }))
}

// ---------- 工具调用 ----------

/// Unix 秒 → RFC3339（miyo-mcp 不依赖 chrono，简化实现：足够用于 created_at）。
fn time_rfc3339(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 天数 → (y, m, d)（民用历反推，1 = 1970-01-01）。
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{sec:02}Z")
}

/// create_test_case：args → TestCase JSON（缺省补齐：id / created_at / 空数组）。
fn build_test_case_json(endpoint_id: uuid::Uuid, args: &serde_json::Value) -> serde_json::Value {
    let req = &args["request"];
    let kv = |field: &str| req.get(field).cloned().unwrap_or(serde_json::json!([]));
    serde_json::json!({
        "id": uuid::Uuid::new_v4(),
        "request_id": endpoint_id,
        "name": args["name"].as_str().unwrap_or("").trim(),
        "category": args.get("category").and_then(|v| v.as_str()).unwrap_or("正向"),
        "folder": args.get("folder").and_then(|v| v.as_str()).unwrap_or(""),
        "method": req["method"].as_str().unwrap_or("GET"),
        "url_path": req["urlPath"].as_str().unwrap_or("/"),
        "path_variables": kv("pathVariables"),
        "params": kv("params"),
        "headers": kv("headers"),
        "body_type": req.get("bodyType").and_then(|v| v.as_str()).unwrap_or("none"),
        "body_content": req.get("bodyContent").cloned().unwrap_or(serde_json::json!("")),
        "assertions": args.get("assertions").cloned().unwrap_or(serde_json::json!([])),
        "extractors": args.get("extractors").cloned().unwrap_or(serde_json::json!([])),
        "auth": req.get("auth").cloned().unwrap_or(serde_json::Value::Null),
        "timeout_ms": req.get("timeoutMs").cloned().unwrap_or(serde_json::Value::Null),
        "disable_cookies": req.get("disableCookies").and_then(|v| v.as_bool()).unwrap_or(false),
        "last_run_status": "Untested",
        "created_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| time_rfc3339(d.as_secs()))
            .unwrap_or_default(),
    })
}

/// update_test_case：读现状 JSON → 覆盖调用方提供的顶层字段（合并语义）。
fn merge_test_case_json(
    mut existing: serde_json::Value,
    args: &serde_json::Value,
) -> serde_json::Value {
    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        existing["name"] = serde_json::json!(name.trim());
    }
    if let Some(v) = args.get("category").and_then(|v| v.as_str()) {
        existing["category"] = serde_json::json!(v);
    }
    if let Some(v) = args.get("folder").and_then(|v| v.as_str()) {
        existing["folder"] = serde_json::json!(v);
    }
    if let Some(req) = args.get("request").filter(|v| v.is_object()) {
        for (k, v) in req.as_object().unwrap() {
            let key = match k.as_str() {
                "method" => "method",
                "urlPath" => "url_path",
                "pathVariables" => "path_variables",
                "bodyType" => "body_type",
                "bodyContent" => "body_content",
                "timeoutMs" => "timeout_ms",
                "disableCookies" => "disable_cookies",
                other => other,
            };
            existing[key] = v.clone();
        }
    }
    if let Some(v) = args.get("assertions") {
        existing["assertions"] = v.clone();
    }
    if let Some(v) = args.get("extractors") {
        existing["extractors"] = v.clone();
    }
    existing
}

/// 连接控制面前的参数校验：工具名与必填参数（含 UUID 格式）先过一遍，
/// 返回与 [`call_tool`] 完全一致的 -32602 错误；通过后才建连接。
/// `call_tool` 内部会再取一次值（校验是幂等的，值本就要在那里消费）。
fn validate_tool_call(params: &Value) -> Result<(), (i64, String)> {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "save_curl" => {
            str_arg(&args, "command")
                .ok_or_else(|| (-32602, "缺少必填参数 command".to_string()))?;
            opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            opt_uuid_arg(&args, "folderId").map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            Ok(())
        }
        "list_projects" | "agent_info" | "get_context" => Ok(()),
        "list_endpoints" => {
            str_arg(&args, "projectId")
                .ok_or_else(|| (-32602, "缺少必填参数 projectId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "projectId 不是合法 UUID".to_string())
                })?;
            Ok(())
        }
        "get_endpoint" => {
            str_arg(&args, "endpointId")
                .ok_or_else(|| (-32602, "缺少必填参数 endpointId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "endpointId 不是合法 UUID".to_string())
                })?;
            Ok(())
        }
        "send_endpoint" => {
            str_arg(&args, "endpointId")
                .ok_or_else(|| (-32602, "缺少必填参数 endpointId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "endpointId 不是合法 UUID".to_string())
                })?;
            str_map_arg(&args, "headers").map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            str_map_arg(&args, "query").map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            str_map_arg(&args, "variables").map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            if let Some(t) = args.get("timeoutMs") {
                if t.as_f64().map(|f| f <= 0.0).unwrap_or(true) {
                    return Err(ToolFailure::new(
                        "INVALID_ARGUMENT",
                        "timeoutMs 必须是正数".to_string(),
                    )
                    .into());
                }
            }
            Ok(())
        }
        "run_scenario" => {
            str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            str_map_arg(&args, "variables").map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            Ok(())
        }
        "list_scenarios" => {
            opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            Ok(())
        }
        "create_scenario" => {
            match str_arg(&args, "name") {
                Some(n) if !n.trim().is_empty() => {}
                _ => {
                    return Err(ToolFailure::new(
                        "INVALID_ARGUMENT",
                        "缺少必填参数 name（不可为空）",
                    )
                    .into())
                }
            }
            opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            uuid_list_arg(&args, "endpointIds")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            Ok(())
        }
        "get_scenario" => {
            str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            Ok(())
        }
        "save_scenario_script" => {
            str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            if args.get("script").and_then(|v| v.as_str()).is_none() {
                return Err((
                    -32602,
                    "缺少必填参数 script（传空字符串表示清除自定义）".to_string(),
                ));
            }
            Ok(())
        }
        "list_environments" => {
            opt_uuid_arg(&args, "projectId").map_err(|e| (-32602, e))?;
            Ok(())
        }
        "get_environment" => {
            uuid_req(&args, "environmentId")?;
            Ok(())
        }
        "update_scenario" | "delete_scenario" | "validate_scenario" => {
            uuid_req(&args, "scenarioId")?;
            if name == "validate_scenario" {
                opt_uuid_arg(&args, "environmentId").map_err(|e| (-32602, e))?;
                if let Some(v) = args.get("runtimeVariables") {
                    if !v.is_null()
                        && !v
                            .as_array()
                            .map(|a| a.iter().all(|x| x.is_string()))
                            .unwrap_or(false)
                    {
                        return Err((-32602, "runtimeVariables 必须是字符串数组".to_string()));
                    }
                }
            }
            Ok(())
        }
        "add_scenario_step" => {
            uuid_req(&args, "scenarioId")?;
            if args.get("step").map(|v| v.is_object()).unwrap_or(false) {
                Ok(())
            } else {
                Err((-32602, "缺少必填参数 step（对象）".to_string()))
            }
        }
        "update_scenario_step" | "delete_scenario_step" => {
            uuid_req(&args, "scenarioId")?;
            index_arg(&args, "index")?;
            Ok(())
        }
        "get_run_result" | "analyze_run_failure" | "get_test_case_run" | "cancel_run" => {
            uuid_req(&args, "runId")?;
            Ok(())
        }
        "list_test_cases" => {
            uuid_req(&args, "endpointId")?;
            Ok(())
        }
        "get_test_case" | "delete_test_case" | "run_test_case" | "list_test_case_runs" => {
            uuid_req(&args, "caseId")?;
            if name == "run_test_case" {
                opt_uuid_arg(&args, "environmentId").map_err(|e| (-32602, e))?;
            }
            Ok(())
        }
        "create_test_case" => {
            uuid_req(&args, "endpointId")?;
            let req_name = args
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if req_name.is_empty() {
                return Err((-32602, "缺少必填参数 name（非空字符串）".to_string()));
            }
            let req = args.get("request").filter(|v| v.is_object());
            if req.is_none() {
                return Err((
                    -32602,
                    "缺少必填参数 request（对象：method/urlPath/…）".to_string(),
                ));
            }
            Ok(())
        }
        "update_test_case" => {
            uuid_req(&args, "caseId")?;
            Ok(())
        }
        "get_step_result" => {
            uuid_req(&args, "runId")?;
            index_arg(&args, "stepIndex")?;
            Ok(())
        }
        other => Err((-32602, format!("未知工具：{other}"))),
    }
}

async fn call_tool(
    client: &ControlClient,
    params: &Value,
    extra: &mut Vec<String>,
) -> Result<Value, ToolFailure> {
    let name = params["name"].as_str().unwrap_or_default();
    // 注册表兜底：未注册工具在协议层即拒绝（与 tools/list 单源一致）。
    let Some(meta) = miyo_agent::registry::tool_meta(name) else {
        return Err(ToolFailure::new(
            "TOOL_NOT_FOUND",
            format!("未注册的 MCP 工具：{name}"),
        ));
    };
    // 服务端兜底之外的第一道拦截：停用的工具调用即报错（响应 isError）。
    if let Ok(states) = client.list_tool_states().await {
        if let Some(s) = states.iter().find(|s| s.name == name) {
            if !s.enabled {
                return Err(ToolFailure::new(
                    "PERMISSION_DENIED",
                    format!("工具 {name} 已在 Miyo 设置 → MCP 服务中停用"),
                ));
            }
        }
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    // 并发控制：注册表 concurrent=false 的工具按名串行（防 read-modify-write
    // 交错覆盖，如 save_scenario_script / update_* 系列）。
    let _serial = if meta.concurrent {
        None
    } else {
        Some(serial_gate(name).await)
    };
    let value: Value = match name {
        "save_curl" => {
            let command = str_arg(&args, "command").ok_or_else(|| {
                ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 command".to_string())
            })?;
            let project_id = opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            let folder_id = opt_uuid_arg(&args, "folderId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            serde_json::to_value(
                client
                    .save_curl(command, str_arg(&args, "name"), project_id, folder_id)
                    .await
                    .map_err(|e| tool_fail("ENDPOINT", &e))?,
            )
            .expect("序列化失败")
        }
        "list_projects" => serde_json::to_value(
            client
                .list_projects()
                .await
                .map_err(|e| tool_fail("PROJECT", &e))?,
        )
        .expect("序列化失败"),
        "get_endpoint" => {
            let endpoint_id: Uuid = str_arg(&args, "endpointId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 endpointId".to_string())
                })?
                .parse()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "endpointId 不是合法 UUID".to_string())
                })?;
            serde_json::to_value(
                client
                    .get_endpoint(endpoint_id)
                    .await
                    .map_err(|e| tool_fail("ENDPOINT", &e))?,
            )
            .expect("序列化失败")
        }
        "send_endpoint" => {
            let endpoint_id: Uuid = str_arg(&args, "endpointId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 endpointId".to_string())
                })?
                .parse()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "endpointId 不是合法 UUID".to_string())
                })?;
            let mut payload = serde_json::json!({});
            for key in ["url", "headers", "query", "body", "variables"] {
                if let Some(v) = args.get(key) {
                    if !v.is_null() {
                        payload[key] = v.clone();
                    }
                }
            }
            if let Some(id) = opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?
            {
                payload["environmentId"] = serde_json::json!(id.to_string());
            }
            if let Some(t) = args.get("timeoutMs").and_then(|v| v.as_f64()) {
                payload["timeoutMs"] = serde_json::json!(t as u64);
            }
            serde_json::to_value(
                client
                    .execute_endpoint(endpoint_id, payload)
                    .await
                    .map_err(|e| tool_fail("ENDPOINT", &e))?,
            )
            .expect("序列化失败")
        }
        "run_scenario" => {
            let scenario_id: Uuid = str_arg(&args, "scenarioId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 scenarioId".to_string())
                })?
                .parse()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            let mut payload = serde_json::json!({});
            if let Some(v) = args.get("stopOnFailure") {
                if !v.is_null() {
                    payload["stopOnFailure"] = v.clone();
                }
            }
            if let Some(v) = args.get("variables") {
                if !v.is_null() {
                    payload["variables"] = v.clone();
                }
            }
            if let Some(id) = opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?
            {
                payload["environmentId"] = serde_json::json!(id.to_string());
            }
            if let Some(v) = args.get("autoIdempotency").and_then(|v| v.as_bool()) {
                payload["autoIdempotency"] = serde_json::json!(v);
            }
            // 数据驱动：多组数据独立执行（每组变量隔离、独立落历史、
            // 响应为逐行汇总——raw 形状，见下方 dataSets 分支）。
            if let Some(ds) = args.get("dataSets").filter(|v| v.is_array()) {
                payload["dataSets"] = ds.clone();
            }
            let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);
            payload["wait"] = serde_json::json!(wait);
            // 进度通知：客户端带 _meta.progressToken 时，运行期间轮询
            // /agent/runs/{id}/progress 并转发 notifications/progress。
            let progress_token = params["_meta"]["progressToken"].clone();
            let run_id_owned = uuid::Uuid::new_v4();
            let notify_progress = !progress_token.is_null();
            if notify_progress {
                payload["runId"] = serde_json::json!(run_id_owned.to_string());
            }
            if !wait {
                // 异步：立即返回 runId（JSON 形状与同步不同），原样透传。
                let raw = client
                    .run_scenario_raw(scenario_id, payload)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?;
                return Ok(tool_result(ok_envelope(raw), false));
            }
            // dataSets 数据驱动：响应为逐行汇总（非 RunScenarioResult 形状），
            // 走 raw 通道透传；进度轮询不适用（响应即全部行）。
            if payload
                .get("dataSets")
                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            {
                let raw = client
                    .run_scenario_raw(scenario_id, payload)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?;
                return Ok(tool_result(ok_envelope(raw), false));
            }
            // 同步等待：起进度轮询任务（客户端给了 progressToken 才有输出），
            // 轮询通知经 channel 收集，主调用返回后并入 extra 输出行。
            let (progress_task, progress_rx) = if notify_progress {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                (
                    Some(spawn_progress_poller(
                        client.clone(),
                        run_id_owned,
                        progress_token,
                        tx,
                    )),
                    Some(rx),
                )
            } else {
                (None, None)
            };
            let result = serde_json::to_value(
                client
                    .run_scenario(scenario_id, payload)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败");
            if let Some(task) = progress_task {
                task.abort();
            }
            if let Some(mut rx) = progress_rx {
                while let Ok(line) = rx.try_recv() {
                    extra.push(line);
                }
            }
            result
        }
        "list_scenarios" => {
            let project_id = opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            serde_json::to_value(
                client
                    .list_scenarios(project_id)
                    .await
                    .map_err(|e| tool_fail("PROJECT", &e))?,
            )
            .expect("序列化失败")
        }
        "create_scenario" => {
            let name = str_arg(&args, "name")
                .ok_or_else(|| {
                    ToolFailure::new(
                        "INVALID_ARGUMENT",
                        "缺少必填参数 name（不可为空）".to_string(),
                    )
                })?
                .to_string();
            let project_id = opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            let description = str_arg(&args, "description").map(str::to_string);
            let endpoint_ids = uuid_list_arg(&args, "endpointIds")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            // steps 完整定义优先（AI 方言由服务端归一化）。
            if let Some(steps) = args.get("steps").and_then(|v| v.as_array()) {
                let category = str_arg(&args, "category");
                serde_json::to_value(
                    client
                        .create_scenario_full(
                            &name,
                            project_id,
                            description.as_deref(),
                            category,
                            steps,
                        )
                        .await
                        .map_err(|e| tool_fail("SCENARIO", &e))?,
                )
                .expect("序列化失败")
            } else {
                serde_json::to_value(
                    client
                        .create_scenario(&name, project_id, description.as_deref(), &endpoint_ids)
                        .await
                        .map_err(|e| tool_fail("SCENARIO", &e))?,
                )
                .expect("序列化失败")
            }
        }
        "get_scenario" => {
            let scenario_id: Uuid = str_arg(&args, "scenarioId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 scenarioId".to_string())
                })?
                .parse()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            serde_json::to_value(
                client
                    .get_scenario(scenario_id)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "save_scenario_script" => {
            let scenario_id: Uuid = str_arg(&args, "scenarioId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 scenarioId".to_string())
                })?
                .parse()
                .map_err(|_| {
                    ToolFailure::new("INVALID_ARGUMENT", "scenarioId 不是合法 UUID".to_string())
                })?;
            let script = args.get("script").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(
                client
                    .save_scenario_script(
                        scenario_id,
                        if script.is_empty() {
                            None
                        } else {
                            Some(script)
                        },
                    )
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "list_endpoints" => {
            let project_id: Uuid = str_arg(&args, "projectId")
                .ok_or_else(|| {
                    ToolFailure::new("INVALID_ARGUMENT", "缺少必填参数 projectId".to_string())
                })?
                .parse()
                .map_err(|_| ToolFailure::new("INVALID_ARGUMENT", "projectId 不是合法 UUID"))?;
            // 筛选/分页 query 直传（服务端逐项校验；非法值 400 → VALIDATION_FAILED）。
            let mut query: Vec<String> = Vec::new();
            if let Some(n) = str_arg(&args, "name") {
                query.push(format!("name={}", urlencoding_encode(n)));
            }
            if let Some(m) = str_arg(&args, "method") {
                query.push(format!("method={}", urlencoding_encode(m)));
            }
            if let Some(id) = opt_uuid_arg(&args, "folderId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?
            {
                query.push(format!("folderId={id}"));
            }
            if let Some(v) = args.get("limit").and_then(|v| v.as_u64()) {
                query.push(format!("limit={v}"));
            }
            if let Some(v) = args.get("offset").and_then(|v| v.as_u64()) {
                query.push(format!("offset={v}"));
            }
            serde_json::to_value(
                client
                    .list_endpoints_filtered(project_id, &query.join("&"))
                    .await
                    .map_err(|e| tool_fail("ENDPOINT", &e))?,
            )
            .expect("序列化失败")
        }
        "list_environments" => {
            let project_id = opt_uuid_arg(&args, "projectId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            serde_json::to_value(
                client
                    .list_environments(project_id)
                    .await
                    .map_err(|e| tool_fail("ENVIRONMENT", &e))?,
            )
            .expect("序列化失败")
        }
        "get_environment" => {
            let env_id = uuid_req(&args, "environmentId")?;
            serde_json::to_value(
                client
                    .get_environment(env_id)
                    .await
                    .map_err(|e| tool_fail("ENVIRONMENT", &e))?,
            )
            .expect("序列化失败")
        }
        "update_scenario" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            let mut body = serde_json::json!({});
            for key in ["name", "description", "category", "steps"] {
                if let Some(v) = args.get(key) {
                    if !v.is_null() {
                        body[key] = v.clone();
                    }
                }
            }
            serde_json::to_value(
                client
                    .update_scenario(scenario_id, body)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "delete_scenario" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            serde_json::to_value(
                client
                    .delete_scenario(scenario_id)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "add_scenario_step" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            let step = args.get("step").cloned().unwrap_or(serde_json::Value::Null);
            let index = args
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            serde_json::to_value(
                client
                    .add_scenario_step(scenario_id, step, index)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "update_scenario_step" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            let index = args.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let step = args.get("step").cloned().unwrap_or(serde_json::Value::Null);
            serde_json::to_value(
                client
                    .update_scenario_step(scenario_id, index, step)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "delete_scenario_step" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            let index = args.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            serde_json::to_value(
                client
                    .delete_scenario_step(scenario_id, index)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "validate_scenario" => {
            let scenario_id = uuid_req(&args, "scenarioId")?;
            let env = opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            let runtime_vars: Vec<String> = args
                .get("runtimeVariables")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            serde_json::to_value(
                client
                    .validate_scenario(scenario_id, env, &runtime_vars)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
        }
        "list_test_cases" => {
            let endpoint_id = uuid_req(&args, "endpointId")?;
            serde_json::to_value(
                client
                    .list_test_cases(endpoint_id)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "get_test_case" => {
            let case_id = uuid_req(&args, "caseId")?;
            serde_json::to_value(
                client
                    .get_test_case(case_id)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "create_test_case" => {
            let endpoint_id = uuid_req(&args, "endpointId")?;
            let case = build_test_case_json(endpoint_id, &args);
            serde_json::to_value(
                client
                    .create_test_case(case)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "update_test_case" => {
            let case_id = uuid_req(&args, "caseId")?;
            // 更新 = 读现状 → 覆盖提供的字段（字段缺省保留原值的合并语义）。
            let existing = client
                .get_test_case(case_id)
                .await
                .map_err(|e| tool_fail("TESTCASE", &e))?;
            let case = merge_test_case_json(existing, &args);
            serde_json::to_value(
                client
                    .update_test_case(case_id, case)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "delete_test_case" => {
            let case_id = uuid_req(&args, "caseId")?;
            serde_json::to_value(
                client
                    .delete_test_case(case_id)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "run_test_case" => {
            let case_id = uuid_req(&args, "caseId")?;
            let env = opt_uuid_arg(&args, "environmentId")
                .map_err(|e| ToolFailure::new("INVALID_ARGUMENT", e))?;
            serde_json::to_value(
                client
                    .run_test_case(case_id, env)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "list_test_case_runs" => {
            let case_id = uuid_req(&args, "caseId")?;
            let limit = args.get("limit").and_then(|v| v.as_i64());
            serde_json::to_value(
                client
                    .list_test_case_runs(case_id, limit)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "get_test_case_run" => {
            let run_id = uuid_req(&args, "runId")?;
            serde_json::to_value(
                client
                    .get_test_case_run(run_id)
                    .await
                    .map_err(|e| tool_fail("TESTCASE", &e))?,
            )
            .expect("序列化失败")
        }
        "get_run_result" => {
            let run_id = uuid_req(&args, "runId")?;
            serde_json::to_value(
                client
                    .get_run_result(run_id)
                    .await
                    .map_err(|e| tool_fail("RUN", &e))?,
            )
            .expect("序列化失败")
        }
        "get_step_result" => {
            let run_id = uuid_req(&args, "runId")?;
            let index = args.get("stepIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            serde_json::to_value(
                client
                    .get_run_step(run_id, index)
                    .await
                    .map_err(|e| tool_fail("RUN", &e))?,
            )
            .expect("序列化失败")
        }
        "analyze_run_failure" => {
            let run_id = uuid_req(&args, "runId")?;
            serde_json::to_value(
                client
                    .analyze_run(run_id)
                    .await
                    .map_err(|e| tool_fail("RUN", &e))?,
            )
            .expect("序列化失败")
        }
        "cancel_run" => {
            let run_id = uuid_req(&args, "runId")?;
            serde_json::to_value(
                client
                    .cancel_run(run_id)
                    .await
                    .map_err(|e| tool_fail("RUN", &e))?,
            )
            .expect("序列化失败")
        }
        "get_context" => serde_json::to_value(
            client
                .get_context()
                .await
                .map_err(|e| tool_fail("CONTEXT", &e))?,
        )
        .expect("序列化失败"),
        "agent_info" => json!({
            "address": client.base(),
            "tokenPath": miyo_agent::token::token_path(&miyo_agent::default_data_dir()),
            "auth": "Authorization: Bearer <token 文件内容>",
            "docs": "POST /agent/curl | GET /agent/projects | GET /agent/endpoints/:projectId（支持 name/method/folderId/limit/offset 过滤）| GET /agent/endpoints/:id/detail | POST /agent/endpoints/:id/execute | GET /agent/environments?projectId= | GET /agent/environments/:id | GET /agent/scenarios?projectId= | POST /agent/scenarios（完整 steps）| GET /agent/scenarios/:id | PUT /agent/scenarios/:id | DELETE /agent/scenarios/:id | POST /agent/scenarios/:id/steps | PUT/DELETE /agent/scenarios/:id/steps/:index | GET /agent/scenarios/:id/validate | POST /agent/scenarios/:id/run（wait=false 异步）| GET /agent/runs/:id | POST /agent/runs/:id/cancel | GET /agent/runs/:id/steps/:index | GET /agent/runs/:id/analysis | PUT /agent/scenarios/:id/script"
        }),
        other => {
            return Err(ToolFailure::new(
                "TOOL_NOT_FOUND",
                format!("未注册的 MCP 工具：{other}"),
            ))
        }
    };
    Ok(tool_result(ok_envelope(value), false))
}

/// MCP 工具结果：信封 JSON 放 content（AI 解析用），isError 标记执行失败。
fn tool_result(envelope: Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&envelope).expect("格式化失败") }],
        "isError": is_error
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().filter(|s| !s.is_empty())
}

/// 可选的「字符串 → 字符串」对象参数（headers / query / variables）。
/// 提供时必须是对象且值全为字符串。
fn str_map_arg(args: &Value, key: &str) -> Result<Option<()>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) if v.is_object() => {
            if v.as_object()
                .unwrap()
                .iter()
                .all(|(_, val)| val.is_string())
            {
                Ok(Some(()))
            } else {
                Err(format!("{key} 的值必须全是字符串"))
            }
        }
        Some(_) => Err(format!("{key} 必须是对象（键值对，值为字符串）")),
    }
}

/// 必填 UUID 参数（返回解析后的 Uuid；MCP 层 -32602 通道）。
fn uuid_req(args: &Value, key: &str) -> Result<Uuid, (i64, String)> {
    str_arg(args, key)
        .ok_or_else(|| (-32602, format!("缺少必填参数 {key}")))?
        .parse::<Uuid>()
        .map_err(|_| (-32602, format!("{key} 不是合法 UUID")))
}

/// 必填非负整数参数（步骤位次）。
fn index_arg(args: &Value, key: &str) -> Result<(), (i64, String)> {
    match args.get(key) {
        Some(v) if v.is_u64() => Ok(()),
        _ => Err((-32602, format!("缺少必填参数 {key}（非负整数）"))),
    }
}

/// percent-encode（query 值；复用 url crate，与 miyo-agent 客户端同源）。
fn urlencoding_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn opt_uuid_arg(args: &Value, key: &str) -> Result<Option<Uuid>, String> {
    match args[key].as_str() {
        None | Some("") => Ok(None),
        Some(s) => s
            .parse::<Uuid>()
            .map(Some)
            .map_err(|_| format!("{key} 不是合法 UUID")),
    }
}

/// 可选的 UUID 数组参数（endpointIds）：缺省 / null → 空列表；
/// 提供时必须是数组且每项为合法 UUID 字符串。
fn uuid_list_arg(args: &Value, key: &str) -> Result<Vec<Uuid>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| format!("{key} 的元素必须是 UUID 字符串"))?
                    .parse::<Uuid>()
                    .map_err(|_| format!("{key} 含非法 UUID"))
            })
            .collect(),
        Some(_) => Err(format!("{key} 必须是 UUID 字符串数组")),
    }
}

// ---------- 并发控制 / 进度通知 ----------

/// 非并发工具（注册表 concurrent=false）的按名串行闸：同名工具调用互斥，
/// 防 read-modify-write 交错互相覆盖；不同工具互不影响。
static SERIAL_GATES: tokio::sync::Mutex<
    Option<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = tokio::sync::Mutex::const_new(None);

async fn serial_gate(name: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let gate = {
        let mut map = SERIAL_GATES.lock().await;
        map.get_or_insert_with(std::collections::HashMap::new)
            .entry(name.to_string())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    gate.lock_owned().await
}

/// 进度轮询任务：每 1.5s 查一次 /agent/runs/{id}/progress，变化即发
/// notifications/progress 行到 channel；主调用返回时 abort（上限约 15 分钟，
/// 防服务端异常时无限轮询）。
fn spawn_progress_poller(
    client: ControlClient,
    run_id: Uuid,
    token: Value,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last: Option<(u64, u64)> = None;
        for _ in 0..600 {
            if tx.is_closed() {
                return;
            }
            if let Ok(v) = client.get_run_progress(run_id).await {
                let done = v["done"].as_u64().unwrap_or(0);
                let total = v["total"].as_u64().unwrap_or(0);
                if Some((done, total)) != last {
                    last = Some((done, total));
                    let note = json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/progress",
                        "params": {
                            "progressToken": token,
                            "progress": done,
                            "total": total,
                            "message": format!("场景执行中：{done}/{total}"),
                        },
                    });
                    if tx.send(note.to_string()).is_err() {
                        return;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
    })
}

/// 工具启停变更轮询：每 15s 拉一次 /agent/tools，(name, enabled) 集合变化
/// 即输出 notifications/tools/list_changed（由 main 写 stdout）。
/// 通知只在首次 initialize 之后由 main 启动（协议要求）。
pub fn spawn_list_changed_poller(notify: impl Fn(String) + Send + 'static) {
    tokio::spawn(async move {
        let mut last: Option<String> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let client = match cached_client().await {
                Ok(c) => c,
                Err(_) => continue, // 控制面未就绪：下个周期重试
            };
            let Ok(states) = client.list_tool_states().await else {
                continue;
            };
            // 稳定序列化（按名排序）后比对。
            let mut snapshot: Vec<(String, bool)> =
                states.iter().map(|s| (s.name.clone(), s.enabled)).collect();
            snapshot.sort();
            let snapshot = serde_json::to_string(&snapshot).unwrap_or_default();
            if last.is_some() && last.as_deref() != Some(snapshot.as_str()) {
                notify(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/tools/list_changed",
                    })
                    .to_string(),
                );
            }
            last = Some(snapshot);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use miyo_agent::server::{build_router, AgentState};
    use miyo_storage::db::init_db;

    /// 防漂移：tools/list 的每个工具都必须在 MCP_TOOLS 注册表中，反之亦然。
    /// 注册表缺项 → 设置页看不到、无法停用（权限管理缺口）；定义缺项 →
    /// tools/list 漏工具。新增工具时两边同步加。
    #[test]
    fn tool_definitions_cover_registry() {
        let names: Vec<String> = tool_definitions()
            .into_iter()
            .map(|d| d["name"].as_str().expect("工具定义缺 name").to_string())
            .collect();
        let registry: Vec<String> = miyo_agent::server::MCP_TOOLS
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            names.len(),
            registry.len(),
            "tools/list（{}）与 MCP_TOOLS（{}）数量不一致",
            names.len(),
            registry.len()
        );
        for n in &names {
            assert!(
                registry.contains(n),
                "工具 {n} 有 tools/list 定义但未注册 MCP_TOOLS（设置页不可见、不可停用）"
            );
        }
        for n in &registry {
            assert!(
                names.contains(n),
                "工具 {n} 已注册 MCP_TOOLS 但 tools/list 缺定义"
            );
        }
    }

    /// 启动临时控制面并构造指向它的 connect 闭包。
    fn test_connect(
        url: String,
    ) -> impl Fn() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
    > {
        let url = std::sync::Arc::new(url);
        move || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new((*url).clone(), "tok")) })
        }
    }

    #[tokio::test]
    async fn initialize_and_list_tools() {
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            || unreachable!(),
        )
        .await
        .remove(0);
        assert!(reply.contains("\"protocolVersion\":\"2024-11-05\""));
        assert!(reply.contains("miyo-mcp"));

        // tools/list 会尝试连控制面取启停状态；不可达时回退全量列表（fail-open）。
        let reply = handle_line_with(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, || {
            Box::pin(async {
                Err(AppError::Mock("控制面不可达".into())) as Result<ControlClient, AppError>
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        })
        .await
        .remove(0);
        for tool in [
            "save_curl",
            "list_projects",
            "list_endpoints",
            "get_endpoint",
            "send_endpoint",
            "run_scenario",
            "list_scenarios",
            "create_scenario",
            "get_scenario",
            "save_scenario_script",
            "agent_info",
        ] {
            assert!(reply.contains(tool), "缺少工具 {tool}");
        }
    }

    #[tokio::test]
    async fn notification_gets_no_reply() {
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            || unreachable!(),
        )
        .await;
        assert!(reply.is_empty());
    }

    #[tokio::test]
    async fn unknown_request_is_method_not_found() {
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":9,"method":"sampling/createMessage"}"#,
            || unreachable!(),
        )
        .await
        .remove(0);
        assert!(reply.contains("-32601"));
    }

    #[tokio::test]
    async fn save_curl_roundtrip_via_mcp() {
        let dir = std::env::temp_dir().join(format!("miyo-mcp-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        let connect = test_connect(format!("http://{addr}"));

        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "save_curl",
                        "arguments": { "command": "curl https://api.example.com/users?full=1" } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("endpointId"), "{reply}");
        assert!(text.contains("/users"));
        assert_eq!(parsed["result"]["isError"], false);

        // 缺参数 → 参数错误（校验在连接之前，connect 闭包不会被调用）
        let bad = serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "save_curl", "arguments": {} }
        })
        .to_string();
        let reply = handle_line_with(&bad, || unreachable!("参数校验应先于连接失败"))
            .await
            .remove(0);
        assert!(reply.contains("-32602"), "{reply}");
    }

    #[tokio::test]
    async fn disabled_tool_hidden_from_list_and_rejected_on_call() {
        use miyo_storage::repository as repo;

        let dir = std::env::temp_dir().join(format!("miyo-mcp-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        let base_url = format!("http://{addr}");
        let connect = || {
            let url = base_url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        // 停用 save_curl
        repo::set_setting(
            &db,
            miyo_agent::server::MCP_DISABLED_TOOLS_KEY,
            &serde_json::json!(["save_curl"]).to_string(),
        )
        .await
        .unwrap();

        // tools/list 不再包含 save_curl
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":20,"method":"tools/list"}"#,
            connect,
        )
        .await
        .remove(0);
        // 按工具名字段断言（描述文案里会提到其他工具名，子串会误判）
        assert!(
            !reply.contains("\"name\":\"save_curl\""),
            "停用工具不应出现在 tools/list：{reply}"
        );
        assert!(reply.contains("list_projects"));

        // tools/call → isError 提示停用
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 21, "method": "tools/call",
            "params": { "name": "save_curl",
                        "arguments": { "command": "curl https://api.example.com/x" } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        // 停用工具：MCP isError 结果 + 信封 PERMISSION_DENIED（AI 可解析）。
        assert!(
            reply.contains("PERMISSION_DENIED")
                && reply.contains("停用")
                && reply.contains("isError\":true"),
            "{reply}"
        );

        // 重新启用 → 回到 tools/list
        repo::set_setting(
            &db,
            miyo_agent::server::MCP_DISABLED_TOOLS_KEY,
            &serde_json::json!([]).to_string(),
        )
        .await
        .unwrap();
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":22,"method":"tools/list"}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(reply.contains("save_curl"), "{reply}");
    }

    /// dataSets 数据驱动端到端：MCP tools/call（stdio 层）→ 控制面 → 引擎。
    /// 两组数据独立执行 / 逐行汇总 / 行间提取零污染 / 每组独立落历史。
    #[tokio::test]
    async fn run_scenario_data_sets_via_mcp() {
        use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, RequestSpec, Scenario};
        use miyo_storage::repository as repo;

        // 本地回显上游：/users/{id} → {"userId": id}
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/users/{id}",
                axum::routing::get(
                    |axum::extract::Path(id): axum::extract::Path<String>| async move {
                        axum::Json(serde_json::json!({ "userId": id }))
                    },
                ),
            );
            let _ = axum::serve(listener, app).await;
        });

        let dir = std::env::temp_dir().join(format!("miyo-mcp-ds-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        let project = repo::create_project(&db, "数据驱动", "").await.unwrap();
        let mut with_base = repo::get_project(&db, project.id).await.unwrap();
        with_base
            .variables
            .insert("base_url".into(), format!("http://{upstream}"));
        repo::save_project(&db, &with_base).await.unwrap();

        let users = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: "用户".into(),
            method: HttpMethod::GET,
            path: "/users/{{uid}}".into(),
            description: String::new(),
            status: EndpointStatus::Developing,
            sort_order: 0,
            request: RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_endpoint(&db, &users).await.unwrap();
        let scenario = Scenario {
            setup: Vec::new(),
            teardown: Vec::new(),
            teardown_enabled: true,
            data_sets: Vec::new(),
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "多组用户".into(),
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

        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        let connect = move || {
            let url = format!("http://{addr}");
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "run_scenario",
                        "arguments": {
                            "scenarioId": scenario.id.to_string(),
                            "dataSets": [
                                { "name": "组1", "vars": { "uid": "1" } },
                                { "name": "组2", "vars": { "uid": "2" } }
                            ]
                        } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        eprintln!(
            "BODY={}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        assert_eq!(body["success"], true, "{reply}");
        assert_eq!(body["data"]["kind"], "data-driven");
        assert_eq!(body["data"]["total"], 2);
        assert_eq!(body["data"]["okRows"], 2);
        let rows = body["data"]["rows"].as_array().unwrap();
        assert_eq!(
            rows[0]["steps"][0]["extracted"]["userId"], "1",
            "行 1 提取自己的数据（隔离）"
        );
        assert_eq!(rows[1]["steps"][0]["extracted"]["userId"], "2");

        // 两组各落一条历史（runId 对应历史行）。
        let run1 = rows[0]["runId"].as_str().unwrap();
        let runs = repo::list_scenario_runs(&db, scenario.id, 10)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().any(|r| r.id.to_string() == run1));
        // meta.matrix 标注（与 Tauri 侧同形状：{"matrix": {...}} 包装）。
        let meta1: Value = serde_json::from_str(
            runs.iter()
                .find(|r| r.id.to_string() == run1)
                .unwrap()
                .meta_json
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(meta1["matrix"]["kind"], "data");
        assert_eq!(meta1["matrix"]["label"], "组1");
    }

    #[tokio::test]
    async fn create_scenario_via_mcp_with_initial_steps() {
        use miyo_storage::repository as repo;

        let dir = std::env::temp_dir().join(format!("miyo-mcp-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        let project = repo::create_project(&db, "下单项目", "").await.unwrap();
        let make_ep = |name: &str, path: &str| miyo_core::model::Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id: project.id,
            folder_id: None,
            name: name.into(),
            method: miyo_core::model::HttpMethod::POST,
            path: path.into(),
            description: String::new(),
            status: miyo_core::model::EndpointStatus::Developing,
            sort_order: 0,
            request: miyo_core::model::RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let first = make_ep("下单", "/orders");
        let second = make_ep("支付", "/pay");
        repo::save_endpoint(&db, &first).await.unwrap();
        repo::save_endpoint(&db, &second).await.unwrap();

        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        let base_url = format!("http://{addr}");
        let connect = || {
            let url = base_url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        // 1. 建带两步的场景：唯一项目自动选中，步骤按 endpointIds 顺序落库
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "create_scenario", "arguments": {
                "name": "收单全流程",
                "description": "下单 → 支付",
                "endpointIds": [first.id.to_string(), second.id.to_string()]
            } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["name"], "收单全流程");
        assert_eq!(body["data"]["stepCount"], 2);
        assert_eq!(parsed["result"]["isError"], false);

        // 2. 持久化正确：步骤顺序、默认启用、无提取/断言
        let scenarios = repo::list_scenarios(&db, project.id).await.unwrap();
        assert_eq!(scenarios.len(), 1);
        let created = &scenarios[0];
        assert_eq!(created.name, "收单全流程");
        assert_eq!(created.steps.len(), 2);
        assert_eq!(created.steps[0].endpoint_id, Some(first.id));
        assert_eq!(created.steps[1].endpoint_id, Some(second.id));
        assert!(created.steps.iter().all(|s| s.enabled));
        assert_eq!(created.script, None);

        // 3. 空场景（不传 endpointIds）
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "create_scenario", "arguments": { "name": "空场景" } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        let body: Value =
            serde_json::from_str(parsed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["data"]["name"], "空场景");
        assert_eq!(body["data"]["stepCount"], 0);

        // 4. 参数校验：name 缺失 / 空串 → -32602；非法 UUID 数组 → -32602
        for args in [
            serde_json::json!({}),
            serde_json::json!({ "name": "   " }),
            serde_json::json!({ "name": "x", "endpointIds": ["not-a-uuid"] }),
            serde_json::json!({ "name": "x", "endpointIds": "abc" }),
        ] {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "create_scenario", "arguments": args }
            })
            .to_string();
            let reply = handle_line_with(&line, connect).await.remove(0);
            assert!(reply.contains("-32602"), "应拒绝 {args}：{reply}");
        }

        // 5. 引用不存在的接口 → 工具级错误，且不落库
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "create_scenario", "arguments": {
                "name": "坏链路", "endpointIds": [Uuid::new_v4().to_string()]
            } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        // 引用不存在接口：业务失败 → isError 结果 + 信封（ENDPOINT_NOT_FOUND）。
        assert!(
            reply.contains("isError\":true") && reply.contains("ENDPOINT_NOT_FOUND"),
            "{reply}"
        );
        assert!(!reply.contains("坏链路"), "{reply}");
        let after = repo::list_scenarios(&db, project.id).await.unwrap();
        assert_eq!(after.len(), 2, "失败的新建不应留下记录");
    }

    #[tokio::test]
    async fn scenario_tools_roundtrip_via_mcp() {
        use miyo_core::model::{Scenario, ScenarioStep};
        use miyo_storage::repository as repo;

        let dir = std::env::temp_dir().join(format!("miyo-mcp-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
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
                extract: serde_json::json!([
                    { "name": "userId", "from": "body", "path": "$.userId" }
                ]),
                assertions: serde_json::json!([]),
                ..Default::default()
            }],
            script: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_scenario(&db, &scenario).await.unwrap();

        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        // Fn 闭包（test_connect 产出的 FnOnce 只能调用一次，本测试要连 4 次）。
        let base_url = format!("http://{addr}");
        let connect = || {
            let url = base_url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        // list_scenarios：唯一项目自动选中
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "list_scenarios", "arguments": {} }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(
            reply.contains("链路演示") && reply.contains("hasCustomScript"),
            "{reply}"
        );

        // get_scenario：链路变量 + 接口配置内嵌
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": { "name": "get_scenario",
                        "arguments": { "scenarioId": scenario.id.to_string() } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(
            reply.contains("chainVariables") && reply.contains("userId"),
            "{reply}"
        );
        assert!(reply.contains("文章详情"));

        // save_scenario_script：写入 → 持久化
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": { "name": "save_scenario_script",
                        "arguments": { "scenarioId": scenario.id.to_string(),
                                       "script": "const BASE = 'https://x';" } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(reply.contains("scriptSet"), "{reply}");
        let stored = repo::get_scenario(&db, scenario.id).await.unwrap();
        assert_eq!(stored.script.as_deref(), Some("const BASE = 'https://x';"));

        // 缺 script 参数 → -32602（连接前拦截）
        let bad = serde_json::json!({
            "jsonrpc": "2.0", "id": 13, "method": "tools/call",
            "params": { "name": "save_scenario_script",
                        "arguments": { "scenarioId": scenario.id.to_string() } }
        })
        .to_string();
        let reply = handle_line_with(&bad, || unreachable!("参数校验应先于连接失败"))
            .await
            .remove(0);
        assert!(reply.contains("-32602"), "{reply}");
    }

    // ---------- 阶段六：集成测试工作流端到端（MCP stdio → 控制面 → 引擎） ----------

    /// 起一个临时 axum 上游（电商三步：校验链路传参、金额数字类型）。
    async fn spawn_shop_upstream() -> String {
        let app = axum::Router::new()
            .route(
                "/shop/orders",
                axum::routing::post(|| async {
                    axum::Json(serde_json::json!({
                        "code": 0,
                        "data": {"orderNo": "ORD001", "payToken": "TOKEN001"}
                    }))
                }),
            )
            .route(
                "/shop/pay-preview",
                axum::routing::get(
                    |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                        let ok = q.get("orderNo").map(|v| v == "ORD001").unwrap_or(false);
                        if ok {
                            (
                                axum::http::StatusCode::OK,
                                axum::Json(
                                    serde_json::json!({"code": 0, "data": {"amount": 99.99}}),
                                ),
                            )
                        } else {
                            (
                                axum::http::StatusCode::BAD_REQUEST,
                                axum::Json(serde_json::json!({"error": "missing orderNo"})),
                            )
                        }
                    },
                ),
            )
            .route(
                "/shop/pay",
                axum::routing::post(|body: String| async move {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let ok = v.get("orderNo").and_then(|x| x.as_str()) == Some("ORD001")
                        && v.get("payToken").and_then(|x| x.as_str()) == Some("TOKEN001")
                        && matches!(v.get("amount"), Some(serde_json::Value::Number(_)));
                    if ok {
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "data": {"status": "SUCCESS", "tradeNo": "TRADE001"}
                            })),
                        )
                    } else {
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(serde_json::json!({"error": "bad chain"})),
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    /// 完整 AI 工作流（stdio 协议层）：
    /// 建项目/接口 → create_scenario（AI 方言 steps）→ validate_scenario →
    /// run_scenario（wait 同步）→ get_run_result → get_step_result →
    /// analyze_run_failure → update/delete_scenario_step 回归。
    #[tokio::test]
    async fn mcp_integration_testing_full_workflow() {
        let dir = std::env::temp_dir().join(format!("miyo-mcp-wf-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        let connect = test_connect(format!("http://{addr}"));
        let upstream = spawn_shop_upstream().await;

        // 工具调用助手：返回信封 data。
        async fn call(
            connect: &dyn Fn() -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
            >,
            id: u64,
            name: &str,
            args: Value,
        ) -> (Value, bool) {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": name, "arguments": args }
            })
            .to_string();
            let reply = handle_line_with(&line, connect).await.remove(0);
            let parsed: Value = serde_json::from_str(&reply).unwrap();
            let text = parsed["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("工具 {name} 响应缺 content text：{reply}"));
            let envelope: Value = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("工具 {name} 信封解析失败：{e}｜{text}"));
            let is_err = parsed["result"]["isError"].as_bool().unwrap_or(false);
            (envelope, is_err)
        }

        // 1. 项目 + 三个接口（内联请求引用 {{base_url}}）。
        let (env, _) = call(&connect, 1, "save_curl", json!({
        "command": format!("curl -X POST {upstream}/shop/orders -H 'Content-Type: application/json' -d '{{}}'"),
        "name": "创建订单"
    })).await;
        assert_eq!(env["success"], true, "{env}");
        let project_id = env["data"]["projectId"].as_str().unwrap().to_string();
        // 再导一个 GET 接口（方法/名称过滤的对照样本）。
        let (env, _) = call(
            &connect,
            18,
            "save_curl",
            json!({
                "command": format!("curl {upstream}/shop/pay-preview?orderNo=x"),
                "name": "订单详情"
            }),
        )
        .await;
        assert_eq!(env["success"], true, "{env}");

        let mut step_defs = vec![
            json!({
                "name": "创建订单",
                "request": {"method": "POST", "url": format!("{upstream}/shop/orders"), "spec": {}},
                "extractors": [
                    {"name": "orderNo", "source": "response.body", "expression": "$.data.orderNo", "required": true},
                    {"name": "payToken", "source": "response.body", "expression": "$.data.payToken"}
                ],
                "assertions": [
                    {"source": "response.status", "operator": "equals", "expected": 200},
                    {"source": "response.body", "expression": "$.data.orderNo", "operator": "exists"}
                ]
            }),
            json!({
                "name": "支付预览",
                "request": {"method": "GET", "url": format!("{upstream}/shop/pay-preview?orderNo={{{{orderNo}}}}"), "spec": {}},
                "extractors": [{"name": "amount", "source": "response.body", "expression": "$.data.amount"}],
                "assertions": [
                    {"source": "response.status", "operator": "equals", "expected": 200},
                    {"source": "response.body", "expression": "$.data.amount", "operator": "greaterThan", "expected": 0}
                ]
            }),
            json!({
                "name": "确认支付",
                "request": {"method": "POST", "url": format!("{upstream}/shop/pay"), "spec": {
                    "params": [], "headers": [], "path_variables": [],
                    "auth": {"type": "none"},
                    "body": {"mode": "json", "raw": "{\"orderNo\": \"{{orderNo}}\", \"payToken\": \"{{payToken}}\", \"amount\": \"{{amount}}\"}"},
                    "timeout_ms": null, "follow_redirects": true, "tests": null
                }},
                "extractors": [{"name": "tradeNo", "source": "response.body", "expression": "$.data.tradeNo"}],
                "assertions": [
                    {"source": "response.status", "operator": "equals", "expected": 200},
                    {"source": "response.body", "expression": "$.data.status", "operator": "equals", "expected": "SUCCESS"},
                    {"source": "response.body", "expression": "$.data.tradeNo", "operator": "exists"}
                ]
            }),
        ];

        // 2. create_scenario（AI 方言）。
        let (env, is_err) = call(
            &connect,
            2,
            "create_scenario",
            json!({
                "name": "下单支付全流程",
                "projectId": project_id,
                "category": "电商",
                "steps": step_defs.clone()
            }),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["success"], true);
        let scenario_id = env["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(env["data"]["stepCount"], 3);

        // 3. validate_scenario：结构合法 + 变量链路可解析。
        let (env, is_err) = call(
            &connect,
            3,
            "validate_scenario",
            json!({"scenarioId": scenario_id}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["valid"], true, "校验应通过：{env}");
        assert_eq!(env["data"]["errors"].as_array().unwrap().len(), 0);
        // 写操作提示存在（POST 步骤）。
        assert!(
            env["data"]["sideEffectSteps"].as_array().unwrap().len() >= 2,
            "应有写操作提示：{env}"
        );

        // 4. run_scenario（同步）：三步全过 + 变量链路。
        let (env, is_err) = call(
            &connect,
            4,
            "run_scenario",
            json!({"scenarioId": scenario_id, "wait": true}),
        )
        .await;
        assert!(!is_err, "{env}");
        let data = &env["data"];
        assert_eq!(data["passed"], 3, "三步应全过：{env}");
        assert_eq!(data["failed"], 0);
        let run_id = data["runId"].as_str().unwrap().to_string();
        // 变量链路：无需手动传值。
        assert_eq!(data["variables"]["orderNo"], "ORD001");
        assert_eq!(data["variables"]["amount"], "99.99");
        assert_eq!(data["variables"]["tradeNo"], "TRADE001");

        // 5. get_run_result（落库回读）：摘要 + 步骤摘要。
        let (env, is_err) = call(&connect, 5, "get_run_result", json!({"runId": run_id})).await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["status"], "completed");
        assert_eq!(env["data"]["passed"], 3, "get_run_result 摘要：{env}");
        assert_eq!(env["data"]["steps"].as_array().unwrap().len(), 3);
        // 步骤摘要不含响应正文（get_step_result 才有）。
        assert!(env["data"]["steps"][0].get("bodyPreview").is_none());

        // 6. get_step_result：第 3 步完整详情（请求快照 + 响应正文 + 断言明细）。
        let (env, is_err) = call(
            &connect,
            6,
            "get_step_result",
            json!({"runId": run_id, "stepIndex": 2}),
        )
        .await;
        assert!(!is_err, "{env}");
        let step = &env["data"];
        assert_eq!(step["name"], "确认支付");
        assert_eq!(step["stepStatus"], "success");
        // 实际请求：变量已替换。
        let req_body = step["request"]["body"].as_str().unwrap_or("");
        assert!(
            req_body.contains("ORD001") && req_body.contains("TOKEN001"),
            "{step}"
        );
        // 响应正文 + 断言逐条。
        assert!(step["response"]["body"]
            .as_str()
            .unwrap_or("")
            .contains("TRADE001"));
        let outcomes = step["assertions"].as_array().unwrap();
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes.iter().all(|o| o["passed"] == true));
        // 类型保留：amount 是数字（引擎级验证已在 miyo-test 覆盖，这里核对请求体形态）。
        assert!(
            !req_body.contains("\"{{amount}}\""),
            "变量应已替换：{req_body}"
        );

        // 7. analyze_run_failure：全过场景 → ok=true 无失败步。
        let (env, is_err) =
            call(&connect, 7, "analyze_run_failure", json!({"runId": run_id})).await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["ok"], true);
        assert_eq!(env["data"]["failedSteps"].as_array().unwrap().len(), 0);
        assert!(env["data"]["disclaimer"].as_str().unwrap().contains("推测"));

        // 8. 失败路径：改坏第 2 步断言（期望 500）→ run → analyze 给出断言失败分析。
        step_defs[1]["assertions"][0]["expected"] = json!(500);
        let (env, is_err) = call(
            &connect,
            8,
            "update_scenario",
            json!({"scenarioId": scenario_id, "steps": step_defs}),
        )
        .await;
        assert!(!is_err, "{env}");
        let (env, is_err) = call(
            &connect,
            9,
            "run_scenario",
            json!({"scenarioId": scenario_id, "stopOnFailure": false}),
        )
        .await;
        assert!(!is_err, "{env}");
        let run2 = env["data"]["runId"].as_str().unwrap().to_string();
        let (env, is_err) = call(&connect, 10, "analyze_run_failure", json!({"runId": run2})).await;
        assert!(!is_err, "{env}");
        let failed_steps = env["data"]["failedSteps"].as_array().unwrap();
        // 步 2 断言失败（期望 500 实际 200）；步 3 连锁（typed 层只吃成功步的
        // 提取，金额回退字符串形态 → 上游 400）——分析器如实分类两者。
        assert_eq!(failed_steps.len(), 2, "{env}");
        assert_eq!(failed_steps[0]["errorType"], "ASSERTION_FAILED");
        assert_eq!(failed_steps[0]["index"], 1);
        assert!(!failed_steps[0]["assertionFailures"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(failed_steps[0]["assertionFailures"][0]["expected"], "500");
        assert_eq!(failed_steps[0]["assertionFailures"][0]["actual"], "200");
        assert!(!failed_steps[0]["suggestions"]
            .as_array()
            .unwrap()
            .is_empty());
        // 首次失败：无 recurring。
        assert!(
            failed_steps[0].get("recurring").is_none() || failed_steps[0]["recurring"].is_null()
        );

        // 8.5 再跑一次同样的坏断言 → analyze 标记连续失败（环境漂移信号）。
        let (env, is_err) = call(
            &connect,
            22,
            "run_scenario",
            json!({"scenarioId": scenario_id, "stopOnFailure": false}),
        )
        .await;
        assert!(!is_err, "{env}");
        let run3 = env["data"]["runId"].as_str().unwrap().to_string();
        let (env, is_err) = call(&connect, 23, "analyze_run_failure", json!({"runId": run3})).await;
        assert!(!is_err, "{env}");
        let failed_steps = env["data"]["failedSteps"].as_array().unwrap();
        assert!(!failed_steps.is_empty(), "{env}");
        let step1 = failed_steps
            .iter()
            .find(|s| s["index"] == 1)
            .expect("步 2 应在失败清单");
        assert_eq!(
            step1["recurring"]["consecutiveRuns"], 2,
            "连续两次失败应标记：{env}"
        );
        assert!(step1["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap_or("").contains("环境漂移")));

        // 8.6 幂等键自动注入：autoIdempotency=true → 写操作步骤带 Idempotency-Key。
        let (env, is_err) = call(
            &connect,
            24,
            "run_scenario",
            json!({
                "scenarioId": scenario_id,
                "stopOnFailure": false,
                "autoIdempotency": true
            }),
        )
        .await;
        assert!(!is_err, "{env}");
        let run4 = env["data"]["runId"].as_str().unwrap().to_string();
        let (env, is_err) = call(
            &connect,
            25,
            "get_step_result",
            json!({"runId": run4, "stepIndex": 0}),
        )
        .await;
        assert!(!is_err, "{env}");
        let headers = env["data"]["request"]["headers"].as_array().unwrap();
        assert!(
            headers.iter().any(|h| h[0]
                .as_str()
                .unwrap_or("")
                .eq_ignore_ascii_case("Idempotency-Key")),
            "写操作步骤应自动带幂等键：{env}"
        );
        let key = headers
            .iter()
            .find(|h| {
                h[0].as_str()
                    .unwrap_or("")
                    .eq_ignore_ascii_case("Idempotency-Key")
            })
            .unwrap()[1]
            .as_str()
            .unwrap();
        assert_eq!(key, format!("{run4}-0"));
        // GET 步骤（步骤 2）不注入。
        let (env, is_err) = call(
            &connect,
            26,
            "get_step_result",
            json!({"runId": run4, "stepIndex": 1}),
        )
        .await;
        assert!(!is_err, "{env}");
        let headers = env["data"]["request"]["headers"].as_array().unwrap();
        assert!(
            !headers.iter().any(|h| h[0]
                .as_str()
                .unwrap_or("")
                .eq_ignore_ascii_case("Idempotency-Key")),
            "读操作不应注入幂等键：{env}"
        );

        // 9. 步骤级操作回归：delete_scenario_step 删到 2 步 + add_scenario_step 加回。
        let (env, is_err) = call(
            &connect,
            11,
            "delete_scenario_step",
            json!({"scenarioId": scenario_id, "index": 2}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["stepCount"], 2);
        let (env, is_err) = call(
            &connect,
            12,
            "add_scenario_step",
            json!({"scenarioId": scenario_id, "step": step_defs[2]}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["stepCount"], 3);
        assert_eq!(env["data"]["index"], 2);

        // 9.4 复原步骤 2 的断言（修回 200），恢复健康场景。
        step_defs[1]["assertions"][0]["expected"] = json!(200);
        let (env, is_err) = call(
            &connect,
            21,
            "update_scenario",
            json!({"scenarioId": scenario_id, "steps": step_defs}),
        )
        .await;
        assert!(!is_err, "{env}");

        // 9.5 异步执行：wait=false → runId → get_run_result 轮询到 completed。
        let (env, is_err) = call(
            &connect,
            19,
            "run_scenario",
            json!({"scenarioId": scenario_id, "wait": false}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["status"], "running", "{env}");
        let async_run_id = env["data"]["runId"].as_str().unwrap().to_string();
        let mut polled = serde_json::Value::Null;
        for _ in 0..50 {
            let (env, is_err) = call(
                &connect,
                20,
                "get_run_result",
                json!({"runId": async_run_id}),
            )
            .await;
            assert!(!is_err, "{env}");
            if env["data"]["status"] == "running" {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
            polled = env.clone();
            break;
        }
        assert_eq!(
            polled["data"]["status"], "completed",
            "异步运行应完成：{polled}"
        );
        assert_eq!(polled["data"]["passed"], 3, "修复后三步全过：{polled}");

        // 10. 环境工具 + list_endpoints 筛选。
        let (env, is_err) = call(
            &connect,
            13,
            "list_environments",
            json!({"projectId": project_id}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"].as_array().unwrap().len(), 0, "新项目无环境");
        let (env, is_err) = call(
            &connect,
            14,
            "list_endpoints",
            json!({"projectId": project_id, "name": "创建"}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"].as_array().unwrap().len(), 1, "名称过滤：{env}");
        let (env, is_err) = call(
            &connect,
            15,
            "list_endpoints",
            json!({"projectId": project_id, "method": "POST"}),
        )
        .await;
        assert!(!is_err, "{env}");
        // 项目里只有 save_curl 落的一个 POST 接口（场景步骤是内联请求，不占接口表）。
        assert_eq!(env["data"].as_array().unwrap().len(), 1, "方法过滤");
        let (env, is_err) = call(
            &connect,
            17,
            "list_endpoints",
            json!({"projectId": project_id, "method": "GET"}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(
            env["data"].as_array().unwrap().len(),
            1,
            "GET 只命中订单详情"
        );

        // 11. 旧工具兼容：get_scenario 仍返回完整上下文（camelCase）。
        let (env, is_err) = call(
            &connect,
            16,
            "get_scenario",
            json!({"scenarioId": scenario_id}),
        )
        .await;
        assert!(!is_err, "{env}");
        assert_eq!(env["data"]["name"], "下单支付全流程");
        assert_eq!(env["data"]["steps"].as_array().unwrap().len(), 3);

        db.close().await;
    }

    // ---------- P0 MCP 升级：注册完整性 / 权限 / Resources / Prompts / 并发 ----------

    /// 注册表元数据 ↔ tools/list 定义 1:1（注解 / Schema / 描述齐全）。
    #[test]
    fn registry_metadata_matches_definitions() {
        let defs = tool_definitions();
        let meta = miyo_agent::registry::all_tool_meta();
        assert_eq!(defs.len(), meta.len());
        for d in &defs {
            let name = d["name"].as_str().unwrap();
            let m = miyo_agent::registry::tool_meta(name)
                .unwrap_or_else(|| panic!("tools/list 多出未注册工具：{name}"));
            assert_eq!(d["annotations"]["readOnlyHint"], m.read_only, "{name}");
            assert_eq!(d["annotations"]["destructiveHint"], m.destructive, "{name}");
            assert_eq!(d["annotations"]["openWorldHint"], m.open_world, "{name}");
            assert_eq!(d["x-miyo"]["permission"], m.permission.as_str(), "{name}");
            assert_eq!(d["inputSchema"]["type"], "object", "{name}");
            assert!(
                d["description"].as_str().unwrap().len() > 10,
                "{name} 描述过短"
            );
        }
        for m in meta {
            assert!(
                defs.iter().any(|d| d["name"].as_str() == Some(m.name)),
                "注册表工具 {} 缺 tools/list 定义",
                m.name
            );
        }
    }

    /// 启动临时控制面（全部权限放开）。返回 (base_url, db 控制句柄元组)。
    /// miyo-mcp 不依赖 sqlx：db 以 `init_db` 返回的池类型经泛型省略——这里用
    /// 具体池类型需要 sqlx 依赖，改为返回池的 `Clone`（miyo-storage 池为
    /// `sqlx::SqlitePool` 的别名，测试里直接用）。为免新增依赖，把池留在
    /// 闭包外用 `Box<dyn std::any::Any>`？——不，直接加 dev-dependency。
    async fn spawn_full_control_plane() -> (String, sqlx::SqlitePool) {
        use miyo_storage::repository as repo;
        let dir = std::env::temp_dir().join(format!("miyo-mcp-up-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = init_db(&dir.join("t.db")).await.unwrap();
        repo::set_setting(
            &db,
            miyo_agent::server::MCP_PERMISSIONS_KEY,
            &serde_json::json!(["read", "write", "execute", "network_execute", "destructive"])
                .to_string(),
        )
        .await
        .unwrap();
        let state = AgentState::new(db.clone(), "tok");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await });
        (format!("http://{addr}"), db)
    }

    /// 注册完整性：注册表内每个工具都经 tools/call 分发到（最小参数调用
    /// 要么成功、要么参数/业务错误——绝不出现「未注册工具」）。
    #[tokio::test]
    async fn every_registered_tool_dispatches() {
        let (url, _db) = spawn_full_control_plane().await;
        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };
        for meta in miyo_agent::registry::all_tool_meta() {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": meta.name, "arguments": {} }
            })
            .to_string();
            let lines = handle_line_with(&line, connect).await;
            let reply = lines.last().unwrap();
            assert!(
                !reply.contains("TOOL_NOT_FOUND") && !reply.contains("未注册"),
                "注册工具 {} 未接入分发：{reply}",
                meta.name
            );
        }
    }

    /// 权限等级服务端拦截：仅 read 时，write / execute / network_execute /
    /// destructive 工具一律 PERMISSION_DENIED（MCP 层拿到 isError 信封）。
    #[tokio::test]
    async fn permission_denial_surfaces_clear_code() {
        use miyo_storage::repository as repo;
        let (url, db) = spawn_full_control_plane().await;
        repo::set_setting(
            &db,
            miyo_agent::server::MCP_PERMISSIONS_KEY,
            &serde_json::json!(["read"]).to_string(),
        )
        .await
        .unwrap();
        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        // read 工具可用。
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(
            reply.contains("success") && reply.contains("isError\":false"),
            "read 工具应放行：{reply}"
        );

        // write（save_curl）→ 拒绝且带等级说明。
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "save_curl",
                        "arguments": { "command": "curl https://api.example.com/x" } }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(
            reply.contains("PERMISSION_DENIED") && reply.contains("isError\":true"),
            "write 工具在仅 read 时必须拒绝：{reply}"
        );
        assert!(
            reply.contains("创建与修改数据"),
            "拒绝消息带等级标签：{reply}"
        );
    }

    /// Resources：清单 / 读取 / 脱敏 / 未知 URI 拒绝。
    #[tokio::test]
    async fn resources_read_with_redaction() {
        use miyo_storage::repository as repo;
        let (url, db) = spawn_full_control_plane().await;
        let project = repo::create_project(&db, "资源项目", "").await.unwrap();
        let env = miyo_core::model::Environment {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "prod".into(),
            base_url: "https://api.example.com".into(),
            variables: vec![miyo_core::model::EnvironmentVariable {
                key: "api_token".into(),
                remote_value: "secret-token-1".into(),
                local_value: String::new(),
                enabled: true,
                local_override: None,
                sensitive: None,
                description: None,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repo::save_environment(&db, &env).await.unwrap();

        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };

        // resources/list：含固定资源与模板。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(
            reply.contains("miyo://projects") && reply.contains("uriTemplate"),
            "{reply}"
        );

        // resources/read miyo://projects：返回项目。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"miyo://projects"}}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(reply.contains("资源项目"), "{reply}");

        // resources/read 环境详情：敏感变量值必须脱敏。
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "resources/read",
            "params": { "uri": format!("miyo://environments/{}", env.id) }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(
            !reply.contains("secret-token-1"),
            "环境敏感值不得明文：{reply}"
        );
        assert!(reply.contains("__MIYO_REDACTED__"), "{reply}");

        // 未知 URI → 明确 NOT_FOUND。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"miyo://nope"}}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(reply.contains("NOT_FOUND"), "{reply}");

        // 运行资源：runId 非法 → VALIDATION_FAILED。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"miyo://runs/zzz"}}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(reply.contains("VALIDATION_FAILED"), "{reply}");
    }

    /// Resources 读取走 read 权限：关闭 read 后资源读取被拒。
    #[tokio::test(flavor = "multi_thread")]
    async fn resources_require_read_permission() {
        use miyo_storage::repository as repo;
        let (url, db) = spawn_full_control_plane().await;
        repo::set_setting(
            &db,
            miyo_agent::server::MCP_PERMISSIONS_KEY,
            &serde_json::json!([]).to_string(),
        )
        .await
        .unwrap();
        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };
        // 直接用闭包产物发请求（隔离 MCP 层）。
        let direct = (connect)().await.unwrap();
        eprintln!(
            "direct states={:?}",
            direct.list_tool_states().await.map(|v| v.len())
        );
        eprintln!(
            "direct list_projects={:?}",
            direct.list_projects().await.map(|v| v.len())
        );
        // 探针：先确认控制面存活（agent_info 是纯客户端回显，必须用真实 HTTP 工具）。
        let probe = handle_line_with(
            r#"{"jsonrpc":"2.0","id":0,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(!probe.contains("连接失败"), "控制面应存活：{probe}");
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"miyo://projects"}}"#,
            connect,
        )
        .await
        .remove(0);
        assert!(
            reply.contains("PERMISSION_DENIED"),
            "read 全关时资源读取必须拒绝：{reply}"
        );
    }

    /// Prompts：清单 + 展开 + 参数校验。
    #[tokio::test]
    async fn prompts_list_and_get() {
        // prompts/list 不需要控制面。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":1,"method":"prompts/list"}"#,
            || unreachable!(),
        )
        .await
        .remove(0);
        assert!(
            reply.contains("design-api-tests") && reply.contains("author-scenario"),
            "{reply}"
        );

        // 缺参 → -32602。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":2,"method":"prompts/get","params":{"name":"design-api-tests"}}"#,
            || unreachable!(),
        )
        .await
        .remove(0);
        assert!(reply.contains("-32602"), "{reply}");

        // 带参展开：消息包含接口 id 与步骤指引。
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "prompts/get",
            "params": { "name": "design-api-tests",
                        "arguments": { "endpointId": "abc-123" } }
        })
        .to_string();
        let reply = handle_line_with(&line, || unreachable!()).await.remove(0);
        assert!(
            reply.contains("abc-123") && reply.contains("get_endpoint"),
            "{reply}"
        );

        // 未知 prompt → -32602。
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":4,"method":"prompts/get","params":{"name":"nope"}}"#,
            || unreachable!(),
        )
        .await
        .remove(0);
        assert!(reply.contains("-32602"), "{reply}");
    }

    /// get_context：返回当前项目摘要，敏感变量脱敏，无请求规格明细。
    #[tokio::test]
    async fn get_context_returns_masked_summary() {
        use miyo_storage::repository as repo;
        let (url, db) = spawn_full_control_plane().await;
        let project = repo::create_project(&db, "上下文项目", "").await.unwrap();
        repo::set_setting(
            &db,
            "active_project_id",
            &serde_json::json!(project.id.to_string()).to_string(),
        )
        .await
        .unwrap();
        let mut with_vars = project.clone();
        with_vars.variables = std::collections::HashMap::from([
            (
                "base_url".to_string(),
                "https://api.example.com".to_string(),
            ),
            ("api_key".to_string(), "secret-key-9".to_string()),
        ]);
        repo::update_project(&db, &with_vars).await.unwrap();

        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "get_context", "arguments": {} }
        })
        .to_string();
        let reply = handle_line_with(&line, connect).await.remove(0);
        assert!(reply.contains("上下文项目"), "应含当前项目：{reply}");
        assert!(!reply.contains("secret-key-9"), "敏感变量不得明文：{reply}");
        assert!(reply.contains("__MIYO_REDACTED__"), "{reply}");
    }

    /// 并发工具调用：两个并发 tools/call 都成功（互不阻塞）。
    #[tokio::test]
    async fn concurrent_tool_calls_both_succeed() {
        let (url, _db) = spawn_full_control_plane().await;
        let connect = || {
            let url = url.clone();
            Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
                >
        };
        let line_a = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}"#;
        let line_b = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"agent_info","arguments":{}}}"#;
        let (a, b) = tokio::join!(
            handle_line_with(line_a, connect),
            handle_line_with(line_b, connect),
        );
        let ra = a.last().unwrap();
        let rb = b.last().unwrap();
        assert!(ra.contains("success"), "并发调用 A 失败：{ra}");
        assert!(rb.contains("address"), "并发调用 B 失败：{rb}");
    }

    /// 串行闸：非并发工具按名互斥（second 在 first 释放前不可进入）。
    #[tokio::test]
    async fn serial_gate_blocks_same_name() {
        let g1 = serial_gate("update_test_case").await;
        let first_done = std::sync::Arc::new(tokio::sync::Notify::new());
        let notifier = first_done.clone();
        let t = tokio::spawn(async move {
            let _g = g1;
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            notifier.notify_waiters();
            notifier.notify_one();
        });
        // 在 g1 持有期间尝试获取：80ms 内不应成功。
        let g2 = tokio::spawn(serial_gate("update_test_case"));
        let acquired = tokio::time::timeout(std::time::Duration::from_millis(30), g2).await;
        assert!(acquired.is_err(), "同名非并发工具在持有期间不得进入");
        t.await.unwrap();
    }
}
