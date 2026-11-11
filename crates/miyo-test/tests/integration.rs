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
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
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
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
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
    // 每步执行前的 runtime 快照（重跑失败步骤的初始变量来源）：
    // 第 1 步前只有静态变量；第 2 步前已含第 1 步的提取值。
    assert_eq!(result.vars_snapshots.len(), 2);
    assert_eq!(
        result.vars_snapshots[0].get("static_v").map(String::as_str),
        Some("s1")
    );
    assert!(!result.vars_snapshots[0].contains_key("token"));
    assert_eq!(
        result.vars_snapshots[1].get("token").map(String::as_str),
        Some("tok-9")
    );
}

#[tokio::test]
async fn scenario_stops_on_failure() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");

    // 第一步断言失败（期望 201 实际 200）+ 失败即停 → 第二步不执行。
    let steps = vec![
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
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
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
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
    assert_eq!(
        result.results.len(),
        2,
        "失败即停：第二步落 skipped 行不执行"
    );
    assert!(!result.results[0].ok);
    assert_eq!(result.results[1].effective_status(), "skipped");
    assert_eq!(result.results[1].url, None, "skipped 步不发送");
}

// ---------- 集成测试工作流（2026-11）：六态状态 / Blocked / 类型保留 / scheme ----------

/// 电商三步服务：下单 → 预览（校验下单传来的参数）→ 支付（校验预览金额）。
async fn spawn_shop_server() -> SocketAddr {
    use axum::Json;
    let app = Router::new()
        .route(
            "/shop/orders",
            post(|| async {
                Json(json!({
                    "orderNo": "SO-1",
                    "payToken": "pt-9",
                    "amount": "398.00"
                }))
            }),
        )
        .route(
            "/shop/pay-preview",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                // 校验链路传参：orderNo / payToken 来自第 1 步。
                let ok = q.get("orderNo").map(|v| v == "SO-1").unwrap_or(false)
                    && q.get("payToken").map(|v| v == "pt-9").unwrap_or(false);
                if ok {
                    (
                        StatusCode::OK,
                        axum::Json(json!({"payableAmount": 388.2, "currency": "CNY"})),
                    )
                } else {
                    (
                        StatusCode::BAD_REQUEST,
                        axum::Json(json!({"error": "missing chain params"})),
                    )
                }
            }),
        )
        .route(
            "/shop/pay",
            post(|body: String| async move {
                // 校验 Body 里 amount 是数字（类型保留）且带链路参数。
                let v: serde_json::Value =
                    serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                let amount_is_number =
                    matches!(v.get("amount"), Some(serde_json::Value::Number(_)));
                let has_token = v
                    .get("payToken")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "pt-9")
                    .unwrap_or(false);
                if amount_is_number && has_token {
                    (
                        StatusCode::OK,
                        Json(json!({"status": "PAID", "tradeNo": "ALI-1"})),
                    )
                } else {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "bad pay body"})),
                    )
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn extract(name: &str, path: &str) -> ExtractSpec {
    ExtractSpec {
        name: name.into(),
        from: "body".into(),
        path: path.into(),
    }
}

fn assert_status(code: i64) -> AssertionSpec {
    AssertionSpec {
        name: None,
        r#type: "status".into(),
        path: None,
        op: Some("eq".into()),
        expected: Some(json!(code)),
    }
}

