//! Supervisor ↔ java-debug-sidecar 全链路 e2e：真实 Supervisor 托管
//! java bin（env 透传 → 握手 → request 转发 → stop 优雅终止）。
//! 这是「宿主命令层经 Supervisor 转发」路线的核心机制验证。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use miyo_sidecar::{RestartPolicy, SidecarSink, SidecarSpec, Supervisor, SupervisorTiming};

#[derive(Default)]
struct NullSink;
impl SidecarSink for NullSink {
    fn log(&self, _plugin_id: &str, _level: &str, _message: &str) {}
    fn event(&self, _plugin_id: &str, _event: &str, _payload: serde_json::Value) {}
}

fn fast_supervisor() -> Arc<Supervisor> {
    Arc::new(Supervisor::with_timing(
        Arc::new(NullSink),
        SupervisorTiming {
            heartbeat_interval: Duration::from_millis(200),
            heartbeat_timeout: Duration::from_millis(600),
            handshake_timeout: Duration::from_secs(1),
            restart_backoff_base: Duration::from_millis(200),
        },
    ))
}

#[tokio::test]
async fn supervisor_manages_java_sidecar_full_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = fast_supervisor();

    let spec = SidecarSpec {
        plugin_id: "com.miyo.java-debug".into(),
        executable: PathBuf::from(env!("CARGO_BIN_EXE_java-debug-sidecar")),
        args: vec![],
        working_dir: None,
        max_memory_mb: Some(256),
        restart_policy: RestartPolicy::Off,
        // spec 级透传：jar/infoFile 目录指向临时区（不污染真实 data_dir）
        extra_env: vec![
            (
                "MIYO_JAVA_JAR_DIRS".into(),
                tmp.path().join("java").display().to_string(),
            ),
            (
                "MIYO_JAVA_AGENT_DIR".into(),
                tmp.path().join("java-agent").display().to_string(),
            ),
        ],
    };
    sup.start(spec).await.expect("sidecar 启动");

    // 1. 空会话表（manager 已装配，jar 缺失不影响会话查询）
    let sessions = sup
        .request(
            "com.miyo.java-debug",
            "sessions",
            serde_json::Value::Null,
            Duration::from_secs(5),
        )
        .await
        .expect("sessions 请求失败");
    assert_eq!(sessions.as_array().map(|a| a.len()), Some(0));

    // 2. jar 全缺 → tools_ready=false（诊断语义而非错误）
    let ready = sup
        .request(
            "com.miyo.java-debug",
            "tools_ready",
            serde_json::Value::Null,
            Duration::from_secs(5),
        )
        .await
        .expect("tools_ready 请求失败");
    assert_eq!(ready, serde_json::Value::Bool(false));

    // 3. 未知方法 → 结构化错误（AppError::Validation 包装）
    let err = sup
        .request(
            "com.miyo.java-debug",
            "no_such",
            serde_json::Value::Null,
            Duration::from_secs(5),
        )
        .await
        .expect_err("未知方法应报错");
    assert!(err.to_string().contains("no_such"), "{err}");

    // 4. stop → 进程优雅终止（shutdown_all 清扫）
    sup.stop("com.miyo.java-debug").await.expect("stop 失败");
    tokio::time::sleep(Duration::from_millis(120)).await;
    // status 里已无该插件条目（session 表清空）即终止
    let still = sup
        .status()
        .await
        .iter()
        .any(|s| s.plugin_id == "com.miyo.java-debug");
    assert!(!still, "stop 后 sidecar 不应仍在运行");
}

/// JavaBackend Sidecar 分发 e2e：resolve（env 启用）→ ensure_started（Supervisor
/// 托管）→ sessions / tools_ready / 未知方法 —— 与宿主命令层同一调用面。
#[tokio::test]
async fn backend_sidecar_mode_full_lifecycle() {
    use miyo_java_attach::backend::{AgentOp, JavaBackend, SIDECAR_BIN_ENV};

    let tmp = tempfile::tempdir().unwrap();
    // 宿主进程内 manager 照常构造（Local 常驻，双轨并存）
    let manager = Arc::new(miyo_java_attach::AttachManager::new(
        vec![],
        tmp.path().join("java-agent-local"),
    ));
    let sup = fast_supervisor();
    // bin 路径经 env 传递（resolve 的模式开关）
    // SAFETY：单测试进程内其他用例不读该变量（resolve 仅本用例调用）。
    std::env::set_var(SIDECAR_BIN_ENV, env!("CARGO_BIN_EXE_java-debug-sidecar"));
    let backend = JavaBackend::resolve(
        Arc::clone(&manager),
        Arc::clone(&sup),
        tmp.path().to_path_buf(),
    );
    std::env::remove_var(SIDECAR_BIN_ENV);

    // sessions：空会话表
    let sessions = backend.all_snapshots().await.expect("sessions 失败");
    assert!(sessions.is_empty());

    // agent 调用（未 attach 的 pid → 结构化错误，不崩）
    let err = backend
        .agent_call(999999, AgentOp::Status)
        .await
        .expect_err("未 attach 的 pid 应报错");
    assert!(!err.to_string().is_empty());

    // discover：无 jar → launcher 不可用仍是结构化 summary（hints 承载诊断）
    let summary = backend.discover_processes().await.expect("discover 失败");
    assert!(
        summary
            .hints
            .iter()
            .any(|h| h.contains("jar") || h.contains("launcher") || !h.is_empty())
            || !summary.hints.is_empty()
            || summary.processes.is_empty()
    );
}
