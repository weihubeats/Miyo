//! 测试套件（Test Suite）与批量执行（Test Run Batch）命令。
//!
//! 设计：
//! * 套件只保存场景引用（顺序 / 启用），不复制场景步骤；
//! * 批量执行调度走 `miyo_test::batch::execute_batch`（串行 / 并行 / 失败策略 /
//!   重试 / 暂停 / 取消），单场景执行复用 `run_scenario_lifecycle` 与
//!   `persist_scenario_run`——不另起一套执行引擎；
//! * 批次与明细先落库再执行，逐项更新状态并推送 `miyo:test-batch-progress`；
//! * 应用异常退出后，启动时 `recover_interrupted_batches` 把未终态批次标记
//!   interrupted，UI 可「恢复执行」（把 interrupted/pending 明细重置为 pending
//!   续跑）或重新执行（新建批次）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use miyo_core::model::{
    Endpoint, GlobalParam, Scenario, ScenarioStep, TestBatchItemStatus, TestBatchMode,
    TestBatchStatus, TestFailurePolicy, TestRunBatch, TestRunBatchItem, TestSuite, TestSuiteItem,
};
use miyo_storage::repository as repo;
use miyo_test::runner::{ScenarioLifecycle, ScenarioOptions, ScenarioStepInput};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 批量执行进度事件名（每项开始 / 结束 + 每波次汇总）。
pub const TEST_BATCH_PROGRESS_EVENT: &str = "miyo:test-batch-progress";

/// 在途批次的控制句柄（暂停标志 + 取消令牌）。
pub struct BatchControl {
    pub cancel: CancellationToken,
    pub paused: Arc<AtomicBool>,
}

/// 在途批次注册表（batch_id → 控制句柄）。
///
/// 批次之间完全隔离：不同 batch_id 各自持有取消令牌与暂停标志，互不污染。
#[derive(Default)]
pub struct TestBatchRegistry {
    controls: Mutex<HashMap<Uuid, Arc<BatchControl>>>,
}

impl TestBatchRegistry {
    fn insert(&self, batch_id: Uuid, control: Arc<BatchControl>) -> bool {
        let mut map = self.controls.lock().unwrap_or_else(|p| p.into_inner());
        if map.contains_key(&batch_id) {
            return false;
        }
        map.insert(batch_id, control);
        true
    }

    fn get(&self, batch_id: Uuid) -> Option<Arc<BatchControl>> {
        self.controls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&batch_id)
            .cloned()
    }

    fn remove(&self, batch_id: Uuid) {
        self.controls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&batch_id);
    }

    /// 是否有在途批次（退出前诊断 / 测试用）。
    pub fn active_count(&self) -> usize {
        self.controls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

fn default_true() -> bool {
    true
}

fn validate_suite_name(name: &str) -> CommandResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CommandError::validation("套件名称不能为空"));
    }
    if name.chars().count() > 120 {
        return Err(CommandError::validation("套件名称不能超过 120 个字符"));
    }
    Ok(name.to_string())
}

// ---------- 套件 CRUD ----------

/// 套件 + 成员（列表页单次 IPC）。
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TestSuiteWithItems {
    #[serde(flatten)]
    pub suite: TestSuite,
    pub items: Vec<TestSuiteItem>,
}

/// 列出项目的全部套件（含成员；成员按套件内顺序）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_test_suites(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<TestSuiteWithItems>> {
    repo::get_project(&state.db, project_id).await?;
    let suites = repo::list_test_suites(&state.db, project_id).await?;
    let items = repo::list_test_suite_items_for_project(&state.db, project_id).await?;
    let mut by_suite: HashMap<Uuid, Vec<TestSuiteItem>> = HashMap::new();
    for item in items {
        by_suite.entry(item.suite_id).or_default().push(item);
    }
    Ok(suites
        .into_iter()
        .map(|suite| TestSuiteWithItems {
            items: by_suite.remove(&suite.id).unwrap_or_default(),
            suite,
        })
        .collect())
}

/// 保存套件（整体覆盖写：套件字段 + 成员列表）。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_test_suite(
    state: State<'_, AppState>,
    mut suite: TestSuite,
    mut items: Vec<TestSuiteItem>,
) -> CommandResult<TestSuiteWithItems> {
    suite.name = validate_suite_name(&suite.name)?;
    suite.description = suite.description.trim().to_string();
    repo::get_project(&state.db, suite.project_id).await?;
    if let Some(folder_id) = suite.folder_id {
        let folder = repo::get_scenario_folder(&state.db, folder_id).await?;
        if folder.project_id != suite.project_id {
            return Err(CommandError::validation("目录不属于当前项目"));
        }
    }
    // 已存在则保留 created_at（前端复制对象时可能带入错误时间）。
    if let Ok(existing) = repo::get_test_suite(&state.db, suite.id).await {
        if existing.project_id != suite.project_id {
            return Err(CommandError::validation("套件不属于当前项目"));
        }
        suite.created_at = existing.created_at;
    }
    // 成员校验：场景必须存在且同项目；同一场景不得重复引用。
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut normalized: Vec<TestSuiteItem> = Vec::with_capacity(items.len());
    for (idx, item) in items.drain(..).enumerate() {
        if !seen.insert(item.scenario_id) {
            return Err(CommandError::validation("同一场景不能重复加入套件"));
        }
        let scenario = repo::get_scenario(&state.db, item.scenario_id).await?;
        if scenario.project_id != suite.project_id {
            return Err(CommandError::validation("场景不属于当前项目"));
        }
        normalized.push(TestSuiteItem {
            id: item.id,
            suite_id: suite.id,
            scenario_id: item.scenario_id,
            sort_order: idx as i64,
            enabled: item.enabled,
        });
    }
    suite.updated_at = chrono::Utc::now();
    repo::save_test_suite(&state.db, &suite, &normalized).await?;
    Ok(TestSuiteWithItems {
        suite,
        items: normalized,
    })
}

