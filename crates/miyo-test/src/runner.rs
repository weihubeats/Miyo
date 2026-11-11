//! 测试运行器（SPEC §17.2）：按配置执行单个接口的测试流程。
//!
//! 并发模型：`run_endpoint` 为单接口串行入口；`run_collection` 为集合入口，
//! worker 数恒为并发度，经原子计数器领任务，运行时变量表由
//! `tokio::sync::Mutex` 共享（锁仅在快照/回写瞬间持有，发送与断言不持锁）。

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use miyo_core::model::{Endpoint, RequestSpec};
use miyo_http::client::{send_request, send_request_cancel, HttpResponseData, RequestSnapshot};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use uuid::Uuid;

use crate::assert::{evaluate, evaluate_javascript, Outcome};
use crate::config::TestSpec;
use crate::extract::extract_variables;

/// 步骤响应快照（JSON 安全，随 [`EndpointResult`] 序列化给前端）。
///
/// `body` 为响应全文（HTTP 层已按 20MB 上限截断并标记 `truncated`）；
/// IPC 预览截断（>2MB）与全文缓存由 miyo-tauri 命令层落地——与单请求
/// 流程同口径（`get_response_body` 按需取回），本结构不感知缓存。
/// Deserialize 供运行历史落库回读（scenario_runs.results_json）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    /// 毫秒（浮点，保留亚毫秒精度）。
    pub duration_ms: f64,
    pub size_bytes: usize,
    pub truncated: bool,
    /// 正文是否仅为 IPC 预览（命令层超限截断时置位）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub body_omitted: bool,
    /// 正文是否为历史落库截断版（64KB 上限；全文不可再取回，仅提示）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub history_truncated: bool,
    /// 实际发出形态（变量渲染 / 认证注入 / Cookie 回放后的最终值）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestSnapshot>,
}

impl StepResponse {
    fn from_resp(resp: &HttpResponseData) -> Self {
        StepResponse {
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body_text(),
            content_type: resp.content_type(),
            duration_ms: resp.duration_ms,
            size_bytes: resp.size_bytes,
            truncated: resp.truncated,
            body_omitted: false,
            history_truncated: false,
            request: resp.request.clone(),
        }
    }
}

/// 单个接口的测试结果。
/// Deserialize 供运行历史落库回读（scenario_runs.results_json）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EndpointResult {
    pub endpoint_id: Uuid,
    pub endpoint_name: String,
    pub method: String,
    pub path: String,
    pub ok: bool,
    pub status: Option<u16>,
    pub duration_ms: Option<f64>,
    /// 请求级错误（发送失败 / 配置错误）。
    pub request_error: Option<String>,
    /// 断言明细。
    pub outcomes: Vec<Outcome>,
    /// 本步提取到的变量（场景链路的时间线展示与调试用）。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extracted: HashMap<String, String>,
    /// 实际发送的渲染后 URL（含变量替换）；配置错误未发送时为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 场景链路中的步骤位次（启用步骤 0-based）；单接口 / 集合运行为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_index: Option<usize>,
    /// 该步因取消而中止（在途请求被 cancel 中止，区别于真实失败）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// 响应快照（发送成功时存在；failed_fast 无响应）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<StepResponse>,
    /// 场景步骤执行状态（六态：success / failed / blocked / skipped /
    /// cancelled / 旧数据 None）。单接口 / 集合运行为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_status: Option<String>,
    /// Blocked 原因（缺哪个变量、应产自哪一步）；仅 blocked 状态有值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// 循环步骤的已执行轮数（>1 时前端展示「×N」；非循环 / 单次为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_runs: Option<u32>,
}

/// 场景步骤执行状态常量（`EndpointResult.step_status`）。
pub mod step_status {
    pub const SUCCESS: &str = "success";
    pub const FAILED: &str = "failed";
    pub const BLOCKED: &str = "blocked";
    pub const SKIPPED: &str = "skipped";
    pub const CANCELLED: &str = "cancelled";
}

impl EndpointResult {
    fn failed_fast(
        id: Uuid,
        ep: &Endpoint,
        url: Option<String>,
        reason: impl Into<String>,
    ) -> Self {
        EndpointResult {
            endpoint_id: id,
            endpoint_name: ep.name.clone(),
            method: ep.method.to_string(),
            path: ep.path.clone(),
            ok: false,
            status: None,
            duration_ms: None,
            request_error: Some(reason.into()),
            outcomes: Vec::new(),
            extracted: HashMap::new(),
            url,
            step_index: None,
            cancelled: false,
            response: None,
            step_status: None,
            blocked_reason: None,
            loop_runs: None,
        }
    }

    /// 派生执行状态：显式 `step_status` 优先；旧结果（无该字段）按
    /// cancelled / ok 推导，兼容历史 JSON 回读。
    pub fn effective_status(&self) -> &'static str {
        match self.step_status.as_deref() {
            Some(s) => match s {
                step_status::BLOCKED => step_status::BLOCKED,
                step_status::SKIPPED => step_status::SKIPPED,
                step_status::CANCELLED => step_status::CANCELLED,
                step_status::FAILED => step_status::FAILED,
                _ => step_status::SUCCESS,
            },
            None if self.cancelled => step_status::CANCELLED,
            None if self.ok => step_status::SUCCESS,
            None => step_status::FAILED,
        }
    }
}

/// 运行单个接口测试。
///
/// - `runtime_vars`：此前由 extract 得到的运行时变量，会被本轮 pre_request / extract
///   更新，并传递给后续接口（共享运行时上下文）。
///
/// 返回测试结果与原始响应。
pub async fn run_endpoint(
    ep: &Endpoint,
    url: &str,
    spec: &RequestSpec,
    runtime_vars: &mut HashMap<String, String>,
    timeout_ms: Option<u64>,
) -> (EndpointResult, Option<HttpResponseData>) {
    // 配置解析由 run_one 统一处理（含"无测试配置跳过"与"配置错误"语义）；
    // 运行时变量经互斥表访问（串行入口独占，集合入口多 worker 共享）。
    let shared = AsyncMutex::new(std::mem::take(runtime_vars));
    let (result, resp) = run_one(ep, url, spec, &shared, timeout_ms, None, false).await;
    *runtime_vars = shared.into_inner();
    (result, resp)
}

