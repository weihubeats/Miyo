//! 批量执行调度引擎（与 Tauri / 存储解耦）。
//!
//! 只负责「按策略调度场景」：串行 / 并行（波次并发）、失败继续 / 停止 /
//! 重试 / 跳过后续、暂停与取消；单场景的物化与执行由调用方通过
//! [`execute_batch`] 的 `execute` 闭包注入（复用现有 `run_scenario_lifecycle`）。
//!
//! 这样调度状态机可以脱离 Tauri / HTTP 单测：调用方传入假执行器即可覆盖
//! 串行、并行、失败策略、重试、取消、暂停与中断恢复的语义。
//!
//! 暂停语义（不伪造「任意时刻冻结」）：只在**波次之间**停止调度——串行时
//! 当前场景跑完即停；并行时当前波次内已在途的场景跑完即停，未开始的保持
//! pending，恢复后继续。取消通过 [`CancellationToken`] 传给执行器，由
//! 单场景引擎在步间 / 在途请求处协作式响应。

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::future::join_all;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// 执行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchMode {
    Serial,
    Parallel,
}

/// 失败处理策略（与 miyo-core `TestFailurePolicy` 同义，引擎侧独立定义以
/// 避免调度层依赖持久化模型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicy {
    Continue,
    Stop,
    Retry,
    SkipDependents,
}

/// 批次执行策略。
#[derive(Debug, Clone)]
pub struct BatchPolicy {
    pub mode: BatchMode,
    /// 并行最大并发（串行恒按 1 处理；0 / 负数由调用方收钳）。
    pub concurrency: usize,
    pub failure_policy: FailurePolicy,
    /// 失败后额外重试次数（策略为 Retry 时生效）。
    pub retry_count: u32,
}

impl BatchPolicy {
    /// 串行 + 失败继续（默认策略）。
    pub fn serial() -> Self {
        BatchPolicy {
            mode: BatchMode::Serial,
            concurrency: 1,
            failure_policy: FailurePolicy::Continue,
            retry_count: 0,
        }
    }

    fn wave_size(&self) -> usize {
        match self.mode {
            BatchMode::Serial => 1,
            BatchMode::Parallel => self.concurrency.max(1),
        }
    }
}

/// 一个待执行的场景（顺序即套件顺序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPlanItem {
    /// 批次明细 id（调用方持久化后回填结果用）。
    pub item_id: String,
    pub scenario_id: Uuid,
    pub name: String,
}

/// 单场景执行结果状态（跳过 / 取消 / 中断与真实失败严格区分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Passed,
    Failed,
    Skipped,
    Cancelled,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Passed => "passed",
            ItemStatus::Failed => "failed",
            ItemStatus::Skipped => "skipped",
            ItemStatus::Cancelled => "cancelled",
        }
    }
}

/// 单场景执行结果（由 `execute` 闭包返回；引擎补 attempts）。
#[derive(Debug, Clone)]
pub struct ItemOutcome {
    pub item_id: String,
    pub scenario_id: Uuid,
    pub status: ItemStatus,
    /// 实际执行次数（1 = 未重试；重试策略下可 > 1）。
    pub attempts: u32,
    pub duration_ms: f64,
    /// 失败原因（首条失败步骤 / 执行错误；非失败为 None）。
    pub error: Option<String>,
    /// 单场景执行记录 id（scenario_runs.id）。
    pub run_id: Option<String>,
}

impl ItemOutcome {
    pub fn passed(item: &BatchPlanItem) -> Self {
        ItemOutcome {
            item_id: item.item_id.clone(),
            scenario_id: item.scenario_id,
            status: ItemStatus::Passed,
            attempts: 1,
            duration_ms: 0.0,
            error: None,
            run_id: None,
        }
    }

    pub fn failed(item: &BatchPlanItem, error: impl Into<String>) -> Self {
        ItemOutcome {
            item_id: item.item_id.clone(),
            scenario_id: item.scenario_id,
            status: ItemStatus::Failed,
            attempts: 1,
            duration_ms: 0.0,
            error: Some(error.into()),
            run_id: None,
        }
    }
}

/// 批次进度快照（UI 进度条 / 计数条单源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchProgress {
    pub total: usize,
    pub done: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: usize,
    /// 当前波次在途数（波次结束为 0）。
    pub running: usize,
}

