<script setup lang="ts">
/**
 * TestCaseDrawer：测试用例详情 / 编辑抽屉（右侧滑出，原地运行）。
 * - 三区分区：用例基本信息 / 请求区（Params · Headers · Body 微型 Tab）+ 响应区（拖拽分割）；
 * - Method + Path 无缝组合输入（Method Badge 下拉 + Path flex-1），Method 联动默认 Tab 与 Content-Type；
 * - Body 与 Response 均使用 CodeMirror 6（JSON 暗黑高亮 / 折叠 / 错误校验 / 只读）；
 * - 请求区与响应区之间可拖拽分割条（双击恢复 50%，双方 min-height 120px）；
 * - 底部操作栏 sticky，左侧「导出代码」（复用调试页 CodeExportMenu，向上弹出 +
 *   左对齐向右展开，弹出层不越出抽屉）+「立即运行」、右侧「取消 / 保存修改」。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { AuthSpec, BodySpec, ExecuteResponse, HttpMethod, KeyValue, TestCase, TestCaseCategory } from '../types/miyoApi'
import { TEST_CASE_CATEGORIES, caseCategoryLabel, formatDuration, restoreBody, statusToneOf, statusTextOf } from '../utils/testCases'
import { applyPathVars, extractPathVarNames } from '../utils/pathVariables'
import { methodTextTone } from '../utils/methodTone'
import { useLocaleStore } from '../stores/locale'
import CodeExportMenu from './CodeExportMenu.vue'
import CustomSelect from './ui/CustomSelect.vue'
import EnvironmentSelector from './EnvironmentSelector.vue'
import FindBar from './ui/FindBar.vue'
import Icon from './ui/Icon.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import KeyValueTable, { type KVRow } from './ui/KeyValueTable.vue'

const props = defineProps<{
  open: boolean
  /** 当前接口 id（request_id），用于保存后回填本地列表。 */
  endpointId: string
  testCase: TestCase | null
  /** 路径 → 可运行 URL（环境 Base URL + 变量解析），导出代码用；与 onRun/onSave 同为父级注入。 */
  resolveUrl: (path: string) => string
  /** 运行回调：返回响应或抛错。由父级（store 或 panel）注入，避免组件耦合执行细节。 */
  onRun: (payload: {
    method: HttpMethod
    urlPath: string
    pathVariables: KeyValue[]
    params: KeyValue[]
    headers: KeyValue[]
    bodyType: string
    bodyContent: string
  }) => Promise<ExecuteResponse | null>
  onSave: (payload: {
    name: string
    category: TestCaseCategory
    method: HttpMethod
    urlPath: string
    pathVariables: KeyValue[]
    params: KeyValue[]
    headers: KeyValue[]
    bodyType: string
    bodyContent: string
  }) => Promise<void>
}>()

const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const locale = useLocaleStore()
const t = locale.t

/** 环境选项与切换动作都在共享的 EnvironmentSelector 内（同源 store），此处不再自持。 */
const name = ref('')
const category = ref<TestCaseCategory>('正向')
const method = ref<HttpMethod>('GET')
const urlPath = ref('')
/** 路径变量取值（`/users/{id}` 里的 id）；用例快照的一部分，随保存落库。 */
const pathVariables = ref<KeyValue[]>([])
const params = ref<KeyValue[]>([])
const headers = ref<KeyValue[]>([])
const bodyType = ref('none')
const bodyContent = ref('')

/** 方法下拉：选项文字按方法语义色渲染（methodTone 单源），选中项浅色高亮。 */
const METHOD_OPTIONS = (['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS'] as HttpMethod[]).map(
  (m) => ({ value: m, label: m, tone: methodTextTone(m) }),
)
/** Body 类型下拉（value 为 body_type 标识，label 随语言刷新）。 */
const bodyOptions = computed(() => [
  { value: 'none', label: t('cases.bodyNone') },
  { value: 'json', label: 'JSON' },
  { value: 'form-data', label: 'Form-Data' },
  { value: 'raw', label: 'Raw' },
  { value: 'urlencoded', label: 'URL-Encoded' },
  { value: 'graphql', label: 'GraphQL' },
  { value: 'binary', label: 'Binary' },
])

