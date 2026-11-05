<script lang="ts">
/**
 * 模块级状态：<script setup> 里的声明随组件实例生灭，而场景编辑器在
 * WorkspaceView 中以 v-if 挂载——切到其它页签即卸载。编辑态缓存与步骤
 * key 序号必须活过卸载，故置于模块作用域（两块脚本编译后同属一个模块）。
 */
import type { ScenarioAssertionRule, ScenarioExtractRule, ScenarioRunResult } from '../types/miyoApi'

/** 断言期望值编辑态统一为字符串，保存时按字面量智能转型。 */
export type EditorAssertion = Omit<ScenarioAssertionRule, 'expected'> & { expected: string }

export interface EditorStep {
  /** 本地稳定 key（v-for / 拖拽定位用，不入库）。 */
  key: string
  endpointId: string
  enabled: boolean
  extract: ScenarioExtractRule[]
  assertions: EditorAssertion[]
  /** 选中态（单选 accordion）：选中步骤的详情展示在右栏。 */
  open: boolean
}

export interface EditorDraft {
  name: string
  description: string
  steps: EditorStep[]
  savedSnapshot: string
  scriptDraft: string
  scriptCustomized: boolean
  viewMode: 'manual' | 'script'
  stopOnFailure: boolean
  runResult: ScenarioRunResult | null
  runStepIndexes: number[]
  /** 当前展示结果是否来自运行历史（区别实时运行）。 */
  runIsHistory: boolean
  /** 当前展示结果的完成时刻（RFC3339；无结果为 null）。 */
  currentRunAt: string | null
  /** 每步执行前的 runtime 快照（重跑失败步骤用；历史回看为空数组）。 */
  varsSnapshots: Record<string, string>[]
}

/** 页签切换的编辑态缓存（见 EditorDraft）；场景删除时随加载失败自然失效。 */
const draftCache = new Map<string, EditorDraft>()

let stepKeySeq = 0
function nextStepKey(): string {
  stepKeySeq += 1
  return `step-${stepKeySeq}`
}
</script>

<script setup lang="ts">
/**
 * ScenarioEditor：场景编排视图（scenario:{id} 伪页签的内容区）。
 *
 * 布局（三栏工作区，左栏为工作区侧栏的「场景」管理区 ScenarioPanel）：
 * - 顶部工具栏：场景名 + 保存状态 + 元信息（项目 / 环境 / 步骤数）+
 *   视图切换 + 执行策略 + 保存 / 运行（主操作突出）；
 * - 中列流程编排：变量链路总览（可折叠，默认收起）+ 纵向步骤卡片
 *   （紧凑摘要：序号 / 方法 / 名称 / URL / 执行状态 / 提取·断言数 / 耗时 /
 *   变量出入参 chips）+ 拖拽 / 启停 / 复制 / 删除（菜单内确认）；
 * - 右栏步骤详情（≥1360px 内联，窄屏抽屉）：Tab 划分请求配置（只读，含
 *   「去接口编辑」直达）/ 变量提取 / 断言规则；
 * - 底部结果 Dock：可折叠执行结果面板（摘要 chips + 每步结果 + 定位）；
 * - 脚本视图：由当前配置实时生成的只读预览（可编辑、复制、导出 .mjs）。
 *
 * 数据流不变：手动步骤是唯一编辑真源；运行串行（前步提取值经 runtime
 * 渲染后续请求模板）+ 进度事件 + 可取消；未保存改动在运行前自动保存。
 * 页签切换（v-if 卸载）时编辑态按 scenario id 暂存模块级缓存，回来恢复。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import Sortable from 'sortablejs'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useVarCandidates } from '../composables/useVarCandidates'
import { copyText } from '../utils/clipboard'
import { applyPathVars } from '../utils/pathVariables'
import { formatDurationShort } from '../utils/format'
import { methodBadgeTone } from '../utils/methodTone'
import {
  computeVarChain,
  extractVarRefs,
  isSensitiveVarName,
  type VarScanStep,
} from '../utils/scenarioVars'
import { generateScenarioScript, type ScriptAuth, type ScriptStep } from '../utils/scenarioScript'
import {
  parseScenarioScript,
  ScriptParseError,
  type ParsedScriptStep,
} from '../utils/scenarioScriptParse'
import { useScenarioRun, type ScenarioProgressEvent } from '../composables/useScenarioRun'
import { mergedVariableMap } from '../utils/environment'
import CustomSelect from './ui/CustomSelect.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import SegmentedControl, { type SegmentOption } from './ui/SegmentedControl.vue'
import EmptyState from './ui/EmptyState.vue'
import EnvBadge from './ui/EnvBadge.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Popconfirm from './ui/Popconfirm.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import ScenarioVarsPanel from './scenario/ScenarioVarsPanel.vue'
import ScenarioStepDetail from './scenario/ScenarioStepDetail.vue'
import ScenarioRunReport from './scenario/ScenarioRunReport.vue'
// ScenarioAssertionRule / ScenarioExtractRule / ScenarioRunResult 等类型
// 由模块级 <script> 块导入（两块脚本同属一个模块作用域）。
import type {
  Endpoint,
  Scenario,
  ScenarioStep,
} from '../types/miyoApi'

const props = defineProps<{ scenarioId: string }>()
const store = useWorkspaceStore()
const t = useLocaleStore().t
const toast = useToast()
const api = useMiyoApi()

// ---------- 编辑态（本地副本 + 显式保存） ----------
// EditorStep / EditorAssertion / draftCache 定义在模块级 <script> 块（须活过组件卸载）。

const name = ref('')
const description = ref('')
const steps = ref<EditorStep[]>([])
const savedSnapshot = ref('')
const loading = ref(true)
const saving = ref(false)

const scenario = computed(() => store.scenarioIndex.get(props.scenarioId) ?? null)

function decodeStep(s: ScenarioStep): EditorStep {
  const extract = Array.isArray(s.extract) ? (s.extract as ScenarioExtractRule[]) : []
  const assertions = Array.isArray(s.assertions) ? (s.assertions as ScenarioAssertionRule[]) : []
  return {
    key: nextStepKey(),
    endpointId: s.endpoint_id,
    enabled: s.enabled,
    extract: extract.map((r) => ({ ...r })),
    assertions: assertions.map((r) => ({
      name: r.name ?? null,
      type: r.type,
      path: r.path ?? '',
      op: r.op ?? 'eq',
      expected: r.expected == null ? '' : String(r.expected),
    })),
    open: false,
  }
}

/** 归一化（去空行）后的步骤，用于脏比较与落库。 */
function normalizedSteps(): unknown[] {
  return steps.value.map((s) => ({
    endpoint_id: s.endpointId,
    enabled: s.enabled,
    extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
    assertions: s.assertions
      .filter((r) => r.type)
      .map((r) => ({
        type: r.type,
        path: r.path?.trim() || null,
        op: r.op || 'eq',
        expected: coerceExpected(r.expected),
      })),
  }))
}

/** 期望值按字面量转型：数字 / 布尔 / 其余保持字符串。 */
function coerceExpected(raw: string): unknown {
  const v = raw.trim()
  if (v === 'true') return true
  if (v === 'false') return false
  if (v !== '' && !Number.isNaN(Number(v))) return Number(v)
  return v
}

function snapshot(): string {
  return JSON.stringify({
    name: name.value.trim(),
    description: description.value.trim(),
    steps: normalizedSteps(),
    // 自定义脚本纳入脏检查；跟随生成版（未编辑）视为无改动。
    script: scriptCustomized.value ? scriptDraft.value : null,
  })
}

const dirty = computed(() => snapshot() !== savedSnapshot.value)

