<script setup lang="ts">
/**
 * GraphQLView：GraphQL 接口编辑与调试视图。
 *
 * - 左：Query 编辑器（语法高亮，覆盖层方案：透明 textarea + 高亮 pre）
 * - 右：Variables JSON 编辑器 + operationName
 * - 顶栏：保存 / 发送 / 历史 / 生成代码
 * - 发送走后端 execute_request（POST + application/json），
 *   响应区分 data 与 errors（GraphQL 语义，errors 优先渲染）
 * - 生成代码在浏览器端完成，算法与 miyo-codegen 的 render_graphql_curl /
 *   render_graphql_js 保持一致
 *
 * 样式沿用项目 miyo- 设计系统（深色 slate 主题，变量名与
 * crates/miyo-desktop/src/styles.rs 的 DESIGN_SYSTEM_CSS 对齐）。
 */
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore, FALLBACK_GLOBAL_TIMEOUT_MS } from '../stores/workspace'
import { formatDateTime } from '../utils/dateTime'
import {
  envBaseUrl,
  mergedVariableMap,
  resolveRequestUrl,
} from '../utils/environment'
import { escapeHtml, highlightGraphQL, highlightJSON } from '../utils/highlight'
import { handleTextareaTab } from '../utils/textareaIndent'
import Modal from '../components/ui/Modal.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import type { BodySpec, ExecuteRequestArgs, ExecuteResponse, GraphQLSpec } from '../types/miyoApi'

const router = useRouter()
const route = useRoute()

const props = withDefaults(
  defineProps<{
    url?: string
    environmentId?: string | null
    /** 当前接口的 BodySpec（mode !== 'graphql' 时使用默认空模板） */
    body?: BodySpec
  }>(),
  { url: '', environmentId: null, body: undefined },
)

const emit = defineEmits<{
  (e: 'save', body: BodySpec): void
}>()

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

const api = useMiyoApi()
const toast = useToast()

// ---------- 编辑器状态 ----------
function initSpec(): GraphQLSpec {
  if (props.body?.mode === 'graphql') {
    return { ...props.body.spec }
  }
  return { query: '', variables: '{}', operation_name: '' }
}

const gql = ref<GraphQLSpec>(initSpec())
// ---------- 初始地址（/graphql 可脱离工作台单独进入，必须自带兜底） ----------
/** 上次成功发送的地址（跨会话记忆）。 */
const URL_STORAGE_KEY = 'miyo_graphql_last_url'

const activeEnv = computed(() => store.environments.find((e) => e.id === store.activeEnvId) ?? null)

/** 环境 + 项目 + 全局变量合并表（单源 mergedVariableMap，与 EndpointEditor 同源）。 */
const envVars = computed(() =>
  mergedVariableMap({
    globalVariables: store.globalVariables,
    projectVariables: store.project?.variables,
    env: activeEnv.value,
  }),
)

/** 路径 → 绝对地址（镜像 EndpointEditor.buildUrl：绝对地址直用，否则拼基址并解析变量）。 */
function absoluteUrl(path: string): string {
  const p = (path ?? '').trim()
  if (!p) return ''
  if (p.startsWith('http://') || p.startsWith('https://')) return p
  if (activeEnv.value && envBaseUrl(activeEnv.value)) {
    return resolveRequestUrl(activeEnv.value, p, envVars.value).url
  }
  const rel = p.startsWith('/') ? p : `/${p}`
  return `${store.urlDomain}${rel}`
}

