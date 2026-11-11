<script setup lang="ts">
/**
 * TestCaseDrawer：测试用例详情 / 编辑抽屉（右侧滑出，原地运行，三 Tab）。
 *
 * - Tab：请求配置（复用调试器 Params/Headers/Auth/Body/PathVariables 面板，
 *   「伪 Endpoint」注入，先例 StepRequestEditor）｜断言与提取（共享 AssertionList /
 *   ExtractList）｜执行结果（引擎结果 + 执行历史 + 两次对比）；
 * - 宽度：默认 680px，左缘拖拽（min 560 / max 75vw），全屏切换；
 * - 关闭：ESC / ✕ / 遮罩；有未保存修改时先出「放弃修改 / 继续编辑」确认条；
 * - 继承 vs 覆盖：用例是保存时的接口快照，与当前接口逐节比对展示「继承 / 覆盖」，
 *   一键「同步自接口」把接口当前配置重灌进用例（显式动作，不静默）；
 * - 底部操作栏 sticky：导出代码 + 立即运行（可取消）｜取消 / 保存修改。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { pushEscHandler } from '../utils/escStack'
import { useToast } from '../composables/useToast'
import type {
  AuthSpec,
  BodySpec,
  Endpoint,
  EndpointResult,
  HttpMethod,
  KeyValue,
  RequestSpec,
  ScenarioAssertionRule,
  ScenarioExtractRule,
  TestCase,
  TestCaseCategory,
  TestCaseRun,
} from '../types/miyoApi'
import { TEST_CASE_CATEGORIES, caseCategoryLabel, formatDuration, relativeRunTime, restoreBody, snapshotRequest } from '../utils/testCases'
import { applyPathVars, extractPathVarNames } from '../utils/pathVariables'
import { methodTextTone, methodTone } from '../utils/methodTone'
import { applyMethodDefaults, smartTabFor } from '../utils/requestBar'
import { useLocaleStore } from '../stores/locale'
import { statusTextOf } from '../utils/testCases'
import { useMiyoApi } from '../composables/useMiyoApi'
import AssertionList from './assertion/AssertionList.vue'
import CodeExportMenu from './CodeExportMenu.vue'
import CustomSelect from './ui/CustomSelect.vue'
import Skeleton from './ui/Skeleton.vue'
import EnvironmentSelector from './EnvironmentSelector.vue'
import ExtractList from './assertion/ExtractList.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import ParamsPanel from './ParamsPanel.vue'
import HeadersPanel from './HeadersPanel.vue'
import AuthPanel from './AuthPanel.vue'
import BodyPanel from './BodyPanel.vue'
import PathVariablesPanel from './PathVariablesPanel.vue'
import RequestDetail from './RequestDetail.vue'
import ResponsePanel from './ResponsePanel.vue'
import TestCaseRunCompare from './cases/TestCaseRunCompare.vue'

const props = defineProps<{
  open: boolean
  /** 当前接口 id（request_id），用于保存后回填本地列表。 */
  endpointId: string
  /** 当前接口（继承对比 / 同步来源）。 */
  endpoint: Endpoint | null
  testCase: TestCase | null
  /** 会话内最近一次运行结果（打开抽屉即能看到上一次运行）。 */
  lastResult?: EndpointResult | null
  /** 路径 → 可运行 URL（环境 Base URL + 变量解析），导出代码用；与 onRun/onSave 同为父级注入。 */
  resolveUrl: (path: string) => string
  /** 运行回调：返回引擎结果或抛错。由父级（store 或 panel）注入，避免组件耦合执行细节。 */
  onRun: (payload: DrawerPayload, runId: string) => Promise<EndpointResult | null>
  onSave: (payload: DrawerSavePayload) => Promise<void>
}>()

/** 运行 / 保存共用的完整载荷（请求快照 + 断言提取 + 执行控制）。 */
interface DrawerPayload {
  name: string
  category: TestCaseCategory
  folder: string
  method: HttpMethod
  urlPath: string
  pathVariables: KeyValue[]
  params: KeyValue[]
  headers: KeyValue[]
  bodyType: string
  bodyContent: string
  assertions: ScenarioAssertionRule[]
  extractors: ScenarioExtractRule[]
  auth: AuthSpec | null
  timeoutMs: number | null
  disableCookies: boolean
}
type DrawerSavePayload = DrawerPayload

const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const locale = useLocaleStore()
const t = locale.t
const api = useMiyoApi()
const toast = useToast()

// ---------- 编辑态 ----------
const name = ref('')
const category = ref<TestCaseCategory>('正向')
const folder = ref('')
const method = ref<HttpMethod>('GET')
const urlPath = ref('')
/** 断言 / 提取（引擎方言，与场景步骤同形状）。 */
const assertions = ref<ScenarioAssertionRule[]>([])
const extractors = ref<ScenarioExtractRule[]>([])
/** 用例级执行控制覆盖。 */
const auth = ref<AuthSpec | null>(null)
const timeoutMs = ref<number | null>(null)
const disableCookies = ref(false)

/**
 * 伪 Endpoint：共享配置面板（Params/Headers/Auth/Body/PathVariables）的编辑载体。
 * 打开 / 切换用例时按快照重建（对象身份稳定，BodyPanel 的草稿按对象弱引用分桶）。
 */
const caseEndpoint = ref<Endpoint | null>(null)

const METHOD_OPTIONS = (['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS'] as HttpMethod[]).map(
  (m) => ({ value: m, label: m, tone: methodTextTone(m) }),
)

