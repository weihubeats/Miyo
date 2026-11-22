//! java-debug-sidecar e2e：spawn 真实二进制，走宿主视角协议链路
//! （hello → hello_ack → ping/pong → req/sessions → EOF 优雅退出）。
//! 不依赖目标 JVM——覆盖协议握手、分发骨架与生命周期清理。

use std::time::Duration;

use miyo_sidecar::protocol::{read_frame, write_frame, SidecarFrame, SIDECAR_PROTOCOL_VERSION};
use tokio::io::BufReader;

fn host_frame(
    kind: &str,
    id: Option<&str>,
    method: Option<&str>,
    params: serde_json::Value,
) -> SidecarFrame {
    SidecarFrame {
        id: id.map(str::to_string),
        method: method.map(str::to_string),
        params: if params.is_null() { None } else { Some(params) },
        ..miyo_sidecar::protocol::host_frame(kind)
    }
}

#[tokio::test]
async fn sidecar_handshake_ping_sessions_and_graceful_exit() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_java-debug-sidecar"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .env_remove("MIYO_JAVA_JAR_DIRS")
        .spawn()
        .expect("sidecar bin spawn");

    let mut stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = BufReader::new(stdout);

    // 1. hello → hello_ack（token 回显 + protocol + sidecar_version）
    write_frame(
        &mut stdin,
        &host_frame(
            "hello",
            None,
            Some("com.miyo.java-debug"),
            serde_json::json!({ "token": "tok-e2e", "protocol": SIDECAR_PROTOCOL_VERSION }),
        ),
    )
    .await
    .unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut reader))
        .await
        .expect("hello_ack 超时")
        .expect("hello_ack 读取失败");
    assert_eq!(ack.kind, "hello_ack");
    let p = ack.params.expect("ack 应带 params");
    assert_eq!(p.get("token").and_then(|t| t.as_str()), Some("tok-e2e"));
    assert_eq!(
        p.get("protocol").and_then(|v| v.as_u64()),
        Some(SIDECAR_PROTOCOL_VERSION as u64)
    );
    assert!(p.get("sidecar_version").and_then(|v| v.as_str()).is_some());

    // 2. ping → pong（id 回传）
    write_frame(
        &mut stdin,
        &host_frame("ping", Some("p1"), None, serde_json::Value::Null),
    )
    .await
    .unwrap();
    let pong = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("pong 超时")
        .expect("pong 读取失败");
    assert_eq!(pong.kind, "pong");
    assert_eq!(pong.id.as_deref(), Some("p1"));

    // 3. req sessions → 空会话表（ok=true）
    write_frame(
        &mut stdin,
        &host_frame("req", Some("s1"), Some("sessions"), serde_json::Value::Null),
    )
    .await
    .unwrap();
    let res = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("res 超时")
        .expect("res 读取失败");
    assert_eq!(res.id.as_deref(), Some("s1"));
    assert_eq!(res.ok, Some(true));
    assert_eq!(
        res.result.and_then(|v| v.as_array().map(|a| a.len())),
        Some(0)
    );

    // 4. req 未知方法 → 结构化错误（JAVA_ERROR），不崩进程
    write_frame(
        &mut stdin,
        &host_frame(
            "req",
            Some("u1"),
            Some("no_such_method"),
            serde_json::Value::Null,
        ),
    )
    .await
    .unwrap();
    let err = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("错误帧超时")
        .expect("错误帧读取失败");
    assert_eq!(err.ok, Some(false));
    assert_eq!(err.error.as_ref().expect("应有错误体").code, "JAVA_ERROR");

    // 5. EOF（关 stdin）→ 优雅退出（shutdown_all 后返回 0）
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("退出超时")
        .expect("wait 失败");
    assert!(status.success(), "优雅退出应返回 0，实际 {status}");
}
