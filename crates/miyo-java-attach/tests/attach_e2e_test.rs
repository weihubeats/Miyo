//! Attach 全链路 E2E（env 门控）：真实 Java 目标进程 + 真实 launcher / agent jar。
//!
//! 前置：
//! ```bash
//! scripts/build_java_debug.sh                       # 构建 jar 到 java/dist/
//! (cd java/examples/plain-app && javac --release 8 -d build/classes8 @build/sources.txt)
//! cargo test -p miyo-java-attach --test attach_e2e_test -- --ignored --nocapture
//! ```
//!
//! 覆盖：进程发现 → attach（身份校验）→ 类 / 方法发现 → 静态调用 →
//! DTO 调用（consent 闸）→ 业务异常 → 取消 → detach → 卸载。
//! 目标 JVM 由测试自动以系统默认 java 启动（进程退出即回收）。

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use miyo_java_attach::manager::{AttachManager, AttachOptions};
use miyo_java_attach::model::JavaProcessInfo;
use serde_json::json;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn dist_dir() -> PathBuf {
    repo_root().join("java").join("dist")
}

fn demo_classes() -> PathBuf {
    repo_root()
        .join("java")
        .join("examples")
        .join("plain-app")
        .join("build")
        .join("classes8")
}

fn prerequisites_ready() -> bool {
    dist_dir().join("miyo-java-agent.jar").is_file()
        && dist_dir().join("miyo-attach-launcher.jar").is_file()
        && demo_classes().is_dir()
}

/// 启动 plain-app 目标进程（返回子进程；测试结束 kill）。
fn spawn_target() -> std::process::Child {
    let java = if let Ok(home) = std::env::var("MIYO_E2E_TARGET_JAVA_HOME") {
        PathBuf::from(home)
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" })
    } else {
        PathBuf::from("java")
    };
    std::process::Command::new(java)
        .arg("-cp")
        .arg(demo_classes())
        .arg("com.example.PlainApp")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("目标进程启动失败（先编译 plain-app：javac --release 8）")
}

async fn wait_for_target(manager: &AttachManager, child_pid: i64) -> JavaProcessInfo {
    for _ in 0..40 {
        if let Ok(info) = manager.probe_pid(child_pid).await {
            if info.attachable {
                return info;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("目标进程 {child_pid} 未就绪 / 未出现在 Attach listener 中");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "env 门控：需要 java/dist jar + plain-app 编译产物（scripts/build_java_debug.sh）"]
async fn attach_full_flow() {
    assert!(
        prerequisites_ready(),
        "缺少前置产物：java/dist/*.jar 与 plain-app/build/classes8（见文件头注释）"
    );
    let child = spawn_target();
    let pid = child.id() as i64;

    let data_dir = tempfile::tempdir().unwrap();
    let manager = AttachManager::new(vec![dist_dir()], data_dir.path().to_path_buf());

    // 1) 发现：目标进程出现且 attachable
    let target = wait_for_target(&manager, pid).await;
    assert_eq!(target.pid, pid);
    assert!(
        target.attachable,
        "target 应可 attach: {:?}",
        target.attach_block_reason
    );
    assert!(
        target.main_class.contains("PlainApp"),
        "{:?}",
        target.main_class
    );

    // 2) attach（身份校验 + 会话注册 + 项目行落库）
    let snapshot = manager
        .attach(pid, &AttachOptions::default())
        .await
        .expect("attach 失败");
    assert_eq!(snapshot.pid, pid);
    assert_eq!(snapshot.protocol, 2);
    assert!(!snapshot.agent_version.is_empty());

    // 3) 类发现（JDK 内部类默认排除；com.example 命中）
    let client = manager.client(pid).await.unwrap();
    let classes = client
        .classes(&miyo_java_attach::ClassesQuery {
            search: Some("com.example".into()),
            ..miyo_java_attach::ClassesQuery::new()
        })
        .await
        .unwrap();
    let names: Vec<String> = classes["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("Calculator")),
        "类清单应含 Calculator：{names:?}"
    );

    // 4) 方法发现（重载标记）
    let methods = client
        .methods("com.example.calculator.Calculator")
        .await
        .unwrap();
    let greet_overloads = methods["methods"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["method_name"] == "greet")
        .count();
    assert_eq!(greet_overloads, 2, "greet 应有 2 个重载");

    // 5) 静态调用
    let result = client
        .invoke(
            json!({
                "class": "com.example.calculator.Calculator",
                "method": "add",
                "param_types": ["long", "long"],
                "args": [20, 22],
                "instance": {"kind": "static"},
                "timeout_ms": 10000,
            }),
            20_000,
        )
        .await
        .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(result["result"], 42);

    // 6) 黑名单拒绝（Agent 侧终检）
    let blocked = client
        .invoke(
            json!({
                "class": "java.lang.Runtime",
                "method": "exec",
                "param_types": ["String"],
                "args": ["ls"],
                "instance": {"kind": "static"},
            }),
            10_000,
        )
        .await;
    assert!(blocked.is_err(), "Runtime#exec 必须被拒绝");

    // 7) consent 闸：新建实例未确认 → consent_required
    let consent = client
        .invoke(
            json!({
                "class": "com.example.order.OrderService",
                "method": "orderCount",
                "instance": {"kind": "new"},
            }),
            10_000,
        )
        .await
        .unwrap_err();
    assert!(
        consent.to_string().contains("consent_required"),
        "{consent}"
    );

    // 8) 慢方法取消（协作式；状态 cancelled/timeout，terminated=false）
    let cancel_handle = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .invoke(
                    json!({
                        "invocation_id": "cancel-e2e",
                        "class": "com.example.order.OrderService",
                        "method": "slowOperation",
                        "param_types": ["long"],
                        "args": [30],
                        "instance": {"kind": "new", "consent": true},
                        "timeout_ms": 60000,
                    }),
                    70_000,
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let cancel = client.cancel("cancel-e2e").await.unwrap();
    assert_eq!(cancel["delivered"], true);
    let invoke_result = cancel_handle.await.unwrap().unwrap();
    assert!(
        invoke_result["status"] == "cancelled" || invoke_result["status"] == "timeout",
        "取消后调用状态应为 cancelled/timeout：{invoke_result}"
    );
    assert_eq!(
        invoke_result["terminated"], false,
        "诚实语义：无法保证实际终止"
    );

    // 9) detach（Agent 保持运行）→ 客户端不可用
    manager.detach(pid).await.unwrap();
    let after_detach = manager.client(pid).await;
    assert!(after_detach.is_err(), "detach 后会话应移除");

    // 10) 重新 attach（幂等重连）→ 卸载
    let again = manager
        .attach(pid, &AttachOptions::default())
        .await
        .unwrap();
    assert!(again.reattached, "第二次 attach 应识别 Agent 已运行");
    manager.unload_agent(pid).await.unwrap();
    let after_unload = manager.client(pid).await;
    assert!(after_unload.is_err(), "卸载后会话应移除");

    let mut child = child;
    child.kill().ok();
    let _ = HashSet::<String>::new(); // 保持导入使用
}
