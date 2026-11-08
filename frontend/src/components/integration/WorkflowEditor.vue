<script setup lang="ts">
/**
 * WorkflowEditor：集成测试场景编辑器（编排 | 脚本双视图 + 运行 / 结果面板）。
 *
 * 布局（信息分层，不堆一屏）：
 * - 顶栏：名称 / 分类 / 执行策略 / 保存 / 运行(停止) / 历史 / 比较 / 导出脚本 / 视图切换；
 * - 中列：可拖拽排序的步骤流（方法徽章 + 名称 + 路径 + 六态状态徽章 +
 *   启停 / 复制 / 删除；写操作步骤带「写操作」角标）；
 * - 右栏：选中步骤详情（请求配置 | 变量提取 | 断言 | 变量链路）；
 * - 底部：运行结果 Dock（复用 ScenarioRunReport：请求 / 响应 / 断言 / 提取详情）。
 *
 * 步骤两种形态：内联请求（步骤自持 method/url/spec，导入导出友好）与
 * 接口引用（endpoint_id，配置跟随接口）。运行统一物化为 BuiltRunStep。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import Sortable from 'sortablejs'
import { useLocaleStore } from '../../stores/locale'
import { useWorkspaceStore } from '../../stores/workspace'
import { useIntegrationStore } from '../../stores/integration'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import {
  useScenarioRun,
  type BuiltRunStep,
  type ScenarioProgressEvent,
} from '../../composables/useScenarioRun'
import { computeVarChain, type VarScanStep } from '../../utils/scenarioVars'
import {
  validateScenario,
  summarizeIssues,
  issuesOfStep,
  type ValidateStep,
  type ValidateIssue,
  type ValidateField,
} from '../../utils/scenarioValidate'
import { computeFlowGraph, requiredVarsFrom as requiredVarsFromFlow, toFlowSteps } from '../../utils/scenarioFlow'
import { applyPolicy, detectPolicy, OVERRIDE_POLICIES, type OverridePolicy } from '../../utils/overridePolicy'
import { applyStepOverrides, applyUrlOverride } from '../../utils/stepOverrides'
import { envColorClass, mergedVariableMap } from '../../utils/environment'
import { applyPathVars } from '../../utils/pathVariables'
import { methodBadgeTone } from '../../utils/methodTone'
import { generateScenarioScript, type ScriptAuth, type ScriptStep } from '../../utils/scenarioScript'
import { parseScenarioScript, ScriptParseError } from '../../utils/scenarioScriptParse'
import { copyText } from '../../utils/clipboard'
import CustomSelect from '../ui/CustomSelect.vue'
import SegmentedControl, { type SegmentOption } from '../ui/SegmentedControl.vue'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import Menu, { type MenuItem } from '../ui/Menu.vue'
import Modal from '../ui/Modal.vue'
import Popconfirm from '../ui/Popconfirm.vue'
import Tabs, { type TabItem } from '../ui/Tabs.vue'
import JsonCodeMirror from '../JsonCodeMirror.vue'
import ScenarioRunReport from '../scenario/ScenarioRunReport.vue'
import ScenarioVarsPanel from '../scenario/ScenarioVarsPanel.vue'
import StepRequestEditor from './StepRequestEditor.vue'
import RunResultView from './RunResultView.vue'
import RunCompareModal from './RunCompareModal.vue'
import type {
  Endpoint,
  Scenario,
  ScenarioAssertionRule,
  ScenarioExtractRule,
  ScenarioStep,
  StepCondition,
  StepLoop,
  StepOverrides,
} from '../../types/miyoApi'

const props = defineProps<{ scenarioId: string }>()
const emit = defineEmits<{ (e: 'back'): void }>()

const router = useRouter()
const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const store = useIntegrationStore()
const api = useMiyoApi()
const toast = useToast()

// ---------- 场景加载与编辑态 ----------
interface EditorExtract {
  name: string
  from: 'body' | 'header'
  path: string
}

interface EditorAssertion {
  name: string
  type: string
  path: string
  op: string
  expected: string
}

interface EditorStep {
  key: string
  enabled: boolean
  /** 'inline' = 内联请求；'ref' = 接口引用。 */
  kind: 'inline' | 'ref'
  endpointId: string | null
  name: string
  /** 内联请求（kind=inline 时有值；spec 与面板共享引用原地改）。 */
  inline: { method: string; url: string; spec: import('../../types/miyoApi').RequestSpec }
  extract: EditorExtract[]
  assertions: EditorAssertion[]
  /** 执行条件（None = 无条件）。 */
  condition: StepCondition | null
  /** 循环（None = 单次）。 */
  loop: StepLoop | null
  /** 执行前等待毫秒。 */
  waitMs: number | null
  /** 与相邻 parallel 步骤成批并发。 */
  parallel: boolean
  /** 引用式步骤的请求参数覆盖（None = 沿用接口配置）。 */
  overrides: StepOverrides | null
}

function emptyInline(): EditorStep['inline'] {
  return {
    method: 'GET',
    url: '{{base_url}}/',
    spec: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'none' },
      timeout_ms: null,
      follow_redirects: true,
      tests: null,
    },
  }
}

let stepSeq = 0
function newStepKey(): string {
  stepSeq += 1
  return `step-${Date.now().toString(36)}-${stepSeq}`
}

const scenario = ref<Scenario | null>(null)
const name = ref('')
const description = ref('')
const category = ref('')
const steps = ref<EditorStep[]>([])
const scriptDraft = ref('')
const loading = ref(true)
const saving = ref(false)

/** 保存基线（dirty 判定 + 保存后更新）。 */
let savedBaseline = ''

const draftSnapshot = (): string =>
  JSON.stringify({
    name: name.value,
    description: description.value,
    category: category.value,
    steps: steps.value.map((s) => ({
      enabled: s.enabled,
      kind: s.kind,
      endpointId: s.endpointId,
      name: s.name,
      inline: s.kind === 'inline' ? s.inline : null,
      extract: s.extract,
      assertions: s.assertions,
      condition: s.condition,
      loop: s.loop,
      waitMs: s.waitMs,
      parallel: s.parallel,
      overrides: s.overrides,
    })),
    script: scriptDraft.value,
  })

const dirty = computed(() => scenario.value !== null && draftSnapshot() !== savedBaseline)

// ---------- 顶栏保存状态（idle / dirty / saving） ----------
const saveStateText = computed(() => {
  if (saving.value) return t('integration.saveStateSaving')
  if (dirty.value) return t('integration.saveStateDirty')
  return t('integration.saveStateSaved')
})
const saveStateClass = computed(() => {
  if (saving.value) return 'saving'
  if (dirty.value) return 'dirty'
  return 'saved'
})

const scenarioRef = computed(() => (scenario.value ? { id: scenario.value.id, name: scenario.value.name } : null))

async function load(): Promise<void> {
  loading.value = true
  const pid = store.projectId
  if (!pid) return
  try {
    const list = await api.listScenarios(pid)
    if (store.projectId !== pid) return // 竞态：已切项目
    const found = list.find((s) => s.id === props.scenarioId)
    if (!found) {
      toast.warning(t('integration.historyMissing'))
      emit('back')
      return
    }
    applyScenario(found)
  } finally {
    loading.value = false
  }
}

/** 后端 Scenario → 编辑态。 */
function applyScenario(s: Scenario): void {
  scenario.value = s
  name.value = s.name
  description.value = s.description
  category.value = s.category
  steps.value = s.steps.map(stepToEditor)
  scriptDraft.value = s.script ?? generateScriptText()
  savedBaseline = draftSnapshot()
}

function stepToEditor(s: ScenarioStep): EditorStep {
  const inlineReq = s.request
    ? {
        method: s.request.method,
        url: s.request.url,
        spec: s.request.spec,
      }
    : emptyInline()
  return {
    key: newStepKey(),
    enabled: s.enabled,
    kind: s.request ? 'inline' : 'ref',
    endpointId: s.endpoint_id ?? null,
    name: s.name ?? '',
    inline: inlineReq,
    extract: parseExtract(s.extract),
    assertions: parseAssertions(s.assertions),
    condition: (s.condition as StepCondition | null | undefined) ?? null,
    loop: (s.loop as StepLoop | null | undefined) ?? null,
    waitMs: s.wait_ms ?? null,
    parallel: s.parallel ?? false,
    overrides: (s.overrides as StepOverrides | null | undefined) ?? null,
  }
}

function parseExtract(v: unknown): EditorExtract[] {
  if (!Array.isArray(v)) return []
  return v
    .map((r) => r as Partial<ScenarioExtractRule>)
    .filter((r) => r && typeof r.name === 'string')
    .map((r) => ({
      name: r.name ?? '',
      from: r.from === 'header' ? 'header' : 'body',
      path: typeof r.path === 'string' ? r.path : '',
    }))
}

function parseAssertions(v: unknown): EditorAssertion[] {
  if (!Array.isArray(v)) return []
  return v
    .map((r) => r as Partial<ScenarioAssertionRule>)
    .filter((r) => r && typeof r.type === 'string')
    .map((r) => ({
      name: typeof r.name === 'string' ? r.name : '',
      type: r.type ?? 'status',
      path: typeof r.path === 'string' ? r.path : '',
      op: typeof r.op === 'string' ? r.op : 'eq',
      expected: r.expected == null ? '' : String(r.expected),
    }))
}

/** 编辑态 → 后端 Scenario（保存体）。 */
function toScenario(): Scenario {
  const base = scenario.value
  const now = new Date().toISOString()
  const outSteps: ScenarioStep[] = steps.value.map((s) => ({
    endpoint_id: s.kind === 'ref' ? s.endpointId : null,
    enabled: s.enabled,
    extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
    assertions: s.assertions
      .filter((r) => r.type)
      .map((r) => ({
        name: r.name.trim() || null,
        type: r.type,
        path: r.path.trim() || null,
        op: r.op || null,
        expected: coerceExpected(r.expected),
      })),
    name: s.name.trim() || null,
    request:
      s.kind === 'inline'
        ? { method: s.inline.method, url: s.inline.url, spec: s.inline.spec }
        : null,
    condition: s.condition,
    loop: s.loop,
    wait_ms: s.waitMs,
    parallel: s.parallel,
    overrides:
      s.kind === 'ref' && s.overrides && Object.keys(s.overrides).length > 0 ? s.overrides : null,
  }))
  return {
    id: base?.id ?? crypto.randomUUID(),
    project_id: base?.project_id ?? store.projectId ?? '',
    name: name.value.trim() || t('integration.newName'),
    description: description.value,
    category: category.value.trim(),
    sort_order: base?.sort_order ?? 0,
    steps: outSteps,
    script: scriptDraft.value.trim() ? scriptDraft.value : null,
    created_at: base?.created_at ?? now,
    updated_at: now,
  }
}

