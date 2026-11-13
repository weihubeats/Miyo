//! M14：接口压测（并发基准测试）。
//!
//! 调度模型：固定 `concurrency` 个 worker 任务 + 原子计数器领任务，
//! 结果经 mpsc 回传聚合。任务句柄数恒为并发数（原来 `spawn(total)`，
//! total=10 万时 10 万句柄常驻），内存占用与总请求数解耦。
//!
//! 调度模式（[`LoadSchedule`]）：
//! - `Fixed`：固定总量跑满 `total` 次（原行为，向后兼容）；
//! - `FixedQps`：令牌桶限速（10ms tick 批量放许可，把突发平摊进窗口），
//!   持续 `duration_secs` 秒；
//! - `Ramp`：QPS 从 start 匀速抬升到 end，覆盖整个运行时长；
//! - `Duration`：不限速、固定并发，跑满 `duration_secs` 秒。
//!
//! 另有：错误率熔断（[`LoadConfig::max_error_rate`]）、延迟直方图
//! （固定桶边界）、进程资源采样（CPU / 内存，均值 + 峰值）。
//! 协议适配说明（WS / SSE / gRPC 流式为何不能套用本 worker 循环）
//! 见 `docs/loadtest-protocol-assessment.md`。

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use miyo_core::model::{HttpMethod, RequestSpec};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use miyo_http::client::send_request_capped;

/// 压测调度模式。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LoadSchedule {
    /// 固定总量：并发跑满 `total` 次（原行为）。
    Fixed,
    /// 固定 QPS：以恒定速率发请求，持续 `duration_secs` 秒。
    FixedQps {
        /// 每秒目标请求数（1..=100_000）。
        qps: u64,
        /// 持续时长（秒，1..=3600）。
        duration_secs: u64,
    },
    /// 阶梯加压：QPS 从 start 匀速抬升到 end，覆盖整个运行时长。
    Ramp {
        /// 起始 QPS。
        start_qps: u64,
        /// 结束 QPS（≥ start）。
        end_qps: u64,
        /// 加压级数（1..=100）。
        steps: u32,
        /// 每级持续时间（秒）；总时长 = steps × stage_secs。
        stage_secs: u64,
    },
    /// 持续压测：不限速、固定并发，跑满 `duration_secs` 秒。
    Duration {
        /// 持续时长（秒，1..=3600）。
        duration_secs: u64,
    },
}

impl LoadSchedule {
    /// 运行时长（Fixed 模式返回 None=按总量跑）。
    pub fn duration_secs(&self) -> Option<u64> {
        match *self {
            LoadSchedule::Fixed => None,
            LoadSchedule::FixedQps { duration_secs, .. } => Some(duration_secs),
            LoadSchedule::Ramp {
                stage_secs, steps, ..
            } => Some(stage_secs.saturating_mul(steps as u64)),
            LoadSchedule::Duration { duration_secs } => Some(duration_secs),
        }
    }

    /// 收钳到安全范围。
    pub fn sanitized(&self) -> Self {
        match *self {
            LoadSchedule::FixedQps { qps, duration_secs } => LoadSchedule::FixedQps {
                qps: qps.clamp(1, 100_000),
                duration_secs: duration_secs.clamp(1, 3600),
            },
            LoadSchedule::Ramp {
                start_qps,
                end_qps,
                steps,
                stage_secs,
            } => {
                let start_qps = start_qps.clamp(1, 100_000);
                LoadSchedule::Ramp {
                    start_qps,
                    end_qps: end_qps.clamp(start_qps, 100_000),
                    steps: steps.clamp(1, 100),
                    stage_secs: stage_secs.clamp(1, 600),
                }
            }
            LoadSchedule::Duration { duration_secs } => LoadSchedule::Duration {
                duration_secs: duration_secs.clamp(1, 3600),
            },
            Fixed => Fixed,
        }
    }
}

use LoadSchedule::Fixed;

/// 压测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LoadConfig {
    /// 并发数（同时进行的请求数）。
    pub concurrency: usize,
    /// 总请求数（仅 Fixed 模式生效；其余模式按时长跑）。
    pub total: usize,
    /// 调度模式；None = [`LoadSchedule::Fixed`]（固定总量，向后兼容）。
    pub schedule: Option<LoadSchedule>,
    /// 最大错误率熔断阈值（0.0..=1.0）；完成数 ≥ 10 且错误率超限即中止，
    /// 结果 `aborted_reason = Some("error_rate")`。None = 不熔断。
    pub max_error_rate: Option<f64>,
}