impl BatchProgress {
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.done * 100) / self.total) as u32
    }
}

/// 批次执行汇总。
#[derive(Debug, Clone)]
pub struct BatchSummary {
    /// 已产生结果的明细（顺序 = 实际完成顺序，串行时与计划顺序一致）。
    pub outcomes: Vec<ItemOutcome>,
    /// 暂停时仍未开始的明细（保持 pending，恢复后继续）。
    pub remaining: Vec<BatchPlanItem>,
    pub paused: bool,
    pub cancelled: bool,
    /// 因失败策略停止调度（剩余项已按 skipped 计入 outcomes）。
    pub stop_triggered: bool,
}

/// 执行批次。
///
/// * `items`：按套件顺序排列的待执行场景；
/// * `execute(item, attempt)`：执行单个场景（attempt 从 1 起；重试时递增）；
/// * `on_start`：每个场景进入执行前回调（发「当前正在执行」事件）；
/// * `on_progress`：每个波次结束后回调（发批次进度事件）。
pub async fn execute_batch<F, Fut, S, P>(
    items: Vec<BatchPlanItem>,
    policy: BatchPolicy,
    cancel: CancellationToken,
    paused: Arc<AtomicBool>,
    execute: F,
    mut on_start: S,
    mut on_progress: P,
) -> BatchSummary
where
    F: Fn(BatchPlanItem, u32) -> Fut + Clone,
    Fut: Future<Output = ItemOutcome>,
    S: FnMut(&BatchPlanItem),
    P: FnMut(BatchProgress),
{
    let total = items.len();
    let wave_size = policy.wave_size();
    let mut pending: VecDeque<BatchPlanItem> = items.into();
    let mut outcomes: Vec<ItemOutcome> = Vec::with_capacity(total);
    let mut paused_out = false;
    let mut cancelled_out = false;
    let mut stop_triggered = false;

    loop {
        if cancel.is_cancelled() {
            cancelled_out = true;
            break;
        }
        if paused.load(Ordering::SeqCst) {
            paused_out = true;
            break;
        }
        if stop_triggered {
            break;
        }
        let mut wave: Vec<BatchPlanItem> = Vec::with_capacity(wave_size);
        while wave.len() < wave_size {
            match pending.pop_front() {
                Some(item) => wave.push(item),
                None => break,
            }
        }
        if wave.is_empty() {
            break;
        }
        for item in &wave {
            on_start(item);
        }
        on_progress(progress_of(&outcomes, total, wave.len()));

        let futs = wave.into_iter().map(|item| {
            let cancel = cancel.clone();
            let policy = policy.clone();
            let execute = execute.clone();
            async move {
                let mut attempt = 0u32;
                loop {
                    attempt += 1;
                    if cancel.is_cancelled() {
                        return ItemOutcome {
                            item_id: item.item_id.clone(),
                            scenario_id: item.scenario_id,
                            status: ItemStatus::Cancelled,
                            attempts: attempt,
                            duration_ms: 0.0,
                            error: None,
                            run_id: None,
                        };
                    }
                    let mut out = execute(item.clone(), attempt).await;
                    out.attempts = attempt;
                    let retryable = out.status == ItemStatus::Failed
                        && policy.failure_policy == FailurePolicy::Retry
                        && attempt <= policy.retry_count;
                    if !retryable {
                        return out;
                    }
                }
            }
        });
        let wave_outcomes = join_all(futs).await;
        for out in &wave_outcomes {
            if out.status == ItemStatus::Failed
                && matches!(
                    policy.failure_policy,
                    FailurePolicy::Stop | FailurePolicy::SkipDependents
                )
            {
                stop_triggered = true;
            }
        }
        outcomes.extend(wave_outcomes);
        on_progress(progress_of(&outcomes, total, 0));
    }

    // 未执行的剩余项：取消 → cancelled；失败停止 → skipped；暂停 → 保持 pending。
    let mut remaining: Vec<BatchPlanItem> = pending.into_iter().collect();
    if cancelled_out {
        for item in remaining.drain(..) {
            outcomes.push(ItemOutcome {
                item_id: item.item_id,
                scenario_id: item.scenario_id,
                status: ItemStatus::Cancelled,
                attempts: 0,
                duration_ms: 0.0,
                error: None,
                run_id: None,
            });
        }
    } else if stop_triggered {
        for item in remaining.drain(..) {
            outcomes.push(ItemOutcome {
                item_id: item.item_id,
                scenario_id: item.scenario_id,
                status: ItemStatus::Skipped,
                attempts: 0,
                duration_ms: 0.0,
                error: None,
                run_id: None,
            });
        }
    }
    on_progress(progress_of(&outcomes, total, 0));

    BatchSummary {
        outcomes,
        remaining,
        paused: paused_out,
        cancelled: cancelled_out,
        stop_triggered,
    }
}

