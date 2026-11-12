/**
 * useScenarioRun：场景运行引擎（集成测试工作流复用）。
 *
 * 职责（从 WorkflowEditor 抽出的运行状态机）：
 * - 运行状态：running/runId/进度/结果/步骤位次/runtime 快照（历史回看无快照）；
 * - 运行：未保存先落库（避免「跑的不是看到的」）→ 宿主 buildRunSteps 物化
 *   （内联请求 / 接口引用统一成运行入参，URL 保留 {{}} 模板）→ 调 runScenario；
 * - 预检警告：后端返回 warnings（未定义变量）→ 非阻断 toast；
 * - 从步骤执行 / 单步执行：以原运行的 runtime 快照重建链路上下文（无快照
 *   时仅环境变量并提示）；
 * - 重跑失败：以原运行 runtime 快照重建链路上下文，失败步按原位次回填合并；
 * - 运行历史：列表加载 / 最近一次恢复 / 详情回看 / 按历史配置重跑；
 * - 结果面板派生：步骤 tone（六态）、汇总、终态、进度百分比、响应行展开、全文加载。
 *
 * 留在宿主的视图层：Sortable 拖拽、locateStep、historyMenu DOM 锚点经 options 传入。
 */
import { computed, ref, type ComputedRef } from 'vue'
import type { useWorkspaceStore } from '../stores/workspace'
import type { useMiyoApi } from '../composables/useMiyoApi'
import type { useToast } from '../composables/useToast'
import type {
  Endpoint,
  ScenarioRunDetail,
  ScenarioRunMeta,
  ScenarioRunResult,
  ScenarioRunSummary,
  ScenarioStepResult,
  StepCondition,
  StepLoop,
  StepRetry,
} from '../types/miyoApi'
import { formatDateTime } from '../utils/dateTime'
import type { MenuItem } from '../components/ui/Menu.vue'

/** 菜单打开能力（宿主持有 Menu 实例；组合式不依赖组件实例类型）。 */
export type MenuOpener = (anchor: HTMLElement, items: MenuItem[]) => void

/** 生命周期运行入参元素（与 runScenario.steps 元素形状一致）。 */
export type ApiLifecycleStep = {
  endpoint: Endpoint
  url: string
  enabled: boolean
  extract: unknown
  assertions: unknown
  retry?: StepRetry | null
}

/** 步骤执行状态 tone（六态 + 运行中/等待两个过程态）。 */
export type StepTone = 'ok' | 'fail' | 'skip' | 'run' | 'wait' | 'cancel' | 'blocked'

/** 结果 → 徽章 tone：step_status 单源（六态），旧结果（无字段）回退
 *  cancelled / ok 推导。供运行面板与比较视图共用。 */
export function toneOf(r: ScenarioStepResult): StepTone {
  const s = r.step_status
  if (s === 'blocked') return 'blocked'
  if (s === 'skipped') return 'skip'
  if (r.cancelled) return 'cancel'
  if (s === 'success') return 'ok'
  if (s === 'failed') return 'fail'
  return r.ok ? 'ok' : 'fail'
}

/** 运行入参步骤（宿主物化后的形状：内联请求 / 接口引用统一）。 */
export interface BuiltRunStep {
  /** 编辑态步骤位次（结果按 step_index 映射回步骤卡）。 */
  index: number
  endpoint: Endpoint
  /** 渲染前 URL 模板（含 {{base_url}} / {{var}} 等未解析占位）。 */
  url: string
  enabled: boolean
  extract: { name: string; path: string }[]
  assertions: { type: string; path: string | null; op: string; expected: unknown }[]
  /** 执行控制：条件 / 循环 / 等待 / 并行（透传给引擎）。 */
  condition?: StepCondition | null
  loop?: StepLoop | null
  wait_ms?: number | null
  parallel?: boolean
  /** 步骤重试策略（显式配置才启用；透传给引擎）。 */
  retry?: StepRetry | null
}

