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
 * 结构（自上而下）：
 * - 头部：场景名 + 描述 + 元信息（项目 / 环境 / 步骤数 / 保存状态）+
 *   视图切换 + 执行策略 + 保存 / 运行；
 * - 变量链路（可折叠）：从启用步骤的真实配置推导「来源 → 引用」，未定义
 *   变量警示；运行后可查看运行时值（敏感名按 miyo-sync 同款词根脱敏）；
 * - 手动编排：纵向步骤卡片（轨道序号 + 连接线），拖拽 / 上移下移 / 复制 /
 *   删除（菜单内确认）/ 启用禁用；折叠态展示方法、URL、变量摘要与执行
 *   状态，展开态为只读请求摘要 + 提取 / 断言编辑；
 * - 执行结果面板（可折叠）：摘要统计 + 每步结果，点击行定位到对应步骤卡；
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
import { formatDateTime } from '../utils/dateTime'
import { methodBadgeTone } from '../utils/methodTone'
import {
  computeVarChain,
  extractVarRefs,
  isSensitiveVarName,
  type VarChainEntry,
  type VarScanStep,
} from '../utils/scenarioVars'
import { generateScenarioScript, type ScriptStep } from '../utils/scenarioScript'
import CustomSelect from './ui/CustomSelect.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import ResponsePanel from './ResponsePanel.vue'
import SegmentedControl, { type SegmentOption } from './ui/SegmentedControl.vue'
import EmptyState from './ui/EmptyState.vue'
import EnvBadge from './ui/EnvBadge.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
// ScenarioAssertionRule / ScenarioExtractRule / ScenarioRunResult 等类型
// 由模块级 <script> 块导入（两块脚本同属一个模块作用域）。
import type {
  Endpoint,
  Scenario,
  ScenarioRunDetail,
  ScenarioRunSummary,
  ScenarioStep,
  ScenarioStepResult,
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
// / 运行结果 / 展开状态按 scenario id 暂存模块级缓存，切回时恢复（保存后
// 仍保留，以维持展开态与运行结果；场景删除时随加载失败自然失效）。

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

const EXTRACT_FROM_OPTIONS = computed(() => [
  { value: 'body', label: t('scenario.fromBody') },
  { value: 'header', label: t('scenario.fromHeader') },
])
const ASSERT_TYPE_OPTIONS = computed(() => [
  { value: 'status', label: t('scenario.assertStatus') },
  { value: 'jsonpath', label: t('scenario.assertJsonpath') },
  { value: 'header', label: t('scenario.assertHeader') },
  { value: 'response_time_ms', label: t('scenario.assertTime') },
])
const ASSERT_OP_OPTIONS = computed(() =>
  (
    ['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'exists', 'not_exists', 'empty', 'not_empty'] as const
  ).map((op) => ({ value: op, label: t(`scenario.op_${op}`) })),
)
const NO_EXPECTED_OPS = new Set(['exists', 'not_exists', 'empty', 'not_empty'])

function addStep(): void {
  if (running.value) return
  const first = store.endpoints[0]
  steps.value.push({
    key: nextStepKey(),
    endpointId: first?.id ?? '',
    enabled: true,
    extract: [],
    assertions: [],
    open: steps.value.length === 0,
  })
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
const missingVarCount = computed(() => varChain.value.filter((v) => v.missing).length)

const varsExpanded = ref(true)
const expandedVar = ref<string | null>(null)
const revealedVars = ref(new Set<string>())

function toggleVarDetail(name_: string): void {
  expandedVar.value = expandedVar.value === name_ ? null : name_
}

/** 变量名 → 最早定义步骤（同步骤先引用后提取在运行时取不到值）。 */
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

/** 某步骤引用的变量在该步执行时是否有值（更早步骤定义过，或来自外部变量池）。 */
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

function varTag(n: string): string {
  return `{{${n}}}`
}

/** 链路条目的一行摘要：来源步骤 + 引用数。 */
function varBrief(v: VarChainEntry): string {
  const usePart = t('scenario.varUsageCount', { n: v.usages.length })
  if (v.sources.length) {
    const src = v.sources[0]!
    return `${t('scenario.stepRef', { n: src.stepIndex + 1, name: src.stepName })} · ${usePart}`
  }
  if (v.external) return `${t('scenario.varExternal')} · ${usePart}`
  return usePart
}

/** 运行时值（后端最终变量池）；敏感名默认掩码，眼睛切换。 */
function runtimeValueOf(varName: string): string | null {
  const vars = runResult.value?.variables
  if (!vars || !(varName in vars)) return null
  return vars[varName] ?? ''
}

function displayRuntimeValue(varName: string): string {
  const value = runtimeValueOf(varName)
  if (value == null) return ''
  if (isSensitiveVarName(varName) && !revealedVars.value.has(varName)) return '••••••'
  return value
}

function toggleReveal(varName: string): void {
  const next = new Set(revealedVars.value)
  if (next.has(varName)) next.delete(varName)
  else next.add(varName)
  revealedVars.value = next
}

// ---------- 运行状态 ----------
// 必须先于视图模式区声明：generatedScript computed 与其 watch 都读 stopOnFailure。
const stopOnFailure = ref(true)
const running = ref(false)
const runId = ref('')
const progress = ref({ done: 0, total: 0 })
const runResult = ref<ScenarioRunResult | null>(null)
/** 本次运行实际执行的步骤（编辑态索引，按运行顺序）；结果按位对应。 */
const runStepIndexes = ref<number[]>([])

// 拖拽容器挂载 / 运行态切换后同步 Sortable（disabled 声明在 running 之后，避开 TDZ）。
watch(stepsEl, () => nextTick(initSortable))
watch(running, () => sortable?.option('disabled', running.value))

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

/** 编辑态 → 生成器输入（启用步骤；Body 只展开 json / text，其余注明以 Miyo 运行为准）。 */
function toScriptSteps(): ScriptStep[] {
  const out: ScriptStep[] = []
  for (const s of steps.value) {
    if (!s.enabled) continue
    const ep = endpointMap.value.get(s.endpointId)
    if (!ep) continue
    const bodyKind =
      ep.request.body.mode === 'json'
        ? ('json' as const)
        : ep.request.body.mode === 'text'
          ? ('text' as const)
          : ('other' as const)
    out.push({
      name: ep.name,
      method: ep.method,
      url: urlTemplateFor(ep),
      headers: ep.request.headers
        .filter((h) => h.enabled)
        .filter((h) => h.key.trim())
        .map((h) => ({ key: h.key.trim(), value: h.value })),
      body:
        ep.request.body.mode === 'json' || ep.request.body.mode === 'text'
          ? ep.request.body.raw
          : null,
      bodyKind,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          type: r.type,
          path: r.path?.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected),
        })),
    })
  }
  return out
}