/** 快照 → 伪 Endpoint 的 RequestSpec（与 store.buildCaseRequest 同构，保持未渲染原值）。 */
function specFromCase(testCase: TestCase): RequestSpec {
  return {
    params: (testCase.params ?? []).map((p) => ({ ...p })),
    headers: (testCase.headers ?? []).map((h) => ({ ...h })),
    path_variables: (testCase.path_variables ?? []).map((p) => ({ ...p })),
    auth: (testCase.auth ?? { type: 'none' }) as AuthSpec,
    body: restoreBody(testCase.body_type, testCase.body_content),
    active_tab: null,
    timeout_ms: testCase.timeout_ms ?? null,
    follow_redirects: true,
    disable_cookies: testCase.disable_cookies ?? false,
    tests: null,
  }
}

// ---------- Tab ----------
type DrawerTab = 'config' | 'checks' | 'result'
const tab = ref<DrawerTab>('config')
/** 执行结果 Tab 的角标：断言条数（跟随最近一次运行 / 正在回看的历史）。 */
const resultTabBadge = computed(() => viewRunResult.value?.outcomes?.length || undefined)
const tabOptions = computed(() => [
  { value: 'config', label: t('casedrawer.tabConfig') },
  { value: 'checks', label: t('casedrawer.tabChecks'), badge: assertions.value.length + extractors.value.length || undefined },
  { value: 'result', label: t('casedrawer.tabResult'), badge: resultTabBadge.value },
])

/** 请求配置内的分段（复用调试器面板组）。 */
type ConfigSection = 'params' | 'path' | 'headers' | 'auth' | 'body'
const configSection = ref<ConfigSection>('params')
const hasPathVars = computed(() => extractPathVarNames(urlPath.value).length > 0)
const configSections = computed(() => [
  { value: 'params', label: t('casedrawer.tabParams') },
  ...(hasPathVars.value ? [{ value: 'path' as ConfigSection, label: t('casedrawer.tabPath') }] : []),
  { value: 'headers', label: t('casedrawer.tabHeaders') },
  { value: 'auth', label: t('casedrawer.tabAuth') },
  { value: 'body', label: t('casedrawer.tabBody') },
])

// ---------- 打开时同步草稿 ----------
// 同时监听 open 与 testCase：抽屉已开时切换用例必须强制重新同步（历史竞态）。
const savedSnapshot = ref('')
const result = ref<EndpointResult | null>(null)
const runError = ref('')
/** 关闭确认条（脏检查拦截后展示）。 */
const confirmClose = ref(false)

function currentSnapshotJson(): string {
  const spec = caseEndpoint.value?.request
  const body = spec ? snapshotRequest(spec) : null
  return JSON.stringify({
    name: name.value,
    category: category.value,
    folder: folder.value,
    method: method.value,
    urlPath: urlPath.value,
    body,
    auth: auth.value,
    timeoutMs: timeoutMs.value,
    disableCookies: disableCookies.value,
    assertions: assertions.value,
    extractors: extractors.value,
  })
}

const isDirty = computed(() => !!props.testCase && currentSnapshotJson() !== savedSnapshot.value)

watch(
  [() => props.open, () => props.testCase],
  ([open, testCase]) => {
    if (!open || !testCase) return
    name.value = testCase.name
    category.value = testCase.category
    folder.value = testCase.folder ?? ''
    method.value = testCase.method
    urlPath.value = testCase.url_path
    assertions.value = Array.isArray(testCase.assertions)
      ? testCase.assertions.map((a) => ({ ...a }))
      : []
    extractors.value = Array.isArray(testCase.extractors)
      ? testCase.extractors.map((e) => ({ ...e }))
      : []
    auth.value = (testCase.auth ?? null) as AuthSpec | null
    timeoutMs.value = testCase.timeout_ms ?? null
    disableCookies.value = testCase.disable_cookies ?? false
    caseEndpoint.value = {
      id: testCase.request_id,
      project_id: props.endpoint?.project_id ?? '',
      folder_id: props.endpoint?.folder_id ?? null,
      name: testCase.name,
      method: testCase.method,
      path: testCase.url_path,
      description: '',
      status: 'designing',
      sort_order: 0,
      request: specFromCase(testCase),
      created_at: testCase.created_at,
      updated_at: testCase.created_at,
    }
    tab.value = 'config'
    // 按方法落默认分段（POST 系 → Body），但不改写用例快照
    configSection.value = smartSectionFor(testCase.method)
    result.value = props.lastResult ?? null
    runError.value = ''
    confirmClose.value = false
    savedSnapshot.value = currentSnapshotJson()
  },
  { immediate: true },
)



// ---------- 关闭（ESC / ✕ / 遮罩）+ 脏检查 ----------
function attemptClose(): void {
  if (isDirty.value) {
    confirmClose.value = true
    return
  }
  emit('update:open', false)
}

function discardAndClose(): void {
  confirmClose.value = false
  emit('update:open', false)
}

// ESC 层级仲裁：抽屉打开时压栈（后开先关——抽屉里的下拉/菜单按 ESC 只关子弹层）。
// 查找条 / 下拉弹层自己处理 ESC：此时消费事件但不关抽屉（防止穿透关掉更外层弹窗）。
function escHandler(): boolean {
  const target = document.activeElement as HTMLElement | null
  if (target && typeof target.closest === 'function' && target.closest('.findbar, .cs-pop')) {
    return true
  }
  attemptClose()
  return true
}

let offEsc: (() => void) | null = null
watch(
  () => props.open,
  (open) => {
    offEsc?.()
    offEsc = null
    // 打开期间锁背景滚动（与 ui/Modal / ui/Drawer 行为一致）。
    document.body.style.overflow = open ? 'hidden' : ''
    if (open) offEsc = pushEscHandler(escHandler)
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  offEsc?.()
  offEsc = null
  document.body.style.overflow = ''
})