// ---------- 请求参数 Tab ----------
type ReqTab = 'params' | 'path' | 'headers' | 'body'
const activeTab = ref<ReqTab>('params')
/** 路径变量页签只在 Path 含 `{var}` 时出现，避免无谓噪音。 */
const hasPathVars = computed(() => extractPathVarNames(urlPath.value).length > 0)
const reqTabs = computed<{ key: ReqTab; label: string }[]>(() => [
  { key: 'params', label: t('casedrawer.tabParams') },
  ...(hasPathVars.value ? [{ key: 'path' as ReqTab, label: t('casedrawer.tabPath') }] : []),
  { key: 'headers', label: t('casedrawer.tabHeaders') },
  { key: 'body', label: t('casedrawer.tabBody') },
])

const result = ref<ExecuteResponse | null>(null)
const runError = ref('')

// ---------- Method 联动：默认 Tab 与 Content-Type ----------
const BODY_METHODS = ['POST', 'PUT', 'PATCH']
const NO_BODY_METHODS = ['GET', 'DELETE', 'HEAD', 'OPTIONS']

/** POST/PUT/PATCH → Body Tab + Content-Type: application/json；其余 → Params Tab。 */
function applyMethodDefaults(m: HttpMethod): void {
  if (BODY_METHODS.includes(m)) {
    activeTab.value = 'body'
    if (bodyType.value === 'none') bodyType.value = 'json'
    if (!headers.value.some((h) => h.key?.toLowerCase() === 'content-type')) {
      headers.value.push({ key: 'Content-Type', value: 'application/json', enabled: true, description: '' })
    }
  } else {
    activeTab.value = 'params'
  }
}

watch(method, (m) => applyMethodDefaults(m))

// ---------- 请求 Body 查找（Find in Request Body，与 BodyPanel 同款交互） ----------
const drawerRef = ref<HTMLElement | null>(null)
const bodyFindOpen = ref(false)
const bodyQuery = ref('')
const bodyActiveMatch = ref(0)

/** 防抖后的实际搜索词：输入跟手，CodeMirror 跳转走防抖值（BodyPanel 同款 160ms）。 */
const bodySearchQuery = ref('')
let bodySearchTimer: ReturnType<typeof setTimeout> | undefined

watch(bodyQuery, (q) => {
  if (bodySearchTimer) clearTimeout(bodySearchTimer)
  if (!q) {
    bodySearchQuery.value = ''
    return
  }
  bodySearchTimer = setTimeout(() => {
    bodySearchQuery.value = q
  }, 160)
})

watch(bodyQuery, () => {
  bodyActiveMatch.value = 0
})

function countBodyOccurrences(text: string, q: string): number {
  if (!text || !q) return 0
  const ql = q.toLowerCase()
  const lower = text.toLowerCase()
  let n = 0
  let from = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) break
    n += 1
    from = idx + ql.length
  }
  return n
}

const bodyTotal = computed(() => countBodyOccurrences(bodyContent.value, bodySearchQuery.value))

watch(bodyTotal, (n) => {
  if (n === 0) bodyActiveMatch.value = 0
  else if (bodyActiveMatch.value >= n) bodyActiveMatch.value = n - 1
})

/** 跳转到当前匹配：经 JsonCodeMirror 选中对应区间并滚动可见。
 * 仅显式上一处/下一处导航时调用——输入过程中只更新计数，
 * 不触碰编辑器，否则每敲一个字符就会被抢走光标。 */
function jumpBodyMatch(): void {
  if (!bodySearchQuery.value || !bodyTotal.value) return
  bodyEditorRef.value?.selectMatch(bodySearchQuery.value, bodyActiveMatch.value)
}

function nextBodyMatch(): void {
  if (!bodyTotal.value) return
  bodyActiveMatch.value = (bodyActiveMatch.value + 1) % bodyTotal.value
  jumpBodyMatch()
}

function prevBodyMatch(): void {
  if (!bodyTotal.value) return
  bodyActiveMatch.value = (bodyActiveMatch.value - 1 + bodyTotal.value) % bodyTotal.value
  jumpBodyMatch()
}

