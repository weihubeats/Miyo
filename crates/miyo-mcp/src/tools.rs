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
            Ok(()) => match connect().await {
                Err(e) => Err(tool_error(&e.user_message())),
                Ok(client) => call_tool(&client, &params).await,
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

/// 工具执行失败按 MCP 约定返回 `isError: true` 的正常结果（非协议错误）。
fn tool_error(message: &str) -> (i64, String) {
    (-32000, message.to_string())
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
            "description": "列出指定项目下的全部接口（方法 / 路径 / 名称 / 请求规格）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "项目 UUID（先调 list_projects 获取）" }
                },
                "required": ["projectId"]
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
            opt_uuid_arg(&args, "projectId").map_err(|e| (-32602, e))?;
            opt_uuid_arg(&args, "folderId").map_err(|e| (-32602, e))?;
            Ok(())
        }
        "list_projects" | "agent_info" => Ok(()),
        "list_endpoints" => {
            str_arg(&args, "projectId")
                .ok_or_else(|| (-32602, "缺少必填参数 projectId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| (-32602, "projectId 不是合法 UUID".to_string()))?;
            Ok(())
        }
        "list_scenarios" => {
            opt_uuid_arg(&args, "projectId").map_err(|e| (-32602, e))?;
            Ok(())
        }
        "get_scenario" => {
            str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| (-32602, "scenarioId 不是合法 UUID".to_string()))?;
            Ok(())
        }
        "save_scenario_script" => {
            str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse::<Uuid>()
                .map_err(|_| (-32602, "scenarioId 不是合法 UUID".to_string()))?;
            if args.get("script").and_then(|v| v.as_str()).is_none() {
                return Err((
                    -32602,
                    "缺少必填参数 script（传空字符串表示清除自定义）".to_string(),
                ));
            }
            Ok(())
        }
        other => Err((-32602, format!("未知工具：{other}"))),
    }
}

async fn call_tool(client: &ControlClient, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"].as_str().unwrap_or_default();
    // 服务端兜底之外的第一道拦截：停用的工具调用即报错（响应 isError）。
    if let Ok(states) = client.list_tool_states().await {
        if let Some(s) = states.iter().find(|s| s.name == name) {
            if !s.enabled {
                return Err(tool_error(&format!(
                    "工具 {name} 已在 Miyo 设置 → MCP 服务中停用"
                )));
            }
        }
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let value: Value = match name {
        "save_curl" => {
            let command = str_arg(&args, "command")
                .ok_or_else(|| (-32602, "缺少必填参数 command".to_string()))?;
            let project_id = opt_uuid_arg(&args, "projectId").map_err(|e| (-32602, e))?;
            let folder_id = opt_uuid_arg(&args, "folderId").map_err(|e| (-32602, e))?;
            serde_json::to_value(
                client
                    .save_curl(command, str_arg(&args, "name"), project_id, folder_id)
                    .await
                    .map_err(|e| tool_error(&e.user_message()))?,
            )
            .expect("序列化失败")
        }
        "list_projects" => serde_json::to_value(
            client
                .list_projects()
                .await
                .map_err(|e| tool_error(&e.user_message()))?,
        )
        .expect("序列化失败"),
        "list_endpoints" => {
            let project_id: Uuid = str_arg(&args, "projectId")
                .ok_or_else(|| (-32602, "缺少必填参数 projectId".to_string()))?
                .parse()
                .map_err(|_| (-32602, "projectId 不是合法 UUID".to_string()))?;
            serde_json::to_value(
                client
                    .list_endpoints(project_id)
                    .await
                    .map_err(|e| tool_error(&e.user_message()))?,
            )
            .expect("序列化失败")
        }
        "list_scenarios" => {
            let project_id = opt_uuid_arg(&args, "projectId").map_err(|e| (-32602, e))?;
            serde_json::to_value(
                client
                    .list_scenarios(project_id)
                    .await
                    .map_err(|e| tool_error(&e.user_message()))?,
            )
            .expect("序列化失败")
        }
        "get_scenario" => {
            let scenario_id: Uuid = str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse()
                .map_err(|_| (-32602, "scenarioId 不是合法 UUID".to_string()))?;
            serde_json::to_value(
                client
                    .get_scenario(scenario_id)
                    .await
                    .map_err(|e| tool_error(&e.user_message()))?,
            )
            .expect("序列化失败")
        }
        "save_scenario_script" => {
            let scenario_id: Uuid = str_arg(&args, "scenarioId")
                .ok_or_else(|| (-32602, "缺少必填参数 scenarioId".to_string()))?
                .parse()
                .map_err(|_| (-32602, "scenarioId 不是合法 UUID".to_string()))?;
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
                    .map_err(|e| tool_error(&e.user_message()))?,
            )
            .expect("序列化失败")
        }
        "agent_info" => json!({
            "address": client.base(),
            "tokenPath": miyo_agent::token::token_path(&miyo_agent::default_data_dir()),
            "auth": "Authorization: Bearer <token 文件内容>",
            "docs": "POST /agent/curl | GET /agent/projects | GET /agent/endpoints/:projectId | GET /agent/scenarios?projectId= | GET /agent/scenarios/:id | PUT /agent/scenarios/:id/script"
        }),
        other => return Err((-32602, format!("未知工具：{other}"))),
    };
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&value).expect("格式化失败") }],
        "isError": false
    }))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().filter(|s| !s.is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_agent::server::{build_router, AgentState};
    use miyo_storage::db::init_db;

    /// 启动临时控制面并构造指向它的 connect 闭包。
    fn test_connect(
        url: String,
    ) -> impl FnOnce() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ControlClient, AppError>> + Send>,
    > {
        move || Box::pin(async move { Ok(ControlClient::new(url, "tok")) })
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
            "list_scenarios",
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
        assert!(
            !reply.contains("save_curl"),
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
        // 协议级拒绝（-32000）：停用的工具在调用层即拒绝，错误信息可读。
        assert!(
            reply.contains("-32000") && reply.contains("停用"),
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
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: ep.id,
                enabled: true,
                extract: serde_json::json!([
                    { "name": "userId", "from": "body", "path": "$.userId" }
                ]),
                assertions: serde_json::json!([]),
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
}
