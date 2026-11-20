//! 任务注册表：状态机 / 取消 / 资源键守卫 / 历史环形缓冲。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use miyo_core::AppError;

/// 任务 ID（前端 uuid 或后端生成，全局唯一由调用方保证——重复 id 的 register
/// 会覆盖旧任务并令其 Cancelled）。
pub type TaskId = String;

/// 任务种类（审计与前端展示用；不参与状态机逻辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    HttpRequest,
    GrpcUnary,
    GrpcStream,
    LoadTest,
    TestCollection,
    TestEndpoint,
    ScenarioRun,
    ScenarioMatrix,
    AiChat,
    WsSession,
    SseSession,
    MockService,
    AgentService,
    JavaAttach,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::HttpRequest => "http_request",
            TaskKind::GrpcUnary => "grpc_unary",
            TaskKind::GrpcStream => "grpc_stream",
            TaskKind::LoadTest => "load_test",
            TaskKind::TestCollection => "test_collection",
            TaskKind::TestEndpoint => "test_endpoint",
            TaskKind::ScenarioRun => "scenario_run",
            TaskKind::ScenarioMatrix => "scenario_matrix",
            TaskKind::AiChat => "ai_chat",
            TaskKind::WsSession => "ws_session",
            TaskKind::SseSession => "sse_session",
            TaskKind::MockService => "mock_service",
            TaskKind::AgentService => "agent_service",
            TaskKind::JavaAttach => "java_attach",
        }
    }
}

/// 任务状态机（任务书定义六态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Cancelled | TaskState::Completed | TaskState::Failed
        )
    }
}

/// 取消结果（三语义统一：remove 型 / get+clone 型 / 报错型收敛于此）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// 从运行中取消成功（令牌已触发）。
    Cancelled,
    /// 任务已处于终态（再次取消无效果——幂等成功语义）。
    AlreadyFinished,
    /// 任务不存在。
    NotFound,
}

/// 任务快照（status 查询 / 历史记录条目）。
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub id: TaskId,
    pub kind: TaskKind,
    /// 资源键（并发守卫：同键运行中拒绝重入；None = 不守卫）。
    pub resource_key: Option<String>,
    pub state: TaskState,
    /// Unix 毫秒（启动时刻）。
    pub started_at_ms: i64,
    /// Unix 毫秒（进入终态时刻；None = 运行中）。
    pub finished_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

/// 取消令牌 + 元数据（注册表条目）。
struct TaskEntry {
    kind: TaskKind,
    resource_key: Option<String>,
    token: CancellationToken,
    state: TaskState,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    last_error: Option<String>,
    /// 取消时需要额外执行的自定义清理（如 WS forward abort / graceful stop）。
    on_cancel: Option<Box<dyn FnOnce() + Send>>,
}

/// RAII 句柄：Drop 时若任务仍登记在册且非终态 → 置 Cancelled 并摘除。
/// 消灭「命令 future 被 drop → 注册表条目泄漏」整类问题。
pub struct TaskHandle {
    supervisor: Arc<TaskSupervisor>,
    id: TaskId,
    /// 本任务自己的令牌快照（duplicate-id 替换后仍指向正确的令牌）。
    token: CancellationToken,
    /// 手动完成标记（AtomicBool：TaskHandle 以 Arc 共享需 Send+Sync）。
    finished: std::sync::atomic::AtomicBool,
}

impl TaskHandle {
    /// 正常完成（Completed 终态；Drop 不再改写）。
    pub fn mark_completed(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.supervisor.finish(&self.id, TaskState::Completed, None);
    }

    /// 失败终态（记录 last_error）。
    pub fn mark_failed(&self, err: impl Into<String>) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let msg = err.into();
        self.supervisor
            .finish(&self.id, TaskState::Failed, Some(msg));
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// 取消令牌（spawn 的任务体轮询 `token.is_cancelled()` 协作退出）。
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        if !self.finished.load(std::sync::atomic::Ordering::Relaxed) {
            self.supervisor.finish(
                &self.id,
                TaskState::Cancelled,
                Some("任务句柄被丢弃".into()),
            );
        }
    }
}

/// TaskSupervisor（AppState 持有 Arc；全部走 &self + 内部 std Mutex 短临界区）。
pub struct TaskSupervisor {
    inner: Mutex<TaskRegistryInner>,
    /// 历史环形缓冲（每 kind 上限见 [`HISTORY_CAP`]；终态时写入）。
    history: Mutex<HashMap<TaskKind, VecDeque<TaskRecord>>>,
    /// 资源键 → 任务 id（并发守卫；终态时移除）。
    resource_keys: Mutex<HashMap<String, TaskId>>,
}

struct TaskRegistryInner {
    tasks: HashMap<TaskId, TaskEntry>,
}

/// 历史环形缓冲每 kind 容量。
pub const HISTORY_CAP: usize = 100;

// TaskSupervisor 只含 std Mutex<HashMap>（Send+Sync 数据），跨线程共享安全。
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TaskSupervisor>();
};

impl TaskSupervisor {
    pub fn new() -> Arc<Self> {
        Arc::new(TaskSupervisor {
            inner: Mutex::new(TaskRegistryInner {
                tasks: HashMap::new(),
            }),
            history: Mutex::new(HashMap::new()),
            resource_keys: Mutex::new(HashMap::new()),
        })
    }