const generatedScript = computed(() =>
  generateScenarioScript(name.value.trim() || t('scenario.untitled'), toScriptSteps(), {
    base: scriptBase.value,
    stopOnFailure: stopOnFailure.value,
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

// ---------- 运行 ----------

/** 步骤 → 运行入参：URL 保留 {{}} 模板（执行期用 runtime 二次渲染）。 */
/** 单个启用步骤 → 运行入参（URL 保留 {{}} 模板，执行期 runtime 二次渲染）。 */
function buildOneRunStep(stepIdx: number) {
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
        type: r.type,
        path: r.path?.trim() || null,
        op: r.op || 'eq',
        expected: coerceExpected(r.expected),
      })),
  }
}

function buildRunSteps() {
  const out: NonNullable<ReturnType<typeof buildOneRunStep>>[] = []
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
      scenarioId: props.scenarioId,
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

// ---------- 步骤执行状态（卡片徽章 + 结果面板共用） ----------
type StepTone = 'ok' | 'fail' | 'skip' | 'run' | 'done' | 'wait' | 'cancel'

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

const resultByStepIndex = computed(() => {
  const m = new Map<number, ScenarioStepResult>()
  const results = runResult.value?.results ?? []
  runStepIndexes.value.forEach((stepIdx, i) => {
    const r = results[i]
    if (r) m.set(stepIdx, r)
  })
  return m
})

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
const historyMenu = ref<InstanceType<typeof Menu> | null>(null)

async function loadHistory(): Promise<void> {
  if (!scenario.value) {
    runHistory.value = []
    return
  }
  try {
    runHistory.value = await api.listScenarioRuns(props.scenarioId)
  } catch {
    runHistory.value = []
  }
}

/** 重开场景恢复最近一次运行（历史能力；无历史时保持空态）。 */
async function restoreLatestRun(scenarioIdAtLoad: string): Promise<void> {
  const latest = runHistory.value[0]
  if (!latest || props.scenarioId !== scenarioIdAtLoad) return
  try {
    const detail = await api.getScenarioRun(latest.id)
    if (!detail || props.scenarioId !== scenarioIdAtLoad) return
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
  historyMenu.value?.openAt(e.currentTarget as HTMLElement, items)
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

// ---------- 重跑失败步骤 ----------
// 仅实时运行结果可重跑（需要 runtime 快照重建链路上下文；历史无快照不提供入口）。
// 语义：失败步骤按原顺序串行重跑；首个重跑步以当时的 runtime 为初始变量，
// 其后每个重跑步先合并原运行中「位于两者之间的成功步骤」的提取增量——
// 即「若这些步骤依次重跑」的链路状态；失败步骤自身不并入旧提取值（重跑会重新提取）。

/** 是否存在可重跑的失败步骤（实时结果 + 有快照 + 步骤仍可构建）。 */
const rerunAvailable = computed(() => {
  if (running.value || runIsHistory.value || !runResult.value || !varsSnapshots.value.length)
    return false
  return rerunPositions().length > 0
})

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
  const built: NonNullable<ReturnType<typeof buildOneRunStep>>[] = []
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
      scenarioId: props.scenarioId,
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

/** 结果行 → 定位步骤卡（切回手动视图、展开并滚动到卡片）。 */
function locateStep(stepIdx: number | null): void {
  if (stepIdx == null) return
  const s = steps.value[stepIdx]
  if (!s) return
  viewMode.value = 'manual'
  s.open = true
  void nextTick(() => {
    const root = rootEl.value
    if (!root) return
    const el = root.querySelector(`[data-step-key="${CSS.escape(s.key)}"]`)
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    el?.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'center' })
  })
}

// ---------- 只读请求摘要（展开态；配置真源在接口编辑器） ----------
function enabledHeaders(i: number): { key: string; value: string }[] {
  const ep = stepEndpoint(i)
  if (!ep) return []
  return ep.request.headers.filter((h) => h.enabled && h.key.trim()).map((h) => ({ key: h.key.trim(), value: h.value }))
}

function enabledParamsCount(i: number): number {
  const ep = stepEndpoint(i)
  return ep ? ep.request.params.filter((p) => p.enabled && p.key.trim()).length : 0
}

/** Body 摘要：模式名 + 规模（字段数 / 字符数）；无 Body 返回空。 */
function bodySummary(i: number): string {
  const ep = stepEndpoint(i)
  if (!ep) return ''
  const body = ep.request.body
  switch (body.mode) {
    case 'json':
      return `${t('scenario.bodyJson')} · ${t('scenario.charCount', { n: body.raw.length })}`
    case 'text':
      return `${t('scenario.bodyText')} · ${t('scenario.charCount', { n: body.raw.length })}`
    case 'urlencoded':
      return `${t('scenario.bodyUrlencoded')} · ${t('scenario.fieldCount', { n: body.fields.filter((f) => f.enabled).length })}`
    case 'multipart':
      return `${t('scenario.bodyMultipart')} · ${t('scenario.fieldCount', { n: body.fields.filter((f) => f.enabled).length })}`
    case 'graphql':
      return t('scenario.bodyGraphql')
    case 'binary':
      return t('scenario.bodyBinary')
    default:
      return ''
  }
}

function stepUrl(i: number): string {
  const ep = stepEndpoint(i)
  return ep ? urlTemplateFor(ep) : ''
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
    expandedVar.value = null
    revealedVars.value = new Set()
    respOpenRows.value = new Set()
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
})