/// 单接口测试的可取消变体：`cancel` 触发时中止在途请求，结果标记
/// `cancelled = true`（测试用例单跑的「取消」按钮走这里）。
/// `force_send`：无断言也照常发送（用例 = 请求快照，发送本身就是目的）。
pub async fn run_endpoint_with_cancel(
    ep: &Endpoint,
    url: &str,
    spec: &RequestSpec,
    runtime_vars: &mut HashMap<String, String>,
    timeout_ms: Option<u64>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    force_send: bool,
) -> (EndpointResult, Option<HttpResponseData>) {
    let shared = AsyncMutex::new(std::mem::take(runtime_vars));
    let (result, resp) = run_one(ep, url, spec, &shared, timeout_ms, cancel, force_send).await;
    *runtime_vars = shared.into_inner();
    (result, resp)
}

/// 单接口四步核心（pre→send→assert→extract），运行时变量表由调用方提供：
///
/// - 锁只在快照/回写瞬间持有，网络发送与断言求值全程不持锁；
/// - 集合并发下多个 worker 共享同一张表：extract 按完成顺序合并，
///   强顺序依赖的链路请用 `concurrency=1`（等价串行）。
async fn run_one(
    ep: &Endpoint,
    url: &str,
    spec: &RequestSpec,
    runtime: &AsyncMutex<HashMap<String, String>>,
    timeout_ms: Option<u64>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    force_send: bool,
) -> (EndpointResult, Option<HttpResponseData>) {
    let config = match TestSpec::from_request_value(ep.request.tests.as_ref()) {
        Ok(c) => c,
        Err(reason) => {
            return (
                EndpointResult::failed_fast(
                    ep.id,
                    ep,
                    Some(url.to_string()),
                    format!("测试配置错误：{reason}"),
                ),
                None,
            );
        }
    };
    // force_send = true（测试用例执行）：无断言也照常发送并返回响应快照，
    // 「空配置跳过」只适用于接口断言集合的旧语义。
    if config.is_empty() && !force_send {
        return (
            EndpointResult {
                endpoint_id: ep.id,
                endpoint_name: ep.name.clone(),
                method: ep.method.to_string(),
                path: ep.path.clone(),
                ok: true,
                status: None,
                duration_ms: None,
                request_error: Some("无测试配置（跳过）".into()),
                outcomes: Vec::new(),
                extracted: HashMap::new(),
                url: Some(url.to_string()),
                step_index: None,
                cancelled: false,
                response: None,
                step_status: None,
                blocked_reason: None,
                loop_runs: None,
            },
            None,
        );
    }
    run_one_core(ep, url, spec, config, runtime, timeout_ms, cancel).await
}

/// 四步核心（pre→send→assert→extract），测试配置由调用方给定：
/// 单接口路径读自 `endpoint.tests`；场景步骤传步骤级配置
///（即使为空也照常发送——场景步骤断言可选，发送本身不能跳过）。
///
/// `cancel` 触发时中止在途请求（`send_request_cancel`），本步结果标记
/// `cancelled = true`；仅取消单步语义，是否终止整轮由调用方判定。
async fn run_one_core(
    ep: &Endpoint,
    url: &str,
    spec: &RequestSpec,
    config: TestSpec,
    runtime: &AsyncMutex<HashMap<String, String>>,
    timeout_ms: Option<u64>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> (EndpointResult, Option<HttpResponseData>) {
    // 1. pre_request：快照→本地解析→回写（值支持 {{$timestamp}} 与已提取变量）。
    // 作用域内借用读锁（resolve_text 同步、无 await），免整表 clone。
    let mut injected = HashMap::with_capacity(config.pre_request.len());
    {
        let snapshot = runtime.lock().await;
        for p in &config.pre_request {
            injected.insert(p.name.clone(), resolve_text(&p.value, &snapshot));
        }
    }
    if !injected.is_empty() {
        runtime.lock().await.extend(injected);
    }

    // 2. 发送请求（带取消时中止在途请求，返回 AppError::Cancelled）。
    let sent = match cancel {
        Some(c) => send_request_cancel(ep.method, url, spec, timeout_ms, c).await,
        None => send_request(ep.method, url, spec, timeout_ms).await,
    };
    let resp = match sent {
        Ok(r) => r,
        Err(e) => {
            if matches!(e, miyo_core::AppError::Cancelled(_)) {
                let mut result =
                    EndpointResult::failed_fast(ep.id, ep, Some(url.to_string()), "请求已取消");
                result.cancelled = true;
                return (result, None);
            }
            return (
                EndpointResult::failed_fast(
                    ep.id,
                    ep,
                    Some(url.to_string()),
                    format!("请求失败：{}", e.user_message()),
                ),
                None,
            );
        }
    };

    // 3. 断言（expected 中的 {{变量}} 按快照解析；body 只解析一次复用）。
    let body_value: Option<Value> = serde_json::from_slice(resp.body.as_ref()).ok();
    // JS 沙箱断言（type=javascript）需 await 沙箱回传，故逐条循环而非 map。
    let mut outcomes: Vec<Outcome> = Vec::with_capacity(config.assertions.len());
    for a in &config.assertions {
        let resolved;
        let a: &crate::config::AssertionSpec = if needs_resolve(a) {
            // 同步解析借用读锁；沙箱调用前守卫随作用域释放。
            let snapshot = runtime.lock().await;
            resolved = resolve(a, &snapshot);
            &resolved
        } else {
            a
        };
        if a.r#type == "javascript" {
            outcomes.push(evaluate_javascript(a, &resp).await);
        } else {
            outcomes.push(evaluate(a, &resp, body_value.as_ref()));
        }
    }
    let ok = outcomes.iter().all(|o| o.passed);

    // 4. 提取变量（供后续接口使用）。
    let extracted: HashMap<String, String> =
        extract_variables(&config.extract, &resp, body_value.as_ref());
    if !extracted.is_empty() {
        runtime.lock().await.extend(extracted.clone());
    }

    (
        EndpointResult {
            endpoint_id: ep.id,
            endpoint_name: ep.name.clone(),
            method: ep.method.to_string(),
            path: ep.path.clone(),
            ok,
            status: Some(resp.status),
            duration_ms: Some(resp.duration_ms),
            request_error: None,
            outcomes,
            extracted,
            url: Some(url.to_string()),
            step_index: None,
            cancelled: false,
            response: Some(StepResponse::from_resp(&resp)),
            step_status: None,
            blocked_reason: None,
            loop_runs: None,
        },
        Some(resp),
    )
}

/// 集合测试条目（调用方已渲染好 url/spec：变量只需解析一次）。
#[derive(Debug, Clone)]
pub struct CollectionItem {
    pub endpoint: Endpoint,
    pub url: String,
    pub spec: RequestSpec,
    /// 无断言也照常发送（测试用例批量执行 = 请求快照，发送本身是目的；
    /// 接口断言集合保持 false = 空配置跳过旧语义）。
    pub force_send: bool,
}

