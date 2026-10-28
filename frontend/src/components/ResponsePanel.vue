<script setup lang="ts">
/**
 * ResponsePanel：响应面板。
 * - 顶栏：高对比状态栏（2xx 实心绿 / 3xx 琥珀 / 4xx-5xx 红，`201 Created`）
 *   + 耗时 / 大小 / 类型指标（同组 gap 分组，数值统一等宽字体）+ 截断徽标；
 * - 工具栏：Body/Headers/Cookies 标签 + 格式化/原始/预览 分段切换（右）
 *   + 查找（⌘F）/ 展开-收起全部 / 保存为示例 / 复制响应（最右）；
 * - 查找：顶部弹出搜索框，高亮匹配 + 上一个/下一个导航（Enter / Shift+Enter / Esc）；
 * - 主体：JSON → 可折叠树形查看器（行号 + VS Code 深色语法着色）；文本 → 行号代码视图；HTML → 沙箱预览。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { copyText } from '../utils/clipboard'
import { escapeHtml, highlightJSONText } from '../utils/highlight'
import { EDITOR_INDENT } from '../constants/editorTheme'
import { formatBytes, formatDuration } from '../utils/format'
import FindBar from './ui/FindBar.vue'
import Icon from './ui/Icon.vue'
import JsonTree from './JsonTree.vue'
import MethodText from './ui/MethodText.vue'
import SegmentedControl, { type SegmentOption } from './ui/SegmentedControl.vue'
import Tabs, { type TabItem } from './ui/Tabs.vue'
import Tooltip from './ui/Tooltip.vue'
import type { ExecuteResponse, RequestSnapshot } from '../types/miyoApi'

const props = defineProps<{
  response: ExecuteResponse
  /** 正文被 IPC 预览截断（`body_omitted`）时的全文加载器：由宿主注入，
   * 不存在时复制退化为预览（测试/无 request_id 场景）。 */
  loadFullBody?: () => Promise<string | null>
  /** 该接口已保存的响应示例数量。>0 时才注册「示例」页签——
   * 由宿主持有数据，本组件只负责展示位置（内容走 #examples 插槽）。 */
  examplesCount?: number
}>()

const emit = defineEmits<{ saveExample: []; dragStart: [event: MouseEvent] }>()

const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

// ---------- 状态 ----------
const activeTab = ref<'request' | 'body' | 'headers' | 'cookies' | 'scripts' | 'examples'>('body')
type ViewMode = 'pretty' | 'raw' | 'preview'
const viewMode = ref<ViewMode>('pretty')

const REASON_PHRASES: Record<number, string> = {
  100: 'Continue',
  101: 'Switching Protocols',
  200: 'OK',
  201: 'Created',
  202: 'Accepted',
  204: 'No Content',
  301: 'Moved Permanently',
  302: 'Found',
  304: 'Not Modified',
  400: 'Bad Request',
  401: 'Unauthorized',
  403: 'Forbidden',
  404: 'Not Found',
  405: 'Method Not Allowed',
  409: 'Conflict',
  410: 'Gone',
  422: 'Unprocessable Entity',
  429: 'Too Many Requests',
  500: 'Internal Server Error',
  502: 'Bad Gateway',
  503: 'Service Unavailable',
  504: 'Gateway Timeout',
}

const tone = computed(() => {
  const s = props.response.status
  if (s < 300) return 'ok'
  if (s < 400) return 'warn'
  return 'err'
})

const statusText = computed(() => {
  const s = props.response.status
  return `${s} ${REASON_PHRASES[s] ?? (s < 400 ? 'OK' : 'Error')}`
})

const sizeText = computed(() => formatBytes(props.response.size_bytes))

const headerRows = computed(() => props.response.headers.map(([k, v]) => ({ k, v })))

// ---------- 正文解析 ----------
// 大响应保护：超过阈值跳过 JSON 解析与树形渲染（全量 parse/渲染会冻结 UI），
// 回退为按行文本视图；行渲染按块渐进加载（每次追加 LINE_CHUNK 行）。
const PARSE_LIMIT_BYTES = 1_000_000
const LINE_CHUNK = 1000
const visibleLines = ref(LINE_CHUNK)
/** 单行渲染上限：压缩 JSON/HTML 常见「整页一行」，后端正文可达 20MB，
 * 单行全量驻留 + 高亮 + 行 key 哈希会冻结主线程，超限截断展示。 */
const MAX_RENDER_LINE = 100_000
/** 超长行跳过语法高亮与查找标记（jsonTokens 正则 / toLowerCase 都是大行上的 O(n) 主线程开销）。 */
const LINE_HIGHLIGHT_LIMIT = 10_000

/** 正文是否有非空白内容（computed 缓存：trim 是全量扫描，禁止进模板逐渲染执行）。 */
const hasBody = computed(() => props.response.body.trim() !== '')

// ---------- 请求（本次实际发出的报文，临时数据不落库） ----------
/** 旧后端 / 未经过 HTTP 引擎发送（gRPC / 手工构造）时为 null，页签不占位。 */
const request = computed<RequestSnapshot | null>(() => props.response.request ?? null)
const requestHeaderRows = computed(() =>
  (request.value?.headers ?? []).map(([k, v]) => ({ k, v })),
)
const requestCookieRows = computed(() =>
  (request.value?.cookies ?? []).map(([name, value]) => ({ name, value })),
)
/** 渲染前模板与实际 URL 一致时无对照价值，不展示。 */
const requestTemplate = computed(() => {
  const snap = request.value
  return snap?.url_template && snap.url_template !== snap.url ? snap.url_template : ''
})