/** 路由 query 取字符串（同名多值取首个）。 */
function queryValue(value: unknown): string {
  if (Array.isArray(value)) return typeof value[0] === 'string' ? value[0].trim() : ''
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * 初始地址优先级：props（显式传入）→ route.query.url → route.query.endpointId →
 * localStorage（上次发送）→ 当前激活端点 → `${urlDomain}/graphql` 兜底。
 * WorkspaceView 不改（侧栏不带参进入），因此上下文全在这里自行恢复。
 */
function resolveInitialUrl(): string {
  const fromProps = queryValue(props.url)
  if (fromProps) return fromProps
  const fromQuery = queryValue(route.query.url)
  if (fromQuery) return fromQuery
  const endpointId = queryValue(route.query.endpointId)
  if (endpointId) {
    const ep = store.endpoints.find((e) => e.id === endpointId)
    const epUrl = ep ? absoluteUrl(ep.path) : ''
    if (epUrl) return epUrl
  }
  try {
    const cached = localStorage.getItem(URL_STORAGE_KEY)
    if (cached && cached.trim()) return cached.trim()
  } catch {
    // localStorage 不可用（隐私模式）：继续向下兜底
  }
  const activeUrl = absoluteUrl(store.activeEndpoint?.path ?? '')
  if (activeUrl) return activeUrl
  const domain = store.urlDomain.replace(/\/+$/, '')
  return domain ? `${domain}/graphql` : ''
}

/** 成功发送后记忆地址（下次无上下文进入时直接可用）。 */
function rememberUrl(): void {
  const v = gqlUrl.value.trim()
  if (!v) return
  try {
    localStorage.setItem(URL_STORAGE_KEY, v)
  } catch {
    // 忽略写入失败
  }
}

/** 地址栏当前值（props 已占用 `url` 键，setup 同名会触发 vue/no-dupe-keys，故命名 gqlUrl）。 */
const gqlUrl = ref(resolveInitialUrl())

/** 项目数据异步到位（端点列表 / 域名刚加载）且地址仍为空时再解析一次。 */
watch(
  () => [store.endpoints.length, store.urlDomain, store.activeEndpoint?.id] as const,
  () => {
    if (!gqlUrl.value.trim()) gqlUrl.value = resolveInitialUrl()
  },
)

/** 携带新上下文进入（/graphql?url=… &endpointId=…）时刷新地址。 */
watch(
  () => [route.query.url, route.query.endpointId] as const,
  () => {
    const next = resolveInitialUrl()
    if (next && next !== gqlUrl.value) gqlUrl.value = next
  },
)
const sending = ref(false)
const queryEditor = ref<HTMLElement | null>(null)
const varsEditor = ref<HTMLElement | null>(null)
const saving = ref(false)
const statusText = ref('')
const response = ref<{ status: number; body: string; durationMs: number } | null>(null)
const responseErrors = ref<{ message: string; locations?: string; path?: string }[]>([])
const responseRaw = ref(false)
const sendFailed = ref(false)

// ---------- 变量 JSON 校验 ----------
function parseVariables(text: string): unknown | null {
  const trimmed = text.trim()
  if (!trimmed || trimmed === '{}') return {}
  try {
    const value = JSON.parse(trimmed)
    if (value !== null && typeof value === 'object' && !Array.isArray(value)) return value
    return null
  } catch {
    return null
  }
}

const variablesValid = computed(() => {
  const t = gql.value.variables.trim()
  if (!t || t === '{}') return true
  try {
    const v = JSON.parse(t)
    return v !== null && typeof v === 'object' && !Array.isArray(v)
  } catch {
    return false
  }
})

// ---------- 高亮（实现见 utils/highlight.ts） ----------
// 防抖：原来每键全量 highlightGraphQL/highlightJSON（正则扫描全文），
// 此处输入即时响应（底层 textarea 非受控），着色延迟 150ms 跟上。
const shownQuery = ref(gql.value.query)
const shownVariables = ref(gql.value.variables)
let gqlHlTimer: ReturnType<typeof setTimeout> | undefined
watch(
  () => [gql.value.query, gql.value.variables],
  ([q, v]) => {
    if (gqlHlTimer) clearTimeout(gqlHlTimer)
    gqlHlTimer = setTimeout(() => {
      shownQuery.value = q
      shownVariables.value = v
    }, 150)
  },
)
onUnmounted(() => {
  if (gqlHlTimer) clearTimeout(gqlHlTimer)
})
const queryHtml = computed(() => highlightGraphQL(shownQuery.value))
const variablesHtml = computed(() => {
  const t = shownVariables.value
  if (variablesValid.value) return highlightJSON(t)
  return escapeHtml(t)
})

// 大响应保护（与 ResponsePanel 的 PARSE_LIMIT_BYTES 对齐）：
// parse + stringify(pretty) + 正则高亮会把体积放大数十倍并生成海量 <span>，
// 超过阈值跳过格式化，仅转义显示前 200KB 纯文本；完整内容可切换「原始」查看。
const PARSE_LIMIT_BYTES = 1_000_000
const PREVIEW_LIMIT_BYTES = 200_000

const responseTooLarge = computed(() => (response.value?.body.length ?? 0) > PARSE_LIMIT_BYTES)

const responseHtml = computed(() => {
  if (!response.value) return ''
  const text = response.value.body
  if (text.length > PARSE_LIMIT_BYTES) return escapeHtml(text.slice(0, PREVIEW_LIMIT_BYTES))
  try {
    const parsed = JSON.parse(text)
    return highlightJSON(JSON.stringify(parsed, null, 2))
  } catch {
    return escapeHtml(text)
  }
})

const statusClass = computed(() => {
  if (!response.value) return ''
  if (response.value.status >= 400 || responseErrors.value.length) return 'status-err'
  return 'status-ok'
})

// ---------- 编辑器滚动同步 ----------
function syncScroll(e: Event, pre: HTMLElement | null) {
  const ta = e.target as HTMLElement
  if (!pre) return
  pre.scrollTop = ta.scrollTop
  pre.scrollLeft = ta.scrollLeft
}

/** Query / Variables 编辑器 Tab 缩进而非跳出焦点。 */
function onCodeKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab') handleTextareaTab(e.target as HTMLTextAreaElement, e)
}

