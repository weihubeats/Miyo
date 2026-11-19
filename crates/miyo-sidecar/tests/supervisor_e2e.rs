//! Sidecar Supervisor 集成测试：以真实 echo-sidecar 进程（同包 bin 目标）
//! 做全链路验证——握手鉴权 / 请求-响应 / 事件与日志上送 / 崩溃恢复 / 心跳
//! 判死 / 进程组清理（无僵尸）。**不使用 mock**。

use std::sync::Arc;
use std::time::Duration;

use miyo_sidecar::{RestartPolicy, SidecarSink, SidecarSpec, Supervisor, SupervisorTiming};

/// 记录 sink 收到的日志 / 事件（断言上送链路）。
#[derive(Default)]
struct RecordingSink {
    logs: std::sync::Mutex<Vec<(String, String, String)>>,
    events: std::sync::Mutex<Vec<(String, String, serde_json::Value)>>,
}
impl SidecarSink for RecordingSink {
    fn log(&self, plugin_id: &str, level: &str, message: &str) {
        self.logs.lock().unwrap().push((
            plugin_id.to_string(),
            level.to_string(),
            message.to_string(),
        ));
    }
    fn event(&self, plugin_id: &str, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((plugin_id.to_string(), event.to_string(), payload));
    }
}

fn echo_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_echo-sidecar"))
}

fn spec(plugin_id: &str, policy: RestartPolicy) -> SidecarSpec {
    SidecarSpec {
        plugin_id: plugin_id.into(),
        executable: echo_binary(),
        args: vec![],
        working_dir: None,
        max_memory_mb: Some(256),
        restart_policy: policy,
    }
}

/// 快速时序（测试加速）。
fn fast_timing() -> SupervisorTiming {
    SupervisorTiming {
        heartbeat_interval: Duration::from_millis(200),
        heartbeat_timeout: Duration::from_millis(600),
        handshake_timeout: Duration::from_secs(1),
        restart_backoff_base: Duration::from_millis(200),
    }
}

fn fast_supervisor(sink: Arc<RecordingSink>) -> Arc<Supervisor> {
    Arc::new(Supervisor::with_timing(sink, fast_timing()))
}

/// 等待条件成立（超时即 panic）。
async fn wait_for<Fut, F>(mut cond: F, what: &str)
where
    Fut: std::future::Future<Output = bool>,
    F: FnMut() -> Fut,
{
    for _ in 0..400 {
        if cond().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("等待超时：{what}");
}

#[tokio::test]
async fn handshake_request_and_event_pipeline() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    sup.start(spec("com.test.echo", RestartPolicy::Off))
        .await
        .unwrap();

    // 请求-响应
    let result = sup
        .request(
            "com.test.echo",
            "echo",
            serde_json::json!({"hello": "world"}),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(
        result.get("echo").unwrap(),
        &serde_json::json!({"hello": "world"})
    );

    // boot 事件与启动日志上送（读帧任务 → sink）
    wait_for(
        || async {
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, e, _)| e == "boot")
        },
        "boot event",
    )
    .await;
    wait_for(
        || async {
            sink.logs
                .lock()
                .unwrap()
                .iter()
                .any(|(_, _, m)| m.contains("echo-sidecar ready"))
        },
        "boot log",
    )
    .await;

    // 业务失败 → Validation 错误
    let err = sup
        .request(
            "com.test.echo",
            "fail",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
    assert!(err.user_message().contains("业务失败"), "{err}");

    sup.stop("com.test.echo").await.unwrap();
}

