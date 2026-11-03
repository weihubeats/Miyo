/**
 * useScenarioRun：场景运行引擎（自 ScenarioEditor 抽出）。
 *
 * 职责（原 ScenarioEditor「运行状态 / 运行 / 步骤执行状态 / 执行结果面板 /
 * 重跑失败步骤」五个区块，行为不变）：
 * - 运行状态：running/runId/进度/结果/步骤位次/runtime 快照（历史回看无快照）；
 * - 运行：未保存先落库（避免「跑的不是看到的」）→ 构建启用步骤（URL 保留
 *   {{}} 模板，执行期 runtime 二次渲染）→ 调 runScenario；
 * - 重跑失败：以原运行 runtime 快照重建链路上下文，失败步按原位次回填合并；
 * - 运行历史：列表加载 / 最近一次恢复 / 详情回看（历史无快照不提供重跑）；
 * - 结果面板派生：步骤 tone、汇总、终态、进度百分比、响应行展开、全文加载。
 *
 * 留在宿主的视图层：Sortable 拖拽、locateStep（切视图 + 滚动定位）、
 * historyMenu DOM 锚点经 options 传入。
 */
import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { useWorkspaceStore } from '../stores/workspace'
import type { useMiyoApi } from '../composables/useMiyoApi'
import type { useToast } from '../composables/useToast'
import type {
  Endpoint,
  ScenarioAssertionRule,
  ScenarioExtractRule,
  ScenarioRunDetail,
  ScenarioRunResult,
  ScenarioRunSummary,
} from '../types/miyoApi'
import { formatDateTime } from '../utils/dateTime'
import type { MenuItem } from '../components/ui/Menu.vue'

/** 菜单打开能力（宿主持有 Menu 实例；组合式不依赖组件实例类型）。 */
export type MenuOpener = (anchor: HTMLElement, items: MenuItem[]) => void

/** 编辑态步骤的最小形状（ScenarioEditor 的 EditorStep；本组合式只读）。
 *  断言字段保持编辑态原始宽形状（op/expected 可空），构建入参时统一兜底。 */
export interface RunEditorStep {
  enabled: boolean
  endpointId: string
  extract: ScenarioExtractRule[]
  assertions: ScenarioAssertionRule[]
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
  /** 编辑态步骤数组（构建运行入参 / 重跑可用性判定用）。 */
  steps: Ref<RunEditorStep[]>
  /** 接口 id → 接口配置。 */
  endpointMap: ComputedRef<Map<string, Endpoint>>
  /** 接口 → URL 模板（路径变量先行代入，{{var}} 保留）。 */
  urlTemplateFor: (ep: Endpoint) => string
  /** 断言期望值字面量转型（数字 / 布尔 / 字符串）。 */
  coerceExpected: (raw: string) => unknown
  /** 编辑态是否有未保存修改（运行前先落库）。 */
  dirty: ComputedRef<boolean>
  /** 落库（运行前置）。 */
  save: () => Promise<boolean>
  /** 历史菜单打开回调（Menu.openAt 的绑定；实例归宿主持有）。 */
  openMenuAt: MenuOpener
}

/** 单个启用步骤 → 运行入参（URL 保留 {{}} 模板，执行期 runtime 二次渲染）。 */
export interface BuiltRunStep {
  index: number
  endpoint: Endpoint
  url: string
  enabled: boolean
  extract: { name: string; path: string }[]
  assertions: { type: string; path: string | null; op: string; expected: unknown }[]
}

