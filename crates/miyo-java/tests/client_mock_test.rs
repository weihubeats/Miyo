//! 客户端 ↔ 协议集成测试：axum 起 mock Runtime，验证握手、发现、调用、
//! 取消与错误归一的完整链路（不依赖真实 Java 进程）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use miyo_java::protocol::{InvocationStatus, PROTOCOL_VERSION};
use miyo_java::{JavaRuntimeClient, SecurityPolicy};

/// 模拟 Runtime 的共享状态。
#[derive(Clone)]
struct MockState {
    token: Option<String>,
    invocations: Arc<AtomicU64>,
}

/// token 守卫：Authorization 不匹配 → 401（与 SDK 行为一致）。
#[allow(clippy::result_large_err)] // 测试 mock：直接以 Response 作 Err 便于断言
fn guard(headers: &HeaderMap, state: &MockState) -> Result<(), Response> {
    let sent = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    match (&state.token, sent) {
        (None, _) => Ok(()),
        (Some(exp), Some(got)) if *exp == got => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"code": "not_authorized", "message": "token 不匹配"}})),
        )
            .into_response()),
    }
}

fn ok_json(v: Value) -> Response {
    (StatusCode::OK, Json(v)).into_response()
}

fn err_json(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

async fn info_handler(State(st): State<MockState>, headers: HeaderMap) -> Response {
    if let Err(resp) = guard(&headers, &st) {
        return resp;
    }
    ok_json(json!({
        "protocol_version": PROTOCOL_VERSION,
        "runtime_version": "0.1.0",
        "app_name": "demo-order-app",
        "java_version": "21.0.6",
        "jvm_version": "OpenJDK 64-Bit Server VM 21.0.6",
        "uptime_ms": 42_000,
        "class_count": 2,
        "method_count": 3,
        "instance_provider": "spring"
    }))
}

async fn classes_handler(State(st): State<MockState>, headers: HeaderMap) -> Response {
    if let Err(resp) = guard(&headers, &st) {
        return resp;
    }
    ok_json(json!([
        {
            "class_name": "com.example.OrderService",
            "package_name": "com.example",
            "simple_name": "OrderService",
            "method_count": 2,
            "bean_name": "orderService"
        },
        {
            "class_name": "com.example.MathUtil",
            "package_name": "com.example",
            "simple_name": "MathUtil",
            "method_count": 1,
            "bean_name": null
        }
    ]))
}

async fn methods_handler(
    State(st): State<MockState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = guard(&headers, &st) {
        return resp;
    }
    match q.get("class").map(String::as_str) {
        Some("com.example.OrderService") => ok_json(json!([
            {
                "method_id": "com.example.OrderService#createOrder(CreateOrderRequest,long)",
                "class_name": "com.example.OrderService",
                "method_name": "createOrder",
                "signature": "public OrderResult createOrder(CreateOrderRequest, long)",
                "return_type": "OrderResult",
                "return_generic": "OrderResult",
                "params": [
                    {"name": "request", "type_name": "CreateOrderRequest", "generic_type": "", "required": true},
                    {"name": "userId", "type_name": "long", "generic_type": "", "required": true}
                ],
                "static_method": false,
                "visibility": "public",
                "overloaded": false,
                "declaring_class": "com.example.OrderService"
            },
            {
                "method_id": "com.example.OrderService#find(long,long)",
                "class_name": "com.example.OrderService",
                "method_name": "find",
                "signature": "public Order find(long, long)",
                "return_type": "Order",
                "return_generic": "",
                "params": [
                    {"name": "orderId", "type_name": "long", "generic_type": "", "required": true},
                    {"name": "userId", "type_name": "long", "generic_type": "", "required": true}
                ],
                "static_method": false,
                "visibility": "public",
                "overloaded": true,
                "declaring_class": "com.example.OrderService"
            }
        ])),
        _ => err_json(StatusCode::NOT_FOUND, "not_registered", "类未注册"),
    }
}

async fn invoke_handler(
    State(st): State<MockState>,
    headers: HeaderMap,
    body: Json<Value>,
) -> Response {
    if let Err(resp) = guard(&headers, &st) {
        return resp;
    }
    st.invocations.fetch_add(1, Ordering::SeqCst);
    let method_id = body.0["method_id"].as_str().unwrap_or_default().to_string();
    let invocation_id = body.0["invocation_id"].clone();
    if method_id.contains("boom") {
        return ok_json(json!({
            "invocation_id": invocation_id,
            "method_id": method_id,
            "status": "failed",
            "result": null,
            "result_truncated": false,
            "exception": {
                "exception_type": "java.lang.NullPointerException",
                "message": "order not found",
                "stack_trace": [
                    "com.example.OrderService.pay(OrderService.java:88)",
                    "com.example.OrderService.pay(OrderService.java:60)"
                ],
                "cause_chain": []
            },
            "duration_ms": 12.5,
            "started_at": "2026-11-16T10:00:00Z",
            "finished_at": "2026-11-16T10:00:00.012Z"
        }));
    }
    if method_id.contains("slow") {
        return ok_json(json!({
            "invocation_id": invocation_id,
            "method_id": method_id,
            "status": "timeout",
            "result": null,
            "result_truncated": false,
            "exception": null,
            "duration_ms": 5000.0,
            "started_at": "2026-11-16T10:00:00Z",
            "finished_at": "2026-11-16T10:00:05Z"
        }));
    }
    ok_json(json!({
        "invocation_id": invocation_id,
        "method_id": method_id,
        "status": "success",
        "result": {"orderNo": "NO-1", "amount": 99.5},
        "result_truncated": false,
        "exception": null,
        "duration_ms": 3.0,
        "started_at": "2026-11-16T10:00:00Z",
        "finished_at": "2026-11-16T10:00:00.003Z"
    }))
}

async fn cancel_handler(State(st): State<MockState>, headers: HeaderMap) -> Response {
    if let Err(resp) = guard(&headers, &st) {
        return resp;
    }
    ok_json(json!({"delivered": true}))
}

/// 启动模拟 Runtime，返回监听地址。
async fn mock_runtime(token: Option<&'static str>) -> (String, Arc<AtomicU64>) {
    let invocations = Arc::new(AtomicU64::new(0));
    let app = Router::new()
        .route("/miyo/debug/info", get(info_handler))
        .route("/miyo/debug/classes", get(classes_handler))
        .route("/miyo/debug/methods", get(methods_handler))
        .route("/miyo/debug/invoke", post(invoke_handler))
        .route("/miyo/debug/invocations/{id}/cancel", post(cancel_handler))
        .with_state(MockState {
            token: token.map(str::to_owned),
            invocations: invocations.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), invocations)
}

#[tokio::test]
async fn handshake_and_discovery() {
    let (endpoint, _) = mock_runtime(Some("t0ken")).await;
    let client =
        JavaRuntimeClient::new(&endpoint, Some("t0ken"), SecurityPolicy::default()).unwrap();

    let info = client.info().await.unwrap();
    assert_eq!(info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(info.app_name, "demo-order-app");
    assert_eq!(info.instance_provider, "spring");

    let classes = client.classes().await.unwrap();
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].bean_name.as_deref(), Some("orderService"));

    let methods = client.methods("com.example.OrderService").await.unwrap();
    assert_eq!(methods.len(), 2);
    assert!(methods[1].overloaded);
    assert_eq!(
        methods[0].display_signature(),
        "createOrder(request: CreateOrderRequest, userId: long) -> OrderResult"
    );
}

