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

use crate::assert::{evaluate, Outcome};
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub body_omitted: bool,
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// 响应快照（发送成功时存在；failed_fast 无响应）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<StepResponse>,
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
    let (result, resp) = run_one(ep, url, spec, &shared, timeout_ms, None).await;
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
    if config.is_empty() {
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
    let snapshot = runtime.lock().await.clone();
    let mut injected = HashMap::with_capacity(config.pre_request.len());
    for p in &config.pre_request {
        injected.insert(p.name.clone(), resolve_text(&p.value, &snapshot));
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
    let snapshot = runtime.lock().await.clone();
    let body_value: Option<Value> = serde_json::from_slice(resp.body.as_ref()).ok();
    let outcomes: Vec<Outcome> = config
        .assertions
        .iter()
        .map(|a| {
            if needs_resolve(a) {
                evaluate(&resolve(a, &snapshot), &resp, body_value.as_ref())
            } else {
                evaluate(a, &resp, body_value.as_ref())
            }
        })
        .collect();
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
///（不读 `endpoint.tests`）。
#[derive(Debug, Clone)]
pub struct ScenarioStepInput {
    pub endpoint: Endpoint,
    pub url_template: String,
    pub spec_template: RequestSpec,
    pub extract: Vec<crate::config::ExtractSpec>,
    pub assertions: Vec<crate::config::AssertionSpec>,
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
}

/// 运行集成测试场景：严格按步骤顺序串行执行。
///
/// 每步执行前用「环境静态变量 + runtime 快照（提取值优先）」渲染该步的
/// URL / 规格模板——这就是链路数据流的打通点：A 步 extract 出 `token`，
/// B 步模板里的 `{{token}}` 即取到 A 的响应值。
pub async fn run_scenario(
    steps: Vec<ScenarioStepInput>,
    base_vars: &HashMap<String, String>,
    options: &ScenarioOptions,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> ScenarioRunResult {
    let total = steps.len();
    // runtime 以环境静态变量为底，提取值按步覆盖（runtime 优先级更高）。
    let runtime = AsyncMutex::new(base_vars.clone());
    let mut results = Vec::with_capacity(total);
    let mut vars_snapshots = Vec::with_capacity(total);
    for (idx, step) in steps.iter().enumerate() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return ScenarioRunResult {
                results,
                cancelled: true,
                stopped_on_failure: false,
                variables: runtime.into_inner(),
                vars_snapshots,
            };
        }
        let render_vars = runtime.lock().await.clone();
        // 每步执行前的 runtime 快照（重跑失败步骤的初始变量来源）。
        vars_snapshots.push(render_vars.clone());
        let url = miyo_core::resolve_variables(&step.url_template, &render_vars);
        let spec = miyo_core::render_spec(&step.spec_template, &render_vars);
        let config = TestSpec {
            pre_request: Vec::new(),
            extract: step.extract.clone(),
            assertions: step.assertions.clone(),
        };
        // 每步独立超时：接口自带 timeout_ms 优先，否则用场景级解析值兜底。
        let step_timeout = step.endpoint.request.timeout_ms.or(options.timeout_ms);
        let (mut result, _) = run_one_core(
            &step.endpoint,
            &url,
            &spec,
            config,
            &runtime,
            step_timeout,
            options.cancel.as_ref(),
        )
        .await;
        // 结果自描述：链路位次 + 渲染后实际 URL（前端 / CLI 免按位对齐猜测）。
        result.step_index = Some(idx);
        let failed = !result.ok;
        results.push(result);
        if let Some(cb) = progress {
            cb(idx + 1, total);
        }
        // 取消优先于失败即停：在途请求被中止的步骤 ok = false 但非真实失败，
        // 先判取消再判失败，避免「取消」被误报为「失败即停」。
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return ScenarioRunResult {
                results,
                cancelled: true,
                stopped_on_failure: false,
                variables: runtime.into_inner(),
                vars_snapshots,
            };
        }
        if failed && options.stop_on_failure {
            return ScenarioRunResult {
                results,
                cancelled: false,
                stopped_on_failure: true,
                variables: runtime.into_inner(),
                vars_snapshots,
            };
        }
    }
    ScenarioRunResult {
        results,
        cancelled: false,
        stopped_on_failure: false,
        variables: runtime.into_inner(),
        vars_snapshots,
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