// ---------- 宽度：左缘拖拽（min 560 / max 75vw）+ 全屏 ----------
const DRAWER_MIN = 560
const DRAWER_DEFAULT = 680
const width = ref(DRAWER_DEFAULT)
const fullscreen = ref(false)
let resizing = false

function onResizeDown(e: MouseEvent): void {
  if (e.button !== 0 || fullscreen.value) return
  e.preventDefault()
  resizing = true
  window.addEventListener('mousemove', onResizeMove)
  window.addEventListener('mouseup', onResizeUp)
}

function onResizeMove(e: MouseEvent): void {
  if (!resizing) return
  const max = Math.floor(window.innerWidth * 0.75)
  width.value = Math.min(max, Math.max(DRAWER_MIN, window.innerWidth - e.clientX))
}

function onResizeUp(): void {
  resizing = false
  window.removeEventListener('mousemove', onResizeMove)
  window.removeEventListener('mouseup', onResizeUp)
}

function toggleFullscreen(): void {
  fullscreen.value = !fullscreen.value
}

onBeforeUnmount(() => onResizeUp())

// ---------- Method / URL ----------
// 方法切换的副作用只在用户显式操作时发生（onMethodChange / syncFromEndpoint）：
// 打开用例时的回填不应顺手给快照补 JSON Body（会被判脏、保存时污染用例）。

/** 路径变量补齐：URL 加了 `{var}` 时补缺失行（保留已有取值）。 */
watch(urlPath, () => {
  const req = caseEndpoint.value
  if (!req) return
  const names = extractPathVarNames(urlPath.value)
  if (!names.length) return
  const existing = new Set(req.request.path_variables.map((p) => p.key))
  for (const n of names) {
    if (!existing.has(n)) {
      req.request.path_variables.push({ key: n, value: '', enabled: true, description: '' })
    }
  }
})

// ---------- 继承 vs 覆盖（与当前接口逐节比对） ----------
function sameKv(a: KeyValue[] = [], b: KeyValue[] = []): boolean {
  const norm = (rows: KeyValue[]) =>
    JSON.stringify(rows.filter((r) => r.key || r.value).map((r) => [r.key, r.value, r.enabled]))
  return norm(a) === norm(b)
}

const inheritFlags = computed(() => {
  const ep = props.endpoint?.request
  const mine = caseEndpoint.value?.request
  if (!ep || !mine) return { params: true, headers: true, body: true, pathVariables: true }
  return {
    params: sameKv(ep.params, mine.params),
    headers: sameKv(ep.headers, mine.headers),
    body: JSON.stringify(ep.body) === JSON.stringify(mine.body),
    pathVariables: sameKv(ep.path_variables, mine.path_variables),
  }
})

/** 方法 → 分段智能默认（POST 系落 Body，其余落 Params；先例 requestBar.smartTabFor）。 */
function smartSectionFor(m: string): ConfigSection {
  const t0 = smartTabFor(m)
  if (t0 === 'body') return 'body'
  if (t0 === 'grpc') return 'body'
  return 'params'
}
/** 用户切换方法：写回默认（POST 空 body 补 `{}` + Content-Type），同时跟随分段。 */
function onMethodChange(v: unknown): void {
  method.value = String(v ?? 'GET') as HttpMethod
  const spec = caseEndpoint.value?.request
  if (spec) configSection.value = smartSectionFor(applyMethodDefaults(spec, method.value))
}

/** 同步自接口：把接口当前配置重灌进用例（显式动作；不带断言 / 提取）。 */
function syncFromEndpoint(): void {
  const ep = props.endpoint
  const mine = caseEndpoint.value
  if (!ep || !mine) return
  mine.request.params = ep.request.params.map((p) => ({ ...p }))
  mine.request.headers = ep.request.headers.map((h) => ({ ...h }))
  mine.request.body = JSON.parse(JSON.stringify(ep.request.body)) as BodySpec
  mine.request.path_variables = ep.request.path_variables.map((p) => ({ ...p }))
  urlPath.value = ep.path
  method.value = ep.method
  configSection.value = smartSectionFor(ep.method)
  toastSynced()
}

function toastSynced(): void {
  toast.success(t('casedrawer.synced'))
}

// ---------- 导出代码 ----------
const exportUrl = computed(() => {
  const rendered = applyPathVars(urlPath.value.trim(), caseEndpoint.value?.request.path_variables ?? [])
  return rendered ? props.resolveUrl(rendered) : ''
})

// ---------- 运行 / 保存 ----------
const saving = ref(false)
const running = ref(false)
/** 在途运行标识（取消按钮）。 */
const activeRunId = ref<string | null>(null)

function buildPayload(): DrawerPayload {
  const spec = caseEndpoint.value?.request
  const snap = spec ? snapshotRequest(spec) : null
  return {
    name: name.value.trim(),
    category: category.value,
    folder: folder.value,
    method: method.value,
    urlPath: urlPath.value.trim(),
    pathVariables: snap?.path_variables ?? [],
    params: snap?.params ?? [],
    headers: snap?.headers ?? [],
    bodyType: snap?.body_type ?? 'none',
    bodyContent: snap?.body_content ?? '',
    assertions: assertions.value,
    extractors: extractors.value,
    auth: auth.value,
    timeoutMs: timeoutMs.value,
    disableCookies: disableCookies.value,
  }
}