#[tokio::test]
async fn auth_failure_maps_to_forbidden() {
    let (endpoint, _) = mock_runtime(Some("t0ken")).await;
    let client =
        JavaRuntimeClient::new(&endpoint, Some("wrong"), SecurityPolicy::default()).unwrap();
    let err = client.info().await.unwrap_err();
    assert!(matches!(err, miyo_core::AppError::Forbidden(_)));
    assert!(err.to_string().contains("token"));
}

#[tokio::test]
async fn invoke_success_failure_and_timeout() {
    let (endpoint, counter) = mock_runtime(None).await;
    let client = JavaRuntimeClient::new(&endpoint, None, SecurityPolicy::default()).unwrap();

    let ok = client
        .invoke(
            "com.example.OrderService#createOrder(CreateOrderRequest,long)",
            vec![json!({"sku": "A"}), json!(7)],
            Some(5_000),
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status, InvocationStatus::Success);
    assert_eq!(ok.result.unwrap()["orderNo"], json!("NO-1"));

    let failed = client
        .invoke(
            "com.example.OrderService#boomPay(String)",
            vec![json!("NO-404")],
            Some(5_000),
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(failed.status, InvocationStatus::Failed);
    let exc = failed.exception.unwrap();
    assert_eq!(exc.exception_type, "java.lang.NullPointerException");
    assert_eq!(exc.stack_trace.len(), 2);

    let timeout = client
        .invoke(
            "com.example.OrderService#slowScan(String)",
            vec![json!("x")],
            Some(10_000),
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(timeout.status, InvocationStatus::Timeout);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn unregistered_class_maps_to_not_found() {
    let (endpoint, _) = mock_runtime(None).await;
    let client = JavaRuntimeClient::new(&endpoint, None, SecurityPolicy::default()).unwrap();
    let err = client.methods("com.other.Secret").await.unwrap_err();
    assert!(matches!(err, miyo_core::AppError::NotFound(_)));
}

#[tokio::test]
async fn cancel_delivers() {
    let (endpoint, _) = mock_runtime(None).await;
    let client = JavaRuntimeClient::new(&endpoint, None, SecurityPolicy::default()).unwrap();
    assert!(client.cancel(Uuid::new_v4()).await.unwrap());
}

#[tokio::test]
async fn unreachable_runtime_maps_to_connection() {
    let client =
        JavaRuntimeClient::new("http://127.0.0.1:1", None, SecurityPolicy::default()).unwrap();
    let err = client.info().await.unwrap_err();
    assert!(matches!(err, miyo_core::AppError::Connection(_)));
}