/// 集合测试选项。
#[derive(Debug, Clone, Default)]
pub struct CollectionOptions {
    /// 并发度（默认 5；1 = 严格串行，extract 按输入顺序传递）。
    pub concurrency: Option<usize>,
    /// 单接口超时毫秒数。
    pub timeout_ms: Option<u64>,
    /// 取消令牌；触发后 worker 停止领新任务。
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

/// 集合测试结果（results 与输入同序；取消时只含已完成项）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CollectionResult {
    pub results: Vec<EndpointResult>,
    pub cancelled: bool,
}

/// 并发跑集合：固定 worker 池 + 原子计数器领任务 + mpsc 回传聚合。
///
/// 原来前端 N 次 IPC 逐个调 `test_endpoint`（N 次变量加载 + 串行等待）；
/// 现在一次 IPC 跑完整个集合，变量加载一次、发送按并发度并行、可取消。
pub async fn run_collection(
    items: Vec<CollectionItem>,
    runtime_vars: &mut HashMap<String, String>,
    options: &CollectionOptions,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> CollectionResult {
    let total = items.len();
    if total == 0 {
        return CollectionResult {
            results: Vec::new(),
            cancelled: false,
        };
    }
    let concurrency = options.concurrency.unwrap_or(5).clamp(1, 64).min(total);
    let runtime = Arc::new(AsyncMutex::new(std::mem::take(runtime_vars)));
    let items = Arc::new(items);
    let next = Arc::new(AtomicUsize::new(0));
    let cancel = options.cancel.clone();
    let timeout_ms = options.timeout_ms;

    let (tx, mut rx) = mpsc::channel::<(usize, EndpointResult)>(concurrency * 2);
    for _ in 0..concurrency {
        let tx = tx.clone();
        let next = next.clone();
        let items = items.clone();
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                    break;
                }
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= items.len() {
                    break;
                }
                let item = &items[idx];
                let (result, _) = run_one(
                    &item.endpoint,
                    &item.url,
                    &item.spec,
                    &runtime,
                    timeout_ms,
                    cancel.as_ref(),
                    item.force_send,
                )
                .await;
                if tx.send((idx, result)).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut ordered: Vec<Option<EndpointResult>> = (0..total).map(|_| None).collect();
    let mut done = 0usize;
    while let Some((idx, result)) = rx.recv().await {
        ordered[idx] = Some(result);
        done += 1;
        if let Some(cb) = progress {
            cb(done, total);
        }
    }
    let results: Vec<EndpointResult> = ordered.into_iter().flatten().collect();

    // 回写合并后的运行时变量（extract 成果保留给调用方后续使用）。
    let merged = Arc::try_unwrap(runtime)
        .map(|m| m.into_inner())
        .unwrap_or_default();
    *runtime_vars = merged;

    CollectionResult {
        results,
        cancelled: cancel.as_ref().is_some_and(|c| c.is_cancelled()),
    }
}

/// 场景步骤输入：URL / 规格保留**渲染前模板**（执行期用 runtime 二次渲染，
/// 前步提取的变量即可成为后续请求的入参），提取与断言为步骤级配置
/// （不读 `endpoint.tests`）。
///
/// `enabled = false` 的禁用步骤同样进入 runner：产出 skipped 结果行
/// （不发送请求），结果面板得以完整呈现「为什么这一步没跑」。
/// `condition` / `loop` / `wait_ms` / `parallel` 为阶段三执行控制
///（条件跳过 / 轮询循环 / 限速等待 / 相邻并行批）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScenarioStepInput {
    pub endpoint: Endpoint,
    pub url_template: String,
    pub spec_template: RequestSpec,
    pub extract: Vec<crate::config::ExtractSpec>,
    pub assertions: Vec<crate::config::AssertionSpec>,
    pub enabled: bool,
    /// 执行条件（不满足 → skipped，不发送；None = 无条件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<crate::config::StepCondition>,
    /// 循环配置（count / while；None = 单次）。
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<crate::config::StepLoop>,
    /// 执行前等待毫秒（0..=60_000）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    /// 与相邻 parallel 步骤成批并发（≥2 连续标记才成批）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parallel: bool,
}

/// 场景进度快照（回调参数）。
///
/// 进度分两半：`done` 是本步之前的已完成步数，`step` 是本步（进入时不含结果，
/// 结束 / 失败 / 取消时带上该步结果）——前端据此在等待网络期间就渲染
/// 「第 N 步 · 执行中 · 已发出请求」，而不是只看到一根 0%→100% 的进度条。
/// 并行批内多步同时在途：`done` 语义退化为「该步位次」（前端按 step_index
/// 映射结果行，不依赖 done 的单调性）。
#[derive(Debug, Clone)]
pub struct ScenarioProgress<'a> {
    /// 已完成步数（0-based 位次）；并行批的进入事件为该步位次。
    pub done: usize,
    pub total: usize,
    /// 本步结果：进入本步时为 `None`，本步结束后为 `Some`。
    pub step: Option<&'a EndpointResult>,
    /// 本步渲染后实际发出的 URL（进入本步时已可确定）。
    pub url: Option<&'a str>,
}

/// 场景运行选项。
#[derive(Debug, Clone, Default)]
pub struct ScenarioOptions {
    pub timeout_ms: Option<u64>,
    pub cancel: Option<tokio_util::sync::CancellationToken>,
    /// 失败即停：某步请求失败或断言不过时，不再执行后续步骤。
    pub stop_on_failure: bool,
}