async function save(): Promise<boolean> {
  if (!scenario.value) return false
  saving.value = true
  try {
    const updated: Scenario = {
      ...scenario.value,
      name: name.value.trim() || t('scenario.untitled'),
      description: description.value.trim(),
      steps: normalizedSteps() as Scenario['steps'],
      script: scriptCustomized.value ? scriptDraft.value : null,
    }
    const saved = await store.upsertScenario(updated)
    if (!saved) return false
    savedSnapshot.value = snapshot()
    return true
  } finally {
    saving.value = false
  }
}

// ---------- 页签切换的编辑态缓存 ----------
// WorkspaceView 以 v-if 挂载编辑器：切到其它页签会卸载本组件。未保存改动
// / 运行结果 / 选中态按 scenario id 暂存模块级缓存，切回时恢复（保存后
// 仍保留，以维持选中态与运行结果；场景删除时随加载失败自然失效）。

function stashDraft(): void {
  if (!scenario.value) return
  draftCache.set(props.scenarioId, {
    name: name.value,
    description: description.value,
    steps: steps.value,
    savedSnapshot: savedSnapshot.value,
    scriptDraft: scriptDraft.value,
    scriptCustomized: scriptCustomized.value,
    viewMode: viewMode.value,
    stopOnFailure: stopOnFailure.value,
    runResult: runResult.value,
    runStepIndexes: runStepIndexes.value,
    runIsHistory: runIsHistory.value,
    currentRunAt: currentRunAt.value,
    varsSnapshots: varsSnapshots.value,
  })
}

// ---------- 步骤编辑 ----------
const endpointOptions = computed(() =>
  store.endpoints.map((e) => ({ value: e.id, label: `${e.method} ${e.name}` })),
)
const endpointMap = computed(() => new Map(store.endpoints.map((e) => [e.id, e] as const)))

function stepEndpoint(i: number): Endpoint | null {
  const s = steps.value[i]
  return (s && endpointMap.value.get(s.endpointId)) || null
}

/** 结果位次（运行顺序）→ 编辑态步骤索引（运行中在途步还没结果，靠它反查名称）。 */
function runPosStepIndex(pos: number): number {
  return runStepIndexes.value[pos] ?? -1
}

/** 在途步的接口名（结果尚未回传时占位显示）。 */
function stepName(pos: number): string {
  const idx = runPosStepIndex(pos)
  const ep = idx >= 0 ? stepEndpoint(idx) : null
  return ep?.name ?? ''
}

/** 在途步的方法（结果尚未回传时占位显示）。 */
function stepMethodName(pos: number): string {
  const idx = runPosStepIndex(pos)
  const ep = idx >= 0 ? stepEndpoint(idx) : null
  return ep?.method ?? ''
}

function addStep(): void {
  if (running.value) return
  const first = store.endpoints[0]
  const step: EditorStep = {
    key: nextStepKey(),
    endpointId: first?.id ?? '',
    enabled: true,
    extract: [],
    assertions: [],
    open: false,
  }
  steps.value.push(step)
  selectStep(step)
}

function duplicateStep(i: number): void {
  if (running.value) return
  const s = steps.value[i]
  if (!s) return
  const copy: EditorStep = {
    key: nextStepKey(),
    endpointId: s.endpointId,
    enabled: s.enabled,
    extract: s.extract.map((r) => ({ ...r })),
    assertions: s.assertions.map((r) => ({ ...r })),
    open: false,
  }
  steps.value.splice(i + 1, 0, copy)
  selectStep(copy)
}

function removeAt(i: number): void {
  if (running.value) return
  steps.value.splice(i, 1)
}

function moveStep(i: number, delta: number): void {
  const j = i + delta
  if (j < 0 || j >= steps.value.length) return
  const [row] = steps.value.splice(i, 1)
  steps.value.splice(j, 0, row)
}

// ---------- 步骤选中态（单选 accordion；详情进右栏） ----------
function selectStep(s: EditorStep): void {
  for (const x of steps.value) x.open = x === s
  drawerOpen.value = true
}

function closeDetail(): void {
  drawerOpen.value = false
}

const selectedStep = computed(() => steps.value.find((s) => s.open) ?? null)
const selectedIndex = computed(() => steps.value.findIndex((s) => s.open))

// ---------- 步骤更多操作菜单 ----------
const stepMenu = ref<InstanceType<typeof Menu> | null>(null)
const stepMenuIdx = ref<number | null>(null)

function stepMenuLabel(i: number): string {
  return stepEndpoint(i)?.name || t('scenario.stepNameFallback')
}

function openStepMenu(e: MouseEvent, i: number): void {
  stepMenuIdx.value = i
  const items: MenuItem[] = [
    { key: 'dup', label: t('scenario.dupStep'), icon: 'copy', disabled: running.value },
    { key: 'up', label: t('scenario.moveUp'), icon: 'chevron-up', disabled: running.value || i === 0 },
    {
      key: 'down',
      label: t('scenario.moveDown'),
      icon: 'chevron-down',
      disabled: running.value || i === steps.value.length - 1,
    },
    {
      key: 'del',
      label: t('scenario.removeStep'),
      icon: 'trash',
      danger: true,
      disabled: running.value,
      dividerBefore: true,
      confirm: t('scenario.confirmRemoveStep', { name: stepMenuLabel(i) }),
    },
  ]
  stepMenu.value?.openAt(e.currentTarget as HTMLElement, items)
}

function onStepMenuSelect(item: MenuItem): void {
  const i = stepMenuIdx.value
  if (i == null) return
  if (item.key === 'dup') duplicateStep(i)
  else if (item.key === 'up') moveStep(i, -1)
  else if (item.key === 'down') moveStep(i, 1)
}

function onStepMenuConfirm(item: MenuItem): void {
  const i = stepMenuIdx.value
  if (i == null) return
  if (item.key === 'del') removeAt(i)
}

// ---------- 拖拽排序（手柄拖动；落点后按 DOM 顺序回写数组） ----------
const stepsEl = ref<HTMLElement | null>(null)
let sortable: Sortable | null = null

function initSortable(): void {
  if (!stepsEl.value) {
    sortable?.destroy()
    sortable = null
    return
  }
  if (!sortable) {
    sortable = Sortable.create(stepsEl.value, {
      handle: '.se-drag',
      animation: 150,
      easing: 'cubic-bezier(0.2, 0, 0, 1)',
      forceFallback: true,
      fallbackTolerance: 3,
      fallbackOnBody: true,
      ghostClass: 'se-ghost',
      chosenClass: 'se-chosen',
      filter: 'button, input, select, textarea',
      preventOnFilter: false,
      onEnd: () => onDragEnd(),
    })
  }
  sortable.option('disabled', running.value)
}

/** 与 ProjectList 同款收口：读 DOM 实际顺序（data-step-key）回写数组。 */
function onDragEnd(): void {
  const el = stepsEl.value
  if (!el || running.value) return
  const byKey = new Map(steps.value.map((s) => [s.key, s] as const))
  const ordered: EditorStep[] = []
  for (const node of el.querySelectorAll<HTMLElement>('[data-step-key]')) {
    const s = node.dataset.stepKey ? byKey.get(node.dataset.stepKey) : undefined
    if (s && !ordered.includes(s)) ordered.push(s)
  }
  if (ordered.length === steps.value.length) steps.value = ordered
}

// ---------- 变量链路（从真实配置推导） ----------
const varCandidates = useVarCandidates()

/** 接口当前配置 → URL 模板（路径变量先行代入；{{var}} 保留给执行期渲染）。 */
function urlTemplateFor(ep: Endpoint): string {
  const domain = store.urlDomain
  const isAbs = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(ep.path)
  const path = ep.path.startsWith('/') ? ep.path : `/${ep.path}`
  return isAbs
    ? applyPathVars(ep.path, ep.request.path_variables)
    : `${domain}${applyPathVars(path, ep.request.path_variables)}`
}