watch(
  () => props.response,
  () => {
    visibleLines.value = LINE_CHUNK
    treeExpanded.value = false
    // 新响应没有请求快照时，先把页签从「请求」上撤下来（否则空白页签）。
    if (activeTab.value === 'request' && !props.response.request) activeTab.value = 'body'
  },
)

const parsed = computed<unknown | null>(() => {
  if (!hasBody.value) return null
  if (props.response.body.length > PARSE_LIMIT_BYTES) return null
  try {
    return JSON.parse(props.response.body)
  } catch {
    return null
  }
})

const isJson = computed(() => parsed.value !== null)

/** 树视图是否接管 pretty 展示（接管时跳过 stringify + 切分，见 pretty/prettySplit）。 */
const useTree = computed(
  () => activeTab.value === 'body' && viewMode.value === 'pretty' && isJson.value && !bodyTooLarge.value,
)

const pretty = computed(() => {
  if (parsed.value === null) return props.response.body
  // 树接管时不需要 pretty 文本：跳过体积放大 2-3 倍的 stringify。
  if (useTree.value) return ''
  return JSON.stringify(parsed.value, null, EDITOR_INDENT)
})

const isHtml = computed(() => props.response.content_type.toLowerCase().includes('html'))

/** 内容类型支持「预览」：HTML / 图片 / 音视频 / 二进制文件类。 */
const isPreviewable = computed(() =>
  /^(text\/html|image\/|audio\/|video\/|application\/(pdf|octet-stream|zip|x-zip|x-.*?zip|json))/i.test(
    props.response.content_type,
  ),
)
const isImage = computed(() => props.response.content_type.toLowerCase().startsWith('image/'))

/** 行数上限：后端最多放行 20MB 响应体，全量 split 会产生数十万行字符串驻留内存。 */
const LINE_LIMIT = 100_000

function splitLines(text: string): { lines: string[]; truncated: boolean; longLine: boolean } {
  const lines = text.split('\n', LINE_LIMIT + 1)
  let truncated = false
  if (lines.length > LINE_LIMIT) {
    lines.length = LINE_LIMIT
    truncated = true
  }
  // 单行截断：压缩 JSON/HTML 往往整页一行（可达 20MB），该行进高亮/查找/行 key
  // 都是主线程 O(n) 开销，先截到 MAX_RENDER_LINE 再进渲染管线。
  let longLine = false
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].length > MAX_RENDER_LINE) {
      lines[i] = lines[i].slice(0, MAX_RENDER_LINE)
      longLine = true
    }
  }
  return { lines, truncated, longLine }
}

/**
 * 切分懒计算：原来两个全量 split 无论当前是否可见都执行
 *（20MB body → 数十万行字符串 ×2 份驻留）。
 */
const prettySplit = computed(() => {
  if (useTree.value || viewMode.value !== 'pretty' || activeTab.value !== 'body')
    return { lines: [] as string[], truncated: false, longLine: false }
  return splitLines(pretty.value)
})
const rawSplit = computed(() => {
  if (activeTab.value !== 'body') return { lines: [] as string[], truncated: false, longLine: false }
  // raw 视图与 preview 回退（非 html 按 raw 文本查）才需要行数组。
  if (viewMode.value !== 'raw' && !(viewMode.value === 'preview' && !isHtml.value))
    return { lines: [] as string[], truncated: false, longLine: false }
  return splitLines(props.response.body)
})

const prettyLines = computed(() => prettySplit.value.lines)
const rawLines = computed(() => rawSplit.value.lines)
const shownPrettyLines = computed(() => prettyLines.value.slice(0, visibleLines.value))
const shownRawLines = computed(() => rawLines.value.slice(0, visibleLines.value))
const hasMorePretty = computed(() => prettyLines.value.length > visibleLines.value)
const hasMoreRaw = computed(() => rawLines.value.length > visibleLines.value)
const bodyTooLarge = computed(() => props.response.body.length > PARSE_LIMIT_BYTES)
/** 行数组截断提示：超大响应全量 split 会产生数十万行字符串驻留内存。 */
const linesTruncated = computed(() => rawSplit.value.truncated || prettySplit.value.truncated)
/** 超长单行截断提示（与行数截断口径不同，分开提示）。 */
const longLinesTruncated = computed(() => rawSplit.value.longLine || prettySplit.value.longLine)
function showMoreLines(): void {
  visibleLines.value += LINE_CHUNK
}

// ---------- Cookies（由 set-cookie 响应头解析） ----------
interface Cookie {
  name: string
  value: string
  domain: string
  path: string
  expires: string
  httpOnly: boolean
  secure: boolean
  sameSite: string
}