/// 场景运行结果。
#[derive(Debug, serde::Serialize)]
pub struct ScenarioRunResult {
    pub results: Vec<EndpointResult>,
    /// 因取消而中止（未跑完剩余步骤）。
    pub cancelled: bool,
    /// 因失败即停而中止（results 末步即失败步）。
    pub stopped_on_failure: bool,
    /// 全部步骤结束后的运行时变量（链路产物；含环境静态变量的兜底值）。
    pub variables: HashMap<String, String>,
    /// 每步执行前的 runtime 快照（与 results 同位序）：重跑失败步骤时作为
    /// 初始变量回传，重建当时的链路上下文。随 IPC 下发、不落运行历史。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vars_snapshots: Vec<HashMap<String, String>>,
    /// 运行前预检警告：执行期仍未定义（环境 / 项目 / 全局 / 前步提取都
    /// 覆盖不到）的变量引用，渲染时按字面量保留。每项如
    /// `"第 2 步：{{payableAmount}} 未定义"`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// 一次运行共享的可变执行状态（runtime / 类型层 / 丢失变量 / 最近状态码）。
/// 每次运行独立构造——执行上下文隔离；并行批内多任务共享同一份（提取按
/// 完成序合并）。
struct SharedRunState {
    runtime: AsyncMutex<HashMap<String, String>>,
    typed: AsyncMutex<HashMap<String, serde_json::Value>>,
    lost: AsyncMutex<std::collections::HashSet<String>>,
    last_status: AsyncMutex<Option<u16>>,
}

/// 单步结果行 + 并行批任务与主循环间的事件。
enum BatchMsg {
    /// 已进入发送阶段（渲染后 URL 确定；进度面板显示「执行中」）。
    Enter { pos: usize, url: String },
    /// 该步执行完毕。
    Done {
        pos: usize,
        result: Box<EndpointResult>,
    },
}

/// 步骤循环的执行信息（结果行聚合展示「×N」）。
fn with_loop_runs(mut r: EndpointResult, rounds: u32) -> EndpointResult {
    if rounds > 1 {
        r.loop_runs = Some(rounds);
    }
    r
}

/// 禁用步骤的占位结果行（skipped；不发送请求）。
fn skipped_result(step: &ScenarioStepInput, idx: usize, reason: &str) -> EndpointResult {
    EndpointResult {
        endpoint_id: step.endpoint.id,
        endpoint_name: step.endpoint.name.clone(),
        method: step.endpoint.method.to_string(),
        path: step.endpoint.path.clone(),
        ok: false,
        status: None,
        duration_ms: None,
        request_error: Some(reason.to_string()),
        outcomes: Vec::new(),
        extracted: HashMap::new(),
        url: None,
        step_index: Some(idx),
        cancelled: false,
        response: None,
        step_status: Some(step_status::SKIPPED.to_string()),
        blocked_reason: None,
        loop_runs: None,
    }
}

/// Blocked 占位结果行（依赖失败步骤产出的变量；不发送请求）。
fn blocked_result(step: &ScenarioStepInput, idx: usize, reason: String) -> EndpointResult {
    EndpointResult {
        endpoint_id: step.endpoint.id,
        endpoint_name: step.endpoint.name.clone(),
        method: step.endpoint.method.to_string(),
        path: step.endpoint.path.clone(),
        ok: false,
        status: None,
        duration_ms: None,
        request_error: None,
        outcomes: Vec::new(),
        extracted: HashMap::new(),
        url: None,
        step_index: Some(idx),
        cancelled: false,
        response: None,
        step_status: Some(step_status::BLOCKED.to_string()),
        blocked_reason: Some(reason),
        loop_runs: None,
    }
}

/// 条件的可读化文本（skipped 原因 / 预检提示用）。
fn condition_text(c: &crate::config::StepCondition) -> String {
    let target = match c.source.as_str() {
        "prevStatus" => "上一步状态码".to_string(),
        "response.status" => "本轮状态码".to_string(),
        "response.body" => format!("响应体 {}", c.expression.as_deref().unwrap_or("")),
        "response.header" => format!("响应头 {}", c.expression.as_deref().unwrap_or("")),
        _ => format!("变量 {{{{{}}}}}", c.name.trim()),
    };
    let expected = c
        .value
        .as_ref()
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let op = c.op.as_str();
    match c.op.as_str() {
        "exists" | "not_exists" | "empty" | "not_empty" => format!("{target} {op}"),
        _ => format!("{target} {op} {expected}"),
    }
}

/// 求值执行条件：`source = var` 对运行时变量（缺变量按 not_exists 语义），
/// `source = prevStatus` 对最近一个已执行步骤的状态码，
/// `source = response.status / response.body / response.header` 对传入的
/// 本轮响应求值（while 循环每轮结束传入该轮响应；执行条件阶段传 None）。
/// 复用断言的值级比较。
fn eval_condition(
    c: &crate::config::StepCondition,
    vars: &HashMap<String, String>,
    prev_status: Option<u16>,
    last_response: Option<&StepResponse>,
) -> bool {
    let actual: Option<serde_json::Value> = match c.source.as_str() {
        "prevStatus" => prev_status.map(|s| serde_json::Value::Number(s.into())),
        "response.status" => last_response.map(|r| serde_json::Value::Number(r.status.into())),
        "response.header" => {
            let name = c.expression.as_deref().unwrap_or_default().trim();
            last_response.and_then(|r| {
                r.headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| serde_json::Value::String(v.clone()))
            })
        }
        "response.body" => {
            let path = c.expression.as_deref().unwrap_or_default().trim();
            last_response.and_then(|r| {
                let json = serde_json::from_str::<serde_json::Value>(&r.body).ok();
                crate::extract::extract_body_json_typed(json.as_ref(), path)
            })
        }
        _ => vars
            .get(c.name.trim())
            .map(|v| serde_json::Value::String(v.clone())),
    };
    let expected = c.value.clone().unwrap_or(serde_json::Value::Null);
    crate::assert::compare_values(&c.op, actual, &expected).is_ok()
}

/// 可取消的睡眠：返回 false = 等待期间被取消。
async fn sleep_cancellable(ms: u64, cancel: Option<&tokio_util::sync::CancellationToken>) -> bool {
    if ms == 0 {
        return true;
    }
    let dur = std::time::Duration::from_millis(ms);
    match cancel {
        Some(c) => {
            tokio::select! {
                _ = tokio::time::sleep(dur) => true,
                _ = c.cancelled() => false,
            }
        }
        None => {
            tokio::time::sleep(dur).await;
            true
        }
    }
}