// ---------- 构建请求 ----------
function buildArgs(): ExecuteRequestArgs {
  return {
    url: gqlUrl.value.trim(),
    method: 'POST',
    spec: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'graphql', spec: { ...gql.value } },
      timeout_ms: null,
      follow_redirects: true,
      tests: null,
    },
    environment_id: props.environmentId ?? store.activeEnvId ?? null,
  }
}

// ---------- 发送 ----------
// request_id + 前端超时兜底：原先裸调 executeRequest，慢请求既不能取消
// 也没有本地超时，弱网下只能干等后端 300s 兜底。
let activeRequestId: string | null = null
let timeoutTimer: ReturnType<typeof setTimeout> | undefined

/** 取消在途请求（发送按钮变成「停止」）。 */
async function cancelSend(): Promise<void> {
  if (!activeRequestId) return
  try {
    await api.cancelRequest(activeRequestId)
  } catch {
    // 已结束的请求取消会失败，忽略
  }
}

async function send() {
  if (sending.value) return
  if (!gqlUrl.value.trim()) {
    statusText.value = t('graphql.urlRequired')
    toast.warning(t('graphql.urlRequired'))
    return
  }
  if (!gql.value.query.trim()) {
    statusText.value = t('graphql.queryRequired')
    toast.warning(t('graphql.queryRequired'))
    return
  }
  if (!variablesValid.value) {
    statusText.value = t('graphql.varsNotObject')
    toast.warning(t('graphql.varsNotObject'))
    return
  }
  statusText.value = ''
  sending.value = true
  sendFailed.value = false
  responseErrors.value = []
  const rid = crypto.randomUUID()
  activeRequestId = rid
  // 前端超时兜底：全局设置 → 内置 300s（与 store.send 同口径；GraphQL 规格无接口级超时）。
  const ms = store.globalTimeoutMs ?? FALLBACK_GLOBAL_TIMEOUT_MS
  if (ms && ms > 0) {
    timeoutTimer = setTimeout(() => void cancelSend(), ms)
  }
  try {
    const res: ExecuteResponse = await api.executeRequest({ ...buildArgs(), request_id: rid })
    response.value = {
      status: res.status,
      body: res.body,
      durationMs: res.duration_ms,
    }
    // GraphQL 语义：errors 字段存在即业务失败，优先展示
    responseErrors.value = parseResponseErrors(res.body)
    pushHistory()
    rememberUrl()
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    statusText.value = message
    responseRaw.value = true
    sendFailed.value = true
    toast.error(t('graphql.sendFail'), { message, duration: 6000 })
  } finally {
    if (timeoutTimer) clearTimeout(timeoutTimer)
    timeoutTimer = undefined
    activeRequestId = null
    sending.value = false
  }
}

/** 发送失败后的重试：上次请求参数原样重发。 */
async function retrySend() {
  await send()
}