const cookies = computed<Cookie[]>(() => {
  const out: Cookie[] = []
  for (const [k, v] of props.response.headers) {
    if (k.toLowerCase() !== 'set-cookie') continue
    const parts = v.split(';').map((s) => s.trim())
    const [nv, ...attrs] = parts
    const eq = nv.indexOf('=')
    const cookie: Cookie = {
      name: eq > 0 ? nv.slice(0, eq).trim() : nv,
      value: eq > 0 ? nv.slice(eq + 1).trim() : '',
      domain: '',
      path: '',
      expires: '',
      httpOnly: false,
      secure: false,
      sameSite: '',
    }
    for (const a of attrs) {
      const i = a.indexOf('=')
      const key = (i > 0 ? a.slice(0, i) : a).toLowerCase()
      const val = i > 0 ? a.slice(i + 1).trim() : 'true'
      if (key === 'domain') cookie.domain = val
      else if (key === 'path') cookie.path = val
      else if (key === 'expires') cookie.expires = val
      else if (key === 'httponly') cookie.httpOnly = true
      else if (key === 'secure') cookie.secure = true
      else if (key === 'samesite') cookie.sameSite = val
    }
    out.push(cookie)
  }
  return out
})

// ---------- 脚本运行摘要（pre/post；无脚本时为空，页签不显示） ----------
const scriptSummaries = computed(() => props.response.scripts ?? [])
const scriptTestCount = computed(() =>
  scriptSummaries.value.reduce((n, s) => n + s.tests.length, 0),
)

const responseTabs = computed<TabItem[]>(() => [
  { key: 'body', label: t('response.tabBody') },
  { key: 'headers', label: t('response.tabHeaders'), count: headerRows.value.length },
  { key: 'cookies', label: t('response.tabCookies'), count: cookies.value.length },
  // 「响应示例」是本接口的历史响应存档：有才显示，避免空页签占位。
  // 内容由宿主经 #examples 插槽注入，本组件不持有其数据。
  ...(props.examplesCount && props.examplesCount > 0
    ? [{ key: 'examples', label: t('response.tabExamples'), count: props.examplesCount }]
    : []),
  // 「脚本」页签：本次请求带脚本运行摘要（断言 + console）时才显示。
  ...(scriptSummaries.value.length
    ? [
        {
          key: 'scripts',
          label: t('response.tabScripts'),
          count: scriptTestCount.value || undefined,
        },
      ]
    : []),
  // 「请求」置末：主视图以响应为主，实际发出报文是排查时的补充信息（无快照不占位）。
  ...(request.value ? [{ key: 'request', label: t('response.tabRequest') }] : []),
])

// ---------- 操作 ----------
/** 复制源：preview/raw 用原始 body；pretty 用格式化文本。树接管时 `pretty`
 * 为 ''（大响应跳过 stringify 的优化），需回退原始 body，否则复制到空串。 */
const copySource = computed(() =>
  viewMode.value === 'raw' || viewMode.value === 'preview'
    ? props.response.body
    : pretty.value || props.response.body,
)

async function copyBody(): Promise<void> {
  let text = copySource.value
  if (props.response.body_omitted) {
    // IPC 只传了预览，先取全文再复制；缓存过期则回退预览并警告。
    const full = props.loadFullBody ? await props.loadFullBody() : null
    if (full != null) text = full
    else toast.warning(t('response.previewCapped'))
  }
  const ok = await copyText(text)
  if (ok) toast.success(t('response.copied'))
  else toast.error(t('response.copyFail'))
}

const MODE_OPTIONS = computed<SegmentOption[]>(() => [
  { value: 'pretty', label: t('response.pretty'), icon: 'list' },
  { value: 'raw', label: t('response.raw'), icon: 'code' },
  ...(isPreviewable.value ? [{ value: 'preview', label: t('response.preview'), icon: 'eye' as const }] : []),
])

/** 响应类型不支持预览时，强制退回「格式化」，避免残留 preview 状态。 */
watch(viewMode, (m) => {
  if (m === 'preview' && !isPreviewable.value) viewMode.value = 'pretty'
})

// ---------- 查找（Find in Response） ----------
const findOpen = ref(false)
const query = ref('')
const activeMatch = ref(0)
/** JSON 树上报的匹配总数（树视图的权威计数）。 */
const treeTotal = ref(0)

/**
 * 防抖后的查找词：树匹配 / 行视图高亮 / 计数全部基于它。
 * 大响应上这些计算都是全量的，逐键执行会明显卡顿；输入框本身
 * （FindBar v-model）仍用即时 query 保持跟手。空词立即生效，保证
 * 清空搜索时高亮无残留。
 */
const searchQuery = ref('')
let searchTimer: ReturnType<typeof setTimeout> | undefined
watch(query, (q) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q) {
    searchQuery.value = ''
    return
  }
  searchTimer = setTimeout(() => {
    searchQuery.value = q
  }, 160)
})

/** JSON 树是否可见（查找对其生效；行视图走本地计数）——即 useTree 别名。 */
const treeVisible = useTree

/** 行视图（rp-lines）当前渲染的行；仅按实际可见行计数，保证大响应不卡顿。 */
const searchLines = computed(() => {
  if (!searchQuery.value) return []
  if (viewMode.value === 'raw') return shownRawLines.value
  if (viewMode.value === 'preview') return isHtml.value ? [] : shownRawLines.value
  return shownPrettyLines.value
})

