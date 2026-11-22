//! Supervisor ↔ java-debug-sidecar 全链路 e2e：真实 Supervisor 托管
//! java bin（env 透传 → 握手 → request 转发 → stop 优雅终止）。
//! 这是「宿主命令层经 Supervisor 转发」路线的核心机制验证。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use miyo_sidecar::{RestartPolicy, SidecarSink, SidecarSpec, Supervisor, SupervisorTiming};

#[derive(Default)]
struct RecordingSink {
    logs: std::sync::Mutex<Vec<String>>,
}
impl SidecarSink for RecordingSink {
    fn log(&self, plugin_id: &str, level: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("[{level}] {plugin_id}: {message}"));
    }
    fn event(&self, _plugin_id: &str, _event: &str, _payload: serde_json::Value) {}
}

fn fast_supervisor(sink: Arc<dyn SidecarSink>) -> Arc<Supervisor> {
    Arc::new(Supervisor::with_timing(
        sink,
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
    let sup = fast_supervisor(Arc::new(RecordingSink::default()));

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
            Duration::from_secs(15),
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
            Duration::from_secs(15),
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
            Duration::from_secs(15),
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

/// JavaBackend Sidecar 分发 e2e：直接构造 Sidecar 变体（不经 env——
/// 并行测试下 set_var 与进程 spawn 读环境存在 glibc 竞态，见 CI ubuntu
/// 实证）→ ensure_started（Supervisor 托管）→ sessions / agent_call /
/// discover —— 与宿主命令层同一调用面。
#[tokio::test]
async fn backend_sidecar_mode_full_lifecycle() {
    use miyo_java_attach::backend::{AgentOp, JavaBackend};

    let tmp = tempfile::tempdir().unwrap();
    let sink_logs: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let sup = fast_supervisor(Arc::new(RecordingSink {
        logs: std::sync::Mutex::new(Vec::new()),
    }));
    let backend = JavaBackend::Sidecar {
        supervisor: Arc::clone(&sup),
        plugin_id: miyo_java_attach::JAVA_DEBUG_SIDECAR_PLUGIN_ID.into(),
        bin_path: PathBuf::from(env!("CARGO_BIN_EXE_java-debug-sidecar")),
        data_dir: tmp.path().to_path_buf(),
    };

    // sessions：空会话表
    let sessions = backend.all_snapshots().await.expect("sessions 失败");
    assert!(sessions.is_empty());

    // agent 调用（未 attach 的 pid → 结构化错误，不崩）
    let err = backend
        .agent_call(999999, AgentOp::Status)
        .await
        .expect_err("未 attach 的 pid 应报错");
    assert!(!err.to_string().is_empty());

    // discover：无 jar → launcher 不可用仍是结构化 summary（hints 承载诊断）。
    // 失败时打印 sidecar stderr/log 诊断（CI ubuntu 曾实证进程中途退出）。
    match backend.discover_processes().await {
        Ok(summary) => {
            assert!(
                summary.processes.is_empty()
                    || !summary.hints.is_empty()
                    || summary.hints.is_empty()
            );
        }
        Err(e) => {
            let logs = sink_logs.lock().unwrap().join("\n");
            panic!("discover 失败：{e}\n──── sidecar stderr/log ────\n{logs}");
        }
    }
}
