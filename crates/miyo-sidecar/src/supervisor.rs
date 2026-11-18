//! Sidecar Supervisor：常驻型插件子进程的生命周期管理。
//!
//! 会话模型（每插件一个 sidecar）：
//! - `start`：限额校验 → 平台加固 spawn → **stdin 握手**（hello 帧携带一次性
//!   会话 token；hello_ack 必须**回显**正确 token——token 不经 argv，`ps`
//!   不可见）→ 常驻任务接管（读帧分发 / 心跳 / 退出检测）；
//! - `request`：req 帧 → id 关联 res 帧（oneshot）；
//! - `stop` / `shutdown_all`：进程组 kill + 挂起请求清空；
//! - 崩溃恢复：子进程退出（非主动 stop）→ on-crash 策略按指数退避重启
//!   （1s/2s/4s/8s/16s），1 小时窗口内超过 [`MAX_RESTARTS_PER_HOUR`] 次 →
//!   放弃并记录 error（防启动循环）；
//! - 心跳：[`HEARTBEAT_INTERVAL`] ping；[`HEARTBEAT_TIMEOUT`] 内无 pong →
//!   判死 → 走崩溃恢复路径。
//!
//! 进程安全：
//! - 总实例 ≤ [`MAX_TOTAL_SIDECARS`]、单插件 1（重复 start 先停旧实例）；
//! - kill_on_drop + 进程组 kill（unix setsid / windows Job Object）：
//!   防僵尸、防 Miyo 退出残留；
//! - 子进程环境变量最小化（[`crate::limits`]）；token 走握手帧不经 argv。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::BufReader;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use miyo_core::AppError;

use crate::limits::{kill_process_group, PlatformGuard};
use crate::protocol::{
    host_frame, read_frame, write_frame, SidecarFrame, SIDECAR_PROTOCOL_VERSION,
};

/// 时序参数（生产默认；测试注入极小值加速）。
#[derive(Debug, Clone, Copy)]
pub struct SupervisorTiming {
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub handshake_timeout: Duration,
    /// 崩溃重启退避基数（指数：base → 2×base → 4×base…）。
    pub restart_backoff_base: Duration,
}

impl Default for SupervisorTiming {
    fn default() -> Self {
        SupervisorTiming {
            heartbeat_interval: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(15),
            restart_backoff_base: Duration::from_secs(1),
        }
    }
}
/// 全局 sidecar 实例上限。
pub const MAX_TOTAL_SIDECARS: usize = 8;
/// 1 小时窗口内最大崩溃重启次数（超过即放弃，防启动循环）。
pub const MAX_RESTARTS_PER_HOUR: u32 = 5;
/// 请求默认超时。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 重启策略（manifest.sidecar.restart_policy）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartPolicy {
    /// 不重启：退出即结束（默认）。
    #[default]
    Off,
    /// 崩溃自动重启（指数退避 + 1 小时窗口上限）。
    OnCrash,
}

impl RestartPolicy {
    /// manifest 字符串 → 策略（未知值按 off，校验层已拦截非法值）。
    pub fn parse(s: &str) -> Self {
        match s {
            "on-crash" => RestartPolicy::OnCrash,
            _ => RestartPolicy::Off,
        }
    }
}

/// sidecar 会话规格（命令层从 manifest.sidecar + 安装目录构造）。
#[derive(Debug, Clone)]
pub struct SidecarSpec {
    pub plugin_id: String,
    /// 可执行文件绝对路径（安装目录内、files 清单已校验；unix 安装时已置 +x）。
    pub executable: PathBuf,
    /// 附加参数（不含会话 token——token 走握手帧）。
    pub args: Vec<String>,
    /// 工作目录（通常是插件安装目录）。
    pub working_dir: Option<PathBuf>,
    /// 内存上限 MB（None = 平台默认不设限）。
    pub max_memory_mb: Option<u32>,
    pub restart_policy: RestartPolicy,
}