function countIn(text: string): number {
  // 超长行不参与查找计数（与高亮降级同口径，避免对大行逐次 toLowerCase 全量扫）。
  if (text.length > LINE_HIGHLIGHT_LIMIT) return 0
  const ql = searchQuery.value.toLowerCase()
  let n = 0
  let from = 0
  for (;;) {
    const idx = text.toLowerCase().indexOf(ql, from)
    if (idx === -1) break
    n += 1
    from = idx + ql.length
  }
  return n
}

const textTotal = computed(() => searchLines.value.reduce((n, ln) => n + countIn(ln), 0))

const total = computed(() =>
  treeVisible.value ? treeTotal.value : textTotal.value,
)

watch(query, () => {
  activeMatch.value = 0
})

watch(total, (t) => {
  if (t === 0) activeMatch.value = 0
  else if (activeMatch.value >= t) activeMatch.value = t - 1
})

/** 行视图文本高亮：转义原文并用 <mark> 包裹所有匹配。 */
function highlightText(raw: string, q: string): string {
  if (!q) return escapeHtml(raw)
  const lower = raw.toLowerCase()
  const ql = q.toLowerCase()
  let out = ''
  let from = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) {
      out += escapeHtml(raw.slice(from))
      return out
    }
    out += escapeHtml(raw.slice(from, idx))
    out += `<mark class="rp-find-mark">${escapeHtml(raw.slice(idx, idx + q.length))}</mark>`
    from = idx + q.length
  }
}

/** Pretty 行视图：JSON 语法高亮 + 查找标记（与请求 Body 编辑器共用同一主题，见 utils/highlight.ts）。 */
function highlightPrettyText(raw: string, q: string): string {
  return highlightJSONText(raw, q)
}

/**
 * 超大行数时关闭逐行 JSON 正则高亮（每行一次正则 + 转义，上万行即掉帧），
 * 降级为纯文本 + 查找标记。
 */
const prettyHighlightOff = computed(() => prettyLines.value.length > 5000)

/** 行视图行对象：稳定 key + 已算好的高亮 HTML。 */
interface LineRow {
  n: number
  key: string
  html: string
}

/**
 * 行 key：行号 + 内容哈希（FNV-1a）+ 长度。
 * 原来用索引当 key，新响应 / 增行时 Vue 会复用错位的 DOM；内容进 key 后
 * 只有真正变化的行会被复用，且重复内容行靠行号区分。哈希只在行数组或
 * 高亮输入变化时算一次（computed 缓存），不进每次渲染。
 */
function rowKey(index: number, text: string): string {
  let h = 0x811c9dc5
  // 只哈希前 4KB：行内容已可能达 100KB，全量哈希在大行上是纯主线程开销；
  // key 已含行号 + 长度，截断哈希不引入复用歧义。
  const n = Math.min(text.length, 4096)
  for (let i = 0; i < n; i++) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return `${index}:${(h >>> 0).toString(36)}:${text.length}`
}

/**
 * 行高亮提升为 computed：模板里原来每次重渲染（切查找、折叠、activeMatch
 * 变化……）都对所有可见行重跑 escape + 正则；现在只在行数组 / 查找词 /
 * 高亮开关变化时重算一次，渲染直接读缓存。
 */
const prettyRows = computed<LineRow[]>(() => {
  const q = searchQuery.value
  const off = prettyHighlightOff.value
  return shownPrettyLines.value.map((ln, i) => ({
    n: i + 1,
    key: rowKey(i, ln),
    // 超长行纯转义（跳过 JSON 分词与查找标记，见 LINE_HIGHLIGHT_LIMIT）。
    html:
      ln.length > LINE_HIGHLIGHT_LIMIT
        ? escapeHtml(ln)
        : off
          ? highlightText(ln, q)
          : highlightPrettyText(ln, q),
  }))
})

const rawRows = computed<LineRow[]>(() => {
  const q = searchQuery.value
  return shownRawLines.value.map((ln, i) => ({
    n: i + 1,
    key: rowKey(i, ln),
    html: ln.length > LINE_HIGHLIGHT_LIMIT ? escapeHtml(ln) : highlightText(ln, q),
  }))
})

function nextMatch(): void {
  if (!total.value) return
  activeMatch.value = (activeMatch.value + 1) % total.value
}

function prevMatch(): void {
  if (!total.value) return
  activeMatch.value = (activeMatch.value - 1 + total.value) % total.value
}

function closeFind(): void {
  findOpen.value = false
  query.value = ''
  activeMatch.value = 0
  treeTotal.value = 0
}

function toggleFind(): void {
  if (findOpen.value) {
    closeFind()
  } else {
    findOpen.value = true
  }
}

/** ⌘F / Ctrl+F 打开查找（输入框内不拦截）。 */
function onWindowKeydown(e: KeyboardEvent): void {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'f') return
  if (activeTab.value !== 'body' || !hasBody.value) return
  const target = e.target as HTMLElement | null
  if (
    target &&
    (target.tagName === 'INPUT' ||
      target.tagName === 'TEXTAREA' ||
      target.isContentEditable)
  ) {
    return
  }
  e.preventDefault()
  findOpen.value = true
}

const treeRef = ref<{ expandAll: () => void; collapseAll: () => void } | null>(null)

/** 展开/收起全部合并为单切换按钮：记录当前树状态决定动作与图标方向。 */
const treeExpanded = ref(false)
function toggleTreeAll(): void {
  if (treeExpanded.value) {
    treeRef.value?.collapseAll()
    treeExpanded.value = false
  } else {
    treeRef.value?.expandAll()
    treeExpanded.value = true
  }
}