/// 删除套件（批次历史保留）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_test_suite(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    Ok(repo::delete_test_suite(&state.db, id).await?)
}

/// 复制套件（新 id / 新成员 id）。
#[tauri::command(rename_all = "camelCase")]
pub async fn duplicate_test_suite(
    state: State<'_, AppState>,
    id: Uuid,
) -> CommandResult<TestSuiteWithItems> {
    let suite = repo::duplicate_test_suite(&state.db, id).await?;
    let items = repo::list_test_suite_items(&state.db, suite.id).await?;
    Ok(TestSuiteWithItems { suite, items })
}

/// 套件内重排。
#[tauri::command(rename_all = "camelCase")]
pub async fn reorder_test_suite_items(
    state: State<'_, AppState>,
    suite_id: Uuid,
    ids: Vec<Uuid>,
) -> CommandResult<()> {
    Ok(repo::reorder_test_suite_items(&state.db, suite_id, &ids).await?)
}

/// 启用 / 禁用套件成员。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_test_suite_item_enabled(
    state: State<'_, AppState>,
    item_id: Uuid,
    enabled: bool,
) -> CommandResult<()> {
    Ok(repo::set_test_suite_item_enabled(&state.db, item_id, enabled).await?)
}

// ---------- 批量执行 ----------

/// 批量执行入参。
#[derive(Debug, Deserialize)]
pub struct RunTestBatchArgs {
    pub project_id: Uuid,
    /// 套件全量执行入口（与 scenario_ids / folder_id 三选一）。
    #[serde(default)]
    pub suite_id: Option<Uuid>,
    /// 手动勾选场景（保持传入顺序）。
    #[serde(default)]
    pub scenario_ids: Vec<Uuid>,
    /// 文件夹内全部场景执行（含子目录）。
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub environment_id: Option<Uuid>,
    #[serde(default)]
    pub mode: Option<TestBatchMode>,
    #[serde(default)]
    pub concurrency: Option<i64>,
    #[serde(default)]
    pub failure_policy: Option<TestFailurePolicy>,
    #[serde(default)]
    pub retry_count: Option<i64>,
    /// 跳过套件里被禁用的场景（默认 true；跳过项仍出现在报告里）。
    #[serde(default = "default_true")]
    pub skip_disabled: bool,
    /// 重跑来源批次（失败场景重试 / 整批重跑；新批次不覆盖历史）。
    #[serde(default)]
    pub parent_batch_id: Option<Uuid>,
}

/// 批次详情（批次 + 明细）。
#[derive(Debug, Serialize)]
pub struct TestBatchDetail {
    #[serde(flatten)]
    pub batch: TestRunBatch,
    pub items: Vec<TestRunBatchItem>,
}

impl TestBatchDetail {
    async fn load(db: &sqlx::SqlitePool, id: Uuid) -> CommandResult<Option<Self>> {
        let Some(batch) = repo::get_test_batch(db, id).await? else {
            return Ok(None);
        };
        let items = repo::list_test_batch_items(db, id).await?;
        Ok(Some(TestBatchDetail { batch, items }))
    }
}

/// 进度事件（item 有值 = 某项状态变化；否则为批次汇总）。
#[derive(Debug, Clone, Serialize)]
pub struct TestBatchProgressEvent {
    pub batch_id: String,
    pub status: String,
    pub total: usize,
    pub done: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: usize,
    pub running: usize,
    /// 当前正在执行的场景（并行时只带首个，逐项事件带 item）。
    pub current: Option<BatchCurrentItem>,
    pub item: Option<TestRunBatchItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCurrentItem {
    pub item_id: String,
    pub scenario_id: String,
    pub scenario_name: String,
}

/// 执行一次「套件 / 勾选 / 文件夹」批量，返回最终批次详情。
///
/// 所有入口最终都落到同一张批次表：新批次记录先落库，再逐项执行。
#[tauri::command(rename_all = "camelCase")]
pub async fn run_test_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    args: RunTestBatchArgs,
) -> CommandResult<TestBatchDetail> {
    repo::get_project(&state.db, args.project_id).await?;
    let resolved = resolve_batch_scenarios(&state, &args).await?;
    if resolved.is_empty() {
        return Err(CommandError::validation("没有可执行的场景"));
    }

    let now = chrono::Utc::now();
    let batch_id = Uuid::new_v4();
    let mode = args.mode.unwrap_or(TestBatchMode::Serial);
    let concurrency = match mode {
        TestBatchMode::Serial => 1,
        TestBatchMode::Parallel => args.concurrency.unwrap_or(3).clamp(1, 50),
    };
    let failure_policy = args.failure_policy.unwrap_or(TestFailurePolicy::Continue);
    let retry_count = args.retry_count.unwrap_or(0).clamp(0, 10);
    let mut items: Vec<TestRunBatchItem> = Vec::with_capacity(resolved.len());
    for (idx, (scenario_id, scenario_name, enabled)) in resolved.into_iter().enumerate() {
        let skipped = !enabled && args.skip_disabled;
        items.push(TestRunBatchItem {
            id: Uuid::new_v4(),
            batch_id,
            scenario_id,
            scenario_name,
            status: if skipped {
                TestBatchItemStatus::Skipped
            } else {
                TestBatchItemStatus::Pending
            },
            sort_order: idx as i64,
            attempt: 0,
            duration_ms: None,
            error_message: skipped.then(|| "场景已禁用（已跳过）".to_string()),
            execution_record_id: None,
            started_at: None,
            finished_at: skipped.then_some(now),
        });
    }
    let pending_count = items
        .iter()
        .filter(|i| i.status == TestBatchItemStatus::Pending)
        .count() as i64;
    let skipped_count = items
        .iter()
        .filter(|i| i.status == TestBatchItemStatus::Skipped)
        .count() as i64;
    let batch = TestRunBatch {
        id: batch_id,
        project_id: args.project_id,
        suite_id: args.suite_id,
        parent_batch_id: args.parent_batch_id,
        environment_id: args.environment_id,
        status: TestBatchStatus::Pending,
        mode,
        concurrency,
        failure_policy,
        retry_count,
        skip_disabled: args.skip_disabled,
        total_count: items.len() as i64,
        passed_count: 0,
        failed_count: 0,
        skipped_count,
        cancelled_count: 0,
        running_count: 0,
        pending_count,
        started_at: None,
        finished_at: None,
        duration_ms: None,
        created_at: now,
    };
    repo::create_test_batch(&state.db, &batch, &items).await?;
    run_batch_engine(&app, &state, batch_id).await
}