/** 启用步骤的链路扫描输入：提取规则 + 参与引用扫描的模板文本。 */
const varScanSteps = computed<VarScanStep[]>(() =>
  steps.value.reduce<VarScanStep[]>((acc, s, i) => {
    if (!s.enabled) return acc
    const ep = endpointMap.value.get(s.endpointId)
    const texts: string[] = []
    if (ep) {
      texts.push(urlTemplateFor(ep))
      for (const h of ep.request.headers) if (h.enabled) texts.push(h.key, h.value)
      for (const p of ep.request.params) if (p.enabled) texts.push(p.key, p.value)
      const body = ep.request.body
      switch (body.mode) {
        case 'json':
        case 'text':
          texts.push(body.raw)
          break
        case 'urlencoded':
        case 'multipart':
          for (const f of body.fields) if (f.enabled) texts.push(f.value)
          break
        case 'graphql':
          texts.push(body.spec.query, body.spec.variables)
          break
        default:
          break
      }
    }
    for (const a of s.assertions) {
      if (!a.type) continue
      texts.push(a.expected)
    }
    acc.push({
      index: i,
      name: ep?.name || t('scenario.stepNameFallback'),
      extracts: s.extract
        .filter((r) => r.name.trim() && r.path.trim())
        .map((r) => ({ name: r.name.trim(), from: r.from, path: r.path.trim() })),
      referenceTexts: texts,
    })
    return acc
  }, []),
)

const varChain = computed(() => computeVarChain(varScanSteps.value, varCandidates.value))

/** 某步骤引用的变量在该步执行时是否有值（更早步骤定义过，或来自外部变量池）。 */
const definedAtIndex = computed(() => {
  const m = new Map<string, number>()
  for (const scan of varScanSteps.value) {
    for (const e of scan.extracts) {
      const at = m.get(e.name)
      if (at === undefined || scan.index < at) m.set(e.name, scan.index)
    }
  }
  return m
})

function isMissingAt(stepIndex: number, varName: string): boolean {
  if (varCandidates.value.includes(varName)) return false
  const at = definedAtIndex.value.get(varName)
  return at === undefined || at >= stepIndex
}

/** 步骤折叠态的入参摘要：该步引用的 {{var}}（保序去重）。 */
function stepInputVars(i: number): string[] {
  const scan = varScanSteps.value.find((v) => v.index === i)
  if (!scan) return []
  const out: string[] = []
  for (const text of scan.referenceTexts) {
    for (const n of extractVarRefs(text)) if (!out.includes(n)) out.push(n)
  }
  return out
}

/** 步骤折叠态的出参摘要：提取规则的有效变量名。 */
function stepOutputVars(i: number): string[] {
  const s = steps.value[i]
  if (!s) return []
  return s.extract.filter((r) => r.name.trim() && r.path.trim()).map((r) => r.name.trim())
}

/** 卡片引用 chips：该步引用的链路变量（有序去重）。 */
function stepRefs(i: number): string[] {
  return stepInputVars(i)
}

function varTag(n: string): string {
  return `{{${n}}}`
}

/** 变量名 → 最早定义步骤（卡片 chip 的来源标记用）。 */
const varSourceIndex = computed<Map<string, number>>(() => {
  const m = new Map<string, number>()
  for (const v of varChain.value) {
    if (!v.sources.length) continue
    const first = v.sources.reduce((a, b) => (a.stepIndex <= b.stepIndex ? a : b))
    m.set(v.name, first.stepIndex)
  }
  return m
})

/** 变量 chip 的悬浮说明：来自第 N 步 / 未定义。 */
function refTitle(i: number, name_: string): string {
  if (isMissingAt(i, name_)) return t('scenario.varMissingWarn')
  const from = varSourceIndex.value.get(name_)
  return from == null ? varTag(name_) : t('scenario.usesFrom', { n: from + 1 })
}

// ---------- 运行引擎（useScenarioRun） ----------
// 运行 / 重跑失败 / 历史回看 / 结果面板派生抽至 composables/useScenarioRun。
// 必须先于视图模式区声明：generatedScript computed 与其 watch 都读 stopOnFailure。
const historyMenu = ref<InstanceType<typeof Menu> | null>(null)
const {
  stopOnFailure,
  running,
  progress,
  runResult,
  runStepIndexes,
  liveRows,
  livePending,
  applyProgressEvent,
  STRATEGY_OPTIONS,
  execStrategy,
  stepTone,
  STEP_TONE_TEXT,
  STEP_TONE_ICON,
  reportOpen,
  runIsHistory,
  currentRunAt,
  varsSnapshots,
  runHistory,
  loadHistory,
  restoreLatestRun,
  openHistoryMenu,
  onHistorySelect,
  respOpenRows,
  toggleRespRow,
  fullBodyLoader,
  runSummary,
  finalState,
  progressPercent,
  resultByStepIndex,
  run,
  stop,
  rerunAvailable,
  rerunFailed,
} = useScenarioRun({
  store,
  api,
  toast,
  t,
  scenario,
  scenarioId: props.scenarioId,
  steps,
  endpointMap,
  urlTemplateFor,
  coerceExpected,
  dirty,
  save,
  openMenuAt: (anchor, items) => historyMenu.value?.openAt(anchor, items),
})

/** 结果 Dock 的「定位」事件：运行位次 → 编辑态步骤索引 → 步骤卡滚动定位。 */
function onReportLocate(pos: number): void {
  locateStep(runPosStepIndex(pos))
}

/** 步骤卡的执行耗时徽章（运行后展示）。 */
function stepDurationMs(i: number): number | null {
  return resultByStepIndex.value.get(i)?.duration_ms ?? null
}

// 拖拽容器挂载 / 运行态切换后同步 Sortable（disabled 声明在 running 之后，避开 TDZ）。
watch(stepsEl, () => nextTick(initSortable))
watch(running, () => sortable?.option('disabled', running.value))

// ---------- 右栏（≥1360px 内联；窄屏抽屉叠加） ----------
const NARROW_QUERY = '(max-width: 1359px)'
const narrowMql = window.matchMedia(NARROW_QUERY)
const isNarrow = ref(narrowMql.matches)
function onNarrowChange(e: MediaQueryListEvent): void {
  isNarrow.value = e.matches
}
narrowMql.addEventListener('change', onNarrowChange)

/** 抽屉开关（窄屏有效；宽屏右栏常驻，仅控制选中态）。 */
const drawerOpen = ref(false)

function onEscKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape' && isNarrow.value && drawerOpen.value) drawerOpen.value = false
}

/** 右栏详情的只读请求分区（宿主按运行同口径物化后下发）。 */
const selectedEndpoint = computed(() => (selectedIndex.value >= 0 ? stepEndpoint(selectedIndex.value) : null))
const selectedUrl = computed(() => (selectedIndex.value >= 0 ? stepUrl(selectedIndex.value) : ''))
const selectedPathVars = computed(() => (selectedIndex.value >= 0 ? enabledPathVars(selectedIndex.value) : []))
const selectedParams = computed(() => (selectedIndex.value >= 0 ? enabledParams(selectedIndex.value) : []))
const selectedHeaders = computed(() => (selectedIndex.value >= 0 ? enabledHeaders(selectedIndex.value) : []))
const selectedBodyKind = computed(() => selectedEndpoint.value?.request.body.mode ?? 'none')
const selectedBodyText = computed(() => (selectedIndex.value >= 0 ? bodyText(selectedIndex.value) : ''))
const selectedTone = computed(() => (selectedIndex.value >= 0 ? stepTone(selectedIndex.value) : null))
const selectedStatusText = computed(() => {
  const tone = selectedTone.value
  return tone ? t(STEP_TONE_TEXT[tone]) : null
})
const selectedDuration = computed(() => (selectedIndex.value >= 0 ? stepDurationMs(selectedIndex.value) : null))