/// 延迟分布桶（`le_ms = None` 表示 +∞ 溢出桶）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LatencyBucket {
    /// 桶上界（毫秒，含）；None = 无上界（溢出桶）。
    pub le_ms: Option<u64>,
    /// 落入该桶的样本数。
    pub count: u64,
}

/// 直方图桶边界（毫秒）；超出最大边界的样本进 +∞ 桶。
pub const HISTOGRAM_EDGES_MS: [u64; 10] = [10, 25, 50, 100, 200, 500, 1000, 2000, 5000, 10000];

/// 压测结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LoadResult {
    pub total: usize,
    pub ok: usize,
    pub failed: usize,
    pub total_ms: u64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub rps: f64,
    /// 最多保留 5 条请求错误。
    pub errors: Vec<String>,
    /// 是否被用户中途取消（取消时 total < 配置的 total）。
    #[serde(default)]
    pub cancelled: bool,
    /// 延迟直方图（固定桶边界，[`HISTOGRAM_EDGES_MS`] + 溢出桶）。
    #[serde(default)]
    pub histogram: Vec<LatencyBucket>,
    /// 熔断原因（`error_rate` = 错误率超限；None = 未熔断）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted_reason: Option<String>,
    /// 进程 CPU 占用均值 / 峰值（百分比，单核口径）。
    #[serde(default)]
    pub res_cpu_avg: f64,
    #[serde(default)]
    pub res_cpu_peak: f64,
    /// 进程内存均值 / 峰值（MB）。
    #[serde(default)]
    pub res_mem_avg_mb: f64,
    #[serde(default)]
    pub res_mem_peak_mb: f64,
}

/// 压测运行选项（`run_load` 保持旧默认行为；精细控制用 `run_load_with`）。
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// 单请求超时毫秒数；None 时走 miyo-http 默认。
    pub timeout_ms: Option<u64>,
    /// 取消令牌；触发后 worker 停止领新任务、在途请求中止。
    pub cancel: Option<CancellationToken>,
    /// 单响应体读取上限；默认 64KB（压测只关心状态码/耗时，
    /// 避免 500 并发 × 大响应体的内存爆炸；耗时口径为"到截断为止"）。
    pub body_cap_bytes: Option<usize>,
}

impl LoadOptions {
    fn body_cap(&self) -> usize {
        self.body_cap_bytes.unwrap_or(64 * 1024)
    }
}

fn percentile(sorted: &[f64], q: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (sorted.len() * q / 100).min(sorted.len() - 1);
    sorted[idx]
}

/// 压测进度快照（回调参数）。
///
/// `total = 0` 表示不定总量模式（QPS / Ramp / Duration 按时长跑），
/// 前端进度条按不定态渲染。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LoadProgress {
    pub done: usize,
    pub total: usize,
    pub ok: usize,
    pub failed: usize,
    /// 已运行毫秒。
    pub elapsed_ms: u64,
    /// 近 2 秒滑动窗口吞吐（req/s）。
    pub rps: f64,
    /// 累计错误率（failed / done；done = 0 时为 0）。
    pub error_rate: f64,
}

/// 单次请求采样（worker → 聚合器）。
struct Sample {
    ok: bool,
    duration_ms: f64,
    err: Option<String>,
}

/// 进程资源采样器：500ms 周期采本进程 CPU% / 内存 MB，均值 + 峰值。
/// `System` 构造失败（平台错误）时为 None，采样降级为空。
struct ResourceSampler {
    sys: Mutex<Option<sysinfo::System>>,
    samples: Mutex<Vec<(f64, f64)>>,
}

impl ResourceSampler {
    fn new() -> Self {
        let mut sys = sysinfo::System::new().ok();
        let pid = sysinfo::Pid::from_u32(std::process::id());
        // CPU 占用需两次刷新差分；首次预热基线。
        if let Some(sys) = sys.as_mut() {
            let _ = sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        }
        ResourceSampler {
            sys: Mutex::new(sys),
            samples: Mutex::new(Vec::new()),
        }
    }

