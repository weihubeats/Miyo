//! 本地 Runtime 发现：注册目录扫描 + 默认端口段指纹探测。
//!
//! 两条发现路径（互为兜底，均为真实探测、不做任何伪装）：
//!
//! 1. **注册文件**：SDK `start()` 会写 `~/.miyo/debug-runtimes/{app}-{port}.json`
//!    （自述信息，不含 token），本模块扫描该目录拿到候选（应用名 / PID / Java 版本）；
//! 2. **端口指纹探测**：对默认端口段（9918..=9925）与注册端口并发发起
//!    `GET /miyo/debug/info`（不带 token）。SDK 对所有端点强制鉴权，
//!    因此「HTTP 401 + 错误体 `not_authorized`」是该 SDK 的唯一指纹——
//!    随机 HTTP 服务返回 404 / HTML，连接拒绝 / 超时直接排除。
//!
//! 探测只确认「这里有一个 Miyo Debug Runtime」，应用名等详情仍需用户
//! 填 token 正常连接后由握手 info 提供（token 永不参与发现环节）。

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

/// 默认探测端口段（SDK 默认 9918；向后扩 8 个，覆盖多应用同机场景）。
pub const PROBE_PORTS: &[u16] = &[9918, 9919, 9920, 9921, 9922, 9923, 9924, 9925];
/// 单端点探测超时（快进快出；本地回环足够）。
const PROBE_TIMEOUT_MS: u64 = 1500;
/// 并发探测上限（16 端口全探也不会瞬间打爆；留余量给未来扩段）。
const PROBE_CONCURRENCY: usize = 16;

/// 发现到的本地 Runtime 候选。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaDiscoveredRuntime {
    /// 来源：`registry`（注册文件）或 `port_probe`（纯端口探测）。
    pub source: &'static str,
    /// 完整端点（`http://127.0.0.1:9918`）。
    pub endpoint: String,
    /// 应用名（探测路径无 token 读不到，展示为空，连接后由 info 补全）。
    pub app_name: String,
    /// Java 版本（仅注册文件路径可提供）。
    pub java_version: String,
    /// 目标进程 PID（仅注册文件路径可提供）。
    pub pid: Option<i64>,
    /// SDK 版本（仅注册文件路径可提供）。
    pub sdk_version: String,
    /// 端口指纹探测确认存活（注册文件可能是崩溃残留，以探测为准）。
    pub alive: bool,
}

/// 注册目录（约 `~/.miyo/debug-runtimes`；home 不可用时返回 None）。
pub fn registry_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".miyo").join("debug-runtimes"))
}

/// 扫描注册目录（目录不存在 / 条目损坏跳过，不报错——发现是尽力而为）。
pub fn read_registry_entries() -> Vec<JavaDiscoveredRuntime> {
    match registry_dir() {
        Some(dir) => read_registry_entries_from(&dir),
        None => Vec::new(),
    }
}

/// 指定目录版（测试注入用；真实扫描走 [`read_registry_entries`]）。
fn read_registry_entries_from(dir: &std::path::Path) -> Vec<JavaDiscoveredRuntime> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let port = v.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16;
        if port == 0 {
            continue;
        }
        let host = v
            .get("bind_host")
            .and_then(|b| b.as_str())
            .filter(|b| !b.is_empty() && *b != "0.0.0.0")
            .unwrap_or("127.0.0.1");
        out.push(JavaDiscoveredRuntime {
            source: "registry",
            endpoint: format!("http://{host}:{port}"),
            app_name: v
                .get("app_name")
                .and_then(|a| a.as_str())
                .unwrap_or_default()
                .to_string(),
            java_version: v
                .get("java_version")
                .and_then(|a| a.as_str())
                .unwrap_or_default()
                .to_string(),
            pid: v.get("pid").and_then(|p| p.as_i64()).filter(|p| *p > 0),
            sdk_version: v
                .get("sdk_version")
                .and_then(|a| a.as_str())
                .unwrap_or_default()
                .to_string(),
            alive: false,
        });
    }
    out
}

/// 探测响应指纹判定（纯函数）：401 + `not_authorized` = SDK 鉴权指纹；
/// 200 + 协议 info 形状（含 `protocol_version`）= 握手成功（罕见的无 token 配置）。
pub fn is_miyo_runtime_response(status: u16, body: &str) -> bool {
    if status == 401 {
        return body.contains("not_authorized");
    }
    if status == 200 {
        // info 必带 protocol_version 数字字段；随机 200 HTML/文本不会命中。
        serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("protocol_version")?.as_u64().map(|_| ()))
            .is_some()
    } else {
        false
    }
}

/// 探测单个端点是否为 Miyo Debug Runtime。网络失败（连接拒绝 / 超时）= false。
pub async fn probe_runtime(endpoint: &str) -> bool {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("{}/miyo/debug/info", endpoint.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).send().await else {
        return false;
    };
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    is_miyo_runtime_response(status, &body)
}