let unlistenProgress: UnlistenFn | null = null
onMounted(async () => {
  unlistenProgress = await listen<{
    runId: string
    done: number
    total: number
  }>('miyo:scenario-progress', (e) => {
    if (e.payload.runId !== runId.value) return
    progress.value = { done: e.payload.done, total: e.payload.total }
  })
})
onBeforeUnmount(() => {
  sortable?.destroy()
  sortable = null
  unlistenProgress?.()
  unlistenProgress = null
})

const rootEl = ref<HTMLElement | null>(null)
const enabledCount = computed(() => steps.value.filter((s) => s.enabled).length)
</script>

<template>
  <div v-if="!scenario && !loading" class="se-missing">
    <EmptyState icon="beaker" :title="t('scenario.missing')" compact />
  </div>
  <div v-else ref="rootEl" class="scenario-editor">
    <!-- ===== 头部：标题 / 元信息 / 操作 ===== -->
    <header class="se-head">
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
        </div>
        <input v-model="description" class="se-desc" :placeholder="t('scenario.descPh')" spellcheck="false" />
      </div>
      <div class="se-side">
        <div class="se-actions">
          <SegmentedControl
            v-model="viewMode"
            :options="VIEW_MODES"
            size="sm"
            :aria-label="t('scenario.viewMode')"
          />
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
            class="miyo-btn miyo-btn-sm"
            type="button"
            @click="stop"
          >
            <Icon name="stop" :size="13" /> {{ t('scenario.stop') }}
          </button>
        </div>
        <label class="se-strategy" :title="stopOnFailure ? t('scenario.stopOnFailureHint') : t('scenario.strategyHintContinue')">
          <span class="se-strategy-label">{{ t('scenario.strategyLabel') }}</span>
          <CustomSelect
            v-model="execStrategy"
            :options="STRATEGY_OPTIONS"
            size="sm"
            class="se-strategy-select"
            :disabled="running"
          />
        </label>
      </div>
    </header>

    <!-- ===== 变量链路（可折叠；来源/引用从启用步骤配置推导） ===== -->
    <section v-if="varChain.length" class="se-vars-panel">
      <button
        type="button"
        class="se-vars-head"
        :aria-label="varsExpanded ? t('scenario.collapseVars') : t('scenario.expandVars')"
        @click="varsExpanded = !varsExpanded"
      >
        <Icon name="zap" :size="12" class="se-vars-glyph" />
        <span class="se-vars-title">{{ t('scenario.linkVars') }}</span>
        <code
          v-for="v in varChain"
          :key="v.name"
          class="se-var-chip"
          :class="{ warn: v.missing }"
          :title="v.missing ? t('scenario.varMissingWarn') : undefined"
        >{{ varTag(v.name) }}</code>
        <span v-if="missingVarCount" class="se-vars-warn">
          <Icon name="alert-triangle" :size="11" />
          {{ t('scenario.varMissing', { n: missingVarCount }) }}
        </span>
        <span class="se-vars-caret">
          <Icon :name="varsExpanded ? 'chevrons-down-up' : 'chevrons-up-down'" :size="13" />
        </span>
      </button>
      <div v-if="varsExpanded" class="se-vars-body">
        <div
          v-for="v in varChain"
          :key="v.name"
          class="se-var-entry"
          :class="{ missing: v.missing }"
        >
          <button type="button" class="se-var-entry-head" @click="toggleVarDetail(v.name)">
            <code class="se-var-chip">{{ varTag(v.name) }}</code>
            <span v-if="v.missing" class="se-var-badge warn">{{ t('scenario.varUndefinedBadge') }}</span>
            <span v-else-if="v.external" class="se-var-badge ext">{{ t('scenario.varExternal') }}</span>
            <span v-else class="se-var-badge ok">{{ t('scenario.varLinked') }}</span>
            <span class="se-var-brief">{{ varBrief(v) }}</span>
            <Icon
              name="chevron-down"
              :size="12"
              class="se-var-caret"
              :class="{ open: expandedVar === v.name }"
            />
          </button>
          <div v-if="expandedVar === v.name" class="se-var-detail">
            <div v-if="v.sources.length" class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varSource') }}</span>
              <span v-for="src in v.sources" :key="src.stepIndex" class="se-var-v">
                {{ t('scenario.stepRef', { n: src.stepIndex + 1, name: src.stepName }) }}
                <code class="se-var-path">{{ src.path }}</code>
              </span>
            </div>
            <div v-if="v.usages.length" class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varUsages') }}</span>
              <span v-for="u in v.usages" :key="u.stepIndex" class="se-var-v">
                {{ t('scenario.stepRef', { n: u.stepIndex + 1, name: u.stepName }) }}
              </span>
            </div>
            <div class="se-var-line">
              <span class="se-var-k">{{ t('scenario.varRuntime') }}</span>
              <template v-if="runtimeValueOf(v.name) !== null">
                <code
                  class="se-var-value"
                  :class="{ masked: isSensitiveVarName(v.name) && !revealedVars.has(v.name) }"
                >{{ displayRuntimeValue(v.name) }}</code>
                <IconButton
                  v-if="isSensitiveVarName(v.name)"
                  name="eye"
                  :size="12"
                  :label="t('scenario.varToggleValue')"
                  :title="t('scenario.varToggleValue')"
                  @click="toggleReveal(v.name)"
                />
              </template>
              <span v-else class="se-var-none">{{ t('scenario.varRuntimeEmpty') }}</span>
            </div>
            <p v-if="v.missing" class="se-var-missing-hint">
              <Icon name="alert-triangle" :size="11" />
              {{ t('scenario.varMissingWarn') }}
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- ===== 手动编排：纵向步骤卡片 ===== -->
    <template v-if="viewMode === 'manual'">
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
          <div class="se-card">
            <div class="se-card-head">
              <span class="se-drag" :title="t('scenario.dragSort')">
                <Icon name="list" :size="13" />
              </span>
              <input
                v-model="s.enabled"
                type="checkbox"
                class="se-step-check"
                :title="t('scenario.stepEnable')"
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
              />
              <span v-if="s.endpointId && !endpointMap.has(s.endpointId)" class="se-step-missing">
                {{ t('scenario.endpointMissing') }}
              </span>
              <span v-if="stepTone(i)" class="se-step-status" :class="`tone-${stepTone(i)}`">
                <Icon :name="STEP_TONE_ICON[stepTone(i)!]" :size="11" />
                {{ t(STEP_TONE_TEXT[stepTone(i)!]) }}
              </span>
              <div class="se-step-tools">
                <IconButton
                  :name="s.open ? 'chevrons-down-up' : 'chevrons-up-down'"
                  :size="13"
                  :title="t('scenario.stepDetail')"
                  @click="s.open = !s.open"
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
              <span v-if="stepInputVars(i).length" class="se-step-vars">
                <code
                  v-for="n in stepInputVars(i).slice(0, 4)"
                  :key="`in-${n}`"
                  class="se-var-chip mini"
                  :class="{ warn: isMissingAt(i, n) }"
                  :title="isMissingAt(i, n) ? t('scenario.varMissingWarn') : varTag(n)"
                >{{ varTag(n) }}</code>
                <span v-if="stepInputVars(i).length > 4" class="se-var-more">
                  +{{ stepInputVars(i).length - 4 }}
                </span>
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

            <div v-if="s.open" class="se-step-body">
              <!-- 只读请求摘要：配置真源在接口编辑器（引用式步骤） -->
              <div v-if="stepEndpoint(i)" class="se-req">
                <div class="se-req-title">{{ t('scenario.reqSummary') }}</div>
                <div class="se-req-row">
                  <code class="se-req-url">{{ stepEndpoint(i)!.method }} {{ stepUrl(i) }}</code>
                </div>
                <div v-if="enabledHeaders(i).length" class="se-req-row">
                  <span class="se-req-k">{{ t('scenario.reqHeaders') }} · {{ enabledHeaders(i).length }}</span>
                  <code
                    v-for="h in enabledHeaders(i).slice(0, 4)"
                    :key="h.key"
                    class="se-req-v"
                  >{{ h.key }}: {{ h.value }}</code>
                  <span v-if="enabledHeaders(i).length > 4" class="se-var-more">
                    +{{ enabledHeaders(i).length - 4 }}
                  </span>
                </div>
                <div v-if="enabledParamsCount(i)" class="se-req-row">
                  <span class="se-req-k">{{ t('scenario.reqParams') }}</span>
                  <span class="se-req-v">{{ t('scenario.fieldCount', { n: enabledParamsCount(i) }) }}</span>
                </div>
                <div v-if="bodySummary(i)" class="se-req-row">
                  <span class="se-req-k">{{ t('scenario.reqBody') }}</span>
                  <code class="se-req-v">{{ bodySummary(i) }}</code>
                </div>
              </div>

              <div class="se-rule-group">
                <div class="se-rule-title">
                  {{ t('scenario.extractSection') }}
                  <button class="se-rule-add" type="button" @click="s.extract.push({ name: '', from: 'body', path: '' })">
                    <Icon name="plus" :size="12" /> {{ t('scenario.addExtract') }}
                  </button>
                </div>
                <div v-for="(r, j) in s.extract" :key="j" class="se-rule-row">
                  <input
                    v-model="r.name"
                    class="miyo-input miyo-input-sm se-r-name"
                    :placeholder="t('scenario.extractNamePh')"
                    spellcheck="false"
                  />
                  <CustomSelect v-model="r.from" :options="EXTRACT_FROM_OPTIONS" size="sm" class="se-r-from" />
                  <input
                    v-model="r.path"
                    class="miyo-input miyo-input-sm se-r-path"
                    :placeholder="r.from === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')"
                    spellcheck="false"
                  />
                  <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="s.extract.splice(j, 1)" />
                </div>
              </div>

              <div class="se-rule-group">
                <div class="se-rule-title">
                  {{ t('scenario.assertSection') }}
                  <button
                    class="se-rule-add"
                    type="button"
                    @click="s.assertions.push({ name: null, type: 'status', path: '', op: 'eq', expected: '' })"
                  >
                    <Icon name="plus" :size="12" /> {{ t('scenario.addAssert') }}
                  </button>
                </div>
                <div v-for="(a, j) in s.assertions" :key="j" class="se-rule-row">
                  <CustomSelect v-model="a.type" :options="ASSERT_TYPE_OPTIONS" size="sm" class="se-a-type" />
                  <input
                    v-if="a.type === 'jsonpath' || a.type === 'header'"
                    v-model="a.path"
                    class="miyo-input miyo-input-sm se-a-path"
                    :placeholder="a.type === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')"
                    spellcheck="false"
                  />
                  <CustomSelect v-model="a.op" :options="ASSERT_OP_OPTIONS" size="sm" class="se-a-op" />
                  <input
                    v-if="!NO_EXPECTED_OPS.has(a.op ?? 'eq')"
                    v-model="a.expected"
                    class="miyo-input miyo-input-sm se-a-exp"
                    :placeholder="t('scenario.expectedPh')"
                    spellcheck="false"
                  />
                  <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="s.assertions.splice(j, 1)" />
                </div>
              </div>
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

      <!-- ===== 执行结果面板（可折叠；支持历史回看 / 重开恢复） ===== -->
      <section v-if="running || runResult" class="se-report" :class="{ fail: finalState?.tone === 'fail' }">
        <div class="se-report-headrow">
          <button type="button" class="se-report-head" @click="reportOpen = !reportOpen">
            <Icon name="gauge" :size="13" />
            <span class="se-report-title">{{ t('scenario.resultTitle') }}</span>
            <span
              v-if="runIsHistory && currentRunAt"
              class="se-report-badge warn"
              :title="t('scenario.historyBadge')"
            >{{ t('scenario.historyBadge') }}</span>
            <template v-if="running">
              <span class="se-report-meta">{{ t('scenario.progress', { done: progress.done, total: progress.total }) }}</span>
            </template>
            <template v-else-if="runSummary && finalState">
              <span class="se-report-badge" :class="finalState.tone">{{ finalState.text }}</span>
              <span class="se-report-meta">
                {{ t('scenario.summaryOk', { n: runSummary.ok }) }}
                · {{ t('scenario.summaryFail', { n: runSummary.fail }) }}
                <template v-if="runSummary.skipped"> · {{ t('scenario.summarySkip', { n: runSummary.skipped }) }}</template>
                · {{ t('scenario.summaryTime', { t: formatDurationShort(runSummary.duration) }) }}
              </span>
            </template>
            <span v-if="runResult?.stopped_on_failure && !running" class="se-report-warn">
              {{ t('scenario.stoppedAt') }}
            </span>
            <span v-if="runResult?.cancelled && !running" class="se-report-warn">
              {{ t('scenario.cancelled') }}
            </span>
            <span v-if="currentRunAt && !running" class="se-report-meta se-report-time">
              {{ formatDateTime(currentRunAt) }}
            </span>
            <Icon
              :name="reportOpen ? 'chevrons-down-up' : 'chevrons-up-down'"
              :size="13"
              class="se-report-caret"
            />
          </button>
          <IconButton
            v-if="rerunAvailable"
            name="refresh"
            :size="13"
            :title="t('scenario.rerunFailed')"
            @click.stop="rerunFailed"
          />
          <IconButton
            name="history"
            :size="13"
            :title="t('scenario.history')"
            :disabled="!runHistory.length"
            @click.stop="openHistoryMenu($event)"
          />
        </div>
        <div v-if="reportOpen" class="se-report-body">
          <div v-if="running" class="se-progress">
            <div class="se-progress-track">
              <div class="se-progress-bar" :style="{ width: `${progressPercent}%` }"></div>
            </div>
          </div>
          <div
            v-for="(r, i) in runResult?.results ?? []"
            :key="i"
            class="se-rrow"
            :class="{ fail: !r.ok && !r.cancelled, cancelled: r.cancelled }"
            role="button"
            tabindex="0"
            :title="t('scenario.locateStep')"
            @click="locateStep(runStepIndexes[i] ?? null)"
            @keydown.enter="locateStep(runStepIndexes[i] ?? null)"
          >
            <div class="se-rrow-head">
              <span class="se-rdot" :class="{ fail: !r.ok && !r.cancelled, cancelled: r.cancelled }" aria-hidden="true"></span>
              <span class="se-rmethod" :class="methodBadgeTone(r.method)">{{ r.method }}</span>
              <span class="se-rname">{{ i + 1 }}. {{ r.endpoint_name }}</span>
              <span v-if="r.status != null" class="se-rstatus">{{ r.status }}</span>
              <span v-if="r.duration_ms != null" class="se-rtime">{{ formatDurationShort(r.duration_ms) }}</span>
              <IconButton
                v-if="r.response"
                :name="respOpenRows.has(i) ? 'chevrons-down-up' : 'chevrons-up-down'"
                :size="13"
                :title="t('scenario.viewResponse')"
                @click.stop="toggleRespRow(i)"
              />
              <Icon v-else name="arrow-up-right" :size="11" class="se-rloc" />
            </div>
            <code v-if="r.url" class="se-rurl" :title="r.url">{{ r.url }}</code>
            <p v-if="r.request_error" class="se-rerr">{{ r.request_error }}</p>
            <ul v-if="r.outcomes.length" class="se-routcomes">
              <li v-for="(o, j) in r.outcomes" :key="j" :class="{ fail: !o.passed }">
                {{ o.description }}
                <span v-if="o.expected != null || o.actual != null" class="se-r-outcome-va">
                  <template v-if="o.expected != null"> · {{ t('scenario.outcomeExpected', { v: o.expected }) }}</template>
                  <template v-if="o.actual != null"> · {{ t('scenario.outcomeActual', { v: o.actual }) }}</template>
                </span>
                <span v-else-if="o.reason"> — {{ o.reason }}</span>
              </li>
            </ul>
            <div v-if="r.extracted && Object.keys(r.extracted).length" class="se-rext">
              <span class="se-rext-label">{{ t('scenario.extracted') }}</span>
              <code v-for="(v, k) in r.extracted" :key="k" class="se-var-chip">
                {{ varTag(k) }} = {{ v }}
              </code>
            </div>
            <!-- 响应详情：复用单请求的响应查看组件（正文预览 + 按需取全文 + 请求快照） -->
            <div v-if="respOpenRows.has(i) && r.response" class="se-rresp">
              <ResponsePanel
                :response="r.response"
                :load-full-body="r.response.body_omitted ? fullBodyLoader(i) : undefined"
              />
            </div>
          </div>
        </div>
      </section>
      <p v-else-if="steps.length" class="se-report-empty">{{ t('scenario.resultEmpty') }}</p>
    </template>

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
  height: 100%;
  min-height: 0;
  overflow: auto;
  padding: 14px 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.se-missing {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* ---- 头部 ---- */
.se-head {
  display: flex;
  align-items: flex-start;
  gap: 14px;
}

.se-head-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
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
  width: 100%;
}