function parseResponseErrors(body: string): { message: string; locations?: string; path?: string }[] {
  // 大响应保护（与 responseHtml 同口径）：超阈值跳过全量 JSON.parse，避免主线程冻结。
  if (body.length > PARSE_LIMIT_BYTES) return []
  try {
    const parsed = JSON.parse(body)
    if (parsed && typeof parsed === 'object' && Array.isArray(parsed.errors)) {
      return parsed.errors.map((entry: Record<string, unknown>) => ({
        message: String(entry.message ?? t('graphql.unknownError')),
        locations: Array.isArray(entry.locations)
          ? entry.locations
              .map((loc: Record<string, unknown>) => `${loc.line}:${loc.column}`)
              .join(', ')
          : undefined,
        path: Array.isArray(entry.path) ? entry.path.join('.') : undefined,
      }))
    }
  } catch {
    // 非 JSON 响应交给原始响应区展示
  }
  return []
}

// ---------- 保存 ----------
function save() {
  if (!variablesValid.value) {
    statusText.value = t('graphql.varsNotObject')
    return
  }
  saving.value = true
  try {
    emit('save', { mode: 'graphql', spec: { ...gql.value } })
    pushHistory()
    statusText.value = t('graphql.saved')
    toast.success(t('graphql.savedToast'))
  } finally {
    saving.value = false
  }
}

// ---------- 历史（localStorage） ----------
interface HistoryEntry {
  query: string
  variables: string
  operation_name: string
  when: string
}

const HISTORY_KEY = 'miyo_graphql_history'
const MAX_HISTORY = 20

function loadHistory(): HistoryEntry[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY)
    return raw ? (JSON.parse(raw) as HistoryEntry[]) : []
  } catch {
    return []
  }
}

function pushHistory() {
  const entries = loadHistory()
  const entry: HistoryEntry = {
    query: gql.value.query,
    variables: gql.value.variables,
    operation_name: gql.value.operation_name,
    when: new Date().toISOString(),
  }
  entries.unshift(entry)
  const deduped = entries.filter(
    (e, i) => i === 0 || e.query !== entries[i - 1].query || e.variables !== entries[i - 1].variables,
  )
  localStorage.setItem(HISTORY_KEY, JSON.stringify(deduped.slice(0, MAX_HISTORY)))
}

const historyOpen = ref(false)
const history = ref<HistoryEntry[]>([])

function openHistory() {
  history.value = loadHistory()
  historyOpen.value = true
}

function applyHistory(entry: HistoryEntry) {
  gql.value = { ...entry }
  historyOpen.value = false
}

function clearHistory() {
  localStorage.removeItem(HISTORY_KEY)
  history.value = []
}

// ---------- 生成代码（与 miyo-codegen 保持一致） ----------
const codegenOpen = ref(false)
const codegenTab = ref<'curl' | 'apollo'>('curl')
const copied = ref(false)

