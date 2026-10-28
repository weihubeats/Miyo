//! 测试 Command：单接口断言测试（`test_endpoint`）+ 并发压测（`load_test`）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use miyo_core::model::{Endpoint, HttpMethod, RequestSpec};

use crate::commands::request::{apply_global_params, render_spec, resolve_timeout_ms};
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// `test_endpoint` 入参。
#[derive(Debug, Clone, Deserialize)]
pub struct TestEndpointArgs {
    pub endpoint: Endpoint,
    pub url: String,
    pub environment_id: Option<Uuid>,
}

/// 运行接口配置的断言测试（tests 未配置时也走通，返回空结果集）。
#[tauri::command(rename_all = "camelCase")]
pub async fn test_endpoint(
    state: State<'_, AppState>,
    args: TestEndpointArgs,
) -> CommandResult<miyo_test::runner::EndpointResult> {
    // 三路独立查询同波次并发（原来串行 3~4 次 DB 往返）。
    // 全局参数 / 超时走进程内缓存，命中时零 DB 往返。
    let (vars, settings, timeout_ms) = tokio::join!(
        state.variables_for(args.environment_id),
        state.global_settings(),
        resolve_timeout_ms(&args.endpoint.request, &state),
    );
    let vars = vars?;
    let settings = settings?;
    let timeout_ms = timeout_ms?;
    let url = miyo_core::resolve_variables(&args.url, &vars);
    let mut spec = render_spec(&args.endpoint.request, &vars);
    apply_global_params(&mut spec, &settings.global_params, &vars);
    let mut runtime = std::collections::HashMap::new();
    let (result, _) = miyo_test::runner::run_endpoint(
        &args.endpoint,
        &url,
        &spec,
        &mut runtime,
        Some(timeout_ms),
    )
    .await;
    // 自增序列若被本次测试推进，回写磁盘（尽力而为）。后台执行，不加在返回路径上。
    let db = state.db.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = super::seq::sync_seq_counters_if_dirty(&db).await {
            tracing::warn!("[test_endpoint] 同步自增序列失败：{e}");
        }
    });
    Ok(result)
}

/// 压测配置。
#[derive(Debug, Clone, Deserialize)]
pub struct LoadTestArgs {
    pub url: String,
    pub method: HttpMethod,
    pub spec: RequestSpec,
    pub environment_id: Option<Uuid>,
    /// 并发数（默认 20）。
    pub concurrency: Option<usize>,
    /// 总请求数（默认 200）。
    pub total: Option<usize>,
    /// 本次压测的运行标识（由前端生成；提供后可通过 `cancel_load_test` 中止）。
    #[serde(default)]
    pub run_id: Option<String>,
}

/// 并发压测：发送 `total` 次请求，最多 `concurrency` 个同时进行。
/// 进度经事件 `miyo:load-progress` 实时推送（done/total/ok/failed）。
///
/// 超时口径：接口级 `timeout_ms` > 全局设置（与单次发送一致），不再硬编码 30s；
/// 响应体只读前 64KB（只关心状态码/耗时，避免大响应内存爆炸）。
#[tauri::command(rename_all = "camelCase")]
pub async fn load_test(
    app: AppHandle,
    state: State<'_, AppState>,
    args: LoadTestArgs,
) -> CommandResult<miyo_test::load::LoadResult> {
    if args.url.trim().is_empty() {
        return Err(CommandError::validation("URL 不能为空"));
    }
    let concurrency = args.concurrency.unwrap_or(20).clamp(1, 500);
    let total = args.total.unwrap_or(200).clamp(1, 100_000);
    // 三路独立查询同波次并发（原来串行 3~4 次 DB 往返）。
    let (vars, settings, timeout_ms) = tokio::join!(
        state.variables_for(args.environment_id),
        state.global_settings(),
        resolve_timeout_ms(&args.spec, &state),
    );
    let vars = vars?;
    let settings = settings?;
    let timeout_ms = timeout_ms?;
    let url = miyo_core::resolve_variables(&args.url, &vars);
    let mut spec = render_spec(&args.spec, &vars);
    apply_global_params(&mut spec, &settings.global_params, &vars);
    let cfg = miyo_test::load::LoadConfig { concurrency, total };

    // 注册取消令牌（前端「取消压测」→ `cancel_load_test` 触发中止）。
    let cancel = args.run_id.as_ref().map(|id| {
        let token = tokio_util::sync::CancellationToken::new();
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), token.clone());
        (id.clone(), token)
    });
    let options = miyo_test::load::LoadOptions {
        timeout_ms: Some(timeout_ms),
        cancel: cancel.as_ref().map(|(_, t)| t.clone()),
        body_cap_bytes: Some(64 * 1024),
    };

    // 进度节流：高并发短请求下逐请求 emit 会产生每秒数千次 IPC，
    // 按 100ms 时间窗合并；终态（done == total）必发一次保证收尾准确。
    let started = Instant::now();
    let last_emit_ms = AtomicU64::new(0);
    let progress = move |p: miyo_test::load::LoadProgress| {
        let now = started.elapsed().as_millis() as u64;
        if p.done < p.total && now.saturating_sub(last_emit_ms.load(Ordering::Relaxed)) < 100 {
            return;
        }
        last_emit_ms.store(now, Ordering::Relaxed);
        let _ = app.emit("miyo:load-progress", &p);
    };
    let result =
        miyo_test::load::run_load_with(args.method, &url, &spec, &cfg, &options, Some(&progress))
            .await;

    // 无论完成 / 取消，都从注册表移除，避免泄漏。
    if let Some((id, _)) = &cancel {
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    // 压测结束后回写一次自增序列（高并发下避免逐请求落库）。
    if let Err(e) = super::seq::sync_seq_counters_if_dirty(&state.db).await {
        tracing::warn!("[load_test] 同步自增序列失败：{e}");
    }
    Ok(result)
}

