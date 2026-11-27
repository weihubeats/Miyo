/**
 * testSuites：测试套件与批量执行状态（集成测试域）。
 *
 * 职责边界：
 * - 套件：项目级列表缓存 + 保存 / 删除 / 复制 / 成员排序 / 启用切换；
 * - 批次：启动参数面板（预设入口）、在途进度（事件驱动）、最终报告、历史列表、
 *   暂停 / 恢复 / 取消 / 失败重试 / 整批重跑。
 *
 * 不在本 store 做任何执行编排：串行 / 并行 / 失败策略 / 重试 / 暂停全部在 Rust
 * 批量引擎（miyo-test::batch + commands/test_suites.rs）完成，前端只展示。
 *
 * 批次之间相互隔离：所有进度事件先按 batch_id 过滤，跨批次事件直接丢弃，
 * 避免上一个批次的迟到事件污染新批次。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import type {
  RunTestBatchArgs,
  TestBatchDetail,
  TestBatchMode,
  TestBatchProgressEvent,
  TestFailurePolicy,
  TestRunBatch,
  TestRunBatchItem,
  TestSuite,
  TestSuiteItem,
  TestSuiteWithItems,
} from '@/types/miyoApi.d'

/** 批量入口预设（三个入口统一收敛到同一张批次表）。 */
export interface BatchRunPreset {
  /** 套件全量执行。 */
  suite_id?: string | null
  /** 手动勾选场景（保持传入顺序）。 */
  scenario_ids?: string[]
  /** 文件夹批量执行（含子目录）。 */
  folder_id?: string | null
  /** 重跑来源批次。 */
  parent_batch_id?: string | null
}

/** 运行配置（面板可改；默认串行 + 失败继续）。 */
export interface BatchRunOptions {
  mode: TestBatchMode
  concurrency: number
  failure_policy: TestFailurePolicy
  retry_count: number
  skip_disabled: boolean
}

export const DEFAULT_BATCH_OPTIONS: BatchRunOptions = {
  mode: 'serial',
  concurrency: 3,
  failure_policy: 'continue',
  retry_count: 0,
  skip_disabled: true,
}

/** 批次终态（不再变化）。 */
const TERMINAL: ReadonlyArray<string> = ['completed', 'failed', 'cancelled', 'interrupted']