async function save(): Promise<void> {
  if (!props.testCase) return
  if (!name.value.trim() || !urlPath.value.trim()) return
  saving.value = true
  try {
    await props.onSave(buildPayload())
    savedSnapshot.value = currentSnapshotJson()
    emit('update:open', false)
  } finally {
    saving.value = false
  }
}

async function run(): Promise<void> {
  if (!props.testCase) return
  if (!urlPath.value.trim()) return
  running.value = true
  // 保留上一条结果：先清空再等待会让「执行结果」页闪白（P0）
  runError.value = ''
  tab.value = 'result'
  const runId = crypto.randomUUID()
  activeRunId.value = runId
  try {
    result.value = await props.onRun(buildPayload(), runId)
  } catch (err) {
    runError.value = err instanceof Error ? err.message : String(err)
  } finally {
    running.value = false
    if (activeRunId.value === runId) activeRunId.value = null
  }
}

function cancelRun(): void {
  if (!activeRunId.value) return
  void api.cancelTestEndpoint(activeRunId.value)
}

// ---------- 执行结果 Tab ----------
const outcomes = computed(() => viewRunResult.value?.outcomes ?? [])
const outcomeStats = computed(() => {
  const ok = outcomes.value.filter((o) => o.passed).length
  return { ok, fail: outcomes.value.length - ok }
})

const extractedEntries = computed(() => Object.entries(viewRunResult.value?.extracted ?? {}))

function resultToneOf(r: EndpointResult | null): string {
  if (!r) return 'var(--text-3)'
  if (r.cancelled) return 'var(--warning)'
  return r.ok ? 'var(--success)' : 'var(--danger)'
}

// ---------- 执行历史 ----------
const history = ref<TestCaseRun[]>([])
const historyLoading = ref(false)
const historyOpen = ref(false)
const viewRun = ref<TestCaseRun | null>(null)
const viewRunResult = computed<EndpointResult | null>(() => {
  if (!viewRun.value) return result.value
  try {
    return JSON.parse(viewRun.value.results_json) as EndpointResult
  } catch {
    return null
  }
})
/** 勾选两条历史做对比。 */
const compareIds = ref<[string, string] | null>(null)

async function loadHistory(): Promise<void> {
  if (!props.testCase) return
  historyLoading.value = true
  try {
    history.value = await api.listTestCaseRuns(props.testCase.id, 50)
  } catch {
    history.value = []
  } finally {
    historyLoading.value = false
  }
}

watch([historyOpen, () => props.testCase?.id], ([open]) => {
  if (open) void loadHistory()
})

function toggleHistory(): void {
  historyOpen.value = !historyOpen.value
  if (historyOpen.value) compareIds.value = null
}

function viewOf(run: TestCaseRun): void {
  viewRun.value = run
}

function backToLatest(): void {
  viewRun.value = null
}

async function removeRun(run: TestCaseRun): Promise<void> {
  try {
    await api.deleteTestCaseRun(run.id)
    history.value = history.value.filter((h) => h.id !== run.id)
    if (viewRun.value?.id === run.id) viewRun.value = null
  } catch {
    /* 删除失败静默（列表仍在） */
  }
}

function rerunFromHistory(): void {
  viewRun.value = null
  historyOpen.value = false
  void run()
}

function toggleCompare(runId: string): void {
  const cur = compareIds.value
  if (!cur) {
    compareIds.value = [runId, runId]
    return
  }
  if (cur[0] === runId || cur[1] === runId) {
    compareIds.value = null
    return
  }
  compareIds.value = [cur[1], runId]
}

const comparePair = computed<{ a: TestCaseRun; b: TestCaseRun } | null>(() => {
  const ids = compareIds.value
  if (!ids) return null
  const a = history.value.find((h) => h.id === ids[0])
  const b = history.value.find((h) => h.id === ids[1])
  if (!a || !b || a.id === b.id) return null
  return { a, b }
})

const compareResults = computed(() => {
  const pair = comparePair.value
  if (!pair) return null
  const parse = (r: TestCaseRun): EndpointResult | null => {
    try {
      return JSON.parse(r.results_json) as EndpointResult
    } catch {
      return null
    }
  }
  const ra = parse(pair.a)
  const rb = parse(pair.b)
  if (!ra || !rb) return null
  return { a: ra, b: rb, labelA: relativeRunTime(pair.a.started_at), labelB: relativeRunTime(pair.b.started_at) }
})




</script>