/// 完整发现：注册目录条目 ∪ 默认端口段 → 并发探测 → 活性确认后合并。
///
/// 返回按「注册文件优先、端点排序」排列；`alive` 为探测结论。
pub async fn discover_local() -> Vec<JavaDiscoveredRuntime> {
    let mut registry = read_registry_entries();
    // 候选端点 = 注册文件端口 ∪ 默认端口段（去重）。
    let mut endpoints: Vec<String> = PROBE_PORTS
        .iter()
        .map(|p| format!("http://127.0.0.1:{p}"))
        .collect();
    for entry in &registry {
        if !endpoints.contains(&entry.endpoint) {
            endpoints.push(entry.endpoint.clone());
        }
    }
    // 并发探测（限流信号量；单点失败不影响整体）。
    let sem = std::sync::Arc::new(Semaphore::new(PROBE_CONCURRENCY));
    let mut tasks = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let permit = sem.clone().acquire_owned().await;
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            let alive = probe_runtime(&endpoint).await;
            (endpoint, alive)
        }));
    }
    let mut probe_map = std::collections::HashMap::new();
    for task in tasks {
        if let Ok((endpoint, alive)) = task.await {
            probe_map.insert(endpoint, alive);
        }
    }
    // 注册条目带活性结论（崩溃残留 → alive=false，前端仍展示但标注过期）。
    for entry in &mut registry {
        entry.alive = probe_map.get(&entry.endpoint).copied().unwrap_or(false);
    }
    // 端口探测路径：仅收录确认存活的（避免展示一堆死端口）。
    let mut probed: Vec<JavaDiscoveredRuntime> = probe_map
        .into_iter()
        .filter(|(_, alive)| *alive)
        .filter(|(endpoint, _)| !registry.iter().any(|r| &r.endpoint == endpoint))
        .map(|(endpoint, _)| JavaDiscoveredRuntime {
            source: "port_probe",
            endpoint,
            app_name: String::new(),
            java_version: String::new(),
            pid: None,
            sdk_version: String::new(),
            alive: true,
        })
        .collect();
    let mut out = registry;
    out.append(&mut probed);
    out.sort_by(|a, b| {
        b.alive
            .cmp(&a.alive)
            .then_with(|| a.source.cmp(b.source))
            .then_with(|| a.endpoint.cmp(&b.endpoint))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    #[test]
    fn fingerprint_401_not_authorized() {
        let body = r#"{"error":{"code":"not_authorized","message":"token 不匹配或缺失"}}"#;
        assert!(is_miyo_runtime_response(401, body));
        // 随机 401（无 SDK 错误码）不是指纹。
        assert!(!is_miyo_runtime_response(401, "unauthorized"));
        assert!(!is_miyo_runtime_response(
            401,
            r#"{"error":{"code":"other"}}"#
        ));
    }

    #[test]
    fn fingerprint_200_protocol_info() {
        let body = r#"{"protocol_version":1,"app_name":"x","java_version":"21"}"#;
        assert!(is_miyo_runtime_response(200, body));
        assert!(!is_miyo_runtime_response(200, "<html>hello</html>"));
        assert!(!is_miyo_runtime_response(200, r#"{"foo":1}"#));
    }

    #[test]
    fn fingerprint_other_status_rejected() {
        assert!(!is_miyo_runtime_response(404, "not_authorized"));
        assert!(!is_miyo_runtime_response(500, r#"{"protocol_version":1}"#));
    }

    #[tokio::test]
    async fn probe_dead_port_returns_false() {
        // 理论上无人监听的回环端口；连接拒绝 → false。
        assert!(!probe_runtime("http://127.0.0.1:9").await);
    }

    #[tokio::test]
    async fn probe_live_runtime_returns_true() {
        async fn info() -> impl IntoResponse {
            (
                StatusCode::UNAUTHORIZED,
                r#"{"error":{"code":"not_authorized","message":"token 不匹配或缺失"}}"#,
            )
        }
        let app = Router::new().route("/miyo/debug/info", get(info));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(probe_runtime(&format!("http://{addr}")).await);
    }

    #[test]
    fn registry_entries_skip_broken_files() {
        // 注入隔离的注册目录：真实 home 不受测试污染。
        let dir = std::env::temp_dir().join(format!("miyo-discover-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("order-service-9918.json"),
            r#"{"app_name":"order-service","port":9918,"bind_host":"127.0.0.1",
                "java_version":"21.0.6","pid":4242,"sdk_version":"0.1.0"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.json"), "{not json").unwrap();
        std::fs::write(dir.join("noport.json"), r#"{"app_name":"x"}"#).unwrap();
        std::fs::write(dir.join("readme.txt"), "ignore").unwrap();

        let parsed = read_registry_entries_from(&dir);
        assert_eq!(parsed.len(), 1, "损坏 / 缺 port / 非 json 均应跳过");
        assert_eq!(parsed[0].app_name, "order-service");
        assert_eq!(parsed[0].pid, Some(4242));
        assert_eq!(parsed[0].endpoint, "http://127.0.0.1:9918");
        assert!(!parsed[0].alive, "注册条目初始无活性结论，以探测为准");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registry_entries_missing_dir_empty() {
        let dir = std::env::temp_dir().join(format!("miyo-discover-absent-{}", std::process::id()));
        assert!(read_registry_entries_from(&dir).is_empty());
    }
}