// ---------- 折叠 ----------
/** 折叠状态：只保留状态栏，正文区整体收起（面板随之收缩，剩余空间由响应区布局接管）。 */
const collapsed = ref(false)

function toggleCollapsed(): void {
  collapsed.value = !collapsed.value
}

onMounted(() => window.addEventListener('keydown', onWindowKeydown))
onUnmounted(() => {
  window.removeEventListener('keydown', onWindowKeydown)
  if (searchTimer) clearTimeout(searchTimer)
})
</script>

<template>
  <div class="rp" :class="[`tone-${tone}`, { collapsed }]">
    <div class="rp-toolbar">
      <div class="rp-metrics">
        <span class="rp-status">
          <Icon name="dot" :size="8" /> {{ statusText }}
        </span>
        <span class="rp-meta">
          <span class="rp-meta-label">{{ t('response.duration') }}</span>
          <span class="rp-meta-value"><Icon name="clock" :size="14" /> {{ formatDuration(response.duration_ms) }}</span>
        </span>
        <span class="rp-meta">
          <span class="rp-meta-label">{{ t('response.size') }}</span>
          <span class="rp-meta-value"><Icon name="package" :size="14" /> {{ sizeText }}</span>
        </span>
        <span v-if="response.content_type" class="rp-type">{{ response.content_type }}</span>
        <span v-if="response.truncated" class="rp-truncated" :title="t('response.truncatedHint')">{{ t('response.truncated') }}</span>
      </div>

      <Tabs v-model="activeTab" :tabs="responseTabs" size="sm" class="rp-inline-tabs" />
      <span class="rp-toolbar-spacer"></span>

      <SegmentedControl
        v-if="activeTab === 'body'"
        class="rp-mode-seg"
        :model-value="viewMode"
        :options="MODE_OPTIONS"
        size="sm"
        @update:model-value="viewMode = $event as ViewMode"
      />
      <span class="rp-actions">
        <Tooltip :content="t('response.findHint')" placement="bottom">
          <button
            class="rp-icon-btn"
            type="button"
            :class="{ active: findOpen }"
            :aria-label="t('response.findHint')"
            @click="toggleFind"
          >
            <Icon name="search" :size="16" />
          </button>
        </Tooltip>
        <Tooltip v-if="treeVisible" :content="treeExpanded ? t('response.collapseAll') : t('response.expandAll')" placement="bottom">
          <button
            class="rp-icon-btn"
            type="button"
            :aria-label="treeExpanded ? t('response.collapseAll') : t('response.expandAll')"
            @click="toggleTreeAll"
          >
            <Icon :name="treeExpanded ? 'chevron-up' : 'chevron-down'" :size="13" />
          </button>
        </Tooltip>
        <Tooltip :content="t('response.saveExample')" placement="bottom">
          <button
            class="rp-icon-btn"
            type="button"
            :aria-label="t('response.saveExample')"
            @click="emit('saveExample')"
          >
            <Icon name="save" :size="16" />
          </button>
        </Tooltip>
        <Tooltip :content="t('response.copyBody')" placement="bottom">
          <button class="rp-icon-btn" type="button" :aria-label="t('response.copyBody')" @click="copyBody">
            <Icon name="copy" :size="16" />
          </button>
        </Tooltip>
        <Tooltip :content="collapsed ? t('response.expand') : t('response.collapse')" placement="bottom">
          <button
            class="rp-icon-btn"
            type="button"
            :aria-label="collapsed ? t('response.expand') : t('response.collapse')"
            @click="toggleCollapsed"
          >
            <Icon :name="collapsed ? 'chevron-down' : 'chevron-up'" :size="13" />
          </button>
        </Tooltip>
      </span>
      <!-- 面板高度拖拽把手（视觉 affordance）：按下向宿主上报，拖拽热区仍在整条分割带 -->
      <span
        class="rp-drag-grip"
        aria-hidden="true"
        :title="t('editor.splitterHint')"
        @mousedown="emit('dragStart', $event as MouseEvent)"
      >
        <Icon name="chevrons-up-down" :size="14" />
      </span>
    </div>

    <!-- 本次实际发出的请求报文（Apifox「实际请求」同款）：
         只读展示，随响应临时返回、不落库；无快照时不渲染（页签同步隐藏）。 -->
    <div v-show="!collapsed" v-if="activeTab === 'request' && request" class="rp-scroll">
      <div class="rp-req-line">
        <MethodText :method="request.method" />
        <span class="rp-req-url">{{ request.url }}</span>
      </div>
      <p v-if="requestTemplate" class="rp-req-template">
        {{ t('response.reqTemplate') }}: <span class="rp-req-template-val">{{ requestTemplate }}</span>
      </p>

      <div class="rp-section">
        {{ t('response.reqHeaders') }}
        <span class="rp-section-count">{{ requestHeaderRows.length }}</span>
      </div>
      <div v-for="(h, i) in requestHeaderRows" :key="`h${i}`" class="rp-header-row">
        <span class="rp-header-key">{{ h.k }}</span>
        <span class="rp-header-val">{{ h.v }}</span>
      </div>
      <p v-if="!requestHeaderRows.length" class="rp-empty">{{ t('response.reqNoHeaders') }}</p>

      <div class="rp-section">
        {{ t('response.reqCookies') }}
        <span class="rp-section-count">{{ requestCookieRows.length }}</span>
        <span class="rp-section-hint">{{ t('response.reqCookiesHint') }}</span>
      </div>
      <div v-for="(c, i) in requestCookieRows" :key="`c${i}`" class="rp-header-row">
        <span class="rp-header-key">{{ c.name }}</span>
        <span class="rp-header-val">{{ c.value }}</span>
      </div>
      <p v-if="!requestCookieRows.length" class="rp-empty">{{ t('response.reqNoCookies') }}</p>

      <div class="rp-section">{{ t('response.reqBody') }}</div>
      <p v-if="request.body_omitted" class="rp-note">{{ t('response.reqBodyTruncated') }}</p>
      <pre v-if="request.body" class="rp-req-body">{{ request.body }}</pre>
      <p v-else class="rp-empty">{{ t('response.reqNoBody') }}</p>
    </div>

    <div v-show="!collapsed" v-if="activeTab === 'body'" class="rp-body">
      <FindBar
        v-if="findOpen && hasBody"
        v-model:query="query"
        :index="activeMatch"
        :total="total"
        @prev="prevMatch"
        @next="nextMatch"
        @close="closeFind"
      />
      <div class="rp-scroll">
        <p v-if="bodyTooLarge" class="rp-note">
          {{ t('response.tooLarge') }}
        </p>
        <p v-if="linesTruncated" class="rp-note">{{ t('response.linesTruncated') }}</p>
        <p v-if="longLinesTruncated" class="rp-note">{{ t('response.lineTooLong') }}</p>
        <p v-if="!hasBody" class="rp-empty">{{ t('response.emptyBody') }}</p>
        <JsonTree
          v-else-if="viewMode === 'pretty' && isJson"
          ref="treeRef"
          :data="parsed"
          :query="treeVisible ? searchQuery : ''"
          :active-match="activeMatch"
          @match-count="treeTotal = $event"
        />
        <div v-else-if="viewMode === 'pretty'" class="rp-lines">
          <div v-for="row in prettyRows" :key="row.key" class="rp-line">
            <span class="rp-line-gutter">{{ row.n }}</span>
            <span class="rp-line-text" v-html="row.html"></span>
          </div>
          <button
            v-if="hasMorePretty"
            class="rp-more"
            type="button"
            @click="showMoreLines"
          >
            {{ t('response.showMore', { shown: visibleLines, total: prettyLines.length }) }}
          </button>
        </div>
        <div v-else-if="viewMode === 'raw'" class="rp-lines">
          <div v-for="row in rawRows" :key="row.key" class="rp-line">
            <span class="rp-line-gutter">{{ row.n }}</span>
            <span class="rp-line-text" v-html="row.html"></span>
          </div>
          <button v-if="hasMoreRaw" class="rp-more" type="button" @click="showMoreLines">
            {{ t('response.showMore', { shown: visibleLines, total: rawLines.length }) }}
          </button>
        </div>
        <iframe
          v-else-if="isHtml && !bodyTooLarge"
          class="rp-frame"
          sandbox="allow-same-origin"
          :srcdoc="response.body"
          :title="t('response.previewTitle')"
        ></iframe>
        <img
          v-else-if="isImage"
          class="rp-frame rp-preview-img"
          :src="`data:${response.content_type};base64,${response.body}`"
          :alt="t('response.imageAlt')"
        />
        <div v-else class="rp-preview-note">{{ t('response.noPreview') }}</div>
      </div>
    </div>

    <div v-show="!collapsed" v-else-if="activeTab === 'headers'" class="rp-scroll">
      <div v-for="(h, i) in headerRows" :key="i" class="rp-header-row">
        <span class="rp-header-key">{{ h.k }}</span>
        <span class="rp-header-val">{{ h.v }}</span>
      </div>
      <p v-if="!headerRows.length" class="rp-empty">{{ t('response.noHeaders') }}</p>
    </div>

    <div v-show="!collapsed" v-else-if="activeTab === 'cookies'" class="rp-scroll">
      <div v-for="(c, i) in cookies" :key="i" class="rp-cookie">
        <div class="rp-cookie-top">
          <span class="rp-cookie-name">{{ c.name }}</span>
          <span class="rp-cookie-value">{{ c.value }}</span>
          <span class="rp-cookie-flags">
            <span v-if="c.secure" class="rp-flag">Secure</span>
            <span v-if="c.httpOnly" class="rp-flag">HttpOnly</span>
            <span v-if="c.sameSite" class="rp-flag">{{ c.sameSite }}</span>
          </span>
        </div>
        <div v-if="c.domain || c.path || c.expires" class="rp-cookie-meta">
          <span v-if="c.domain">{{ t('response.cookieDomain', { v: c.domain }) }}</span>
          <span v-if="c.path">{{ t('response.cookiePath', { v: c.path }) }}</span>
          <span v-if="c.expires">{{ t('response.cookieExpires', { v: c.expires }) }}</span>
        </div>
      </div>
      <p v-if="!cookies.length" class="rp-empty">{{ t('response.noCookies') }}</p>
    </div>

    <!-- 脚本运行摘要：pre/post 两段各一张卡，断言逐条 pass/fail + console 输出 -->
    <div v-show="!collapsed" v-else-if="activeTab === 'scripts'" class="rp-scroll">
      <div v-for="(s, i) in scriptSummaries" :key="`${s.phase}-${i}`" class="script-card">
        <div class="script-card-head">
          <span class="script-phase">{{
            s.phase === 'pre_request' ? t('scripts.phasePre') : t('scripts.phasePost')
          }}</span>
          <span
            v-if="s.tests.length"
            class="script-count"
            :class="{ 'script-count-fail': s.tests.some((x) => !x.passed) }"
          >
            {{
              t('scripts.testCount', {
                pass: s.tests.filter((x) => x.passed).length,
                total: s.tests.length,
              })
            }}
          </span>
        </div>
        <p v-if="s.error" class="script-error">{{ s.error }}</p>
        <ul v-if="s.tests.length" class="script-tests">
          <li v-for="(test, j) in s.tests" :key="j" class="script-test" :class="{ fail: !test.passed }">
            <span class="script-dot" aria-hidden="true"></span>
            <span class="script-test-name">{{ test.name }}</span>
            <span v-if="test.error" class="script-test-err">{{ test.error }}</span>
          </li>
        </ul>
        <pre v-if="s.logs.length" class="script-logs">{{ s.logs.join('\n') }}</pre>
      </div>
    </div>

    <!-- 响应示例：内容由宿主持有（#examples 插槽），本组件只提供页签与滚动容器 -->
    <div v-show="!collapsed" v-else-if="activeTab === 'examples'" class="rp-scroll">
      <slot name="examples" />
    </div>
  </div>