<template>
  <Teleport to="body">
    <Transition name="drw">
      <div v-if="open" class="drw-mask" @mousedown.self="attemptClose">
        <div
          class="drw"
          :class="{ 'is-fullscreen': fullscreen, 'is-resizing': resizing }"
          :style="fullscreen ? undefined : { width: `${width}px` }"
          role="dialog"
          aria-modal="true"
        >
          <!-- 左缘宽度拖拽 -->
          <div
            v-if="!fullscreen"
            class="drw-resizer"
            :title="t('casedrawer.resizeHint')"
            @mousedown="onResizeDown"
          ></div>

          <!-- 顶部标题（固定）：名称 / 分类 / 环境 / 全屏 / 关闭 -->
          <div class="drw-header">
            <div class="drw-title-row">
              <Icon name="beaker" :size="15" class="drw-title-icon" />
              <input v-model="name" class="drw-name" type="text" :placeholder="t('cases.colName')" spellcheck="false" />
              <EnvironmentSelector variant="solo" :manage="false" />
              <IconButton
                :name="fullscreen ? 'minimize-2' : 'maximize-2'"
                :size="14"
                :title="fullscreen ? t('casedrawer.exitFullscreen') : t('casedrawer.fullscreen')"
                @click="toggleFullscreen"
              />
              <IconButton name="x" :size="15" :title="t('common.close')" @click="attemptClose" />
            </div>
            <div class="drw-meta-row">
              <CustomSelect
                class="drw-category"
                :model-value="category"
                :options="TEST_CASE_CATEGORIES.map((c) => ({ value: c, label: caseCategoryLabel(c) }))"
                size="sm"
                :pop-min-width="110"
                @update:model-value="(v) => (category = String(v ?? '正向') as TestCaseCategory)"
              />
              <input
                v-model="folder"
                class="miyo-input drw-folder"
                type="text"
                :placeholder="t('testcase.folderPh')"
                spellcheck="false"
                maxlength="60"
              />
              <span v-if="isDirty" class="drw-dirty" :title="t('casedrawer.dirtyHint')">
                <Icon name="dot" :size="9" /> {{ t('casedrawer.dirty') }}
              </span>
            </div>
            <!-- Tab 行 -->
            <div class="drw-tabs">
              <button
                v-for="opt in tabOptions"
                :key="opt.value"
                class="drw-tab"
                :class="{ active: tab === opt.value }"
                type="button"
                @click="tab = opt.value as DrawerTab"
              >
                {{ opt.label }}
                <span v-if="opt.badge" class="drw-tab-badge">{{ opt.badge }}</span>
              </button>
            </div>
            <!-- 未保存确认条 -->
            <div v-if="confirmClose" class="drw-confirm">
              <span>{{ t('casedrawer.unsavedConfirm') }}</span>
              <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="discardAndClose">
                {{ t('casedrawer.discard') }}
              </button>
              <button class="miyo-btn miyo-btn-sm" type="button" @click="confirmClose = false">
                {{ t('casedrawer.keepEditing') }}
              </button>
            </div>
          </div>

          <!-- 内容区（独立滚动） -->
          <div class="drw-body">
            <!-- ===== 请求配置 ===== -->
            <div v-show="tab === 'config'" class="drw-pane">
              <div class="drw-urlrow">
                <CustomSelect
                  class="drw-method"
                  :model-value="method"
                  :options="METHOD_OPTIONS"
                  size="sm"
                  :pop-min-width="120"
                  @update:model-value="onMethodChange"
                >
                  <template #display="{ label }">
                    <span class="drw-method-badge" :class="methodTone(method)">{{ label }}</span>
                  </template>
                </CustomSelect>
                <input
                  v-model="urlPath"
                  class="miyo-input drw-url"
                  type="text"
                  :placeholder="t('casedrawer.urlPh')"
                  spellcheck="false"
                />
              </div>

              <div class="drw-inherit-row">
                <button
                  class="drw-sync-btn"
                  type="button"
                  :disabled="!endpoint"
                  :title="t('casedrawer.syncHint')"
                  @click="syncFromEndpoint"
                >
                  <Icon name="refresh" :size="12" /> {{ t('casedrawer.syncFromEndpoint') }}
                </button>
                <span class="drw-inherit-flags">
                  <span class="drw-inherit-tag" :class="{ over: !inheritFlags.params }">
                    {{ inheritFlags.params ? t('casedrawer.inherited') : t('casedrawer.overridden') }} · {{ t('casedrawer.tabParams') }}
                  </span>
                  <span class="drw-inherit-tag" :class="{ over: !inheritFlags.headers }">
                    {{ inheritFlags.headers ? t('casedrawer.inherited') : t('casedrawer.overridden') }} · {{ t('casedrawer.tabHeaders') }}
                  </span>
                  <span class="drw-inherit-tag" :class="{ over: !inheritFlags.body }">
                    {{ inheritFlags.body ? t('casedrawer.inherited') : t('casedrawer.overridden') }} · {{ t('casedrawer.tabBody') }}
                  </span>
                </span>
              </div>

              <div class="drw-section-tabs">
                <button
                  v-for="sec in configSections"
                  :key="sec.value"
                  class="drw-section-tab"
                  :class="{ active: configSection === sec.value }"
                  type="button"
                  @click="configSection = sec.value as ConfigSection"
                >
                  {{ sec.label }}
                </button>
              </div>

              <div v-if="caseEndpoint" class="drw-section-body" :class="{ 'is-body': configSection === 'body' }">
                <ParamsPanel v-show="configSection === 'params'" :draft="caseEndpoint" />
                <PathVariablesPanel v-show="configSection === 'path'" :draft="caseEndpoint" />
                <HeadersPanel v-show="configSection === 'headers'" :draft="caseEndpoint" />
                <AuthPanel v-show="configSection === 'auth'" :draft="caseEndpoint" />
                <BodyPanel v-show="configSection === 'body'" :draft="caseEndpoint" />
              </div>

              <div class="drw-exec-controls">
                <label class="drw-exec-field">
                  <span class="drw-exec-label">{{ t('casedrawer.timeout') }}</span>
                  <input
                    class="miyo-input drw-timeout"
                    type="number"
                    min="0"
                    max="60000"
                    :placeholder="t('casedrawer.timeoutPh')"
                    :value="timeoutMs ?? ''"
                    @input="timeoutMs = ($event.target as HTMLInputElement).value ? Number(($event.target as HTMLInputElement).value) : null"
                  />
                  <span class="drw-exec-unit">{{ t("casedrawer.msUnit") }}</span>
                </label>
                <label class="drw-exec-check">
                  <input v-model="disableCookies" type="checkbox" />
                  <span>{{ t('casedrawer.disableCookies') }}</span>
                </label>
              </div>
            </div>

            <!-- ===== 断言与提取 ===== -->
            <div v-show="tab === 'checks'" class="drw-pane">
              <section class="drw-checks-sec">
                <h4 class="drw-sec-title">{{ t('casedrawer.secExtractors') }}</h4>
                <ExtractList v-model:rules="extractors" />
              </section>
              <section class="drw-checks-sec">
                <h4 class="drw-sec-title">{{ t('casedrawer.secAssertions') }}</h4>
                <AssertionList v-model:rules="assertions" />
                <p class="drw-js-hint">{{ t('casedrawer.jsHint') }}</p>
              </section>
            </div>

            <!-- ===== 执行结果 ===== -->
            <div v-show="tab === 'result'" class="drw-pane drw-pane-result">
              <!-- 历史回看头 -->
              <div v-if="viewRun" class="drw-history-viewing">
                <IconButton class="drw-back" name="chevron-right" :size="13" :title="t('caselog.backLatest')" @click="backToLatest" />
                <span>{{ t('caselog.viewing', { time: relativeRunTime(viewRun.started_at) }) }}</span>
                <span class="drw-history-viewing-status" :style="{ color: resultToneOf(viewRunResult) }">
                  {{ viewRun.status }}
                </span>
                <IconButton name="trash" :size="13" :title="t('common.delete')" @click="removeRun(viewRun)" />
              </div>

              <!-- 摘要 -->
              <div v-if="viewRunResult" class="drw-result-head">
                <span class="drw-result-status" :style="{ color: resultToneOf(viewRunResult) }">
                  {{ viewRunResult.status ?? '—' }}
                  <template v-if="viewRunResult.status != null">{{ statusTextOf(viewRunResult.status) }}</template>
                </span>
                <span class="drw-result-chip">{{ formatDuration(viewRunResult.duration_ms ?? 0) }}</span>
                <span v-if="viewRunResult.response" class="drw-result-chip">
                  {{ (viewRunResult.response.size_bytes / 1024).toFixed(1) }}KB
                </span>
                <span class="drw-result-chip" :class="{ fail: outcomeStats.fail }">
                  {{ t('caselog.assertStats', { ok: outcomeStats.ok, fail: outcomeStats.fail }) }}
                </span>
              </div>
              <p v-else-if="runError" class="drw-run-error">{{ t('casedrawer.runFailed') }}：{{ runError }}</p>
              <div v-else class="drw-result-empty">
                <Icon name="play" :size="18" />
                <p>{{ t('caselog.emptyHint') }}</p>
              </div>

              <!-- 断言明细 -->
              <div v-if="viewRunResult && outcomes.length" class="drw-outcomes">
                <div v-for="(o, i) in outcomes" :key="i" class="drw-outcome" :class="{ fail: !o.passed }">
                  <span class="drw-outcome-mark">{{ o.passed ? '✓' : '✕' }}</span>
                  <span class="drw-outcome-desc" :title="o.description">{{ o.description }}</span>
                  <span v-if="o.expected != null" class="drw-outcome-pair">
                    {{ t('caselog.expected') }} <code>{{ o.expected }}</code>
                  </span>
                  <span v-if="o.actual != null" class="drw-outcome-pair">
                    {{ t('caselog.actual') }} <code>{{ o.actual }}</code>
                  </span>
                  <span v-if="!o.passed && o.reason" class="drw-outcome-reason" :title="o.reason">{{ o.reason }}</span>
                </div>
              </div>

              <!-- 提取值 -->
              <div v-if="viewRunResult && extractedEntries.length" class="drw-extracted">
                <span class="drw-sec-title-sm">{{ t('caselog.extracted') }}</span>
                <code v-for="[k, v] in extractedEntries" :key="k" class="drw-extracted-item">
                  {{ k }} = {{ v }}
                </code>
              </div>

              <!-- 请求快照 + 响应（变量替换后的真值） -->
              <template v-if="viewRunResult?.response">
                <RequestDetail :response="viewRunResult.response" :url="viewRunResult.url ?? undefined" />
                <div class="drw-response-wrap">
                  <ResponsePanel :response="viewRunResult.response as unknown as import('../types/miyoApi').ExecuteResponse" />
                </div>
              </template>

              <!-- 执行历史 -->
              <div class="drw-history">
                <button class="drw-history-toggle" type="button" @click="toggleHistory">
                  <Icon :name="historyOpen ? 'chevron-down' : 'chevron-right'" :size="12" />
                  {{ t('caselog.title') }}
                  <span v-if="history.length" class="drw-tab-badge">{{ history.length }}</span>
                  <template v-if="comparePair && compareResults">
                    <span class="drw-history-compare-hint">{{ t('caselog.comparing', { a: compareResults.labelA, b: compareResults.labelB }) }}</span>
                  </template>
                </button>
                <div v-if="historyOpen" class="drw-history-list">
                  <div v-if="historyLoading" class="drw-history-loading">
                    <Skeleton :lines="3" width="92%" height="14px" />
                  </div>
                  <p v-else-if="!history.length" class="drw-history-empty">{{ t('caselog.empty') }}</p>
                  <template v-else>
                    <TestCaseRunCompare
                      v-if="compareResults"
                      class="drw-compare"
                      :a="compareResults.a"
                      :b="compareResults.b"
                      :label-a="compareResults.labelA"
                      :label-b="compareResults.labelB"
                    />
                    <div v-for="h in history" :key="h.id" class="drw-history-row" :class="{ viewing: viewRun?.id === h.id }">
                      <input
                        type="checkbox"
                        class="drw-history-check"
                        :checked="!!compareIds && compareIds.includes(h.id)"
                        :title="t('caselog.pickCompare')"
                        @change="toggleCompare(h.id)"
                      />
                      <button class="drw-history-main" type="button" @click="viewOf(h)">
                        <span class="drw-history-status" :style="{ color: h.status === 'Success' ? 'var(--success)' : 'var(--danger)' }">
                          {{ h.status }}
                        </span>
                        <span class="drw-history-time">{{ relativeRunTime(h.started_at) }}</span>
                        <span class="drw-history-dur">{{ formatDuration(h.duration_ms ?? 0) }}</span>
                        <span class="drw-history-asserts" :class="{ fail: h.fail_count }">
                          {{ h.ok_count }}/{{ h.ok_count + h.fail_count }}
                        </span>
                        <span v-if="h.status_code" class="drw-history-code">HTTP {{ h.status_code }}</span>
                      </button>
                      <IconButton name="trash" :size="12" :title="t('common.delete')" @click="removeRun(h)" />
                    </div>
                    <button class="miyo-btn miyo-btn-sm drw-history-rerun" type="button" @click="rerunFromHistory">
                      <Icon name="play" :size="12" /> {{ t('caselog.rerun') }}
                    </button>
                  </template>
                </div>
              </div>
            </div>
          </div>

          <!-- 底部操作栏（固定） -->
          <div class="drw-footer">
            <CodeExportMenu
              class="drw-export"
              :method="method"
              :url="exportUrl"
              :headers="caseEndpoint?.request.headers ?? []"
              :body="caseEndpoint?.request.body ?? ({ mode: 'none' } as BodySpec)"
              :auth="(auth ?? { type: 'none' }) as AuthSpec"
              :disabled="!urlPath.trim()"
              placement="top"
            />
            <div class="drw-footer-main">
              <button
                v-if="running && activeRunId"
                class="miyo-btn miyo-btn-sm miyo-btn-danger"
                type="button"
                @click="cancelRun"
              >
                {{ t('common.cancel') }}
              </button>
              <button
                class="miyo-btn miyo-btn-sm miyo-btn-primary"
                type="button"
                :disabled="running || !urlPath.trim()"
                @click="run"
              >
                <Icon name="play" :size="13" />
                {{ running ? t('casedrawer.sending') : t('casedrawer.runNow') }}
              </button>
              <button
                class="miyo-btn miyo-btn-sm"
                type="button"
                :disabled="saving || !name.trim() || !urlPath.trim()"
                @click="save"
              >
                {{ t('casedrawer.save') }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.drw-mask {
  position: fixed;
  inset: 0;
  z-index: var(--z-drawer);
  background: var(--mask);
  backdrop-filter: blur(2px);
}

.drw {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  display: flex;
  flex-direction: column;
  max-width: 100vw;
  background: var(--bg-panel);
  border-left: 1px solid var(--border);
  box-shadow: var(--shadow-drawer);
}
.drw.is-fullscreen {
  width: 100vw !important;
  border-left: none;
}

/* 左缘拖拽把手：透明叠加在边框上，hover 高亮 */
.drw-resizer {
  position: absolute;
  left: -4px;
  top: 0;
  bottom: 0;
  width: 8px;
  cursor: col-resize;
  z-index: 2;
}
.drw-resizer:hover,
.drw.is-resizing {
  cursor: col-resize;
}
.drw-resizer:hover {
  background: color-mix(in srgb, var(--accent) 30%, transparent);
}

/* ---- 头部（固定） ---- */
.drw-header {
  flex: none;
  padding: 10px 14px 0;
  border-bottom: 1px solid var(--border);
}

.drw-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.drw-title-icon {
  color: var(--accent);
  flex-shrink: 0;
}
.drw-name {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  font-family: inherit;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-1);
  outline: none;
}