    fn sample(&self) {
        let mut guard = self.sys.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sys) = guard.as_mut() else {
            return;
        };
        let pid = sysinfo::Pid::from_u32(std::process::id());
        if sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true) == 0 {
            return;
        }
        if let Some(proc) = sys.process(pid) {
            let cpu = proc.cpu_usage() as f64;
            let mem_mb = proc.memory() as f64 / 1024.0 / 1024.0;
            self.samples
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((cpu, mem_mb));
        }
    }

    fn summary(&self) -> (f64, f64, f64, f64) {
        let samples = self
            .samples
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if samples.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let cpu_avg = samples.iter().map(|(c, _)| *c).sum::<f64>() / samples.len() as f64;
        let mem_avg = samples.iter().map(|(_, m)| *m).sum::<f64>() / samples.len() as f64;
        let cpu_peak = samples.iter().map(|(c, _)| *c).fold(0.0, f64::max);
        let mem_peak = samples.iter().map(|(_, m)| *m).fold(0.0, f64::max);
        (cpu_avg, cpu_peak, mem_avg, mem_peak)
    }
}

/// 并发压测：`total` 次请求，最多 `concurrency` 个同时进行。
/// `progress` 可选：每完成一次请求回调一次快照（用于进度推送）。
///
/// 超时走 miyo-http 默认，body 上限 64KB，不可取消；需要精细控制用
/// [`run_load_with`]。
pub async fn run_load(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    cfg: &LoadConfig,
    progress: Option<&(dyn Fn(LoadProgress) + Send + Sync)>,
) -> LoadResult {
    run_load_with(method, url, spec, cfg, &LoadOptions::default(), progress).await
}