async function save(): Promise<boolean> {
  if (saving.value) return false
  saving.value = true
  try {
    const saved = await api.saveScenario(toScenario())
    scenario.value = saved
    savedBaseline = draftSnapshot()
    toast.success(t('integration.saved'))
    void store.loadLibrary(true)
    return true
  } catch (err) {
    toast.error(t('integration.saveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
    return false
  } finally {
    saving.value = false
  }
}

// ---------- 断言期望值转型（数字 / 布尔 / 空） ----------
function coerceExpected(raw: string): unknown {
  const v = raw.trim()
  if (v === '') return null
  if (v === 'true') return true
  if (v === 'false') return false
  if (v.startsWith('{') || v.startsWith('[')) {
    try {
      return JSON.parse(v)
    } catch {
      /* 非法 JSON 按字符串 */
    }
  }
  const n = Number(v)
  if (v !== '' && Number.isFinite(n)) return n
  return v
}

// ---------- 步骤增删改 ----------
function addInlineStep(): void {
  steps.value.push({
    key: newStepKey(),
    enabled: true,
    kind: 'inline',
    endpointId: null,
    name: '',
    inline: emptyInline(),
    extract: [],
    assertions: [{ name: '', type: 'status', path: '', op: 'eq', expected: '200' }],
    condition: null,
    loop: null,
    waitMs: null,
    parallel: false,
    overrides: null,
  })
  selectStep(steps.value[steps.value.length - 1]!)
}

function addRefStep(ep: Endpoint): void {
  steps.value.push({
    key: newStepKey(),
    enabled: true,
    kind: 'ref',
    endpointId: ep.id,
    name: '',
    inline: emptyInline(),
    extract: [],
    assertions: [],
    condition: null,
    loop: null,
    waitMs: null,
    parallel: false,
    overrides: null,
  })
  selectStep(steps.value[steps.value.length - 1]!)
}

function duplicateStep(s: EditorStep): void {
  const idx = steps.value.indexOf(s)
  if (idx === -1) return
  const copy: EditorStep = JSON.parse(JSON.stringify(s))
  copy.key = newStepKey()
  steps.value.splice(idx + 1, 0, copy)
  selectStep(copy)
}

/** 引用式步骤的覆盖对象惰性创建（KV 表需要可写目标；空对象落盘前归 null）。 */
function ensureOverrides(s: EditorStep): StepOverrides {
  if (!s.overrides) s.overrides = {}
  return s.overrides
}

// ---------- 覆盖策略三态（继承 / 合并 / 替换） ----------
const POLICY_OPTIONS = computed(() =>
  OVERRIDE_POLICIES.map((p) => ({ value: p, label: t(`integration.policy.${p}`) })),
)

/** 当前步骤的覆盖策略（由 overrides 内容推断，无新增持久化字段）。 */
function stepPolicy(s: EditorStep): OverridePolicy {
  return detectPolicy(endpointOf(s)?.request ?? null, s.overrides)
}

/** 切换策略：inherit → null；merge/replace → 按策略重算 overrides。 */
function onPolicyChange(s: EditorStep, value: string): void {
  const policy = (OVERRIDE_POLICIES as string[]).includes(value) ? (value as OverridePolicy) : 'merge'
  s.overrides = applyPolicy(policy, endpointOf(s)?.request ?? null, s.overrides)
}

/** 引用式 → 内联：复制接口当前配置为步骤私有请求（此后与接口解耦）。 */
function convertToInline(s: EditorStep): void {
  const ep = endpointOf(s)
  if (!ep) return
  const spec: import('../../types/miyoApi').RequestSpec = JSON.parse(JSON.stringify(ep.request))
  s.inline = { method: ep.method, url: urlTemplateFor(ep), spec }
  s.kind = 'inline'
  s.endpointId = null
  s.overrides = null
  if (!s.name.trim()) s.name = ep.name
  selectStep(s)
}

function removeStep(s: EditorStep): void {
  const idx = steps.value.indexOf(s)
  if (idx === -1) return
  steps.value.splice(idx, 1)
  if (selectedKey.value === s.key) selectedKey.value = null
}

// ---------- 选中步骤 + 右栏 ----------
const selectedKey = ref<string | null>(null)
const selectedStep = computed(() => steps.value.find((s) => s.key === selectedKey.value) ?? null)
const selectedEndpoint = computed(() =>
  selectedStep.value?.endpointId
    ? (workspace.endpoints.find((e) => e.id === selectedStep.value!.endpointId) ?? null)
    : null,
)

function selectStep(s: EditorStep | null): void {
  selectedKey.value = s?.key ?? null
  if (s) detailTab.value = 'request'
}

const detailTabs: TabItem[] = [
  { key: 'request', label: t('integration.tabRequest') },
  { key: 'control', label: t('integration.tabControl') },
  { key: 'extract', label: t('integration.tabExtract') },
  { key: 'assert', label: t('integration.tabAssert') },
  { key: 'vars', label: t('integration.tabVars') },
]
const detailTab = ref('request')

// ---------- 执行控制（条件 / 循环 / 等待 / 并行）编辑桥 ----------
/** 条件 op 集（与断言算子同集）。 */
function condOpOptions(c?: StepCondition) {
  const ops = [
    'eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte',
    'exists', 'not_exists', 'matches', 'not_matches', 'empty', 'not_empty',
  ]
  const cur = c?.op ?? 'eq'
  return ops.map((o) => ({ value: o, label: t(`integration.assertOp.${o}`), header: o === cur ? undefined : undefined }))
}
/** exists/empty 族不需要期望值。 */
function condNeedsValue(c?: StepCondition): boolean {
  return !['exists', 'not_exists', 'empty', 'not_empty'].includes(c?.op ?? 'eq')
}
/** 切条件源：prevStatus 无变量名。 */
function onCondSourceChange(c: StepCondition, source: string): void {
  c.source = source === 'prevStatus' ? 'prevStatus' : 'var'
  if (c.source === 'prevStatus') c.name = ''
  else if (!c.name) c.name = ''
}
/** 切循环模式：count 需轮数；while 需继续条件。 */
function onLoopModeChange(l: StepLoop, mode: string): void {
  l.mode = mode === 'while' ? 'while' : 'count'
  if (l.mode === 'count' && !l.count) l.count = 3
  if (l.mode === 'while') {
    if (!l.while_condition) l.while_condition = { source: 'var', name: '', op: 'exists', value: null }
    if (!l.max_loops) l.max_loops = 10
  }
}
/** number|null ⇄ 输入框文本桥（空串 = 清除）。 */
function numBridge(get: () => number | null, set: (v: number | null) => void) {
  return computed({
    get: () => (get() == null ? '' : String(get())),
    set: (v: string) => {
      const n = Number.parseInt(v, 10)
      set(Number.isFinite(n) && n > 0 ? n : null)
    },
  })
}
const condValueText = computed({
  get: () => (selectedStep.value?.condition?.value == null ? '' : String(selectedStep.value.condition.value)),
  set: (v: string) => {
    const c = selectedStep.value?.condition
    if (c) c.value = coerceExpected(v)
  },
})
const loopCountText = numBridge(
  () => selectedStep.value?.loop?.count ?? null,
  (v) => { if (selectedStep.value?.loop) selectedStep.value.loop.count = v },
)
const loopMaxText = numBridge(
  () => selectedStep.value?.loop?.max_loops ?? null,
  (v) => { if (selectedStep.value?.loop) selectedStep.value.loop.max_loops = v },
)
const loopIntervalText = numBridge(
  () => selectedStep.value?.loop?.interval_ms ?? null,
  (v) => { if (selectedStep.value?.loop) selectedStep.value.loop.interval_ms = v },
)
const loopCondValueText = computed({
  get: () => (selectedStep.value?.loop?.while_condition?.value == null ? '' : String(selectedStep.value.loop!.while_condition!.value)),
  set: (v: string) => {
    const c = selectedStep.value?.loop?.while_condition
    if (c) c.value = coerceExpected(v)
  },
})
const waitMsText = numBridge(
  () => selectedStep.value?.waitMs ?? null,
  (v) => { if (selectedStep.value) selectedStep.value.waitMs = v },
)

// ---------- 提取 / 断言行编辑（原地改数组） ----------
const selExtract = computed(() => selectedStep.value?.extract ?? [])
const selAssertions = computed(() => selectedStep.value?.assertions ?? [])

function addExtract(): void {
  selectedStep.value?.extract.push({ name: '', from: 'body', path: '' })
}
function addAssertion(): void {
  selectedStep.value?.assertions.push({ name: '', type: 'status', path: '', op: 'eq', expected: '200' })
}

const ASSERT_TYPES = ['status', 'jsonpath', 'header', 'response_time_ms'] as const
/** 各断言类型的可用算子（status/数字类无 contains；body 只做包含判断）。 */
function opsFor(type: string): string[] {
  switch (type) {
    case 'status':
    case 'response_time_ms':
      return ['eq', 'neq', 'gt', 'gte', 'lt', 'lte']
    case 'header':
      return ['eq', 'neq', 'contains', 'not_contains', 'exists', 'not_exists', 'matches', 'not_matches', 'empty', 'not_empty']
    case 'jsonpath':
      return ['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'exists', 'not_exists', 'matches', 'not_matches', 'empty', 'not_empty']
    case 'body':
      return ['contains', 'not_contains', 'exists', 'not_exists']
    default:
      return ['eq']
  }
}

const ASSERT_TYPE_KEYS = computed(() =>
  ASSERT_TYPES.map((v) => ({ value: v, label: t(`integration.assertType.${v}`) })),
)
function assertOpOptions(type: string) {
  return opsFor(type).map((op) => ({ value: op, label: t(`integration.assertOp.${op}`) }))
}
function onAssertTypeChange(a: EditorAssertion, type: string): void {
  a.type = type
  const ops = opsFor(type)
  if (!ops.includes(a.op)) a.op = ops[0] ?? 'eq'
  if (type === 'status' || type === 'response_time_ms' || type === 'body') a.path = ''
}
/** 需要 path 目标的断言类型。 */
function needsPath(type: string): boolean {
  return type === 'jsonpath' || type === 'header'
}
/** 需要期望值的断言类型（exists/empty 类不需要）。 */
function needsExpected(a: EditorAssertion): boolean {
  return !['exists', 'not_exists', 'empty', 'not_empty'].includes(a.op)
}

// ---------- 接口引用：URL 模板 / 展示 ----------
function endpointOf(step: EditorStep): Endpoint | null {
  return step.endpointId ? (workspace.endpoints.find((e) => e.id === step.endpointId) ?? null) : null
}

function urlTemplateFor(ep: Endpoint): string {
  const isAbs = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(ep.path)
  const path = ep.path.startsWith('/') ? ep.path : `/${ep.path}`
  return isAbs
    ? applyPathVars(ep.path, ep.request.path_variables)
    : `{{base_url}}${applyPathVars(path, ep.request.path_variables)}`
}

/** 步骤卡展示行：方法 / 名称 / 路径。 */
function stepMethod(s: EditorStep): string {
  return s.kind === 'inline' ? s.inline.method : (endpointOf(s)?.method ?? 'GET')
}

function stepPath(s: EditorStep): string {
  return s.kind === 'inline' ? s.inline.url : (endpointOf(s)?.path ?? '')
}

function stepTitle(s: EditorStep): string {
  return s.name.trim() || (s.kind === 'ref' ? (endpointOf(s)?.name ?? '') : '') || t('integration.stepUntitled', { n: steps.value.indexOf(s) + 1 })
}

/** 循环徽章：while 显示「≤上限」，count 显示固定轮数。 */
function loopBadgeCount(l: StepLoop): number {
  return l.mode === 'count' ? (l.count ?? 1) : (l.max_loops ?? 10)
}
function loopBadgeTitle(l: StepLoop): string {
  return l.mode === 'count'
    ? t('integration.loopCountTitle', { n: l.count ?? 1 })
    : t('integration.loopWhileTitle', { n: l.max_loops ?? 10 })
}
function conditionBadgeTitle(c: StepCondition): string {
  const target = c.source === 'prevStatus' ? t('integration.condSourcePrevStatus') : `{{${c.name ?? ''}}}`
  const v = c.value == null ? '' : String(c.value)
  return `${target} ${c.op} ${v}`.trim()
}

/** 写操作方法（副作用提示）。 */
const WRITE_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE'])
function isWriteStep(s: EditorStep): boolean {
  return s.enabled && WRITE_METHODS.has(stepMethod(s).toUpperCase())
}

// ---------- 变量链路（复用 scenarioVars 计算 + ScenarioVarsPanel 展示） ----------
const varScanSteps = computed<VarScanStep[]>(() =>
  steps.value.map((s, i) => {
    const texts: string[] = []
    if (s.kind === 'inline') {
      texts.push(s.inline.url)
      for (const kv of [...s.inline.spec.params, ...s.inline.spec.headers]) {
        texts.push(kv.value)
      }
      if (s.inline.spec.body.mode === 'json' || s.inline.spec.body.mode === 'text') {
        texts.push(s.inline.spec.body.raw)
      }
    } else {
      const ep = endpointOf(s)
      if (ep) {
        texts.push(urlTemplateFor(ep))
        for (const kv of [...ep.request.params, ...ep.request.headers]) texts.push(kv.value)
        if (ep.request.body.mode === 'json' || ep.request.body.mode === 'text') {
          texts.push(ep.request.body.raw)
        }
      }
    }
    for (const a of s.assertions) texts.push(a.expected)
    return {
      index: i,
      name: stepTitle(s),
      extracts: s.extract.filter((e) => e.name.trim() && e.path.trim()),
      referenceTexts: texts,
    }
  }),
)

const externalVarNames = computed(() =>
  Object.keys(
    mergedVariableMap({
      globalVariables: workspace.globalVariables,
      projectVariables: workspace.project?.variables ?? null,
      env: workspace.environments.find((e) => e.id === workspace.activeEnvId),
    }),
  ),
)
const varChain = computed(() => computeVarChain(varScanSteps.value, externalVarNames.value))

// ---------- 运行前校验（纯函数物化，见 utils/scenarioValidate） ----------
/** 表达式面板里「值型算子」集合：这些算子缺期望值无法求值。 */
const VALUE_OPS = new Set(['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'regex', 'in'])

const validateSteps = computed<ValidateStep[]>(() =>
  steps.value.map((s, i) => {
    const ep = s.kind === 'ref' ? endpointOf(s) : null
    const texts = varScanSteps.value[i]?.referenceTexts ?? []
    const cond = s.condition
    const wc = s.loop?.while_condition ?? null
    return {
      index: i,
      enabled: s.enabled,
      name: stepTitle(s),
      endpointId: s.endpointId,
      endpointResolved: s.kind === 'inline' || ep != null,
      url:
        s.kind === 'inline'
          ? s.inline.url
          : ep
            ? applyUrlOverride(urlTemplateFor(ep), s.overrides)
            : '',
      referenceTexts: texts,
      extracts: s.extract.filter((e) => e.name.trim() || e.path.trim()),
      assertions: s.assertions.map((a) => ({
        type: a.type,
        path: a.path,
        op: a.op,
        needsExpected: VALUE_OPS.has(a.op),
        expected: a.expected,
      })),
      assertionsParsed: true,
      extractParsed: true,
      condition: cond
        ? {
            source: (cond.source ?? 'var') as 'var' | 'prevStatus',
            name: cond.name ?? '',
            op: cond.op ?? 'eq',
            value: cond.value ?? null,
            needsValue: VALUE_OPS.has(cond.op ?? 'eq'),
          }
        : null,
      loop: s.loop
        ? {
            mode: s.loop.mode,
            count: s.loop.count ?? null,
            maxLoops: s.loop.max_loops ?? null,
            intervalMs: s.loop.interval_ms ?? null,
            whileCondition: wc
              ? {
                  name: wc.name ?? '',
                  op: wc.op ?? 'eq',
                  value: wc.value ?? null,
                  needsValue: VALUE_OPS.has(wc.op ?? 'eq'),
                }
              : null,
          }
        : null,
      parallel: s.parallel,
    }
  }),
)

const issues = computed(() =>
  validateScenario(validateSteps.value, {
    externalNames: externalVarNames.value,
    scenarioName: name.value,
    hasEnvironment: !!workspace.activeEnvId,
  }),
)
const issueSummary = computed(() => summarizeIssues(issues.value))
const errorCount = computed(() => issueSummary.value.errors)
const warnCount = computed(() => issueSummary.value.warnings)
/** 校验面板展开态：有阻断时默认展开，用户可手动收起。 */
const validateOpen = ref(false)
watch(
  errorCount,
  (n, prev) => {
    if (n > 0 && !prev) validateOpen.value = true
  },
  { immediate: true },
)

/** 单步问题（步骤卡角标）。 */
function stepIssues(stepIndex: number): ValidateIssue[] {
  return issuesOfStep(issues.value, stepIndex)
}

/** 校验项 → 定位：切到编排视图、选中步骤、切到对应页签、滚动到视口。 */
function gotoIssue(issue: ValidateIssue): void {
  if (issue.stepIndex < 0) {
    viewMode.value = 'manual'
    return
  }
  viewMode.value = 'manual'
  const s = steps.value[issue.stepIndex]
  if (!s) return
  selectStep(s)
  const field: ValidateField = issue.field === 'general' ? 'request' : issue.field
  if (detailTabs.some((x) => x.key === field)) detailTab.value = field
  void nextTick(() => {
    const el = stepsEl.value?.querySelector(`[data-step-key="${s.key}"]`)
    el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  })
}

// ---------- 工作流连线（变量流向可视化） ----------
const flowSteps = computed(() =>
  toFlowSteps(
    steps.value.map((s, i) => ({
      index: i,
      enabled: s.enabled,
      name: stepTitle(s),
      extracts: s.extract,
      referenceTexts: varScanSteps.value[i]?.referenceTexts ?? [],
      conditionName: s.condition?.name ?? null,
      loopWhileName: s.loop?.while_condition?.name ?? null,
      looping: s.loop != null,
      parallel: s.parallel,
    })),
  ),
)
const flowGraph = computed(() => computeFlowGraph(flowSteps.value, externalVarNames.value))
/** 步骤下标 → 该步「上一条连线」（入口边 from=-1 挂在首个启用步骤上）。 */
const edgeInto = computed(() => {
  const map = new Map<number, (typeof flowGraph.value.edges)[number]>()
  for (const e of flowGraph.value.edges) map.set(e.to, e)
  return map
})

/** 连线上的变量标签：超过 3 个折叠为 +N。 */
function edgeVarsLabel(e: { vars: string[]; externalVars: string[] }): { shown: string[]; more: number; external: boolean } {
  const all = e.vars
  const shown = all.slice(0, 3)
  return { shown, more: Math.max(0, all.length - shown.length), external: all.length === 0 && e.externalVars.length > 0 }
}

// ---------- 运行（useScenarioRun + 物化） ----------
function buildOneRunStep(s: EditorStep): BuiltRunStep | null {
  if (!s.enabled) return null
  if (s.kind === 'inline') {
    const name = s.name.trim() || t('integration.stepUntitled', { n: steps.value.indexOf(s) + 1 })
    const spec = JSON.parse(JSON.stringify(s.inline.spec))
    return {
      index: steps.value.indexOf(s),
      endpoint: {
        id: s.key,
        project_id: '',
        folder_id: null,
        name,
        method: s.inline.method as Endpoint['method'],
        path: s.inline.url,
        description: '',
        status: 'developing',
        sort_order: 0,
        request: spec,
        created_at: '',
        updated_at: '',
      },
      url: s.inline.url,
      enabled: true,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          type: r.type,
          path: r.path.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected),
        })),
      condition: s.condition,
      loop: s.loop,
      wait_ms: s.waitMs,
      parallel: s.parallel,
    }
  }
  const ep = endpointOf(s)
  if (!ep) return null
  // 引用式：克隆接口配置并应用步骤级参数覆盖（未覆盖项沿用接口）。
  const spec: import('../../types/miyoApi').RequestSpec = JSON.parse(JSON.stringify(ep.request))
  try {
    applyStepOverrides(spec, s.overrides)
  } catch (err) {
    toast.error(t('integration.overridesInvalid'), {
      message: err instanceof Error ? err.message : String(err),
    })
    return null
  }
  return {
    index: steps.value.indexOf(s),
    endpoint: { ...ep, request: spec },
    url: applyUrlOverride(urlTemplateFor(ep), s.overrides),
    enabled: true,
    extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
    assertions: s.assertions
      .filter((r) => r.type)
      .map((r) => ({
        type: r.type,
        path: r.path.trim() || null,
        op: r.op || 'eq',
        expected: coerceExpected(r.expected),
      })),
    condition: s.condition,
    loop: s.loop,
    wait_ms: s.waitMs,
    parallel: s.parallel,
  }
}

function buildRunSteps(): BuiltRunStep[] {
  const out: BuiltRunStep[] = []
  steps.value.forEach((s) => {
    const built = buildOneRunStep(s)
    if (built) out.push(built)
  })
  return out
}

const menuRef = ref<InstanceType<typeof Menu> | null>(null)
function openMenuAt(anchor: HTMLElement, items: MenuItem[]): void {
  menuRef.value?.openAt(anchor, items)
}

/** 「从接口添加」菜单：列出当前项目接口（方法 + 名称）。 */
function openEndpointMenu(anchor: HTMLElement): void {
  menuContext.value = 'add-endpoint'
  openMenuAt(
    anchor,
    workspace.endpoints.map((ep) => ({
      key: ep.id,
      label: `${ep.method} ${ep.name}`,
      icon: 'file' as const,
    })),
  )
}

const run = useScenarioRun({
  store: workspace,
  api,
  toast,
  t,
  scenario: scenarioRef,
  scenarioId: props.scenarioId,
  buildRunSteps,
  dirty,
  save,
  openMenuAt,
})

// ---------- 写操作确认（副作用提示） ----------
const confirmRunOpen = ref(false)

const writeStepTitles = computed(() =>
  steps.value.filter(isWriteStep).map((s) => `${stepMethod(s)} ${stepTitle(s)}`),
)

/** 环境名含生产语义时追加警示（复用 envColorClass 的命名启发式）。 */
const envProdLike = computed(() => {
  const env = workspace.environments.find((e) => e.id === workspace.activeEnvId)
  return envColorClass(env?.name ?? '') === 'prod'
})

function onRunClick(): void {
  if (run.running.value) {
    void run.stop()
    return
  }
  // 运行前校验：有阻断项先展开面板让用户定位修复（不静默拒绝——面板里有「仍然运行」）。
  if (errorCount.value > 0) {
    validateOpen.value = true
    viewMode.value = viewMode.value === 'script' ? 'manual' : viewMode.value
  }
  if (writeStepTitles.value.length) {
    confirmRunOpen.value = true
    return
  }
  void run.run()
}

function confirmRun(): void {
  confirmRunOpen.value = false
  void run.run()
}

/** 校验面板的「仍然运行」：忽略阻断项。写操作照旧走副作用确认。 */
function forceRun(): void {
  validateOpen.value = false
  if (writeStepTitles.value.length) {
    confirmRunOpen.value = true
    return
  }
  void run.run()
}

// ---------- 手填前置变量（单步 / 从步骤执行且无运行时快照时） ----------
const varsFillOpen = ref(false)
const varsFillValues = ref<Record<string, string>>({})
const varsFillNewName = ref('')
const varsFillTarget = ref<{ from: number; only: boolean } | null>(null)
/** 该步（及后续步骤）真正依赖的变量：必须手填，否则运行会拿到空值。 */
const varsFillRequired = computed(() => (varsFillTarget.value ? requiredVarsFrom(varsFillTarget.value.from) : []))
/** 弹窗里列出的行：依赖变量 ∪ 用户手动添加的。 */
const varsFillMissing = computed(() => {
  const names = [...varsFillRequired.value]
  for (const k of Object.keys(varsFillValues.value)) if (!names.includes(k)) names.push(k)
  return names
})
/** 所有必需变量都填了才允许运行。 */
const varsFillReady = computed(() =>
  varsFillRequired.value.every((n) => (varsFillValues.value[n] ?? '').trim().length > 0),
)

/**
 * 计算某步起真正需要的外部输入：该步（含）之后消费、但不是更早启用步骤提取的变量。
 * 这些正是「无快照时链路断掉」的部分，必须由用户补齐。
 */
function requiredVarsFrom(fromIdx: number): string[] {
  return requiredVarsFromFlow(flowSteps.value, flowGraph.value.definedBy, externalVarNames.value, fromIdx)
}

/** 选「从步骤执行 / 单步执行」：有快照直接跑；否则先弹窗收变量。 */
function requestRunFrom(fromIdx: number, onlyThis: boolean): void {
  const start = buildRunSteps().findIndex((b) => b.index === fromIdx)
  if (start === -1) return
  if (run.varsSnapshots.value[start]) {
    void run.runFrom(fromIdx, onlyThis)
    return
  }
  const required = requiredVarsFrom(fromIdx)
  if (!required.length) {
    // 该步不依赖任何前序产物：直接跑，不会产生隐式空变量。
    void run.runFrom(fromIdx, onlyThis)
    return
  }
  varsFillTarget.value = { from: fromIdx, only: onlyThis }
  varsFillValues.value = Object.fromEntries(required.map((n) => [n, '']))
  varsFillNewName.value = ''
  varsFillOpen.value = true
}

function addVarsFillRow(): void {
  const name = varsFillNewName.value.trim()
  if (!name) return
  if (!(name in varsFillValues.value)) varsFillValues.value[name] = ''
  varsFillNewName.value = ''
}

function confirmVarsFill(): void {
  const target = varsFillTarget.value
  if (!target || !varsFillReady.value) return
  const vars: Record<string, string> = {}
  for (const [k, v] of Object.entries(varsFillValues.value)) {
    if (v.trim()) vars[k] = v
  }
  varsFillOpen.value = false
  varsFillTarget.value = null
  void run.runFrom(target.from, target.only, vars)
}

// ---------- 进度事件 ----------
let unlisten: UnlistenFn | null = null
onMounted(async () => {
  unlisten = await listen<ScenarioProgressEvent>('miyo:scenario-progress', (e) => {
    run.applyProgressEvent(e.payload)
  })
  await load()
  void run.loadHistory()
  void run.restoreLatestRun(props.scenarioId)
  // 列表「执行历史」直达：加载指定运行并进入结果视图。
  const pending = store.takePendingRun()
  if (pending) {
    try {
      const detail = await api.getScenarioRun(pending)
      if (detail && scenario.value?.id === props.scenarioId) {
        run.applyHistoryDetail(detail)
        viewMode.value = 'result'
      }
    } catch {
      // 历史可能已被清理：保持编辑态
    }
  }
})
onBeforeUnmount(() => {
  unlisten?.()
  unlisten = null
  sortInstance?.destroy()
  sortInstance = null
})

// ---------- 拖拽排序（SortableJS，与旧 ScenarioEditor 同款收口） ----------
let sortInstance: Sortable | null = null
const stepsEl = ref<HTMLElement | null>(null)
onMounted(() => {
  if (!stepsEl.value) return
  sortInstance = Sortable.create(stepsEl.value, {
    handle: '.wf-drag',
    animation: 150,
    forceFallback: true,
    ghostClass: 'wf-ghost',
    filter: 'button, input, select, textarea',
    onEnd: () => {
      // DOM 顺序 → 数组顺序（data-step-key 稳定键）。
      const order = Array.from(stepsEl.value?.querySelectorAll('[data-step-key]') ?? []).map(
        (el) => (el as HTMLElement).dataset.stepKey ?? '',
      )
      const map = new Map(steps.value.map((s) => [s.key, s]))
      const next: EditorStep[] = []
      for (const key of order) {
        const s = map.get(key)
        if (s) {
          next.push(s)
          map.delete(key)
        }
      }
      steps.value = [...next, ...map.values()]
    },
  })
})

// ---------- 脚本视图 ----------
const viewMode = ref<'manual' | 'script' | 'result'>('manual')
const VIEW_OPTIONS = computed<SegmentOption[]>(() => {
  const base: SegmentOption[] = [
    { value: 'manual', label: t('integration.viewManual') },
    { value: 'script', label: t('integration.viewScript') },
  ]
  // 有运行结果时才出现「执行结果」视图（运行完成自动切换 / 历史直达）。
  if (run.runResult.value || run.runIsHistory.value) {
    base.push({ value: 'result', label: t('integration.viewResult') })
  }
  return base
})

/** 运行完成自动进入结果视图（实时进度仍在 Dock；结果视图是完整复盘）。 */
watch(
  () => run.running.value,
  (now, was) => {
    if (was && !now && run.runResult.value) viewMode.value = 'result'
  },
)

/** 历史菜单选中 → 打开该次运行的结果视图。 */
async function onHistorySelect(item: MenuItem): Promise<void> {
  await run.onHistorySelect(item)
  if (run.runIsHistory.value) viewMode.value = 'result'
}

function toScriptSteps(): ScriptStep[] {
  return steps.value.map((s) => {
    const ep = s.kind === 'ref' ? endpointOf(s) : null
    if (s.kind === 'inline') {
      const body = s.inline.spec.body
      const kind = body.mode
      const enabledHeaders = s.inline.spec.headers
        .filter((h) => h.enabled)
        .map((h) => ({ key: h.key, value: h.value }))
      const auth = staticAuth(s.inline.spec.auth)
      return {
        name: stepTitle(s),
        method: s.inline.method,
        url: s.inline.url,
        params: s.inline.spec.params.filter((p) => p.enabled).map((p) => ({ key: p.key, value: p.value })),
        headers: enabledHeaders,
        auth: auth?.auth ?? null,
        authUnsupported: auth?.unsupported ?? null,
        body:
          body.mode === 'json' || body.mode === 'text'
            ? body.raw
            : body.mode === 'graphql'
              ? body.spec.query
              : null,
        bodyFields:
          body.mode === 'urlencoded' || body.mode === 'multipart'
            ? body.fields.filter((f) => f.enabled).map((f) => ({ key: f.key, value: f.value }))
            : [],
        bodyFileFields:
          body.mode === 'multipart' ? body.fields.filter((f) => f.value_type === 'file_path').map((f) => f.key) : [],
        bodyVariables: body.mode === 'graphql' ? body.spec.variables : '',
        bodyPath: body.mode === 'binary' ? body.path : '',
        bodyKind: kind === 'none' ? 'none' : (kind as ScriptStep['bodyKind']),
        condition: s.condition,
        loop: s.loop,
        waitMs: s.waitMs,
        parallel: s.parallel,
        extract: s.extract as ScenarioExtractRule[],
        assertions: s.assertions as unknown as ScenarioAssertionRule[],
      }
    }
    if (!ep) {
      return {
        name: stepTitle(s),
        method: 'GET',
        url: '',
        headers: [],
        body: null,
        bodyKind: 'none' as const,
        extract: s.extract as ScenarioExtractRule[],
        assertions: s.assertions as unknown as ScenarioAssertionRule[],
      }
    }
    const auth = staticAuth(ep.request.auth)
    const body = ep.request.body
    return {
      name: stepTitle(s),
      method: ep.method,
      url: urlTemplateFor(ep),
      params: ep.request.params.filter((p) => p.enabled).map((p) => ({ key: p.key, value: p.value })),
      headers: ep.request.headers.filter((h) => h.enabled).map((h) => ({ key: h.key, value: h.value })),
      auth: auth?.auth ?? null,
      authUnsupported: auth?.unsupported ?? null,
      body:
        body.mode === 'json' || body.mode === 'text'
          ? body.raw
          : body.mode === 'graphql'
            ? body.spec.query
            : null,
      bodyFields:
        body.mode === 'urlencoded' || body.mode === 'multipart'
          ? body.fields.filter((f) => f.enabled).map((f) => ({ key: f.key, value: f.value }))
          : [],
      bodyFileFields:
        body.mode === 'multipart' ? body.fields.filter((f) => f.value_type === 'file_path').map((f) => f.key) : [],
      bodyVariables: body.mode === 'graphql' ? body.spec.variables : '',
      bodyPath: body.mode === 'binary' ? body.path : '',
      bodyKind: body.mode === 'none' ? 'none' : (body.mode as ScriptStep['bodyKind']),
      condition: s.condition,
      loop: s.loop,
      waitMs: s.waitMs,
      parallel: s.parallel,
      extract: s.extract as ScenarioExtractRule[],
      assertions: s.assertions as unknown as ScenarioAssertionRule[],
    }
  })
}

/** AuthSpec → 可静态映射的脚本鉴权（不可映射的给名字生成注释）。 */
function staticAuth(auth: import('../../types/miyoApi').AuthSpec): {
  auth: ScriptAuth | null
  unsupported: string | null
} {
  switch (auth.type) {
    case 'bearer':
      return { auth: { kind: 'bearer', token: auth.token }, unsupported: null }
    case 'basic':
      return { auth: { kind: 'basic', username: auth.username, password: auth.password }, unsupported: null }
    case 'apikey':
      return auth.in === 'header'
        ? { auth: { kind: 'apikey_header', key: auth.key, value: auth.value }, unsupported: null }
        : { auth: { kind: 'apikey_query', key: auth.key, value: auth.value }, unsupported: null }
    case 'oauth2':
      return { auth: null, unsupported: 'OAuth2' }
    case 'digest':
      return { auth: null, unsupported: 'Digest' }
    case 'hawk':
      return { auth: null, unsupported: 'Hawk' }
    case 'awsv4':
      return { auth: null, unsupported: 'AWS SigV4' }
    case 'hmac':
      return { auth: null, unsupported: 'HMAC' }
    case 'dynamic_signature':
      return { auth: null, unsupported: 'DynamicSignature' }
    default:
      return { auth: null, unsupported: null }
  }
}

const mergedVars = computed(() =>
  mergedVariableMap({
    globalVariables: workspace.globalVariables,
    projectVariables: workspace.project?.variables ?? null,
    env: workspace.environments.find((e) => e.id === workspace.activeEnvId),
  }),
)
const envBaseUrlValue = computed(() => mergedVars.value['base_url'] ?? '')

function generateScriptText(): string {
  return generateScenarioScript(name.value.trim() || t('integration.newName'), toScriptSteps(), {
    base: envBaseUrlValue.value,
    stopOnFailure: run.stopOnFailure.value,
    envDefaults: Object.fromEntries(
      Object.entries(mergedVars.value).filter(([k]) => !k.startsWith('$')),
    ),
  })
}

/** 手动视图改动后切到脚本视图：重新生成（未自定义时实时跟随）。 */
watch(viewMode, (v) => {
  if (v === 'script' && !scriptDraft.value.trim()) {
    scriptDraft.value = generateScriptText()
  }
})

async function regenerateScript(): Promise<void> {
  scriptDraft.value = generateScriptText()
}

/** 应用脚本到步骤（反向解析；内联步骤形态）。 */
function applyScriptToSteps(): void {
  try {
    const parsed = parseScenarioScript(scriptDraft.value)
    steps.value = parsed.steps.map((p) => {
      const inlineStep = emptyInline()
      inlineStep.method = p.method || 'GET'
      inlineStep.url = p.url
      inlineStep.spec.body =
        p.bodyKind === 'json'
          ? { mode: 'json', raw: p.body ?? '' }
          : p.bodyKind === 'text'
            ? { mode: 'text', raw: p.body ?? '' }
            : p.bodyKind === 'graphql'
              ? { mode: 'graphql', spec: { query: p.body ?? '', variables: p.bodyVariables, operation_name: '' } }
              : p.bodyKind === 'urlencoded'
                ? {
                    mode: 'urlencoded',
                    fields: p.bodyFields.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '' })),
                  }
                : { mode: 'none' }
      inlineStep.spec.params = p.params.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '' }))
      // 头无法与鉴权区分（解析器并入 headers），按普通头落盘。
      inlineStep.spec.headers = p.headers.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '' }))
      return {
        key: newStepKey(),
        enabled: true,
        kind: 'inline' as const,
        endpointId: null,
        name: p.name,
        inline: inlineStep,
        extract: p.extract.map((e) => ({ name: e.name, from: e.from, path: e.path })),
        condition: null,
        loop: null,
        waitMs: null,
        parallel: false,
        overrides: null,
        assertions: p.assertions.map((a) => ({
          name: a.name ?? '',
          type: a.type ?? 'status',
          path: a.path ?? '',
          op: a.op ?? 'eq',
          expected: a.expected == null ? '' : String(a.expected),
        })),
      }
    })
    scriptDraft.value = scriptDraft.value // 保留脚本
    toast.success(t('integration.saved'))
  } catch (err) {
    if (err instanceof ScriptParseError) {
      toast.error(t('importdlg.parseFail'), { message: err.message })
    } else {
      toast.error(t('integration.saveFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }
}

/** 导出执行结果 Markdown 报告（目录选择 + 写文件）。 */
async function exportRunReport(markdown: string): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, title: t('integration.resultExport') })
    if (!dir || Array.isArray(dir)) return
    const base = (name.value.trim() || t('integration.newName')).replace(/[\\/:*?"<>|\s]+/g, '_') || 'run'
    const path = await join(dir, `${base}-report.md`)
    await api.writeTextFile(path, markdown)
    toast.success(t('integration.resultExported'), { message: path })
  } catch (err) {
    toast.error(t('integration.exportFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function exportScript(): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, title: t('scenario.scriptExportTitle') })
    if (!dir || Array.isArray(dir)) return
    const base = (name.value.trim() || t('integration.newName')).replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await join(dir, `${base}.mjs`)
    await api.writeTextFile(path, scriptDraft.value)
    toast.success(t('scenario.scriptExported'), { message: path })
  } catch (err) {
    toast.error(t('scenario.scriptExportFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function copyScript(): Promise<void> {
  try {
    await copyText(scriptDraft.value)
    toast.success(t('scenario.scriptCopied'))
  } catch {
    toast.error(t('scenario.scriptCopyFail'))
  }
}

// ---------- 历史 / 比较 / 运行菜单 ----------
const compareOpen = ref(false)

/** Dock 历史菜单入口：上下文标记为 history，选中项走 run.onHistorySelect。 */
function onHistoryMenu(e: MouseEvent): void {
  menuContext.value = 'history'
  run.openHistoryMenu(e)
}

/** 步骤卡行内菜单：从此步执行 / 仅执行此步。 */
function openStepMenu(e: MouseEvent, s: EditorStep): void {
  menuContext.value = 'step'
  const idx = steps.value.indexOf(s)
  const items: MenuItem[] = [
    { key: 'run-from', label: t('integration.runFromStep'), icon: 'play' as const },
    { key: 'run-only', label: t('integration.runSingleStep'), icon: 'send' as const },
    { key: 'dup', label: t('integration.stepDuplicate'), icon: 'copy' as const },
  ]
  // 引用式步骤可转内联（复制接口当前配置为步骤私有请求）。
  if (s.kind === 'ref' && endpointOf(s)) {
    items.push({ key: 'to-inline', label: t('integration.toInline'), icon: 'pencil' as const })
  }
  items.push({ key: 'del', label: t('integration.stepDelete'), icon: 'trash' as const, danger: true })
  menuRef.value?.openAt(e.currentTarget as HTMLElement, items)
  pendingMenuStep.value = { step: s, idx }
}

const pendingMenuStep = ref<{ step: EditorStep; idx: number } | null>(null)
/** 当前打开的菜单：'step' = 步骤行内；'add-endpoint' = 从接口添加；'history' = 运行历史。 */
const menuContext = ref<'step' | 'add-endpoint' | 'history'>('step')

function onMenuSelect(item: MenuItem): void {
  if (menuContext.value === 'history') {
    // 运行历史项（key = runId）：回读详情并进入结果视图。
    void onHistorySelect(item)
    return
  }
  if (menuContext.value === 'add-endpoint') {
    const ep = workspace.endpoints.find((e) => e.id === item.key)
    if (ep) addRefStep(ep)
    return
  }
  const target = pendingMenuStep.value
  pendingMenuStep.value = null
  if (!target) return
  if (item.key === 'dup') duplicateStep(target.step)
  else if (item.key === 'to-inline') convertToInline(target.step)
  else if (item.key === 'del') removeStep(target.step)
  else if (item.key === 'run-from') requestRunFrom(target.idx, false)
  else if (item.key === 'run-only') requestRunFrom(target.idx, true)
}

// ---------- 定位 ----------
/** 引用式步骤跳调试器：进入工作区并聚焦该接口（focus query 复用首页定位链路）。 */
function gotoEndpoint(s: EditorStep): void {
  if (!s.endpointId) return
  void router.push({ path: '/workspace', query: { focus: s.endpointId } })
}

function locateStep(pos: number): void {
  viewMode.value = 'manual'
  const s = steps.value[pos]
  if (s) {
    selectStep(s)
    void nextTick(() => {
      const el = stepsEl.value?.querySelector(`[data-step-key="${s.key}"]`)
      el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
    })
  }
}

/** 结果面板运行位次 → 步骤名 / 方法（ScenarioRunReport 占位回调）。 */
function runStepName(pos: number): string {
  const s = steps.value[run.runStepIndexes.value[pos]]
  return s ? stepTitle(s) : ''
}
function runStepMethod(pos: number): string {
  const s = steps.value[run.runStepIndexes.value[pos]]
  return s ? stepMethod(s) : 'GET'
}

</script>

<template>
  <div class="wf-root">
    <Menu ref="menuRef" @select="onMenuSelect" />
    <Modal
      v-model:open="confirmRunOpen"
      :title="t('integration.sideEffectConfirmTitle')"
      width="440px"
    >
      <p class="wf-confirm-body">
        {{ t('integration.sideEffectConfirmBody', { n: writeStepTitles.length, methods: writeStepTitles.join('、') }) }}
      </p>
      <p v-if="envProdLike" class="wf-confirm-warn">
        <Icon name="alert-triangle" :size="13" /> {{ t('integration.sideEffectEnvWarning') }}
      </p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="confirmRunOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="confirmRun">
          <Icon name="play" :size="12" /> {{ t('integration.run') }}
        </button>
      </template>
    </Modal>

    <RunCompareModal v-model:open="compareOpen" :scenario-id="props.scenarioId" />

    <!-- 单步 / 从步骤执行：无运行时快照时手填前置变量，绝不隐式传空变量 -->
    <Modal
      v-model:open="varsFillOpen"
      :title="t('integration.varsFillTitle')"
      width="460px"
    >
      <p class="wf-confirm-body">{{ t('integration.varsFillBody') }}</p>
      <div class="wf-varsfill">
        <div v-for="(v, k) in varsFillMissing" :key="v" class="wf-varsfill-row">
          <span class="wf-varsfill-name" :title="v">{{ v }}</span>
          <input
            v-model="varsFillValues[v]"
            class="miyo-input wf-varsfill-input"
            type="text"
            spellcheck="false"
            :placeholder="t('integration.varsFillPh')"
          >
          <IconButton
            v-if="!varsFillRequired.includes(v)"
            name="x"
            :size="12"
            :title="t('common.delete')"
            @click="delete varsFillValues[v]; k"
          />
        </div>
        <div class="wf-varsfill-row">
          <input
            v-model="varsFillNewName"
            class="miyo-input wf-varsfill-input"
            type="text"
            spellcheck="false"
            :placeholder="t('integration.varsFillNamePh')"
          >
          <button class="miyo-btn miyo-btn-sm" type="button" :disabled="!varsFillNewName.trim()" @click="addVarsFillRow">
            <Icon name="plus" :size="12" /> {{ t('integration.varsFillAdd') }}
          </button>
        </div>
      </div>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="varsFillOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!varsFillReady"
          @click="confirmVarsFill"
        >
          <Icon name="play" :size="12" /> {{ t('integration.run') }}
        </button>
      </template>
    </Modal>

    <!-- ===== 顶栏 ===== -->
    <header class="wf-topbar">
      <IconButton name="chevron-right" :size="14" :title="t('integration.back')" @click="emit('back')" />
      <input v-model="name" class="wf-name" type="text" :placeholder="t('integration.editorNamePh')" spellcheck="false" />
      <input v-model="category" class="wf-category" type="text" :placeholder="t('integration.categoryLabel')" spellcheck="false" />
      <SegmentedControl v-model="viewMode" :options="VIEW_OPTIONS" size="sm" />
      <span class="wf-save-state" :class="saveStateClass" :title="saveStateText">
        <span class="wf-save-dot"></span>{{ saveStateText }}
      </span>
      <div class="wf-topbar-spacer"></div>
      <CustomSelect
        class="wf-strategy"
        :model-value="String(run.execStrategy.value)"
        :options="run.STRATEGY_OPTIONS.value"
        size="sm"
        :pop-min-width="150"
        @update:model-value="(v) => (run.execStrategy.value = String(v ?? 'stop'))"
      />
      <IconButton name="history" :size="14" :title="t('integration.history')" :disabled="!run.runHistory.value.length" @click.stop="run.openHistoryMenu($event)" />
      <IconButton name="layout-grid" :size="14" :title="t('integration.compare')" :disabled="!run.runHistory.value.length" @click="compareOpen = true" />
      <IconButton name="download" :size="14" :title="t('integration.exportScript')" @click="exportScript" />
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="saving" @click="save">
        <Icon name="save" :size="12" /> {{ t('integration.save') }}
      </button>
      <button
        class="miyo-btn miyo-btn-sm"
        :class="run.running.value ? 'miyo-btn-danger-solid' : 'miyo-btn-primary'"
        type="button"
        @click="onRunClick"
      >
        <Icon :name="run.running.value ? 'stop' : 'play'" :size="12" />
        {{ run.running.value ? t('integration.stop') : t('integration.run') }}
      </button>
    </header>

    <!-- 运行进度条：只在运行中显示，避免常驻占位造成布局跳动 -->
    <div v-if="run.running.value" class="wf-progress">
      <div class="wf-progress-bar" :style="{ width: `${Math.max(2, run.progressPercent.value)}%` }"></div>
      <span class="wf-progress-text">{{ run.progress.value }} / {{ run.progressPercent.value }}%</span>
    </div>

    <!-- ===== 执行结果视图（独立模式；数据为落库快照） ===== -->
    <RunResultView
      v-if="viewMode === 'result' && run.runResult.value"
      :scenario-name="name.trim() || t('integration.newName')"
      :run-id="run.runId.value"
      :running="run.running.value"
      :started-at="run.currentRunStartedAt.value"
      :finished-at="run.currentRunAt.value"
      :result="run.runResult.value"
      :var-chain="varChain"
      @back="viewMode = 'manual'"
      @rerun="onRunClick"
      @export="exportRunReport"
    />

    <!-- ===== 脚本视图 ===== -->
    <div v-if="viewMode === 'script'" class="wf-script">
      <div class="wf-script-actions">
        <button class="miyo-btn miyo-btn-sm" type="button" @click="regenerateScript">
          <Icon name="refresh" :size="12" /> {{ t('scenario.scriptReset') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="applyScriptToSteps">
          {{ t('scenario.syncToSteps') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="copyScript">
          {{ t('common.copy') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="exportScript">
          <Icon name="download" :size="12" /> {{ t('integration.exportScript') }}
        </button>
      </div>
      <JsonCodeMirror
        v-model="scriptDraft"
        language="javascript"
        class="wf-script-editor"
      />
    </div>

    <!-- ===== 编排视图 ===== -->
    <div v-else-if="viewMode === 'manual'" class="wf-main">
      <section class="wf-flow">
        <div class="wf-flow-head">
          <span class="wf-flow-title">{{ t('integration.libTitle') }}</span>
          <span class="wf-flow-count">{{ t('integration.libSteps', { n: steps.length }) }}</span>
          <div class="wf-flow-spacer"></div>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="addInlineStep">
            <Icon name="plus" :size="12" /> {{ t('integration.addStep') }}
          </button>
          <button
            class="miyo-btn miyo-btn-sm"
            type="button"
            :disabled="!workspace.endpoints.length"
            @click="(e) => openEndpointMenu(e.currentTarget as HTMLElement)"
          >
            <Icon name="file-plus" :size="12" /> {{ t('integration.addFromEndpoint') }}
          </button>
        </div>

        <!-- 运行前校验面板：有阻断才自动展开；可收起，面板内可「仍然运行」 -->
        <div v-if="issues.length" class="wf-validate" :class="{ collapsed: !validateOpen }">
          <button class="wf-validate-head" type="button" @click="validateOpen = !validateOpen">
            <Icon name="chevron-right" :size="12" class="wf-validate-caret" />
            <span class="wf-validate-title">{{ t('integration.validateTitle') }}</span>
            <span v-if="errorCount" class="wf-validate-tag error">{{ t('integration.validateErrors', { n: errorCount }) }}</span>
            <span v-if="warnCount" class="wf-validate-tag warn">{{ t('integration.validateWarnings', { n: warnCount }) }}</span>
            <div class="wf-validate-spacer"></div>
            <span v-if="!validateOpen" class="wf-validate-hint">{{ t('integration.validateExpand') }}</span>
          </button>
          <ul v-if="validateOpen" class="wf-validate-list">
            <li
              v-for="(is, k) in issues"
              :key="k"
              class="wf-validate-item"
              :class="is.severity"
              role="button"
              tabindex="0"
              @click="gotoIssue(is)"
              @keydown.enter="gotoIssue(is)"
            >
              <span class="wf-validate-sev">{{ is.severity === 'error' ? '!' : '·' }}</span>
              <span class="wf-validate-where">
                {{ is.stepIndex >= 0 ? t('integration.validateStepAt', { n: is.stepIndex + 1 }) : t('integration.validateScene') }}
              </span>
              <span class="wf-validate-msg">{{ t(is.message, is.detail ?? {}) }}</span>
              <span class="wf-validate-go">{{ t('integration.validateGoto') }}</span>
            </li>
          </ul>
          <div v-if="validateOpen && errorCount" class="wf-validate-foot">
            <span class="wf-validate-foot-hint">{{ t('integration.validateForceHint') }}</span>
            <button class="miyo-btn miyo-btn-sm" type="button" @click="forceRun">
              {{ t('integration.validateForceRun') }}
            </button>
          </div>
        </div>

        <div ref="stepsEl" class="wf-steps">
          <!-- 入口连线：链路起点 -->
          <div v-if="steps.length" class="wf-edge wf-edge-entry">
            <span class="wf-edge-dot"></span>
            <span class="wf-edge-label">{{ t('integration.flowStart') }}</span>
          </div>
          <div
            v-for="(s, i) in steps"
            :key="s.key"
            class="wf-step-wrap"
            :data-step-key="s.key"
          >
            <!-- 步骤间连线：挂上从前序步骤流转过来的变量名 -->
            <div
              v-if="edgeInto.get(i)"
              class="wf-edge"
              :class="{
                muted: !s.enabled || edgeInto.get(i)!.muted,
                dash: edgeInto.get(i)!.parallel,
                loop: !!(s.loop || edgeInto.get(i)!.vars.length),
              }"
            >
              <span class="wf-edge-line"></span>
              <span v-if="edgeInto.get(i)!.vars.length" class="wf-edge-chips">
                <span
                  v-for="v in edgeVarsLabel(edgeInto.get(i)!).shown"
                  :key="v"
                  class="wf-edge-chip"
                  :title="t('integration.flowVarsTitle', { name: v })"
                >{{ v }}</span>
                <span v-if="edgeVarsLabel(edgeInto.get(i)!).more" class="wf-edge-chip more">
                  +{{ edgeVarsLabel(edgeInto.get(i)!).more }}
                </span>
              </span>
              <span
                v-else-if="edgeInto.get(i)!.externalVars.length"
                class="wf-edge-chip ext"
                :title="t('integration.flowExternalTitle', { names: edgeInto.get(i)!.externalVars.join(', ') })"
              >{{ t('integration.flowExternal', { n: edgeInto.get(i)!.externalVars.length }) }}</span>
              <span v-if="s.loop" class="wf-edge-loop" :title="loopBadgeTitle(s.loop)">
                {{ t('integration.flowLoop', { n: loopBadgeCount(s.loop) }) }}
              </span>
            </div>

            <div
              class="wf-step"
              :class="{
                selected: selectedKey === s.key,
                disabled: !s.enabled,
                invalid: stepIssues(i).some((x) => x.severity === 'error'),
                batch: s.parallel,
              }"
              role="button"
              tabindex="0"
              @click="selectStep(s)"
              @keydown.enter="selectStep(s)"
            >
            <span class="wf-drag" :title="t('integration.addStep')" aria-hidden="true">⋮⋮</span>
            <span class="wf-step-index num">{{ i + 1 }}</span>
            <span class="wf-step-method" :class="methodBadgeTone(stepMethod(s))">{{ stepMethod(s) }}</span>
            <div class="wf-step-main">
              <span class="wf-step-name" :title="stepTitle(s)">{{ stepTitle(s) }}</span>
              <span class="wf-step-path" :title="stepPath(s)">{{ stepPath(s) }}</span>
            </div>
            <span v-if="isWriteStep(s)" class="wf-step-side" :title="t('integration.sideEffectWarn')">
              {{ t('integration.sideEffectWarn') }}
            </span>
            <span
              v-if="run.stepTone(i)"
              class="wf-step-tone"
              :class="`tone-${run.stepTone(i)}`"
            >{{ t(run.STEP_TONE_TEXT[run.stepTone(i) as NonNullable<ReturnType<typeof run.stepTone>>]) }}</span>
            <span class="wf-step-badges">
              <span v-if="s.loop" class="wf-badge wf-badge-accent" :title="loopBadgeTitle(s.loop)">
                {{ t('integration.loopBadge', { n: loopBadgeCount(s.loop) }) }}
              </span>
              <span v-if="s.condition" class="wf-badge" :title="conditionBadgeTitle(s.condition)">
                {{ t('integration.conditionBadge') }}
              </span>
              <span v-if="s.parallel" class="wf-badge" :title="t('integration.parallelBadge')">
                {{ t('integration.parallelBadge') }}
              </span>
              <span v-if="(s.waitMs ?? 0) > 0" class="wf-badge" :title="t('integration.waitBadge', { n: s.waitMs ?? 0 })">
                {{ t('integration.waitBadge', { n: s.waitMs ?? 0 }) }}
              </span>
              <span v-if="s.extract.length" class="wf-badge">{{ t('scenario.stepExtractBadge', { n: s.extract.length }) }}</span>
              <span v-if="s.assertions.length" class="wf-badge">{{ t('scenario.stepAssertBadge', { n: s.assertions.length }) }}</span>
            </span>
            <span class="wf-step-actions" @click.stop>
              <button
                class="wf-act"
                type="button"
                :class="{ off: !s.enabled }"
                :title="t('integration.stepEnabledHint')"
                @click="s.enabled = !s.enabled"
              >
                <Icon :name="s.enabled ? 'eye' : 'eye-off'" :size="12" />
              </button>
              <button class="wf-act" type="button" :title="t('integration.stepDuplicate')" @click="duplicateStep(s)">
                <Icon name="copy" :size="12" />
              </button>
              <button class="wf-act" type="button" :title="t('workspace.moreTools')" @click="openStepMenu($event, s)">
                <Icon name="more-horizontal" :size="12" />
              </button>
              <Popconfirm
                :title="t('integration.stepDelete')"
                :confirm-text="t('common.delete')"
                @confirm="removeStep(s)"
              >
                <button class="wf-act" type="button" :title="t('integration.stepDelete')" @click.stop>
                  <Icon name="trash" :size="12" />
                </button>
              </Popconfirm>
            </span>
            <span v-if="stepIssues(i).length" class="wf-step-issue" :class="{ err: stepIssues(i).some((x) => x.severity === 'error') }" :title="stepIssues(i).map((x) => t(x.message, x.detail ?? {})).join('\n')">
              {{ stepIssues(i).length }}
            </span>
          </div>
          </div>
          <div v-if="!steps.length" class="wf-steps-empty">
            <Icon name="beaker" :size="20" />
            <p>{{ t('integration.libEmptyDesc') }}</p>
            <button class="miyo-btn miyo-btn-sm" type="button" @click="addInlineStep">
              <Icon name="plus" :size="12" /> {{ t('integration.addInline') }}
            </button>
          </div>
        </div>

        <!-- 运行结果 Dock（复用） -->
        <ScenarioRunReport
          v-model:open="run.reportOpen.value"
          :running="run.running.value"
          :progress="run.progress.value"
          :progress-percent="run.progressPercent.value"
          :run-result="run.runResult.value"
          :run-is-history="run.runIsHistory.value"
          :current-run-at="run.currentRunAt.value"
          :live-rows="run.liveRows.value"
          :live-pending="run.livePending.value"
          :resp-open-rows="run.respOpenRows.value"
          :run-summary="run.runSummary.value"
          :final-state="run.finalState.value"
          :rerun-available="run.rerunAvailable.value"
          :has-history="run.runHistory.value.length > 0"
          :rerun-history-available="run.rerunHistoryAvailable.value"
          :step-name="runStepName"
          :step-method="runStepMethod"
          :full-body-loader="run.fullBodyLoader"
          @locate="locateStep"
          @rerun="run.rerunFailed"
          @rerun-history="run.rerunFromHistory"
          @history="onHistoryMenu"
          @toggle-row="run.toggleRespRow"
        />
      </section>

      <!-- ===== 右栏：步骤详情 ===== -->
      <aside class="wf-detail" :class="{ empty: !selectedStep }">
        <template v-if="selectedStep">
          <div class="wf-detail-head">
            <input v-model="selectedStep.name" class="miyo-input wf-detail-name" type="text" :placeholder="t('integration.stepUntitled', { n: steps.indexOf(selectedStep) + 1 })" spellcheck="false" />
            <span class="wf-detail-kind">{{ selectedStep.kind === 'inline' ? t('integration.stepKindInline') : t('integration.stepKindRef') }}</span>
            <IconButton name="x" :size="13" :title="t('common.close')" @click="selectStep(null)" />
          </div>
          <Tabs v-model="detailTab" :tabs="detailTabs" size="sm" />
          <div class="wf-detail-body">
            <!-- 覆盖策略三态（仅引用式步骤）：数据上只体现为 overrides 内容，无新增字段 -->
            <div v-if="selectedStep.kind === 'ref'" class="wf-policy">
              <span class="wf-policy-label">{{ t('integration.policyLabel') }}</span>
              <CustomSelect
                class="wf-policy-select"
                :model-value="stepPolicy(selectedStep)"
                :options="POLICY_OPTIONS"
                size="sm"
                :pop-min-width="150"
                @update:model-value="(v) => onPolicyChange(selectedStep!, String(v ?? 'merge'))"
              />
              <span class="wf-policy-hint">{{ t(`integration.policyHint.${stepPolicy(selectedStep)}`) }}</span>
            </div>
            <StepRequestEditor
              v-show="detailTab === 'request'"
              :kind="selectedStep.kind"
              :inline="selectedStep.kind === 'inline' ? selectedStep.inline : null"
              :endpoint="selectedEndpoint"
              :step-key="selectedStep.key"
              :overrides="selectedStep.kind === 'ref' ? ensureOverrides(selectedStep) : null"
              @goto-endpoint="gotoEndpoint(selectedStep)"
            />
            <div v-show="detailTab === 'control'" class="wf-rules wf-control">
              <!-- 执行条件 -->
              <div class="wf-ctrl-block">
                <div class="wf-ctrl-head">
                  <span class="wf-ctrl-title">{{ t('integration.condTitle') }}</span>
                  <button
                    v-if="!selectedStep.condition"
                    class="miyo-btn miyo-btn-sm"
                    type="button"
                    @click="selectedStep.condition = { source: 'var', name: '', op: 'exists', value: null }"
                  >
                    <Icon name="plus" :size="12" /> {{ t('integration.condAdd') }}
                  </button>
                  <IconButton
                    v-else
                    name="trash"
                    :size="13"
                    :title="t('common.delete')"
                    @click="selectedStep.condition = null"
                  />
                </div>
                <p class="wf-ctrl-hint">{{ t('integration.condHint') }}</p>
                <div v-if="selectedStep.condition" class="wf-rule-row">
                  <CustomSelect
                    class="wf-ctrl-src"
                    :model-value="String(selectedStep.condition.source ?? 'var')"
                    :options="[
                      { value: 'var', label: t('integration.condSourceVar') },
                      { value: 'prevStatus', label: t('integration.condSourcePrevStatus') },
                    ]"
                    size="sm"
                    :pop-min-width="140"
                    @update:model-value="(v) => onCondSourceChange(selectedStep!.condition!, String(v ?? 'var'))"
                  />
                  <input
                    v-if="(selectedStep.condition.source ?? 'var') === 'var'"
                    v-model="selectedStep.condition.name"
                    class="miyo-input wf-rule-name"
                    type="text"
                    spellcheck="false"
                    :placeholder="t('integration.condNamePh')"
                  />
                  <CustomSelect
                    class="wf-rule-op"
                    :model-value="String(selectedStep.condition.op ?? 'eq')"
                    :options="condOpOptions(selectedStep.condition)"
                    size="sm"
                    :pop-min-width="130"
                    @update:model-value="(v) => (selectedStep!.condition!.op = String(v ?? 'eq'))"
                  />
                  <input
                    v-if="condNeedsValue(selectedStep.condition)"
                    v-model="condValueText"
                    class="miyo-input wf-rule-expected"
                    type="text"
                    spellcheck="false"
                    :placeholder="t('integration.assertExpectedPh')"
                  />
                </div>
              </div>

              <!-- 循环 -->
              <div class="wf-ctrl-block">
                <div class="wf-ctrl-head">
                  <span class="wf-ctrl-title">{{ t('integration.loopTitle') }}</span>
                  <button
                    v-if="!selectedStep.loop"
                    class="miyo-btn miyo-btn-sm"
                    type="button"
                    @click="selectedStep.loop = { mode: 'count', count: 3 }"
                  >
                    <Icon name="plus" :size="12" /> {{ t('integration.loopAdd') }}
                  </button>
                  <IconButton
                    v-else
                    name="trash"
                    :size="13"
                    :title="t('common.delete')"
                    @click="selectedStep.loop = null"
                  />
                </div>
                <p class="wf-ctrl-hint">{{ t('integration.loopHint') }}</p>
                <template v-if="selectedStep.loop">
                  <div class="wf-rule-row">
                    <CustomSelect
                      class="wf-ctrl-src"
                      :model-value="String(selectedStep.loop.mode)"
                      :options="[
                        { value: 'count', label: t('integration.loopModeCount') },
                        { value: 'while', label: t('integration.loopModeWhile') },
                      ]"
                      size="sm"
                      :pop-min-width="130"
                      @update:model-value="(v) => onLoopModeChange(selectedStep!.loop!, String(v ?? 'count'))"
                    />
                    <template v-if="selectedStep.loop.mode === 'count'">
                      <input
                        v-model="loopCountText"
                        class="miyo-input wf-ctrl-num"
                        type="text"
                        inputmode="numeric"
                        spellcheck="false"
                        :placeholder="t('integration.loopCountPh')"
                      />
                    </template>
                    <template v-else>
                      <input
                        v-model="loopMaxText"
                        class="miyo-input wf-ctrl-num"
                        type="text"
                        inputmode="numeric"
                        spellcheck="false"
                        :placeholder="t('integration.loopMaxPh')"
                      />
                    </template>
                    <input
                      v-model="loopIntervalText"
                      class="miyo-input wf-ctrl-num"
                      type="text"
                      inputmode="numeric"
                      spellcheck="false"
                      :placeholder="t('integration.loopIntervalPh')"
                    />
                  </div>
                  <div v-if="selectedStep.loop.mode === 'while'" class="wf-rule-row wf-loop-cond">
                    <span class="wf-ctrl-inline-label">{{ t('integration.loopWhileCond') }}</span>
                    <input
                      v-model="selectedStep.loop.while_condition!.name"
                      class="miyo-input wf-rule-name"
                      type="text"
                      spellcheck="false"
                      :placeholder="t('integration.condNamePh')"
                    />
                    <CustomSelect
                      class="wf-rule-op"
                      :model-value="String(selectedStep.loop.while_condition?.op ?? 'eq')"
                      :options="condOpOptions(selectedStep.loop.while_condition ?? undefined)"
                      size="sm"
                      :pop-min-width="130"
                      @update:model-value="(v) => { if (selectedStep!.loop!.while_condition) selectedStep!.loop!.while_condition.op = String(v ?? 'eq') }"
                    />
                    <input
                      v-if="condNeedsValue(selectedStep.loop.while_condition ?? undefined)"
                      v-model="loopCondValueText"
                      class="miyo-input wf-rule-expected"
                      type="text"
                      spellcheck="false"
                      :placeholder="t('integration.assertExpectedPh')"
                    />
                  </div>
                  <!-- 退出条件：把引擎的终止语义显式写出来，避免用户以为「失败会继续重试」 -->
                  <div class="wf-loop-exit">
                    <div class="wf-loop-exit-row">
                      <span class="wf-loop-exit-badge ok">{{ t('integration.loopExitSuccess') }}</span>
                      <span class="wf-loop-exit-text">
                        {{ selectedStep.loop.mode === 'count' ? t('integration.loopExitCount') : t('integration.loopExitWhile') }}
                      </span>
                    </div>
                    <div class="wf-loop-exit-row">
                      <span class="wf-loop-exit-badge fail">{{ t('integration.loopExitFail') }}</span>
                      <span class="wf-loop-exit-text">{{ t('integration.loopExitFailText') }}</span>
                    </div>
                    <div class="wf-loop-exit-row">
                      <span class="wf-loop-exit-badge cap">{{ t('integration.loopExitCap') }}</span>
                      <span class="wf-loop-exit-text">{{ t('integration.loopExitCapText') }}</span>
                    </div>
                  </div>
                </template>
              </div>

              <!-- 等待与并行 -->
              <div class="wf-ctrl-block">
                <div class="wf-ctrl-head">
                  <span class="wf-ctrl-title">{{ t('integration.waitTitle') }}</span>
                </div>
                <div class="wf-rule-row">
                  <input
                    v-model="waitMsText"
                    class="miyo-input wf-ctrl-num"
                    type="text"
                    inputmode="numeric"
                    spellcheck="false"
                    :placeholder="t('integration.waitPh')"
                  />
                  <label class="wf-ctrl-check">
                    <input v-model="selectedStep.parallel" type="checkbox" />
                    <span>{{ t('integration.parallelCheck') }}</span>
                  </label>
                </div>
                <p class="wf-ctrl-hint">{{ t('integration.waitHint') }}</p>
                <p class="wf-ctrl-hint">{{ t('integration.parallelHint') }}</p>
              </div>
            </div>
            <div v-show="detailTab === 'extract'" class="wf-rules">
              <div class="wf-rules-head">
                <span class="wf-rules-hint">{{ t('integration.extractEmpty') }}</span>
                <button class="miyo-btn miyo-btn-sm" type="button" @click="addExtract">
                  <Icon name="plus" :size="12" /> {{ t('integration.extractAdd') }}
                </button>
              </div>
              <div v-for="(r, j) in selExtract" :key="j" class="wf-rule-row">
                <input v-model="r.name" class="miyo-input wf-rule-name" type="text" :placeholder="t('integration.extractNamePh')" spellcheck="false" />
                <CustomSelect
                  class="wf-rule-from"
                  :model-value="r.from"
                  :options="[
                    { value: 'body', label: t('integration.extractFromBody') },
                    { value: 'header', label: t('integration.extractFromHeader') },
                  ]"
                  size="sm"
                  :pop-min-width="130"
                  @update:model-value="(v) => (r.from = v === 'header' ? 'header' : 'body')"
                />
                <input
                  v-model="r.path"
                  class="miyo-input wf-rule-path"
                  type="text"
                  spellcheck="false"
                  :placeholder="r.from === 'header' ? t('integration.extractHeaderNamePh') : t('integration.extractPathPh')"
                />
                <IconButton name="trash" :size="13" :title="t('common.delete')" @click="selExtract.splice(j, 1)" />
              </div>
            </div>
            <div v-show="detailTab === 'assert'" class="wf-rules">
              <div class="wf-rules-head">
                <span class="wf-rules-hint">{{ t('integration.assertEmpty') }}</span>
                <button class="miyo-btn miyo-btn-sm" type="button" @click="addAssertion">
                  <Icon name="plus" :size="12" /> {{ t('integration.assertAdd') }}
                </button>
              </div>
              <div v-for="(a, j) in selAssertions" :key="j" class="wf-rule-row wf-assert-row">
                <input v-model="a.name" class="miyo-input wf-rule-name" type="text" :placeholder="t('integration.assertNamePh')" spellcheck="false" />
                <CustomSelect
                  class="wf-rule-type"
                  :model-value="a.type"
                  :options="ASSERT_TYPE_KEYS"
                  size="sm"
                  :pop-min-width="150"
                  @update:model-value="(v) => onAssertTypeChange(a, String(v ?? 'status'))"
                />
                <input
                  v-if="needsPath(a.type)"
                  v-model="a.path"
                  class="miyo-input wf-rule-path"
                  type="text"
                  spellcheck="false"
                  :placeholder="t('integration.assertTargetPh')"
                />
                <CustomSelect
                  class="wf-rule-op"
                  :model-value="a.op"
                  :options="assertOpOptions(a.type)"
                  size="sm"
                  :pop-min-width="130"
                  @update:model-value="(v) => (a.op = String(v ?? 'eq'))"
                />
                <input
                  v-if="needsExpected(a)"
                  v-model="a.expected"
                  class="miyo-input wf-rule-expected"
                  type="text"
                  spellcheck="false"
                  :placeholder="t('integration.assertExpectedPh')"
                />
                <IconButton name="trash" :size="13" :title="t('common.delete')" @click="selAssertions.splice(j, 1)" />
              </div>
            </div>
            <ScenarioVarsPanel
              v-show="detailTab === 'vars'"
              :chain="varChain"
              :runtime-vars="run.runResult.value?.variables ?? null"
              @locate="locateStep"
            />
          </div>
        </template>
        <div v-else class="wf-detail-placeholder">
          <Icon name="pencil" :size="18" />
          <p>{{ t('integration.stepSelect') }}</p>
        </div>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.wf-root {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg-app);
  color: var(--text-1);
}

/* ---- 顶栏 ---- */
.wf-topbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
}

.wf-name {
  width: 220px;
  font-weight: 600;
}

.wf-category {
  width: 120px;
}

.wf-topbar-spacer {
  flex: 1;
}

.wf-strategy {
  width: 130px;
}

/* ---- 脚本视图 ---- */
.wf-script {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 10px 12px;
  gap: 8px;
}

.wf-script-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.wf-script-editor {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border-editor, var(--border));
  border-radius: var(--radius-md);
  overflow: hidden;
}

/* ---- 编排视图 ---- */
.wf-main {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 12px;
  padding: 10px 12px;
}

.wf-flow {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.wf-flow-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.wf-flow-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.wf-flow-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-flow-spacer {
  flex: 1;
}

.wf-steps {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0;
  padding-right: 2px;
}

/* ---- 工作流连线（步骤间的变量流向可视） ---- */
.wf-step-wrap {
  display: flex;
  flex-direction: column;
  padding-bottom: 6px;
}

.wf-edge {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 18px;
  padding-left: 30px;
  min-width: 0;
}

.wf-edge-entry {
  min-height: 16px;
}

.wf-edge-line {
  flex: none;
  width: 1px;
  height: 100%;
  min-height: 14px;
  background: var(--border-strong);
}

/* 虚线段：批内并行 / 入口 */
.wf-edge.dash .wf-edge-line {
  background: repeating-linear-gradient(to bottom, var(--border-strong) 0 3px, transparent 3px 6px);
}

.wf-edge.muted .wf-edge-line {
  opacity: 0.4;
}

.wf-edge-dot {
  flex: none;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent);
  margin-left: 28px;
}

.wf-edge-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  letter-spacing: 0.04em;
}

.wf-edge-chips {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  flex-wrap: wrap;
}

.wf-edge-chip {
  flex: none;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0 5px;
  border-radius: var(--radius-sm);
  background: var(--accent-tint);
  color: var(--accent);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  line-height: 15px;
}

.wf-edge-chip.ext {
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-edge-chip.more {
  background: transparent;
  color: var(--text-3);
}

.wf-edge.muted .wf-edge-chip {
  opacity: 0.5;
}

.wf-edge-loop {
  flex: none;
  padding: 0 5px;
  border-radius: var(--radius-sm);
  border: 1px dashed var(--border-strong);
  color: var(--text-3);
  font-size: var(--fs-xxs);
  line-height: 14px;
}

/* 批次内的并行步骤：左侧色条标出「同批并发」 */
.wf-step.batch {
  box-shadow: inset 2px 0 0 var(--accent);
}

.wf-step.invalid {
  border-color: var(--danger);
}

.wf-step-issue {
  flex: none;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--bg-hover);
  color: var(--text-3);
  font-size: var(--fs-xxs);
  line-height: 16px;
  text-align: center;
}

.wf-step-issue.err {
  background: var(--danger);
  color: #fff;
}

/* ---- 运行前校验面板 ---- */
.wf-validate {
  flex: none;
  margin: 0 0 6px;
  border-radius: var(--radius-md);
  background: var(--bg-panel);
}

.wf-validate-head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  cursor: pointer;
  text-align: left;
}

.wf-validate-caret {
  flex: none;
  transition: transform 0.15s ease;
  transform: rotate(90deg);
}

.wf-validate.collapsed .wf-validate-caret {
  transform: rotate(0deg);
}

.wf-validate-title {
  font-weight: 600;
}

.wf-validate-tag {
  padding: 0 5px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  line-height: 16px;
}

.wf-validate-tag.error {
  background: var(--danger);
  color: #fff;
}

.wf-validate-tag.warn {
  background: var(--warning);
  color: #1a1a1a;
}

.wf-validate-spacer {
  flex: 1;
}

.wf-validate-hint {
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

.wf-validate-list {
  list-style: none;
  margin: 0;
  padding: 0 0 4px;
  max-height: 180px;
  overflow-y: auto;
}

.wf-validate-item {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 3px 8px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  border-radius: var(--radius-sm);
}

.wf-validate-item:hover {
  background: var(--bg-hover);
}

.wf-validate-sev {
  flex: none;
  width: 10px;
  text-align: center;
  font-weight: 700;
}

.wf-validate-item.error .wf-validate-sev {
  color: var(--danger);
}

.wf-validate-item.warning .wf-validate-sev {
  color: var(--warning);
}

.wf-validate-where {
  flex: none;
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
}

.wf-validate-msg {
  flex: 1;
  min-width: 0;
}

.wf-validate-go {
  flex: none;
  opacity: 0;
  color: var(--accent);
  font-size: var(--fs-xxs);
  transition: opacity 0.12s ease;
}

.wf-validate-item:hover .wf-validate-go {
  opacity: 1;
}

/* ---- 顶栏保存状态 ---- */
.wf-save-state {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

.wf-save-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--text-3);
}

.wf-save-state.dirty {
  color: var(--warning);
}

.wf-save-state.dirty .wf-save-dot {
  background: var(--warning);
}

.wf-save-state.saving .wf-save-dot {
  background: var(--accent);
  animation: wf-pulse 1s ease-in-out infinite;
}

@keyframes wf-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.25;
  }
}