.drw-meta-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}
.drw-category {
  width: 110px;
  flex-shrink: 0;
}
.drw-folder {
  flex: 1;
  min-width: 0;
  height: 26px;
  font-size: var(--fs-xs);
}
.drw-dirty {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

/* Tab 行 */
.drw-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-top: 8px;
}
.drw-tab {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 7px 12px;
  border: none;
  border-bottom: 2px solid transparent;
  background: none;
  font-family: inherit;
  font-size: var(--fs-sm);
  color: var(--text-2);
  cursor: pointer;
  transition: color var(--dur) var(--ease);
}
.drw-tab:hover {
  color: var(--text-1);
}
.drw-tab.active {
  color: var(--text-1);
  border-bottom-color: var(--accent);
}
.drw-tab-badge {
  min-width: 16px;
  padding: 0 4px;
  border-radius: 999px;
  background: var(--accent-tint);
  color: var(--accent);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  text-align: center;
  line-height: 16px;
}

/* 未保存确认条 */
.drw-confirm {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0 10px;
  padding: 7px 10px;
  border: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--warning) 10%, transparent);
  font-size: var(--fs-xs);
  color: var(--text-1);
}
.drw-confirm span {
  margin-right: auto;
}

/* ---- 内容区（独立滚动） ---- */
.drw-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
}

