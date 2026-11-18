//! Attach 管理器：会话注册表 + attach 流程编排（launcher sidecar → IPC 连接 →
//! 身份校验）。
//!
//! 会话以 pid 为键（同 JVM 内 Agent 单实例）。重复 attach：launcher 会走
//! Agent 的换发 token 路径——会话表以新 token / 新连接覆盖旧会话。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio::sync::Mutex;

use miyo_core::AppError;

use crate::client::AgentClient;
use crate::jdk::ToolchainRegistry;
use crate::launcher::{LauncherRunner, LauncherToolPaths};
use crate::model::{
    AgentHandshakeInfo, AttachSessionSnapshot, JavaProcessInfo, ProcessListSummary,
};
use crate::{process as proc_scan, AGENT_JAR_NAME, LAUNCHER_JAR_NAME};

/// 单 Miyo 实例的 attach 会话上限（防无界注入；超出需先断开空闲会话）。
pub const MAX_ATTACH_SESSIONS: usize = 8;
/// infoFile 过期时长：活跃会话在 detach / 卸载 / 失联移除时即时清理自己的
/// 文件；这里兜底 Miyo 崩溃、目标 JVM 静默死亡等残留（心跳循环周期清扫）。
const INFO_FILE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

/// attach 选项（安全 / 策略上限）。
#[derive(Debug, Clone)]
pub struct AttachOptions {
    /// 并发调用上限（传给 Agent，取 min(Agent 侧)）。
    pub max_concurrent: u32,
    /// 超时上限毫秒。
    pub max_timeout_ms: u64,
    /// 结果序列化深度。
    pub max_depth: u32,
    /// 静态字段扫描（危险，默认关）。
    pub allow_static_scan: bool,
}

impl Default for AttachOptions {
    fn default() -> Self {
        AttachOptions {
            max_concurrent: 4,
            max_timeout_ms: 300_000,
            max_depth: 8,
            allow_static_scan: false,
        }
    }
}

/// 一个活跃的 attach 会话（token 不外泄——快照不含）。
pub struct AttachSession {
    pub pid: i64,
    pub vm_start_ms: i64,
    pub token: String,
    pub port: u16,
    pub client: AgentClient,
    pub handshake: AgentHandshakeInfo,
    pub launcher_java_home: String,
    /// 本次 attach 的连接信息文件（会话移除时一并清理）。
    pub info_file: PathBuf,
    pub attached_at: chrono::DateTime<chrono::Utc>,
    pub reattached: bool,
}

/// Attach 管理器（AppState 持有；全部走 &self + 内部锁）。
pub struct AttachManager {
    /// jar 搜索路径（resource dir / dev fallback；按序取第一个就绪的）。
    jar_candidates: Vec<PathBuf>,
    /// infoFile / 临时目录根（Miyo 数据目录下 java-agent/）。
    data_dir: PathBuf,
    inner: Mutex<ManagerInner>,
}

struct ManagerInner {
    sessions: HashMap<i64, AttachSession>,
    registry: ToolchainRegistry,
}

impl AttachManager {
    pub fn new(jar_candidates: Vec<PathBuf>, data_dir: PathBuf) -> Self {
        AttachManager {
            jar_candidates,
            data_dir,
            inner: Mutex::new(ManagerInner {
                sessions: HashMap::new(),
                registry: ToolchainRegistry::default(),
            }),
        }
    }

    /// jar 路径组（第一个就绪的候选；全缺 → Err 带路径详情）。
    fn resolve_tools(&self) -> Result<LauncherToolPaths, AppError> {
        for candidate in &self.jar_candidates {
            let tools = LauncherToolPaths {
                launcher_jar: candidate.join(LAUNCHER_JAR_NAME),
                agent_jar: candidate.join(AGENT_JAR_NAME),
            };
            if tools.ready() {
                return Ok(tools);
            }
        }
        Err(AppError::NotFound(format!(
            "Attach 工具 jar 缺失（miyo-attach-launcher.jar / miyo-java-agent.jar）。\
             搜索目录：{}。\
             组件会在插件启用后自动下载到首选项目录；也可在工作台点「下载组件」手动获取，\
             或开发模式运行 scripts/build_java_debug.sh",
            self.jar_candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("；")
        )))
    }

    /// jar 就绪探测（诊断 / 前端列表汇总）。
    pub fn tools_ready(&self) -> bool {
        self.resolve_tools().is_ok()
    }

    pub fn jar_dirs(&self) -> Vec<PathBuf> {
        self.jar_candidates.clone()
    }