/// 精细版并发压测：固定 worker 池 + 共享请求规格 + 调度模式 + 可取消。
///
/// - `spec`/`url` 经 `Arc` 在 worker 间共享，不再逐请求深拷贝；
/// - worker 数恒为 `concurrency`，句柄与内存占用和请求总数解耦；
/// - 非 Fixed 模式经令牌桶限速 / 截止时间驱动，`total` 仅作上限保护；
/// - 取消（用户 / 熔断）后已完成样本保留，`cancelled=true`。
pub async fn run_load_with(
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    cfg: &LoadConfig,
    options: &LoadOptions,
    progress: Option<&(dyn Fn(LoadProgress) + Send + Sync)>,
) -> LoadResult {
    let concurrency = cfg.concurrency.clamp(1, 500).max(1);
    let schedule = cfg.schedule.map(|s| s.sanitized());
    let start = Instant::now();
    let spec = Arc::new(spec.clone());
    let url: Arc<str> = Arc::from(url);

    // 熔断用子令牌：父（用户取消）触发时子联动；熔断只取消子。
    let cancel = options
        .cancel
        .as_ref()
        .map(|p| p.child_token())
        .unwrap_or_default();
    let trip = cancel.clone();

    let deadline = schedule.and_then(|s| {
        s.duration_secs()
            .map(|secs| start + Duration::from_secs(secs))
    });
    let total_cap = if schedule.is_some() {
        usize::MAX // 时长模式：总量仅作溢出保护（u64 计数不会触顶）
    } else {
        cfg.total.clamp(1, 100_000).max(1)
    };

    // 限速游标（仅 QPS / Ramp 模式）：worker 发送前预约「下一个可发时刻」，
    // 全局槽位间隔 = 1/当前 qps（Ramp 随时间插值）。不用信号量——
    // SemaphorePermit 的 Drop-归还语义会让许可在 worker 间自我循环，限速失效。
    let qps_gate = match schedule {
        Some(s @ (LoadSchedule::FixedQps { .. } | LoadSchedule::Ramp { .. })) => {
            let _ = s;
            Some(Arc::new(Mutex::new(start)))
        }
        _ => None,
    };

    // 资源采样（500ms 周期；无样本时结果为 0）。
    let sampler = Arc::new(ResourceSampler::new());
    {
        let sampler = sampler.clone();
        let sample_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.tick().await;
            while !sample_cancel.is_cancelled() {
                sampler.sample();
                tick.tick().await;
            }
        });
    }

    // 缓冲 = worker 数 × 2：背压天然存在，无需额外信号量。
    let (tx, mut rx) = mpsc::channel::<Sample>(concurrency * 2);
    // 共享领任务计数器（Fixed 模式的总量上限；时长模式计数不参与退出判定）
    let next = Arc::new(AtomicUsize::new(0));
    // options 只借用到本函数体：worker 任务是 'static，先把 Copy 字段提出来
    let timeout_ms = options.timeout_ms;
    let body_cap = options.body_cap();
    for _ in 0..concurrency {
        let tx = tx.clone();
        let spec = spec.clone();
        let url = url.clone();
        let cancel = cancel.clone();
        let next = next.clone();
        let qps_gate = qps_gate.clone();
        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                if schedule.is_none() && next.fetch_add(1, Ordering::Relaxed) >= total_cap {
                    break;
                }
                if let Some(deadline) = deadline {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                if let Some(gate) = &qps_gate {
                    let wait_until = {
                        let mut next_slot = gate.lock().unwrap_or_else(|e| e.into_inner());
                        let slot = (*next_slot).max(Instant::now());
                        let qps = match schedule.unwrap() {
                            LoadSchedule::FixedQps { qps, .. } => qps as f64,
                            LoadSchedule::Ramp {
                                start_qps,
                                end_qps,
                                steps,
                                stage_secs,
                            } => {
                                let total = stage_secs.saturating_mul(steps as u64) as f64;
                                let t = if total > 0.0 {
                                    ((slot - start).as_secs_f64() / total).clamp(0.0, 1.0)
                                } else {
                                    1.0
                                };
                                start_qps as f64 + (end_qps as f64 - start_qps as f64) * t
                            }
                            _ => unreachable!(),
                        };
                        *next_slot = slot + Duration::from_secs_f64(1.0 / qps.max(1.0));
                        slot
                    };
                    tokio::time::sleep_until(wait_until.into()).await;
                    if cancel.is_cancelled() {
                        break;
                    }
                }
                let t = Instant::now();
                let r =
                    send_request_capped(method, &url, &spec, timeout_ms, Some(&cancel), body_cap)
                        .await;
                let d = t.elapsed().as_secs_f64() * 1000.0;
                let sample = match r {
                    Ok(_) => Sample {
                        ok: true,
                        duration_ms: d,
                        err: None,
                    },
                    Err(e) => Sample {
                        ok: false,
                        duration_ms: d,
                        err: Some(e.user_message()),
                    },
                };
                if tx.send(sample).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut samples: Vec<f64> = Vec::new();
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut done = 0usize;
    let mut aborted_reason: Option<String> = None;
    // 近 2 秒完成时刻窗口（进度 rps 口径）。
    let mut window: VecDeque<Instant> = VecDeque::new();

    while let Some(s) = rx.recv().await {
        done += 1;
        samples.push(s.duration_ms);
        if s.ok {
            ok += 1;
        } else {
            failed += 1;
        }
        if let Some(e) = s.err {
            if errors.len() < 5 {
                errors.push(e);
            }
        }
        // 错误率熔断：完成数 ≥ 10 且超阈值 → 取消子令牌（父令牌不受影响）。
        if aborted_reason.is_none() {
            if let Some(threshold) = cfg.max_error_rate {
                if done >= 10 && (failed as f64 / done as f64) >= threshold {
                    aborted_reason = Some("error_rate".to_string());
                    trip.cancel();
                }
            }
        }
        if let Some(cb) = progress {
            let now = Instant::now();
            window.push_back(now);
            let cutoff = now - Duration::from_secs(2);
            while window.front().is_some_and(|t| *t < cutoff) {
                window.pop_front();
            }
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let rps = if elapsed_ms > 0 {
                window.len() as f64 * 1000.0 / elapsed_ms.min(2000) as f64
            } else {
                0.0
            };
            cb(LoadProgress {
                done,
                total: if schedule.is_none() { total_cap } else { 0 },
                ok,
                failed,
                elapsed_ms,
                rps,
                error_rate: if done > 0 {
                    failed as f64 / done as f64
                } else {
                    0.0
                },
            });
        }
    }

    let cancelled =
        options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) || cancel.is_cancelled();
    let total_ms = start.elapsed().as_millis() as u64;
    let mut sorted = samples.clone();
    sorted.sort_unstable_by(|a, b| a.total_cmp(b));
    let done_ms = total_ms.max(1);

    // 延迟直方图（固定桶边界 + 溢出桶；互斥计数，各桶之和 = 样本总数）。
    let mut counts = vec![0u64; HISTOGRAM_EDGES_MS.len() + 1];
    for d in &samples {
        let idx = HISTOGRAM_EDGES_MS
            .iter()
            .position(|edge| *d <= *edge as f64)
            .unwrap_or(HISTOGRAM_EDGES_MS.len());
        counts[idx] += 1;
    }
    let mut histogram = Vec::with_capacity(HISTOGRAM_EDGES_MS.len() + 1);
    for (i, edge) in HISTOGRAM_EDGES_MS.iter().enumerate() {
        histogram.push(LatencyBucket {
            le_ms: Some(*edge),
            count: counts[i],
        });
    }
    histogram.push(LatencyBucket {
        le_ms: None,
        count: counts[HISTOGRAM_EDGES_MS.len()],
    });

    let (cpu_avg, cpu_peak, mem_avg, mem_peak) = sampler.summary();
    LoadResult {
        total: ok + failed,
        ok,
        failed,
        total_ms,
        avg_ms: if samples.is_empty() {
            0.0
        } else {
            samples.iter().sum::<f64>() / samples.len() as f64
        },
        p50_ms: percentile(&sorted, 50),
        p90_ms: percentile(&sorted, 90),
        p99_ms: percentile(&sorted, 99),
        rps: (ok + failed) as f64 * 1000.0 / done_ms as f64,
        errors,
        cancelled,
        histogram,
        aborted_reason,
        res_cpu_avg: cpu_avg,
        res_cpu_peak: cpu_peak,
        res_mem_avg_mb: mem_avg,
        res_mem_peak_mb: mem_peak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::KeyValue;
    use tokio::net::TcpListener;

    async fn ping_app() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route("/ping", axum::routing::get(|| async { "pong" }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, handle)
    }

    #[test]
    fn percentile_basic() {
        assert_eq!(percentile(&[], 50), 0.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0, 40.0], 50), 30.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0, 40.0], 90), 40.0);
        assert_eq!(percentile(&[5.0], 99), 5.0);
    }

    #[test]
    fn schedule_sanitized_clamps() {
        let s = LoadSchedule::FixedQps {
            qps: 0,
            duration_secs: 0,
        }
        .sanitized();
        assert_eq!(
            s,
            LoadSchedule::FixedQps {
                qps: 1,
                duration_secs: 1
            }
        );
        let r = LoadSchedule::Ramp {
            start_qps: 100,
            end_qps: 10,
            steps: 0,
            stage_secs: 0,
        }
        .sanitized();
        assert_eq!(
            r,
            LoadSchedule::Ramp {
                start_qps: 100,
                end_qps: 100,
                steps: 1,
                stage_secs: 1
            },
            "end < start 收钳为 start"
        );
    }

    #[tokio::test]
    async fn run_load_basic() {
        let (addr, _server) = ping_app().await;

        let spec = RequestSpec {
            params: vec![KeyValue::new("x", "1")],
            ..Default::default()
        };
        let cfg = LoadConfig {
            concurrency: 4,
            total: 20,
            schedule: None,
            max_error_rate: None,
        };
        let result = run_load(
            HttpMethod::GET,
            &format!("http://{addr}/ping"),
            &spec,
            &cfg,
            None,
        )
        .await;
        assert_eq!(result.total, 20, "总请求数应等于配置");
        assert_eq!(result.failed, 0, "本地服务不应失败");
        assert_eq!(result.ok, 20);
        assert!(result.avg_ms >= 0.0);
        assert!(result.rps > 0.0);
        assert!(result.errors.is_empty());
        // 直方图：20 个成功样本全部落在前几桶，总数守恒
        let sum: u64 = result.histogram.iter().map(|b| b.count).sum();
        assert_eq!(sum, 20, "直方图计数应守恒");
        assert_eq!(result.histogram.len(), HISTOGRAM_EDGES_MS.len() + 1);
        assert!(result.histogram.last().unwrap().le_ms.is_none());
    }

    #[tokio::test]
    async fn run_load_fixed_qps_hits_rate() {
        let (addr, _server) = ping_app().await;

        let cfg = LoadConfig {
            concurrency: 20,
            total: 0,
            schedule: Some(LoadSchedule::FixedQps {
                qps: 50,
                duration_secs: 1,
            }),
            max_error_rate: None,
        };
        let result = run_load(
            HttpMethod::GET,
            &format!("http://{addr}/ping"),
            &RequestSpec::default(),
            &cfg,
            None,
        )
        .await;
        assert_eq!(result.failed, 0);
        // 1 秒 @50qps：允许调度误差（50%~150%），不应跑出数倍
        assert!(
            (30..=120).contains(&result.total),
            "FixedQps 1s 应≈50 次，实际 {}",
            result.total
        );
        assert!(!result.cancelled);
    }

    #[tokio::test]
    async fn run_load_duration_mode_runs_to_deadline() {
        let (addr, _server) = ping_app().await;

        let cfg = LoadConfig {
            concurrency: 8,
            total: 0,
            schedule: Some(LoadSchedule::Duration { duration_secs: 1 }),
            max_error_rate: None,
        };
        let start = Instant::now();
        let result = run_load(
            HttpMethod::GET,
            &format!("http://{addr}/ping"),
            &RequestSpec::default(),
            &cfg,
            None,
        )
        .await;
        let wall = start.elapsed();
        assert_eq!(result.failed, 0);
        assert!(result.total > 0, "Duration 模式应有产出");
        assert!(
            wall >= Duration::from_millis(900) && wall < Duration::from_millis(2500),
            "Duration 1s 应按时长结束，实际 {wall:?}"
        );
        assert!(result.res_cpu_peak >= 0.0);
    }

    #[tokio::test]
    async fn run_load_ramp_reaches_end_qps() {
        let (addr, _server) = ping_app().await;

        let cfg = LoadConfig {
            concurrency: 50,
            total: 0,
            schedule: Some(LoadSchedule::Ramp {
                start_qps: 20,
                end_qps: 100,
                steps: 4,
                stage_secs: 1,
            }),
            max_error_rate: None,
        };
        let result = run_load(
            HttpMethod::GET,
            &format!("http://{addr}/ping"),
            &RequestSpec::default(),
            &cfg,
            None,
        )
        .await;
        assert_eq!(result.failed, 0);
        // 4s，平均 qps ≈ (20+100)/2 = 60 → 总量 ≈ 240（±50% 容忍调度误差）
        assert!(
            (120..=500).contains(&result.total),
            "Ramp 4s 平均 60qps 应≈240 次，实际 {}",
            result.total
        );
    }

    #[tokio::test]
    async fn run_load_error_rate_circuit_breaks() {
        // 全失败目标（discard 端口）+ 30% 熔断阈值 → 远不到 total 即熔断
        let spec = RequestSpec::default();
        let cfg = LoadConfig {
            concurrency: 2,
            total: 5000,
            schedule: None,
            max_error_rate: Some(0.3),
        };
        let options = LoadOptions {
            timeout_ms: Some(2_000),
            cancel: None,
            body_cap_bytes: Some(1024),
        };
        let result = run_load_with(
            HttpMethod::GET,
            "http://127.0.0.1:9/nope",
            &spec,
            &cfg,
            &options,
            None,
        )
        .await;
        assert_eq!(result.ok, 0);
        assert_eq!(
            result.aborted_reason.as_deref(),
            Some("error_rate"),
            "错误率 100% > 30% 应熔断"
        );
        assert!(result.cancelled, "熔断应标记 cancelled");
        assert!(result.total < 5000, "熔断后不应跑满");
    }

    #[tokio::test]
    async fn run_load_with_cancel_stops_early() {
        use std::time::Duration;
        // 慢服务：取消后不应跑满 total。
        let app = axum::Router::new().route(
            "/slow",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "slow"
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            canceller.cancel();
        });
        let options = LoadOptions {
            timeout_ms: Some(5_000),
            cancel: Some(cancel),
            body_cap_bytes: Some(1024),
        };
        let cfg = LoadConfig {
            concurrency: 2,
            total: 50,
            schedule: None,
            max_error_rate: None,
        };
        let result = run_load_with(
            HttpMethod::GET,
            &format!("http://{addr}/slow"),
            &RequestSpec::default(),
            &cfg,
            &options,
            None,
        )
        .await;
        assert!(result.cancelled, "取消后应标记 cancelled");
        assert!(result.total < 50, "取消后不应跑满，实际 {}", result.total);
    }

    #[tokio::test]
    async fn run_load_handles_failures() {
        let addr = "127.0.0.1:9"; // discard 端口，连接必然失败。
        let spec = RequestSpec::default();
        let cfg = LoadConfig {
            concurrency: 2,
            total: 6,
            schedule: None,
            max_error_rate: None,
        };
        let result = run_load(
            HttpMethod::GET,
            &format!("http://{addr}/nope"),
            &spec,
            &cfg,
            None,
        )
        .await;
        assert_eq!(result.total, 6);
        assert_eq!(result.ok, 0);
        assert_eq!(result.failed, 6);
        assert!(!result.errors.is_empty());
        assert!(result.errors.len() <= 5);
        assert!(result.aborted_reason.is_none(), "未配置熔断不应触发");
    }
}