/** 「在接口编辑器中修改」直达：打开步骤引用的接口页签。 */
function editSelectedEndpoint(): void {
  const ep = selectedEndpoint.value
  if (!ep) return
  store.openEndpoint(ep)
}

// ---------- 视图模式：手动编排 ⇄ 脚本 ----------
/** 手动是唯一编辑真源；脚本视图由步骤实时生成的只读预览（可复制/导出/进 CI）。 */
const viewMode = ref<'manual' | 'script'>('manual')
const VIEW_MODES = computed<SegmentOption[]>(() => [
  { value: 'manual', label: t('scenario.tabManual') },
  { value: 'script', label: t('scenario.tabScript') },
])

const scriptBase = computed(() => {
  const env = store.environments.find((e) => e.id === store.activeEnvId)
  const envBase = (env?.base_url ?? '').trim()
  if (envBase) return envBase
  return store.urlDomain === '{{base_url}}' ? store.sessionBaseUrl : store.urlDomain
})

/** 鉴权 → 可静态映射的脚本形态；不可映射的返回方式名（生成明确注释）。 */
function toScriptAuth(ep: Endpoint): { auth: ScriptAuth | null; unsupported: string | null } {
  const a = ep.request.auth
  switch (a.type) {
    case 'none':
      return { auth: null, unsupported: null }
    case 'bearer':
      return { auth: { kind: 'bearer', token: a.token }, unsupported: null }
    case 'basic':
      return { auth: { kind: 'basic', username: a.username, password: a.password }, unsupported: null }
    case 'apikey':
      if (a.in === 'header') return { auth: { kind: 'apikey_header', key: a.key, value: a.value }, unsupported: null }
      if (a.in === 'query') return { auth: { kind: 'apikey_query', key: a.key, value: a.value }, unsupported: null }
      return { auth: null, unsupported: `ApiKey（${a.in}）` }
    case 'oauth2':
      return { auth: null, unsupported: 'OAuth2' }
    case 'dynamic_signature':
      return { auth: null, unsupported: `${t('auth.dynamicSignature')} (DynamicSignature)` }
    case 'digest':
      return { auth: null, unsupported: 'HTTP Digest' }
    case 'hawk':
      return { auth: null, unsupported: 'Hawk' }
    case 'awsv4':
      return { auth: null, unsupported: 'AWS Signature V4' }
    case 'hmac':
      return { auth: null, unsupported: 'HMAC (AK-SK)' }
    default:
      return { auth: null, unsupported: t('auth.unknownType') }
  }
}

/** 编辑态 → 生成器输入（启用步骤；与发送路径同口径物化，见 ScriptStep 注释）。 */
function toScriptSteps(): ScriptStep[] {
  const out: ScriptStep[] = []
  for (const s of steps.value) {
    if (!s.enabled) continue
    const ep = endpointMap.value.get(s.endpointId)
    if (!ep) continue
    const body = ep.request.body
    const { auth, unsupported } = toScriptAuth(ep)
    let bodyKind: ScriptStep['bodyKind']
    let bodyContent: string | null = null
    let bodyFields: { key: string; value: string }[] = []
    let bodyFileFields: string[] = []
    let bodyVariables = ''
    let bodyPath = ''
    switch (body.mode) {
      case 'json':
        bodyKind = 'json'
        bodyContent = body.raw
        break
      case 'text':
        bodyKind = 'text'
        bodyContent = body.raw
        break
      case 'urlencoded':
        bodyKind = 'urlencoded'
        bodyFields = body.fields
          .filter((f) => f.enabled && f.key.trim())
          .map((f) => ({ key: f.key.trim(), value: f.value }))
        break
      case 'multipart': {
        bodyKind = 'multipart'
        for (const f of body.fields) {
          if (!f.enabled || !f.key.trim()) continue
          if (f.value_type === 'text') bodyFields.push({ key: f.key.trim(), value: f.value })
          else bodyFileFields.push(f.key.trim())
        }
        break
      }
      case 'graphql':
        bodyKind = 'graphql'
        bodyContent = body.spec.query
        bodyVariables = body.spec.variables ?? ''
        break
      case 'binary':
        bodyKind = 'binary'
        bodyPath = body.path
        break
      case 'grpc':
        bodyKind = 'grpc'
        break
      default:
        bodyKind = 'other'
        break
    }
    if (body.mode === 'none') bodyKind = 'none'
    out.push({
      name: ep.name,
      method: ep.method,
      url: urlTemplateFor(ep),
      params: ep.request.params
        .filter((p) => p.enabled)
        .filter((p) => p.key.trim())
        .map((p) => ({ key: p.key.trim(), value: p.value })),
      headers: ep.request.headers
        .filter((h) => h.enabled)
        .filter((h) => h.key.trim())
        .map((h) => ({ key: h.key.trim(), value: h.value })),
      auth,
      authUnsupported: unsupported,
      body: bodyContent,
      bodyFields,
      bodyFileFields,
      bodyVariables,
      bodyPath,
      bodyKind,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          // name 参与生成（check 具名 label）；曾遗漏导致同步往返后
          // 具名断言退化为自动 label、脚本被误判为已自定义。
          name: r.name ?? null,
          type: r.type,
          path: r.path?.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected),
        })),
    })
  }
  return out
}

/** 生成器可内嵌的非敏感变量默认值（敏感名一律环境变量注入，见 isSensitiveVarName）。 */
const scriptEnvDefaults = computed<Record<string, string>>(() => {
  const merged = mergedVariableMap({
    globalVariables: store.globalVariables,
    projectVariables: store.project?.variables,
    env: store.environments.find((e) => e.id === store.activeEnvId),
  })
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(merged)) {
    if (!isSensitiveVarName(k)) out[k] = v
  }
  return out
})

const generatedScript = computed(() =>
  generateScenarioScript(name.value.trim() || t('scenario.untitled'), toScriptSteps(), {
    base: scriptBase.value,
    stopOnFailure: stopOnFailure.value,
    envDefaults: scriptEnvDefaults.value,
  }),
)

/** 脚本编辑态：未自定义时实时跟随生成版；一旦编辑即以脚本为准（可重置）。 */
const scriptDraft = ref('')
const scriptCustomized = ref(false)

// 手动步骤变化时，未自定义的脚本跟随重新生成。
watch(generatedScript, (g) => {
  if (!scriptCustomized.value) scriptDraft.value = g
})

function onScriptInput(v: string): void {
  scriptDraft.value = v
  // 手改回与生成版一致时自动解除自定义标记。
  scriptCustomized.value = v !== generatedScript.value
}

function resetScript(): void {
  scriptCustomized.value = false
  scriptDraft.value = generatedScript.value
}

// ---------- 脚本 → 编排联动（解析回写 + 按脚本运行） ----------
/**
 * 同步边界：编排步骤是 endpoint_id 引用式，URL / 请求头 / Body 的真源在接口，
 * 同步只回写步骤自有字段（提取 / 断言 / 步骤增删）；脚本步骤与接口物化 URL
 * 的差异以警示呈现（接口层改动需到接口编辑器修正）。
 */
interface ScriptSyncPlan {
  steps: ParsedScriptStep[]
  /** 位置对齐的编辑态启用步骤（前 min(N, M) 个）。 */
  targets: EditorStep[]
  /** 脚本比编排多出的步骤：按 方法 + URL 模板 匹配到的接口 id。 */
  appended: { parsed: ParsedScriptStep; endpointId: string }[]
  /** URL 漂移警示（步骤序号 1-based）。 */
  drifts: string[]
  /** 编排启用步骤多于脚本的剩余数（保持原样）。 */
  remaining: number
  /** 阻断同步的问题（全部解决才可应用）。 */
  blockers: string[]
}