.drw-pane {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* URL 行 */
.drw-urlrow {
  display: flex;
  align-items: center;
  gap: 6px;
}
.drw-method {
  width: 118px;
  flex-shrink: 0;
}
/* 触发区方法徽章：与场景步骤 / 主请求栏同款（method-* 语义色，深浅主题自适应）。 */
.drw-method-badge {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.04em;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}
.drw-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
}

/* 继承 / 同步行 */
.drw-inherit-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.drw-sync-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition: all var(--dur) var(--ease);
}
.drw-sync-btn:hover:not(:disabled) {
  color: var(--text-1);
  border-color: var(--border-strong);
}
.drw-sync-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.drw-inherit-flags {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.drw-inherit-tag {
  padding: 1px 7px;
  border-radius: 999px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  background: var(--bg-2);
  border: 1px solid var(--border);
}
.drw-inherit-tag.over {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  background: var(--accent-tint);
}

/* 配置分段页签 */
.drw-section-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  border-bottom: 1px solid var(--border);
}
.drw-section-tab {
  padding: 5px 10px;
  border: none;
  border-bottom: 2px solid transparent;
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
}
.drw-section-tab:hover {
  color: var(--text-1);
}
.drw-section-tab.active {
  color: var(--text-1);
  border-bottom-color: var(--accent);
}
.drw-section-body {
  min-height: 120px;
}
/**
 * BodyPanel 的高度契约：面板 `flex:1`、JsonEditor 填满分派高度（调试器里由卡片 /
 * 分割条给高度）。抽屉是纵向滚动容器，没有分割条分派高度，父级高度 auto →
 * `.json-editor` 只能退回 wrap 的 120px 地板并转框内滚动，看起来就是「框很小、
 * 只能看到中间几行」。这里显式分派一个可视高度，编辑器行为与主请求面板一致。
 */