function closeBodyFind(): void {
  bodyFindOpen.value = false
  bodyQuery.value = ''
  bodySearchQuery.value = ''
  bodyActiveMatch.value = 0
}

function toggleBodyFind(): void {
  if (bodyFindOpen.value) closeBodyFind()
  else bodyFindOpen.value = true
}

/** 抽屉内 ⌘F / Ctrl+F 聚焦 Body 查找（FindBar 输入中不重复触发）。 */
function onDrawerKeydown(e: KeyboardEvent): void {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'f') return
  const target = e.target as HTMLElement | null
  if (target?.closest('.findbar')) return
  const root = drawerRef.value
  if (root && target && root.contains(target)) {
    if (!bodyEditable.value) return
    e.preventDefault()
    bodyFindOpen.value = true
  }
}

onMounted(() => {
  window.addEventListener('keydown', onDrawerKeydown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onDrawerKeydown)
  if (bodySearchTimer) clearTimeout(bodySearchTimer)
})

// ---------- 打开时同步草稿 ----------
// 同时监听 `props.open` 与 `props.testCase`：抽屉已打开（open 恒为 true）时
// 切换用例，`drawerCase` 会变而 `open` 不变，若只监听 open 则本地编辑 ref
// 仍是旧用例数据——保存时把旧 body/params/headers 写进新用例，表现为
// 「保存成功但 body 还是修改前的样子」（偶发竞态）。监听 testCase 后任何
// 用例切换都强制重新同步。
watch(
  [() => props.open, () => props.testCase],
  ([open, testCase]) => {
    if (!open || !testCase) return
    name.value = testCase.name
    category.value = testCase.category
    method.value = testCase.method
    urlPath.value = testCase.url_path
    pathVariables.value = (testCase.path_variables ?? []).map((p) => ({ ...p }))
    params.value = testCase.params.map((p) => ({ ...p }))
    headers.value = testCase.headers.map((h) => ({ ...h }))
    bodyType.value = testCase.body_type
    bodyContent.value = testCase.body_content
    activeTab.value = 'params'
    syncPathVars()
    result.value = null
    runError.value = ''
    closeBodyFind()
    applyMethodDefaults(method.value)
  },
  { immediate: true },
)

// ---------- 请求参数 Tab ----------
const paramRows = computed({
  get: () => params.value,
  set: (rows: KVRow[]) => {
    params.value = rows as KeyValue[]
  },
})
const headerRows = computed({
  get: () => headers.value,
  set: (rows: KVRow[]) => {
    headers.value = rows as KeyValue[]
  },
})
const pathVarRows = computed({
  get: () => pathVariables.value,
  set: (rows: KVRow[]) => {
    pathVariables.value = rows as KeyValue[]
  },
})

/**
 * 路径变量补齐：用例可能存于「模型无 path_variables」的旧版本，或 URL 后来
 * 才加上 `{id}`——按 URL 模板补出缺失行（保留已有取值，不删用户手写的行）。
 */
function syncPathVars(): void {
  const names = extractPathVarNames(urlPath.value)
  if (!names.length) return
  const existing = new Set(pathVariables.value.map((p) => p.key))
  for (const n of names) {
    if (!existing.has(n)) {
      pathVariables.value.push({ key: n, value: '', enabled: true, description: '' })
    }
  }
}
watch(urlPath, syncPathVars)

// ---------- 请求参数 Tab ----------
const bodyLabel = computed(() => bodyOptions.value.find((o) => o.value === bodyType.value)?.label ?? bodyType.value)
const bodyEditable = computed(() => bodyType.value !== 'none')

// ---------- 导出代码（复用调试页 CodeExportMenu） ----------
/** 用例没有独立鉴权配置（运行时 auth 固定 none），导出与实际执行一致。 */
const NO_AUTH: AuthSpec = { type: 'none' }
/** 导出用 URL：Path 非空时先代入路径变量，再经父级解析（Base URL + 环境变量）。 */
const exportUrl = computed(() => {
  const rendered = applyPathVars(urlPath.value.trim(), pathVariables.value)
  return rendered ? props.resolveUrl(rendered) : ''
})
/** body_type + 内容 → BodySpec（JSON 非法时降级 raw 文本，与运行时 restoreBody 同源）。 */
const exportBody = computed<BodySpec>(() => restoreBody(bodyType.value, bodyContent.value))