function analyzeScriptSync(): ScriptSyncPlan {
  const { steps: parsedSteps } = parseScenarioScript(scriptDraft.value)
  const enabledSteps = steps.value.filter((s) => s.enabled)
  const targets = enabledSteps.slice(0, parsedSteps.length)
  const appended: ScriptSyncPlan['appended'] = []
  const drifts: string[] = []
  const blockers: string[] = []

  for (const [i, target] of targets.entries()) {
    const parsed = parsedSteps[i]
    const ep = endpointMap.value.get(target.endpointId)
    if (ep && urlTemplateFor(ep) !== parsed.url) {
      drifts.push(t('scenario.syncDriftStep', { n: i + 1, name: ep.name }))
    }
  }
  for (const parsed of parsedSteps.slice(enabledSteps.length)) {
    const ep = store.endpoints.find(
      (e) => e.method === parsed.method && urlTemplateFor(e) === parsed.url,
    )
    if (!ep) {
      blockers.push(t('scenario.syncNoMatch', { name: parsed.name }))
      continue
    }
    appended.push({ parsed, endpointId: ep.id })
  }
  return {
    steps: parsedSteps,
    targets,
    appended,
    drifts,
    remaining: Math.max(0, enabledSteps.length - parsedSteps.length),
    blockers,
  }
}

/** 编辑态断言行形状（与 decodeStep 一致：期望值统一字符串）。 */
function toEditorAssertions(rules: ScenarioAssertionRule[]): EditorStep['assertions'] {
  return rules.map((r) => ({
    name: r.name ?? null,
    type: r.type,
    path: r.path ?? '',
    op: r.op ?? 'eq',
    expected: r.expected == null ? '' : String(r.expected),
  }))
}

function applyScriptSync(plan: ScriptSyncPlan): void {
  for (const [i, target] of plan.targets.entries()) {
    const parsed = plan.steps[i]
    target.extract = parsed.extract.map((r) => ({ ...r }))
    target.assertions = toEditorAssertions(parsed.assertions)
  }
  for (const add of plan.appended) {
    steps.value.push({
      key: nextStepKey(),
      endpointId: add.endpointId,
      enabled: true,
      extract: add.parsed.extract.map((r) => ({ ...r })),
      assertions: toEditorAssertions(add.parsed.assertions),
      open: false,
    })
  }
  // 同步后重算自定义标记：再生成本与脚本一致（往返无损）即解除自定义态
  scriptCustomized.value = scriptDraft.value !== generatedScript.value
}

/** 同步到编排（Popconfirm 确认后）。全部解析通过才应用（all-or-nothing）。 */
function onSyncConfirm(): void {
  // 空场景（编排无启用步骤且脚本未自定义）：没有可同步的内容，友好提示而非解析报错
  if (!steps.value.some((s) => s.enabled) && !scriptCustomized.value) {
    toast.info(t('scenario.syncEmpty'))
    return
  }
  let plan: ScriptSyncPlan
  try {
    plan = analyzeScriptSync()
  } catch (e) {
    toast.error(e instanceof ScriptParseError ? e.message : String(e))
    return
  }
  if (plan.blockers.length) {
    toast.error(plan.blockers.join('；'))
    return
  }
  applyScriptSync(plan)
  toast.success(t('scenario.syncDone', { n: plan.steps.length }))
  if (plan.drifts.length) {
    toast.warning(plan.drifts.slice(0, 3).join('；'), { duration: 0 })
  }
  if (plan.remaining) {
    toast.warning(t('scenario.syncRemaining', { n: plan.remaining }), { duration: 0 })
  }
  viewMode.value = 'manual'
}

/** 脚本视图「运行」：自定义脚本先同步回编排（失败中止），再走现有运行引擎。 */
async function runFromScript(): Promise<void> {
  if (running.value) return
  if (scriptCustomized.value) {
    let plan: ScriptSyncPlan
    try {
      plan = analyzeScriptSync()
    } catch (e) {
      toast.error(e instanceof ScriptParseError ? e.message : String(e))
      return
    }
    if (plan.blockers.length) {
      toast.error(plan.blockers.join('；'))
      return
    }
    applyScriptSync(plan)
    if (plan.drifts.length) {
      toast.warning(plan.drifts.slice(0, 3).join('；'), { duration: 0 })
    }
  }
  viewMode.value = 'manual'
  await run()
}

async function copyScript(): Promise<void> {
  try {
    await copyText(scriptDraft.value)
    toast.success(t('scenario.scriptCopied'))
  } catch {
    toast.error(t('scenario.scriptCopyFail'))
  }
}

// ---------- 导出 .mjs（复用环境的导出链路：目录选择 + save_text_file） ----------
const exporting = ref(false)

async function exportScript(): Promise<void> {
  if (exporting.value) return
  exporting.value = true
  try {
    const dir = await openDialog({ directory: true, title: t('scenario.scriptExportTitle') })
    if (!dir || Array.isArray(dir)) return
    const base =
      (name.value.trim() || t('scenario.untitled')).replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await join(dir, `${base}.mjs`)
    await api.writeTextFile(path, scriptDraft.value)
    toast.success(t('scenario.scriptExported'), { message: path })
  } catch (err) {
    toast.error(t('scenario.scriptExportFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    exporting.value = false
  }
}

// ---------- 定位步骤（结果行 / 变量链路 → 步骤卡） ----------
/** 选中对应步骤卡（右栏详情随之展示）并滚动到卡片。 */
function locateStep(stepIdx: number | null): void {
  if (stepIdx == null) return
  const s = steps.value[stepIdx]
  if (!s) return
  viewMode.value = 'manual'
  selectStep(s)
  void nextTick(() => {
    const root = rootEl.value
    if (!root) return
    const el = root.querySelector(`[data-step-key="${CSS.escape(s.key)}"]`)
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    el?.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'center' })
  })
}

defineExpose({ locateStep })

// ---------- 只读请求分区（右栏详情；配置真源在接口编辑器） ----------
function enabledHeaders(i: number): { key: string; value: string }[] {
  const ep = stepEndpoint(i)
  if (!ep) return []
  return ep.request.headers.filter((h) => h.enabled && h.key.trim()).map((h) => ({ key: h.key.trim(), value: h.value }))
}

function enabledParams(i: number): { key: string; value: string }[] {
  const ep = stepEndpoint(i)
  if (!ep) return []
  return ep.request.params
    .filter((p) => p.enabled && p.key.trim())
    .map((p) => ({ key: p.key.trim(), value: p.value }))
}

function enabledPathVars(i: number): { key: string; value: string }[] {
  const ep = stepEndpoint(i)
  if (!ep) return []
  return ep.request.path_variables
    .filter((p) => p.enabled && p.key.trim())
    .map((p) => ({ key: p.key.trim(), value: p.value }))
}

function stepUrl(i: number): string {
  const ep = stepEndpoint(i)
  return ep ? urlTemplateFor(ep) : ''
}

/** 实际发送的请求体文本：JSON 原样（含 {{var}} 模板），表单按字段拼接。 */
function bodyText(i: number): string {
  const ep = stepEndpoint(i)
  if (!ep) return ''
  const body = ep.request.body
  switch (body.mode) {
    case 'json':
    case 'text':
      return body.raw.trim()
    case 'urlencoded':
      return body.fields
        .filter((f) => f.enabled && f.key.trim())
        .map((f) => `${f.key.trim()}=${f.value}`)
        .join('&')
    case 'multipart':
      return body.fields
        .filter((f) => f.enabled && f.key.trim())
        .map((f) => `${f.key.trim()}=${f.value}`)
        .join('&')
    default:
      return ''
  }
}