function sq(s: string): string {
  return s.replace(/'/g, "'\\''")
}

function dq(s: string): string {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t')
}

function gqlJsonBody(): string {
  const payload: Record<string, unknown> = { query: gql.value.query }
  const vars = parseVariables(gql.value.variables)
  if (vars && Object.keys(vars as object).length > 0) payload.variables = vars
  if (gql.value.operation_name.trim()) payload.operationName = gql.value.operation_name.trim()
  return JSON.stringify(payload)
}

const curlCode = computed(() => {
  let out = `curl -X POST '${sq(gqlUrl.value.trim())}'`
  out += ` \\\n     -H 'Content-Type: application/json'`
  out += ` \\\n     --data '${sq(gqlJsonBody())}'`
  return out
})

const apolloCode = computed(() => {
  let out = `import { ApolloClient, InMemoryCache, gql } from '@apollo/client';\n\n`
  out += `const client = new ApolloClient({\n`
  out += `  uri: '${sq(gqlUrl.value.trim())}',\n`
  out += `  cache: new InMemoryCache(),\n`
  out += `});\n\n`
  out += `const QUERY = gql\`\n${gql.value.query}\n\`;\n\n`
  const vars = parseVariables(gql.value.variables) ?? {}
  out += `const variables = ${JSON.stringify(vars)};\n\n`
  const op = gql.value.operation_name.trim()
  if (op) {
    out += `const result = await client.query({\n  query: QUERY,\n  variables,\n  operationName: '${dq(op)}',\n});\n`
  } else {
    out += `const result = await client.query({\n  query: QUERY,\n  variables,\n});\n`
  }
  out += `console.log(result.data);\n`
  return out
})

function openCodegen() {
  codegenOpen.value = true
  copied.value = false
  codegenTab.value = 'curl'
}

async function copyCode() {
  const text = codegenTab.value === 'curl' ? curlCode.value : apolloCode.value
  try {
    await navigator.clipboard.writeText(text)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    statusText.value = t('graphql.copyFail')
  }
}
</script>

<template>
  <div class="gql-root">
    <div class="row miyo-mb-2">
      <button class="miyo-btn miyo-btn-sm" type="button" @click="router.push('/workspace')">← {{ t('graphql.backToWorkspace') }}</button>
      <input v-model="gqlUrl" class="miyo-input gql-url" :placeholder="t('graphql.urlPh')" spellcheck="false" />
      <button class="miyo-btn miyo-btn-sm" :disabled="saving" @click="save">{{ t('common.save') }}</button>
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-primary"
        type="button"
        @click="sending ? cancelSend() : send()"
      >
        {{ sending ? t('scenario.stop') : t('editor.send') }}
      </button>
      <button
        v-if="sendFailed"
        class="miyo-btn miyo-btn-sm miyo-btn-ghost"
        :disabled="sending"
        @click="retrySend"
      >
        {{ sending ? t('common.retrying') : t('common.retry') }}
      </button>
      <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" @click="openHistory">{{ t('graphql.historyBtn') }}</button>
      <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" @click="openCodegen">{{ t('graphql.codegen') }}</button>
    </div>

    <div class="gql-grid">
      <div class="gql-pane">
        <div class="pane-title">
          <span>{{ t('graphql.queryTitle') }}</span>
          <span class="hint-inline">{{ t('graphql.queryHint') }}</span>
        </div>
        <div class="hl-wrap">
          <pre class="hl-pre" aria-hidden="true" v-html="queryHtml"></pre>
          <textarea
            ref="queryEditor"
            class="hl-ta"
            :value="gql.query"
            spellcheck="false"
            placeholder="query Hero($id: ID!) {&#10;  hero(id: $id) { name }&#10;}"
            @input="gql.query = ($event.target as HTMLTextAreaElement).value"
            @scroll="syncScroll($event, (queryEditor as HTMLElement | null))"
            @keydown="onCodeKeydown"
          ></textarea>
        </div>
      </div>

      <div class="gql-pane">
        <div class="pane-title">
          <span>{{ t('graphql.varsTitle') }}</span>
          <span class="hint-inline" :class="{ 'vars-invalid': !variablesValid }">
            {{ variablesValid ? t('graphql.varsValid') : t('graphql.varsInvalid') }}
          </span>
        </div>
        <div class="hl-wrap">
          <pre class="hl-pre" aria-hidden="true" v-html="variablesHtml"></pre>
          <textarea
            ref="varsEditor"
            class="hl-ta"
            :value="gql.variables"
            spellcheck="false"
            placeholder='{"id": "42"}'
            @input="gql.variables = ($event.target as HTMLTextAreaElement).value"
            @scroll="syncScroll($event, (varsEditor as HTMLElement | null))"
            @keydown="onCodeKeydown"
          ></textarea>
        </div>
        <label class="op-name-label">
          operationName
          <input
            v-model="gql.operation_name"
            class="miyo-input miyo-input-sm"
            :placeholder="t('graphql.opNamePh')"
            spellcheck="false"
          />
        </label>
      </div>
    </div>

    <div class="resp-head">
      <div class="hint-inline" :class="statusClass">
        {{ statusText || (response ? `${response.status} · ${response.durationMs}ms` : '') }}
      </div>
      <button v-if="response" class="miyo-btn miyo-btn-sm miyo-btn-ghost" @click="responseRaw = !responseRaw">
        {{ responseRaw ? t('response.pretty') : t('response.raw') }}
      </button>
    </div>

    <div v-if="responseErrors.length" class="resp-errors">
      <div v-for="(err, i) in responseErrors" :key="i" class="resp-error">
        <span class="err-msg">{{ err.message }}</span>
        <span v-if="err.locations" class="hint-inline">{{ t('graphql.errLocations', { v: err.locations }) }}</span>
        <span v-if="err.path" class="hint-inline">{{ t('graphql.errPath', { v: err.path }) }}</span>
      </div>
    </div>

    <div v-if="response" class="resp-body">
      <p v-if="responseTooLarge && !responseRaw" class="resp-too-large">
        {{ t('graphql.respTooLarge') }}
      </p>
      <pre v-if="!responseRaw" class="resp-pre" v-html="responseHtml"></pre>
      <pre v-else class="resp-pre">{{ response.body }}</pre>
    </div>
    <div v-else class="resp-empty">
      <p class="hint">{{ t('graphql.respEmpty') }}</p>
    </div>

    <!-- 历史 -->
    <Modal v-model:open="historyOpen" :title="t('graphql.historyTitle')" width="560px">
      <div v-if="!history.length"><EmptyState icon="history" :title="t('graphql.historyEmpty')" compact /></div>
      <ul v-else class="history-list">
        <li v-for="(entry, i) in history" :key="i" class="history-item" @click="applyHistory(entry)">
          <pre class="history-query">{{ entry.query }}</pre>
          <span class="hint-inline">{{ formatDateTime(entry.when) }}</span>
        </li>
      </ul>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger-solid" @click="clearHistory">{{ t('graphql.clearHistory') }}</button>
      </template>
    </Modal>

    <!-- 生成代码 -->
    <Modal v-model:open="codegenOpen" :title="t('graphql.codegen')" width="640px">
      <div class="miyo-tabs">
        <button class="miyo-tab" :class="{ active: codegenTab === 'curl' }" @click="codegenTab = 'curl'">curl</button>
        <button class="miyo-tab" :class="{ active: codegenTab === 'apollo' }" @click="codegenTab = 'apollo'">JavaScript (Apollo Client)</button>
      </div>
      <pre class="codegen-out">{{ codegenTab === 'curl' ? curlCode : apolloCode }}</pre>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" @click="copyCode">{{ copied ? t('codegen.copied') : t('common.copy') }}</button>
      </template>
    </Modal>
  </div>
</template>

<style scoped>
/* 私有色板映射到全局 miyo-* 令牌，随主题联动 */
.gql-root {
  --bg: var(--miyo-bg);
  --panel: var(--miyo-bg-panel);
  --panel-2: var(--miyo-bg-panel-2);
  --border: var(--miyo-border);
  --border-2: var(--miyo-text-muted);
  --text: var(--miyo-text);
  --text-2: var(--miyo-text-secondary);
  --muted: var(--miyo-text-muted);
  --accent: var(--miyo-info);
  --accent-2: var(--miyo-accent-weak);
  --accent-soft: var(--miyo-info-tint);
  --accent-line: color-mix(in srgb, var(--info) 45%, transparent);
  --success: var(--miyo-success);
  --warning: var(--miyo-warning);
  --danger: var(--miyo-danger);
  --danger-weak: var(--miyo-danger-tint);
  --mono: var(--font-mono);
  --r-s: var(--miyo-radius-sm);
  --s-2: 8px;
  --s-3: 12px;
  --s-4: 16px;
  color: var(--text);
  background: var(--bg);
  font-size: var(--fs-md);
  padding: 48px 12px 12px;
}

.row {
  display: flex;
  align-items: center;
  gap: var(--s-2);
}

.miyo-mb-2 {
  margin-bottom: var(--s-2);
}

.miyo-input-sm {
  flex: 1;
  min-width: 0;
  padding: 4px 8px;
}

.miyo-btn-ghost {
  background: transparent;
  border-color: var(--border);
  color: var(--text-2);
}

.miyo-btn-ghost:hover:not(:disabled) {
  background: var(--panel-2);
  color: var(--text);
  border-color: var(--border-2);
}

.miyo-tabs {
  display: flex;
  gap: 4px;
  margin-bottom: var(--s-3);
  border-bottom: 1px solid var(--border);
}

.miyo-tab {
  padding: 6px 12px;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-2);
  cursor: pointer;
  font-size: 13px;
}

