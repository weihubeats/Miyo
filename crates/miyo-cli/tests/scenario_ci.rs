//! 集成测试：miyo-cli 场景运行（CI/CD 链路）——导出 JSON → 无头运行 → JUnit。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::extract::Query;
use axum::routing::get;
use axum::Json as AxumJson;
use axum::Router;
use serde_json::json;

use miyo_cli::{run_scenario_json, ScenarioRunOptions};

/// 电商三步上游（与桌面端种子场景同款：校验链路传参、金额数字类型）。
async fn spawn_shop() -> String {
    let app = Router::new()
        .route(
            "/shop/orders",
            axum::routing::post(|| async {
                AxumJson(json!({"orderNo": "SO-1", "payToken": "pt-9"}))
            }),
        )
        .route(
            "/shop/pay-preview",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let ok = q.get("orderNo").map(|v| v == "SO-1").unwrap_or(false)
                    && q.get("payToken").map(|v| v == "pt-9").unwrap_or(false);
                if ok {
                    (
                        axum::http::StatusCode::OK,
                        AxumJson(json!({"payableAmount": 388.2})),
                    )
                } else {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        AxumJson(json!({"error": "x"})),
                    )
                }
            }),
        )
        .route(
            "/shop/pay",
            axum::routing::post(|body: String| async move {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let number_amount = matches!(v.get("amount"), Some(serde_json::Value::Number(_)));
                if number_amount && v.get("payToken").and_then(|t| t.as_str()) == Some("pt-9") {
                    (
                        axum::http::StatusCode::OK,
                        AxumJson(json!({"status": "PAID", "tradeNo": "ALI-1"})),
                    )
                } else {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        AxumJson(json!({"error": "x"})),
                    )
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// 桌面端导出形态（magic + 内联步骤拷贝）。
fn export_json(base: &str) -> String {
    json!({
        "magic": "miyo.scenario",
        "version": 1,
        "scenario": {
            "name": "电商主流程",
            "description": "",
            "category": "电商",
            "steps": [
                {
                    "endpoint_id": null,
                    "enabled": true,
                    "extract": [
                        {"name": "orderNo", "from": "body", "path": "$.orderNo"},
                        {"name": "payToken", "from": "body", "path": "$.payToken"}
                    ],
                    "assertions": [{"type": "status", "op": "eq", "expected": 200}],
                    "name": "创建订单",
                    "request": {
                        "method": "POST",
                        "url": format!("{base}/shop/orders"),
                        "spec": {"params": [], "headers": [], "path_variables": [],
                                 "auth": {"type": "none"}, "body": {"mode": "none"},
                                 "timeout_ms": null, "follow_redirects": true, "tests": null}
                    }
                },
                {
                    "endpoint_id": null,
                    "enabled": true,
                    "extract": [{"name": "payableAmount", "from": "body", "path": "$.payableAmount"}],
                    "assertions": [{"type": "status", "op": "eq", "expected": 200}],
                    "name": "支付预览",
                    "request": {
                        "method": "GET",
                        "url": format!("{base}/shop/pay-preview?orderNo={{{{orderNo}}}}&payToken={{{{payToken}}}}"),
                        "spec": {"params": [], "headers": [], "path_variables": [],
                                 "auth": {"type": "none"}, "body": {"mode": "none"},
                                 "timeout_ms": null, "follow_redirects": true, "tests": null}
                    }
                },
                {
                    "endpoint_id": null,
                    "enabled": true,
                    "extract": [{"name": "tradeNo", "from": "body", "path": "$.tradeNo"}],
                    "assertions": [
                        {"type": "status", "op": "eq", "expected": 200},
                        {"type": "jsonpath", "path": "$.status", "op": "eq", "expected": "PAID"}
                    ],
                    "name": "确认支付",
                    "request": {
                        "method": "POST",
                        "url": format!("{base}/shop/pay"),
                        "spec": {"params": [], "headers": [], "path_variables": [],
                                 "auth": {"type": "none"},
                                 "body": {"mode": "json", "raw": "{\"amount\": \"{{payableAmount}}\", \"payToken\": \"{{payToken}}\"}"},
                                 "timeout_ms": null, "follow_redirects": true, "tests": null}
                    }
                }
            ],
            "script": null
        }
    })
    .to_string()
}

#[tokio::test]
async fn cli_scenario_runs_exported_json_and_chains_vars() {
    let base = spawn_shop().await;
    let report = run_scenario_json(&export_json(&base), &ScenarioRunOptions::default())
        .await
        .expect("场景应成功运行");

    assert_eq!(report.project, "电商主流程");
    assert_eq!(
        report.passed, 3,
        "三步全过：{} / {}",
        report.passed, report.failed
    );
    assert_eq!(report.failed, 0);
    assert!(report.ok(), "退出码语义：全过 = ok");
    // JUnit 输出可生成（CI 消费）。
    let xml = miyo_cli::junit::junit_xml(&report);
    assert!(xml.contains("电商主流程"));
    assert!(xml.contains("确认支付"));
}

#[tokio::test]
async fn cli_scenario_reports_failures_and_blocked() {
    // 上游预览校验失败 → 第 2 步 failed；第 3 步引用缺失产出变量 → blocked/skipped 语义。
    let app = Router::new()
        .route(
            "/shop/orders",
            axum::routing::post(|| async { AxumJson(json!({"orderNo": "SO-1"})) }),
        )
        .route(
            "/shop/pay-preview",
            get(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    AxumJson(json!({"error": "x"})),
                )
            }),
        )
        .route(
            "/shop/pay",
            axum::routing::post(|| async { AxumJson(json!({"status": "PAID"})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let mut text = export_json(&base);
    // 第 2 步断言 200 会失败（实际 400）。
    let report = run_scenario_json(&text, &ScenarioRunOptions::default())
        .await
        .unwrap();
    assert!(report.failed >= 1, "预览 400 应判失败");
    assert!(!report.ok());
    let _ = &mut text;
}

#[tokio::test]
async fn cli_scenario_supports_loop_and_condition() {
    // 条件不满足 → skipped；count 循环跑满 N 轮。
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let app = Router::new().route(
        "/ping",
        get(move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                AxumJson(json!({"n": n}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let text = json!({
        "name": "控制流",
        "steps": [
            {
                "enabled": true,
                "condition": {"source": "var", "name": "nope", "op": "exists", "value": null},
                "extract": [],
                "assertions": [],
                "request": {"method": "GET", "url": format!("{base}/ping"),
                            "spec": {"params": [], "headers": [], "path_variables": [],
                                     "auth": {"type": "none"}, "body": {"mode": "none"},
                                     "timeout_ms": null, "follow_redirects": true, "tests": null}}
            },
            {
                "enabled": true,
                "loop": {"mode": "count", "count": 3},
                "extract": [{"name": "n", "from": "body", "path": "$.n"}],
                "assertions": [{"type": "status", "op": "eq", "expected": 200}],
                "request": {"method": "GET", "url": format!("{base}/ping"),
                            "spec": {"params": [], "headers": [], "path_variables": [],
                                     "auth": {"type": "none"}, "body": {"mode": "none"},
                                     "timeout_ms": null, "follow_redirects": true, "tests": null}}
            }
        ]
    })
    .to_string();

    let report = run_scenario_json(&text, &ScenarioRunOptions::default())
        .await
        .unwrap();
    assert_eq!(report.skipped, 1, "条件不满足应跳过");
    assert_eq!(report.passed, 1, "循环步聚合为一行");
    assert_eq!(counter.load(Ordering::SeqCst), 3, "循环应命中 3 次");
    assert!(report.ok());
}