// ---------- 场景切换加载：放在全部状态声明之后——immediate 回调在 setup 期间同步
// 执行，若提前会踩到下方 runResult 等的暂时性死区（WebKit：Cannot access
// uninitialized variable，场景页签一打开即崩）。
watch(
  () => props.scenarioId,
  () => {
    loading.value = true
    const s = scenario.value
    const cached = draftCache.get(props.scenarioId)
    if (s && cached) {
      // 页签切回：恢复暂存的编辑态与运行结果。
      name.value = cached.name
      description.value = cached.description
      steps.value = cached.steps
      savedSnapshot.value = cached.savedSnapshot
      scriptCustomized.value = cached.scriptCustomized
      // 未自定义的脚本始终跟随当前生成版：暂存期间接口配置可能已在别处更新。
      scriptDraft.value = cached.scriptCustomized ? cached.scriptDraft : generatedScript.value
      viewMode.value = cached.viewMode
      stopOnFailure.value = cached.stopOnFailure
      runResult.value = cached.runResult
      runStepIndexes.value = cached.runStepIndexes
      runIsHistory.value = cached.runIsHistory
      currentRunAt.value = cached.currentRunAt
      varsSnapshots.value = cached.varsSnapshots
    } else {
      draftCache.delete(props.scenarioId)
      name.value = s?.name ?? ''
      description.value = s?.description ?? ''
      steps.value = (s?.steps ?? []).map(decodeStep)
      scriptDraft.value = s?.script ?? generatedScript.value
      scriptCustomized.value = !!s?.script
      savedSnapshot.value = snapshot()
      runResult.value = null
      runStepIndexes.value = []
      runIsHistory.value = false
      currentRunAt.value = null
      varsSnapshots.value = []
      viewMode.value = 'manual'
    }
    progress.value = { done: 0, total: 0 }
    respOpenRows.value = new Set()
    drawerOpen.value = false
    loading.value = false
    // 历史菜单数据总是刷新；重开（无缓存）时恢复最近一次运行结果。
    const scenarioIdAtLoad = props.scenarioId
    void loadHistory().then(() => {
      if (!cached) void restoreLatestRun(scenarioIdAtLoad)
    })
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  stashDraft()
  narrowMql.removeEventListener('change', onNarrowChange)
})

let unlistenProgress: UnlistenFn | null = null
onMounted(async () => {
  // 后端每步发两次事件：进入本步（url 有值、result 缺省）、本步结束（result 有值）。
  // 结果在 composable 内过滤 runId 并累积，面板据 liveRows 边跑边渲染。
  unlistenProgress = await listen<ScenarioProgressEvent>('miyo:scenario-progress', (e) => {
    applyProgressEvent(e.payload)
  })
  window.addEventListener('keydown', onEscKeydown)
})
onBeforeUnmount(() => {
  sortable?.destroy()
  sortable = null
  unlistenProgress?.()
  unlistenProgress = null
  window.removeEventListener('keydown', onEscKeydown)
})

const rootEl = ref<HTMLElement | null>(null)
const enabledCount = computed(() => steps.value.filter((s) => s.enabled).length)
</script>

