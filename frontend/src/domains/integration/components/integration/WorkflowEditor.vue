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
import { type Ref, computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import Sortable from 'sortablejs'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { lazyComponent } from '@/composables/lazyComponent'
import { useToast } from '@/composables/useToast'
import { useTauriEvent } from '@/composables/useTauriEvent'
import {
  useScenarioRun,
  type BuiltRunStep,
  type ScenarioProgressEvent,
} from '@/domains/integration/composables/useScenarioRun'
import { useLifecycleEditor } from '@/domains/integration/composables/useLifecycleEditor'
import type { ValidateIssue, ValidateField } from '@/utils/scenarioValidate'
import { computeFlowGraph, requiredVarsFrom as requiredVarsFromFlow, toFlowSteps } from '@/utils/scenarioFlow'
import { applyPolicy, detectPolicy, OVERRIDE_POLICIES, type OverridePolicy } from '@/utils/overridePolicy'
import { applyStepOverrides, applyUrlOverride } from '@/utils/stepOverrides'
import { envColorClass, mergedVariableMap } from '@/utils/environment'
import { applyPathVars } from '@/utils/pathVariables'
import { methodBadgeTone } from '@/utils/methodTone'
import { generateScenarioScript, type ScriptAuth, type ScriptStep } from '@/utils/scenarioScript'
import { EXPORT_PLACEHOLDER, isSensitiveKeyName, redactJsonText, redactUrlQuery } from '@/utils/exportRedact'
import { parseScenarioScript, ScriptParseError } from '@/utils/scenarioScriptParse'
import { copyText } from '@/utils/clipboard'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import SegmentedControl, { type SegmentOption } from '@/components/ui/SegmentedControl.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Menu, { type MenuItem } from '@/components/ui/Menu.vue'
import Modal from '@/components/ui/Modal.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import Tabs, { type TabItem } from '@/components/ui/Tabs.vue'
import SplitPanes from '@/components/ui/SplitPanes.vue'
const JsonCodeMirror = lazyComponent(() => import('@/components/ui/JsonCodeMirror.vue'))
import ScenarioRunReport from '@/domains/integration/components/scenario/scenario/ScenarioRunReport.vue'
import ScenarioVarsPanel from '@/domains/integration/components/scenario/scenario/ScenarioVarsPanel.vue'
import ExtractList from '@/domains/test-case/components/assertion/assertion/ExtractList.vue'
import AssertionList from '@/domains/test-case/components/assertion/assertion/AssertionList.vue'
import StepRequestEditor from '@/domains/integration/components/integration/StepRequestEditor.vue'
import RunResultView from '@/domains/integration/components/integration/RunResultView.vue'
import RunCompareModal from '@/domains/integration/components/integration/RunCompareModal.vue'
import type {
  Endpoint,
  Scenario,
  ScenarioAssertionRule,
  ScenarioExtractRule,
  ScenarioStep,
  StepCondition,
  StepLoop,
  StepOverrides,
} from '@/types/miyoApi.d'
import type { EditorAssertion, EditorExtract, EditorStep } from '@/domains/integration/components/integration/editorTypes'
import { useScenarioValidation } from '@/domains/integration/composables/useScenarioValidation'
import { errorText } from '@/utils/errorText'

const props = defineProps<{ scenarioId: string }>()
const emit = defineEmits<{ (e: 'back'): void }>()

const router = useRouter()
const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const store = useIntegrationStore()
const api = useMiyoApi()
const toast = useToast()

// ---------- 场景加载与编辑态 ----------
// 编辑态类型共享于 ./editorTypes.ts（校验/脚本视图组合式同样消费）。

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
      disable_cookies: false,
    },
  }
}

let stepSeq = 0
function newStepKey(): string {
  stepSeq += 1
  return `step-${Date.now().toString(36)}-${stepSeq}`
}