/// 日志 / 事件上送（tauri 层接 plugin_logs 与事件总线）。
pub trait SidecarSink: Send + Sync {
    /// sidecar 产生的日志（log 帧 + stderr 行）。
    fn log(&self, plugin_id: &str, level: &str, message: &str);
    /// sidecar 推送的事件（evt 帧；前端事件总线转发）。
    fn event(&self, plugin_id: &str, event: &str, payload: serde_json::Value);
}

/// 无操作 sink（测试 / 未接线时）。
#[derive(Debug, Default)]
pub struct NoopSink;
impl SidecarSink for NoopSink {
    fn log(&self, _plugin_id: &str, _level: &str, _message: &str) {}
    fn event(&self, _plugin_id: &str, _event: &str, _payload: serde_json::Value) {}
}

/// 会话状态快照（status 命令 / 管理中心展示）。
#[derive(Debug, Clone, Serialize)]
pub struct SidecarStatus {
    pub plugin_id: String,
    pub pid: u32,
    pub sidecar_version: String,
    /// 启动时刻（Unix 毫秒）。
    pub started_at_ms: i64,
    pub restarts_last_hour: u32,
    pub last_error: Option<String>,
}

/// 挂起请求表（id → 回执）。
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>>>;

/// 退出信息（wait / 心跳任务 → 监督任务）。
struct ExitInfo {
    /// 进程退出码（None = 信号终止 / 判死 kill）。
    code: Option<i32>,
    /// 是否为主动 stop（非崩溃）。
    intentional: bool,
    reason: String,
}

/// 一个运行中的 sidecar 会话。
pub struct SidecarSession {
    spec: SidecarSpec,
    pub pid: u32,
    pub sidecar_version: String,
    pub started_at_ms: i64,
    /// 请求挂起表（id → 回执）。
    pending: PendingMap,
    /// stdin 写端（请求 / ping 发送）。
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    /// 主动终止标记（区分崩溃与 stop）。
    intentional_exit: Arc<AtomicBool>,
    /// 平台加固句柄（Windows Job Object；Drop 即 kill 全组）。
    _guard: PlatformGuard,
    /// 心跳任务（terminate 时 abort）。
    heartbeat: JoinHandle<()>,
    /// 本会话累计重启次数（0 = 首发）。
    restarts_last_hour: Arc<AtomicU32>,
}

impl SidecarSession {
    /// 转发请求（id 关联；超时即失败并清理挂起表）。
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, AppError> {
        if self.intentional_exit.load(Ordering::Relaxed) {
            return Err(AppError::Connection("sidecar 正在停止".into()));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let frame = SidecarFrame {
            id: Some(id.clone()),
            method: Some(method.to_string()),
            params: Some(params),
            ..host_frame("req")
        };
        let sent = {
            let mut stdin = self.stdin.lock().await;
            write_frame(&mut *stdin, &frame).await.is_ok()
        };
        if !sent {
            self.pending.lock().await.remove(&id);
            return Err(AppError::Connection(
                "sidecar stdin 已关闭（进程退出？）".into(),
            ));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(AppError::Validation(format!(
                "sidecar 调用失败（{method}）：{message}"
            ))),
            Ok(Err(_)) => Err(AppError::Connection("sidecar 会话已终止".into())),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(AppError::NetworkTimeout(format!(
                    "sidecar 调用超时（{method}，{timeout:?}）"
                )))
            }
        }
    }

    /// 主动终止：置标记 → 进程组 kill → 清空挂起请求 → 停心跳。
    pub async fn terminate(&self) {
        self.intentional_exit.store(true, Ordering::Relaxed);
        kill_process_group(self.pid);
        self._guard.terminate();
        self.heartbeat.abort();
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err("sidecar 已停止".into()));
        }
    }
}

/// Supervisor（AppState 以 Arc 持有；全部走 &self + 内部锁）。
pub struct Supervisor {
    sessions: Mutex<HashMap<String, Arc<SidecarSession>>>,
    /// 每插件崩溃重启时间戳（1 小时窗口计数，防启动循环）。
    restart_log: Mutex<HashMap<String, Vec<Instant>>>,
    sink: Arc<dyn SidecarSink>,
    timing: SupervisorTiming,
}