/// 恢复执行（暂停 / 中断 / 尚未开始的批次）：未完成明细重置为 pending 后续跑。
#[tauri::command(rename_all = "camelCase")]
pub async fn resume_test_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    batch_id: Uuid,
) -> CommandResult<TestBatchDetail> {
    let Some(batch) = repo::get_test_batch(&state.db, batch_id).await? else {
        return Err(CommandError::validation("批次不存在"));
    };
    if batch.status == TestBatchStatus::Running {
        return Err(CommandError::validation("批次正在执行中"));
    }
    if matches!(
        batch.status,
        TestBatchStatus::Completed | TestBatchStatus::Cancelled
    ) {
        return Err(CommandError::validation("批次已结束，无法恢复"));
    }
    // 中断 / 暂停批次：把 pending / running / interrupted 明细重置为 pending，
    // 已通过 / 失败 / 跳过的明细保持原状（报告不丢已完成部分）。
    let items = repo::list_test_batch_items(&state.db, batch_id).await?;
    for mut item in items {
        if matches!(
            item.status,
            TestBatchItemStatus::Pending
                | TestBatchItemStatus::Running
                | TestBatchItemStatus::Interrupted
        ) {
            item.status = TestBatchItemStatus::Pending;
            item.started_at = None;
            item.finished_at = None;
            item.error_message = None;
            repo::update_test_batch_item(&state.db, &item).await?;
        }
    }
    run_batch_engine(&app, &state, batch_id).await
}

/// 暂停：只停止调度尚未开始的后续场景；在途场景跑完当前波次。
#[tauri::command(rename_all = "camelCase")]
pub fn pause_test_batch(state: State<'_, AppState>, batch_id: Uuid) -> CommandResult<bool> {
    let Some(control) = state.test_batches.get(batch_id) else {
        return Ok(false);
    };
    control.paused.store(true, Ordering::SeqCst);
    Ok(true)
}

/// 取消整个批次（在途场景协作式取消；未开始项标记 cancelled）。
#[tauri::command(rename_all = "camelCase")]
pub async fn cancel_test_batch(state: State<'_, AppState>, batch_id: Uuid) -> CommandResult<bool> {
    if let Some(control) = state.test_batches.get(batch_id) {
        control.cancel.cancel();
        return Ok(true);
    }
    // 不在途（暂停 / 尚未开始）：直接把未完成项标记 cancelled 并收尾。
    let Some(mut batch) = repo::get_test_batch(&state.db, batch_id).await? else {
        return Ok(false);
    };
    if batch.status.is_terminal() {
        return Ok(false);
    }
    let items = repo::list_test_batch_items(&state.db, batch_id).await?;
    let now = chrono::Utc::now();
    for mut item in items {
        if !item.status.is_terminal() {
            item.status = TestBatchItemStatus::Cancelled;
            item.finished_at = Some(now);
            repo::update_test_batch_item(&state.db, &item).await?;
        }
    }
    batch.status = TestBatchStatus::Cancelled;
    batch.finished_at = Some(now);
    recompute_counts(
        &mut batch,
        &repo::list_test_batch_items(&state.db, batch_id).await?,
    );
    repo::update_test_batch(&state.db, &batch).await?;
    Ok(true)
}

/// 项目最近批次列表（新→旧；不含明细）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_test_batches(
    state: State<'_, AppState>,
    project_id: Uuid,
    limit: Option<i64>,
) -> CommandResult<Vec<TestRunBatch>> {
    Ok(repo::list_test_batches(&state.db, project_id, limit.unwrap_or(50).clamp(1, 200)).await?)
}

/// 批次详情（批次 + 明细）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_test_batch(
    state: State<'_, AppState>,
    id: Uuid,
) -> CommandResult<Option<TestBatchDetail>> {
    TestBatchDetail::load(&state.db, id).await
}