/// 验收主链路：三步电商流程，全程无需手动传值，金额保持数字类型。
#[tokio::test]
async fn scenario_ecommerce_flow_with_typed_vars() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let steps = vec![
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep("创建订单", HttpMethod::POST, "/shop/orders", serde_json::Value::Null),
            url_template: format!("{base}/shop/orders"),
            spec_template: RequestSpec::default(),
            extract: vec![extract("orderNo", "$.orderNo"), extract("payToken", "$.payToken")],
            assertions: vec![assert_status(200)],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep("支付预览", HttpMethod::GET, "/shop/pay-preview", serde_json::Value::Null),
            url_template: format!("{base}/shop/pay-preview?orderNo={{{{orderNo}}}}&payToken={{{{payToken}}}}"),
            spec_template: RequestSpec::default(),
            extract: vec![extract("payableAmount", "$.payableAmount")],
            assertions: vec![assert_status(200)],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep("确认支付", HttpMethod::POST, "/shop/pay", serde_json::Value::Null),
            url_template: format!("{base}/shop/pay"),
            spec_template: RequestSpec {
                body: miyo_core::model::BodySpec::Json {
                    raw: r#"{"amount": "{{payableAmount}}", "orderNo": "{{orderNo}}", "payToken": "{{payToken}}"}"#
                        .into(),
                },
                ..Default::default()
            },
            extract: vec![extract("tradeNo", "$.tradeNo")],
            assertions: vec![
                assert_status(200),
                AssertionSpec {
                    name: None,
                    r#type: "jsonpath".into(),
                    path: Some("$.status".into()),
                    op: Some("eq".into()),
                    expected: Some(json!("PAID")),
                },
            ],
        },
    ];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    assert!(!result.cancelled);
    assert!(!result.stopped_on_failure);
    assert_eq!(result.results.len(), 3);
    assert!(
        result.results.iter().all(|r| r.ok),
        "三步应全过：{:?}",
        result.results
    );
    for r in &result.results {
        assert_eq!(r.effective_status(), "success");
    }
    // 变量链路：orderNo / payToken / payableAmount / tradeNo 全部落池。
    for (k, v) in [
        ("orderNo", "SO-1"),
        ("payToken", "pt-9"),
        ("payableAmount", "388.2"),
        ("tradeNo", "ALI-1"),
    ] {
        assert_eq!(
            result.variables.get(k).map(String::as_str),
            Some(v),
            "变量 {k} 应落池"
        );
    }
    // 第三步实际发出的 Body（RequestSnapshot 快照）：amount 是数字（类型保留），
    // orderNo / payToken 已代入。
    let body = result.results[2]
        .response
        .as_ref()
        .expect("第 3 步应有响应")
        .request
        .as_ref()
        .expect("应有请求快照")
        .body
        .clone();
    let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(sent["amount"].is_number(), "amount 应保持数字：{sent}");
    assert_eq!(sent["orderNo"], json!("SO-1"));
    assert_eq!(sent["payToken"], json!("pt-9"));
    // 运行间隔离：fresh base_vars 不含上次残留（每次运行独立上下文由调用方保证，
    // 这里断言 warnings 为空——链路变量都有来源）。
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

/// Blocked：第 2 步失败后，第 3 步引用其产出变量 → 不发送、标记 blocked。
#[tokio::test]
async fn scenario_blocks_on_missing_upstream_var() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let steps = vec![
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "创建订单",
                HttpMethod::POST,
                "/shop/orders",
                serde_json::Value::Null,
            ),
            url_template: format!("{base}/shop/orders"),
            spec_template: RequestSpec::default(),
            extract: vec![extract("orderNo", "$.orderNo")],
            assertions: vec![assert_status(200)],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "支付预览",
                HttpMethod::GET,
                "/shop/pay-preview",
                serde_json::Value::Null,
            ),
            // 缺 orderNo 参数 → 服务端 400 → 本步断言失败。
            url_template: format!("{base}/shop/pay-preview?payToken=bad"),
            spec_template: RequestSpec::default(),
            extract: vec![extract("payableAmount", "$.payableAmount")],
            assertions: vec![assert_status(200)],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "确认支付",
                HttpMethod::POST,
                "/shop/pay",
                serde_json::Value::Null,
            ),
            url_template: format!("{base}/shop/pay"),
            spec_template: RequestSpec {
                body: miyo_core::model::BodySpec::Json {
                    raw: r#"{"amount": "{{payableAmount}}", "payToken": "{{payToken}}"}"#.into(),
                },
                ..Default::default()
            },
            extract: vec![],
            assertions: vec![],
        },
    ];
    // 失败继续：第 3 步依赖 payableAmount / payToken（第 2 步失败未产出）→
    // Blocked 而非用空变量发送。payToken 虽然第 1 步产出过，但第 2 步也失败
    // （其 extract 不含 payToken，不在丢失集合），仍由第 1 步成功提供。
    let result = run_scenario(
        steps,
        &HashMap::new(),
        &ScenarioOptions {
            stop_on_failure: false,
            ..Default::default()
        },
        None,
    )
    .await;

    assert_eq!(result.results.len(), 3, "失败继续：三步都应有结果行");
    assert_eq!(result.results[1].effective_status(), "failed");
    let third = &result.results[2];
    assert_eq!(third.effective_status(), "blocked", "第 3 步应被 Blocked");
    assert!(third.blocked_reason.is_some());
    assert!(third.url.is_none(), "Blocked 步不应发送请求");
    assert!(
        third
            .blocked_reason
            .as_deref()
            .unwrap_or("")
            .contains("payableAmount"),
        "Blocked 原因应点名缺失变量：{:?}",
        third.blocked_reason
    );
}