.se-desc:focus {
  border-bottom: 1px solid var(--border);
}

.se-side {
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 7px;
}

.se-actions {
  display: flex;
  gap: 8px;
}

.se-run {
  background: var(--accent, #8b5cf6);
  color: #fff;
}

.se-strategy {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: default;
}

.se-strategy-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

.se-strategy-select {
  width: 132px;
}

/* ---- 变量链路面板 ---- */
.se-vars-panel {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
}

.se-vars-head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 7px 10px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
}

.se-vars-glyph {
  color: var(--accent);
  flex: none;
}

.se-vars-title {
  flex: none;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.se-vars-warn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  margin-left: auto;
}

.se-vars-caret {
  display: inline-flex;
  color: var(--text-3);
}

.se-vars-body {
  border-top: 1px solid var(--border);
  padding: 6px 10px;
  display: flex;
  flex-direction: column;
}

.se-var-entry + .se-var-entry {
  border-top: 1px dashed var(--border);
}

.se-var-entry-head {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 5px 2px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
}

.se-var-entry-head:hover {
  background: var(--bg-hover);
  border-radius: var(--radius-sm);
}

.se-var-entry.missing > .se-var-entry-head > .se-var-chip {
  border-color: var(--warning);
  color: var(--warning);
}

.se-var-badge {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-3);
}

