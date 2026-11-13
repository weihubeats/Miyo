//! 应用内 AI 循环的工具分发（name + args → 控制面 HTTP → JSON）。
//!
//! 与 miyo-mcp 的 `call_tool` 同源不同层：MCP 层有协议信封 / 进度轮询 /
//! 串行闸等协议关切；这里只做「参数透传 + 路径映射」，服务端已统一做
//! 权限校验（`ensure_tool_allowed`）与脱敏。参数名与 MCP inputSchema 一致
//! （camelCase），服务端 serde 双向兼容。

use serde_json::{json, Value};
use uuid::Uuid;

use crate::client::ControlClient;
use miyo_core::{AppError, Result};

/// 分发一次工具调用。
pub async fn dispatch_tool(client: &ControlClient, name: &str, args: &Value) -> Result<Value> {
    match name {
        // ---- 项目 / 接口 ----
        "list_projects" => client.get_raw_json("/agent/projects").await,
        "get_context" => client.get_raw_json("/agent/context").await,
        "agent_info" => Ok(json!({
            "address": client.base(),
            "tokenPath": crate::token::token_path(&crate::default_data_dir()).display().to_string(),
        })),
        // 试点：入参经 typed struct 反序列化（与 registry 生成 schema 同源）
        "list_endpoints" => {
            let a: crate::args::ListEndpointsArgs = serde_json::from_value(args.clone())
                .map_err(|e| AppError::Validation(format!("list_endpoints 参数无效：{e}")))?;
            let project = uuid::Uuid::parse_str(&a.project_id).map_err(|_| {
                AppError::Validation(format!("projectId 不是合法 UUID：{}", a.project_id))
            })?;
            let folder =
                match &a.folder_id {
                    Some(f) => Some(uuid::Uuid::parse_str(f).map_err(|_| {
                        AppError::Validation(format!("folderId 不是合法 UUID：{f}"))
                    })?),
                    None => None,
                };
            let mut path = format!("/agent/endpoints/{project}");
            let mut query = Vec::new();
            if let Some(n) = &a.name {
                query.push(format!("name={}", urlencoded(n)));
            }
            if let Some(m) = &a.method {
                query.push(format!("method={}", urlencoded(m)));
            }
            if let Some(f) = folder {
                query.push(format!("folderId={f}"));
            }
            if let Some(l) = a.limit {
                query.push(format!("limit={l}"));
            }
            if let Some(o) = a.offset {
                query.push(format!("offset={o}"));
            }
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query.join("&"));
            }
            client.get_raw_json(&path).await
        }
        "get_endpoint" => {
            let a: crate::args::GetEndpointArgs = serde_json::from_value(args.clone())
                .map_err(|e| AppError::Validation(format!("get_endpoint 参数无效：{e}")))?;
            let id = uuid::Uuid::parse_str(&a.endpoint_id).map_err(|_| {
                AppError::Validation(format!("endpointId 不是合法 UUID：{}", a.endpoint_id))
            })?;
            client
                .get_raw_json(&format!("/agent/endpoints/{id}/detail"))
                .await
        }
        "send_endpoint" => {
            let id = arg_uuid(args, "endpointId")?;
            client
                .post_raw_json(&format!("/agent/endpoints/{id}/execute"), args.clone())
                .await
        }
        "save_curl" => {
            let mut body = args.clone();
            // projectId 缺省时服务端自动选中唯一项目——body 原样透传。
            let _ = &mut body;
            client.post_raw_json("/agent/curl", body).await
        }

        // ---- 场景 ----
        "list_scenarios" => {
            let path = match opt_arg_uuid(args, "projectId")? {
                Some(id) => format!("/agent/scenarios?projectId={id}"),
                None => "/agent/scenarios".to_string(),
            };
            client.get_raw_json(&path).await
        }
        "get_scenario" => {
            let id = arg_uuid(args, "scenarioId")?;
            client.get_raw_json(&format!("/agent/scenarios/{id}")).await
        }
        "create_scenario" => client.post_raw_json("/agent/scenarios", args.clone()).await,
        "update_scenario" => {
            let id = arg_uuid(args, "scenarioId")?;
            client
                .put_raw_json(&format!("/agent/scenarios/{id}"), args.clone())
                .await
        }
        "delete_scenario" => {
            let id = arg_uuid(args, "scenarioId")?;
            client
                .delete_raw_json(&format!("/agent/scenarios/{id}"))
                .await
        }
        "add_scenario_step" => {
            let id = arg_uuid(args, "scenarioId")?;
            let step = args.get("step").cloned().unwrap_or(Value::Null);
            client
                .post_raw_json(&format!("/agent/scenarios/{id}/steps"), step)
                .await
        }
        "update_scenario_step" => {
            let id = arg_uuid(args, "scenarioId")?;
            let index = arg_usize(args, "index")?;
            let step = args.get("step").cloned().unwrap_or(Value::Null);
            client
                .put_raw_json(&format!("/agent/scenarios/{id}/steps/{index}"), step)
                .await
        }
        "delete_scenario_step" => {
            let id = arg_uuid(args, "scenarioId")?;
            let index = arg_usize(args, "index")?;
            client
                .delete_raw_json(&format!("/agent/scenarios/{id}/steps/{index}"))
                .await
        }
        "save_scenario_script" => {
            let id = arg_uuid(args, "scenarioId")?;
            let script = args.get("script").and_then(|v| v.as_str()).unwrap_or("");
            client
                .put_raw_json(
                    &format!("/agent/scenarios/{id}/script"),
                    json!({ "script": script }),
                )
                .await
        }
        "validate_scenario" => {
            let id = arg_uuid(args, "scenarioId")?;
            let mut path = format!("/agent/scenarios/{id}/validate");
            if let Some(env) = opt_arg_uuid(args, "environmentId")? {
                path.push_str(&format!("?environmentId={env}"));
            }
            client.get_raw_json(&path).await
        }

        // ---- 运行 ----
        "run_scenario" => {
            let id = arg_uuid(args, "scenarioId")?;
            client
                .post_raw_json(&format!("/agent/scenarios/{id}/run"), args.clone())
                .await
        }
        "get_run_result" => {
            let id = arg_uuid(args, "runId")?;
            client.get_raw_json(&format!("/agent/runs/{id}")).await
        }
        "get_run_step" => {
            let id = arg_uuid(args, "runId")?;
            let index = arg_usize(args, "stepIndex")?;
            client
                .get_raw_json(&format!("/agent/runs/{id}/steps/{index}"))
                .await
        }
        "analyze_run_failure" => {
            let id = arg_uuid(args, "runId")?;
            client
                .get_raw_json(&format!("/agent/runs/{id}/analysis"))
                .await
        }
        "cancel_run" => {
            let id = arg_uuid(args, "runId")?;
            client
                .post_raw_json(&format!("/agent/runs/{id}/cancel"), json!({}))
                .await
        }

        // ---- 环境 ----
        "list_environments" => {
            let path = match opt_arg_uuid(args, "projectId")? {
                Some(id) => format!("/agent/environments?projectId={id}"),
                None => "/agent/environments".to_string(),
            };
            client.get_raw_json(&path).await
        }
        "get_environment" => {
            let id = arg_uuid(args, "environmentId")?;
            client
                .get_raw_json(&format!("/agent/environments/{id}"))
                .await
        }

        // ---- 测试用例 ----
        "list_test_cases" => {
            let id = arg_uuid(args, "endpointId")?;
            client
                .get_raw_json(&format!("/agent/endpoints/{id}/test-cases"))
                .await
        }
        "get_test_case" => {
            let id = arg_uuid(args, "caseId")?;
            client
                .get_raw_json(&format!("/agent/test-cases/{id}"))
                .await
        }
        "create_test_case" => {
            client
                .post_raw_json("/agent/test-cases", args.clone())
                .await
        }
        "update_test_case" => {
            let id = arg_uuid(args, "caseId")?;
            client
                .put_raw_json(&format!("/agent/test-cases/{id}"), args.clone())
                .await
        }
        "delete_test_case" => {
            let id = arg_uuid(args, "caseId")?;
            client
                .delete_raw_json(&format!("/agent/test-cases/{id}"))
                .await
        }
        "run_test_case" => {
            let id = arg_uuid(args, "caseId")?;
            client
                .post_raw_json(&format!("/agent/test-cases/{id}/run"), args.clone())
                .await
        }
        "list_test_case_runs" => {
            let id = arg_uuid(args, "caseId")?;
            client
                .get_raw_json(&format!("/agent/test-cases/{id}/runs"))
                .await
        }
        "get_test_case_run" => {
            let id = arg_uuid(args, "runId")?;
            client
                .get_raw_json(&format!("/agent/test-case-runs/{id}"))
                .await
        }

        // ---- AI 测试中心 ----
        "analyze_api_dependencies" => {
            let path = match opt_arg_uuid(args, "projectId")? {
                Some(id) => format!("/agent/analyze/dependencies?projectId={id}"),
                None => "/agent/analyze/dependencies".to_string(),
            };
            client.get_raw_json(&path).await
        }
        "analyze_project_test_coverage" => {
            let path = match opt_arg_uuid(args, "projectId")? {
                Some(id) => format!("/agent/analyze/test-coverage?projectId={id}"),
                None => "/agent/analyze/test-coverage".to_string(),
            };
            client.get_raw_json(&path).await
        }
        "generate_test_cases" => {
            let id = arg_uuid(args, "endpointId")?;
            client
                .post_raw_json(
                    &format!("/agent/endpoints/{id}/test-case-drafts"),
                    json!({}),
                )
                .await
        }
        "generate_integration_scenario" => {
            client
                .post_raw_json("/agent/scenario-drafts", args.clone())
                .await
        }
        "propose_scenario_fix" => {
            let id = arg_uuid(args, "runId")?;
            client
                .post_raw_json(
                    &format!("/agent/runs/{id}/fix-proposal"),
                    json!({ "notes": args.get("notes").and_then(|v| v.as_str()) }),
                )
                .await
        }

        other => Err(AppError::Validation(format!("未注册的 MCP 工具：{other}"))),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn arg_uuid(args: &Value, key: &str) -> Result<Uuid> {
    let s =
        arg_str(args, key).ok_or_else(|| AppError::Validation(format!("缺少必填参数 {key}")))?;
    Uuid::parse_str(s).map_err(|_| AppError::Validation(format!("{key} 不是合法 UUID")))
}

fn opt_arg_uuid(args: &Value, key: &str) -> Result<Option<Uuid>> {
    match arg_str(args, key) {
        None => Ok(None),
        Some(s) => Uuid::parse_str(s)
            .map(Some)
            .map_err(|_| AppError::Validation(format!("{key} 不是合法 UUID"))),
    }
}

fn arg_usize(args: &Value, key: &str) -> Result<usize> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or_else(|| AppError::Validation(format!("缺少必填参数 {key}（非负整数）")))
}

fn urlencoded(s: &str) -> String {
    let mut out = String::new();
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
