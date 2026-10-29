//! 集成测试：启动 axum 测试服务，验证 Test Runner 执行闭环（SPEC §28.2.5）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, RequestSpec};
use miyo_test::config::{AssertionSpec, ExtractSpec};
use miyo_test::runner::{run_endpoint, run_scenario, ScenarioOptions, ScenarioStepInput};
use serde_json::json;
use uuid::Uuid;

fn make_ep(name: &str, method: HttpMethod, path: &str, tests: serde_json::Value) -> Endpoint {
    let request = RequestSpec {
        tests: Some(tests),
        ..Default::default()
    };
    Endpoint {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        folder_id: None,
        name: name.to_string(),
        method,
        path: path.to_string(),
        description: String::new(),
        status: EndpointStatus::Developing,
        sort_order: 0,
        request,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn spawn_server() -> SocketAddr {
    let app = Router::new()
        .route(
            "/users/{id}",
            get(
                |axum::extract::Path(id): axum::extract::Path<String>| async move {
                    axum::Json(json!({"id": id, "name": "miyo"}))
                },
            ),
        )
        .route("/echo", post(handle_echo))
        .route(
            "/login",
            post(|| async { axum::Json(json!({"token": "tok-9"})) }),
        )
        .route(
            "/slow",
            get(async || {
                tokio::time::sleep(Duration::from_millis(120)).await;
                axum::Json(json!({"ok": true}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn handle_echo(
    Query(q): Query<HashMap<String, String>>,
) -> (
    StatusCode,
    [(axum::http::HeaderName, String); 1],
    axum::Json<serde_json::Value>,
) {
    let env = q.get("env").cloned().unwrap_or_default();
    let uid = q.get("uid").cloned().unwrap_or_default();
    (
        StatusCode::CREATED,
        [(
            axum::http::HeaderName::from_static("x-miyo"),
            format!("{env}-{uid}"),
        )],
        axum::Json(json!({"ok": true, "env": env, "uid": uid})),
    )
}

#[tokio::test]
async fn runner_full_flow_with_variable_chain() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");
    let mut runtime: HashMap<String, String> = HashMap::new();

    // 接口 1：pre_request 注入变量 + extract userId + 断言。
    let ep1 = make_ep(
        "用户详情",
        HttpMethod::GET,
        "/users/:id",
        json!({
            "pre_request": [
                {"name": "stamp", "value": "{{$timestamp}}"}
            ],
            "extract": [
                {"name": "userId", "from": "body", "path": "$.id"}
            ],
            "assertions": [
                {"type": "status", "op": "eq", "expected": 200},
                {"type": "jsonpath", "path": "$.name", "op": "contains", "expected": "miyo"},
                {"type": "response_time_ms", "op": "lt", "expected": 2000},
                {"type": "jsonpath", "path": "$.id", "op": "exists"}
            ]
        }),
    );
    let url1 = format!("{base}/users/7");
    let (r1, _) = run_endpoint(&ep1, &url1, &ep1.request, &mut runtime, None).await;
    assert!(r1.ok, "接口 1 应通过：{r1:?}");
    assert_eq!(r1.status, Some(200));
    assert_eq!(
        runtime.get("userId"),
        Some(&"7".to_string()),
        "extract 应写入运行时变量"
    );
    assert!(
        runtime.get("stamp").map(|s| s.len()).unwrap_or(0) > 0,
        "pre_request {{$timestamp}} 应已解析"
    );

    // 接口 2：使用接口 1 提取的变量（模拟页面按运行时变量渲染 URL）。
    let ep2 = make_ep(
        "回显",
        HttpMethod::POST,
        "/echo",
        json!({
            "extract": [
                {"name": "xmiyo", "from": "header", "path": "x-miyo"}
            ],
            "assertions": [
                {"type": "status", "op": "eq", "expected": 201},
                {"type": "header", "path": "x-miyo", "op": "contains", "expected": "7"},
                {"type": "jsonpath", "path": "$.uid", "op": "eq", "expected": "{{userId}}"}
            ]
        }),
    );
    let url2 = format!(
        "{base}/echo?env=prod&uid={}",
        runtime.get("userId").cloned().unwrap_or_default()
    );
    let mut sp = ep2.request.clone();
    sp.tests = Some(
        miyo_test::TestSpec::from_request_value(ep2.request.tests.as_ref())
            .unwrap()
            .to_value(),
    );
    let _ = &mut sp;
    let (r2, _) = run_endpoint(&ep2, &url2, &ep2.request, &mut runtime, None).await;
    assert!(r2.ok, "接口 2 应通过：{r2:?}");
    assert_eq!(runtime.get("xmiyo"), Some(&"prod-7".to_string()));
}

#[tokio::test]
async fn runner_reports_assertion_failures() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");
    let ep = make_ep(
        "慢接口",
        HttpMethod::GET,
        "/slow",
        json!({"assertions": [
            {"type": "status", "op": "eq", "expected": 500},
            {"type": "response_time_ms", "op": "lt", "expected": 50}
        ]}),
    );
    let url = format!("{base}/slow");
    let mut runtime = HashMap::new();
    let (r, _) = run_endpoint(&ep, &url, &ep.request, &mut runtime, None).await;
    assert!(!r.ok);
    assert_eq!(r.status, Some(200));
    assert_eq!(r.outcomes.len(), 2);
    let failures: Vec<&str> = r
        .outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| o.reason.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(failures.len(), 2, "两个断言都应失败：{failures:?}");
}

#[tokio::test]
async fn runner_sends_request_and_extracts() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");
    // 无测试配置 → 跳过。
    let ep_none = make_ep(
        "无配置",
        HttpMethod::GET,
        "/users/1",
        serde_json::Value::Null,
    );
    let mut runtime = HashMap::new();
    let (r, _) = run_endpoint(
        &ep_none,
        &format!("{base}/users/1"),
        &ep_none.request,
        &mut runtime,
        None,
    )
    .await;
    assert!(r.ok);
    assert!(r.request_error.is_some());
    // 配置错误 → 快速失败。
    let ep_bad = make_ep(
        "坏配置",
        HttpMethod::GET,
        "/users/1",
        json!({"assertions": [{"type": 1}]}),
    );
    let (r2, _) = run_endpoint(
        &ep_bad,
        &format!("{base}/users/1"),
        &ep_bad.request,
        &mut runtime,
        None,
    )
    .await;
    assert!(!r2.ok);
    assert!(r2.request_error.is_some());
}

#[tokio::test]
async fn scenario_chain_flows_extracted_vars() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");

    // A：登录 → 提取 token；B：URL 模板用 {{token}}，echo 原样返回供断言。
    let steps = vec![
        ScenarioStepInput {
            endpoint: make_ep("登录", HttpMethod::POST, "/login", serde_json::Value::Null),
            url_template: format!("{base}/login"),
            spec_template: RequestSpec::default(),
            extract: vec![ExtractSpec {
                name: "token".into(),
                from: "body".into(),
                path: "$.token".into(),
            }],
            assertions: vec![AssertionSpec {
                name: None,
                r#type: "status".into(),
                path: None,
                op: Some("eq".into()),
                expected: Some(json!(200)),
            }],
        },
        ScenarioStepInput {
            endpoint: make_ep("回显", HttpMethod::POST, "/echo", serde_json::Value::Null),
            url_template: format!("{base}/echo?env={{{{token}}}}"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![AssertionSpec {
                name: None,
                r#type: "jsonpath".into(),
                path: Some("$.env".into()),
                op: Some("eq".into()),
                expected: Some(json!("tok-9")),
            }],
        },
    ];
    let mut base_vars = HashMap::new();
    base_vars.insert("static_v".to_string(), "s1".to_string());
    let result = run_scenario(
        steps,
        &base_vars,
        &ScenarioOptions {
            timeout_ms: None,
            cancel: None,
            stop_on_failure: true,
        },
        None,
    )
    .await;

    assert!(!result.cancelled);
    assert!(!result.stopped_on_failure, "{:?}", result.results);
    assert!(
        result.results.iter().all(|r| r.ok),
        "全部步骤应通过：{:?}",
        result.results
    );
    // A 步提取成果上报 + 进入 runtime。
    assert_eq!(
        result.results[0].extracted.get("token").map(String::as_str),
        Some("tok-9")
    );
    assert_eq!(
        result.variables.get("token").map(String::as_str),
        Some("tok-9")
    );
    // 环境静态变量作为兜底仍在变量池中。
    assert_eq!(
        result.variables.get("static_v").map(String::as_str),
        Some("s1")
    );
}

#[tokio::test]
async fn scenario_stops_on_failure() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");

    // 第一步断言失败（期望 201 实际 200）+ 失败即停 → 第二步不执行。
    let steps = vec![
        ScenarioStepInput {
            endpoint: make_ep("登录", HttpMethod::POST, "/login", serde_json::Value::Null),
            url_template: format!("{base}/login"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![AssertionSpec {
                name: None,
                r#type: "status".into(),
                path: None,
                op: Some("eq".into()),
                expected: Some(json!(201)),
            }],
        },
        ScenarioStepInput {
            endpoint: make_ep("回显", HttpMethod::GET, "/echo", serde_json::Value::Null),
            url_template: format!("{base}/echo"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![],
        },
    ];
    let result = run_scenario(
        steps,
        &HashMap::new(),
        &ScenarioOptions {
            timeout_ms: None,
            cancel: None,
            stop_on_failure: true,
        },
        None,
    )
    .await;

    assert!(result.stopped_on_failure);
    assert_eq!(result.results.len(), 1, "失败即停：第二步不应执行");
    assert!(!result.results[0].ok);
}