// ---------- 底部操作 ----------
const saving = ref(false)
const running = ref(false)

function payload() {
  return {
    method: method.value,
    urlPath: urlPath.value,
    pathVariables: pathVariables.value.filter((p) => p.key || p.value),
    params: params.value.filter((p) => p.key || p.value),
    headers: headers.value.filter((h) => h.key || h.value),
    bodyType: bodyType.value,
    bodyContent: bodyContent.value,
  }
}

async function save(): Promise<void> {
  if (!props.testCase) return
  if (!name.value.trim() || !urlPath.value.trim()) return
  saving.value = true
  try {
    await props.onSave({ ...payload(), name: name.value.trim(), category: category.value })
    emit('update:open', false)
  } finally {
    saving.value = false
  }
}

async function run(): Promise<void> {
  if (!props.testCase) return
  if (!urlPath.value.trim()) return
  running.value = true
  result.value = null
  runError.value = ''
  try {
    result.value = await props.onRun(payload())
  } catch (err) {
    runError.value = err instanceof Error ? err.message : String(err)
  } finally {
    running.value = false
  }
}

// ---------- 响应展示 ----------
/** 响应 Body：JSON 时美化缩进，非 JSON 原样展示（交由 CodeMirror 只读渲染）。 */
const prettyBody = computed(() => {
  const text = result.value?.body ?? ''
  if (!text) return ''
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
})

// ---------- Method 联动：默认 Tab 与 Content-Type ----------
const bodyTabHint = computed(() =>
  activeTab.value === 'body' && NO_BODY_METHODS.includes(method.value)
    ? t('casedrawer.noBodyHint', { m: method.value })
    : '',
)

const copied = ref(false)

async function copyBody(): Promise<void> {
  if (!result.value?.body) return
  await navigator.clipboard.writeText(result.value.body)
  copied.value = true
  setTimeout(() => (copied.value = false), 1500)
}