export const useTestSuitesStore = defineStore('testSuites', () => {
  const api = useMiyoApi()
  const workspace = useWorkspaceStore()
  const integration = useIntegrationStore()

  // ---------- 套件 ----------
  const suites = ref<TestSuiteWithItems[]>([])
  const suitesLoading = ref(false)
  /** 当前打开的套件（详情层级；null = 未选中，路由 query 同步用）。 */
  const activeSuiteId = ref<string | null>(null)

  function setActiveSuite(id: string | null): void {
    activeSuiteId.value = id
  }

  // ---------- 批次 ----------
  const batches = ref<TestRunBatch[]>([])
  const activeBatchId = ref<string | null>(null)
  const activeBatch = ref<TestBatchDetail | null>(null)
  const liveProgress = ref<TestBatchProgressEvent | null>(null)
  const liveItems = ref<Record<string, TestRunBatchItem>>({})
  /** 批次面板（进度 / 报告）是否展示。 */
  const panelOpen = ref(false)
  /** 启动 / 恢复请求在途（禁用按钮，避免重复提交）。 */
  const starting = ref(false)
  /** 运行配置弹窗。 */
  const runDialogOpen = ref(false)
  const runPreset = ref<BatchRunPreset | null>(null)
  const runOptions = ref<BatchRunOptions>({ ...DEFAULT_BATCH_OPTIONS })

  const hasProject = computed(() => !!integration.projectId)

  // ---------- 套件 ----------

  /** 拉取当前项目全部套件（含成员）。 */
  async function loadSuites(): Promise<void> {
    const projectId = integration.projectId
    if (!projectId) {
      suites.value = []
      return
    }
    suitesLoading.value = true
    try {
      suites.value = await api.listTestSuites(projectId)
    } finally {
      suitesLoading.value = false
    }
  }

  /** 保存套件（覆盖写）；返回后端归一化后的结果。 */
  async function saveSuite(
    suite: TestSuite,
    items: TestSuiteItem[],
  ): Promise<TestSuiteWithItems> {
    const saved = await api.saveTestSuite(suite, items)
    const idx = suites.value.findIndex((s) => s.id === saved.id)
    if (idx >= 0) suites.value[idx] = saved
    else suites.value.push(saved)
    suites.value.sort((a, b) => a.name.localeCompare(b.name))
    return saved
  }

  async function removeSuite(id: string): Promise<void> {
    await api.deleteTestSuite(id)
    suites.value = suites.value.filter((s) => s.id !== id)
  }

  async function duplicateSuite(id: string): Promise<TestSuiteWithItems | null> {
    const copy = await api.duplicateTestSuite(id)
    suites.value.push(copy)
    suites.value.sort((a, b) => a.name.localeCompare(b.name))
    return copy
  }

  /** 套件内重排（本地先应用，后端失败时由调用方刷新还原）。 */
  async function reorderItems(suiteId: string, ids: string[]): Promise<void> {
    await api.reorderTestSuiteItems(suiteId, ids)
    const suite = suites.value.find((s) => s.id === suiteId)
    if (!suite) return
    const byId = new Map(suite.items.map((i) => [i.id, i]))
    suite.items = ids
      .map((id, index) => {
        const item = byId.get(id)
        return item ? { ...item, sort_order: index } : null
      })
      .filter((i): i is TestSuiteItem => !!i)
  }

  async function toggleItem(suiteId: string, itemId: string, enabled: boolean): Promise<void> {
    await api.setTestSuiteItemEnabled(itemId, enabled)
    const suite = suites.value.find((s) => s.id === suiteId)
    const item = suite?.items.find((i) => i.id === itemId)
    if (item) item.enabled = enabled
  }

  // ---------- 批次 ----------

  /** 项目最近批次（新→旧）。 */
  async function refreshBatches(): Promise<void> {
    const projectId = integration.projectId
    if (!projectId) {
      batches.value = []
      return
    }
    batches.value = await api.listTestBatches(projectId, 50)
  }

  /** 打开运行配置（入口：套件全量 / 勾选场景 / 文件夹）。 */
  function openRunDialog(preset: BatchRunPreset): void {
    runPreset.value = preset
    runDialogOpen.value = true
  }

  function closeRunDialog(): void {
    runDialogOpen.value = false
  }

  /** 关闭面板：保留已加载的批次快照，便于从历史再次打开。 */
  function closePanel(): void {
    panelOpen.value = false
  }

  /** 从批次详情返回执行历史：留在面板内，清掉当前批次上下文。 */
  function showHistory(): void {
    resetLive()
    panelOpen.value = true
  }

  function resetLive(): void {
    activeBatchId.value = null
    activeBatch.value = null
    liveProgress.value = null
    liveItems.value = {}
  }

  function mergeItems(items: TestRunBatchItem[]): void {
    const next = { ...liveItems.value }
    for (const item of items) next[item.id] = item
    liveItems.value = next
  }

  /** 加载批次详情（历史查看 / 进度事件带出 batch id 时补全明细）。 */
  async function loadBatch(id: string): Promise<TestBatchDetail | null> {
    const detail = await api.getTestBatch(id)
    if (!detail) return null
    // 迟到结果不覆盖更新的批次（面板已切到别的批次）。
    if (activeBatchId.value && activeBatchId.value !== id) return null
    activeBatchId.value = detail.id
    activeBatch.value = detail
    mergeItems(detail.items)
    return detail
  }

  /** 打开批次面板并刷新历史（无在途批次时也可回看历史）。 */
  async function openPanel(): Promise<void> {
    panelOpen.value = true
    await refreshBatches().catch(() => {})
  }

  /** 打开历史批次报告。 */
  async function openBatch(id: string): Promise<void> {
    activeBatchId.value = id
    panelOpen.value = true
    const detail = await loadBatch(id)
    if (!detail) {
      // 批次不存在（已删除 / 深链失效）：回到执行历史，不留在空详情。
      resetLive()
      await refreshBatches().catch(() => {})
    }
  }

  /** 启动批次（不阻塞调用方：进度由事件驱动，结束时回填完整报告）。 */
  async function startBatch(
    preset: BatchRunPreset,
    options?: Partial<BatchRunOptions>,
  ): Promise<void> {
    const projectId = integration.projectId
    if (!projectId) return
    runOptions.value = { ...runOptions.value, ...options }
    const opts = runOptions.value
    resetLive()
    panelOpen.value = true
    starting.value = true
    const args: RunTestBatchArgs = {
      project_id: projectId,
      suite_id: preset.suite_id ?? null,
      scenario_ids: preset.scenario_ids ?? [],
      folder_id: preset.folder_id ?? null,
      environment_id: workspace.activeEnvId,
      mode: opts.mode,
      concurrency: opts.mode === 'parallel' ? opts.concurrency : 1,
      failure_policy: opts.failure_policy,
      retry_count: opts.retry_count,
      skip_disabled: opts.skip_disabled,
      parent_batch_id: preset.parent_batch_id ?? null,
    }
    try {
      const detail = await api.runTestBatch(args)
      activeBatchId.value = detail.id
      activeBatch.value = detail
      mergeItems(detail.items)
    } catch (err) {
      // 启动即失败（无可用场景 / 参数错误）：不留下空白面板。
      if (!activeBatchId.value) panelOpen.value = false
      throw err
    } finally {
      starting.value = false
      await refreshBatches().catch(() => {})
    }
  }

  /** 恢复执行（暂停 / 中断 / 未开始的批次）。 */
  async function resumeBatch(id?: string): Promise<void> {
    const batchId = id ?? activeBatchId.value
    if (!batchId) return
    panelOpen.value = true
    starting.value = true
    try {
      const detail = await api.resumeTestBatch(batchId)
      activeBatchId.value = detail.id
      activeBatch.value = detail
      mergeItems(detail.items)
    } finally {
      starting.value = false
      await refreshBatches().catch(() => {})
    }
  }

  /** 暂停：只停止调度尚未开始的场景（在途场景跑完当前波次）。 */
  async function pauseBatch(): Promise<boolean> {
    const batchId = activeBatchId.value
    if (!batchId) return false
    return api.pauseTestBatch(batchId)
  }

  /** 取消整批（在途协作式取消，未开始项标记 cancelled）。 */
  async function cancelBatch(): Promise<boolean> {
    const batchId = activeBatchId.value
    if (!batchId) return false
    const ok = await api.cancelTestBatch(batchId)
    await loadBatch(batchId).catch(() => {})
    await refreshBatches().catch(() => {})
    return ok
  }

  async function removeBatch(id: string): Promise<void> {
    await api.deleteTestBatch(id)
    batches.value = batches.value.filter((b) => b.id !== id)
    if (activeBatchId.value === id) {
      resetLive()
      panelOpen.value = false
    }
  }

  /**
   * 失败场景重试：新建批次（不覆盖原记录），只跑失败项，
   * 沿用原批次的模式 / 并发 / 失败策略 / 环境。
   */
  async function retryFailed(): Promise<void> {
    const batch = activeBatch.value
    if (!batch) return
    const failed = activeItems.value.filter((i) => i.status === 'failed')
    if (!failed.length) return
    await startBatch(
      {
        scenario_ids: failed.map((i) => i.scenario_id),
        parent_batch_id: batch.id,
      },
      {
        mode: batch.mode,
        concurrency: batch.concurrency,
        failure_policy: batch.failure_policy,
        retry_count: batch.retry_count,
        skip_disabled: batch.skip_disabled,
      },
    )
  }

  /**
   * 整批重跑：新建批次（parent_batch_id 指向原批次）。
   * 套件批次优先复用套件（成员可能已更新）；手动 / 文件夹批次复用场景清单。
   */
  async function rerunBatch(): Promise<void> {
    const batch = activeBatch.value
    if (!batch) return
    const preset: BatchRunPreset = batch.suite_id
      ? { suite_id: batch.suite_id, parent_batch_id: batch.id }
      : {
          scenario_ids: activeItems.value.map((i) => i.scenario_id),
          parent_batch_id: batch.id,
        }
    await startBatch(preset, {
      mode: batch.mode,
      concurrency: batch.concurrency,
      failure_policy: batch.failure_policy,
      retry_count: batch.retry_count,
      skip_disabled: batch.skip_disabled,
    })
  }

  // ---------- 事件 ----------

  /**
   * 应用进度事件：先按 batch_id 过滤（跨批次事件丢弃），再更新计数与明细；
   * 终态事件触发一次详情回读与历史刷新（事件只是增量，报告以 DB 为准）。
   */
  function applyProgress(event: TestBatchProgressEvent): void {
    if (!activeBatchId.value) {
      activeBatchId.value = event.batch_id
      panelOpen.value = true
      void loadBatch(event.batch_id).catch(() => {})
    }
    if (event.batch_id !== activeBatchId.value) return
    liveProgress.value = event
    if (event.item) mergeItems([event.item])
    if (activeBatch.value?.id === event.batch_id && !TERMINAL.includes(event.status)) {
      // 让批次头部的计数与进度条在详情回读前也保持流动。
      activeBatch.value = {
        ...activeBatch.value,
        status: activeBatch.value.status,
        total_count: event.total,
        passed_count: event.passed,
        failed_count: event.failed,
        skipped_count: event.skipped,
        cancelled_count: event.cancelled,
        running_count: event.running,
        pending_count: Math.max(
          0,
          event.total - event.passed - event.failed - event.skipped - event.cancelled,
        ),
      }
    }
    if (TERMINAL.includes(event.status)) {
      void loadBatch(event.batch_id).catch(() => {})
      void refreshBatches().catch(() => {})
    }
  }

  // ---------- 派生 ----------

  /** 当前批次明细（事件增量覆盖 DB 快照，按套件顺序展示）。 */
  const activeItems = computed<TestRunBatchItem[]>(() => {
    const merged = new Map<string, TestRunBatchItem>()
    for (const item of activeBatch.value?.items ?? []) merged.set(item.id, item)
    for (const item of Object.values(liveItems.value)) merged.set(item.id, item)
    return [...merged.values()].sort((a, b) => a.sort_order - b.sort_order)
  })

  /** 进度计数（事件优先，其次批次快照）。 */
  const progress = computed(() => {
    const event = liveProgress.value
    const batch = activeBatch.value
    if (event && batch && event.batch_id === batch.id) {
      return {
        total: event.total,
        done: event.done,
        passed: event.passed,
        failed: event.failed,
        skipped: event.skipped,
        cancelled: event.cancelled,
        running: event.running,
      }
    }
    const total = batch?.total_count ?? 0
    const passed = batch?.passed_count ?? 0
    const failed = batch?.failed_count ?? 0
    const skipped = batch?.skipped_count ?? 0
    const cancelled = batch?.cancelled_count ?? 0
    return {
      total,
      done: passed + failed + skipped + cancelled,
      passed,
      failed,
      skipped,
      cancelled,
      running: batch?.running_count ?? 0,
    }
  })

  const percent = computed(() => {
    const p = progress.value
    return p.total ? Math.round((p.done * 100) / p.total) : 0
  })

  /** 是否在途（可暂停 / 取消）。 */
  const isRunning = computed(() => {
    if (starting.value) return true
    const status = activeBatch.value?.status
    return status === 'running' || status === 'pending'
  })

  const isPaused = computed(() => activeBatch.value?.status === 'paused')

  /** 是否可恢复（暂停 / 中断 / 未开始）。 */
  const canResume = computed(() => {
    const status = activeBatch.value?.status
    return status === 'paused' || status === 'interrupted' || status === 'pending'
  })

  const failedCount = computed(() => progress.value.failed)

  const hasReport = computed(() => !!activeBatch.value && !isRunning.value)

  return {
    // 套件
    suites,
    suitesLoading,
    activeSuiteId,
    setActiveSuite,
    loadSuites,
    saveSuite,
    removeSuite,
    duplicateSuite,
    reorderItems,
    toggleItem,
    // 批次
    batches,
    activeBatchId,
    activeBatch,
    activeItems,
    liveProgress,
    panelOpen,
    starting,
    runDialogOpen,
    runPreset,
    runOptions,
    hasProject,
    refreshBatches,
    openRunDialog,
    closeRunDialog,
    closePanel,
    openPanel,
    showHistory,
    loadBatch,
    openBatch,
    startBatch,
    resumeBatch,
    pauseBatch,
    cancelBatch,
    removeBatch,
    retryFailed,
    rerunBatch,
    applyProgress,
    // 派生
    progress,
    percent,
    isRunning,
    isPaused,
    canResume,
    failedCount,
    hasReport,
  }
})