/// 禁用步骤 → skipped 行；失败即停后剩余步骤 → skipped 行（不丢步骤）。
#[tokio::test]
async fn scenario_reports_disabled_and_early_stop_as_skipped() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let steps = vec![
        ScenarioStepInput {
            enabled: false,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "禁用步",
                HttpMethod::GET,
                "/shop/pay-preview",
                serde_json::Value::Null,
            ),
            url_template: format!("{base}/shop/pay-preview"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "失败步",
                HttpMethod::POST,
                "/shop/pay-preview",
                serde_json::Value::Null,
            ),
            // GET 服务的路由只挂了 POST /shop/orders 与 /shop/pay —— 405 → 失败。
            url_template: format!("{base}/shop/pay-preview"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![assert_status(200)],
        },
        ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "后续步",
                HttpMethod::GET,
                "/shop/pay-preview",
                serde_json::Value::Null,
            ),
            url_template: format!("{base}/shop/pay-preview"),
            spec_template: RequestSpec::default(),
            extract: vec![],
            assertions: vec![],
        },
    ];
    let result = run_scenario(
        steps,
        &HashMap::new(),
        &ScenarioOptions {
            stop_on_failure: true,
            ..Default::default()
        },
        None,
    )
    .await;

    assert!(result.stopped_on_failure);
    assert_eq!(result.results.len(), 3, "三步都应有结果行");
    assert_eq!(result.results[0].effective_status(), "skipped");
    assert_eq!(result.results[1].effective_status(), "failed");
    assert_eq!(result.results[2].effective_status(), "skipped");
    assert_eq!(result.results[0].url, None, "禁用步不发送");
}