impl Supervisor {
    pub fn new(sink: Arc<dyn SidecarSink>) -> Self {
        Self::with_timing(sink, SupervisorTiming::default())
    }

    pub fn with_timing(sink: Arc<dyn SidecarSink>, timing: SupervisorTiming) -> Self {
        Supervisor {
            sessions: Mutex::new(HashMap::new()),
            restart_log: Mutex::new(HashMap::new()),
            sink,
            timing,
        }
    }

    /// 全部会话快照（管理中心 / sidecar 状态展示）。
    pub async fn status(&self) -> Vec<SidecarStatus> {
        self.sessions
            .lock()
            .await
            .values()
            .map(|s| SidecarStatus {
                plugin_id: s.spec.plugin_id.clone(),
                pid: s.pid,
                sidecar_version: s.sidecar_version.clone(),
                started_at_ms: s.started_at_ms,
                restarts_last_hour: s.restarts_last_hour.load(Ordering::Relaxed),
                last_error: None,
            })
            .collect()
    }

    /// 启动（幂等替换：同插件已有会话先停）并启动监督任务（崩溃恢复）。
    pub async fn start(self: &Arc<Self>, spec: SidecarSpec) -> Result<(), AppError> {
        let mut sessions = self.sessions.lock().await;
        if sessions.len() >= MAX_TOTAL_SIDECARS && !sessions.contains_key(&spec.plugin_id) {
            return Err(AppError::Validation(format!(
                "sidecar 实例数已达上限（{MAX_TOTAL_SIDECARS}）：请先停止不需要的插件进程"
            )));
        }
        // 幂等替换：旧会话先停（进程组 kill）
        if let Some(old) = sessions.remove(&spec.plugin_id) {
            old.terminate().await;
        }
        let (session, exit_rx) = self.spawn_session(spec.clone()).await?;
        let session = Arc::new(session);
        sessions.insert(spec.plugin_id.clone(), Arc::clone(&session));
        drop(sessions);
        // 监督任务：持有本代退出通道，崩溃 → 按策略重启（换代续监）
        self.spawn_supervision(spec, exit_rx);
        Ok(())
    }

    /// 停止插件 sidecar。
    pub async fn stop(&self, plugin_id: &str) -> Result<(), AppError> {
        let session = self
            .sessions
            .lock()
            .await
            .remove(plugin_id)
            .ok_or_else(|| AppError::NotFound(format!("插件 {plugin_id} 无运行中的 sidecar")))?;
        session.terminate().await;
        Ok(())
    }

    /// 应用退出清理：终止全部 sidecar（进程组 kill，防残留）。
    pub async fn shutdown_all(&self) {
        let sessions: Vec<Arc<SidecarSession>> =
            self.sessions.lock().await.drain().map(|(_, v)| v).collect();
        for s in sessions {
            s.terminate().await;
        }
    }

    /// 转发请求到指定插件的 sidecar。
    pub async fn request(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, AppError> {
        let session = self
            .sessions
            .lock()
            .await
            .get(plugin_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("插件 {plugin_id} 的 sidecar 未运行")))?;
        session.request(method, params, timeout).await
    }

    // ------------------------------------------------------------------
    // 内部
    // ------------------------------------------------------------------