.miyo-tab:hover {
  color: var(--text);
}

.miyo-tab.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

/* ---------- 布局 ---------- */
.gql-url {
  flex: 1;
  min-width: 0;
}

.gql-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--s-3);
}

/* ---- 窄窗回退（<1080px）：查询 / 变量两栏改单栏上下堆叠，顶部工具条允许换行、
   URL 输入给足回绕宽度，避免 1080px 以下两栏各挤半屏 / 按钮横向溢出（≥1080px 不变） ---- */
@media (max-width: 1080px) {
  .gql-grid {
    grid-template-columns: 1fr;
  }
  .row {
    flex-wrap: wrap;
    row-gap: var(--s-2);
  }
  .gql-url {
    flex: 1 1 260px;
  }
}

.gql-pane {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--s-2);
}

.pane-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-2);
  font-weight: 600;
}

.hint-inline {
  color: var(--muted);
  font-size: 12px;
}

.vars-invalid {
  color: var(--danger);
}

/* ---------- 高亮编辑器（透明 textarea + pre 覆盖层） ---------- */
.hl-wrap {
  position: relative;
  background: var(--miyo-input-bg);
  border: 1px solid var(--border);
  border-radius: var(--r-s);
  height: 300px;
  overflow: hidden;
}

.hl-wrap:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.hl-pre,
.hl-ta {
  margin: 0;
  padding: 10px 12px;
  font-family: var(--mono);
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre;
  tab-size: 2;
  overflow: hidden;
}