@media (prefers-reduced-motion: reduce) {
  .wf-save-state.saving .wf-save-dot {
    animation: none;
  }
}

/* ---- 运行进度条 ---- */
.wf-progress {
  flex: none;
  position: relative;
  height: 2px;
  background: var(--bg-hover);
}

.wf-progress-bar {
  height: 100%;
  background: var(--accent);
  transition: width 0.25s ease;
}

.wf-progress-text {
  position: absolute;
  right: 8px;
  top: 3px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 循环退出条件说明 ---- */
.wf-loop-exit {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 6px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
}

.wf-loop-exit-row {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.wf-loop-exit-badge {
  flex: none;
  padding: 0 5px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  line-height: 15px;
}

.wf-loop-exit-badge.ok {
  background: color-mix(in srgb, var(--success) 18%, transparent);
  color: var(--success);
}

.wf-loop-exit-badge.fail {
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
}

.wf-loop-exit-badge.cap {
  background: color-mix(in srgb, var(--warning) 20%, transparent);
  color: var(--warning);
}

.wf-loop-exit-text {
  flex: 1;
  min-width: 0;
  line-height: 1.5;
}

/* ---- 校验面板底部：忽略阻断仍然运行 ---- */
.wf-validate-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px 6px;
  border-top: 1px solid var(--border);
}

.wf-validate-foot-hint {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 手填变量弹窗 ---- */
.wf-varsfill {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 260px;
  overflow-y: auto;
  margin-top: 8px;
}

.wf-varsfill-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.wf-varsfill-name {
  flex: none;
  width: 130px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--accent);
}

.wf-varsfill-input {
  flex: 1;
  min-width: 0;
}

/* ---- 覆盖策略 ---- */
.wf-policy {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 0 8px;
  margin-bottom: 8px;
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}

.wf-policy-label {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-policy-select {
  width: 110px;
}

.wf-policy-hint {
  flex: 1;
  min-width: 120px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-step {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  cursor: pointer;
  min-width: 0;
}

.wf-step:hover {
  border-color: var(--border-strong);
}

.wf-step.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

.wf-step.disabled {
  opacity: 0.55;
}

.wf-ghost {
  opacity: 0.4;
}

.wf-drag {
  flex: none;
  cursor: grab;
  color: var(--text-3);
  font-size: 10px;
  letter-spacing: -2px;
  user-select: none;
}

.wf-step-index {
  flex: none;
  width: 18px;
  text-align: center;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-step-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}

.wf-step-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.wf-step-name {
  font-size: var(--fs-xs);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-step-path {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-step-side {
  flex: none;
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-tone {
  flex: none;
  font-size: 10px;
  padding: 1px 8px;
  border-radius: 999px;
}

.wf-step-tone.tone-ok {
  background: var(--success-tint);
  color: var(--success);
}

.wf-step-tone.tone-fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.wf-step-tone.tone-skip {
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-step-tone.tone-blocked {
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-tone.tone-run {
  background: var(--accent-tint);
  color: var(--accent);
}

.wf-step-tone.tone-wait {
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-step-tone.tone-cancel {
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-badges {
  flex: none;
  display: inline-flex;
  gap: 4px;
}

.wf-badge {
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
  font-family: var(--font-mono);
}

.wf-badge-accent {
  background: var(--accent-tint);
  color: var(--accent);
}

/* 执行控制面板 */
.wf-control {
  gap: 14px;
}

.wf-ctrl-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.wf-ctrl-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.wf-ctrl-title {
  flex: 1;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.wf-ctrl-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.5;
}

.wf-ctrl-src {
  flex: none;
  width: 128px;
}

.wf-ctrl-num {
  width: 110px;
  flex: none;
}

.wf-ctrl-inline-label {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-loop-cond {
  padding-left: 2px;
}

.wf-ctrl-check {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-1);
  cursor: pointer;
  margin-left: auto;
}

.wf-step-actions {
  flex: none;
  display: none;
  align-items: center;
  gap: 2px;
}

.wf-step:hover .wf-step-actions {
  display: inline-flex;
}

.wf-act {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  padding: 3px;
  border-radius: var(--radius-sm);
}

.wf-act:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.wf-act.off {
  color: var(--warning);
}

.wf-steps-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--text-3);
  font-size: var(--fs-xs);
}

.wf-steps-empty p {
  margin: 0;
  max-width: 380px;
  text-align: center;
  line-height: 1.6;
}

/* ---- 右栏详情 ---- */
.wf-detail {
  flex: none;
  width: 420px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  padding: 10px 12px;
  overflow: hidden;
}

.wf-detail.empty {
  align-items: center;
  justify-content: center;
}

.wf-detail-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.wf-detail-name {
  flex: 1;
  min-width: 0;
}

.wf-detail-kind {
  flex: none;
  font-size: 10px;
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-detail-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
}

.wf-detail-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

/* 提取 / 断言行编辑 */
.wf-rules {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.wf-rules-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.wf-rules-hint {
  flex: 1;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.5;
}

.wf-rule-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.wf-rule-name {
  flex: none;
  width: 110px;
}

.wf-rule-from {
  flex: none;
  width: 104px;
}

.wf-rule-type {
  flex: none;
  width: 128px;
}

.wf-rule-op {
  flex: none;
  width: 116px;
}

.wf-rule-path {
  flex: 1;
  min-width: 0;
}

.wf-rule-expected {
  flex: 1;
  min-width: 0;
}

/* 确认弹窗 */
.wf-confirm-body {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
  word-break: break-all;
}

.wf-confirm-warn {
  margin: 8px 0 0;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

/* 窄屏收窄右栏 */
@media (max-width: 1180px) {
  .wf-detail {
    width: 340px;
  }
}
</style>