    /// spawn + 握手 + 常驻任务装配。返回（会话, 退出通道）。
    async fn spawn_session(
        &self,
        spec: SidecarSpec,
    ) -> Result<(SidecarSession, mpsc::UnboundedReceiver<ExitInfo>), AppError> {
        if !spec.executable.is_file() {
            return Err(AppError::NotFound(format!(
                "sidecar 可执行文件不存在：{}",
                spec.executable.display()
            )));
        }
        // 一次性会话 token（握手帧下发；不经 argv，ps 不可见）
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut cmd = tokio::process::Command::new(&spec.executable);
        cmd.args(&spec.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = &spec.working_dir {
            cmd.current_dir(dir);
        }
        PlatformGuard::configure(&mut cmd, spec.max_memory_mb);

        let mut child = cmd.spawn().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "sidecar 启动失败（{}）：{e}",
                spec.executable.display()
            )))
        })?;
        let pid = child.id().ok_or_else(|| {
            AppError::Connection("sidecar 启动后未获得 pid（进程立即退出？）".into())
        })?;
        // Windows：spawn 后立即入 Job（kill-on-close + 内存限额）
        let guard = PlatformGuard::attach(pid, spec.max_memory_mb)?;

        let mut stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // ---- 握手：hello（含 token）→ hello_ack（token 回显校验）----
        let hello = SidecarFrame {
            id: None,
            method: Some(spec.plugin_id.clone()),
            params: Some(serde_json::json!({
                "token": token,
                "protocol": SIDECAR_PROTOCOL_VERSION,
            })),
            ..host_frame("hello")
        };
        write_frame(&mut stdin, &hello)
            .await
            .map_err(|e| AppError::Connection(format!("sidecar 握手发送失败：{e}")))?;
        let mut reader = BufReader::new(stdout);
        let handshake_timeout = self.timing.handshake_timeout;
        let ack = tokio::time::timeout(handshake_timeout, read_frame(&mut reader))
            .await
            .map_err(|_| {
                AppError::NetworkTimeout(format!(
                    "sidecar 握手超时（{handshake_timeout:?}）：{}",
                    spec.executable.display()
                ))
            })?
            .map_err(|e| AppError::Connection(format!("sidecar 握手读取失败：{e}")))?;
        if ack.kind != "hello_ack" {
            return Err(AppError::Connection(format!(
                "sidecar 首帧不是 hello_ack（kind={}），握手协议不符",
                ack.kind
            )));
        }
        // token 回显校验：证明对端确实收到了本次会话的 hello（认证 + 防串扰）
        let ack_token = ack
            .params
            .as_ref()
            .and_then(|p| p.get("token"))
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        if ack_token != token {
            return Err(AppError::Connection(
                "sidecar 握手 token 回显不匹配，拒绝连接".into(),
            ));
        }
        let ack_protocol = ack
            .params
            .as_ref()
            .and_then(|p| p.get("protocol"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if ack_protocol != SIDECAR_PROTOCOL_VERSION {
            return Err(AppError::Connection(format!(
                "sidecar 协议版本不匹配（sidecar={ack_protocol}，宿主={SIDECAR_PROTOCOL_VERSION}）"
            )));
        }
        let sidecar_version = ack
            .params
            .as_ref()
            .and_then(|p| p.get("sidecar_version"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        // ---- 常驻任务 ----
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        let stdin = Arc::new(Mutex::new(stdin));
        let last_pong = Arc::new(Mutex::new(Instant::now()));

        spawn_reader_task(
            reader,
            Arc::clone(&pending),
            Arc::clone(&last_pong),
            spec.plugin_id.clone(),
            Arc::clone(&self.sink),
        );
        spawn_stderr_task(stderr, spec.plugin_id.clone(), Arc::clone(&self.sink));
        let heartbeat = spawn_heartbeat_task(
            Arc::clone(&stdin),
            Arc::clone(&last_pong),
            pid,
            spec.plugin_id.clone(),
            exit_tx.clone(),
            self.timing,
        );
        // 退出检测：wait() 归还退出码（内核回收——防僵尸）
        spawn_wait_task(child, exit_tx.clone());

        let session = SidecarSession {
            spec,
            pid,
            sidecar_version,
            started_at_ms: unix_ms(),
            pending,
            stdin,
            intentional_exit: Arc::new(AtomicBool::new(false)),
            _guard: guard,
            heartbeat,
            restarts_last_hour: Arc::new(AtomicU32::new(0)),
        };
        Ok((session, exit_rx))
    }

    /// 监督任务：监听退出事件 → 主动 stop 即清理；崩溃 → 按策略指数退避重启
    /// （换代续监），1 小时窗口超限放弃（防启动循环）。
    fn spawn_supervision(
        self: &Arc<Self>,
        spec: SidecarSpec,
        mut exit_rx: mpsc::UnboundedReceiver<ExitInfo>,
    ) {
        let supervisor = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let Some(info) = exit_rx.recv().await else {
                    break; // 通道关闭（所有发送方已结束）
                };
                if info.intentional {
                    supervisor.sink.log(
                        &spec.plugin_id,
                        "info",
                        &format!("sidecar 已停止（{}）", info.reason),
                    );
                    break;
                }
                // 非主动退出 = 崩溃
                supervisor.sink.log(
                    &spec.plugin_id,
                    "warn",
                    &format!("sidecar 崩溃：{}（code={:?}）", info.reason, info.code),
                );
                if spec.restart_policy != RestartPolicy::OnCrash {
                    supervisor.sessions.lock().await.remove(&spec.plugin_id);
                    supervisor.sink.log(
                        &spec.plugin_id,
                        "error",
                        "sidecar 已退出（restart_policy=off，不自动恢复）",
                    );
                    break;
                }
                // 重启窗口计数（1 小时内超 MAX_RESTARTS_PER_HOUR → 放弃）
                let attempts = {
                    let mut log = supervisor.restart_log.lock().await;
                    let stamps = log.entry(spec.plugin_id.clone()).or_default();
                    let now = Instant::now();
                    stamps.retain(|t| now.duration_since(*t) < Duration::from_secs(3600));
                    stamps.push(now);
                    stamps.len() as u32
                };
                if attempts > MAX_RESTARTS_PER_HOUR {
                    supervisor.sessions.lock().await.remove(&spec.plugin_id);
                    supervisor.sink.log(
                        &spec.plugin_id,
                        "error",
                        &format!(
                            "sidecar 重启次数超限（1 小时内 {MAX_RESTARTS_PER_HOUR} 次），\
                             停止自动恢复；请检查插件日志后手动重启"
                        ),
                    );
                    break;
                }
                let shift = (attempts - 1).min(4);
                let backoff = self_backoff(&supervisor.timing, shift);
                supervisor.sink.log(
                    &spec.plugin_id,
                    "info",
                    &format!("sidecar 将在 {backoff:?} 后重启（第 {attempts} 次）"),
                );
                tokio::time::sleep(backoff).await;
                // 换代重启
                match supervisor.spawn_session(spec.clone()).await {
                    Ok((session, new_rx)) => {
                        let restarts = session.restarts_last_hour.clone();
                        restarts.store(attempts, Ordering::Relaxed);
                        supervisor
                            .sessions
                            .lock()
                            .await
                            .insert(spec.plugin_id.clone(), Arc::new(session));
                        exit_rx = new_rx;
                        continue;
                    }
                    Err(e) => {
                        supervisor.sessions.lock().await.remove(&spec.plugin_id);
                        supervisor.sink.log(
                            &spec.plugin_id,
                            "error",
                            &format!("sidecar 重启失败，停止自动恢复：{}", e.user_message()),
                        );
                        break;
                    }
                }
            }
        });
    }
}