</template>

<style scoped>
.rp {
  border: 1px solid var(--border-panel);
  /* 顶边细线即请求/响应边界（分割条已无视觉元素，white/8 档细线） */
  border-radius: var(--radius-md);
  background: var(--bg-card);
  box-shadow: var(--shadow-panel);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.rp.tone-err {
  border-color: var(--danger-border);
}
/* 折叠时只保留状态栏，面板收缩为内容高度 */
.rp.collapsed {
  height: auto;
}

/* ---- 单行工具栏：状态指标 + 页签 + 模式 + 操作 ----
 * 整排 flex items-center 同一水平中轴；横向节奏统一 12px（gap-3） */
.rp-toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 40px;
  padding: 4px 12px;
  border-bottom: 1px solid var(--border);
  background: transparent;
  flex-shrink: 0;
  overflow: hidden;
}

.rp-inline-tabs {
  flex-shrink: 0;
}
.rp-inline-tabs :deep(.tabs) {
  border-bottom: none;
}
.rp-inline-tabs :deep(.tab) {
  height: 28px;
}

.rp-toolbar-spacer {
  flex: 1 1 auto;
  min-width: 8px;
}

.rp-mode-seg {
  flex-shrink: 0;
}

/* 状态：小圆点 + 同色数值/文本（无色块底，视觉负担最小） */
.rp-status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
  font-size: 12px;
  font-family: var(--font-mono);
  line-height: 1.4;
  letter-spacing: 0.02em;
}
.rp.tone-ok .rp-status {
  color: var(--success);
}
.rp.tone-warn .rp-status {
  color: var(--warning);
}
.rp.tone-err .rp-status {
  color: var(--danger);
}