<template>
  <div v-if="!scenario && !loading" class="se-missing">
    <EmptyState icon="beaker" :title="t('scenario.missing')" compact />
  </div>
  <div v-else ref="rootEl" class="scenario-editor">
    <!-- ===== 顶部工具栏：标题 / 保存状态 / 视图切换 / 执行策略 / 保存 / 运行 ===== -->
    <header class="se-toolbar">
      <div class="se-head-main">
        <div class="se-title-row">
          <span class="se-kind">{{ t('scenario.kind') }}</span>
          <input v-model="name" class="se-name" :placeholder="t('scenario.namePh')" spellcheck="false" />
          <span v-if="dirty" class="se-unsaved" :title="t('scenario.unsaved')">
            <span class="se-dirty" aria-hidden="true"></span>{{ t('scenario.unsaved') }}
          </span>
          <span v-else-if="saving" class="se-savedtxt">{{ t('scenario.saving') }}</span>
          <span v-else class="se-savedtxt"><Icon name="check" :size="11" />{{ t('scenario.saved') }}</span>
        </div>
        <div class="se-meta-row">
          <span v-if="store.project" class="se-meta-item" :title="store.project.name">
            <Icon name="folder" :size="12" />{{ store.project.name }}
          </span>
          <EnvBadge class="se-meta-env" />
          <span class="se-meta-item">
            <Icon name="list" :size="12" />
            {{ t('scenario.stepCount', { n: steps.length }) }}
            <template v-if="enabledCount !== steps.length">
              · {{ t('scenario.enabledCount', { n: enabledCount }) }}
            </template>
          </span>
          <input v-model="description" class="se-desc" :placeholder="t('scenario.descPh')" spellcheck="false" />
        </div>
      </div>
      <div class="se-actions">
        <SegmentedControl
          v-model="viewMode"
          :options="VIEW_MODES"
          size="sm"
          :aria-label="t('scenario.viewMode')"
        />
        <span
          class="se-strategy"
          :title="stopOnFailure ? t('scenario.stopOnFailureHint') : t('scenario.strategyHintContinue')"
        >
          <CustomSelect
            v-model="execStrategy"
            :options="STRATEGY_OPTIONS"
            size="sm"
            class="se-strategy-select"
            :disabled="running"
            :aria-label="t('scenario.strategyLabel')"
          />
        </span>
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="saving || !dirty || running"
          @click="save"
        >
          <Icon name="save" :size="13" /> {{ t('common.save') }}
        </button>
        <button
          v-if="viewMode === 'manual' && !running"
          class="miyo-btn miyo-btn-sm se-run"
          type="button"
          @click="run"
        >
          <Icon name="play" :size="13" /> {{ t('scenario.run') }}
        </button>
        <button
          v-else-if="viewMode === 'manual'"
          class="miyo-btn miyo-btn-sm se-stop"
          type="button"
          @click="stop"
        >
          <Icon name="stop" :size="13" /> {{ t('scenario.stop') }}
        </button>
      </div>
    </header>

    <!-- ===== 中列编排 + 右栏详情 ===== -->
    <div v-if="viewMode === 'manual'" class="se-layout">
      <section class="se-flow">
        <!-- 变量链路（默认折叠为总览行；来源/引用从启用步骤配置推导） -->
        <ScenarioVarsPanel
          :chain="varChain"
          :runtime-vars="runResult?.variables ?? null"
          @locate="locateStep"
        />

        <!-- 步骤列表（紧凑卡片；纵向轨道） -->
        <div v-if="steps.length" ref="stepsEl" class="se-steps">
          <div
            v-for="(s, i) in steps"
            :key="s.key"
            class="se-step"
            :class="{ off: !s.enabled, fail: stepTone(i) === 'fail' }"
            :data-step-key="s.key"
          >
            <div class="se-rail">
              <span class="se-step-idx" :class="`tone-${stepTone(i) ?? 'none'}`">{{ i + 1 }}</span>
              <span v-if="i < steps.length - 1" class="se-rail-line" aria-hidden="true"></span>
            </div>
            <div
              class="se-card"
              :class="{ selected: s.open }"
              role="button"
              tabindex="0"
              :aria-label="t('scenario.stepSelect')"
              @click="selectStep(s)"
              @keydown.enter="selectStep(s)"
            >
              <div class="se-card-head">
                <span class="se-drag" :title="t('scenario.dragSort')" @click.stop>
                  <Icon name="list" :size="13" />
                </span>
                <input
                  v-model="s.enabled"
                  type="checkbox"
                  class="se-step-check"
                  :title="t('scenario.stepEnable')"
                  @click.stop
                />
                <span
                  v-if="stepEndpoint(i)"
                  class="se-method"
                  :class="methodBadgeTone(stepEndpoint(i)!.method)"
                >{{ stepEndpoint(i)!.method }}</span>
                <CustomSelect
                  v-model="s.endpointId"
                  :options="endpointOptions"
                  size="sm"
                  :placeholder="t('scenario.pickEndpoint')"
                  class="se-step-ep"
                  :disabled="running"
                  @click.stop
                />
                <span v-if="s.endpointId && !endpointMap.has(s.endpointId)" class="se-step-missing">
                  {{ t('scenario.endpointMissing') }}
                </span>
                <span v-if="stepTone(i)" class="se-step-status" :class="`tone-${stepTone(i)}`">
                  <Icon :name="STEP_TONE_ICON[stepTone(i)!]" :size="11" />
                  {{ t(STEP_TONE_TEXT[stepTone(i)!]) }}
                </span>
                <div class="se-step-tools" @click.stop>
                  <IconButton
                    name="arrow-up-right"
                    :size="13"
                    :title="t('scenario.locateStep')"
                    @click="locateStep(i)"
                  />
                  <IconButton
                    name="more-horizontal"
                    :size="13"
                    :title="t('scenario.moreActions')"
                    @click.stop="openStepMenu($event, i)"
                  />
                </div>
              </div>
              <div class="se-card-sub">
                <code v-if="stepUrl(i)" class="se-step-url" :title="stepUrl(i)">{{ stepUrl(i) }}</code>
                <span class="se-step-badges">
                  <span v-if="stepOutputVars(i).length" class="se-badge" :title="t('scenario.detailTabExtract')">
                    {{ t('scenario.stepExtractBadge', { n: stepOutputVars(i).length }) }}
                  </span>
                  <span v-if="s.assertions.length" class="se-badge" :title="t('scenario.detailTabAssert')">
                    {{ t('scenario.stepAssertBadge', { n: s.assertions.length }) }}
                  </span>
                  <span v-if="stepDurationMs(i) != null" class="se-badge time" :title="t('scenario.stepDuration')">
                    {{ formatDurationShort(stepDurationMs(i)!) }}
                  </span>
                </span>
              </div>
              <!-- 变量出入参 chips：一眼看清这一步的数据流向（详情在右栏） -->
              <div v-if="stepRefs(i).length || stepOutputVars(i).length" class="se-card-vars">
                <span v-if="stepRefs(i).length" class="se-step-vars">
                  <code
                    v-for="n in stepRefs(i)"
                    :key="`in-${n}`"
                    class="se-var-chip mini"
                    :class="{ warn: isMissingAt(i, n) }"
                    :title="refTitle(i, n)"
                  >{{ varTag(n) }}<span v-if="!isMissingAt(i, n) && varSourceIndex.get(n) != null" class="se-chip-from">← {{ t('scenario.usesFrom', { n: varSourceIndex.get(n)! + 1 }) }}</span></code>
                </span>
                <span v-if="stepOutputVars(i).length" class="se-step-vars">
                  <Icon name="download" :size="10" class="se-var-io" />
                  <code
                    v-for="n in stepOutputVars(i).slice(0, 3)"
                    :key="`out-${n}`"
                    class="se-var-chip mini out"
                    :title="varTag(n)"
                  >{{ varTag(n) }}</code>
                  <span v-if="stepOutputVars(i).length > 3" class="se-var-more">
                    +{{ stepOutputVars(i).length - 3 }}
                  </span>
                </span>
              </div>
            </div>
          </div>
        </div>

        <div v-else class="se-no-steps">
          <EmptyState icon="beaker" :title="t('scenario.noStepsTitle')" :description="t('scenario.noStepsDesc')" compact>
            <button class="miyo-btn miyo-btn-sm" type="button" :disabled="running" @click="addStep">
              <Icon name="plus" :size="13" /> {{ t('scenario.addStep') }}
            </button>
          </EmptyState>
        </div>

        <button v-if="steps.length" class="se-add-step" type="button" :disabled="running" @click="addStep">
          <Icon name="plus" :size="13" /> {{ t('scenario.addStep') }}
        </button>

        <!-- ===== 底部结果 Dock（可折叠；支持历史回看 / 重跑失败 / 定位） ===== -->
        <ScenarioRunReport
          v-if="running || runResult"
          v-model:open="reportOpen"
          :running="running"
          :progress="progress"
          :progress-percent="progressPercent"
          :run-result="runResult"
          :run-is-history="runIsHistory"
          :current-run-at="currentRunAt"
          :live-rows="liveRows"
          :live-pending="livePending"
          :resp-open-rows="respOpenRows"
          :run-summary="runSummary"
          :final-state="finalState"
          :rerun-available="rerunAvailable"
          :has-history="runHistory.length > 0"
          :step-name="stepName"
          :step-method="stepMethodName"
          :full-body-loader="fullBodyLoader"
          @locate="onReportLocate"
          @rerun="rerunFailed"
          @history="openHistoryMenu"
          @toggle-row="toggleRespRow"
        />
        <p v-else-if="steps.length" class="se-report-empty">{{ t('scenario.resultEmpty') }}</p>
      </section>

      <!-- ===== 右栏：步骤详情（宽屏内联；窄屏抽屉叠加，Esc 关闭） ===== -->
      <aside v-if="!isNarrow || drawerOpen" class="se-detail" :class="{ overlay: isNarrow }">
        <ScenarioStepDetail
          :step="selectedStep"
          :index="selectedIndex + 1"
          :endpoint="selectedEndpoint"
          :url="selectedUrl"
          :path-vars="selectedPathVars"
          :params="selectedParams"
          :headers="selectedHeaders"
          :body-kind="selectedBodyKind"
          :body-text="selectedBodyText"
          :status-tone="selectedTone"
          :status-text="selectedStatusText"
          :duration-ms="selectedDuration"
          @edit-endpoint="editSelectedEndpoint"
          @close="closeDetail"
        />
      </aside>
    </div>

    <!-- ===== 脚本视图：由当前配置实时生成；可编辑 / 复制 / 导出 .mjs ===== -->
    <template v-else>
      <div class="se-script-bar">
        <div class="se-script-info">
          <span class="se-script-lang"><Icon name="code" :size="11" /> {{ t('scenario.scriptLang') }}</span>
          <span class="se-script-hint">
            {{ scriptCustomized ? t('scenario.scriptCustomized') : t('scenario.scriptGenerated') }}
          </span>
        </div>
        <div class="se-script-actions">
          <Popconfirm
            :title="t('scenario.syncConfirm')"
            :description="t('scenario.syncConfirmDesc')"
            :confirm-text="t('common.confirm')"
            :cancel-text="t('common.cancel')"
            @confirm="onSyncConfirm"
          >
            <button
              class="miyo-btn miyo-btn-sm"
              type="button"
              :disabled="running"
            >
              <Icon name="arrow-up-right" :size="13" /> {{ t('scenario.syncToSteps') }}
            </button>
          </Popconfirm>
          <button
            class="miyo-btn miyo-btn-sm se-run"
            type="button"
            :disabled="running"
            @click="runFromScript"
          >
            <Icon name="play" :size="13" /> {{ t('scenario.run') }}
          </button>
          <button
            v-if="scriptCustomized"
            class="miyo-btn miyo-btn-sm"
            type="button"
            @click="resetScript"
          >
            <Icon name="refresh" :size="13" /> {{ t('scenario.scriptReset') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="copyScript">
            <Icon name="copy" :size="13" /> {{ t('common.copyAction') }}
          </button>
          <button
            class="miyo-btn miyo-btn-sm"
            type="button"
            :disabled="exporting"
            @click="exportScript"
          >
            <Icon name="download" :size="13" /> {{ t('scenario.scriptExport') }}
          </button>
        </div>
      </div>
      <JsonCodeMirror
        :model-value="scriptDraft"
        language="javascript"
        class="se-script"
        @update:model-value="onScriptInput"
      />
    </template>

    <Menu ref="stepMenu" @select="onStepMenuSelect" @confirm="onStepMenuConfirm" />
    <Menu ref="historyMenu" @select="onHistorySelect" />
  </div>
</template>

<style scoped>
.scenario-editor {
  position: relative;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  padding: 10px 14px 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.se-missing {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* ---- 顶部工具栏 ---- */
.se-toolbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 14px;
}

.se-head-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.se-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.se-kind {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 9px;
  border-radius: 999px;
  background: var(--accent-tint);
  color: var(--accent);
  font-size: var(--fs-xxs);
  font-weight: 600;
}

.se-name {
  flex: 1;
  min-width: 0;
  border: none;
  border-bottom: 1px dashed transparent;
  border-radius: 0;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-md);
  font-weight: 600;
  outline: none;
  padding: 1px 2px;
}

.se-name:hover {
  border-bottom-color: var(--text-3);
}

.se-unsaved {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  white-space: nowrap;
}

.se-dirty {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--warning);
}

.se-savedtxt {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

.se-meta-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px 12px;
  min-width: 0;
}

.se-meta-item {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-meta-env {
  font-size: var(--fs-xxs);
  padding: 1px 7px;
}

.se-desc {
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  outline: none;
  padding: 0;
  flex: 1;
  min-width: 120px;
}

.se-desc:focus {
  border-bottom: 1px solid var(--border);
}

.se-actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
}

.se-run {
  background: var(--accent, #8b5cf6);
  color: #fff;
}

.se-run:hover:not(:disabled) {
  background: var(--accent-hover, var(--accent));
  color: #fff;
}

.se-stop {
  border-color: var(--danger-border, var(--danger));
  color: var(--danger);
}

.se-strategy {
  display: inline-flex;
  align-items: center;
  cursor: default;
}

.se-strategy-select {
  width: 132px;
}

/* ---- 三栏布局：中列编排 + 右栏详情 ---- */
.se-layout {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 10px;
}

.se-flow {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.se-detail {
  flex: none;
  width: 400px;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  overflow: hidden;
}

/* 窄屏抽屉：叠加在编排区右侧（Esc / 关闭按钮退出） */
.se-detail.overlay {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  z-index: 30;
  box-shadow: var(--shadow-drawer);
}

@media (max-width: 1359px) {
  .se-detail {
    width: min(420px, 92vw);
  }
}

/* ---- 步骤列表（纵向轨道 + 卡片） ---- */
.se-steps {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  /* 滚动条不遮拖拽手柄 */
  scrollbar-gutter: stable;
}

.se-step {
  display: flex;
  gap: 10px;
}

.se-rail {
  flex: none;
  width: 22px;
  display: flex;
  flex-direction: column;
  align-items: center;
}

.se-step-idx {
  flex: none;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--bg-hover);
  border: 1px solid var(--border);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-weight: 600;
  margin-top: 8px;
}

.se-step-idx.tone-ok {
  background: var(--success-tint);
  border-color: transparent;
  color: var(--success);
}

.se-step-idx.tone-fail {
  background: var(--danger-tint);
  border-color: transparent;
  color: var(--danger);
}

.se-step-idx.tone-skip {
  opacity: 0.55;
}

.se-step-idx.tone-cancel {
  background: var(--warning-tint);
  border-color: transparent;
  color: var(--warning);
  opacity: 0.8;
}

.se-step-idx.tone-run {
  background: var(--accent-tint);
  border-color: transparent;
  color: var(--accent);
  animation: se-pulse 1.6s var(--ease) infinite;
}

@keyframes se-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.6;
  }
}