export interface UseScenarioRunOptions {
  store: ReturnType<typeof useWorkspaceStore>
  api: ReturnType<typeof useMiyoApi>
  toast: ReturnType<typeof useToast>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
  /** 当前场景（null = 已切换 / 未加载）。 */
  scenario: ComputedRef<{ id: string; name: string } | null>
  /** props.scenarioId（防异步竞态：恢复历史前校验仍是当前场景）。 */
  scenarioId: string
  /** 宿主物化：编辑态 → 运行入参步骤（调用方保证顺序与启用标记）。 */
  buildRunSteps: () => BuiltRunStep[]
  /** 编辑态是否有未保存修改（运行前先落库）。 */
  dirty: ComputedRef<boolean>
  /** 落库（运行前置）。 */
  save: () => Promise<boolean>
  /** 历史菜单打开回调（Menu.openAt 的绑定；实例归宿主持有）。 */
  openMenuAt: MenuOpener
  /** 生命周期：Setup / Teardown 运行入参（宿主从场景配置物化；缺省空）。
   *  形状与 steps 元素一致（引擎按同一物化路径处理）。 */
  lifecycleSetup?: () => BuiltRunStep[] | unknown[]
  lifecycleTeardown?: () => BuiltRunStep[] | unknown[]
  /** 自动清理开关（宿主读场景 teardown_enabled；缺省 true）。 */
  lifecycleTeardownEnabled?: () => boolean
}

/** 场景进度事件载荷（后端 `miyo:scenario-progress`，每步发两次：
 *  进入本步时带 url、结束本步时带 result）。 */
export interface ScenarioProgressEvent {
  runId: string
  done: number
  total: number
  /** 进入某步时的实际 URL（该步结束时缺省）。 */
  url?: string | null
  /** 该步已结束时的结果（进入时缺省）。 */
  result?: ScenarioRunResult['results'][number] | null
  /** 执行阶段（main 缺省；setup / teardown = 生命周期实时进度，done/total
   *  按该阶段内位次计）。 */
  phase?: string | null
}