/// 收集一个步骤全部模板里的变量引用（URL + 规格 + Body + 断言期望值）。
fn step_var_refs(step: &ScenarioStepInput) -> Vec<String> {
    let mut refs = miyo_core::collect_var_refs(&step.url_template);
    for kv in step
        .spec_template
        .params
        .iter()
        .chain(step.spec_template.headers.iter())
        .chain(step.spec_template.path_variables.iter())
    {
        refs.extend(miyo_core::collect_var_refs(&kv.key));
        refs.extend(miyo_core::collect_var_refs(&kv.value));
    }
    // Body 各变体的文本模板（与 render 的覆盖面对齐）。
    match &step.spec_template.body {
        miyo_core::model::BodySpec::None => {}
        miyo_core::model::BodySpec::Json { raw } | miyo_core::model::BodySpec::Text { raw } => {
            refs.extend(miyo_core::collect_var_refs(raw));
        }
        miyo_core::model::BodySpec::UrlEncoded { fields } => {
            for f in fields {
                refs.extend(miyo_core::collect_var_refs(&f.key));
                refs.extend(miyo_core::collect_var_refs(&f.value));
            }
        }
        miyo_core::model::BodySpec::Multipart { fields } => {
            for f in fields {
                refs.extend(miyo_core::collect_var_refs(&f.key));
                refs.extend(miyo_core::collect_var_refs(&f.value));
            }
        }
        miyo_core::model::BodySpec::GraphQL { spec } => {
            refs.extend(miyo_core::collect_var_refs(&spec.query));
            refs.extend(miyo_core::collect_var_refs(&spec.variables));
        }
        miyo_core::model::BodySpec::Binary { path } => {
            refs.extend(miyo_core::collect_var_refs(path));
        }
        miyo_core::model::BodySpec::Grpc { spec } => {
            refs.extend(miyo_core::collect_var_refs(&spec.service));
            refs.extend(miyo_core::collect_var_refs(&spec.method));
            refs.extend(miyo_core::collect_var_refs(&spec.message));
        }
    }
    // 循环条件也参与引用（Blocked 判定 / 预检覆盖）。
    if let Some(l) = &step.r#loop {
        if let Some(wc) = &l.while_condition {
            refs.extend(miyo_core::collect_var_refs(&wc.name));
        }
    }
    for a in &step.assertions {
        if let Some(serde_json::Value::String(s)) = &a.expected {
            refs.extend(miyo_core::collect_var_refs(s));
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

/// 运行前预检：逐步找「执行到该步时仍无来源」的变量引用（环境静态变量 /
/// 前步提取都覆盖不到）。这些引用渲染时按字面量保留，不阻断执行——
/// 报告给前端做非阻断提示。
fn undefined_var_warnings(
    steps: &[ScenarioStepInput],
    base_vars: &HashMap<String, String>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut available: std::collections::HashSet<String> = base_vars.keys().cloned().collect();
    for (idx, step) in steps.iter().enumerate() {
        if !step.enabled {
            continue;
        }
        let mut missing: Vec<String> = Vec::new();
        for r in step_var_refs(step) {
            // 条件引用的变量允许「不存在」语义（exists / not_exists），
            // 其余仍按未定义告警。
            let is_cond_var = step
                .condition
                .as_ref()
                .is_some_and(|c| c.source == "var" && c.name.trim() == r)
                || step.r#loop.as_ref().is_some_and(|l| {
                    l.while_condition
                        .as_ref()
                        .is_some_and(|c| c.source == "var" && c.name.trim() == r)
                });
            if is_cond_var {
                continue;
            }
            if !available.contains(&r) && !missing.contains(&r) {
                missing.push(r);
            }
        }
        if !missing.is_empty() {
            let list = missing
                .iter()
                .map(|m| format!("{{{{{m}}}}}"))
                .collect::<Vec<_>>()
                .join("、");
            warnings.push(format!(
                "第 {} 步（{}）：{list} 未定义，将按字面量发送",
                idx + 1,
                step.endpoint.name
            ));
        }
        // 本步的提取产出对后续步可见（无论本步成败——预检只看静态来源）。
        for e in &step.extract {
            available.insert(e.name.clone());
        }
    }
    warnings
}

/// 单轮发送（渲染 → scheme 校验 → 发送 / 断言 / 提取 → 簿记）。
///
/// 每轮用最新的 runtime / 类型层快照渲染——while 轮询里上一轮提取的值
/// 当轮即可参与条件求值与下一轮渲染。
#[allow(clippy::too_many_arguments)]
async fn send_round(
    step: &ScenarioStepInput,
    pos: usize,
    shared: &SharedRunState,
    options: &ScenarioOptions,
    emit_enter: Option<&(dyn Fn(String) + Send + Sync)>,
) -> EndpointResult {
    // 渲染输入借用读锁（resolve / render 同步、无 await），免整表 clone。
    let (url, spec) = {
        let render_vars = shared.runtime.lock().await;
        let typed_snapshot = shared.typed.lock().await;
        let url = miyo_core::resolve_variables(&step.url_template, &render_vars);
        let spec = miyo_core::render_spec_typed(&step.spec_template, &render_vars, &typed_snapshot);
        (url, spec)
    };
    let config = TestSpec {
        pre_request: Vec::new(),
        extract: step.extract.clone(),
        assertions: step.assertions.clone(),
    };
    // scheme 白名单（对齐 execute_request：仅 http/https），防变量替换
    // 把 scheme / host 换成任意协议。
    let scheme_ok = {
        let lower = url.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    };
    if !scheme_ok {
        let mut r = EndpointResult::failed_fast(
            step.endpoint.id,
            &step.endpoint,
            Some(url.clone()),
            format!("URL 仅支持 http/https 协议：{url}"),
        );
        r.step_index = Some(pos);
        r.step_status = Some(step_status::FAILED.to_string());
        let mut lost = shared.lost.lock().await;
        for e in &step.extract {
            lost.insert(e.name.clone());
        }
        return r;
    }
    // 进入本步：先报「已发出」（实际 URL 已确定），让前端在等待网络期间
    // 就能展示正在跑哪一步、打到哪个地址。循环只在首轮报（避免刷屏）。
    if let Some(cb) = emit_enter {
        cb(url.clone());
    }
    let step_timeout = step.endpoint.request.timeout_ms.or(options.timeout_ms);
    let (mut result, _) = run_one_core(
        &step.endpoint,
        &url,
        &spec,
        config,
        &shared.runtime,
        step_timeout,
        options.cancel.as_ref(),
    )
    .await;
    result.step_index = Some(pos);
    result.step_status = Some(if result.cancelled {
        step_status::CANCELLED.to_string()
    } else if result.ok {
        step_status::SUCCESS.to_string()
    } else {
        step_status::FAILED.to_string()
    });
    // 簿记：最近状态码（prevStatus 条件源）+ 失败步提取名进丢失集合 +
    // 成功步 body 提取进类型层。
    if let Some(st) = result.status {
        *shared.last_status.lock().await = Some(st);
    }
    if !result.ok {
        let mut lost = shared.lost.lock().await;
        for e in &step.extract {
            lost.insert(e.name.clone());
        }
    } else {
        let body_value: Option<serde_json::Value> = result
            .response
            .as_ref()
            .and_then(|r| serde_json::from_str(&r.body).ok());
        if let Some(body) = body_value {
            let mut t = shared.typed.lock().await;
            for e in &step.extract {
                if let Some(v) = crate::extract::extract_body_json_typed(Some(&body), &e.path) {
                    if !matches!(v, serde_json::Value::String(_)) {
                        t.insert(e.name.clone(), v);
                    }
                }
            }
        }
    }
    result
}