/// scheme 白名单：变量替换出非 http(s) URL → 该步 failed_fast 不发送。
#[tokio::test]
async fn scenario_rejects_non_http_scheme() {
    let _addr = spawn_shop_server().await;
    let steps = vec![ScenarioStepInput {
        enabled: true,
        condition: None,
        r#loop: None,
        wait_ms: None,
        parallel: false,
        endpoint: make_ep("坏协议", HttpMethod::GET, "/x", serde_json::Value::Null),
        url_template: "file:///etc/passwd".to_string(),
        spec_template: RequestSpec::default(),
        extract: vec![],
        assertions: vec![],
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;
    assert_eq!(result.results.len(), 1);
    let r = &result.results[0];
    assert_eq!(r.effective_status(), "failed");
    assert!(r.request_error.as_deref().unwrap_or("").contains("http"));
    assert!(r.response.is_none(), "scheme 非法不应发送");
    assert!(r.url.is_some(), "失败行保留解析后 URL 供排查");
}

/// 未定义变量预检：warnings 点名步骤与变量；发送时字面量保留。
#[tokio::test]
async fn scenario_warns_undefined_vars() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let steps = vec![ScenarioStepInput {
        enabled: true,
        condition: None,
        r#loop: None,
        wait_ms: None,
        parallel: false,
        endpoint: make_ep(
            "引用缺失变量",
            HttpMethod::GET,
            "/shop/pay-preview",
            serde_json::Value::Null,
        ),
        url_template: format!("{base}/shop/pay-preview?orderNo={{{{nope}}}}"),
        spec_template: RequestSpec::default(),
        extract: vec![],
        assertions: vec![],
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(result.warnings[0].contains("nope"));
    // 字面量保留：实际 URL 仍带 {{nope}}。
    let r = &result.results[0];
    assert!(r.url.as_deref().unwrap_or("").contains("{{nope}}"));
}

/// 运行间变量隔离：同一批步骤跑两轮，第二轮的变量池不残留第一轮提取值。
#[tokio::test]
async fn scenario_runs_have_isolated_contexts() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let mk_steps = || {
        vec![ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                "创建订单",
                HttpMethod::POST,
                "/shop/orders",
                serde_json::Value::Null,
            ),
            url_template: format!("{base}/shop/orders"),
            spec_template: RequestSpec::default(),
            extract: vec![extract("orderNo", "$.orderNo")],
            assertions: vec![],
        }]
    };
    let r1 = run_scenario(
        mk_steps(),
        &HashMap::new(),
        &ScenarioOptions::default(),
        None,
    )
    .await;
    assert_eq!(
        r1.variables.get("orderNo").map(String::as_str),
        Some("SO-1")
    );
    // 第二轮：全新 base_vars（等价新执行上下文），不应看到第一轮的 orderNo
    //（除非本步重新提取——本步确实重新提取并覆盖，值为相同；
    // 这里断言快照语义：第 1 步执行前快照只含 base_vars）。
    let r2 = run_scenario(
        mk_steps(),
        &HashMap::new(),
        &ScenarioOptions::default(),
        None,
    )
    .await;
    assert_eq!(r2.vars_snapshots[0], HashMap::new(), "新运行初始快照应为空");
    assert_eq!(
        r2.variables.get("orderNo").map(String::as_str),
        Some("SO-1")
    );
}

// ---------- 阶段三：条件 / 循环 / 等待 / 并行 ----------

use miyo_test::config::{StepCondition, StepLoop};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

fn var_cond(name: &str, op: &str, value: serde_json::Value) -> StepCondition {
    StepCondition {
        source: "var".into(),
        name: name.into(),
        expression: None,
        op: op.into(),
        value: Some(value),
    }
}

/// 条件不满足 → skipped（不发送、不触发失败即停）；满足 → 正常执行。
#[tokio::test]
async fn scenario_condition_skips_step() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    // 变量条件基于存在的变量求值（缺失变量除 exists/empty 族外均不满足）。
    let mut base_vars = HashMap::new();
    base_vars.insert("paid".to_string(), "false".to_string());
    let steps = vec![
        ScenarioStepInput {
            enabled: true,
            condition: Some(var_cond("paid", "eq", json!("true"))),
            ..mk_get(format!("{base}/shop/pay-preview"))
        },
        ScenarioStepInput {
            enabled: true,
            condition: Some(var_cond("paid", "neq", json!("true"))),
            ..mk_get(format!(
                "{base}/shop/pay-preview?orderNo=SO-1&payToken=pt-9"
            ))
        },
        ScenarioStepInput {
            enabled: true,
            condition: Some(StepCondition {
                source: "prevStatus".into(),
                name: String::new(),
                expression: None,
                op: "eq".into(),
                value: Some(json!(200)),
            }),
            ..mk_get(format!("{base}/shop/pay-preview"))
        },
    ];
    let result = run_scenario(steps, &base_vars, &ScenarioOptions::default(), None).await;

    assert_eq!(result.results.len(), 3);
    // 第 1 步：paid 未定义 → 条件不满足 → skipped（不算失败）。
    assert_eq!(result.results[0].effective_status(), "skipped");
    assert!(result.results[0].url.is_none());
    assert!(result.results[0]
        .request_error
        .as_deref()
        .unwrap_or_default()
        .contains("条件不满足"));
    // 第 2 步：neq true 成立（paid 缺失 ≠ "true"）→ 执行成功。
    assert_eq!(result.results[1].effective_status(), "success");
    // 第 3 步：prevStatus 取第 2 步的 200 → 执行。
    assert_eq!(result.results[2].effective_status(), "success");
    // skipped 不算失败：stopped_on_failure=false（默认本就 false；再验 fail 计数语义由调用方算）。
    assert!(!result.stopped_on_failure);
}