export function useScenarioRun(options: UseScenarioRunOptions) {
  const { store, api, toast, t, scenario, scenarioId, steps, endpointMap, urlTemplateFor, coerceExpected, dirty, save, openMenuAt } =
    options

  // ---------- 运行状态 ----------
  const stopOnFailure = ref(true)
  const running = ref(false)
  const runId = ref('')
  const progress = ref({ done: 0, total: 0 })
  const runResult = ref<ScenarioRunResult | null>(null)
  /** 本次运行实际执行的步骤（编辑态索引，按运行顺序）；结果按位对应。 */
  const runStepIndexes = ref<number[]>([])

  /** 执行策略（失败即停的可读化表达，底层仍是 stop_on_failure 布尔）。 */
  const STRATEGY_OPTIONS = computed(() => [
    { value: 'stop', label: t('scenario.stopOnFailure') },
    { value: 'continue', label: t('scenario.strategyContinue') },
  ])
  const execStrategy = computed({
    get: () => (stopOnFailure.value ? 'stop' : 'continue'),
    set: (v: string | null) => {
      stopOnFailure.value = v !== 'continue'
    },
  })

  // ---------- 步骤执行状态（卡片徽章 + 结果面板共用） ----------
  type StepTone = 'ok' | 'fail' | 'skip' | 'run' | 'done' | 'wait' | 'cancel'

  const resultByStepIndex = computed(() => {
    const m = new Map<number, ScenarioRunResult['results'][number]>()
    const results = runResult.value?.results ?? []
    runStepIndexes.value.forEach((stepIdx, i) => {
      const r = results[i]
      if (r) m.set(stepIdx, r)
    })
    return m
  })

  /** 结果按位对应 runStepIndexes；运行中仅进度可辨（无逐步中间结果）。 */
  function stepTone(i: number): StepTone | null {
    const pos = runStepIndexes.value.indexOf(i)
    if (pos === -1) return null
    if (running.value) {
      if (pos < progress.value.done) return 'done'
      return pos === progress.value.done ? 'run' : 'wait'
    }
    if (!runResult.value) return null
    const r = resultByStepIndex.value.get(i)
    if (r) {
      if (r.cancelled) return 'cancel'
      return r.ok ? 'ok' : 'fail'
    }
    return runResult.value.stopped_on_failure || runResult.value.cancelled ? 'skip' : 'wait'
  }

  const STEP_TONE_TEXT: Record<StepTone, string> = {
    ok: 'scenario.stepStatusOk',
    fail: 'scenario.stepStatusFail',
    skip: 'scenario.stepStatusSkip',
    run: 'scenario.stepStatusRun',
    done: 'scenario.stepStatusDone',
    wait: 'scenario.stepStatusWait',
    cancel: 'scenario.stepStatusCancelled',
  }
  const STEP_TONE_ICON: Record<StepTone, 'check' | 'x' | 'chevron-right' | 'dot' | 'clock' | 'stop'> = {
    ok: 'check',
    fail: 'x',
    skip: 'chevron-right',
    run: 'dot',
    done: 'check',
    wait: 'clock',
    cancel: 'stop',
  }

  // ---------- 执行结果面板 ----------
  const reportOpen = ref(true)

  /** 当前展示结果是否来自运行历史（重开恢复 / 历史回看）；实时运行为 false。 */
  const runIsHistory = ref(false)
  /** 当前展示结果的完成时刻（RFC3339）。 */
  const currentRunAt = ref<string | null>(null)
  /** 每步执行前的 runtime 快照（与 results 同位序；重跑失败步骤的初始变量来源，
   * 仅实时运行有，历史回看无）。 */
  const varsSnapshots = ref<Record<string, string>[]>([])

  /** 运行历史列表（新→旧；场景切换后刷新）。 */
  const runHistory = ref<ScenarioRunSummary[]>([])

  async function loadHistory(): Promise<void> {
    if (!scenario.value) {
      runHistory.value = []
      return
    }
    try {
      runHistory.value = await api.listScenarioRuns(scenarioId)
    } catch {
      runHistory.value = []
    }
  }

  /** 重开场景恢复最近一次运行（历史能力；无历史时保持空态）。 */
  async function restoreLatestRun(scenarioIdAtLoad: string): Promise<void> {
    const latest = runHistory.value[0]
    if (!latest || scenarioId !== scenarioIdAtLoad) return
    try {
      const detail = await api.getScenarioRun(latest.id)
      if (!detail || scenarioId !== scenarioIdAtLoad) return
      applyHistoryDetail(detail)
    } catch {
      // 历史恢复失败静默：面板保持空态，不打扰首次编辑。
    }
  }

  function applyHistoryDetail(detail: ScenarioRunDetail): void {
    runResult.value = {
      results: detail.results,
      cancelled: detail.cancelled,
      stopped_on_failure: detail.stopped_on_failure,
      variables: detail.variables,
    }
    // 结果自带链路位次（step_index），按位映射到步骤卡。
    runStepIndexes.value = detail.results.map((r) => r.step_index ?? -1)
    runId.value = detail.id
    progress.value = { done: detail.results.length, total: detail.total_steps }
    currentRunAt.value = detail.finished_at
    runIsHistory.value = true
    reportOpen.value = true
    respOpenRows.value = new Set()
    // 历史结果无 runtime 快照：重跑失败步骤不可用（不提供假按钮）。
    varsSnapshots.value = []
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
      : [{ key: 'empty', label: t('scenario.historyEmpty'), disabled: true }]
    openMenuAt(e.currentTarget as HTMLElement, items)
  }

  async function onHistorySelect(item: MenuItem): Promise<void> {
    try {
      const detail = await api.getScenarioRun(item.key)
      if (!detail) {
        toast.warning(t('scenario.historyMissing'))
        await loadHistory()
        return
      }
      applyHistoryDetail(detail)
    } catch (err) {
      toast.error(t('scenario.runFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }

  // ---------- 运行 ----------
  /** 单个启用步骤 → 运行入参。 */
  function buildOneRunStep(stepIdx: number): BuiltRunStep | null {
    const s = steps.value[stepIdx]
    if (!s || !s.enabled) return null
    const ep = endpointMap.value.get(s.endpointId)
    if (!ep) return null
    return {
      index: stepIdx,
      endpoint: ep,
      url: urlTemplateFor(ep),
      enabled: true,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          type: r.type as string,
          path: r.path?.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected as unknown as string),
        })),
    }
  }

  function buildRunSteps(): BuiltRunStep[] {
    const out: BuiltRunStep[] = []
    steps.value.forEach((_, i) => {
      const built = buildOneRunStep(i)
      if (built) out.push(built)
    })
    return out
  }

  async function run(): Promise<void> {
    if (!scenario.value || running.value) return
    // 未保存先落库（后端按提交内容执行，避免「跑的不是看到的」）。
    if (dirty.value && !(await save())) return
    const built = buildRunSteps()
    if (!built.length) {
      toast.warning(t('scenario.noEnabledSteps'))
      return
    }
    runResult.value = null
    runStepIndexes.value = built.map((b) => b.index)
    respOpenRows.value = new Set()
    runIsHistory.value = false
    currentRunAt.value = null
    varsSnapshots.value = []
    progress.value = { done: 0, total: built.length }
    reportOpen.value = true
    running.value = true
    runId.value = crypto.randomUUID()
    try {
      const result = await api.runScenario({
        steps: built,
        environmentId: store.activeEnvId,
        stopOnFailure: stopOnFailure.value,
        runId: runId.value,
        scenarioId,
      })
      runResult.value = result
      varsSnapshots.value = result.vars_snapshots ?? []
      currentRunAt.value = new Date().toISOString()
      void loadHistory()
    } catch (err) {
      toast.error(t('scenario.runFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    } finally {
      running.value = false
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

  // ---------- 重跑失败步骤 ----------
  // 仅实时运行结果可重跑（需要 runtime 快照重建链路上下文；历史无快照不提供入口）。
  // 语义：失败步骤按原顺序串行重跑；首个重跑步以当时的 runtime 为初始变量，
  // 其后每个重跑步先合并原运行中「位于两者之间的成功步骤」的提取增量——
  // 即「若这些步骤依次重跑」的链路状态；失败步骤自身不并入旧提取值（重跑会重新提取）。

  /** 可重跑的结果位次（失败且非取消、且对应步骤仍能按当前编辑态构建）。 */
  function rerunPositions(): number[] {
    const res = runResult.value
    if (!res) return []
    const out: number[] = []
    res.results.forEach((r, i) => {
      if (r.ok || r.cancelled) return
      const stepIdx = runStepIndexes.value[i]
      if (stepIdx == null || stepIdx < 0) return
      if (buildOneRunStep(stepIdx)) out.push(i)
    })
    return out
  }

  /** 是否存在可重跑的失败步骤（实时结果 + 有快照 + 步骤仍可构建）。 */
  const rerunAvailable = computed(() => {
    if (running.value || runIsHistory.value || !runResult.value || !varsSnapshots.value.length)
      return false
    return rerunPositions().length > 0
  })

  async function rerunFailed(): Promise<void> {
    if (!scenario.value || running.value) return
    const res = runResult.value
    if (!res) return
    const positions = rerunPositions()
    if (!positions.length) {
      toast.warning(t('scenario.rerunUnavailable'))
      return
    }
    if (!varsSnapshots.value.length) {
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
    for (const pos of positions) {
      const stepIdx = runStepIndexes.value[pos]
      const one = buildOneRunStep(stepIdx)
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
    respOpenRows.value = new Set()
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
      toast.error(t('scenario.runFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    } finally {
      running.value = false
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
    const ok = r.results.filter((x) => x.ok).length
    const duration = r.results.reduce((acc, x) => acc + (x.duration_ms ?? 0), 0)
    return {
      total: runStepIndexes.value.length,
      ok,
      fail: r.results.length - ok,
      // 失败即停 / 取消时未执行的启用步骤计为跳过。
      skipped: Math.max(0, runStepIndexes.value.length - r.results.length),
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
    STRATEGY_OPTIONS,
    execStrategy,
    // 步骤执行状态
    stepTone,
    STEP_TONE_TEXT,
    STEP_TONE_ICON,
    resultByStepIndex,
    // 结果面板
    reportOpen,
    runIsHistory,
    currentRunAt,
    varsSnapshots,
    runHistory,
    loadHistory,
    restoreLatestRun,
    applyHistoryDetail,
    openHistoryMenu,
    onHistorySelect,
    respOpenRows,
    toggleRespRow,
    fullBodyLoader,
    runSummary,
    finalState,
    progressPercent,
    // 运行 / 重跑
    buildOneRunStep,
    buildRunSteps,
    run,
    stop,
    rerunAvailable,
    rerunFailed,
  }
}