/// 单步执行管线：Blocked 判定 → 执行条件 → 等待 → 循环（或单次）发送。
///
/// 结果自描述（step_index / step_status / loop_runs）；失败步的提取名、
/// 成功步的类型值、最近状态码的簿记都在此收口——串行与并行批共用。
async fn execute_step(
    step: &ScenarioStepInput,
    pos: usize,
    shared: &SharedRunState,
    options: &ScenarioOptions,
    emit_enter: &(dyn Fn(String) + Send + Sync),
) -> EndpointResult {
    // ---- Blocked 判定：缺失变量产自失败 / Blocked / Skipped 步骤时不发送 ----
    // 借用读锁（filter 同步），守卫随作用域释放，避免与下文 lost 写锁互踩。
    let prev_status = *shared.last_status.lock().await;
    let blocked: Vec<String> = {
        let render_vars = shared.runtime.lock().await;
        let lost = shared.lost.lock().await;
        step_var_refs(step)
            .into_iter()
            .filter(|r| !render_vars.contains_key(r) && lost.contains(r))
            .collect()
    };
    if !blocked.is_empty() {
        let list = blocked
            .iter()
            .map(|m| format!("{{{{{m}}}}}"))
            .collect::<Vec<_>>()
            .join("、");
        let mut lostg = shared.lost.lock().await;
        for e in &step.extract {
            lostg.insert(e.name.clone());
        }
        return blocked_result(
            step,
            pos,
            format!("依赖的变量 {list} 因前序步骤失败 / 跳过而缺失"),
        );
    }

    // ---- 执行条件：不满足 → skipped（不发请求；不算失败，不触发失败即停）----
    if let Some(cond) = &step.condition {
        if !cond.is_valid() {
            let mut r = EndpointResult::failed_fast(
                step.endpoint.id,
                &step.endpoint,
                None,
                "执行条件配置无效（source / name / op 组合不合法）",
            );
            r.step_index = Some(pos);
            r.step_status = Some(step_status::FAILED.to_string());
            let mut lostg = shared.lost.lock().await;
            for e in &step.extract {
                lostg.insert(e.name.clone());
            }
            return r;
        }
        if !eval_condition(cond, &*shared.runtime.lock().await, prev_status, None) {
            let mut lostg = shared.lost.lock().await;
            for e in &step.extract {
                lostg.insert(e.name.clone());
            }
            return skipped_result(step, pos, &format!("条件不满足：{}", condition_text(cond)));
        }
    }

    // 步骤墙钟起点：耗时含执行前等待与轮间隔（限速是步骤的真实成本）。
    let wall = std::time::Instant::now();
    // ---- 执行前等待（限速 / 等待上游就绪；可取消）----
    if let Some(w) = step.wait_ms {
        if !sleep_cancellable(
            w.min(crate::config::StepLoop::MAX_INTERVAL_MS),
            options.cancel.as_ref(),
        )
        .await
        {
            let mut r = EndpointResult::failed_fast(
                step.endpoint.id,
                &step.endpoint,
                None,
                "等待期间运行被取消",
            );
            r.step_index = Some(pos);
            r.cancelled = true;
            r.step_status = Some(step_status::CANCELLED.to_string());
            return r;
        }
    }

    // ---- 循环（count / while）或单次发送 ----
    let loop_cfg = step.r#loop.clone().filter(|l| l.is_valid());
    if let Some(bad) = &step.r#loop {
        if loop_cfg.is_none() {
            // while 的 source / expression 用法错误直接报真实原因（issue #156）。
            let reason = bad.while_problem().unwrap_or_else(|| {
                "循环配置无效（mode / count / while_condition 组合不合法）".into()
            });
            let mut r = EndpointResult::failed_fast(step.endpoint.id, &step.endpoint, None, reason);
            r.step_index = Some(pos);
            r.step_status = Some(step_status::FAILED.to_string());
            let mut lostg = shared.lost.lock().await;
            for e in &step.extract {
                lostg.insert(e.name.clone());
            }
            return r;
        }
        let _ = bad;
    }

    let mut rounds: u32 = 0;
    let mut last: Option<EndpointResult> = None;
    let mut while_unmet = false;
    let cancel_fired = |o: &ScenarioOptions| o.cancel.as_ref().is_some_and(|c| c.is_cancelled());

    loop {
        if cancel_fired(options) {
            break;
        }
        if rounds > 0 {
            if let Some(l) = &loop_cfg {
                if l.interval() > 0
                    && !sleep_cancellable(l.interval(), options.cancel.as_ref()).await
                {
                    break;
                }
            }
        }
        let r = send_round(step, pos, shared, options, Some(emit_enter)).await;
        rounds += 1;
        let round_failed = !r.ok && !r.cancelled;
        let round_cancelled = r.cancelled;
        last = Some(r);
        if round_cancelled || round_failed {
            break;
        }
        // while 条件求值用「本轮自己的响应」：response.* 源取本轮状态码 /
        // 正文 / 头，prevStatus 也指本轮（并行批内不受其他步骤干扰）。
        // 经 last 借用读取（此前整份 clone 本轮响应；变量池同样借用读锁，
        // 求值为同步调用、持锁期间无 await）。
        let round_resp = last.as_ref().and_then(|x| x.response.as_ref());
        let st_now = round_resp.map(|x| x.status);
        match &loop_cfg {
            // count 模式：跑满即止。
            Some(l) if l.mode == "count" => {
                if rounds >= l.rounds_cap() {
                    break;
                }
            }
            // while 模式：每轮结束后求值，false 退出；达上限仍 true 判失败。
            Some(l) => {
                if rounds >= l.rounds_cap() {
                    if let Some(wc) = &l.while_condition {
                        let vars = shared.runtime.lock().await;
                        if eval_condition(wc, &vars, st_now, round_resp) {
                            while_unmet = true;
                        }
                    }
                    break;
                }
                if let Some(wc) = &l.while_condition {
                    let vars = shared.runtime.lock().await;
                    if !eval_condition(wc, &vars, st_now, round_resp) {
                        break;
                    }
                }
            }
            None => break,
        }
    }

    let mut result = last.unwrap_or_else(|| {
        // 首轮前就被取消（等待 / 取消竞态）：取消行。
        let mut r =
            EndpointResult::failed_fast(step.endpoint.id, &step.endpoint, None, "运行已取消");
        r.step_index = Some(pos);
        r.cancelled = true;
        r.step_status = Some(step_status::CANCELLED.to_string());
        r
    });
    result.duration_ms = Some(wall.elapsed().as_secs_f64() * 1000.0);
    result = with_loop_runs(result, rounds);
    if while_unmet {
        result.ok = false;
        result.status = None;
        result.step_status = Some(step_status::FAILED.to_string());
        result.request_error = Some(format!("循环条件在 {rounds} 轮后仍未满足"));
    }
    result
}