const scenario = ref<Scenario | null>(null) as Ref<Scenario | null> // `as Ref`：绕开 UnwrapRef 对深嵌套场景类型的 TS2589（运行时行为不变）
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
      retry: s.retry,
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
  if (!pid) {
    loading.value = false
    return
  }
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
  } catch (err) {
    // 加载失败不再静默吞掉后续初始化：提示后返回列表（此前异常上抛会
    // 跳过 onMounted 的历史恢复 / pendingRunId，且 loading 永久卡 true）。
    toast.error(t('integration.loadFail'), {
      message: errorText(err),
    })
    emit('back')
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
    retry: (s.retry as import('@/types/miyoApi.d').StepRetry | null | undefined) ?? null,
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
      from: r.from === 'header' || r.from === 'cookie' || r.from === 'body_regex' ? r.from : 'body',
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
      type: (r.type ?? 'status') as ScenarioAssertionRule['type'],
      path: typeof r.path === 'string' ? r.path : '',
      op: (typeof r.op === 'string' ? r.op : 'eq') as NonNullable<ScenarioAssertionRule['op']>,
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
      message: errorText(err),
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
    retry: null,
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
    retry: null,
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
  const spec: import('@/types/miyoApi.d').RequestSpec = JSON.parse(JSON.stringify(ep.request))
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
  { key: 'lifecycle', label: t('integration.lifecycleTab') },
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
const retryStatusText = computed({
  get: () => (selectedStep.value?.retry?.retry_on_status ?? []).join(','),
  set: (v: string) => {
    if (!selectedStep.value?.retry) return
    selectedStep.value.retry.retry_on_status = v
      .split(/[,\s]+/)
      .map((x) => Number.parseInt(x, 10))
      .filter((n) => Number.isFinite(n) && n >= 100 && n <= 599)
  },
})

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

// ---------- 提取 / 断言行编辑（行编辑器抽为共享组件；增删经 update 事件回写步骤字段） ----------
const selExtract = computed(() => selectedStep.value?.extract ?? [])
const selAssertions = computed(() => selectedStep.value?.assertions ?? [])

function onExtractRules(rules: ScenarioExtractRule[]): void {
  if (selectedStep.value) selectedStep.value.extract = rules
}

function onAssertionRules(rules: ScenarioAssertionRule[]): void {
  // 共享组件按基类型回抛；行数据源自编辑态（含必填字段），此处按编辑态类型收窄。
  if (selectedStep.value) selectedStep.value.assertions = rules as EditorAssertion[]
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

// ---------- 变量链路 + 运行前校验（纯派生状态，见 composables/useScenarioValidation） ----------
const {
  varScanSteps,
  varChain,
  externalVarNames,
  issues,
  errorCount,
  warnCount,
  stepIssues,
} = useScenarioValidation({
  steps,
  workspace,
  name,
  endpointOf,
  urlTemplateFor,
  stepTitle,
})

/**
 * 校验面板展开态（纯视图状态，归宿主）。
 * 不能放进 useScenarioValidation：`savedBaseline = draftSnapshot()` → `toScenario()` →
 * `generateScriptText()` 会读它，那条链路在 setup 里就能被 `await run.run()`
 * 触发（未保存时先落库），早于组合式内部 `const` 初始化 → TDZ「Cannot access
 * uninitialized variable」。
 */
const validateOpen = ref(false)
watch(
  errorCount,
  (n, prev) => {
    if (n > 0 && !prev) validateOpen.value = true
  },
  { immediate: true },
)

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
/** 生命周期步骤 → 运行入参（形状与 steps 元素一致；引用式步骤物化为内联）。 */
function lifecycleStepsPayload(phase: 'setup' | 'teardown') {
  const sc = scenario.value
  if (!sc) return []
  return (sc[phase] ?? [])
    .map((st, i) => {
      if (st.request) {
        return {
          endpoint: {
            id: st.request.url,
            project_id: '',
            folder_id: null,
            name: st.name || `${phase}-${i + 1}`,
            method: st.request.method as Endpoint['method'],
            path: st.request.url,
            description: '',
            status: 'developing' as const,
            sort_order: 0,
            request: st.request.spec,
            created_at: '',
            updated_at: '',
          },
          url: st.request.url,
          enabled: st.enabled,
          extract: st.extract ?? [],
          assertions: st.assertions ?? [],
          retry: st.retry ?? null,
        }
      }
      const ep = workspace.endpoints.find((e) => e.id === st.endpoint_id)
      if (!ep) return null
      return {
        endpoint: ep,
        url: `{{base_url}}${ep.path.startsWith('/') ? ep.path : `/${ep.path}`}`,
        enabled: st.enabled,
        extract: st.extract ?? [],
        assertions: st.assertions ?? [],
        retry: st.retry ?? null,
      }
    })
    .filter((x): x is NonNullable<typeof x> => x !== null)
}

function buildOneRunStep(s: EditorStep): BuiltRunStep | null {
  if (!s.enabled) return null
  if (s.kind === 'inline') {
    const name = s.name.trim() || t('integration.stepUntitled', { n: steps.value.indexOf(s) + 1 })
    const spec = JSON.parse(JSON.stringify(s.inline.spec))
    return {
      retry: s.retry,
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
  const spec: import('@/types/miyoApi.d').RequestSpec = JSON.parse(JSON.stringify(ep.request))
  try {
    applyStepOverrides(spec, s.overrides)
  } catch (err) {
    toast.error(t('integration.overridesInvalid'), {
      message: errorText(err),
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

// ---------- 生命周期（setup / teardown）与数据集（数据驱动）编辑 ----------
// 编辑桥抽为 useLifecycleEditor（三 Tab JSON 同步 + 数据集原地增删改 + inline 行编辑）。
const {
  lifecycleTab,
  lifecycleText,
  lifecycleError,
  lifecycleJson,
  applyLifecycleText,
  dsIndex,
  dsCurrent,
  addDataSet,
  removeDataSet,
  dsName,
  dsKind,
  dsKindOptions,
  setDsKind,
  dsRows,
  addRow,
  removeRow,
  rowEntries,
  rowSetKey,
  rowSetValue,
  rowRemoveKey,
  rowAddKey,
  dsText,
} = useLifecycleEditor({ scenario, t })

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
  lifecycleSetup: () => lifecycleStepsPayload('setup'),
  lifecycleTeardown: () => lifecycleStepsPayload('teardown'),
  lifecycleTeardownEnabled: () => scenario.value?.teardown_enabled ?? true,
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

// ---------- 进度事件（scope 卸载自动解绑 + 晚到竞态兜底，见 useTauriEvent） ----------
useTauriEvent<ScenarioProgressEvent>('miyo:scenario-progress', (p) => {
  run.applyProgressEvent(p)
})
onMounted(async () => {
  await load()
  // 先加载历史列表，再恢复最近一次运行：restoreLatestRun 读 runHistory[0]，
  // 并行调用时列表尚未返回必然空跑（恢复功能整体失效）。
  void run.loadHistory().then(() => run.restoreLatestRun())
  // 列表「执行历史」直达：加载指定运行并进入结果视图（与最近一次恢复
  // 共用恢复序号：只有最新一次请求能落状态，乱序返回不互相覆盖）。
  const pending = store.takePendingRun()
  if (pending) {
    const outcome = await run.restoreRun(pending)
    if (outcome.status === 'applied') {
      viewMode.value = 'result'
    }
  }
})
onBeforeUnmount(() => {
  // 组合式内的在途恢复请求随之失效（卸载后不再写状态）。
  run.dispose()
  sortInstance?.destroy()
  sortInstance = null
  if (elapsedTimer != null) window.clearInterval(elapsedTimer)
  window.removeEventListener('resize', syncDetailMax)
})

onMounted(() => {
  syncDetailMax()
  window.addEventListener('resize', syncDetailMax)
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
/** 报告落盘：目录选择一次 + 建议文件名写入（历史菜单导出项的宿主回调）。 */
const reportWriteHook = async (suggestedName: string, content: string): Promise<void> => {
  const dir = await openDialog({ directory: true, title: t('integration.reportExport') })
  if (!dir || Array.isArray(dir)) throw new Error('cancelled')
  const path = await api.joinPath(dir, suggestedName)
  await api.writeTextFile(path, content)
}

async function onHistorySelect(item: MenuItem): Promise<void> {
  await run.onHistorySelect(item, { writeTextFile: reportWriteHook })
  if (run.runIsHistory.value) viewMode.value = 'result'
}

/**
 * 步骤 → 脚本入参。`redact`（导出用）= 敏感值以占位符生成：
 * 真实 Token / 密码不写入导出的 .mjs（脚本会离开本机）；应用内
 * scriptDraft 保持明文（用户可见可改，随场景保存属本地资产）。
 */
function toScriptSteps(redact = false): ScriptStep[] {
  const redactKv = (items: { key: string; value: string }[]) =>
    redact
      ? items.map((kv) =>
          isSensitiveKeyName(kv.key) && kv.value ? { ...kv, value: EXPORT_PLACEHOLDER } : kv,
        )
      : items
  const redactAuth = <T extends { kind: string; token?: string; password?: string; value?: string; key?: string }>(
    a: T | null,
  ): T | null =>
    redact && a
      ? ({
          ...a,
          ...(a.token !== undefined && a.token ? { token: EXPORT_PLACEHOLDER } : {}),
          ...(a.password !== undefined && a.password ? { password: EXPORT_PLACEHOLDER } : {}),
          ...(a.value !== undefined && a.value ? { value: EXPORT_PLACEHOLDER } : {}),
        } as T)
      : a
  const redactBodyRaw = (raw: string | null) => (redact && raw ? redactJsonText(raw) : raw)
  const redactUrl = (u: string) => (redact ? redactUrlQuery(u) : u)

  return steps.value.map((s) => {
    const ep = s.kind === 'ref' ? endpointOf(s) : null
    if (s.kind === 'inline') {
      const body = s.inline.spec.body
      const kind = body.mode
      const enabledHeaders = redactKv(
        s.inline.spec.headers.filter((h) => h.enabled).map((h) => ({ key: h.key, value: h.value })),
      )
      const auth = staticAuth(s.inline.spec.auth)
      return {
        name: stepTitle(s),
        method: s.inline.method,
        url: redactUrl(s.inline.url),
        params: redactKv(
          s.inline.spec.params.filter((p) => p.enabled).map((p) => ({ key: p.key, value: p.value })),
        ),
        headers: enabledHeaders,
        auth: redactAuth(auth?.auth ?? null),
        authUnsupported: auth?.unsupported ?? null,
        body:
          body.mode === 'json' || body.mode === 'text'
            ? redactBodyRaw(body.raw)
            : body.mode === 'graphql'
              ? body.spec.query
              : null,
        bodyFields: redactKv(
          (body.mode === 'urlencoded' || body.mode === 'multipart'
            ? body.fields.filter((f) => f.enabled).map((f) => ({ key: f.key, value: f.value }))
            : []) as { key: string; value: string }[],
        ),
        bodyFileFields:
          body.mode === 'multipart' ? body.fields.filter((f) => f.value_type === 'file_path').map((f) => f.key) : [],
        bodyVariables: body.mode === 'graphql' ? (redact ? redactJsonText(body.spec.variables) : body.spec.variables) : '',
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
      url: redactUrl(urlTemplateFor(ep)),
      params: redactKv(ep.request.params.filter((p) => p.enabled).map((p) => ({ key: p.key, value: p.value }))),
      headers: redactKv(ep.request.headers.filter((h) => h.enabled).map((h) => ({ key: h.key, value: h.value }))),
      auth: redactAuth(auth?.auth ?? null),
      authUnsupported: auth?.unsupported ?? null,
      body:
        body.mode === 'json' || body.mode === 'text'
          ? redactBodyRaw(body.raw)
          : body.mode === 'graphql'
            ? body.spec.query
            : null,
      bodyFields: redactKv(
        (body.mode === 'urlencoded' || body.mode === 'multipart'
          ? body.fields.filter((f) => f.enabled).map((f) => ({ key: f.key, value: f.value }))
          : []) as { key: string; value: string }[],
      ),
      bodyFileFields:
        body.mode === 'multipart' ? body.fields.filter((f) => f.value_type === 'file_path').map((f) => f.key) : [],
      bodyVariables:
        body.mode === 'graphql' ? (redact ? redactJsonText(body.spec.variables) : body.spec.variables) : '',
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
function staticAuth(auth: import('@/types/miyoApi.d').AuthSpec): {
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

function generateScriptText(redact = false): string {
  return generateScenarioScript(name.value.trim() || t('integration.newName'), toScriptSteps(redact), {
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
                    fields: p.bodyFields.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '', field_type: 'string' as const, required: true, example: '' })),
                  }
                : { mode: 'none' }
      inlineStep.spec.params = p.params.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '', field_type: 'string' as const, required: true, example: '' }))
      // 头无法与鉴权区分（解析器并入 headers），按普通头落盘。
      inlineStep.spec.headers = p.headers.map((f) => ({ key: f.key, value: f.value, enabled: true, description: '', field_type: 'string' as const, required: true, example: '' }))
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
    retry: null,
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
        message: errorText(err),
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
    const path = await api.joinPath(dir, `${base}-report.md`)
    await api.writeTextFile(path, markdown)
    toast.success(t('integration.resultExported'), { message: path })
  } catch (err) {
    toast.error(t('integration.exportFail'), {
      message: errorText(err),
    })
  }
}

async function exportScript(): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, title: t('scenario.scriptExportTitle') })
    if (!dir || Array.isArray(dir)) return
    const base = (name.value.trim() || t('integration.newName')).replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await api.joinPath(dir, `${base}.mjs`)
    // 导出用脱敏再生成（不用 scriptDraft 原文）：真实凭证不写入 .mjs，
    // 敏感值以占位符出现，接手方按提示回填。
    await api.writeTextFile(path, generateScriptText(true))
    toast.success(t('scenario.scriptExported'), { message: path })
  } catch (err) {
    toast.error(t('scenario.scriptExportFail'), {
      message: errorText(err),
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

/** 连线上的变量 chip 点击 → 定位到定义该变量的步骤卡（scenarioFlow.definedBy）。 */
function locateVarSource(varName: string): void {
  const at = flowGraph.value.definedBy.get(varName)
  if (at !== undefined && at >= 0) locateStep(at)
}

// ---------- 右栏详情宽度（拖拽 + 记忆，SplitPanes widthOn=second） ----------
const DETAIL_WIDTH_KEY = 'miyo.wf-detail-width'
const DETAIL_WIDTH_DEFAULT = 420
const DETAIL_WIDTH_MIN = 320
const DETAIL_WIDTH_MAX = 900

function readDetailWidth(): number {
  try {
    const raw = localStorage.getItem(DETAIL_WIDTH_KEY)
    const n = raw ? Number(raw) : NaN
    if (!Number.isFinite(n)) return DETAIL_WIDTH_DEFAULT
    return Math.min(Math.max(Math.round(n), DETAIL_WIDTH_MIN), DETAIL_WIDTH_MAX)
  } catch {
    return DETAIL_WIDTH_DEFAULT
  }
}
const detailWidth = ref(readDetailWidth())
watch(detailWidth, (w) => {
  try {
    localStorage.setItem(DETAIL_WIDTH_KEY, String(Math.round(w)))
  } catch {
    /* 隐私模式等存储不可用：仅本次会话内生效 */
  }
})
/** 详情栏上限随窗口收缩（1366px 小窗不给拖到挤压流程列表）。 */
const detailMaxWidth = ref(DETAIL_WIDTH_MAX)
function syncDetailMax(): void {
  detailMaxWidth.value = Math.max(DETAIL_WIDTH_MIN + 80, Math.round(window.innerWidth * 0.6))
}

// ---------- 运行跟随：在途步骤自动滚入视口，用户手动滚动即暂停 ----------
const followRun = ref(true)
/** 最小在途位次（并行批内多步在途时跟随最先进入的）。 */
const firstInFlightPos = computed(() => {
  let min = -1
  for (const k of run.inFlight.value.keys()) {
    if (min === -1 || k < min) min = k
  }
  return min
})
watch(firstInFlightPos, (pos) => {
  if (!run.running.value || !followRun.value || pos < 0 || viewMode.value !== 'manual') return
  const stepIdx = run.runStepIndexes.value[pos]
  const s = steps.value[stepIdx]
  if (!s) return
  void nextTick(() => {
    const el = stepsEl.value?.querySelector(`[data-step-key="${s.key}"]`)
    el?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  })
})
/** 新一轮运行开始时恢复跟随；用户滚轮滚动（运行中）暂停到本轮结束。 */
watch(
  () => run.running.value,
  (now, was) => {
    if (now && !was) followRun.value = true
  },
)

// ---------- 在途步骤耗时（本地秒级计时；进度事件不带开始时刻） ----------
const flightStart = ref<Map<number, number>>(new Map())
watch(run.inFlight, (m) => {
  const now = Date.now()
  const next = new Map<number, number>()
  for (const k of m.keys()) next.set(k, flightStart.value.get(k) ?? now)
  flightStart.value = next
})
const elapsedTick = ref(0)
let elapsedTimer: number | null = null
watch(
  () => run.running.value,
  (r) => {
    if (r && elapsedTimer == null) {
      elapsedTimer = window.setInterval(() => {
        elapsedTick.value++
      }, 1000)
    } else if (!r && elapsedTimer != null) {
      window.clearInterval(elapsedTimer)
      elapsedTimer = null
    }
  },
)
/** 在途步骤已用时间文案（mm:ss / Ns）；读 elapsedTick 触发每秒重算。 */
function flightElapsed(pos: number): string {
  void elapsedTick.value
  const start = flightStart.value.get(pos)
  if (start === undefined) return ''
  const sec = Math.max(0, Math.floor((Date.now() - start) / 1000))
  if (sec < 60) return `${sec}s`
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`
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
      <!-- 左流程 / 右详情可拖分栏：详情宽度记忆到 miyo.wf-detail-width -->
      <SplitPanes
        v-model:width="detailWidth"
        class="wf-split"
        width-on="second"
        :min="320"
        :max="detailMaxWidth"
        @reset="detailWidth = 420"
      >
        <template #first>
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
                <button
                  v-for="v in edgeVarsLabel(edgeInto.get(i)!).shown"
                  :key="v"
                  type="button"
                  class="wf-edge-chip"
                  :title="t('integration.flowVarsLocateTitle', { name: v })"
                  @click="locateVarSource(v)"
                >{{ v }}</button>
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
                failed: run.stepTone(i) === 'fail',
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
            <span
              v-if="run.stepTone(i) === 'run' && flightElapsed(run.runStepIndexes.value.indexOf(i))"
              class="wf-step-elapsed num"
            >{{ flightElapsed(run.runStepIndexes.value.indexOf(i)) }}</span>
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
        </template>
        <template #second>
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

                <!-- 步骤重试策略（显式配置才启用；副作用步骤自动幂等键） -->
                <div class="wf-ctrl-block">
                  <div class="wf-ctrl-head">
                    <span class="wf-ctrl-label">{{ t('integration.retryTitle') }}</span>
                    <button
                      v-if="!selectedStep.retry"
                      class="miyo-btn miyo-btn-sm"
                      type="button"
                      @click="selectedStep.retry = { max_attempts: 3, interval_ms: 500, exponential: false, retry_on_status: [502, 503, 504], retry_on_network: true }"
                    >
                      <Icon name="plus" :size="12" /> {{ t('integration.retryAdd') }}
                    </button>
                    <IconButton
                      v-else
                      name="trash"
                      :size="13"
                      :title="t('common.delete')"
                      @click="selectedStep.retry = null"
                    />
                  </div>
                  <p class="wf-ctrl-hint">{{ t('integration.retryHint') }}</p>
                  <div v-if="selectedStep.retry" class="wf-rule-row wf-retry-grid">
                    <label class="wf-ctrl-check">
                      <input
                        v-model.number="selectedStep.retry.max_attempts"
                        type="number"
                        min="1"
                        max="10"
                        class="miyo-input"
                      />
                      <span>{{ t('integration.retryAttempts') }}</span>
                    </label>
                    <label class="wf-ctrl-check">
                      <input
                        v-model.number="selectedStep.retry.interval_ms"
                        type="number"
                        min="0"
                        class="miyo-input"
                      />
                      <span>{{ t('integration.retryInterval') }}</span>
                    </label>
                    <label class="wf-ctrl-check">
                      <input v-model="selectedStep.retry.exponential" type="checkbox" />
                      <span>{{ t('integration.retryExponential') }}</span>
                    </label>
                    <label class="wf-ctrl-check">
                      <input v-model="selectedStep.retry.retry_on_network" type="checkbox" />
                      <span>{{ t('integration.retryNetwork') }}</span>
                    </label>
                    <input
                      v-model="retryStatusText"
                      class="miyo-input"
                      type="text"
                      spellcheck="false"
                      :placeholder="t('integration.retryStatusPh')"
                    />
                  </div>
                </div>
              </div>
            </div>
            <ExtractList
              v-show="detailTab === 'extract'"
              :rules="selExtract"
              @update:rules="onExtractRules"
            />
            <AssertionList
              v-show="detailTab === 'assert'"
              :rules="selAssertions"
              @update:rules="onAssertionRules"
            />
            <ScenarioVarsPanel
              v-show="detailTab === 'vars'"
              :chain="varChain"
              :runtime-vars="run.runResult.value?.variables ?? null"
              @locate="locateStep"
            />

            <!-- 生命周期 + 数据集（数据驱动）：setup/teardown JSON 编辑 +
                 数据集结构化编辑（名称 / 源类型 / 行表格），保存走既有链路 -->
            <div v-if="detailTab === 'lifecycle'" class="wf-lifecycle">
              <div class="wf-ctrl-head">
                <CustomSelect
                  :model-value="lifecycleTab"
                  :options="[
                    { value: 'setup', label: t('integration.lifecycleSetup') },
                    { value: 'teardown', label: t('integration.lifecycleTeardown') },
                    { value: 'datasets', label: t('integration.lifecycleDatasets') },
                  ]"
                  size="sm"
                  :pop-min-width="130"
                  @update:model-value="(v) => (lifecycleTab = String(v ?? 'setup') as 'setup' | 'teardown' | 'datasets')"
                />
                <label
                  v-if="lifecycleTab !== 'datasets' && scenario"
                  class="wf-ctrl-check"
                >
                  <input v-model="scenario.teardown_enabled" type="checkbox" />
                  <span>{{ t('integration.lifecycleAutoClean') }}</span>
                </label>
              </div>

              <!-- 数据集结构化编辑 -->
              <template v-if="lifecycleTab === 'datasets'">
                <p class="wf-ctrl-hint">{{ t('integration.dsHint') }}</p>
                <div class="wf-ds-list">
                  <button
                    v-for="(ds, i) in scenario?.data_sets ?? []"
                    :key="ds.id"
                    type="button"
                    class="wf-ds-chip"
                    :class="{ active: i === dsIndex }"
                    @click="dsIndex = i"
                  >
                    {{ ds.name || `#${i + 1}` }}
                  </button>
                  <button type="button" class="miyo-btn miyo-btn-sm" @click="addDataSet">
                    <Icon name="plus" :size="12" /> {{ t('integration.dsAdd') }}
                  </button>
                  <IconButton
                    v-if="scenario?.data_sets?.length"
                    name="trash"
                    :size="13"
                    :title="t('common.delete')"
                    @click="removeDataSet"
                  />
                </div>
                <template v-if="dsCurrent">
                  <div class="wf-rule-row mt-3">
                    <input
                      v-model="dsName"
                      class="miyo-input wf-rule-name"
                      type="text"
                      :placeholder="t('integration.dsNamePh')"
                      spellcheck="false"
                    />
                    <CustomSelect
                      class="wf-ctrl-src"
                      :model-value="dsKind"
                      :options="dsKindOptions"
                      size="sm"
                      :pop-min-width="120"
                      @update:model-value="setDsKind"
                    />
                  </div>

                  <!-- inline：行表格（键值对编辑） -->
                  <div v-if="dsKind === 'inline'" class="wf-ds-rows">
                    <div
                      v-for="(row, ri) in dsRows"
                      :key="ri"
                      class="wf-ds-row"
                    >
                      <div class="flex flex-col gap-1">
                        <div
                          v-for="[k, v] in rowEntries(row)"
                          :key="k"
                          class="flex items-center gap-1.5"
                        >
                          <input
                            class="miyo-input wf-ds-key"
                            :value="k"
                            type="text"
                            spellcheck="false"
                            :placeholder="t('integration.dsKeyPh')"
                            @change="rowSetKey(row, k, ($event.target as HTMLInputElement).value)"
                          />
                          <input
                            class="miyo-input wf-ds-val"
                            :value="v"
                            type="text"
                            spellcheck="false"
                            @input="rowSetValue(row, k, ($event.target as HTMLInputElement).value)"
                          />
                          <IconButton
                            name="trash"
                            :size="12"
                            :title="t('common.delete')"
                            @click="rowRemoveKey(row, k)"
                          />
                        </div>
                      </div>
                      <div class="flex items-center gap-1">
                        <IconButton name="plus" :size="12" :title="t('integration.dsAddKey')" @click="rowAddKey(row)" />
                        <IconButton name="trash" :size="12" :title="t('integration.dsRemoveRow')" @click="removeRow(ri)" />
                      </div>
                    </div>
                    <button type="button" class="miyo-btn miyo-btn-sm" @click="addRow">
                      <Icon name="plus" :size="12" /> {{ t('integration.dsAddRow') }}
                    </button>
                  </div>

                  <!-- json / csv：文本域 -->
                  <textarea
                    v-else-if="dsKind !== 'environment'"
                    v-model="dsText"
                    class="miyo-input wf-lifecycle-json"
                    rows="8"
                    spellcheck="false"
                    :placeholder="dsKind === 'json' ? t('integration.dsJsonPh') : t('integration.dsCsvPh')"
                  />
                  <p v-else class="wf-ctrl-hint">{{ t('integration.dsEnvHint') }}</p>
                </template>
              </template>

              <!-- setup / teardown：JSON 编辑（保存走既有链路） -->
              <template v-else>
                <p class="wf-ctrl-hint">{{ t('integration.lifecycleHint') }}</p>
                <textarea
                  v-model="lifecycleText"
                  class="miyo-input wf-lifecycle-json"
                  rows="12"
                  spellcheck="false"
                />
                <p v-if="lifecycleError" class="wf-lifecycle-error">{{ lifecycleError }}</p>
                <div class="wf-lifecycle-actions">
                  <button type="button" class="miyo-btn miyo-btn-sm" @click="lifecycleText = lifecycleJson()">
                    {{ t('integration.lifecycleReload') }}
                  </button>
                  <button type="button" class="miyo-btn miyo-btn-sm" @click="applyLifecycleText()">
                    {{ t('integration.lifecycleApply') }}
                  </button>
                </div>
              </template>
            </div>
          </div>
        </template>
        <div v-else class="wf-detail-placeholder">
          <Icon name="pencil" :size="18" />
          <p>{{ t('scenario.stepSelect') }}</p>
        </div>
      </aside>
        </template>
      </SplitPanes>
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
  padding: 10px 12px;
}

/* 左流程 / 右详情分栏容器（SplitPanes 分割线取代旧 gap:12px） */
.wf-split {
  flex: 1;
  min-width: 0;
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
  border: none;
  border-radius: var(--radius-sm);
  background: var(--accent-tint);
  color: var(--accent);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  line-height: 15px;
  cursor: pointer;
  transition: filter var(--dur) var(--ease);
}
.wf-edge-chip:hover {
  filter: brightness(1.2);
}
.wf-edge-chip.more {
  cursor: default;
}
.wf-edge-chip.more:hover {
  filter: none;
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

/* 运行失败态：卡片级红左缘 + 危险淡底（区别于校验问题的纯红描边），
   让「哪一步挂了」在长列表里一眼可见 */
.wf-step.failed {
  border-color: var(--danger-border);
  background: var(--danger-tint);
  box-shadow: inset 3px 0 0 var(--danger);
}
.wf-step.failed.selected {
  border-color: var(--danger);
  background: var(--danger-tint);
}

/* 在途步骤实时耗时（秒级跳动，等宽数字防抖动） */
.wf-step-elapsed {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-2xxs);
  color: var(--accent);
  background: var(--accent-tint);
  border-radius: var(--radius-sm);
  padding: 0 5px;
  line-height: 15px;
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
  font-size: var(--fs-2xxs);
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
  font-size: var(--fs-2xxs);
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-tone {
  flex: none;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
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
  font-size: var(--fs-2xxs);
  padding: 1px 7px;
  border-radius: var(--radius-pill);
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

.wf-ds-list {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.wf-ds-chip {
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  background: var(--bg-hover);
}

.wf-ds-chip.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-tint);
}

.wf-ds-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md, 8px);
}

.wf-ds-key {
  width: 120px;
  font-family: var(--font-mono);
}

.wf-ds-val {
  flex: 1;
}

.wf-retry-grid {
  display: flex;
  flex-direction: column;
  gap: 8px;
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
/* 详情栏：宽度由 SplitPanes（.sp-second 内联样式）控制，这里只负责撑满与外观 */
.wf-detail {
  flex: 1;
  min-width: 0;
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
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
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


.wf-rule-op {
  flex: none;
  width: 116px;
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

/* 窄屏不再硬编码右栏宽：宽度由 SplitPanes 记忆值 + 动态上限（窗口 60%）接管 */
</style>