    /// 取消（幂等）：触发令牌 + 自定义清理 → 立即终态化（Cancelled 进历史、
    /// 摘出注册表、释放资源键）。任务体经令牌协作退出；重复取消返回
    /// AlreadyFinished。
    pub fn cancel(&self, id: &str) -> CancelOutcome {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(mut entry) = inner.tasks.remove(id) else {
            // 已终态摘出（历史里有）→ 幂等成功；从未注册 → NotFound
            if self.has_history(id) {
                return CancelOutcome::AlreadyFinished;
            }
            return CancelOutcome::NotFound;
        };
        if entry.state.is_terminal() {
            return CancelOutcome::AlreadyFinished;
        }
        entry.state = TaskState::Cancelled;
        entry.finished_at_ms = Some(unix_ms());
        entry.token.cancel();
        if let Some(on_cancel) = entry.on_cancel.take() {
            on_cancel();
        }
        if let Some(key) = &entry.resource_key {
            self.resource_keys
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(key);
        }
        let record = TaskRecord {
            id: id.to_string(),
            kind: entry.kind,
            resource_key: entry.resource_key,
            state: TaskState::Cancelled,
            started_at_ms: entry.started_at_ms,
            finished_at_ms: entry.finished_at_ms,
            last_error: entry.last_error,
        };
        let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
        let deque = history.entry(entry.kind).or_default();
        if deque.len() >= HISTORY_CAP {
            deque.pop_front();
        }
        deque.push_back(record);
        CancelOutcome::Cancelled
    }

    /// 注册取消时的自定义清理钩子（如 WS forward abort；每任务一次）。
    pub fn set_on_cancel(&self, id: &str, hook: Box<dyn FnOnce() + Send>) -> Result<(), AppError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(entry) = inner.tasks.get_mut(id) else {
            return Err(AppError::NotFound(format!("任务不存在：{id}")));
        };
        entry.on_cancel = Some(hook);
        Ok(())
    }

    /// 任务快照。
    pub fn status(&self, id: &str) -> Option<TaskRecord> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.tasks.get(id).map(|e| TaskRecord {
            id: id.to_string(),
            kind: e.kind,
            resource_key: e.resource_key.clone(),
            state: e.state,
            started_at_ms: e.started_at_ms,
            finished_at_ms: e.finished_at_ms,
            last_error: e.last_error.clone(),
        })
    }

    /// 全部运行中任务快照（管理中心 / 退出前诊断）。
    pub fn running(&self) -> Vec<TaskRecord> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .tasks
            .values()
            .filter(|e| e.state == TaskState::Running)
            .map(|e| TaskRecord {
                id: "n/a".into(),
                kind: e.kind,
                resource_key: e.resource_key.clone(),
                state: e.state,
                started_at_ms: e.started_at_ms,
                finished_at_ms: e.finished_at_ms,
                last_error: e.last_error.clone(),
            })
            .collect()
    }

    /// 历史（新→旧，每 kind 上限 [`HISTORY_CAP`]）。
    pub fn history(&self, kind: TaskKind) -> Vec<TaskRecord> {
        self.history
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&kind)
            .map(|d| d.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 终态化并写历史（mark_* 与 RAII Drop 共用入口）。
    pub fn finish(&self, id: &str, state: TaskState, error: Option<String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut entry) = inner.tasks.remove(id) {
            entry.state = state;
            entry.finished_at_ms = Some(unix_ms());
            entry.last_error = error;
            if let Some(key) = &entry.resource_key {
                self.resource_keys
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(key);
            }
            let record = TaskRecord {
                id: id.to_string(),
                kind: entry.kind,
                resource_key: entry.resource_key,
                state: entry.state,
                started_at_ms: entry.started_at_ms,
                finished_at_ms: entry.finished_at_ms,
                last_error: entry.last_error,
            };
            let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
            let deque = history.entry(entry.kind).or_default();
            if deque.len() >= HISTORY_CAP {
                deque.pop_front();
            }
            deque.push_back(record);
        }
    }

    /// 应用退出清理：触发全部在途任务取消令牌 + 自定义钩子（进程随 runtime
    /// 结束，无需等待）。
    pub fn shutdown_all(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        for entry in inner.tasks.values_mut() {
            entry.token.cancel();
            entry.state = TaskState::Cancelling;
            if let Some(on_cancel) = entry.on_cancel.take() {
                on_cancel();
            }
        }
    }

    fn has_history(&self, id: &str) -> bool {
        self.history
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .any(|d| d.iter().any(|r| r.id == id))
    }
}

impl TaskSupervisor {
    pub fn start(
        self: &Arc<Self>,
        id: impl Into<TaskId>,
        kind: TaskKind,
        resource_key: Option<String>,
    ) -> Result<Arc<TaskHandle>, AppError> {
        let id = id.into();
        let token = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(key) = &resource_key {
                let keys = self.resource_keys.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(existing) = keys.get(key) {
                    let _ = existing;
                    return Err(AppError::Validation(format!(
                        "同类任务（{key}）正在运行中：请先取消或等待完成"
                    )));
                }
            }
            if let Some(old) = inner.tasks.remove(&id) {
                old.token.cancel();
                if let Some(key) = &old.resource_key {
                    self.resource_keys
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(key);
                }
            }
            if let Some(key) = &resource_key {
                self.resource_keys
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(key.clone(), id.clone());
            }
            let token = CancellationToken::new();
            inner.tasks.insert(
                id.clone(),
                TaskEntry {
                    kind,
                    resource_key: resource_key.clone(),
                    token: token.clone(),
                    state: TaskState::Running,
                    started_at_ms: unix_ms(),
                    finished_at_ms: None,
                    last_error: None,
                    on_cancel: None,
                },
            );
            token
        };
        Ok(Arc::new(TaskHandle {
            supervisor: Arc::clone(self),
            id,
            token,
            finished: std::sync::atomic::AtomicBool::new(false),
        }))
    }
}

/// 供 [`TaskSupervisor::start`]（非 Arc 版）使用的占位——编译期禁止：
fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