/// 并发上限：单批并行步数（写操作场景默认谨慎，压测另有专用通道）。
const PARALLEL_BATCH_CAP: usize = 8;

/// 运行集成测试场景：按步骤顺序执行；相邻 parallel 步骤成批并发。
///
/// 每步执行前用「环境静态变量 + runtime 快照（提取值优先）」渲染该步的
/// URL / 规格模板——这就是链路数据流的打通点：A 步 extract 出 `token`，
/// B 步模板里的 `{{token}}` 即取到 A 的响应值。
///
/// 执行策略语义（六态状态落 `EndpointResult.step_status`）：
/// - 禁用步骤 → skipped（不发请求）；
/// - 执行条件不满足 → skipped（不算失败，不触发失败即停）；
/// - 引用了「应产自失败 / Blocked / Skipped 步骤」的缺失变量 → blocked
///   （不发请求，不允许用空变量继续）；
/// - 循环：任一轮失败即整步 failed；while 达上限条件仍成立 → failed；
/// - 失败即停 / 取消后的剩余步骤 → skipped；
/// - JSON Body 类型保留渲染（`render_spec_typed`：提取的数字进 Body 仍是数字）；
/// - 每步渲染后校验 URL scheme 仅 http/https（对齐 execute_request）。
pub async fn run_scenario(
    steps: Vec<ScenarioStepInput>,
    base_vars: &HashMap<String, String>,
    options: &ScenarioOptions,
    progress: Option<&(dyn Fn(ScenarioProgress<'_>) + Send + Sync)>,
) -> ScenarioRunResult {
    let total = steps.len();
    let warnings = undefined_var_warnings(&steps, base_vars);
    // runtime 以环境静态变量为底，提取值按步覆盖（runtime 优先级更高）。
    let shared = std::sync::Arc::new(SharedRunState {
        runtime: AsyncMutex::new(base_vars.clone()),
        typed: AsyncMutex::new(HashMap::new()),
        lost: AsyncMutex::new(std::collections::HashSet::new()),
        last_status: AsyncMutex::new(None),
    });
    let mut results: Vec<EndpointResult> = Vec::with_capacity(total);
    let mut vars_snapshots = Vec::with_capacity(total);
    let mut cancelled = false;
    let mut stopped_on_failure = false;

    /// 推一行结果并报「该步完成」进度。
    macro_rules! push_row {
        ($step:expr, $result:expr) => {{
            let done = results.len() + 1;
            results.push($result);
            if let Some(cb) = progress {
                cb(ScenarioProgress {
                    done,
                    total,
                    step: results.last(),
                    url: None,
                });
            }
            let _ = $step;
        }};
    }

    let mut idx = 0usize;
    while idx < total {
        let step = &steps[idx];

        // ---- 禁用步骤：占位 skipped，不发送 ----
        if !step.enabled {
            vars_snapshots.push(shared.runtime.lock().await.clone());
            push_row!(step, skipped_result(step, idx, "步骤已禁用"));
            idx += 1;
            continue;
        }

        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            cancelled = true;
            break;
        }

        // ---- 并行批：从 idx 起连续 parallel 启用步骤（≥2 成批并发）----
        if step.parallel {
            let mut end = idx;
            while end < total && steps[end].enabled && steps[end].parallel {
                end += 1;
            }
            if end - idx >= 2 {
                let snapshot = shared.runtime.lock().await.clone();
                let (tx, mut rx) = mpsc::unbounded_channel::<BatchMsg>();
                // 并发上限（信号量）：大批量时在途请求不超过 PARALLEL_BATCH_CAP。
                let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL_BATCH_CAP));
                for (k, s) in steps[idx..end].iter().enumerate() {
                    let step_arc = std::sync::Arc::new(s.clone());
                    let shared_c = shared.clone();
                    let opts_c = options.clone();
                    let tx_c = tx.clone();
                    let permits_c = permits.clone();
                    let pos = idx + k;
                    tokio::spawn(async move {
                        let _permit = match permits_c.acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => return, // 信号量已关闭（不会发生：主循环持引用）
                        };
                        let r = execute_step(&step_arc, pos, &shared_c, &opts_c, &|url| {
                            let _ = tx_c.send(BatchMsg::Enter { pos, url });
                        })
                        .await;
                        drop(_permit);
                        let _ = tx_c.send(BatchMsg::Done {
                            pos,
                            result: Box::new(r),
                        });
                    });
                }
                drop(tx);
                let mut slot: Vec<Option<EndpointResult>> =
                    (0..(end - idx)).map(|_| None).collect();
                let mut batch_entered: std::collections::HashSet<usize> = Default::default();
                while let Some(msg) = rx.recv().await {
                    match msg {
                        BatchMsg::Enter { pos, url } => {
                            batch_entered.insert(pos);
                            if let Some(cb) = progress {
                                cb(ScenarioProgress {
                                    done: pos,
                                    total,
                                    step: None,
                                    url: Some(&url),
                                });
                            }
                        }
                        BatchMsg::Done { pos, result } => {
                            slot[pos - idx] = Some(*result);
                        }
                    }
                    if slot.iter().all(|s| s.is_some()) {
                        break;
                    }
                }
                for (k, s) in steps[idx..end].iter().enumerate() {
                    vars_snapshots.push(snapshot.clone());
                    let r = slot[k].take().unwrap_or_else(|| {
                        let mut r = EndpointResult::failed_fast(
                            s.endpoint.id,
                            &s.endpoint,
                            None,
                            "并行批任务未返回（取消或调度异常）",
                        );
                        r.step_index = Some(idx + k);
                        r.cancelled = true;
                        r.step_status = Some(step_status::CANCELLED.to_string());
                        r
                    });
                    push_row!(s, r);
                }
                if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                    cancelled = true;
                    break;
                }
                if options.stop_on_failure
                    && results
                        .iter()
                        .rev()
                        .take(end - idx)
                        .any(|r| !r.ok && !r.cancelled)
                {
                    stopped_on_failure = true;
                    break;
                }
                idx = end;
                continue;
            }
        }

        // ---- 串行步骤：完整管线（Blocked / 条件 / 等待 / 循环 / 发送）----
        vars_snapshots.push(shared.runtime.lock().await.clone());
        let enter_cb = |url: String| {
            if let Some(cb) = progress {
                cb(ScenarioProgress {
                    done: idx,
                    total,
                    step: None,
                    url: Some(&url),
                });
            }
        };
        let r = execute_step(step, idx, &shared, options, &enter_cb).await;
        let failed = !r.ok && !r.cancelled;
        push_row!(step, r);
        // 取消优先于失败即停：在途请求被中止的步骤 ok = false 但非真实失败，
        // 先判取消再判失败，避免「取消」被误报为「失败即停」。
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            cancelled = true;
            break;
        }
        if failed && options.stop_on_failure {
            stopped_on_failure = true;
            break;
        }
        idx += 1;
    }

    // ---- 早停补账：失败即停 / 取消后的剩余启用步骤落 skipped 行 ----
    if cancelled || stopped_on_failure {
        let reason = if cancelled {
            "运行已取消"
        } else {
            "失败即停，未执行"
        };
        for (idx, step) in steps.iter().enumerate().skip(results.len()) {
            vars_snapshots.push(shared.runtime.lock().await.clone());
            push_row!(step, skipped_result(step, idx, reason));
        }
    }

    // 此时批任务已全部回收（rx 关闭即全部 Done），Arc 应独占；防御性兜底空表。
    let final_vars = std::sync::Arc::try_unwrap(shared)
        .map(|s| s.runtime.into_inner())
        .unwrap_or_default();
    ScenarioRunResult {
        results,
        cancelled,
        stopped_on_failure,
        variables: final_vars,
        vars_snapshots,
        warnings,
    }
}