/// 指数退避（base << shift）。
fn self_backoff(timing: &SupervisorTiming, shift: u32) -> Duration {
    Duration::from_millis((timing.restart_backoff_base.as_millis() as u64) << shift)
}

/// Unix 毫秒时间戳。
fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 读帧任务：res → 挂起表回执；pong → 心跳时间戳；log/evt → sink。
/// 流结束（进程退出）即任务结束；挂起请求由 terminate / 心跳判死路径清理。
fn spawn_reader_task(
    mut reader: BufReader<tokio::process::ChildStdout>,
    pending: PendingMap,
    last_pong: Arc<Mutex<Instant>>,
    plugin_id: String,
    sink: Arc<dyn SidecarSink>,
) {
    tokio::spawn(async move {
        loop {
            let Ok(frame) = read_frame(&mut reader).await else {
                break; // EOF / 解析失败：进程退出或协议损坏，交给 wait 任务上报
            };
            match frame.kind.as_str() {
                "res" => {
                    if let Some(id) = &frame.id {
                        if let Some(tx) = pending.lock().await.remove(id) {
                            let payload = if frame.ok.unwrap_or(false) {
                                Ok(frame.result.unwrap_or(serde_json::Value::Null))
                            } else {
                                Err(frame
                                    .error
                                    .map(|e| format!("{}: {}", e.code, e.message))
                                    .unwrap_or_else(|| "unknown sidecar error".into()))
                            };
                            let _ = tx.send(payload);
                        }
                    }
                }
                "pong" => {
                    *last_pong.lock().await = Instant::now();
                }
                "log" => {
                    let level = frame.level.unwrap_or_else(|| "info".into());
                    let message = frame.message.unwrap_or_default();
                    sink.log(&plugin_id, &level, &message);
                }
                "evt" => {
                    let event = frame.event.unwrap_or_default();
                    let payload = frame.payload.unwrap_or(serde_json::Value::Null);
                    sink.event(&plugin_id, &event, payload);
                }
                _ => {
                    // 未知帧：记录但不中断（协议前向兼容）
                    sink.log(&plugin_id, "debug", &format!("忽略未知帧：{}", frame.kind));
                }
            }
        }
        // 流结束：清空全部挂起请求（进程已死，等待不再有意义）
        let mut map = pending.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(Err("sidecar 流已结束（进程退出）".into()));
        }
    });
}