/// 删除批次（仅历史记录；单场景执行历史保留）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_test_batch(state: State<'_, AppState>, id: Uuid) -> CommandResult<()> {
    if state.test_batches.get(id).is_some() {
        return Err(CommandError::validation("批次正在执行中，请先取消"));
    }
    Ok(repo::delete_test_batch(&state.db, id).await?)
}

/// 启动恢复：把上次进程遗留的未完成批次标记 interrupted（返回标记数量）。
#[tauri::command(rename_all = "camelCase")]
pub async fn recover_test_batches(state: State<'_, AppState>) -> CommandResult<u64> {
    Ok(repo::recover_interrupted_batches(&state.db).await?)
}

// ---------- 内部：场景解析 / 执行引擎 ----------

/// 解析批量入口 → (场景 id, 场景名, 是否启用)，保持入口顺序。
async fn resolve_batch_scenarios(
    state: &State<'_, AppState>,
    args: &RunTestBatchArgs,
) -> CommandResult<Vec<(Uuid, String, bool)>> {
    let mut out: Vec<(Uuid, String, bool)> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    if let Some(suite_id) = args.suite_id {
        let suite = repo::get_test_suite(&state.db, suite_id).await?;
        if suite.project_id != args.project_id {
            return Err(CommandError::validation("套件不属于当前项目"));
        }
        for item in repo::list_test_suite_items(&state.db, suite_id).await? {
            if !seen.insert(item.scenario_id) {
                continue;
            }
            match repo::get_scenario(&state.db, item.scenario_id).await {
                Ok(scenario) => out.push((scenario.id, scenario.name, item.enabled)),
                // 场景已删除：保留明细并标记禁用（报告里可见「场景不存在」）。
                Err(_) => out.push((item.scenario_id, "（场景已删除）".to_string(), false)),
            }
        }
        return Ok(out);
    }
    if !args.scenario_ids.is_empty() {
        for id in &args.scenario_ids {
            if !seen.insert(*id) {
                continue;
            }
            let scenario = repo::get_scenario(&state.db, *id).await?;
            if scenario.project_id != args.project_id {
                return Err(CommandError::validation("存在不属于当前项目的场景"));
            }
            out.push((scenario.id, scenario.name, true));
        }
        return Ok(out);
    }
    if let Some(folder_id) = args.folder_id {
        let folder = repo::get_scenario_folder(&state.db, folder_id).await?;
        if folder.project_id != args.project_id {
            return Err(CommandError::validation("目录不属于当前项目"));
        }
        let folders = repo::list_scenario_folders(&state.db, args.project_id).await?;
        let scope = collect_folder_scope(&folders, folder_id);
        for scenario in repo::list_scenarios(&state.db, args.project_id).await? {
            let in_scope = scenario
                .folder_id
                .map(|f| scope.contains(&f))
                .unwrap_or(false);
            if in_scope && seen.insert(scenario.id) {
                out.push((scenario.id, scenario.name, true));
            }
        }
        return Ok(out);
    }
    Err(CommandError::validation(
        "请选择测试套件、勾选场景或指定目录",
    ))
}