/// 按目录排序执行顺序：文件夹排序在前，文件夹内按接口 sort_order。
pub fn order_endpoints<'a>(
    endpoints: &'a [Endpoint],
    folder_order: &HashMap<Uuid, i64>,
) -> Vec<&'a Endpoint> {
    let mut pairs: Vec<(i64, i64, &Endpoint)> = endpoints
        .iter()
        .map(|ep| {
            let f = ep
                .folder_id
                .and_then(|id| folder_order.get(&id).copied())
                .unwrap_or(i64::MAX);
            (f, ep.sort_order, ep)
        })
        .collect();
    pairs.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.name.cmp(&b.2.name))
    });
    pairs.into_iter().map(|(_, _, ep)| ep).collect()
}

fn resolve_text(input: &str, vars: &HashMap<String, String>) -> String {
    miyo_core::resolve_variables(input, vars)
}

/// 是否需要解析：expected 为含 `{{}}` 的字符串时才需 clone + 解析。
fn needs_resolve(a: &crate::config::AssertionSpec) -> bool {
    matches!(&a.expected, Some(serde_json::Value::String(s)) if s.contains("{{"))
}

/// 断言 expected 里的 `{{变量}}` 先解析（仅在字符串期望时）。
fn resolve(
    a: &crate::config::AssertionSpec,
    vars: &HashMap<String, String>,
) -> crate::config::AssertionSpec {
    let mut a = a.clone();
    if let Some(serde_json::Value::String(s)) = &a.expected {
        if s.contains("{{") {
            a.expected = Some(serde_json::Value::String(resolve_text(s, vars)));
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, RequestSpec};

    fn ep(name: &str, path: &str, folder_id: Option<Uuid>, sort: i64) -> Endpoint {
        let request = RequestSpec {
            tests: Some(serde_json::json!({})),
            ..Default::default()
        };
        Endpoint {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            folder_id,
            name: name.to_string(),
            method: HttpMethod::GET,
            path: path.to_string(),
            description: String::new(),
            status: EndpointStatus::Developing,
            sort_order: sort,
            request,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn ep_with_tests(name: &str, path: &str) -> Endpoint {
        let mut e = ep(name, path, None, 0);
        e.request.tests = Some(serde_json::json!({
            "assertions": [{"type": "status", "op": "eq", "expected": 200}],
            "extract": [{"name": "last_n", "from": "body", "path": "$.n"}],
        }));
        e
    }

    #[tokio::test]
    async fn collection_runs_concurrently_in_input_order() {
        use tokio::net::TcpListener;
        let app = axum::Router::new()
            .route("/fast", axum::routing::get(|| async { "{\"n\":1}" }))
            .route(
                "/slow",
                axum::routing::get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    "{\"n\":2}"
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mk = |name: &str, path: &str| {
            let e = ep_with_tests(name, path);
            let url = format!("http://{addr}{path}");
            CollectionItem {
                endpoint: e,
                url,
                spec: RequestSpec::default(),
                force_send: false,
            }
        };
        // slow 在前：串行会是慢→快→慢；并发 2 下结果仍与输入同序。
        let items = vec![
            mk("slow-1", "/slow"),
            mk("fast", "/fast"),
            mk("slow-2", "/slow"),
        ];
        let options = CollectionOptions {
            concurrency: Some(2),
            timeout_ms: Some(5_000),
            cancel: None,
        };
        let mut runtime = HashMap::new();
        let started = std::time::Instant::now();
        let result = run_collection(items, &mut runtime, &options, None).await;
        let elapsed = started.elapsed();

        assert!(!result.cancelled);
        assert_eq!(result.results.len(), 3);
        let names: Vec<&str> = result
            .results
            .iter()
            .map(|r| r.endpoint_name.as_str())
            .collect();
        assert_eq!(names, vec!["slow-1", "fast", "slow-2"]);
        assert!(result.results.iter().all(|r| r.ok), "断言应全过");
        assert!(
            elapsed.as_millis() < 400,
            "并发 2 跑 2×150ms 慢请求应远小于串行 300ms+，实际 {}ms",
            elapsed.as_millis()
        );
        assert!(
            runtime.contains_key("last_n"),
            "extract 成果应回写运行时变量表"
        );
    }

    #[test]
    fn ordering_by_folder_then_sort() {
        let f1 = Uuid::new_v4();
        let f2 = Uuid::new_v4();
        let mut map = HashMap::new();
        map.insert(f1, 2);
        map.insert(f2, 1);
        let eps = vec![
            ep("c", "/c", Some(f1), 10),
            ep("a", "/a", None, 5),
            ep("b", "/b", Some(f2), 1),
            ep("d", "/d", Some(f1), 1),
        ];
        let ordered = order_endpoints(&eps, &map);
        let names: Vec<&str> = ordered.iter().map(|e| e.name.as_str()).collect();
        // f2 (order 1) 的 b 在前；f1（order 2）内按 sort_order：d(1) 在 c(10) 前；根目录最后。
        assert_eq!(names, vec!["b", "d", "c", "a"]);
    }
}