fn mk_get(url: String) -> ScenarioStepInput {
    ScenarioStepInput {
        endpoint: make_ep("步", HttpMethod::GET, "/x", serde_json::Value::Null),
        url_template: url,
        spec_template: RequestSpec::default(),
        extract: vec![],
        assertions: vec![],
        enabled: true,
        condition: None,
        r#loop: None,
        wait_ms: None,
        parallel: false,
    }
}

/// count 循环：跑满 N 轮、结果聚合（loop_runs = N、耗时合计）。
#[tokio::test]
async fn scenario_count_loop_runs_n_rounds() {
    // 有状态上游：每次 ping 计数 +1，返回当前计数。
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let app = Router::new().route(
        "/ping",
        get(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                axum::Json(json!({"n": n}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let steps = vec![ScenarioStepInput {
        enabled: true,
        r#loop: Some(StepLoop {
            mode: "count".into(),
            count: Some(3),
            while_condition: None,
            max_loops: None,
            interval_ms: None,
        }),
        extract: vec![extract("n", "$.n")],
        assertions: vec![assert_status(200)],
        ..mk_get(format!("{base}/ping"))
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    assert_eq!(result.results.len(), 1);
    let r = &result.results[0];
    assert!(r.ok, "{r:?}");
    assert_eq!(r.loop_runs, Some(3), "应跑满 3 轮");
    // 每轮重新渲染：最后一轮提取的 n = 3（前两轮被覆盖）。
    assert_eq!(r.extracted.get("n").map(String::as_str), Some("3"));
    assert_eq!(counter.load(Ordering::SeqCst), 3, "上游应被命中 3 次");
}

/// while 循环轮询：上游第 3 次起 paid=true，条件成立退出（成功）。
#[tokio::test]
async fn scenario_while_loop_polls_until_condition() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let app = Router::new().route(
        "/status",
        get(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                axum::Json(json!({"paid": n >= 3}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let steps = vec![ScenarioStepInput {
        enabled: true,
        r#loop: Some(StepLoop {
            mode: "while".into(),
            count: None,
            while_condition: Some(var_cond("paid", "neq", json!("true"))),
            max_loops: Some(10),
            interval_ms: None,
        }),
        extract: vec![extract("paid", "$.paid")],
        assertions: vec![assert_status(200)],
        ..mk_get(format!("{base}/status"))
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    let r = &result.results[0];
    assert!(r.ok, "轮询到条件成立应成功：{r:?}");
    assert_eq!(r.loop_runs, Some(3), "第 3 轮 paid=true 后退出");
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

/// while 循环达上限条件仍成立 → 整步 failed（轮询超时语义）。
#[tokio::test]
async fn scenario_while_loop_max_reached_fails() {
    let app = Router::new().route(
        "/status",
        get(|| async { axum::Json(json!({"paid": false})) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let steps = vec![ScenarioStepInput {
        enabled: true,
        r#loop: Some(StepLoop {
            mode: "while".into(),
            count: None,
            // 恒为「继续」的条件：达上限仍成立 → 轮询超时语义的失败。
            while_condition: Some(var_cond("paid", "neq", json!("true"))),
            max_loops: Some(3),
            interval_ms: None,
        }),
        extract: vec![extract("paid", "$.paid")],
        assertions: vec![assert_status(200)],
        ..mk_get(format!("{base}/status"))
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    let r = &result.results[0];
    assert!(!r.ok);
    assert_eq!(r.effective_status(), "failed");
    assert!(r
        .request_error
        .as_deref()
        .unwrap_or_default()
        .contains("循环条件"));
    assert_eq!(r.loop_runs, Some(3));
}

/// while 响应源（issue #156）：while_condition 直接引用本轮响应体
/// JSONPath，无需先 extract 再引用 var。上游第 3 次起 paid=true。
#[tokio::test]
async fn scenario_while_loop_response_body_condition() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let app = Router::new().route(
        "/status",
        get(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                axum::Json(json!({"paid": n >= 3}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let wc: StepCondition = serde_json::from_value(json!({
        "source": "response.body", "expression": "$.paid", "op": "neq", "value": true
    }))
    .unwrap();
    let steps = vec![ScenarioStepInput {
        enabled: true,
        r#loop: Some(StepLoop {
            mode: "while".into(),
            count: None,
            while_condition: Some(wc),
            max_loops: Some(10),
            interval_ms: None,
        }),
        assertions: vec![assert_status(200)],
        ..mk_get(format!("{base}/status"))
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    let r = &result.results[0];
    assert!(r.ok, "响应体条件轮询应成功：{r:?}");
    assert_eq!(r.loop_runs, Some(3), "第 3 轮 $.paid=true 后退出");
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

/// while 响应源：response.status 直接对状态码求值（首轮即满足 → 1 轮退出）。
#[tokio::test]
async fn scenario_while_loop_response_status_condition() {
    let app = Router::new().route("/ok", get(|| async { axum::Json(json!({"ok": true})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let wc: StepCondition =
        serde_json::from_value(json!({"source": "response.status", "op": "neq", "value": 200}))
            .unwrap();
    let steps = vec![ScenarioStepInput {
        enabled: true,
        r#loop: Some(StepLoop {
            mode: "while".into(),
            count: None,
            while_condition: Some(wc),
            max_loops: Some(10),
            interval_ms: None,
        }),
        assertions: vec![assert_status(200)],
        ..mk_get(format!("{base}/ok"))
    }];
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;

    let r = &result.results[0];
    assert!(r.ok, "{r:?}");
    // 首轮 200 即不满足继续条件：1 轮退出；loop_runs 仅在 >1 轮时记录
    assert_eq!(r.loop_runs, None);
}

/// wait_ms：执行前等待（限速语义），单步耗时包含等待。
#[tokio::test]
async fn scenario_wait_ms_sleeps_before_send() {
    let addr = spawn_shop_server().await;
    let base = format!("http://{addr}");
    let steps = vec![ScenarioStepInput {
        enabled: true,
        wait_ms: Some(120),
        ..mk_get(format!("{base}/shop/pay-preview"))
    }];
    let started = std::time::Instant::now();
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;
    let elapsed = started.elapsed().as_millis();
    assert_eq!(result.results[0].effective_status(), "success");
    assert!(elapsed >= 100, "应至少等待 wait_ms：{elapsed}ms");
    assert!(
        result.results[0].duration_ms.unwrap_or(0.0) >= 100.0,
        "步骤耗时含等待：{:?}",
        result.results[0].duration_ms
    );
}

/// 并行批：两个慢步骤并发执行（总耗时 ≈ 单步而非两步之和）。
#[tokio::test]
async fn scenario_parallel_batch_runs_concurrently() {
    let app = Router::new()
        .route(
            "/slow-a",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(120)).await;
                axum::Json(json!({"v": "A"}))
            }),
        )
        .route(
            "/slow-b",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(120)).await;
                axum::Json(json!({"v": "B"}))
            }),
        )
        .route("/fast", get(|| async { axum::Json(json!({"v": "F"})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let steps = vec![
        ScenarioStepInput {
            enabled: true,
            parallel: true,
            extract: vec![extract("va", "$.v")],
            ..mk_get(format!("{base}/slow-a"))
        },
        ScenarioStepInput {
            enabled: true,
            parallel: true,
            extract: vec![extract("vb", "$.v")],
            ..mk_get(format!("{base}/slow-b"))
        },
        // 非并行步骤收尾（串行依赖语义不变）。
        ScenarioStepInput {
            enabled: true,
            extract: vec![extract("vf", "$.v")],
            ..mk_get(format!("{base}/fast"))
        },
    ];
    let started = std::time::Instant::now();
    let result = run_scenario(steps, &HashMap::new(), &ScenarioOptions::default(), None).await;
    let elapsed = started.elapsed().as_millis();

    assert_eq!(result.results.len(), 3);
    assert!(result.results.iter().all(|r| r.ok), "{:?}", result.results);
    assert!(
        elapsed < 220,
        "两个 120ms 慢步应并发（<220ms），实际 {elapsed}ms"
    );
    // 结果按原位次排列；两步提取值都进变量池。
    assert_eq!(
        result.results[0].extracted.get("va").map(String::as_str),
        Some("A")
    );
    assert_eq!(
        result.results[1].extracted.get("vb").map(String::as_str),
        Some("B")
    );
    assert_eq!(result.variables.get("vf").map(String::as_str), Some("F"));
}

/// 性能冒烟：100 步 × 大变量池（500 变量）+ 每步提取 1 个变量。
/// 锁定借用化重构后的量级（秒级而非分钟级）；不做精确计时断言，
/// 只防 O(n²) 级退化（如恢复「每步整表克隆 × 渲染快照」会显著变慢）。
#[tokio::test]
async fn perf_smoke_100_steps_large_var_pool() {
    let addr = spawn_server().await;
    let base = format!("http://{addr}");

    // 大变量池：500 个静态变量。
    let base_vars: HashMap<String, String> = (0..500)
        .map(|i| (format!("static_{i}"), format!("value-{i}")))
        .collect();

    // 100 步：每步 POST /echo，提取 1 个链路变量（变量池逐步增长）。
    let mut steps = Vec::with_capacity(100);
    for i in 0u32..100 {
        steps.push(ScenarioStepInput {
            enabled: true,
            condition: None,
            r#loop: None,
            wait_ms: None,
            parallel: false,
            endpoint: make_ep(
                &format!("步{i}"),
                HttpMethod::POST,
                "/echo",
                serde_json::Value::Null,
            ),
            url_template: if i == 0 {
                format!("{base}/echo?step={i}&v0={{{{static_0}}}}")
            } else {
                format!("{base}/echo?step={i}&env={{{{chain_{}}}}}", i - 1)
            },
            spec_template: RequestSpec::default(),
            extract: vec![ExtractSpec {
                name: format!("chain_{i}"),
                from: "body".into(),
                path: "$.env".into(),
            }],
            assertions: vec![AssertionSpec {
                name: None,
                r#type: "status".into(),
                path: None,
                op: Some("eq".into()),
                expected: Some(json!(201)), // /echo 返回 CREATED
            }],
        });
    }

    let options = ScenarioOptions {
        timeout_ms: Some(10_000),
        cancel: None,
        stop_on_failure: true,
    };
    let started = std::time::Instant::now();
    let result = run_scenario(steps, &base_vars, &options, None).await;
    let elapsed = started.elapsed();

    assert_eq!(result.results.len(), 100, "全部步骤应有结果行");
    assert!(
        result.results.iter().filter(|r| r.ok).count() == 100,
        "全部步骤应通过（链路变量逐层提取）"
    );
    // 链路变量逐层提取成功（最后一步的 chain_99 已入池）。
    assert!(
        result.variables.contains_key("chain_99"),
        "链路变量应逐层入池"
    );
    // 快照与结果同位序（借用化重构不得破坏该契约）。
    assert_eq!(result.vars_snapshots.len(), 100);
    // 宽松上界（CI 慢机余量）：本地串行 100 次真实 HTTP 往返 + 500 变量池渲染
    // 通常 << 5s；若借用化回退为整表深拷贝且变量池继续增长，会显著逼近该界。
    assert!(
        elapsed.as_secs() < 30,
        "100 步大变量池耗时异常：{elapsed:?}"
    );
}
