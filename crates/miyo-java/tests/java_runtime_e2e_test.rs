//! Rust 客户端 ↔ 真实 Java Runtime SDK 的对接联调（环境门控）。
//!
//! 前置：demo Runtime 已在运行（`java/examples/demo-app/run_demo.sh` 起的
//! 进程保持存活，或手动启动）。通过环境变量指向：
//! - `MIYO_E2E_RUNTIME`：Runtime 地址（如 http://127.0.0.1:9918）
//! - `MIYO_E2E_TOKEN`：Runtime token
//!
//! 未设置环境变量时整体跳过（门禁稳定，不依赖 Java 进程）。
//! 本测试验证两端协议实现的真实一致性（Rust 序列化 ↔ SDK JSON 解析）。

use miyo_java::{JavaRuntimeClient, SecurityPolicy};

fn runtime_env() -> Option<(String, String)> {
    let endpoint = std::env::var("MIYO_E2E_RUNTIME").ok()?;
    let token = std::env::var("MIYO_E2E_TOKEN").ok()?;
    Some((endpoint, token))
}

#[tokio::test]
async fn real_java_runtime_e2e() {
    let Some((endpoint, token)) = runtime_env() else {
        eprintln!("跳过：未设置 MIYO_E2E_RUNTIME / MIYO_E2E_TOKEN（java/examples/demo-app/run_demo.sh 可启动）");
        return;
    };
    let client = JavaRuntimeClient::new(&endpoint, Some(&token), SecurityPolicy::default())
        .expect("客户端构造");

    // 1. 握手：协议版本由客户端校验（不匹配即拒绝）。
    let info = client.info().await.expect("握手");
    assert_eq!(info.protocol_version, miyo_java::PROTOCOL_VERSION);
    assert_eq!(info.app_name, "demo-order-app");
    assert!(info.method_count > 0);

    // 2. 类发现：白名单含 demo 的 OrderService / PaymentService / MathUtil。
    let classes = client.classes().await.expect("classes");
    assert!(classes.iter().any(|c| c
        .class_name
        .starts_with("com.example.demo.DemoApplication$OrderService")));

    let order_class = classes
        .iter()
        .find(|c| {
            c.class_name
                .starts_with("com.example.demo.DemoApplication$OrderService")
        })
        .expect("OrderService 在白名单")
        .class_name
        .clone();
    let methods = client.methods(&order_class).await.expect("methods");
    let create = methods
        .iter()
        .find(|m| {
            m.method_id
                .contains("#createOrder(String,CreateOrderRequest)")
        })
        .expect("createOrder(String, CreateOrderRequest) 在白名单");
    assert_eq!(create.params.len(), 2);
    assert_eq!(create.return_type, "OrderResult");

    // 3. 调用：DTO JSON → Java 对象注入 → OrderResult 反序列化回 JSON。
    let ok = client
        .invoke(
            &create.method_id,
            vec![
                serde_json::json!("u-e2e"),
                serde_json::json!({
                    "userId": "u-e2e",
                    "items": [
                        {"sku": "SKU-1", "qty": 2},
                        {"sku": "SKU-2", "qty": 1}
                    ],
                    "coupon": "E2E"
                }),
            ],
            Some(5_000),
            uuid::Uuid::new_v4(),
        )
        .await
        .expect("invoke");
    assert_eq!(
        ok.status,
        miyo_java::protocol::InvocationStatus::Success,
        "调用应成功：{ok:?}"
    );
    let result = ok.result.expect("返回值");
    assert_eq!(result["status"], serde_json::json!("CREATED"));
    let amount = result["amount"].as_f64().expect("amount 数值");
    // 3 件 × 9.9（demo 定价）。
    assert!((amount - 29.7).abs() < 1e-6, "金额 3×9.9，实际 {amount}");

    // 4. 异常路径：不存在的订单 → failed + 结构化异常。
    let find = methods
        .iter()
        .find(|m| m.method_id.contains("#find(String)"))
        .expect("find(String) 在白名单");
    let failed = client
        .invoke(
            &find.method_id,
            vec![serde_json::json!("NO-404")],
            Some(5_000),
            uuid::Uuid::new_v4(),
        )
        .await
        .expect("invoke(异常路径) 链路本身成功");
    assert_eq!(failed.status, miyo_java::protocol::InvocationStatus::Failed);
    let exception = failed.exception.expect("异常详情");
    assert!(exception
        .message
        .as_deref()
        .unwrap_or("")
        .contains("NO-404"));
    assert!(!exception.stack_trace.is_empty(), "堆栈非空");

    // 5. 未注册方法 → NotFound。
    let err = client
        .invoke(
            "com.example.demo.DemoApplication$OrderService#notThere()",
            vec![],
            Some(1_000),
            uuid::Uuid::new_v4(),
        )
        .await
        .expect_err("未注册方法应拒绝");
    assert!(matches!(err, miyo_core::AppError::NotFound(_)));
}

#[tokio::test]
async fn real_java_runtime_cancel() {
    let Some((endpoint, token)) = runtime_env() else {
        return;
    };
    let client =
        JavaRuntimeClient::new(&endpoint, Some(&token), SecurityPolicy::default()).unwrap();

    // slowInvoke：PaymentService 无慢方法——取消语义由 SDK 自测覆盖；
    // 这里验证 cancel 端点链路（对已结束/未知 id 返回 delivered=false 也不报错）。
    let delivered = client
        .cancel(uuid::Uuid::new_v4())
        .await
        .expect("cancel 端点可达");
    let _ = delivered; // 未知 id → false；在途 → true。仅验证链路。
}