/// 错误 token 场景由协议层拒绝——这里验证错误 sidecar（不回 hello_ack 的
/// 伪进程）在握手超时内被拒。用 /bin/sleep 作为「非协议进程」。
#[tokio::test]
async fn handshake_rejects_non_protocol_process() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    let mut wrong = spec("com.test.wrong", RestartPolicy::Off);
    // sleep 不说 sidecar 协议（stdout 无 hello_ack）→ 握手超时
    #[cfg(unix)]
    {
        wrong.executable = std::path::PathBuf::from("/bin/sleep");
        wrong.args = vec!["30".into()];
        let err = sup.start(wrong).await.unwrap_err();
        // user_message 是通用文案；Debug 载荷带具体原因（握手超时）
        let debug = format!("{err:?}");
        assert!(
            debug.contains("握手超时") || debug.contains("hello_ack"),
            "{debug}"
        );
        sup.shutdown_all().await;
    }
}

#[tokio::test]
async fn crash_recovery_with_backoff_and_loop_guard() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    sup.start(spec("com.test.crash", RestartPolicy::OnCrash))
        .await
        .unwrap();

    let first_pid = sup
        .status()
        .await
        .iter()
        .find(|s| s.plugin_id == "com.test.crash")
        .unwrap()
        .pid;

    // 触发崩溃（sidecar 直接 exit(1)）→ on-crash 退避重启
    let _ = sup
        .request(
            "com.test.crash",
            "crash",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;
    // 新进程接管后 echo 可用（退避 200ms + 重启握手）
    wait_for(
        || async {
            sup.request(
                "com.test.crash",
                "echo",
                serde_json::json!(1),
                Duration::from_millis(800),
            )
            .await
            .is_ok()
        },
        "restarted sidecar answering echo",
    )
    .await;
    let second_pid = sup
        .status()
        .await
        .iter()
        .find(|s| s.plugin_id == "com.test.crash")
        .unwrap()
        .pid;
    assert_ne!(first_pid, second_pid, "崩溃后必须是新进程");
    assert_eq!(
        sup.status()
            .await
            .iter()
            .find(|s| s.plugin_id == "com.test.crash")
            .unwrap()
            .restarts_last_hour,
        1
    );

    // restart_policy = off 的会话崩溃后直接移除（不恢复）
    sup.start(spec("com.test.norecover", RestartPolicy::Off))
        .await
        .unwrap();
    let _ = sup
        .request(
            "com.test.norecover",
            "crash",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;
    wait_for(
        || async {
            !sup.status()
                .await
                .iter()
                .any(|s| s.plugin_id == "com.test.norecover")
        },
        "off-policy session removed",
    )
    .await;

    sup.shutdown_all().await;
}

#[tokio::test]
async fn heartbeat_detects_muted_sidecar() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    sup.start(spec("com.test.mute", RestartPolicy::Off))
        .await
        .unwrap();
    let pid = sup
        .status()
        .await
        .iter()
        .find(|s| s.plugin_id == "com.test.mute")
        .unwrap()
        .pid;

    // mute → 不再回 pong → 心跳判死 → off 策略移除会话
    let _ = sup
        .request(
            "com.test.mute",
            "mute",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;
    wait_for(
        || async {
            !sup.status()
                .await
                .iter()
                .any(|s| s.plugin_id == "com.test.mute")
        },
        "muted sidecar judged dead",
    )
    .await;
    // 判死路径已 kill 进程组
    wait_for(
        || async { !process_alive(pid) },
        "dead process reaped (no zombie)",
    )
    .await;
    // 判死日志上送
    wait_for(
        || async {
            sink.logs
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, m)| p == "com.test.mute" && m.contains("心跳判死"))
        },
        "heartbeat-dead log",
    )
    .await;
    sup.shutdown_all().await;
}