.hl-pre {
  position: absolute;
  inset: 0;
  color: var(--text);
  pointer-events: none;
  overflow: auto;
}

.hl-ta {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: transparent;
  caret-color: var(--accent);
  overflow: auto;
}

.hl-ta::selection {
  background: var(--accent-soft);
}

:deep(.hl-c) {
  color: var(--muted);
  font-style: italic;
}

:deep(.hl-s) {
  color: var(--success);
}

:deep(.hl-k) {
  color: var(--accent);
  font-weight: 600;
}

:deep(.hl-v) {
  color: var(--warning);
}

:deep(.hl-n) {
  color: #c084fc;
}

:deep(.hl-p) {
  color: var(--text-2);
}

:deep(.hl-b) {
  color: var(--danger);
}

:deep(.hl-null) {
  color: var(--text-3);
}

.op-name-label {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  font-size: 12px;
  color: var(--text-2);
}

/* ---------- 响应区 ---------- */
.resp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: var(--s-4);
}

.status-ok {
  color: var(--success);
}

.status-err {
  color: var(--danger);
  font-weight: 700;
}

.resp-errors {
  margin-top: var(--s-2);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.resp-error {
  padding: 8px 12px;
  border: 1px solid var(--danger-weak);
  background: var(--danger-weak);
  border-radius: var(--r-s);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.err-msg {
  color: var(--danger);
  font-weight: 600;
  font-family: var(--mono);
  font-size: 12.5px;
}

.resp-body {
  margin-top: var(--s-2);
  background: var(--miyo-input-bg);
  border: 1px solid var(--border);
  border-radius: var(--r-s);
  max-height: 320px;
  overflow: auto;
}

.resp-too-large {
  margin: 0;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--accent-soft);
  color: var(--text-2);
  font-size: 12px;
}

.resp-pre {
  margin: 0;
  padding: 10px 12px;
  font-family: var(--mono);
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}

.resp-empty {
  margin-top: var(--s-4);
  text-align: center;
}

.hint {
  color: var(--muted);
  font-size: 12px;
  text-align: center;
  margin-top: var(--s-2);
}

/* ---------- 历史 ---------- */

.history-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.history-item {
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: var(--r-s);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--s-3);
}

.history-item:hover {
  border-color: var(--accent-line);
  background: var(--accent-soft);
}

.history-query {
  margin: 0;
  flex: 1;
  min-width: 0;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 64px;
  overflow: auto;
}

/* ---------- 生成代码 ---------- */
.codegen-out {
  margin: 0;
  padding: 12px;
  background: var(--miyo-input-bg);
  border: 1px solid var(--border);
  border-radius: var(--r-s);
  font-family: var(--mono);
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre;
  overflow: auto;
  max-height: 400px;
}
</style>