.drw-section-body.is-body {
  display: flex;
  flex-direction: column;
  height: clamp(240px, 42vh, 460px);
  min-height: 0;
}

/* 执行控制 */
.drw-exec-controls {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-2);
}
.drw-exec-field {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.drw-exec-label {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.drw-timeout {
  width: 90px;
  height: 26px;
  font-size: var(--fs-xs);
}
.drw-exec-unit {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.drw-exec-check {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
}
.drw-exec-check input {
  accent-color: var(--accent);
}

/* 断言与提取 Tab */
.drw-checks-sec {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.drw-sec-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.drw-sec-title-sm {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.drw-js-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.6;
}

/* 执行结果 */
.drw-history-viewing {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.drw-history-viewing-status {
  font-family: var(--font-mono);
}

.drw-result-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.drw-result-status {
  font-family: var(--font-mono);
  font-size: 16px;
  font-weight: 700;
}
.drw-result-chip {
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  color: var(--text-2);
}
.drw-result-chip.fail {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, transparent);
}

.drw-run-error {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--danger);
  word-break: break-all;
}

.drw-result-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 24px 0;
  color: var(--text-3);
  font-size: var(--fs-xs);
}

/* 断言明细 */
.drw-outcomes {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.drw-outcome {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  font-size: var(--fs-xs);
  border-top: 1px solid var(--border);
  flex-wrap: wrap;
}
.drw-outcome:first-child {
  border-top: none;
}
.drw-outcome.fail {
  background: color-mix(in srgb, var(--danger) 6%, transparent);
}
.drw-outcome-mark {
  flex-shrink: 0;
  font-family: var(--font-mono);
  color: var(--success);
}
.drw-outcome.fail .drw-outcome-mark {
  color: var(--danger);
}
.drw-outcome-desc {
  color: var(--text-1);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 40%;
}
.drw-outcome-pair {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}
.drw-outcome-pair code {
  font-family: var(--font-mono);
  color: var(--text-2);
}
.drw-outcome-reason {
  margin-left: auto;
  font-size: var(--fs-xxs);
  color: var(--danger);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 30%;
}

/* 提取值 */
.drw-extracted {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.drw-extracted-item {
  padding: 2px 8px;
  border-radius: var(--radius);
  background: var(--bg-code);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.drw-response-wrap {
  min-height: 220px;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

/* 执行历史 */
.drw-history {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.drw-history-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  align-self: flex-start;
  padding: 3px 0;
  border: none;
  background: none;
  font-family: inherit;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  cursor: pointer;
}
.drw-history-compare-hint {
  font-weight: 400;
  font-size: var(--fs-xxs);
  color: var(--accent);
}
.drw-history-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.drw-history-loading,
.drw-history-empty {
  font-size: var(--fs-xs);
  color: var(--text-3);
  padding: 8px 0;
}
.drw-history-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
}
.drw-history-row.viewing {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}
.drw-history-check {
  accent-color: var(--accent);
  flex-shrink: 0;
}
.drw-history-main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  border: none;
  background: none;
  padding: 0;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  text-align: left;
}
.drw-history-status {
  font-family: var(--font-mono);
  flex-shrink: 0;
}
.drw-history-time {
  color: var(--text-3);
  flex-shrink: 0;
}
.drw-history-dur,
.drw-history-asserts,
.drw-history-code {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.drw-history-asserts.fail {
  color: var(--danger);
}
.drw-compare {
  margin-bottom: 6px;
}
.drw-history-rerun {
  align-self: flex-start;
}

/* ---- 底部操作栏（固定） ---- */
.drw-footer {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-top: 1px solid var(--border);
  background: var(--bg-panel);
}
.drw-export {
  flex-shrink: 0;
}
.drw-footer-main {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.drw-back {
  transform: rotate(180deg);
}

/* ---- 动画（保留原 drw 过渡） ---- */
.drw-enter-active,
.drw-leave-active {
  transition: opacity 0.18s ease;
}
.drw-enter-active .drw,
.drw-leave-active .drw {
  transition: transform 0.22s cubic-bezier(0.2, 0, 0, 1);
}
.drw-enter-from,
.drw-leave-to {
  opacity: 0;
}
.drw-enter-from .drw,
.drw-leave-to .drw {
  transform: translateX(100%);
}
</style>