.se-var-badge.ok {
  background: var(--success-tint);
  border-color: transparent;
  color: var(--success);
}

.se-var-badge.ext {
  background: var(--bg-hover);
  border-color: transparent;
  color: var(--text-3);
}

.se-var-badge.warn {
  background: var(--warning-tint);
  border-color: transparent;
  color: var(--warning);
}

.se-var-brief {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-var-caret {
  flex: none;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease);
}

.se-var-caret.open {
  transform: rotate(180deg);
}

.se-var-detail {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 4px 2px 8px 34px;
}

.se-var-line {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: var(--fs-xxs);
  line-height: 1.6;
}

.se-var-k {
  flex: none;
  width: 52px;
  color: var(--text-3);
}

.se-var-v {
  color: var(--text-2);
}

.se-var-path {
  font-family: var(--font-mono);
  color: var(--tok-key);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0 5px;
}

.se-var-value {
  font-family: var(--font-mono);
  color: var(--text-1);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0 5px;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-var-value.masked {
  color: var(--text-3);
  letter-spacing: 2px;
}

.se-var-none {
  color: var(--text-3);
}

.se-var-missing-hint {
  margin: 2px 0 0;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

/* ---- 变量 chip（面板 / 卡片共用） ---- */
.se-var-chip {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  background: var(--bg-code);
  border: 1px solid var(--border);
  color: var(--tok-key);
  white-space: nowrap;
}

.se-var-chip.warn {
  border-color: var(--warning);
  color: var(--warning);
}

.se-var-chip.mini {
  padding: 0 5px;
  font-size: 10px;
  line-height: 16px;
}

.se-var-chip.out {
  color: var(--accent);
  border-color: var(--accent-tint);
  background: var(--accent-tint);
}

.se-var-more {
  font-size: 10px;
  color: var(--text-3);
}

.se-var-io {
  color: var(--text-3);
}

/* ---- 步骤列表（纵向轨道 + 卡片） ---- */
.se-steps {
  display: flex;
  flex-direction: column;
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

.se-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.4px;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
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

.se-card-sub {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 10px;
  padding: 0 10px 7px 36px;
  min-height: 18px;
}

.se-step-url {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  max-width: 46%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-step-vars {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px;
}

/* ---- 步骤展开态 ---- */
.se-step-body {
  border-top: 1px solid var(--border);
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.se-req {
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.se-req-title {
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
}

.se-req-row {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: var(--fs-xxs);
  line-height: 1.6;
}

.se-req-k {
  flex: none;
  color: var(--text-3);
}

.se-req-url,
.se-req-v {
  font-family: var(--font-mono);
  color: var(--text-2);
  word-break: break-all;
}

.se-req-url {
  color: var(--text-1);
}

.se-rule-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.se-rule-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-2);
}

.se-rule-add {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
}

.se-rule-add:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.se-rule-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.se-r-name {
  flex: 0 0 130px;
}

.se-r-from {
  flex: 0 0 90px;
}

.se-r-path,
.se-a-path,
.se-a-exp {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

.se-a-type {
  flex: 0 0 120px;
}

.se-a-op {
  flex: 0 0 100px;
}

/* ---- 添加步骤 / 空状态 ---- */
.se-add-step {
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
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  padding: 18px 0;
}

/* ---- 执行结果面板 ---- */
.se-report {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  margin-top: 4px;
}

.se-report.fail {
  border-color: var(--danger-border, var(--danger));
}

.se-report-headrow {
  display: flex;
  align-items: center;
  gap: 2px;
  padding-right: 6px;
}

.se-report-head {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  border: none;
  background: transparent;
  padding: 8px 10px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
}

.se-report-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.se-report-meta,
.se-report-warn {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-report-warn {
  color: var(--danger);
}

.se-report-badge {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 8px;
  border-radius: 999px;
}

.se-report-badge.ok {
  background: var(--success-tint);
  color: var(--success);
}

.se-report-badge.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.se-report-badge.warn {
  background: var(--warning-tint);
  color: var(--warning);
}

.se-report-caret {
  margin-left: auto;
  color: var(--text-3);
}

.se-report-body {
  border-top: 1px solid var(--border);
  padding: 8px 10px 10px;
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.se-progress {
  padding: 2px 0 4px;
}

.se-progress-track {
  height: 3px;
  border-radius: 999px;
  background: var(--bg-hover);
  overflow: hidden;
}

.se-progress-bar {
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width 0.3s var(--ease);
}

.se-rrow {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  cursor: pointer;
}

.se-rrow:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}

.se-rrow.fail {
  border-color: var(--danger-border, var(--danger));
}

.se-rrow.cancelled {
  opacity: 0.75;
}

.se-rdot.cancelled {
  background: var(--warning);
}

/* 渲染后的实际 URL（含变量替换结果），定位「跑的是哪个地址」 */
.se-rurl {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 结构化断言对比（期望 x · 实际 y），失败行红色承接 */
.se-r-outcome-va {
  font-family: var(--font-mono);
  word-break: break-all;
}

/* 响应详情：复用 ResponsePanel（弹性布局，需给界内固定高度） */
.se-rresp {
  height: 320px;
  display: flex;
  flex-direction: column;
  border-top: 1px dashed var(--border);
  padding-top: 8px;
}

.se-rrow-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.se-rdot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
}

.se-rdot.fail {
  background: var(--danger);
}

.se-rmethod {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.4px;
  padding: 1px 5px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}

.se-rname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.se-rstatus {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.se-rtime {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-rloc {
  flex: none;
  color: var(--text-3);
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}

.se-rrow:hover .se-rloc {
  opacity: 1;
}

.se-rerr {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--danger);
  white-space: pre-wrap;
  word-break: break-all;
}

.se-routcomes {
  margin: 0;
  padding: 0 0 0 16px;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.se-routcomes li {
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-2);
  word-break: break-all;
}

.se-routcomes li.fail {
  color: var(--danger);
}

.se-rext {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.se-rext-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-report-empty {
  margin: 0;
  padding: 8px 2px;
  border-top: 1px dashed var(--border);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 脚本视图 ---- */
.se-script-bar {
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