/* 指标组：状态徽章 + 耗时/大小/类型 同组 flex items-center gap-3（12px），随 flex 居中同一水平轴线 */
.rp-metrics {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.rp-meta {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}

/* 指标明暗对比：标签 text-3 档；数值统一 mono / text-xs / medium / text-value 档（深 zinc-300·浅 zinc-600） */
.rp-meta-label {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.rp-meta-value {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xs);
  font-family: var(--font-mono);
  font-weight: 500;
  color: var(--text-value);
}
.rp-meta-value svg {
  color: var(--accent);
  opacity: 0.9;
}

.rp-type {
  min-width: 0;
  font-size: var(--fs-xs);
  font-family: var(--font-mono);
  font-weight: 500;
  color: var(--text-value);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.rp-truncated {
  flex-shrink: 0;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 600;
  color: var(--warning);
  background: var(--warning-tint);
}

/* 最右操作区（纯图标按钮，自带 Tooltip）；与分段切换的间距由工具栏 gap 统一 */
.rp-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 4px;
}

/* 面板高度拖拽把手（工具栏最右）：按下上报宿主开拖，拖拽热区仍覆盖整条分割带 */
.rp-drag-grip {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  color: var(--text-3);
  cursor: row-resize;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.rp-drag-grip:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.rp-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.rp-icon-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.rp-icon-btn svg {
  color: var(--accent);
}
.rp-icon-btn.active {
  color: var(--accent);
  background: var(--accent-tint, var(--bg-hover));
}

/* ---- 查找 ---- */
:deep(.rp-find-mark) {
  background: var(--accent-tint, rgba(99, 102, 241, 0.25));
  color: inherit;
  border-radius: 2px;
  padding: 0 1px;
}

/* ---- 正文区 ---- */
.rp-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

/* 响应正文 = 输出代码块：圆角卡片 + 半透边框 + 代码底色 */
.rp-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  margin: 0 12px 12px;
  border: 1px solid var(--border-editor);
  border-radius: var(--radius);
  background: var(--bg-code);
  padding: 8px 0;
}

.rp-lines {
  font-family: var(--font-mono);
  font-size: 12.5px;
  line-height: 1.55;
}

.rp-line {
  display: flex;
  align-items: flex-start;
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-1);
}