@media (prefers-reduced-motion: reduce) {
  .se-step-idx.tone-run {
    animation: none;
  }
}

.se-rail-line {
  flex: 1;
  min-height: 14px;
  width: 1px;
  background: var(--border);
  margin: 4px 0;
}

.se-card {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  margin: 5px 0;
  overflow: hidden;
  cursor: pointer;
  transition: border-color var(--dur) var(--ease);
}

.se-card:hover {
  border-color: var(--border-strong);
}

/* 选中态：accent 边框 + 序号同步高亮 */
.se-card.selected {
  border-color: var(--accent);
}

.se-step:last-child .se-card {
  margin-bottom: 0;
}

.se-step:first-child .se-card {
  margin-top: 0;
}

.se-step.off .se-card {
  opacity: 0.55;
}

.se-step.fail .se-card {
  border-color: var(--danger-border, var(--danger));
}

.se-step.fail .se-card.selected {
  border-color: var(--danger);
}

.se-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
}

.se-drag {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 24px;
  color: var(--text-3);
  cursor: grab;
  border-radius: var(--radius-sm);
}

.se-drag:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.se-drag:active {
  cursor: grabbing;
}

.se-ghost {
  opacity: 0.45;
}

.se-chosen {
  box-shadow: 0 6px 18px rgb(0 0 0 / 0.25);
}

.se-step-check {
  flex: none;
}

.se-step-ep {
  flex: 0 1 240px;
  min-width: 140px;
}

.se-step-missing {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--danger);
}

.se-step-status {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  padding: 1px 8px;
  border-radius: 999px;
  border: 1px solid transparent;
}

.se-step-status.tone-ok,
.se-step-status.tone-done {
  background: var(--success-tint);
  color: var(--success);
}

.se-step-status.tone-fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.se-step-status.tone-skip {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-step-status.tone-run {
  background: var(--accent-tint);
  color: var(--accent);
}

.se-step-status.tone-wait {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-step-status.tone-cancel {
  background: var(--warning-tint);
  color: var(--warning);
}

.se-step-tools {
  display: flex;
  gap: 2px;
  margin-left: auto;
}

/* ---- 紧凑摘要行：URL + 提取/断言/耗时徽章 ---- */
.se-card-sub {
  display: flex;
  align-items: center;
  gap: 4px 10px;
  padding: 0 10px 7px 36px;
  min-height: 18px;
}

.se-step-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-step-badges {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.se-badge {
  flex: none;
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
  white-space: nowrap;
}

.se-badge.time {
  font-family: var(--font-mono);
}

/* ---- 变量出入参 chips 行 ---- */
.se-card-vars {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 12px;
  padding: 0 10px 8px 36px;
}

.se-step-vars {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px;
}

.se-var-io {
  color: var(--text-3);
}

/* ---- 添加步骤 / 空状态 ---- */
.se-add-step {
  flex: none;
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px dashed var(--border);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  padding: 6px 12px;
  border-radius: var(--radius-md);
  cursor: pointer;
}

.se-add-step:hover:not(:disabled) {
  color: var(--text-1);
  border-color: var(--text-3);
}

.se-add-step:disabled {
  opacity: 0.45;
  cursor: default;
}

.se-no-steps {
  flex: none;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  padding: 18px 0;
}

/* ---- 结果 Dock 之外的操作提示 ---- */
.se-report-empty {
  flex: none;
  margin: 0;
  padding: 8px 2px;
  border-top: 1px dashed var(--border);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 脚本视图 ---- */
.se-script-bar {
  flex: none;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.se-script-info {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 10px;
  min-width: 0;
}

.se-script-lang {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-tint);
  padding: 2px 8px;
  border-radius: 999px;
  white-space: nowrap;
}

.se-script-hint {
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}

.se-script {
  flex: 1;
  min-height: 0;
}

.se-script-actions {
  display: flex;
  gap: 8px;
  flex: none;
}
</style>