fn progress_of(outcomes: &[ItemOutcome], total: usize, running: usize) -> BatchProgress {
    let mut p = BatchProgress {
        total,
        done: outcomes.len(),
        running,
        ..Default::default()
    };
    for out in outcomes {
        match out.status {
            ItemStatus::Passed => p.passed += 1,
            ItemStatus::Failed => p.failed += 1,
            ItemStatus::Skipped => p.skipped += 1,
            ItemStatus::Cancelled => p.cancelled += 1,
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn plan(n: usize) -> Vec<BatchPlanItem> {
        (0..n)
            .map(|i| BatchPlanItem {
                item_id: format!("item-{i}"),
                scenario_id: Uuid::new_v4(),
                name: format!("场景 {i}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn serial_runs_all_in_order() {
        let items = plan(8);
        let summary = execute_batch(
            items.clone(),
            BatchPolicy::serial(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| async move { ItemOutcome::passed(&item) },
            |_| {},
            |_p| {},
        )
        .await;
        assert_eq!(summary.outcomes.len(), 8);
        assert!(summary
            .outcomes
            .iter()
            .all(|o| o.status == ItemStatus::Passed));
        assert_eq!(
            summary
                .outcomes
                .iter()
                .map(|o| o.item_id.clone())
                .collect::<Vec<_>>(),
            items.iter().map(|i| i.item_id.clone()).collect::<Vec<_>>()
        );
        assert!(summary.remaining.is_empty());
        assert!(!summary.paused && !summary.cancelled && !summary.stop_triggered);
    }

    #[tokio::test]
    async fn parallel_respects_max_concurrency() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let summary = execute_batch(
            plan(8),
            BatchPolicy {
                mode: BatchMode::Parallel,
                concurrency: 3,
                failure_policy: FailurePolicy::Continue,
                retry_count: 0,
            },
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            {
                let in_flight = in_flight.clone();
                let peak = peak.clone();
                move |item, _attempt| {
                    let in_flight = in_flight.clone();
                    let peak = peak.clone();
                    async move {
                        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(15)).await;
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        ItemOutcome::passed(&item)
                    }
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert_eq!(summary.outcomes.len(), 8);
        assert!(summary
            .outcomes
            .iter()
            .all(|o| o.status == ItemStatus::Passed));
        assert!(peak.load(Ordering::SeqCst) <= 3, "并发不得超过上限");
        assert!(peak.load(Ordering::SeqCst) > 1, "并行应真正并发");
    }

    #[tokio::test]
    async fn failure_continue_keeps_going() {
        let summary = execute_batch(
            plan(4),
            BatchPolicy::serial(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| async move {
                if item.item_id == "item-1" {
                    ItemOutcome::failed(&item, "boom")
                } else {
                    ItemOutcome::passed(&item)
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert_eq!(summary.outcomes.len(), 4);
        assert_eq!(summary.outcomes[1].status, ItemStatus::Failed);
        assert_eq!(summary.outcomes[2].status, ItemStatus::Passed);
        assert_eq!(summary.outcomes[3].status, ItemStatus::Passed);
        assert!(!summary.stop_triggered);
    }

    #[tokio::test]
    async fn failure_stop_skips_remaining() {
        let summary = execute_batch(
            plan(5),
            BatchPolicy {
                failure_policy: FailurePolicy::Stop,
                ..BatchPolicy::serial()
            },
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| async move {
                if item.item_id == "item-1" {
                    ItemOutcome::failed(&item, "boom")
                } else {
                    ItemOutcome::passed(&item)
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert!(summary.stop_triggered);
        assert_eq!(summary.outcomes.len(), 5);
        assert_eq!(summary.outcomes[0].status, ItemStatus::Passed);
        assert_eq!(summary.outcomes[1].status, ItemStatus::Failed);
        assert!(summary.outcomes[2..]
            .iter()
            .all(|o| o.status == ItemStatus::Skipped));
        assert!(summary.remaining.is_empty());
    }

    #[tokio::test]
    async fn failure_retry_attempts_then_continues() {
        let attempts = Arc::new(std::sync::Mutex::new(Vec::<u32>::new()));
        let summary = execute_batch(
            plan(2),
            BatchPolicy {
                failure_policy: FailurePolicy::Retry,
                retry_count: 2,
                ..BatchPolicy::serial()
            },
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            {
                let attempts = attempts.clone();
                move |item, attempt| {
                    let attempts = attempts.clone();
                    async move {
                        attempts.lock().unwrap().push(attempt);
                        if item.item_id == "item-0" && attempt < 3 {
                            ItemOutcome::failed(&item, "flaky")
                        } else {
                            ItemOutcome::passed(&item)
                        }
                    }
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert_eq!(summary.outcomes[0].status, ItemStatus::Passed);
        assert_eq!(summary.outcomes[0].attempts, 3);
        assert_eq!(summary.outcomes[1].status, ItemStatus::Passed);
        assert_eq!(summary.outcomes[1].attempts, 1);
        assert_eq!(*attempts.lock().unwrap(), vec![1, 2, 3, 1]);
    }

    #[tokio::test]
    async fn cancel_marks_pending_cancelled() {
        let cancel = CancellationToken::new();
        let summary = execute_batch(
            plan(4),
            BatchPolicy::serial(),
            cancel.clone(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| {
                let cancel = cancel.clone();
                async move {
                    // 第一个场景执行时触发取消：当前项由执行器返回 cancelled，
                    // 后续未开始项由引擎标记 cancelled。
                    cancel.cancel();
                    ItemOutcome {
                        item_id: item.item_id,
                        scenario_id: item.scenario_id,
                        status: ItemStatus::Cancelled,
                        attempts: 1,
                        duration_ms: 0.0,
                        error: None,
                        run_id: None,
                    }
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert!(summary.cancelled);
        assert_eq!(summary.outcomes.len(), 4);
        assert!(summary
            .outcomes
            .iter()
            .all(|o| o.status == ItemStatus::Cancelled));
    }

    #[tokio::test]
    async fn pause_keeps_remaining_pending() {
        let paused = Arc::new(AtomicBool::new(true));
        let summary = execute_batch(
            plan(3),
            BatchPolicy::serial(),
            CancellationToken::new(),
            paused,
            |item, _attempt| async move { ItemOutcome::passed(&item) },
            |_| {},
            |_p| {},
        )
        .await;
        assert!(summary.paused);
        assert!(summary.outcomes.is_empty());
        assert_eq!(summary.remaining.len(), 3);
    }

    #[tokio::test]
    async fn skip_dependents_stops_without_failed_batch() {
        let summary = execute_batch(
            plan(3),
            BatchPolicy {
                failure_policy: FailurePolicy::SkipDependents,
                ..BatchPolicy::serial()
            },
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| async move {
                if item.item_id == "item-0" {
                    ItemOutcome::failed(&item, "boom")
                } else {
                    ItemOutcome::passed(&item)
                }
            },
            |_| {},
            |_p| {},
        )
        .await;
        assert!(summary.stop_triggered);
        assert!(summary.outcomes[1..]
            .iter()
            .all(|o| o.status == ItemStatus::Skipped));
    }

    #[tokio::test]
    async fn progress_reports_counts_and_percent() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<BatchProgress>::new()));
        let _ = execute_batch(
            plan(2),
            BatchPolicy::serial(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            |item, _attempt| async move { ItemOutcome::passed(&item) },
            |_| {},
            {
                let seen = seen.clone();
                move |p| seen.lock().unwrap().push(p)
            },
        )
        .await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.last().unwrap().done, 2);
        assert_eq!(seen.last().unwrap().passed, 2);
        assert_eq!(seen.last().unwrap().percent(), 100);
    }
}