/* 空折叠列占位：与树视图（箭头专列）同宽，Body/原始 切换时代码与行号不横跳 */
.rp-line::before {
  content: '';
  flex-shrink: 0;
  width: var(--code-fold-w);
}

.rp-line-gutter {
  flex-shrink: 0;
  /* 与请求编辑器共用 --code-gutter-* 几何（style.css 单源），代码左缘绝对对齐 */
  width: var(--code-gutter-w);
  padding-right: var(--code-gutter-gap);
  text-align: right;
  /* user-select:none 只挡住「从行号起选」这一种路径；从正文往左拖、或
   * 三击/全选时，浏览器仍会把跨过的行号一起涂蓝（正文与行号同处一个
   * 选择流，不同于 CodeMirror 把 gutter 放在独立 DOM 层）。
   * 故再抹掉行号自身的选区着色，使复制观感与请求编辑器一致。 */
  user-select: none;
  color: var(--tok-gutter);
  font-size: var(--fs-xxs);
}
.rp-line-gutter::selection {
  /* 行号被扫过时不显示高亮：选区视觉上只在正文上 */
  background-color: transparent;
  color: var(--tok-gutter);
}

.rp-line-text {
  min-width: 0;
  flex: 1;
}

.rp-frame {
  display: block;
  width: 100%;
  height: 100%;
  border: none;
  background: var(--bg-panel);
}

.rp-preview-img {
  object-fit: contain;
  padding: 8px;
}

.rp-preview-note {
  margin: 0;
  padding: 14px 16px;
  font-size: 12px;
  color: var(--text-3);
}

.rp-empty {
  margin: 0;
  padding: 14px 16px;
  font-size: 12px;
  color: var(--text-3);
}

.rp-note {
  margin: 0;
  padding: 6px 12px 0;
  font-size: 11.5px;
  color: var(--warning);
}

.rp-more {
  display: block;
  width: 100%;
  padding: 8px;
  border: none;
  border-top: 1px dashed var(--border);
  background: none;
  font-family: inherit;
  font-size: 11.5px;
  color: var(--accent);
  cursor: pointer;
}
.rp-more:hover {
  opacity: 0.8;
}

/* ---- Headers ---- */
.rp-header-row {
  display: grid;
  grid-template-columns: minmax(120px, 260px) 1fr;
  gap: 10px;
  align-items: baseline;
  padding: 5px 12px;
  border-bottom: 1px dashed var(--border);
  font-size: 11.5px;
}
.rp-header-row:last-child {
  border-bottom: none;
}

.rp-header-key {
  font-weight: 600;
  color: var(--text-1);
  word-break: break-all;
  overflow-wrap: anywhere;
}

.rp-header-val {
  color: var(--text-2);
  word-break: break-all;
  overflow-wrap: anywhere;
}

/* ---- 请求（实际发出形态，只读） ---- */
.rp-req-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 2px 12px 8px;
  border-bottom: 1px dashed var(--border);
}

.rp-req-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
  word-break: break-all;
  overflow-wrap: anywhere;
}

.rp-req-template {
  margin: 0;
  padding: 6px 12px 0;
  font-size: 11.5px;
  color: var(--text-3);
  word-break: break-all;
  overflow-wrap: anywhere;
}

.rp-req-template-val {
  font-family: var(--font-mono);
  color: var(--text-2);
}

.rp-section {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 10px 12px 2px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.rp-section-count {
  font-weight: 600;
  color: var(--text-3);
}

.rp-section-hint {
  font-weight: 400;
  letter-spacing: 0;
  text-transform: none;
}

.rp-req-body {
  margin: 4px 12px 8px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-panel);
  font-family: var(--font-mono);
  font-size: 11.5px;
  line-height: 1.55;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 320px;
  overflow: auto;
}

/* ---- Cookies ---- */
.rp-cookie {
  margin: 0 12px 8px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-panel);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.rp-cookie-top {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.rp-cookie-name {
  flex-shrink: 0;
  font-weight: 700;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
}

.rp-cookie-value {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--text-2);
  word-break: break-all;
}

.rp-cookie-flags {
  flex-shrink: 0;
  display: inline-flex;
  gap: 4px;
}

.rp-flag {
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  color: var(--info);
  background: var(--info-tint, var(--accent-tint));
}

.rp-cookie-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 脚本运行摘要（pre/post 断言 + console 输出） ---- */
.script-card {
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
}

.script-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.script-phase {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.script-count {
  font-size: var(--fs-xxs);
  color: var(--success);
}

.script-count-fail {
  color: var(--danger);
}

.script-error {
  margin: 6px 0 0;
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--danger);
  white-space: pre-wrap;
  word-break: break-all;
}

.script-tests {
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.script-test {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--text-1);
  min-width: 0;
}

.script-dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success);
}

.script-test.fail .script-dot {
  background: var(--danger);
}

.script-test-name {
  word-break: break-all;
}

.script-test-err {
  color: var(--text-3);
  word-break: break-all;
}

.script-logs {
  margin: 8px 0 0;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>
