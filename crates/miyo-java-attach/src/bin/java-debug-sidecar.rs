//! java-debug-sidecar：Java Debug 2.0 的常驻 sidecar 二进制（ROADMAP §1 第 1 步）。
//!
//! 把 AttachManager 的全部编排能力（会话表 / AgentClient 长连 / JDK 工具链 /
//! launcher spawn / 心跳清扫）搬出宿主进程，经 miyo-sidecar stdio 协议
//! （v1——req/res 原语已够：attach 会话以 pid 参数多路复用，无需新帧型）对外
//! 提供。宿主侧接线（命令转发 / 插件 kind 迁移）为下一阶段，当前双轨并行：
//! 宿主进程内编排仍是默认路径。
//!
//! 环境变量（Supervisor env 白名单需同步放行，见 miyo-sidecar/limits.rs）：
//! - `MIYO_JAVA_JAR_DIRS`：jar 候选目录，`;` 分隔（按序取第一个就绪者）；
//! - `MIYO_JAVA_AGENT_DIR`：attach infoFile 落地目录。
//!
//! 生命周期：stdin EOF → `shutdown_all`（尽力 detach + 清 infoFile）→ 退出。
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use miyo_java_attach::backend::ClassesQuery;
use miyo_java_attach::{AttachManager, AttachOptions};
use miyo_sidecar::protocol::{
    read_frame, write_frame, SidecarErrorBody, SidecarFrame, SIDECAR_PROTOCOL_VERSION,
};
use serde_json::Value;

/// jar 候选目录：MIYO_JAVA_JAR_DIRS 显式优先；否则按宿主同款约定从数据目录
/// 推导（`{data_dir}/java`，data_dir 解析 MIYO_DATA_DIR / 默认目录，
/// 与 miyo-tauri lib.rs 候选序一致——远程下载落地目录优先于随包资源由宿主
/// 侧负责下载到此）。
fn jar_dirs() -> Vec<PathBuf> {
    let explicit = std::env::var("MIYO_JAVA_JAR_DIRS")
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }
    vec![data_dir().join("java")]
}