    /// 进程发现（OS 扫描 + launcher list 交叉 + 版本富化）。
    pub async fn discover_processes(&self) -> ProcessListSummary {
        let started = std::time::Instant::now();
        let mut hints = Vec::new();
        let mut inner = self.inner.lock().await;
        // launcher list：Attach API 视角的可达性（无可用 JDK / 无 HotSpot 时为空 + 提示）
        let attachable_pids = match self.launch_attachable_pids(&mut inner).await {
            Ok(pids) => pids,
            Err(e) => {
                hints.push(format!("Attach 探测不可用：{}", e.user_message()));
                HashSet::new()
            }
        };
        let attached_pids: HashSet<i64> = inner.sessions.keys().copied().collect();
        let processes =
            proc_scan::discover_processes(&mut inner.registry, &attachable_pids, &attached_pids)
                .await;
        let toolchain_count = inner.registry.all().len();
        ProcessListSummary {
            processes,
            toolchain_count,
            launcher_ready: self.tools_ready(),
            agent_ready: self.tools_ready(),
            scan_ms: started.elapsed().as_millis() as u64,
            hints,
        }
    }

    /// 手动 pid 探测。
    pub async fn probe_pid(&self, pid: i64) -> Result<JavaProcessInfo, AppError> {
        let mut inner = self.inner.lock().await;
        let attachable_pids = self
            .launch_attachable_pids(&mut inner)
            .await
            .unwrap_or_default();
        proc_scan::probe_pid(pid, &mut inner.registry, &attachable_pids)
            .await
            .ok_or_else(|| AppError::NotFound(format!("进程 {pid} 不存在或不可见")))
    }

    /// launcher list → attachable pid 集合（附带注册表刷新）。
    async fn launch_attachable_pids(
        &self,
        inner: &mut ManagerInner,
    ) -> Result<HashSet<i64>, AppError> {
        if inner.registry.is_empty() {
            inner.registry.refresh();
        }
        let tools = self.resolve_tools()?;
        let target = inner.registry.select_for_target(None)?;
        let runner = LauncherRunner::new(&target, tools);
        let vms = runner.list_attachable().await?;
        Ok(vms.into_iter().map(|vm| vm.pid).collect())
    }

    /// 当前会话 pid 集合。
    pub async fn attached_pids(&self) -> HashSet<i64> {
        self.inner.lock().await.sessions.keys().copied().collect()
    }

    /// 会话快照（含握手缓存；前端展示）。
    pub async fn session_snapshot(&self, pid: i64) -> Result<AttachSessionSnapshot, AppError> {
        let inner = self.inner.lock().await;
        let session = inner
            .sessions
            .get(&pid)
            .ok_or_else(|| AppError::NotFound(format!("pid {pid} 未 attach")))?;
        Ok(AttachSessionSnapshot {
            pid: session.pid,
            vm_start_ms: session.vm_start_ms,
            agent_version: session.handshake.agent_version.clone(),
            protocol: session.handshake.protocol,
            handshake: session.handshake.clone(),
            launcher_java_home: session.launcher_java_home.clone(),
            attached_at: session.attached_at.to_rfc3339(),
            reattached: session.reattached,
        })
    }

    /// 会话客户端（命令层用）。
    pub async fn client(&self, pid: i64) -> Result<AgentClient, AppError> {
        let inner = self.inner.lock().await;
        let session = inner
            .sessions
            .get(&pid)
            .ok_or_else(|| AppError::NotFound(format!("pid {pid} 未 attach")))?;
        Ok(session.client.clone())
    }