/** 大小格式化：<1KB 显示 B，否则 KB（1 位小数）。 */
function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`
  return `${(bytes / 1024).toFixed(1)}KB`
}

const resultTone = computed(() => (result.value ? statusToneOf(result.value.status) : 'info'))

/** Body 自动格式化（JSON / GraphQL 可解析时美化缩进）。 */
function formatBody(): void {
  if (!['json', 'graphql'].includes(bodyType.value)) return
  try {
    bodyContent.value = JSON.stringify(JSON.parse(bodyContent.value), null, 2)
  } catch {
    /* 非法 JSON 不动 */
  }
}

const methodClass = computed(() => methodTextTone(method.value))

// ---------- 请求区 / 响应区垂直拖拽分割 ----------
const splitArea = ref<HTMLElement | null>(null)
const bodyEditorRef = ref<InstanceType<typeof JsonCodeMirror> | null>(null)
const respEditorRef = ref<InstanceType<typeof JsonCodeMirror> | null>(null)

/** 请求区占比（0~1），响应区占剩余空间。 */
const reqRatio = ref(0.55)
const MIN_PX = 120
let dragging = false
let startY = 0
let startRatio = 0

/** 请求区百分比（去除浮点尾差，如 55.00000000000001 → 55%）。 */
const reqPct = computed(() => `${parseFloat((reqRatio.value * 100).toFixed(2))}%`)

function measureEditors(): void {
  requestAnimationFrame(() => {
    bodyEditorRef.value?.requestMeasure()
    respEditorRef.value?.requestMeasure()
  })
}

function onSplitterDown(e: MouseEvent): void {
  if (e.button !== 0) return
  e.preventDefault()
  dragging = true
  startY = e.clientY
  startRatio = reqRatio.value
  window.addEventListener('mousemove', onSplitterMove)
  window.addEventListener('mouseup', onSplitterUp)
}

function onSplitterMove(e: MouseEvent): void {
  if (!dragging) return
  const rect = splitArea.value?.getBoundingClientRect()
  if (!rect || rect.height === 0) return
  const min = MIN_PX / rect.height
  const ratio = startRatio + (e.clientY - startY) / rect.height
  reqRatio.value = Math.min(1 - min, Math.max(min, ratio))
  measureEditors()
}

function onSplitterUp(): void {
  dragging = false
  window.removeEventListener('mousemove', onSplitterMove)
  window.removeEventListener('mouseup', onSplitterUp)
}

function onSplitterDblClick(): void {
  reqRatio.value = 0.5
  measureEditors()
}
</script>

<template>
  <Teleport to="body">
    <Transition name="drw">
      <div v-if="open" class="drw-mask" @mousedown.self="emit('update:open', false)">
        <aside ref="drawerRef" class="drw" role="dialog" aria-modal="true">
          <header class="drw-head">
            <h3 class="drw-title">{{ t('casedrawer.title') }}</h3>
            <button
              class="drw-close"
              type="button"
              :title="t('common.close')"
              :aria-label="t('common.close')"
              @click="emit('update:open', false)"
            >
              <Icon name="x" :size="15" />
            </button>
          </header>

          <div class="drw-body">
            <!-- ① 用例基本信息（右上角环境选择器：运行前确认/切换请求发往哪个环境） -->
            <section class="drw-sec">
              <div class="drw-sec-head">
                <h4 class="drw-sec-title">{{ t('casedrawer.basicInfo') }}</h4>
                <!-- 与顶栏同一个环境选择器（solo 外观）：运行前确认 / 切换请求发往哪个环境；
                     manage=false —— 抽屉遮罩(120)盖得住 Modal(100)，且不拉环境管理大 chunk -->
                <EnvironmentSelector
                  class="drw-env-select"
                  variant="solo"
                  :manage="false"
                  :pop-min-width="240"
                />
              </div>
              <div class="drw-grid3">
                <label class="drw-field drw-span2">
                  <span class="drw-label">{{ t('testcase.name') }}</span>
                  <input v-model="name" class="drw-input" type="text" spellcheck="false" :placeholder="t('testcase.namePh')" />
                </label>
                <label class="drw-field">
                  <span class="drw-label">{{ t('casedrawer.category') }}</span>
                  <CustomSelect
                    :model-value="category"
                    :options="TEST_CASE_CATEGORIES.map((c) => ({ value: c, label: caseCategoryLabel(c) }))"
                    @update:model-value="category = String($event) as TestCaseCategory"
                  />
                </label>
              </div>

              <!-- Method + Path 组合输入 -->
              <label class="drw-field">
                <span class="drw-label">{{ t('casedrawer.url') }}</span>
                <div class="drw-url-group">
                  <CustomSelect
                    class="drw-method"
                    :class="methodClass"
                    :model-value="method"
                    :options="METHOD_OPTIONS"
                    pop-class="cs-method-pop"
                    :pop-min-width="130"
                    @update:model-value="method = String($event) as HttpMethod"
                  >
                    <template #display="{ label }">
                      <span :class="['drw-method-badge', methodTextTone(label)]">{{ label }}</span>
                    </template>
                  </CustomSelect>
                  <input
                    v-model="urlPath"
                    class="drw-input drw-mono drw-path-input"
                    type="text"
                    spellcheck="false"
                    :placeholder="t('casedrawer.urlPh')"
                  />
                </div>
              </label>
            </section>

            <!-- ②③ 请求区 / 响应区：垂直拖拽分割 -->
            <div ref="splitArea" class="drw-split">
              <section class="drw-sec drw-req-sec" :style="{ flexBasis: reqPct }">
                <div class="drw-sec-head">
                  <h4 class="drw-sec-title">{{ t('casedrawer.reqConfig') }}</h4>
                  <div class="drw-tabs" role="tablist">
                    <button
                      v-for="tab in reqTabs"
                      :key="tab.key"
                      class="drw-tab"
                      :class="{ active: activeTab === tab.key }"
                      type="button"
                      role="tab"
                      @click="activeTab = tab.key"
                    >
                      {{ tab.label }}
                    </button>
                  </div>
                </div>

                <KeyValueTable
                  v-if="activeTab === 'params'"
                  v-model="paramRows"
                  :column-widths="['30%', '50%', '20%']"
                />
                <KeyValueTable
                  v-else-if="activeTab === 'path'"
                  v-model="pathVarRows"
                  :column-widths="['30%', '50%', '20%']"
                />
                <KeyValueTable
                  v-else-if="activeTab === 'headers'"
                  v-model="headerRows"
                  :column-widths="['30%', '50%', '20%']"
                />
                <div v-else class="drw-body-pane">
                  <div class="drw-body-type">
                    <span class="drw-label">{{ t('casedrawer.bodyType') }}</span>
                    <CustomSelect
                      :model-value="bodyType"
                      :options="bodyOptions"
                      pop-auto-width
                      @update:model-value="bodyType = String($event)"
                    />
                    <button
                      v-if="bodyEditable"
                      class="drw-fmt-btn"
                      type="button"
                      :title="t('casedrawer.formatJson')"
                      @click="formatBody"
                    >
                      {{ t('casedrawer.format') }}
                    </button>
                    <button
                      v-if="bodyEditable"
                      class="drw-icon-btn"
                      :class="{ active: bodyFindOpen }"
                      type="button"
                      :title="t('body.findHint')"
                      :aria-label="t('body.findHint')"
                      @click="toggleBodyFind"
                    >
                      <Icon name="search" :size="13" />
                    </button>
                    <span class="drw-body-hint">{{ bodyLabel }}</span>
                  </div>
                  <FindBar
                    v-if="bodyFindOpen && bodyEditable"
                    v-model:query="bodyQuery"
                    :index="bodyActiveMatch"
                    :total="bodyTotal"
                    :placeholder="t('body.findPh')"
                    @prev="prevBodyMatch"
                    @next="nextBodyMatch"
                    @close="closeBodyFind"
                  />
                  <div v-if="bodyEditable" class="drw-cm-wrap">
                    <JsonCodeMirror
                      ref="bodyEditorRef"
                      :model-value="bodyContent"
                      :placeholder-text='bodyType === "json" ? "{\"key\": \"value\"}" : t("casedrawer.bodyPh")'
                      :query="bodyFindOpen ? bodySearchQuery : ''"
                      :active-match="bodyActiveMatch"
                      @update:model-value="bodyContent = $event"
                    />
                  </div>
                  <p v-else class="drw-none">{{ t('casedrawer.noBodyContent') }}</p>
                  <p v-if="bodyTabHint" class="drw-body-warn">{{ bodyTabHint }}</p>
                </div>
              </section>

              <div
                class="drw-splitter"
                role="separator"
                aria-orientation="horizontal"
                :title="t('casedrawer.splitterHint')"
                @mousedown="onSplitterDown"
                @dblclick="onSplitterDblClick"
              ></div>

              <section class="drw-sec drw-resp-sec">
                <h4 class="drw-sec-title">{{ t('casedrawer.respTitle') }}</h4>

                <div v-if="running" class="drw-result drw-result-idle">
                  <span class="drw-spinner"></span>
                  <span class="drw-result-msg">{{ t('casedrawer.sending') }}</span>
                </div>
                <div v-else-if="runError" class="drw-result">
                  <div class="drw-result-bar">
                    <span class="drw-badge err">{{ t('casedrawer.failed') }}</span>
                    <span class="drw-result-msg">{{ t('casedrawer.runFailed', { v: runError }) }}</span>
                  </div>
                </div>
                <div v-else-if="result" class="drw-result drw-result-main">
                  <div class="drw-result-bar">
                    <span class="drw-badge" :class="`tone-${resultTone}`">
                      {{ result.status }} {{ statusTextOf(result.status) }}
                    </span>
                    <span class="drw-meta">{{ formatDuration(result.duration_ms) }}</span>
                    <span class="drw-meta">{{ sizeText(result.size_bytes) }}</span>
                    <span class="drw-copy">
                      <button
                        class="drw-copy-btn"
                        type="button"
                        :title="t('casedrawer.copyBody')"
                        @click="copyBody"
                      >
                        <Icon :name="copied ? 'check' : 'copy'" :size="12" />
                        {{ copied ? t('codegen.copied') : t('common.copy') }}
                      </button>
                    </span>
                  </div>
                  <div v-if="prettyBody" class="drw-cm-wrap">
                    <JsonCodeMirror
                      ref="respEditorRef"
                      :model-value="prettyBody"
                      readonly
                    />
                  </div>
                  <p v-else class="drw-none">{{ t('casedrawer.emptyBody') }}</p>
                </div>
                <div v-else class="drw-result drw-result-idle">
                  <span class="drw-result-msg">{{ t('casedrawer.noResult') }}</span>
                </div>
              </section>
            </div>
          </div>

          <footer class="drw-foot">
            <CodeExportMenu
              placement="top"
              align="left"
              :method="method"
              :url="exportUrl"
              :headers="headers"
              :body="exportBody"
              :auth="NO_AUTH"
              :disabled="!urlPath.trim()"
            />
            <button
              class="miyo-btn drw-run-btn"
              type="button"
              :disabled="running || !urlPath.trim()"
              @click="run"
            >
              <span v-if="running" class="drw-spinner"></span>
              <Icon v-else name="play" :size="13" /> {{ running ? t('cases.running') : t('casedrawer.runNow') }}
            </button>
            <span class="drw-spacer"></span>
            <button class="miyo-btn" type="button" :disabled="saving || running" @click="emit('update:open', false)">
              {{ t('common.cancel') }}
            </button>
            <button
              class="miyo-btn miyo-btn-primary"
              type="button"
              :disabled="saving || running || !name.trim() || !urlPath.trim()"
              @click="save"
            >
              <Icon name="save" :size="13" /> {{ t('casedrawer.save') }}
            </button>
          </footer>
        </aside>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.drw-mask {
  position: fixed;
  inset: 0;
  z-index: 120;
  background: var(--mask);
  backdrop-filter: blur(2px);
}

.drw {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 720px;
  max-width: 94vw;
  display: flex;
  flex-direction: column;
  background: var(--bg-2);
  border-left: 1px solid var(--border);
  box-shadow: var(--shadow-drawer);
}

.drw-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.drw-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}

.drw-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.drw-close:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.drw-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px 18px 20px;
}

/* 请求区 + 分割条 + 响应区（垂直拖拽分割） */
.drw-split {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.drw-splitter {
  flex: 0 0 6px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--text-3) 25%, transparent);
  cursor: row-resize;
  transition: background 0.15s ease;
  user-select: none;
}
.drw-splitter:hover,
.drw-splitter:active {
  background: color-mix(in srgb, var(--accent) 55%, transparent);
}

/* ---------- 分区卡片 ---------- */
.drw-sec {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-card, var(--bg-1));
}

/* 请求区：高度由分割比例控制，内容超限时区内滚动 */
.drw-req-sec {
  flex: 0 0 auto;
  min-height: 120px;
  overflow: auto;
}

.drw-sec-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

/* 顶部环境选择器（基本信息卡右上）：外观与几何由 EnvironmentSelector（solo）自持，
   此处只管占位——不随环境名长短抖动，过长名称由组件内部截断。 */
.drw-env-select {
  flex-shrink: 0;
  max-width: 220px;
}

.drw-sec-title {
  margin: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.5px;
  text-transform: uppercase;
  color: var(--text-3);
}

/* ---------- 基本信息 ---------- */
.drw-grid3 {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: 12px;
}

.drw-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.drw-label {
  font-size: 12px;
  color: var(--text-2);
}

.drw-input {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  font-family: inherit;
  font-size: 13px;
  color: var(--text-1);
  background: var(--bg-1);
  outline: none;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.drw-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint, rgba(168, 85, 247, 0.18));
}

.drw-mono {
  font-family: var(--font-mono);
  font-size: 12px;
}

/* ---------- Method + Path 组合 ---------- */
.drw-url-group {
  display: flex;
  align-items: stretch;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-1);
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.drw-url-group:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint, rgba(168, 85, 247, 0.18));
}

.drw-method {
  flex-shrink: 0;
  border: none;
  background: transparent;
  border-radius: 0;
}

.drw-method-badge {
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 8px;
  border-radius: 5px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.3px;
  background: color-mix(in srgb, currentColor 14%, transparent);
}

.drw-path-input {
  flex: 1;
  min-width: 0;
  border: none;
  border-left: 1px solid var(--border);
  border-radius: 0;
  background: transparent;
}
.drw-path-input:focus {
  box-shadow: none;
}

/* ---------- 请求参数 Tab ---------- */
.drw-tabs {
  display: flex;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-2);
}

.drw-tab {
  padding: 4px 12px;
  border: none;
  border-radius: 5px;
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-3);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.drw-tab:hover {
  color: var(--text-1);
}
.drw-tab.active {
  color: var(--accent);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-sm);
}

.drw-body-pane {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.drw-body-type {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}

.drw-body-hint {
  font-size: 12px;
  color: var(--text-3);
}

.drw-cm-wrap {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  background: var(--bg-2);
}

.drw-body-warn {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--warning);
}

.drw-fmt-btn {
  margin-left: auto;
  padding: 4px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.drw-fmt-btn:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.drw-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: none;
  color: var(--text-2);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.drw-icon-btn:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.drw-icon-btn.active {
  color: var(--accent);
  border-color: var(--accent);
}

.drw-none {
  margin: 0;
  padding: 8px 2px;
  font-size: 12.5px;
  color: var(--text-3);
}

/* ---------- 响应区 ---------- */
.drw-resp-sec {
  flex: 1 1 0;
  min-height: 120px;
  overflow: hidden;
  gap: 10px;
}

.drw-result {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 有结果时：撑满响应区，CodeMirror 只读预览随高度自适应 */
.drw-result-main {
  flex: 1;
  min-height: 0;
}

.drw-result-idle {
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding: 6px 2px;
  min-height: 0;
}

.drw-result-bar {
  display: flex;
  align-items: center;
  gap: 10px;
}

.drw-badge {
  flex-shrink: 0;
  padding: 2px 10px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 600;
  font-family: var(--font-mono);
  border: 1px solid transparent;
}
.drw-badge.tone-ok {
  color: var(--ok);
  background: color-mix(in srgb, var(--ok) 10%, transparent);
  border-color: color-mix(in srgb, var(--ok) 22%, transparent);
}
.drw-badge.tone-warn {
  color: var(--warning);
  background: color-mix(in srgb, var(--warning) 10%, transparent);
  border-color: color-mix(in srgb, var(--warning) 22%, transparent);
}
.drw-badge.tone-err {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  border-color: color-mix(in srgb, var(--danger) 22%, transparent);
}
.drw-badge.tone-info {
  color: var(--info);
  background: color-mix(in srgb, var(--info) 10%, transparent);
  border-color: color-mix(in srgb, var(--info) 22%, transparent);
}

.drw-meta {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.drw-copy {
  margin-left: auto;
}

.drw-copy-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.drw-copy-btn:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.drw-result-msg {
  font-size: 12px;
  color: var(--text-3);
}

.drw-spinner {
  width: 13px;
  height: 13px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: drw-spin 0.7s linear infinite;
}

@keyframes drw-spin {
  to {
    transform: rotate(360deg);
  }
}

/* ---------- Sticky Footer ---------- */
.drw-foot {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--border);
  background: color-mix(in srgb, var(--bg-2) 90%, transparent);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
  z-index: 2;
}

.drw-run-btn {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 55%, transparent);
}
.drw-run-btn:hover:not(:disabled) {
  background: var(--accent-tint, rgba(168, 85, 247, 0.14));
}

.drw-spacer {
  flex: 1;
}

.drw-mask-enter-active,
.drw-mask-leave-active {
  transition: opacity 0.18s var(--ease);
}
.drw-mask-enter-from,
.drw-mask-leave-to {
  opacity: 0;
}
.drw-mask-enter-active .drw,
.drw-mask-leave-active .drw {
  transition: transform 0.22s var(--ease);
}
.drw-mask-enter-from .drw,
.drw-mask-leave-to .drw {
  transform: translateX(100%);
}
</style>