/// attach infoFile 目录：MIYO_JAVA_AGENT_DIR 显式优先；否则 `{data_dir}/java-agent`。
fn agent_dir() -> PathBuf {
    std::env::var("MIYO_JAVA_AGENT_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir().join("java-agent"))
}

/// 数据目录解析（与 miyo-core::paths 同优先级：MIYO_DATA_DIR > 默认目录）。
fn data_dir() -> PathBuf {
    miyo_core::paths::data_dir_from_env().unwrap_or_else(miyo_core::paths::default_data_dir)
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("sidecar tokio runtime");
    rt.block_on(run());
}

async fn run() {
    // 0. stdin 上的 hello 握手：method = plugin_id，params = { token, protocol }；
    //    ack 回显 token + protocol + sidecar_version（supervisor 校验三件套）。
    let mut stdin = tokio::io::stdin();
    let hello = match read_frame(&mut stdin).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[java-debug-sidecar] 首帧读取失败：{e}，退出");
            return;
        }
    };
    if hello.kind != "hello" {
        eprintln!(
            "[java-debug-sidecar] 首帧不是 hello（kind={}），退出",
            hello.kind
        );
        return;
    }
    let token = hello
        .params
        .as_ref()
        .and_then(|p| p.get("token"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let ack = SidecarFrame {
        method: hello.method.clone(),
        params: Some(serde_json::json!({
            "token": token,
            "protocol": SIDECAR_PROTOCOL_VERSION,
            "sidecar_version": env!("CARGO_PKG_VERSION"),
        })),
        ..miyo_sidecar::protocol::host_frame("hello_ack")
    };
    let mut writer = tokio::io::stdout();
    if let Err(e) = write_frame(&mut writer, &ack).await {
        eprintln!("[java-debug-sidecar] hello_ack 写出失败：{e}，退出");
        return;
    }

    // 1. AttachManager 装配（jar 候选与 infoFile 目录来自环境变量）。
    let manager = Arc::new(AttachManager::new(jar_dirs(), agent_dir()));
    // 心跳：失联会话移除 + infoFile 清扫（宿主侧的 stdio ping/pong 互不影响）。
    let _heartbeat = manager.spawn_heartbeat(Duration::from_secs(30));
    manager.sweep_info_files().await;

    // 2. req 分发主循环：EOF（宿主关闭 / 崩溃）→ 优雅清理退出。
    loop {
        let frame = match read_frame(&mut stdin).await {
            Ok(f) => f,
            Err(e) => {
                // EOF = 宿主关闭（正常）；其他读取失败也如实留痕——
                // 此前进程静默消失无从诊断（CI ubuntu 实证）。
                eprintln!("[java-debug-sidecar] stdin 读取结束：{e:?}，shutdown_all 后退出");
                manager.shutdown_all().await;
                return;
            }
        };
        match frame.kind.as_str() {
            "ping" => {
                let pong = SidecarFrame {
                    id: frame.id,
                    ..miyo_sidecar::protocol::host_frame("pong")
                };
                if let Err(e) = write_frame(&mut writer, &pong).await {
                    eprintln!("[java-debug-sidecar] pong 写出失败：{e}，退出");
                    manager.shutdown_all().await;
                    return;
                }
            }
            "req" => {
                let out = match dispatch(&manager, &frame).await {
                    Ok(value) => SidecarFrame {
                        id: frame.id,
                        ok: Some(true),
                        result: Some(value),
                        ..miyo_sidecar::protocol::host_frame("res")
                    },
                    Err(message) => SidecarFrame {
                        id: frame.id,
                        ok: Some(false),
                        error: Some(SidecarErrorBody {
                            code: "JAVA_ERROR".into(),
                            message,
                        }),
                        ..miyo_sidecar::protocol::host_frame("res")
                    },
                };
                if let Err(e) = write_frame(&mut writer, &out).await {
                    eprintln!("[java-debug-sidecar] res 写出失败：{e}，退出");
                    manager.shutdown_all().await;
                    return;
                }
            }
            _ => {
                // 未知帧静默丢弃（与宿主桥同口径，不回执不给探测者信息）。
            }
        }
    }
}

/// req 分发：method → AttachManager / AgentClient 调用。错误统一 String
///（JAVA_ERROR 码由外层包装）。
async fn dispatch(manager: &Arc<AttachManager>, frame: &SidecarFrame) -> Result<Value, String> {
    let method = frame.method.as_deref().unwrap_or("");
    let params = frame.params.clone().unwrap_or(Value::Null);
    let pid = |p: &Value| -> Result<i64, String> {
        p.get("pid")
            .and_then(Value::as_i64)
            .ok_or_else(|| "缺少 pid 参数".to_string())
    };
    match method {
        "discover" => {
            serde_json::to_value(manager.discover_processes().await).map_err(|e| e.to_string())
        }
        "probe" => serde_json::to_value(manager.probe_pid(pid(&params)?).await.map_err(e2s)?)
            .map_err(|e| e.to_string()),
        "attach" => {
            let options: AttachOptions = params
                .get("options")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| format!("options 非法：{e}"))?
                .unwrap_or_default();
            serde_json::to_value(manager.attach(pid(&params)?, &options).await.map_err(e2s)?)
                .map_err(|e| e.to_string())
        }
        "detach" => manager
            .detach(pid(&params)?)
            .await
            .map_err(e2s)
            .map(|_| Value::Null),
        "unload" => manager
            .unload_agent(pid(&params)?)
            .await
            .map_err(e2s)
            .map(|_| Value::Null),
        "sessions" => {
            serde_json::to_value(manager.all_snapshots().await).map_err(|e| e.to_string())
        }
        "tools_ready" => Ok(Value::Bool(manager.tools_ready())),
        "client.ping" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .ping()
            .await
            .map_err(e2s),
        "client.status" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .status()
            .await
            .map_err(e2s),
        "client.spring_status" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .spring_status()
            .await
            .map_err(e2s),
        "client.classes" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .classes(&classes_query(&params))
            .await
            .map_err(e2s),
        "client.classes_stats" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .classes_stats(opt_str(&params, "businessPackages").as_deref())
            .await
            .map_err(e2s),
        "client.classes_reindex" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .classes_reindex(opt_str(&params, "businessPackages").as_deref())
            .await
            .map_err(e2s),
        "client.class_detail" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .class_detail(&req_str(&params, "class")?)
            .await
            .map_err(e2s),
        "client.methods" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .methods(&req_str(&params, "class")?)
            .await
            .map_err(e2s),
        "client.beans" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .beans(
                &req_str(&params, "search")?,
                &req_str(&params, "contextId")?,
                params.get("limit").and_then(Value::as_u64).unwrap_or(50) as u32,
            )
            .await
            .map_err(e2s),
        "client.invoke" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .invoke(
                params.get("params").cloned().unwrap_or(Value::Null),
                params
                    .get("timeoutMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(30_000),
            )
            .await
            .map_err(e2s),
        "client.cancel" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .cancel(&req_str(&params, "invocationId")?)
            .await
            .map_err(e2s),
        "client.stop" => manager
            .client(pid(&params)?)
            .await
            .map_err(e2s)?
            .stop_agent()
            .await
            .map_err(e2s),
        other => Err(format!("未知方法：{other}")),
    }
}

fn e2s(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn opt_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `client.classes` 参数 → ClassesQuery（camelCase 与 snake_case 都接受，
/// 宿主 Local 路径经 serde 序列化出 snake_case，前端调试直连时常用 camelCase）。
fn classes_query(params: &Value) -> ClassesQuery {
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    fn pick(params: &Value, snake: &str, camel: &str) -> Option<String> {
        opt_str(params, snake)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| opt_str(params, camel).filter(|s| !s.trim().is_empty()))
    }
    fn flag(params: &Value, snake: &str, camel: &str) -> bool {
        params
            .get(snake)
            .or_else(|| params.get(camel))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    ClassesQuery {
        search: pick(params, "search", "search"),
        include: pick(params, "include", "include"),
        exclude: pick(params, "exclude", "exclude"),
        category: pick(params, "category", "category"),
        package_prefix: pick(params, "package_prefix", "packagePrefix"),
        include_dependencies: flag(params, "include_dependencies", "includeDependencies"),
        include_jdk: flag(params, "include_jdk", "includeJdk"),
        limit: params.get("limit").and_then(Value::as_u64).unwrap_or(200) as u32,
        offset: params.get("offset").and_then(Value::as_u64).unwrap_or(0) as u32,
        business_packages: pick(params, "business_packages", "businessPackages"),
        refresh_index: params
            .get("refresh_index")
            .or_else(|| params.get("refreshIndex"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }
}

fn req_str(params: &Value, key: &str) -> Result<String, String> {
    opt_str(params, key).ok_or_else(|| format!("缺少 {key} 参数"))
}