    /// attach：launcher sidecar attach → IPC 连接 → 身份校验 → 会话注册。
    pub async fn attach(
        &self,
        pid: i64,
        options: &AttachOptions,
    ) -> Result<AttachSessionSnapshot, AppError> {
        // 已 attach 且健康 → 直接返回（幂等）
        {
            let inner = self.inner.lock().await;
            if let Some(session) = inner.sessions.get(&pid) {
                if session.client.ping().await.is_ok() {
                    let snapshot = session_to_snapshot(session, true);
                    return Ok(snapshot);
                }
            }
        }
        // 进程身份前置确认（存在且是 Java）
        let mut inner = self.inner.lock().await;
        // 会话上限：新 pid 才计数（幂等重连 / 失联重建不受限）
        if !inner.sessions.contains_key(&pid) {
            check_session_limit(inner.sessions.len())?;
        }
        let proc_info = proc_scan::probe_pid(pid, &mut inner.registry, &HashSet::new())
            .await
            .ok_or_else(|| AppError::NotFound(format!("进程 {pid} 不存在（可能已退出）")))?;
        if proc_info.main_class.is_empty() && !proc_info.command_line.contains("java") {
            return Err(AppError::Validation(format!(
                "pid {pid} 不是 Java 进程：{}",
                proc_info.command_line.chars().take(120).collect::<String>()
            )));
        }

        let tools = self.resolve_tools()?;
        let target_major = crate::jdk::parse_major(&proc_info.java_version);
        let target_major = (target_major > 0).then_some(target_major);
        let toolchain = inner.registry.select_for_target(target_major)?;
        let runner = LauncherRunner::new(&toolchain, tools);

        // 一次性 token（256-bit hex）
        let token = generate_token();
        let info_file = self.data_dir.join(format!(
            "attach-{pid}-{}.json",
            chrono::Utc::now().timestamp_millis()
        ));
        let mut agent_args = format!(
            "token={token},infoFile={},maxConcurrent={},maxTimeoutMs={},maxDepth={}",
            info_file.display(),
            options.max_concurrent,
            options.max_timeout_ms,
            options.max_depth
        );
        if options.allow_static_scan {
            agent_args.push_str(",allowStaticScan=true");
        }
        agent_args.push_str(&format!(",expectedPid={pid}"));

        let info = runner.attach(pid, &agent_args, &info_file, 25_000).await?;
        if info.pid != pid {
            return Err(AppError::Connection(format!(
                "Agent 上报 pid（{}）与请求 pid（{pid}）不一致，拒绝连接",
                info.pid
            )));
        }
        // IPC 握手（ensure_connection 内自动完成：token + 协议版本校验；
        // ping 验证链路可用并触发握手）
        let client = AgentClient::new(info.port, token.clone());
        client.ping().await.map_err(|e| e.to_app_error())?;

        // 重复 attach 识别：Agent 侧 existing 标记（Agent 已运行 → 仅换发 token，
        // 无新注入）。detach 后重连同样命中——对用户语义是「无需重新注入」。
        let reattached = info.existing;
        if let Some(old) = inner.sessions.get(&pid) {
            if old.vm_start_ms != info.vm_start_ms {
                // 进程重启（pid 复用）：旧会话作废，按新身份重建
                tracing::info!(
                    "pid {pid} 检测到 JVM 重启（vm_start {} → {}），重建会话",
                    old.vm_start_ms,
                    info.vm_start_ms
                );
            }
        }

        let handshake = AgentHandshakeInfo {
            protocol: info.protocol,
            agent_version: info.agent_version.clone(),
            pid: info.pid,
            vm_start_ms: info.vm_start_ms,
            java_version: proc_info.java_version.clone(),
            java_vm_version: String::new(),
            jvm_vendor: proc_info.jvm_vendor.clone(),
            java_home: proc_info.java_home.clone(),
            max_concurrent: options.max_concurrent,
            max_timeout_ms: options.max_timeout_ms,
            uptime_ms: 0,
        };
        let session = AttachSession {
            pid,
            vm_start_ms: info.vm_start_ms,
            token,
            port: info.port,
            client,
            handshake: handshake.clone(),
            launcher_java_home: toolchain.home.clone(),
            info_file: info_file.clone(),
            attached_at: chrono::Utc::now(),
            reattached,
        };
        let snapshot = session_to_snapshot(&session, reattached);
        inner.sessions.insert(pid, session);
        Ok(snapshot)
    }