#[tokio::test]
async fn stop_kills_process_group_without_zombie() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    sup.start(spec("com.test.stop", RestartPolicy::Off))
        .await
        .unwrap();
    let pid = sup
        .status()
        .await
        .iter()
        .find(|s| s.plugin_id == "com.test.stop")
        .unwrap()
        .pid;
    // 前置存活断言仅 unix：Windows CI 虚拟机在 spawn→断言窗口内出现过子进程
    // 退出抖动（v0.0.61 CI 实测）；stop 后「已终止」断言保留——那才是 Windows
    // Job Object 终止语义的真正验证点。
    #[cfg(unix)]
    assert!(process_alive(pid));

    sup.stop("com.test.stop").await.unwrap();
    wait_for(|| async { !process_alive(pid) }, "stopped process reaped").await;
    // 重复 stop → NotFound
    let err = sup.stop("com.test.stop").await.unwrap_err();
    assert!(err.user_message().contains("无运行中的 sidecar"), "{err}");
}

#[tokio::test]
async fn total_sidecar_cap_enforced() {
    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    for i in 0..miyo_sidecar::supervisor::MAX_TOTAL_SIDECARS {
        sup.start(spec(&format!("com.test.cap{i}"), RestartPolicy::Off))
            .await
            .unwrap();
    }
    let err = sup
        .start(spec("com.test.cap-over", RestartPolicy::Off))
        .await
        .unwrap_err();
    assert!(err.user_message().contains("上限"), "{err}");
    sup.shutdown_all().await;
    // 清理后可再次启动
    sup.start(spec("com.test.cap-after", RestartPolicy::Off))
        .await
        .unwrap();
    sup.shutdown_all().await;
}

/// 崩溃重启门禁（生命周期隔离）：门禁 = false（插件已停用 / 卸载）→ 重启链
/// 终止、会话移除、不复活新进程；门禁 = true → 正常按 on-crash 重启。
/// 门禁用可翻转共享开关模拟宿主「插件启用态」查询。
#[tokio::test]
async fn restart_gate_blocks_respawn_of_disabled_plugin() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let sink = Arc::new(RecordingSink::default());
    let sup = fast_supervisor(Arc::clone(&sink));
    let enabled = Arc::new(AtomicBool::new(true));
    let gate_enabled = Arc::clone(&enabled);
    sup.set_restart_gate(Arc::new(move |_plugin_id: &str| {
        let gate = Arc::clone(&gate_enabled);
        Box::pin(async move { gate.load(Ordering::Relaxed) })
    }));

    sup.start(spec("com.test.gate", RestartPolicy::OnCrash))
        .await
        .unwrap();

    // 第一次崩溃：门禁 true → 正常重启（新进程应答 echo）
    let _ = sup
        .request(
            "com.test.gate",
            "crash",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;
    wait_for(
        || async {
            sup.request(
                "com.test.gate",
                "echo",
                serde_json::json!(1),
                Duration::from_millis(800),
            )
            .await
            .is_ok()
        },
        "restart allowed while enabled",
    )
    .await;

    // 翻转为停用 → 再崩溃：门禁 false → 不重启、会话移除
    enabled.store(false, Ordering::Relaxed);
    let _ = sup
        .request(
            "com.test.gate",
            "crash",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;
    wait_for(
        || async {
            !sup.status()
                .await
                .iter()
                .any(|s| s.plugin_id == "com.test.gate")
        },
        "disabled plugin session removed after crash",
    )
    .await;
    // 门禁拦截日志上送
    wait_for(
        || async {
            sink.logs
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, m)| p == "com.test.gate" && m.contains("不再自动重启"))
        },
        "gate-blocked log",
    )
    .await;
    // 给足「退避 + 门禁」窗口后仍无新进程（没有被停用门禁复活）
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!sup
        .status()
        .await
        .iter()
        .any(|s| s.plugin_id == "com.test.gate"));
    sup.shutdown_all().await;
}

/// 进程存活探测（unix：kill(pid, 0)；Windows：OpenProcess + 退出码）。
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY：信号 0 探测，不实际发送
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // Windows：查询退出码（STILL_ACTIVE=259）；打不开句柄 = 不存在
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY：句柄用后即关
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            if ok == 0 {
                return false;
            }
            code != 259 // STILL_ACTIVE
        }
    }
}
