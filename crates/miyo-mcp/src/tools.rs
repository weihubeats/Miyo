//! MCP 协议处理：JSON-RPC 分发 + 工具定义 + 工具调用。
//!
//! 只实现 server 必需的最小集：`initialize` / `ping` / `tools/list` /
//! `tools/call`（+ 未知方法 -32601）。协议版本回显客户端请求值，兼容性最好。

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
pub async fn handle_line_discover(line: &str) -> Option<String> {
    handle_line_with(line, || Box::pin(cached_client())).await
}

pub(crate) async fn handle_line_with(
    line: &str,
    connect: impl FnOnce() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
    >,
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
        // 先校验工具与参数，全部通过才建立控制面连接——缺参 / 未知工具
        // 这类注定失败的调用不该付一次连接往返（也不该把连接错误盖过
        // 更贴近调用方的 -32602 参数错误）。
        "tools/call" => match validate_tool_call(&params) {
            Err(e) => Err(e),
            // 执行失败按 MCP 约定返回 isError: true 的正常结果（信封在 content 里，
            // AI 可解析 {success:false, error:{code,message,details}}）。
            Ok(()) => match connect().await {
                Err(e) => Err(tool_error(&e.user_message())),
                Ok(client) => match call_tool(&client, &params).await {
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
        "capabilities": { "tools": { "listChanged": false } },
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
        _ => "INTERNAL".to_string(),
    };
    ToolFailure {
        code,
        message: e.user_message(),
        details: Value::Null,
    }
}

// ---------- 工具定义 ----------

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "save_curl",
            "description": "把一条 cURL 命令解析并保存为 Miyo 接口（API 调试工具中的可编辑接口）。URL 拆为 base_url + 路径 + 查询参数；返回 endpointId。多个项目时必须传 projectId。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "完整 cURL 命令字符串" },
                    "name": { "type": "string", "description": "可选，接口名称；缺省从 URL 路径推导" },
                    "projectId": { "type": "string", "description": "可选，目标项目 UUID；缺省时唯一项目自动选中、零项目自动创建" },
                    "folderId": { "type": "string", "description": "可选，归属文件夹 UUID" }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "list_projects",
            "description": "列出 Miyo 中全部项目（id / 名称 / 描述 / 变量）。",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_endpoints",
            "description": "列出指定项目下的接口（方法 / 路径 / 名称 / 请求规格）。支持按名称子串（匹配名称或路径）、HTTP 方法、所属分组过滤，支持分页；全缺省时返回全量（与旧版兼容）。创建场景前用它了解项目里有哪些接口。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "项目 UUID（先调 list_projects 获取）" },
                    "name": { "type": "string", "description": "可选，名称/路径子串过滤（大小写不敏感）" },
                    "method": { "type": "string", "description": "可选，HTTP 方法过滤（GET/POST/…，大小写不敏感）" },
                    "folderId": { "type": "string", "description": "可选，分组 UUID 过滤" },
                    "limit": { "type": "number", "description": "可选，分页条数（配合 offset；缺省全量）" },
                    "offset": { "type": "number", "description": "可选，分页偏移（缺省 0）" }
                },
                "required": ["projectId"]
            }
        }),
        json!({
            "name": "get_endpoint",
            "description": "读取单个接口的完整定义（方法 / 路径 / 请求头 / 参数 / Body / 认证形态），调用 send_endpoint 前先看它。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endpointId": { "type": "string", "description": "接口 UUID（list_endpoints / save_curl 返回）" }
                },
                "required": ["endpointId"]
            }
        }),
        json!({
            "name": "send_endpoint",
            "description": "发送一次真实 HTTP 请求（链式调用的核心工具）。按 Miyo 中已保存接口的当前配置执行，支持覆盖：url（完整地址）、headers、query、body（请求体原文）、variables（额外变量，渲染进 {{var}} 占位）。返回完整响应（状态码 / 耗时 / 正文，超 64KB 截断）。典型链路：调用下单接口 → 从响应正文提取订单号 → 用 variables/body 把订单号传给支付预览接口 → 再调支付接口。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endpointId": { "type": "string", "description": "接口 UUID（list_endpoints / save_curl 返回）" },
                    "url": { "type": "string", "description": "可选，覆盖完整 URL 模板（支持 {{var}} 占位）" },
                    "headers": { "type": "object", "description": "可选，追加/覆盖请求头 {\"名\": \"值\"}", "additionalProperties": { "type": "string" } },
                    "query": { "type": "object", "description": "可选，追加/覆盖 query 参数", "additionalProperties": { "type": "string" } },
                    "body": { "type": "string", "description": "可选，覆盖请求体原文（Json/Text 原位替换；其余类型降级为 text）" },
                    "variables": { "type": "object", "description": "可选，额外变量注入（优先级最高），如 {\"orderId\": \"A1024\"}", "additionalProperties": { "type": "string" } },
                    "environmentId": { "type": "string", "description": "可选，环境 UUID（缺省用全局+项目变量）" },
                    "timeoutMs": { "type": "number", "description": "可选，本次请求超时毫秒（缺省 接口级→全局设置→300s）" }
                },
                "required": ["endpointId"]
            }
        }),
        json!({
            "name": "run_scenario",
            "description": "运行集成测试场景：按步骤顺序执行，条件/循环/并行按配置生效，前一步提取的链路变量自动传入后续步骤，逐步执行断言并落运行历史。返回 runId 与每步摘要（状态码/断言/提取值/响应预览）。长场景建议 wait=false 拿 runId 后用 get_run_result 轮询。生产环境或含写操作（POST/PUT/DELETE）的场景先经用户确认，建议带幂等键（如 Idempotency-Key 请求头）防重复下单。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID（list_scenarios 返回）" },
                    "environmentId": { "type": "string", "description": "可选，环境 UUID（缺省用全局+项目变量；生产环境先经用户确认）" },
                    "stopOnFailure": { "type": "boolean", "description": "可选，失败即停（默认 true）" },
                    "variables": { "type": "object", "description": "可选，初始变量注入（优先级最高）", "additionalProperties": { "type": "string" } },
                    "autoIdempotency": { "type": "boolean", "description": "可选，true 时为写操作步骤自动补 Idempotency-Key 头（键 = runId-步骤位次，重试不重复执行业务）——AI 重试或不确定幂等性时建议开启" },
                    "wait": { "type": "boolean", "description": "可选，true（默认）同步等待完整结果；false 立即返回 runId，用 get_run_result 轮询" }
                },
                "required": ["scenarioId"]
            }
        }),
        json!({
            "name": "list_scenarios",
            "description": "列出 Miyo 中的集成测试场景（接口链路）。返回 id / 名称 / 描述 / 步骤数 / 是否已有自定义脚本。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "可选，项目 UUID；缺省时仅一个项目自动选中" }
                }
            }
        }),
        json!({
            "name": "create_scenario",
            "description": "新建集成测试场景。两种步骤来源：① steps 完整定义（推荐）——每步含内联请求或接口引用、变量提取（extractors 或 extract）、断言（source/operator 形状自动翻译）、执行条件/循环，顺序即数组顺序；② endpointIds 顺序引用（仅建骨架，无提取/断言）。返回 scenarioId。推荐工作流：list_endpoints → get_endpoint 看清参数与响应 → create_scenario（steps 一步到位）→ validate_scenario → run_scenario。接口信息不足时先问用户，不要编造字段。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "场景名称（不可为空）" },
                    "projectId": { "type": "string", "description": "可选，目标项目 UUID；缺省时仅一个项目自动选中" },
                    "description": { "type": "string", "description": "可选，场景说明" },
                    "category": { "type": "string", "description": "可选，分类标签" },
                    "endpointIds": { "type": "array", "items": { "type": "string" }, "description": "可选，按顺序引用的接口 UUID（无提取/断言的骨架）" },
                    "steps": {
                        "type": "array",
                        "description": "可选，完整步骤数组（优先于 endpointIds）。每步：{\"name\":\"步骤名\",\"request\":{\"method\":\"POST\",\"url\":\"{{base_url}}/path\",\"spec\":{}}} 或 {\"endpointId\":\"UUID\",\"extractors\":[{\"name\":\"orderNo\",\"source\":\"response.body\",\"expression\":\"$.data.orderNo\"}],\"assertions\":[{\"source\":\"response.status\",\"operator\":\"equals\",\"expected\":200},{\"source\":\"response.body\",\"expression\":\"$.data.status\",\"operator\":\"equals\",\"expected\":\"SUCCESS\"}],\"condition\":{\"source\":\"var\",\"name\":\"paid\",\"op\":\"exists\"},\"loop\":{\"mode\":\"while\",\"while_condition\":{...},\"max_loops\":10,\"interval_ms\":500},\"waitMs\":0,\"parallel\":false}。URL/Body 里用 {{变量名}} 引用前一步提取的值。",
                        "items": { "type": "object" }
                    }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "get_scenario",
            "description": "读取集成测试场景的完整上下文，用于编写或修改场景脚本（Node fetch 风格）：每一步引用接口的当前配置（方法 / 路径 / 请求规格）、提取规则与断言、链路变量（前步提取、后步引用）、项目变量（含 base_url 兜底地址）与当前自定义脚本。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID（先调 list_scenarios 获取）" }
                },
                "required": ["scenarioId"]
            }
        }),
        json!({
            "name": "save_scenario_script",
            "description": "写入集成测试场景的自定义脚本（覆盖保存）。脚本为 Node 18+ 原生 fetch 风格（顶层 await），链路变量直接用前步提取的变量名引用；传空字符串表示清除自定义、回到由手动步骤实时生成。应用内运行始终以手动步骤为准，脚本用于复制 / CI 导出。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "script": { "type": "string", "description": "脚本全文（空字符串 = 清除自定义）" }
                },
                "required": ["scenarioId", "script"]
            }
        }),
        json!({
            "name": "list_environments",
            "description": "列出项目的执行环境（名称 / Base URL / 启用变量数）。场景执行前用它确认目标环境；生产命名的环境执行前必须经用户确认。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "可选，项目 UUID；缺省时仅一个项目自动选中" }
                }
            }
        }),
        json!({
            "name": "get_environment",
            "description": "读取环境详情：Base URL 与全部启用变量（敏感变量的值已脱敏为占位符，不可读出明文）。用于确认 {{变量}} 引用是否在环境中可用。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "environmentId": { "type": "string", "description": "环境 UUID（list_environments 返回）" }
                },
                "required": ["environmentId"]
            }
        }),
        json!({
            "name": "update_scenario",
            "description": "更新场景：名称 / 描述 / 分类按需覆盖；steps 提供时整体替换全部步骤（形状同 create_scenario 的 steps，支持 AI 方言）。字段缺省保留原值。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "name": { "type": "string", "description": "可选，新名称" },
                    "description": { "type": "string", "description": "可选，新描述" },
                    "category": { "type": "string", "description": "可选，新分类" },
                    "steps": { "type": "array", "items": { "type": "object" }, "description": "可选，整体替换的步骤数组（形状同 create_scenario.steps）" }
                },
                "required": ["scenarioId"]
            }
        }),
        json!({
            "name": "delete_scenario",
            "description": "删除场景（不可恢复）。运行历史随场景一并删除，响应中 deletedRuns 为被删历史条数——如需保留历史请先告知用户确认。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" }
                },
                "required": ["scenarioId"]
            }
        }),
        json!({
            "name": "add_scenario_step",
            "description": "向场景追加一个步骤（index 缺省追加到末尾）。步骤形状同 create_scenario 的 steps 元素（支持 AI 方言：extractors + source/operator 断言）。用于逐步构建或修改场景，免整场景重发。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "step": { "type": "object", "description": "步骤定义（同 create_scenario.steps 元素）" },
                    "index": { "type": "number", "description": "可选，插入位次（0-based；缺省 = 末尾）" }
                },
                "required": ["scenarioId", "step"]
            }
        }),
        json!({
            "name": "update_scenario_step",
            "description": "替换场景中指定位次的步骤（完整定义，形状同 create_scenario.steps 元素）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "index": { "type": "number", "description": "步骤位次（0-based）" },
                    "step": { "type": "object", "description": "新步骤定义" }
                },
                "required": ["scenarioId", "index", "step"]
            }
        }),
        json!({
            "name": "delete_scenario_step",
            "description": "删除场景中指定位次的步骤（其后步骤前移）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "index": { "type": "number", "description": "步骤位次（0-based）" }
                },
                "required": ["scenarioId", "index"]
            }
        }),
        json!({
            "name": "validate_scenario",
            "description": "运行前校验场景：项目/接口/环境存在性、步骤配置完整性、变量引用分析（缺失/先引用后定义）、提取与断言表达式合法性、条件/循环配置、写操作步骤清单（副作用提示）。返回结构化 issues（可多条）。执行 run_scenario 前必须先校验通过；校验不通过不要绕过，缺接口信息时先询问用户。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenarioId": { "type": "string", "description": "场景 UUID" },
                    "environmentId": { "type": "string", "description": "可选，按指定环境校验变量可用性" },
                    "runtimeVariables": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选，运行期将经 run_scenario 的 variables 注入的变量名（如每次运行传唯一订单号/幂等键）——这些引用不再报 VARIABLE_NOT_FOUND"
                    }
                },
                "required": ["scenarioId"]
            }
        }),
        json!({
            "name": "get_run_result",
            "description": "获取场景执行结果摘要：runId / 状态（running/completed/failed/unknown）/ 汇总（通过/失败/跳过步数、耗时）与每步摘要（状态码/断言通过数/提取值——不含响应正文）。run_scenario 传 wait=false 后用它轮询；completed 后用 get_step_result 看单步详情，用 analyze_run_failure 分析失败。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "runId": { "type": "string", "description": "运行 UUID（run_scenario 返回）" }
                },
                "required": ["runId"]
            }
        }),
        json!({
            "name": "get_step_result",
            "description": "获取场景运行中单个步骤的完整详情：实际请求（URL/方法/头/Body 快照）、完整响应（状态码/头/正文，超 64KB 截断）、变量提取结果、逐条断言（预期值/实际值/失败原因）。避免一次性拉全部响应，按需逐步查看。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "runId": { "type": "string", "description": "运行 UUID" },
                    "stepIndex": { "type": "number", "description": "步骤位次（0-based，get_run_result 摘要中的 index）" }
                },
                "required": ["runId", "stepIndex"]
            }
        }),
        json!({
            "name": "analyze_run_failure",
            "description": "分析场景运行的失败步骤：结合请求/响应/断言/提取情况，按规则分类错误类型（请求错误/断言失败/依赖阻断/取消）并给出可能原因与建议。输出为启发式推测而非确定结论，修复前请结合 get_step_result 的原始响应核对。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "runId": { "type": "string", "description": "运行 UUID" }
                },
                "required": ["runId"]
            }
        }),
        json!({
            "name": "agent_info",
            "description": "返回 Miyo Agent 控制面连接信息（地址与令牌文件路径），供需要直接发 HTTP 的场景使用。",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

// ---------- 工具调用 ----------

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
        "list_projects" | "agent_info" => Ok(()),
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
        "get_run_result" | "analyze_run_failure" => {
            uuid_req(&args, "runId")?;
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

async fn call_tool(client: &ControlClient, params: &Value) -> Result<Value, ToolFailure> {
    let name = params["name"].as_str().unwrap_or_default();
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
            let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);
            payload["wait"] = serde_json::json!(wait);
            if !wait {
                // 异步：立即返回 runId（JSON 形状与同步不同），原样透传。
                let raw = client
                    .run_scenario_raw(scenario_id, payload)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?;
                return Ok(tool_result(ok_envelope(raw), false));
            }
            serde_json::to_value(
                client
                    .run_scenario(scenario_id, payload)
                    .await
                    .map_err(|e| tool_fail("SCENARIO", &e))?,
            )
            .expect("序列化失败")
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
        "agent_info" => json!({
            "address": client.base(),
            "tokenPath": miyo_agent::token::token_path(&miyo_agent::default_data_dir()),
            "auth": "Authorization: Bearer <token 文件内容>",
            "docs": "POST /agent/curl | GET /agent/projects | GET /agent/endpoints/:projectId（支持 name/method/folderId/limit/offset 过滤）| GET /agent/endpoints/:id/detail | POST /agent/endpoints/:id/execute | GET /agent/environments?projectId= | GET /agent/environments/:id | GET /agent/scenarios?projectId= | POST /agent/scenarios（完整 steps）| GET /agent/scenarios/:id | PUT /agent/scenarios/:id | DELETE /agent/scenarios/:id | POST /agent/scenarios/:id/steps | PUT/DELETE /agent/scenarios/:id/steps/:index | GET /agent/scenarios/:id/validate | POST /agent/scenarios/:id/run（wait=false 异步）| GET /agent/runs/:id | GET /agent/runs/:id/steps/:index | GET /agent/runs/:id/analysis | PUT /agent/scenarios/:id/script"
        }),
        other => {
            return Err(ToolFailure::new(
                "UNKNOWN_TOOL",
                format!("未知工具：{other}"),
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

/// 极简 percent-encode（query 值：保留字母数字与 -_.~，其余转 %XX）。
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use miyo_agent::server::{build_router, AgentState};
    use miyo_storage::db::init_db;

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
        .unwrap();
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
        .unwrap();
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
        assert!(reply.is_none());
    }

    #[tokio::test]
    async fn unknown_request_is_method_not_found() {
        let reply = handle_line_with(
            r#"{"jsonrpc":"2.0","id":9,"method":"resources/list"}"#,
            || unreachable!(),
        )
        .await
        .unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
            .unwrap();
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
        .unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
        .unwrap();
        assert!(reply.contains("save_curl"), "{reply}");
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
            let reply = handle_line_with(&line, connect).await.unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
        let reply = handle_line_with(&line, connect).await.unwrap();
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
            .unwrap();
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
            let reply = handle_line_with(&line, connect).await.unwrap();
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
}