/// 目录子树（含自身）。
fn collect_folder_scope(folders: &[miyo_core::model::ScenarioFolder], root: Uuid) -> HashSet<Uuid> {
    let mut scope = HashSet::new();
    scope.insert(root);
    loop {
        let mut grew = false;
        for folder in folders {
            if let Some(parent) = folder.parent_id {
                if scope.contains(&parent) && scope.insert(folder.id) {
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    scope
}

/// 批次执行核心：加载未完成明细 → 调度引擎 → 落最终状态。
async fn run_batch_engine(
    app: &AppHandle,
    state: &State<'_, AppState>,
    batch_id: Uuid,
) -> CommandResult<TestBatchDetail> {
    let Some(mut batch) = repo::get_test_batch(&state.db, batch_id).await? else {
        return Err(CommandError::validation("批次不存在"));
    };
    let items = repo::list_test_batch_items(&state.db, batch_id).await?;
    let pending: Vec<TestRunBatchItem> = items
        .into_iter()
        .filter(|i| i.status == TestBatchItemStatus::Pending)
        .collect();
    if pending.is_empty() {
        // 没有待执行项（全部禁用跳过 / 已完成批次再次恢复）：按现有明细收尾，
        // 不能传空数组，否则 total / passed / skipped 计数会被清空。
        let existing = repo::list_test_batch_items(&state.db, batch_id).await?;
        finalize_batch(&state.db, &mut batch, &existing, false, false).await?;
        let detail = TestBatchDetail::load(&state.db, batch_id)
            .await?
            .ok_or_else(|| CommandError::validation("批次不存在"))?;
        return Ok(detail);
    }

    // 变量 / 全局参数 / 超时：整批一次快照（环境切换 = 新建批次）。
    let vars = state.variables_for(batch.environment_id).await?;
    let settings = state.global_settings().await?;
    let global_params = settings.global_params.clone();
    let timeout_ms = settings
        .http_timeout_ms
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS);
    let java_hook = crate::commands::scenarios::java_hook_for(state).await;

    let cancel = CancellationToken::new();
    let paused = Arc::new(AtomicBool::new(false));
    let control = Arc::new(BatchControl {
        cancel: cancel.clone(),
        paused: paused.clone(),
    });
    if !state.test_batches.insert(batch_id, control) {
        return Err(CommandError::validation("批次正在执行中"));
    }

    let now = chrono::Utc::now();
    batch.status = TestBatchStatus::Running;
    if batch.started_at.is_none() {
        batch.started_at = Some(now);
    }
    batch.finished_at = None;
    batch.running_count = 0;
    batch.pending_count = pending.len() as i64;
    repo::update_test_batch(&state.db, &batch).await?;

    let items_by_id: HashMap<String, TestRunBatchItem> = pending
        .iter()
        .map(|i| (i.id.to_string(), i.clone()))
        .collect();
    let plan: Vec<miyo_test::batch::BatchPlanItem> = pending
        .iter()
        .map(|i| miyo_test::batch::BatchPlanItem {
            item_id: i.id.to_string(),
            scenario_id: i.scenario_id,
            name: i.scenario_name.clone(),
        })
        .collect();
    let policy = miyo_test::batch::BatchPolicy {
        mode: match batch.mode {
            TestBatchMode::Serial => miyo_test::batch::BatchMode::Serial,
            TestBatchMode::Parallel => miyo_test::batch::BatchMode::Parallel,
        },
        concurrency: batch.concurrency.max(1) as usize,
        failure_policy: match batch.failure_policy {
            TestFailurePolicy::Continue => miyo_test::batch::FailurePolicy::Continue,
            TestFailurePolicy::Stop => miyo_test::batch::FailurePolicy::Stop,
            TestFailurePolicy::Retry => miyo_test::batch::FailurePolicy::Retry,
            TestFailurePolicy::SkipDependents => miyo_test::batch::FailurePolicy::SkipDependents,
        },
        retry_count: batch.retry_count.max(0) as u32,
    };

    let db = state.db.clone();
    let batch_ref = batch.clone();
    // 恢复执行时已完成明细不在本轮 outcomes 里：进度条按「已完成 + 本轮」叠加，
    // 总数用批次总数（含套件里被跳过的禁用项），避免续跑后总数缩水。
    let base_total = batch.total_count as usize;
    let base_passed = batch.passed_count as usize;
    let base_failed = batch.failed_count as usize;
    let base_skipped = batch.skipped_count as usize;
    let base_cancelled = batch.cancelled_count as usize;
    let base_done = base_passed + base_failed + base_skipped + base_cancelled;
    let items_by_id_ref = &items_by_id;
    let vars_ref = &vars;
    let params_ref = global_params.as_slice();
    let cancel_ref = cancel.clone();
    let summary = miyo_test::batch::execute_batch(
        plan,
        policy,
        cancel.clone(),
        paused.clone(),
        |item, attempt| {
            let app = app.clone();
            let db = db.clone();
            let batch = batch_ref.clone();
            let base_item =
                items_by_id_ref
                    .get(&item.item_id)
                    .cloned()
                    .unwrap_or(TestRunBatchItem {
                        id: Uuid::new_v4(),
                        batch_id: batch_ref.id,
                        scenario_id: item.scenario_id,
                        scenario_name: item.name.clone(),
                        status: TestBatchItemStatus::Pending,
                        sort_order: 0,
                        attempt: 0,
                        duration_ms: None,
                        error_message: None,
                        execution_record_id: None,
                        started_at: None,
                        finished_at: None,
                    });
            let vars = vars_ref.clone();
            let params = params_ref.to_vec();
            let java_hook = java_hook.clone();
            let cancel = cancel_ref.clone();
            async move {
                execute_batch_item(
                    &app, &db, &batch, base_item, attempt, &vars, &params, timeout_ms, java_hook,
                    cancel,
                )
                .await
            }
        },
        |item| {
            let _ = app.emit(
                TEST_BATCH_PROGRESS_EVENT,
                TestBatchProgressEvent {
                    batch_id: batch_id.to_string(),
                    status: "running".into(),
                    total: base_total,
                    done: base_done,
                    passed: base_passed,
                    failed: base_failed,
                    skipped: base_skipped,
                    cancelled: base_cancelled,
                    running: 0,
                    current: Some(BatchCurrentItem {
                        item_id: item.item_id.clone(),
                        scenario_id: item.scenario_id.to_string(),
                        scenario_name: item.name.clone(),
                    }),
                    item: None,
                },
            );
        },
        |progress| {
            let _ = app.emit(
                TEST_BATCH_PROGRESS_EVENT,
                TestBatchProgressEvent {
                    batch_id: batch_id.to_string(),
                    status: "running".into(),
                    total: base_total,
                    done: base_done + progress.done,
                    passed: base_passed + progress.passed,
                    failed: base_failed + progress.failed,
                    skipped: base_skipped + progress.skipped,
                    cancelled: base_cancelled + progress.cancelled,
                    running: progress.running,
                    current: None,
                    item: None,
                },
            );
        },
    )
    .await;
    state.test_batches.remove(batch_id);

    // 剩余项（暂停时保持 pending；取消 / 停止由引擎已转成 cancelled / skipped）。
    for item in &summary.remaining {
        if let Some(mut row) = items_by_id_ref.get(&item.item_id).cloned() {
            row.status = TestBatchItemStatus::Pending;
            let _ = repo::update_test_batch_item(&db, &row).await;
        }
    }

    let final_items = repo::list_test_batch_items(&db, batch_id).await?;
    finalize_batch(
        &db,
        &mut batch,
        &final_items,
        summary.paused,
        summary.cancelled,
    )
    .await?;
    let detail = TestBatchDetail {
        batch,
        items: final_items,
    };
    let _ = app.emit(
        TEST_BATCH_PROGRESS_EVENT,
        TestBatchProgressEvent {
            batch_id: batch_id.to_string(),
            status: detail.batch.status.as_str().into(),
            total: detail.batch.total_count as usize,
            done: (detail.batch.passed_count
                + detail.batch.failed_count
                + detail.batch.skipped_count
                + detail.batch.cancelled_count) as usize,
            passed: detail.batch.passed_count as usize,
            failed: detail.batch.failed_count as usize,
            skipped: detail.batch.skipped_count as usize,
            cancelled: detail.batch.cancelled_count as usize,
            running: 0,
            current: None,
            item: None,
        },
    );
    Ok(detail)
}

/// 批次最终状态与计数。
async fn finalize_batch(
    db: &sqlx::SqlitePool,
    batch: &mut TestRunBatch,
    items: &[TestRunBatchItem],
    paused: bool,
    cancelled: bool,
) -> CommandResult<()> {
    recompute_counts(batch, items);
    batch.status = if paused {
        TestBatchStatus::Paused
    } else if cancelled {
        TestBatchStatus::Cancelled
    } else if batch.failed_count > 0 && batch.failure_policy != TestFailurePolicy::SkipDependents {
        TestBatchStatus::Failed
    } else {
        TestBatchStatus::Completed
    };
    let now = chrono::Utc::now();
    batch.running_count = 0;
    if batch.status.is_terminal() || batch.status == TestBatchStatus::Paused {
        batch.finished_at = Some(now);
        batch.duration_ms = batch
            .started_at
            .map(|start| (now - start).num_milliseconds().max(0) as f64);
    }
    repo::update_test_batch(db, batch).await?;
    Ok(())
}

fn recompute_counts(batch: &mut TestRunBatch, items: &[TestRunBatchItem]) {
    let count =
        |status: TestBatchItemStatus| items.iter().filter(|i| i.status == status).count() as i64;
    batch.total_count = items.len() as i64;
    batch.passed_count = count(TestBatchItemStatus::Passed);
    batch.failed_count = count(TestBatchItemStatus::Failed);
    batch.skipped_count = count(TestBatchItemStatus::Skipped);
    batch.cancelled_count = count(TestBatchItemStatus::Cancelled);
    batch.running_count = count(TestBatchItemStatus::Running);
    batch.pending_count =
        count(TestBatchItemStatus::Pending) + count(TestBatchItemStatus::Interrupted);
}

/// 执行批次中的单个场景（attempt 从 1 起；重试时同一明细多次执行）。
#[allow(clippy::too_many_arguments)]
async fn execute_batch_item(
    app: &AppHandle,
    db: &sqlx::SqlitePool,
    batch: &TestRunBatch,
    mut item: TestRunBatchItem,
    attempt: u32,
    vars: &HashMap<String, String>,
    global_params: &[GlobalParam],
    timeout_ms: u64,
    java_hook: Option<Arc<miyo_test::runner::JavaInvokeFn>>,
    cancel: CancellationToken,
) -> miyo_test::batch::ItemOutcome {
    let started_at = chrono::Utc::now();
    item.status = TestBatchItemStatus::Running;
    item.attempt = attempt as i64;
    item.started_at = Some(started_at);
    item.finished_at = None;
    item.error_message = None;
    let _ = repo::update_test_batch_item(db, &item).await;
    let _ = app.emit(
        TEST_BATCH_PROGRESS_EVENT,
        TestBatchProgressEvent {
            batch_id: batch.id.to_string(),
            status: "running".into(),
            total: batch.total_count as usize,
            done: 0,
            passed: batch.passed_count as usize,
            failed: batch.failed_count as usize,
            skipped: batch.skipped_count as usize,
            cancelled: batch.cancelled_count as usize,
            running: batch.running_count.max(1) as usize,
            current: Some(BatchCurrentItem {
                item_id: item.id.to_string(),
                scenario_id: item.scenario_id.to_string(),
                scenario_name: item.scenario_name.clone(),
            }),
            item: Some(item.clone()),
        },
    );

    let outcome = match load_and_run_scenario(
        db,
        batch,
        &item,
        vars,
        global_params,
        timeout_ms,
        java_hook,
        cancel,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(message) => miyo_test::batch::ItemOutcome {
            item_id: item.id.to_string(),
            scenario_id: item.scenario_id,
            status: miyo_test::batch::ItemStatus::Failed,
            attempts: attempt,
            duration_ms: 0.0,
            error: Some(message),
            run_id: None,
        },
    };

    let finished_at = chrono::Utc::now();
    item.status = match outcome.status {
        miyo_test::batch::ItemStatus::Passed => TestBatchItemStatus::Passed,
        miyo_test::batch::ItemStatus::Failed => TestBatchItemStatus::Failed,
        miyo_test::batch::ItemStatus::Skipped => TestBatchItemStatus::Skipped,
        miyo_test::batch::ItemStatus::Cancelled => TestBatchItemStatus::Cancelled,
    };
    item.duration_ms = Some(outcome.duration_ms);
    item.error_message = outcome.error.clone();
    item.execution_record_id = outcome
        .run_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok());
    item.finished_at = Some(finished_at);
    let _ = repo::update_test_batch_item(db, &item).await;
    let _ = app.emit(
        TEST_BATCH_PROGRESS_EVENT,
        TestBatchProgressEvent {
            batch_id: batch.id.to_string(),
            status: "running".into(),
            total: batch.total_count as usize,
            done: 0,
            passed: batch.passed_count as usize,
            failed: batch.failed_count as usize,
            skipped: batch.skipped_count as usize,
            cancelled: batch.cancelled_count as usize,
            running: 0,
            current: None,
            item: Some(item.clone()),
        },
    );
    outcome
}

/// 加载场景 → 物化步骤 → 执行 → 落单场景运行历史。
#[allow(clippy::too_many_arguments)]
async fn load_and_run_scenario(
    db: &sqlx::SqlitePool,
    batch: &TestRunBatch,
    item: &TestRunBatchItem,
    vars: &HashMap<String, String>,
    global_params: &[GlobalParam],
    timeout_ms: u64,
    java_hook: Option<Arc<miyo_test::runner::JavaInvokeFn>>,
    cancel: CancellationToken,
) -> Result<miyo_test::batch::ItemOutcome, String> {
    let scenario = repo::get_scenario(db, item.scenario_id)
        .await
        .map_err(|_| "场景不存在（可能已被删除）".to_string())?;
    let (steps, setup, teardown) =
        materialize_saved_scenario(db, &scenario, global_params, vars).await?;
    if steps.iter().all(|s| !s.enabled) {
        return Err("场景没有启用中的步骤".into());
    }
    let run_id = Uuid::new_v4();
    let mut steps = steps;
    miyo_test::runner::apply_retry_idempotency(&mut steps, &run_id.to_string());
    let snapshot = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string());
    let started_at = chrono::Utc::now();
    let lifecycle = ScenarioLifecycle {
        setup: &setup,
        teardown: &teardown,
        teardown_enabled: scenario.teardown_enabled,
    };
    let options = ScenarioOptions {
        timeout_ms: Some(timeout_ms),
        cancel: Some(cancel),
        // 批次级失败策略在调度层；单场景内继续跑完所有步骤以收集完整报告。
        stop_on_failure: false,
        java_invoke: java_hook,
    };
    let result =
        miyo_test::runner::run_scenario_lifecycle(&lifecycle, steps, vars, &options, None).await;

    // 落单场景执行历史（批次明细通过 execution_record_id 关联）。
    let meta = serde_json::json!({
        "batch": {
            "batchId": batch.id.to_string(),
            "itemId": item.id.to_string(),
            "attempt": item.attempt,
        }
    });
    if let Err(e) = crate::commands::scenarios::persist_scenario_run(
        db,
        scenario.id,
        batch.environment_id,
        Some(&run_id.to_string()),
        started_at,
        false,
        &snapshot,
        vars,
        result.results.len(),
        &result,
        Some(meta),
    )
    .await
    {
        tracing::warn!("[test_batch] 单场景运行历史落库失败：{e}");
    }

    let main = &result.results;
    let cancelled = result.cancelled || main.iter().any(|r| r.cancelled);
    let has_failure = main
        .iter()
        .any(|r| matches!(r.effective_status(), "failed" | "blocked"));
    let status = if cancelled {
        miyo_test::batch::ItemStatus::Cancelled
    } else if main.is_empty() || has_failure {
        miyo_test::batch::ItemStatus::Failed
    } else {
        miyo_test::batch::ItemStatus::Passed
    };
    let error =
        (status == miyo_test::batch::ItemStatus::Failed).then(|| first_failure_reason(main));
    let duration_ms = result
        .setup_results
        .iter()
        .chain(main.iter())
        .chain(result.teardown_results.iter())
        .filter_map(|r| r.duration_ms)
        .sum();
    Ok(miyo_test::batch::ItemOutcome {
        item_id: item.id.to_string(),
        scenario_id: item.scenario_id,
        status,
        attempts: item.attempt.max(1) as u32,
        duration_ms,
        error,
        run_id: Some(run_id.to_string()),
    })
}

/// 失败原因（首个失败 / Blocked 步：请求错误 > 断言原因 > 通用文案）。
fn first_failure_reason(results: &[miyo_test::runner::EndpointResult]) -> String {
    for r in results {
        if !matches!(r.effective_status(), "failed" | "blocked") {
            continue;
        }
        if let Some(err) = r.request_error.as_deref().filter(|s| !s.trim().is_empty()) {
            return err.to_string();
        }
        if let Some(outcome) = r.outcomes.iter().find(|o| !o.passed) {
            if let Some(reason) = outcome.reason.as_deref().filter(|s| !s.trim().is_empty()) {
                return reason.to_string();
            }
            return outcome.description.clone();
        }
        if let Some(reason) = r.blocked_reason.as_deref().filter(|s| !s.trim().is_empty()) {
            return reason.to_string();
        }
        return format!("{} {}", r.method, r.path);
    }
    "场景执行失败".into()
}

/// 已保存场景 → 运行入参（与单场景运行 / Agent 控制面同口径：引用式取接口
/// 当前配置 + 步骤覆盖；内联步骤自持；Java 步骤合成虚拟端点）。
async fn materialize_saved_scenario(
    db: &sqlx::SqlitePool,
    scenario: &Scenario,
    global_params: &[GlobalParam],
    vars: &HashMap<String, String>,
) -> Result<
    (
        Vec<ScenarioStepInput>,
        Vec<ScenarioStepInput>,
        Vec<ScenarioStepInput>,
    ),
    String,
> {
    async fn materialize_rows(
        db: &sqlx::SqlitePool,
        scenario: &Scenario,
        rows: &[ScenarioStep],
        global_params: &[GlobalParam],
        vars: &HashMap<String, String>,
        label_prefix: &str,
    ) -> Result<Vec<ScenarioStepInput>, String> {
        let project_id = scenario.project_id;
        let created_at = scenario.created_at;
        let updated_at = scenario.updated_at;
        let mut out = Vec::with_capacity(rows.len());
        for (idx, step) in rows.iter().enumerate() {
            let (ep, mut url_template) = match (&step.request, step.endpoint_id) {
                (None, None) if step.java_invocation.is_some() => {
                    let java = step.java_invocation.as_ref().expect("上方已判定");
                    let raw =
                        serde_json::to_string(&java.arguments).unwrap_or_else(|_| "[]".into());
                    (
                        Endpoint {
                            id: Uuid::new_v4(),
                            project_id,
                            folder_id: None,
                            name: step.display_name().unwrap_or(&java.method_id).to_string(),
                            method: miyo_core::model::HttpMethod::Java,
                            path: java.method_id.clone(),
                            description: String::new(),
                            status: miyo_core::model::EndpointStatus::Developing,
                            sort_order: idx as i64,
                            request: miyo_core::model::RequestSpec {
                                body: miyo_core::model::BodySpec::Json { raw },
                                timeout_ms: java.timeout_ms,
                                ..miyo_core::model::RequestSpec::default()
                            },
                            created_at,
                            updated_at,
                        },
                        format!("java://{}/{}", java.java_project_id, java.class_name),
                    )
                }
                (Some(inline), _) => {
                    let method = inline
                        .method
                        .parse::<miyo_core::model::HttpMethod>()
                        .map_err(|_| {
                            format!(
                                "{label_prefix}第 {} 步的 HTTP 方法无效（{}）",
                                idx + 1,
                                inline.method
                            )
                        })?;
                    (
                        Endpoint {
                            id: Uuid::new_v4(),
                            project_id,
                            folder_id: None,
                            name: step.display_name().unwrap_or("内联步骤").to_string(),
                            method,
                            path: inline.url.clone(),
                            description: String::new(),
                            status: miyo_core::model::EndpointStatus::Developing,
                            sort_order: idx as i64,
                            request: inline.spec.clone(),
                            created_at,
                            updated_at,
                        },
                        inline.url.clone(),
                    )
                }
                (None, Some(endpoint_id)) => {
                    let ep = repo::get_endpoint(db, endpoint_id)
                        .await
                        .map_err(|e| format!("接口不存在（{endpoint_id}）：{e}"))?;
                    let url = url_template_for(&ep);
                    (ep, url)
                }
                (None, None) => {
                    return Err(format!(
                        "{label_prefix}第 {} 步既无接口引用也无内联请求或 Java 调用定义",
                        idx + 1
                    ))
                }
            };
            if let Some(u) = step
                .overrides
                .as_ref()
                .and_then(|ov| ov.get("url").and_then(|v| v.as_str()))
            {
                url_template = u.to_string();
            }
            out.push(
                miyo_test::materialize::materialize_step(
                    miyo_test::materialize::StepMaterialInput {
                        endpoint: ep,
                        url_template,
                        extract_json: step.extract.clone(),
                        assertions_json: step.assertions.clone(),
                        condition_json: step.condition.clone(),
                        loop_json: step.r#loop.clone(),
                        retry: step.retry.clone(),
                        enabled: step.enabled,
                        wait_ms: step.wait_ms,
                        parallel: step.parallel,
                        overrides_json: step.overrides.clone(),
                        auto_idempotency_key: None,
                        step_label: format!("{label_prefix}第 {} 步", idx + 1),
                        java_invocation: step.java_invocation.clone(),
                    },
                    global_params,
                    vars,
                )
                .map_err(|e| format!("{label_prefix}{e}"))?,
            );
        }
        Ok(out)
    }

    let main = materialize_rows(db, scenario, &scenario.steps, global_params, vars, "").await?;
    let setup =
        materialize_rows(db, scenario, &scenario.setup, global_params, vars, "setup ").await?;
    let teardown = materialize_rows(
        db,
        scenario,
        &scenario.teardown,
        global_params,
        vars,
        "teardown ",
    )
    .await?;
    Ok((main, setup, teardown))
}

/// 接口当前配置 → URL 模板（与前端 / Agent 控制面同构）。
fn url_template_for(ep: &Endpoint) -> String {
    let mut with_vars = ep.path.clone();
    let mut rows: Vec<&miyo_core::model::KeyValue> = ep
        .request
        .path_variables
        .iter()
        .filter(|r| r.enabled && !r.key.trim().is_empty() && !r.value.trim().is_empty())
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.key.chars().count()));
    for row in rows {
        with_vars = with_vars
            .replace(&format!("{{{{{}}}}}", row.key), &row.value)
            .replace(&format!("{{{}}}", row.key), &row.value);
    }
    let is_abs = with_vars.contains("://");
    if is_abs {
        with_vars
    } else if with_vars.starts_with('/') {
        format!("{{{{base_url}}}}{with_vars}")
    } else {
        format!("{{{{base_url}}}}/{with_vars}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::ScenarioFolder;

    fn folder(id: Uuid, parent: Option<Uuid>) -> ScenarioFolder {
        ScenarioFolder {
            id,
            project_id: Uuid::new_v4(),
            parent_id: parent,
            name: "f".into(),
            sort_order: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn folder_scope_includes_descendants() {
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let grandchild = Uuid::new_v4();
        let other = Uuid::new_v4();
        let folders = vec![
            folder(root, None),
            folder(child, Some(root)),
            folder(grandchild, Some(child)),
            folder(other, None),
        ];
        let scope = collect_folder_scope(&folders, root);
        assert_eq!(scope.len(), 3);
        assert!(scope.contains(&root) && scope.contains(&child) && scope.contains(&grandchild));
        assert!(!scope.contains(&other));
    }
}