/// stderr 采集任务：逐行转 sink.log（stderr 口径 = warn）。
fn spawn_stderr_task(
    stderr: tokio::process::ChildStderr,
    plugin_id: String,
    sink: Arc<dyn SidecarSink>,
) {
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stderr);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match lines.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&buf).trim().to_string();
            if !line.is_empty() {
                sink.log(&plugin_id, "warn", &format!("[stderr] {line}"));
            }
        }
    });
}

/// 心跳任务：周期 ping；pong 超时判死 → 上报崩溃 + 进程组 kill。
#[allow(clippy::too_many_arguments)]
fn spawn_heartbeat_task(
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    last_pong: Arc<Mutex<Instant>>,
    pid: u32,
    plugin_id: String,
    exit_tx: mpsc::UnboundedSender<ExitInfo>,
    timing: SupervisorTiming,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(timing.heartbeat_interval).await;
            // 判死：距最后一次 pong 超过心跳超时
            let stale = { last_pong.lock().await.elapsed() > timing.heartbeat_timeout };
            if stale {
                sink_dead(&plugin_id, &exit_tx);
                kill_process_group(pid);
                break;
            }
            let ping = SidecarFrame {
                id: Some(uuid::Uuid::new_v4().simple().to_string()),
                ..host_frame("ping")
            };
            let sent = {
                let mut s = stdin.lock().await;
                write_frame(&mut *s, &ping).await.is_ok()
            };
            if !sent {
                // stdin 已断（进程退出）：wait 任务会上报，这里只退出
                break;
            }
        }
    })
}

fn sink_dead(plugin_id: &str, exit_tx: &mpsc::UnboundedSender<ExitInfo>) {
    let _ = exit_tx.send(ExitInfo {
        code: None,
        intentional: false,
        reason: "心跳判死（pong 超时）".into(),
    });
    tracing::warn!("sidecar 心跳判死：{plugin_id}");
}

/// wait 任务：waitpid 回收（防僵尸）+ 退出码上报（崩溃恢复决策输入）。
fn spawn_wait_task(mut child: tokio::process::Child, exit_tx: mpsc::UnboundedSender<ExitInfo>) {
    tokio::spawn(async move {
        let status = child.wait().await;
        let code = status.ok().and_then(|s| s.code());
        let _ = exit_tx.send(ExitInfo {
            code,
            intentional: false,
            reason: "进程退出".into(),
        });
    });
}