    /// 断开连接（不卸载 Agent；Agent 服务继续运行）。本会话 infoFile 一并清理。
    pub async fn detach(&self, pid: i64) -> Result<(), AppError> {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.remove(&pid) {
            session.client.disconnect().await;
            let _ = tokio::fs::remove_file(&session.info_file).await;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("pid {pid} 未 attach")))
        }
    }

    /// 卸载 Agent：agent.stop → 断开 → 会话移除 + infoFile 清理
    /// （类不可卸载——诚实语义）。
    pub async fn unload_agent(&self, pid: i64) -> Result<(), AppError> {
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.remove(&pid) {
            // stop 失败不阻塞会话移除（目标进程可能已退出）
            let _ = session.client.stop_agent().await;
            session.client.disconnect().await;
            let _ = tokio::fs::remove_file(&session.info_file).await;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("pid {pid} 未 attach")))
        }
    }

    /// 全部会话快照（重连 UI 恢复用）。
    pub async fn all_snapshots(&self) -> Vec<AttachSessionSnapshot> {
        let inner = self.inner.lock().await;
        inner
            .sessions
            .values()
            .map(|s| session_to_snapshot(s, false))
            .collect()
    }

    /// 启动会话心跳循环（lib.rs setup 装配时调用一次，随应用生命周期常驻）。
    ///
    /// 每个 interval：① 对全部会话 ping（内置 5s 超时），失联会话移除并清理
    /// 其 infoFile（目标 JVM 静默死亡后 stale 会话不再滞留；下次 attach 幂等
    /// 重建）；② 清扫 data_dir 下过期 infoFile（兜底 Miyo 崩溃残留）。
    /// interval 首个 tick 立即触发——启动即完成一次清扫。
    pub fn spawn_heartbeat(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                manager.heartbeat_once().await;
                manager.sweep_info_files().await;
            }
        })
    }

    /// 单轮心跳：收集快照 → 锁外逐个 ping → 移除失联会话。
    async fn heartbeat_once(&self) {
        let probes: Vec<(i64, AgentClient)> = {
            let inner = self.inner.lock().await;
            inner
                .sessions
                .values()
                .map(|s| (s.pid, s.client.clone()))
                .collect()
        };
        let mut dead: Vec<i64> = Vec::new();
        for (pid, client) in &probes {
            if client.ping().await.is_err() {
                dead.push(*pid);
            }
        }
        if dead.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().await;
        for pid in dead {
            if let Some(session) = inner.sessions.remove(&pid) {
                tracing::warn!("attach 会话失联，移除：pid={pid}（目标进程可能已退出）");
                session.client.disconnect().await;
                let _ = tokio::fs::remove_file(&session.info_file).await;
            }
        }
    }

    /// 清扫 data_dir 下过期 infoFile（attach-*.json，mtime 超过
    /// [`INFO_FILE_MAX_AGE_MS`] 即删）。返回删除数。
    pub async fn sweep_info_files(&self) -> usize {
        let Ok(mut entries) = tokio::fs::read_dir(&self.data_dir).await else {
            return 0;
        };
        let mut removed = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("attach-") || !name.ends_with(".json") {
                continue;
            }
            let expired = match tokio::fs::metadata(&path).await {
                Ok(meta) => meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .map(|age| age.as_millis() as u64 > INFO_FILE_MAX_AGE_MS)
                    .unwrap_or(false),
                Err(_) => false, // 元数据读不到 → 保守跳过，下轮再试
            };
            if expired && tokio::fs::remove_file(&path).await.is_ok() {
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!("清理过期 attach infoFile：{removed} 个");
        }
        removed
    }

    /// 应用退出清理：对全部会话尽力 agent.stop（停目标 JVM 内 IPC 服务 +
    /// 清 infoFile；已加载类不可卸载——诚实语义），随后移除全部会话。
    /// stop 各自限时 2s，不阻塞退出。
    pub async fn shutdown_all(&self) {
        let sessions: Vec<AttachSession> = {
            let mut inner = self.inner.lock().await;
            std::mem::take(&mut inner.sessions).into_values().collect()
        };
        for session in sessions {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.client.stop_agent()).await;
            session.client.disconnect().await;
            let _ = tokio::fs::remove_file(&session.info_file).await;
        }
    }
}

/// 会话上限校验（独立纯函数便于单测）。
fn check_session_limit(existing: usize) -> Result<(), AppError> {
    if existing >= MAX_ATTACH_SESSIONS {
        return Err(AppError::Validation(format!(
            "attach 会话数已达上限（{MAX_ATTACH_SESSIONS}）：请先断开不再需要的会话"
        )));
    }
    Ok(())
}

fn session_to_snapshot(session: &AttachSession, reattached: bool) -> AttachSessionSnapshot {
    AttachSessionSnapshot {
        pid: session.pid,
        vm_start_ms: session.vm_start_ms,
        agent_version: session.handshake.agent_version.clone(),
        protocol: session.handshake.protocol,
        handshake: session.handshake.clone(),
        launcher_java_home: session.launcher_java_home.clone(),
        attached_at: session.attached_at.to_rfc3339(),
        reattached,
    }
}

/// 一次性 attach token（256-bit hex；经 launcher → loadAgent 参数传给 Agent）。
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
impl AttachManager {
    /// 测试钩子：直接注册会话（绕过 launcher / 目标 JVM），供心跳 / 上限 /
    /// 清理逻辑测试。
    pub(crate) async fn register_session_for_test(&self, session: AttachSession) {
        self.inner
            .lock()
            .await
            .sessions
            .insert(session.pid, session);
    }