export function useScenarioRun(options: UseScenarioRunOptions) {
  const {
    store,
    api,
    toast,
    t,
    scenario,
    scenarioId,
    buildRunSteps,
    dirty,
    save,
    openMenuAt,
    lifecycleSetup,
    lifecycleTeardown,
    lifecycleTeardownEnabled,
  } = options

  /** 生命周期运行入参（宿主未提供时为空——旧宿主 / 测试兼容）。 */
  const lifecycleSetupPayload = () =>
    lifecycleSetup?.() ?? []
  const lifecycleTeardownPayload = () =>
    lifecycleTeardown?.() ?? []

  // ---------- 运行状态 ----------
  const stopOnFailure = ref(true)
  const running = ref(false)
  const runId = ref('')
  const progress = ref({ done: 0, total: 0 })
  const runResult = ref<ScenarioRunResult | null>(null)
  /** 本次运行实际执行的步骤（编辑态索引，按运行顺序）；结果按位对应。 */
  const runStepIndexes = ref<number[]>([])

  // ---------- 逐步实时结果（后端进度事件 `miyo:scenario-progress`） ----------
  /** 运行中按位累积的已完成步结果：后端每步结束即回传，前端立刻可见
   *  （不必等整轮跑完）；运行结束后由 runScenario 的返回整体覆盖。 */
  const liveResults = ref<(ScenarioStepResult | undefined)[]>([])
  /** 运行中「已进入但未结束」的步：位次 → 渲染后实际发出的 URL。
   *  并行批内多步同时在途（Map 语义），串行时至多一项。 */
  const inFlight = ref<Map<number, string>>(new Map())

  /** 接收进度事件（由宿主组件转发 `miyo:scenario-progress`）。 */
  function applyProgressEvent(p: ScenarioProgressEvent): void {
    if (!runId.value || p.runId !== runId.value) return
    // 生命周期实时进度（phase = setup / teardown）：结果按阶段内位次进
    // lifecycleRows，不与主步骤位次混合。
    if (p.phase && p.phase !== 'main') {
      const bucket = p.phase === 'setup' ? 'setup' : 'teardown'
      const rowList = [...lifecycleRows.value[bucket]]
      rowList.length = Math.max(rowList.length, p.done)
      if (p.result) rowList[p.done - 1] = p.result
      lifecycleRows.value = { ...lifecycleRows.value, [bucket]: rowList }
      return
    }
    progress.value = { done: p.done, total: p.total || progress.value.total }
    if (p.result) {
      const stepIdx = p.result.step_index ?? p.done - 1
      const pos = runStepIndexes.value.indexOf(stepIdx)
      const at = pos === -1 ? Math.max(0, p.done - 1) : pos
      const next = [...liveResults.value]
      next[at] = p.result
      liveResults.value = next
      if (inFlight.value.has(at)) {
        const m = new Map(inFlight.value)
        m.delete(at)
        inFlight.value = m
      }
      return
    }
    const m = new Map(inFlight.value)
    m.set(p.done, p.url ?? '')
    inFlight.value = m
  }

  /** 运行中的结果行：已完成步 + 在途步占位（步骤一进入就有行，不必等整轮）。 */
  const liveRows = computed<
    { result: ScenarioStepResult | null; pos: number; url: string | null }[]
  >(() => {
    if (!running.value) return []
    const rows: {
      result: ScenarioStepResult | null
      pos: number
      url: string | null
    }[] = []
    const total = runStepIndexes.value.length
    for (let pos = 0; pos < total; pos++) {
      const r = liveResults.value[pos]
      if (r) {
        rows.push({ result: r, pos, url: r.url ?? null })
        continue
      }
      // 尚未结束：在途步占位「执行中」；并行批内的后续步也可能在途，
      // 未到的保持徽章「待执行」。
      if (inFlight.value.has(pos)) {
        rows.push({ result: null, pos, url: inFlight.value.get(pos) || null })
      } else if (!inFlight.value.size) {
        // 无任何在途（串行等待下一步进入）：保持旧行为，到此为止。
        break
      }
    }
    return rows
  })

  /** 某步在「运行中」是否还没有任何结果（面板显示等待提示）。 */
  const livePending = computed(() => liveRows.value.some((r) => !r.result))

  /** 执行策略（失败即停的可读化表达，底层仍是 stop_on_failure 布尔）。 */
  const STRATEGY_OPTIONS = computed(() => [
    { value: 'stop', label: t('integration.strategyStop') },
    { value: 'continue', label: t('integration.strategyContinue') },
  ])
  const execStrategy = computed({
    get: () => (stopOnFailure.value ? 'stop' : 'continue'),
    set: (v: string | null) => {
      stopOnFailure.value = v !== 'continue'
    },
  })

  // ---------- 步骤执行状态（卡片徽章 + 结果面板共用；六态） ----------
  const resultByStepIndex = computed(() => {
    const m = new Map<number, ScenarioStepResult>()
    const results = runResult.value?.results ?? []
    runStepIndexes.value.forEach((stepIdx, i) => {
      const r = results[i]
      if (r) m.set(stepIdx, r)
    })
    return m
  })

  /** 结果按位对应 runStepIndexes；运行中已完成步按真实结果着色，
   * 在途步「执行中」，未到达的「待执行」。 */
  function stepTone(i: number): StepTone | null {
    const pos = runStepIndexes.value.indexOf(i)
    if (pos === -1) return null
    if (running.value) {
      const live = liveResults.value[pos]
      if (live) return toneOf(live)
      return inFlight.value.has(pos) ? 'run' : 'wait'
    }
    if (!runResult.value) return null
    const r = resultByStepIndex.value.get(i)
    if (r) return toneOf(r)
    return runResult.value.stopped_on_failure || runResult.value.cancelled ? 'skip' : 'wait'
  }

  const STEP_TONE_TEXT: Record<StepTone, string> = {
    ok: 'integration.statusSuccess',
    fail: 'integration.statusFailed',
    skip: 'integration.statusSkipped',
    run: 'integration.statusRunning',
    wait: 'integration.statusPending',
    cancel: 'scenario.stepStatusCancelled',
    blocked: 'integration.statusBlocked',
  }
  const STEP_TONE_ICON: Record<StepTone, 'check' | 'x' | 'chevron-right' | 'dot' | 'clock' | 'stop'> = {
    ok: 'check',
    fail: 'x',
    skip: 'chevron-right',
    run: 'dot',
    wait: 'clock',
    cancel: 'stop',
    blocked: 'x',
  }

  // ---------- 执行结果面板 ----------
  const reportOpen = ref(true)

  /** 当前展示结果是否来自运行历史（重开恢复 / 历史回看）；实时运行为 false。 */
  const runIsHistory = ref(false)
  /** 当前展示结果的完成时刻（RFC3339）。 */
  const currentRunAt = ref<string | null>(null)
  /** 当前展示结果的开始时刻（meta.startedAt；旧历史缺失为 null）。 */
  const currentRunStartedAt = ref<string | null>(null)
  /** 当前展示结果的运行元信息（历史重跑的配置真源；实时运行为 null）。 */
  const runMeta = ref<ScenarioRunMeta | null>(null)
  /** 每步执行前的 runtime 快照（与 results 同位序；重跑失败步骤 / 从本步
   * 执行的初始变量来源，仅实时运行有，历史回看无）。 */
  const varsSnapshots = ref<Record<string, string>[]>([])

  /** 生命周期结果行（历史回看：setup / teardown；实时运行随 runResult 分离）。 */
  const lifecycleRows = ref<{ setup: ScenarioStepResult[]; teardown: ScenarioStepResult[] }>({
    setup: [],
    teardown: [],
  })

  /** 运行历史列表（新→旧；场景切换后刷新）。 */
  const runHistory = ref<ScenarioRunSummary[]>([])

  /** 历史加载序号：并发请求只有最新一次可写入（挂载 / 每轮运行结束 /
   *  重跑失败步 / 历史缺失刷新都会触发，乱序返回时丢弃过期结果）。 */
  let historySeq = 0

  async function loadHistory(): Promise<void> {
    if (!scenario.value) {
      runHistory.value = []
      return
    }
    const seq = ++historySeq
    try {
      const runs = await api.listScenarioRuns(scenarioId)
      if (seq === historySeq) runHistory.value = runs
    } catch {
      // 失败保留现值：首次加载本就是空（行为不变），后续失败不清掉已有列表，
      // 也不覆盖乱序间新请求写入的数据。
    }
  }

  // ---------- 历史恢复（竞态防护） ----------
  /**
   * 恢复请求序号 + 卸载标记：历史详情是异步 IPC，快速连点 / 挂载期两条
   * 恢复路径（直达 runId + 最近一次）并发时，慢响应不得覆盖新选择；
   * 组件卸载后回调不得再写状态。restoreLoading 只反映「最新一次」请求。
   */
  let restoreSeq = 0
  let disposed = false
  const restoreLoading = ref(false)

  /** 宿主在 onBeforeUnmount 调用：此后所有在途恢复回调全部失效。 */
  function dispose(): void {
    disposed = true
    restoreSeq++
    restoreLoading.value = false
  }

  /** 恢复结果：applied = 已应用；stale = 已有更新的选择 / 已卸载 / 已开跑；
   *  missing = 运行历史不存在（已被清理）。 */
  type RestoreOutcome =
    | { status: 'applied'; detail: ScenarioRunDetail }
    | { status: 'stale' }
    | { status: 'missing' }

  /** 单一恢复入口（直达 runId / 历史菜单 / 最近一次共用）：
   *  序号仲裁——只有最新一次恢复能落状态。 */
  async function restoreRun(runIdToRestore: string): Promise<RestoreOutcome> {
    if (running.value || disposed || !runIdToRestore) return { status: 'stale' }
    const seq = ++restoreSeq
    restoreLoading.value = true
    try {
      const detail = await api.getScenarioRun(runIdToRestore)
      // 运行中不接受回填（runId 被历史覆盖会吞实时进度事件）；卸载 /
      // 更新选择已发起 → 本次结果丢弃。
      if (disposed || running.value || seq !== restoreSeq) return { status: 'stale' }
      if (!detail) return { status: 'missing' }
      applyHistoryDetail(detail)
      return { status: 'applied', detail }
    } catch {
      // 请求失败视为缺失（历史可能已被清理）；不影响后续恢复。
      return { status: 'missing' }
    } finally {
      if (seq === restoreSeq && !disposed) restoreLoading.value = false
    }
  }

  /** 重开场景恢复最近一次运行（历史能力；无历史时保持空态）。 */
  async function restoreLatestRun(): Promise<void> {
    const latest = runHistory.value[0]
    if (!latest || disposed || running.value) return
    // 静默恢复：失败 / 过期都不打扰首次编辑。
    await restoreRun(latest.id)
  }

  function applyHistoryDetail(detail: ScenarioRunDetail): void {
    // 运行中不接受历史回填：runId 被历史覆盖会让实时进度事件全部被过滤
    // （面板「卡死」直到运行结束）。运行中的历史选择在 onHistorySelect 拦截，
    // 此处兜底（pendingRunId 直达 / 恢复路径不会在运行中触发）。
    if (running.value || disposed) return
    // 生命周期行（setup / teardown）不映射步骤卡：主步骤按 phase 缺省过滤；
    // 生命周期行单独暴露供结果面板分组展示。
    lifecycleRows.value = {
      setup: detail.results.filter((r) => r.phase === 'setup'),
      teardown: detail.results.filter((r) => r.phase === 'teardown'),
    }
    runResult.value = {
      results: detail.results.filter((r) => !r.phase),
      cancelled: detail.cancelled,
      stopped_on_failure: detail.stopped_on_failure,
      variables: detail.variables,
    }
    // 结果自带链路位次（step_index），按位映射到步骤卡——**只映射主步骤**
    //（与 runResult.results 同源过滤；phase 行位次独立，不进步骤卡映射）。
    const mainResults = detail.results.filter((r) => !r.phase)
    runStepIndexes.value = mainResults.map((r) => r.step_index ?? -1)
    runId.value = detail.id
    progress.value = { done: mainResults.length, total: detail.total_steps }
    currentRunAt.value = detail.finished_at
    currentRunStartedAt.value = detail.meta?.startedAt ?? null
    runMeta.value = detail.meta ?? null
    runIsHistory.value = true
    reportOpen.value = true
    respOpenRows.value = new Set()
    // 历史恢复：meta.varsSnapshots（v2 落库，敏感值为占位符）→ 重跑失败
    // 步骤可用；旧行缺省为空（不提供假按钮）。
    varsSnapshots.value =
      ((detail.meta?.varsSnapshots as Record<string, string>[] | undefined) ?? []).map(
        (m) => ({ ...m }),
      )
  }

  function openHistoryMenu(e: MouseEvent): void {
    const items: MenuItem[] = runHistory.value.length
      ? runHistory.value.slice(0, 20).map((r) => {
          const label = [
            formatDateTime(r.finished_at),
            t('scenario.summaryOk', { n: r.ok_count }),
            r.fail_count > 0 ? t('scenario.summaryFail', { n: r.fail_count }) : null,
            r.skipped_count > 0 ? t('scenario.summarySkip', { n: r.skipped_count }) : null,
          ]
            .filter(Boolean)
            .join(' · ')
          return {
            key: r.id,
            label,
            icon: r.cancelled ? ('stop' as const) : r.fail_count > 0 ? ('x' as const) : ('check' as const),
          }
        })
      : [{ key: 'empty', label: t('integration.historyEmpty'), disabled: true }]
    // 报告导出（HTML / JUnit XML）：由宿主经 onHistorySelect 分发处理。
    if (runHistory.value.length) {
      items.push({ key: 'export-report-html', label: t('integration.reportHtml'), icon: 'download' })
      items.push({ key: 'export-report-junit', label: t('integration.reportJunit'), icon: 'download' })
    }
    openMenuAt(e.currentTarget as HTMLElement, items)
  }

  /**
   * 导出测试报告（HTML / JUnit XML）：纳入全部已加载的运行历史，
   * 内容自包含（可直接归档 / CI 消费）；落盘由宿主回调完成。
   */
  function exportReport(
    writeTextFile: (path: string, contents: string) => Promise<void>,
  ): (format: 'html' | 'junit_xml') => Promise<void> {
    return async (format) => {
      if (!scenario.value || !runHistory.value.length) return
      try {
        const { content, suggestedName } = await api.exportScenarioReport({
          scenarioId,
          scenarioName: scenario.value.name,
          runIds: runHistory.value.map((r) => r.id),
          format,
        })
        await writeTextFile(suggestedName, content)
        toast.success(t('integration.reportExported'), { message: suggestedName })
      } catch (err) {
        toast.error(t('integration.reportExportFail'), {
          message: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  async function onHistorySelect(
    item: MenuItem,
    hooks?: { writeTextFile: (path: string, contents: string) => Promise<void> },
  ): Promise<void> {
    // 报告导出项（历史菜单尾部分组）。
    if (item.key === 'export-report-html' || item.key === 'export-report-junit') {
      if (!hooks?.writeTextFile) return
      await exportReport(hooks.writeTextFile)(
        item.key === 'export-report-html' ? 'html' : 'junit_xml',
      )
      return
    }
    if (running.value) {
      toast.info(t('integration.historyLockedWhileRunning'))
      return
    }
    const outcome = await restoreRun(item.key)
    if (outcome.status === 'missing') {
      toast.warning(t('integration.historyMissing'))
      await loadHistory()
      return
    }
    if (outcome.status === 'stale') {
      // 已有更新的选择 / 已开跑 / 已卸载：迟到响应静默丢弃（不覆盖新状态）。
      return
    }
  }

  // ---------- 运行 ----------
  function resetRunState(built: BuiltRunStep[]): void {
    lifecycleRows.value = { setup: [], teardown: [] }
    runResult.value = null
    runStepIndexes.value = built.map((b) => b.index)
    respOpenRows.value = new Set()
    runIsHistory.value = false
    currentRunAt.value = null
    currentRunStartedAt.value = null
    runMeta.value = null
    varsSnapshots.value = []
    liveResults.value = []
    inFlight.value = new Map()
    progress.value = { done: 0, total: built.length }
    reportOpen.value = true
    running.value = true
    runId.value = crypto.randomUUID()
  }

  /** 运行前置：未保存先落库 + 物化步骤（空启用步骤时 toast 并返回 null）。 */
  async function prepareRun(): Promise<BuiltRunStep[] | null> {
    if (!scenario.value || running.value) return null
    if (dirty.value && !(await save())) return null
    const built = buildRunSteps()
    if (!built.length) {
      toast.warning(t('scenario.noEnabledSteps'))
      return null
    }
    return built
  }

  /** 运行后置：接收结果 / 预检警告 / 历史刷新。 */
  function finishRun(result: ScenarioRunResult, built: BuiltRunStep[]): void {
    runResult.value = result
    varsSnapshots.value = result.vars_snapshots ?? []
    currentRunAt.value = new Date().toISOString()
    if (result.warnings?.length) {
      toast.warning(t('integration.undefinedVarsWarning', { vars: result.warnings.join('；') }), {
        duration: 8000,
      })
    }
    void loadHistory()
    void built
  }

  function runErrorToast(err: unknown): void {
    toast.error(t('integration.runStartFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }

  async function run(): Promise<void> {
    const built = await prepareRun()
    if (!built) return
    resetRunState(built)
    try {
      const result = await api.runScenario({
        steps: built,
        environmentId: store.activeEnvId,
        stopOnFailure: stopOnFailure.value,
        runId: runId.value,
        scenarioId,
        setup: lifecycleSetupPayload() as ApiLifecycleStep[],
        teardown: lifecycleTeardownPayload() as ApiLifecycleStep[],
        teardownEnabled: lifecycleTeardownEnabled?.() ?? true,
      })
      finishRun(result, built)
    } catch (err) {
      // 抛错时把已跑完的步骤留住（实时结果不清空），便于排查是哪一步出的问题。
      if (liveResults.value.some(Boolean)) {
        runResult.value = {
          results: liveResults.value.filter((r): r is ScenarioStepResult => !!r),
          cancelled: false,
          stopped_on_failure: false,
          variables: {},
        }
      }
      runErrorToast(err)
    } finally {
      running.value = false
      inFlight.value = new Map()
    }
  }

  async function stop(): Promise<void> {
    if (!runId.value) return
    try {
      await api.cancelScenarioRun(runId.value)
    } catch {
      // 已结束的运行取消会失败，忽略即可
    }
  }

  // ---------- 从指定步骤执行 / 单步执行 ----------
  /**
   * 从编辑态位次 `fromIdx` 起（含）执行到末尾；单步执行 = 只跑该位次。
   * 初始变量：原运行在该步执行前的 runtime 快照（重建链路上下文）；
   * 无快照（从未整跑过 / 历史回看）时用调用方手填的 `varsOverride`，
   * 仍无则由宿主拦截——绝不隐式传 undefined 让后端拿到空变量。
   */
  async function runFrom(fromIdx: number, onlyThis: boolean, varsOverride?: Record<string, string>): Promise<void> {
    const built = await prepareRun()
    if (!built) return
    const all = buildRunSteps()
    const start = all.findIndex((b) => b.index === fromIdx)
    if (start === -1) return
    const subset = onlyThis ? [all[start]] : all.slice(start)
    if (!subset.length) return

    const snapshot = varsSnapshots.value[start]
    const initialVars = snapshot ? { ...snapshot } : varsOverride && Object.keys(varsOverride).length ? { ...varsOverride } : undefined
    if (!snapshot && !initialVars) {
      toast.info(t('integration.runFromStepNoSnapshot'))
    }

    resetRunState(subset)
    try {
      const result = await api.runScenario({
        steps: subset,
        environmentId: store.activeEnvId,
        stopOnFailure: stopOnFailure.value,
        runId: runId.value,
        scenarioId,
        initialVars,
      })
      finishRun(result, subset)
    } catch (err) {
      if (liveResults.value.some(Boolean)) {
        runResult.value = {
          results: liveResults.value.filter((r): r is ScenarioStepResult => !!r),
          cancelled: false,
          stopped_on_failure: false,
          variables: {},
        }
      }
      runErrorToast(err)
    } finally {
      running.value = false
      inFlight.value = new Map()
    }
  }

  // ---------- 重跑失败步骤 ----------
  // 仅实时运行结果可重跑（需要 runtime 快照重建链路上下文；历史无快照不提供入口）。
  // 语义：失败步骤按原顺序串行重跑；首个重跑步以当时的 runtime 为初始变量，
  // 其后每个重跑步先合并原运行中「位于两者之间的成功步骤」的提取增量——
  // 即「若这些步骤依次重跑」的链路状态；失败步骤自身不并入旧提取值（重跑会重新提取）。

  /** 可重跑的结果位次（失败 / Blocked 且非取消、且对应步骤仍能按当前编辑态构建）。 */
  function rerunPositions(): number[] {
    const res = runResult.value
    if (!res) return []
    const out: number[] = []
    res.results.forEach((r, i) => {
      const status = r.step_status
      const rerunnable = status === 'failed' || status === 'blocked' || (!status && !r.ok)
      if (!rerunnable || r.cancelled) return
      const stepIdx = runStepIndexes.value[i]
      if (stepIdx == null || stepIdx < 0) return
      out.push(i)
    })
    return out
  }

  /** 是否存在可重跑的失败步骤（实时结果 + 有快照 + 步骤仍可构建）。 */
  const rerunAvailable = computed(() => {
    if (running.value || !runResult.value || !varsSnapshots.value.length) return false
    // 历史回看：meta.varsSnapshots（v2 落库，敏感值已脱敏）提供恢复上下文。
    if (runIsHistory.value) return true
    return rerunPositions().length > 0
  })

  async function rerunFailed(): Promise<void> {
    if (!scenario.value || running.value) return
    const res = runResult.value
    if (!res) return
    const positions = rerunPositions()
    if (!positions.length || !varsSnapshots.value.length) {
      toast.warning(t('scenario.rerunUnavailable'))
      return
    }

    // 初始变量：首个重跑步执行前的 runtime + 原运行中位于重跑步之间的成功步提取增量。
    const initial: Record<string, string> = { ...(varsSnapshots.value[positions[0]] ?? {}) }
    for (let k = 1; k < positions.length; k++) {
      for (let j = positions[k - 1] + 1; j < positions[k]; j++) {
        const r = res.results[j]
        if (r?.ok && r.extracted) Object.assign(initial, r.extracted)
      }
    }

    // 子集步骤按当前编辑态重建（修正后的配置即参跑）。
    const built: BuiltRunStep[] = []
    const builtPositions: number[] = []
    const all = buildRunSteps()
    for (const pos of positions) {
      const stepIdx = runStepIndexes.value[pos]
      const one = all.find((b) => b.index === stepIdx)
      if (!one) continue
      built.push(one)
      builtPositions.push(pos)
    }
    if (!built.length) {
      toast.warning(t('scenario.rerunUnavailable'))
      return
    }

    runIsHistory.value = false
    currentRunAt.value = null
    currentRunStartedAt.value = null
    runMeta.value = null
    respOpenRows.value = new Set()
    // 重跑的结果按原位次回填，实时事件位次是「重跑子集内」的位次，两者不同位序，
    // 故重跑不做逐步实时渲染，整轮回填后一次展示（避免错位）。
    liveResults.value = []
    inFlight.value = new Map()
    progress.value = { done: 0, total: built.length }
    reportOpen.value = true
    running.value = true
    runId.value = crypto.randomUUID()
    try {
      const result = await api.runScenario({
        steps: built,
        environmentId: store.activeEnvId,
        stopOnFailure: false,
        runId: runId.value,
        scenarioId,
        initialVars: initial,
      })
      // 结果回填原位次：合并展示「原成功步 + 重跑步新结果」，变量池重跑值覆盖。
      const merged = [...res.results]
      const mergedSnapshots = [...varsSnapshots.value]
      result.results.forEach((nr, k) => {
        const pos = builtPositions[k]
        if (pos == null || pos >= merged.length) return
        merged[pos] = { ...nr, step_index: res.results[pos]?.step_index ?? nr.step_index }
        mergedSnapshots[pos] = {
          ...(varsSnapshots.value[pos] ?? {}),
          ...(result.vars_snapshots?.[k] ?? {}),
        }
      })
      runResult.value = {
        results: merged,
        cancelled: result.cancelled,
        stopped_on_failure: false,
        variables: { ...res.variables, ...result.variables },
      }
      varsSnapshots.value = mergedSnapshots
      currentRunAt.value = new Date().toISOString()
      void loadHistory()
    } catch (err) {
      runErrorToast(err)
    } finally {
      running.value = false
    }
  }

  // ---------- 按历史配置重跑 ----------
  /**
   * 历史回看态的「重跑」：用 meta.steps（运行当时的材料化配置快照）重建
   * 运行入参——不是「当前编辑态」，正是「历史执行基于当时配置快照」的兑现。
   * 初始变量取 meta.initialVars（敏感值已脱敏，缺失的敏感变量需用户重配环境）。
   */
  const rerunHistoryAvailable = computed(
    () => !running.value && runIsHistory.value && !!runMeta.value?.steps?.length,
  )

  async function rerunFromHistory(): Promise<void> {
    const meta = runMeta.value
    if (!scenario.value || running.value || !meta?.steps?.length) return
    const built: BuiltRunStep[] = meta.steps.map((m, i) => ({
      index: i,
      endpoint: { ...m.endpoint, request: m.spec_template },
      url: m.url_template,
      enabled: true,
      extract: m.extract.map((e) => ({ name: e.name, path: e.path })),
      assertions: m.assertions.map((a) => ({
        type: a.type,
        path: a.path,
        op: a.op ?? 'eq',
        expected: a.expected,
      })),
    }))
    resetRunState(built)
    const initialVars = meta.initialVars && Object.keys(meta.initialVars).length ? { ...meta.initialVars } : undefined
    try {
      const result = await api.runScenario({
        steps: built,
        environmentId: store.activeEnvId,
        stopOnFailure: meta.stopOnFailure ?? stopOnFailure.value,
        runId: runId.value,
        scenarioId,
        initialVars,
      })
      finishRun(result, built)
    } catch (err) {
      if (liveResults.value.some(Boolean)) {
        runResult.value = {
          results: liveResults.value.filter((r): r is ScenarioStepResult => !!r),
          cancelled: false,
          stopped_on_failure: false,
          variables: {},
        }
      }
      runErrorToast(err)
    } finally {
      running.value = false
      inFlight.value = new Map()
    }
  }

  // ---------- 结果面板派生 ----------
  /** 展开响应详情的结果行（结果位次集合；新运行 / 切场景时清空）。 */
  const respOpenRows = ref(new Set<number>())

  function toggleRespRow(i: number): void {
    const next = new Set(respOpenRows.value)
    if (next.has(i)) next.delete(i)
    else next.add(i)
    respOpenRows.value = next
  }

  /** 正文被 IPC 预览截断时经 get_response_body 按需取全文（键 = {runId}:{结果位次}）。 */
  function fullBodyLoader(i: number): (() => Promise<string | null>) | undefined {
    return () => api.getResponseBody(`${runId.value}:${i}`)
  }

  const runSummary = computed(() => {
    const r = runResult.value
    if (!r) return null
    const byStatus = (statuses: string[]) =>
      r.results.filter((x) => statuses.includes(x.step_status ?? '')).length
    const ok = byStatus(['success'])
    const blocked = byStatus(['blocked'])
    const skipped = byStatus(['skipped'])
    const failed = r.results.length - ok - blocked - skipped
    const duration = r.results.reduce((acc, x) => acc + (x.duration_ms ?? 0), 0)
    return {
      total: runStepIndexes.value.length,
      ok,
      fail: failed,
      blocked,
      skipped,
      duration,
    }
  })

  const finalState = computed(() => {
    const r = runResult.value
    const s = runSummary.value
    if (!r || !s) return null
    if (r.cancelled) return { text: t('scenario.cancelled'), tone: 'warn' as const }
    if (s.fail > 0) return { text: t('scenario.finalFail'), tone: 'fail' as const }
    return { text: t('scenario.finalOk'), tone: 'ok' as const }
  })

  const progressPercent = computed(() =>
    progress.value.total ? Math.round((progress.value.done / progress.value.total) * 100) : 0,
  )

  return {
    // 运行状态
    stopOnFailure,
    running,
    runId,
    progress,
    runResult,
    runStepIndexes,
    liveResults,
    inFlight,
    liveRows,
    livePending,
    applyProgressEvent,
    STRATEGY_OPTIONS,
    execStrategy,
    // 步骤执行状态
    stepTone,
    STEP_TONE_TEXT,
    STEP_TONE_ICON,
    resultByStepIndex,
    // 结果面板
    reportOpen,
    lifecycleRows,
    runIsHistory,
    currentRunAt,
    currentRunStartedAt,
    varsSnapshots,
    runHistory,
    loadHistory,
    restoreLatestRun,
    applyHistoryDetail,
    openHistoryMenu,
    onHistorySelect,
    exportReport,
    respOpenRows,
    toggleRespRow,
    fullBodyLoader,
    runSummary,
    finalState,
    progressPercent,
    // 运行 / 停止 / 子集执行 / 重跑
    run,
    stop,
    runFrom,
    rerunAvailable,
    rerunFailed,
    runMeta,
    rerunHistoryAvailable,
    rerunFromHistory,
    // 历史恢复（竞态防护）
    restoreRun,
    restoreLoading,
    dispose,
  }
}
