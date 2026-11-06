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
} from '../types/miyoApi'
import { formatDateTime } from '../utils/dateTime'
import type { MenuItem } from '../components/ui/Menu.vue'

/** 菜单打开能力（宿主持有 Menu 实例；组合式不依赖组件实例类型）。 */
export type MenuOpener = (anchor: HTMLElement, items: MenuItem[]) => void

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
}

export function useScenarioRun(options: UseScenarioRunOptions) {
  const { store, api, toast, t, scenario, scenarioId, buildRunSteps, dirty, save, openMenuAt } =
    options

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
  /** 运行中「已进入但未结束」的步：位次 + 渲染后实际发出的 URL。 */
  const inFlight = ref<{ pos: number; url: string } | null>(null)

  /** 接收进度事件（由宿主组件转发 `miyo:scenario-progress`）。 */
  function applyProgressEvent(p: ScenarioProgressEvent): void {
    if (!runId.value || p.runId !== runId.value) return
    progress.value = { done: p.done, total: p.total || progress.value.total }
    if (p.result) {
      const stepIdx = p.result.step_index ?? p.done - 1
      const pos = runStepIndexes.value.indexOf(stepIdx)
      const at = pos === -1 ? Math.max(0, p.done - 1) : pos
      const next = [...liveResults.value]
      next[at] = p.result
      liveResults.value = next
      if (inFlight.value?.pos === at) inFlight.value = null
      return
    }
    inFlight.value = { pos: p.done, url: p.url ?? '' }
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
      // 尚未结束：仅在途步占位（后续步骤保持徽章「待执行」）。
      if (inFlight.value?.pos === pos) {
        rows.push({ result: null, pos, url: inFlight.value.url || null })
      }
      break
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
      return inFlight.value?.pos === pos ? 'run' : 'wait'
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
    currentRunStartedAt.value = detail.meta?.startedAt ?? null
    runMeta.value = detail.meta ?? null
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
      : [{ key: 'empty', label: t('integration.historyEmpty'), disabled: true }]
    openMenuAt(e.currentTarget as HTMLElement, items)
  }

  async function onHistorySelect(item: MenuItem): Promise<void> {
    try {
      const detail = await api.getScenarioRun(item.key)
      if (!detail) {
        toast.warning(t('integration.historyMissing'))
        await loadHistory()
        return
      }
      applyHistoryDetail(detail)
    } catch (err) {
      toast.error(t('integration.runStartFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }

  // ---------- 运行 ----------
  function resetRunState(built: BuiltRunStep[]): void {
    runResult.value = null
    runStepIndexes.value = built.map((b) => b.index)
    respOpenRows.value = new Set()
    runIsHistory.value = false
    currentRunAt.value = null
    currentRunStartedAt.value = null
    runMeta.value = null
    varsSnapshots.value = []
    liveResults.value = []
    inFlight.value = null
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
      inFlight.value = null
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
   * 无快照（从未整跑过 / 历史回看）时仅用环境变量并提示。
   */
  async function runFrom(fromIdx: number, onlyThis: boolean): Promise<void> {
    const built = await prepareRun()
    if (!built) return
    const all = buildRunSteps()
    const start = all.findIndex((b) => b.index === fromIdx)
    if (start === -1) return
    const subset = onlyThis ? [all[start]] : all.slice(start)
    if (!subset.length) return

    const snapshot = varsSnapshots.value[start]
    const initialVars = snapshot ? { ...snapshot } : undefined
    if (!snapshot) {
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
      inFlight.value = null
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
    if (running.value || runIsHistory.value || !runResult.value || !varsSnapshots.value.length)
      return false
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
    inFlight.value = null
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
      inFlight.value = null
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
  }
}