    /// 测试钩子：当前会话数。
    pub(crate) async fn session_count_for_test(&self) -> usize {
        self.inner.lock().await.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shape() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, generate_token());
    }

    #[test]
    fn session_limit_rejects_over_cap() {
        for n in 0..MAX_ATTACH_SESSIONS {
            assert!(check_session_limit(n).is_ok());
        }
        let err = check_session_limit(MAX_ATTACH_SESSIONS).unwrap_err();
        assert!(err.user_message().contains("上限"), "{err}");
        assert!(check_session_limit(MAX_ATTACH_SESSIONS + 1).is_err());
    }

    /// mock Agent IPC：握手成功；`answer_ping=false` 时对 ping 不回包
    /// （客户端 5s 超时 → 心跳判死）。
    async fn spawn_mock_agent(answer_ping: bool) -> (u16, Arc<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    loop {
                        let Ok(frame) = crate::client::read_frame(&mut socket).await else {
                            return;
                        };
                        if frame["op"] == "handshake" {
                            let body = serde_json::json!({"id": "handshake", "ok": true,
                                "data": {"protocol": 2, "pid": 1, "agent_version": "t"}});
                            let _ = crate::client::write_frame(&mut socket, &body).await;
                        } else if frame["op"] == "ping" && answer_ping {
                            let body = serde_json::json!({"id": frame["id"], "ok": true,
                                "data": {"pong": 1}});
                            let _ = crate::client::write_frame(&mut socket, &body).await;
                        }
                    }
                });
            }
        });
        (port, Arc::new(()))
    }

    fn test_session(pid: i64, port: u16, dir: &std::path::Path) -> AttachSession {
        AttachSession {
            pid,
            vm_start_ms: 1,
            token: "t".into(),
            port,
            client: AgentClient::new(port, "t".into()),
            handshake: AgentHandshakeInfo {
                protocol: 2,
                agent_version: "t".into(),
                pid,
                vm_start_ms: 1,
                java_version: String::new(),
                java_vm_version: String::new(),
                jvm_vendor: String::new(),
                java_home: String::new(),
                max_concurrent: 4,
                max_timeout_ms: 300_000,
                uptime_ms: 0,
            },
            info_file: dir.join(format!("attach-{pid}-0.json")),
            launcher_java_home: String::new(),
            attached_at: chrono::Utc::now(),
            reattached: false,
        }
    }

    #[tokio::test]
    async fn heartbeat_removes_stale_session_and_info_file() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = AttachManager::new(vec![], tmp.path().to_path_buf());
        let (port, _keep) = spawn_mock_agent(false).await; // 不回 ping → 判死
        manager
            .register_session_for_test(test_session(101, port, tmp.path()))
            .await;
        std::fs::write(tmp.path().join("attach-101-0.json"), "{}").unwrap();

        manager.heartbeat_once().await;

        assert_eq!(manager.session_count_for_test().await, 0);
        assert!(!tmp.path().join("attach-101-0.json").exists());
    }

    #[tokio::test]
    async fn heartbeat_keeps_healthy_session() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = AttachManager::new(vec![], tmp.path().to_path_buf());
        let (port, _keep) = spawn_mock_agent(true).await;
        manager
            .register_session_for_test(test_session(102, port, tmp.path()))
            .await;

        manager.heartbeat_once().await;

        assert_eq!(manager.session_count_for_test().await, 1);
    }

    #[tokio::test]
    async fn sweep_removes_only_expired_info_files() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = AttachManager::new(vec![], tmp.path().to_path_buf());
        // 过期文件（mtime 拨回 25h 前）+ 新鲜文件 + 非匹配名
        let old = tmp.path().join("attach-1-0.json");
        let fresh = tmp.path().join("attach-2-0.json");
        let other = tmp.path().join("unrelated.json");
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(&fresh, "{}").unwrap();
        std::fs::write(&other, "{}").unwrap();
        let stale = std::time::Duration::from_millis(INFO_FILE_MAX_AGE_MS + 3_600_000);
        let past = std::time::SystemTime::now() - stale;
        let f = std::fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(past).unwrap();
        drop(f);

        let removed = manager.sweep_info_files().await;

        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
    }

    #[tokio::test]
    async fn shutdown_all_stops_disconnects_and_cleans() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = AttachManager::new(vec![], tmp.path().to_path_buf());
        let (port, _keep) = spawn_mock_agent(true).await;
        manager
            .register_session_for_test(test_session(103, port, tmp.path()))
            .await;
        std::fs::write(tmp.path().join("attach-103-0.json"), "{}").unwrap();

        manager.shutdown_all().await;

        assert_eq!(manager.session_count_for_test().await, 0);
        assert!(!tmp.path().join("attach-103-0.json").exists());
    }
}