/// 取消一个在途压测（`run_id` 不存在或已完成时返回 `false`）。
#[tauri::command(rename_all = "camelCase")]
pub fn cancel_load_test(state: State<'_, AppState>, run_id: String) -> CommandResult<bool> {
    Ok(cancel_run(&state, &run_id))
}

/// 取消一个在途集合测试（`run_id` 不存在或已完成时返回 `false`）。
#[tauri::command(rename_all = "camelCase")]
pub fn cancel_test_collection(state: State<'_, AppState>, run_id: String) -> CommandResult<bool> {
    Ok(cancel_run(&state, &run_id))
}

/// 长任务取消的公共实现（压测 / 集合测试共用 `run_cancels` 注册表）。
fn cancel_run(state: &AppState, run_id: &str) -> bool {
    let token = state
        .run_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(run_id);
    if let Some(token) = token {
        token.cancel();
        true
    } else {
        false
    }
}

/// 集合测试条目（url/spec 由前端按当前草稿原文透传，后端统一渲染）。
#[derive(Debug, Clone, Deserialize)]
pub struct TestCollectionItem {
    pub endpoint: Endpoint,
    pub url: String,
    pub spec: RequestSpec,
}

/// 集合测试入参。
#[derive(Debug, Clone, Deserialize)]
pub struct TestCollectionArgs {
    pub items: Vec<TestCollectionItem>,
    pub environment_id: Option<Uuid>,
    /// 并发度（默认 5，上限 64；1 = 严格串行）。
    pub concurrency: Option<usize>,
    /// 运行标识（由前端生成；提供后可通过 `cancel_test_collection` 中止）。
    #[serde(default)]
    pub run_id: Option<String>,
}

/// 集合测试进度（事件 `miyo:test-progress` 载荷）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TestCollectionProgress {
    pub done: usize,
    pub total: usize,
}

/// 并发跑集合测试：一次 IPC 跑完整个集合（原来前端 N 次 IPC 逐个调
/// `test_endpoint`：N 次变量加载 + 串行等待 + 无取消）。
///
/// - 变量表 / 全局参数 / 超时只加载一次，各条目复用；
/// - 结果与输入同序；进度经 `miyo:test-progress` 推送；可取消。
#[tauri::command(rename_all = "camelCase")]
pub async fn test_collection(
    app: AppHandle,
    state: State<'_, AppState>,
    args: TestCollectionArgs,
) -> CommandResult<miyo_test::runner::CollectionResult> {
    if args.items.is_empty() {
        return Err(CommandError::validation("测试集合为空"));
    }
    if args.items.len() > 2000 {
        return Err(CommandError::validation("单次集合测试最多 2000 个接口"));
    }
    // 三路独立查询同波次并发。
    let (vars, settings, timeout_ms) = tokio::join!(
        state.variables_for(args.environment_id),
        state.global_settings(),
        resolve_timeout_ms(&args.items[0].spec, &state),
    );
    let vars = vars?;
    let settings = settings?;
    let timeout_ms = timeout_ms?;

    let items: Vec<miyo_test::runner::CollectionItem> = args
        .items
        .iter()
        .map(|it| {
            let url = miyo_core::resolve_variables(&it.url, &vars);
            let mut spec = render_spec(&it.spec, &vars);
            apply_global_params(&mut spec, &settings.global_params, &vars);
            miyo_test::runner::CollectionItem {
                endpoint: it.endpoint.clone(),
                url,
                spec,
            }
        })
        .collect();

    let cancel = args.run_id.as_ref().map(|id| {
        let token = tokio_util::sync::CancellationToken::new();
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), token.clone());
        (id.clone(), token)
    });
    let options = miyo_test::runner::CollectionOptions {
        concurrency: args.concurrency,
        timeout_ms: Some(timeout_ms),
        cancel: cancel.as_ref().map(|(_, t)| t.clone()),
    };
    let progress = move |done: usize, total: usize| {
        let _ = app.emit(
            "miyo:test-progress",
            &TestCollectionProgress { done, total },
        );
    };
    let mut runtime = std::collections::HashMap::new();
    let result =
        miyo_test::runner::run_collection(items, &mut runtime, &options, Some(&progress)).await;

    if let Some((id, _)) = &cancel {
        state
            .run_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }
    if let Err(e) = super::seq::sync_seq_counters_if_dirty(&state.db).await {
        tracing::warn!("[test_collection] 同步自增序列失败：{e}");
    }
    Ok(result)
}
