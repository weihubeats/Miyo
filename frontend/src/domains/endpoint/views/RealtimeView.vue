<script setup lang="ts">
/**
 * RealtimeView：实时调试视图（WebSocket / SSE）。
 *
 * - WebSocket：后端 `miyo-http::ws_client` 早已完备（自动重连/心跳/离线补发），
 *   本视图经 ws_* 命令接入：建连 → 收发帧 → 断开，事件经 `miyo:ws-event` 推送；
 * - SSE：`sse_connect` 拉流转发原始文本块，前端按帧解析（event/data/id），事件经
 *   `miyo:sse-event` 推送；断线续传带 `Last-Event-ID`；
 * - 帧日志统一为 FrameItem 形状（FrameLog 组件渲染：过滤 + 虚拟滚动 + 自动滚底），
 *   选中帧在右侧详情（SplitPanes 分栏，JSON → JsonTree）；
 * - 发送支持文本 / 二进制（base64，与后端 WsSendArgs 契约一致，Ping 同样编码）
 *   与发送历史（↑/↓ 回溯）；连接配置持久化到 localStorage（miyo.realtime.*）；
 * - 消息日志上限 500 条（超限丢弃最旧）；卸载视图时自动断开，避免后台泄漏。
 */
import { computed, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useGoBack } from '@/composables/useGoBack'
import { useToast } from '@/composables/useToast'
import { useTauriEvent } from '@/composables/useTauriEvent'
import { useLocaleStore } from '@/stores/locale'
import { formatTime } from '@/utils/dateTime'
import { toBase64Utf8, fromBase64Utf8Safe } from '@/utils/base64'
import { lazyComponent } from '@/composables/lazyComponent'
import PageHeader from '@/components/navigation/PageHeader.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import JsonTree from '@/components/ui/JsonTree.vue'
import KeyValueTable, { type KVRow } from '@/components/ui/KeyValueTable.vue'
import SegmentedControl, { type SegmentOption } from '@/components/ui/SegmentedControl.vue'
import Spinner from '@/components/ui/Spinner.vue'
import SplitPanes from '@/components/ui/SplitPanes.vue'
import StatusBadge from '@/components/ui/StatusBadge.vue'
import CollapsibleSection from '@/components/ui/CollapsibleSection.vue'
import Menu, { type MenuItem } from '@/components/ui/Menu.vue'
import Tabs, { type TabItem } from '@/components/ui/Tabs.vue'
import FrameLog, { type FrameItem } from '@/domains/endpoint/components/realtime/realtime/FrameLog.vue'
import type { SseEventPayload, WsEventPayload } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

/** JSON 模式编辑器（CodeMirror 较重，仅 JSON 模式挂载）。 */
const JsonCodeMirror = lazyComponent(() => import('@/components/ui/JsonCodeMirror.vue'))

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

/**
 * 多窗口：实时视图可在独立窗口打开（边工作边监控 WS/SSE）。
 * 工作区本身保持单窗口——双开工作区会导致两份 Pinia 草稿分叉，
 * 故仅实时视图（无本地草稿态）支持弹出。
 */
const isMainWindow = ref(true)
async function detectWindow(): Promise<void> {
  try {
    if (!('__TAURI_INTERNALS__' in window)) return
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    isMainWindow.value = getCurrentWindow().label === 'main'
  } catch {
    // 非 Tauri 环境：保持默认（主窗口行为）
  }
}

/**
 * 返回来源页：实时调试可从工作区工具抽屉进入，也可从仪表板左侧导航进入，
 * 因此不能写死回 `/workspace`（从仪表板进来会被弹到某个项目里）。
 * 统一走 useGoBack（第四阶段返回策略）：
 * - 会话内有可回退历史 → `router.back()`，原路返回调用方页面；
 * - 直接深链打开（浅历史，回退会离开应用）→ 回仪表板兜底。
 */
const goBack = useGoBack()

async function popout(): Promise<void> {
  try {
    // Tauri v2：带 URL 的新窗口经 WebviewWindow 创建。
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow')
    const label = `realtime-${Date.now()}`
    const win = new WebviewWindow(label, {
      url: '/realtime',
      title: t('realtime.windowTitle'),
      width: 960,
      height: 700,
      minWidth: 640,
      minHeight: 480,
    })
    await win.once('tauri://error', (e: { payload: unknown }) => {
      toast.error(t('realtime.popoutFail'), { message: String(e.payload) })
    })
  } catch (err) {
    toast.error(t('realtime.popoutFail'), { message: errorText(err) })
  }
}

// ---------- 连接配置持久化（localStorage，try/catch 静默降级） ----------
const PERSIST_KEY = 'miyo.realtime.config'
const SPLIT_KEY = 'miyo.realtime-split'

interface RealtimePersist {
  wsUrl: string
  sseUrl: string
  autoReconnect: boolean
  subprotocols: string
  wsHeaders: KVRow[]
  sseHeaders: KVRow[]
}

function loadPersist(): Partial<RealtimePersist> {
  try {
    const raw = localStorage.getItem(PERSIST_KEY)
    return raw ? (JSON.parse(raw) as Partial<RealtimePersist>) : {}
  } catch {
    return {}
  }
}
const saved = loadPersist()

function savePersist(): void {
  try {
    const data: RealtimePersist = {
      wsUrl: wsUrl.value,
      sseUrl: sseUrl.value,
      autoReconnect: wsAutoReconnect.value,
      subprotocols: wsSubprotocols.value,
      wsHeaders: wsHeaders.value,
      sseHeaders: sseHeaders.value,
    }
    localStorage.setItem(PERSIST_KEY, JSON.stringify(data))
  } catch {
    // 存储不可用（隐私模式等）：仅本次会话内生效
  }
}

/** 帧日志 | 详情 分栏宽度（记忆 miyo.realtime-split）。 */
const splitWidth = ref((() => {
  try {
    const n = Number(localStorage.getItem(SPLIT_KEY))
    return Number.isFinite(n) && n >= 360 ? Math.round(n) : 560
  } catch {
    return 560
  }
})())
watch(splitWidth, (w) => {
  try {
    localStorage.setItem(SPLIT_KEY, String(Math.round(w)))
  } catch {
    /* 存储不可用仅本会话生效 */
  }
})

type MainTab = 'ws' | 'sse'
const mainTab = ref<MainTab>('ws')
const MAIN_TABS = computed<SegmentOption[]>(() => [
  { value: 'ws', label: t('realtime.tabWs') },
  { value: 'sse', label: t('realtime.tabSse') },
])

const MAX_LOG = 500
function cap<T>(arr: T[]): T[] {
  if (arr.length > MAX_LOG) arr.splice(0, arr.length - MAX_LOG)
  return arr
}
/**
 * 帧单调 id：头部裁剪（cap）会使后续所有行的索引整体前移，
 * 若列表用 index 作 key，列表满后每来一条消息 500 行全部重新 patch；
 * 稳定 key 下只卸载被裁掉的行。popout 新窗口各自从 0 计数，无跨窗共享。
 */
let logSeq = 0
function nowTime(): string {
  return formatTime(new Date(), { hour12: false })
}

// ---------- WebSocket ----------
const wsUrl = ref(saved.wsUrl ?? 'ws://127.0.0.1:4010')
const wsAutoReconnect = ref(saved.autoReconnect ?? true)
const wsConnId = ref<string | null>(null)
const wsState = ref<'idle' | 'connecting' | 'open' | 'closed' | 'error'>('idle')
const wsConnecting = ref(false)

/**
 * 连接级自定义请求头 / WS 子协议：与 URL 同为组件本地状态 + localStorage
 * 持久化（调试工具的核心诉求是「下次打开还在」）——鉴权头是 WS/SSE 的真实
 * 需求，后端 ws_connect / sse_connect 早已接受 headers / subprotocols。
 */
const wsHeadersOpen = ref(false)
const wsHeaders = ref<KVRow[]>(saved.wsHeaders ?? [])
const wsSubprotocols = ref(saved.subprotocols ?? '')
const sseHeadersOpen = ref(false)
const sseHeaders = ref<KVRow[]>(saved.sseHeaders ?? [])

watch([wsUrl, wsAutoReconnect, wsSubprotocols, wsHeaders], savePersist, { deep: true })

/** 已启用且有键名的头行数（折叠按钮上的计数）。 */
function headerCount(rows: KVRow[]): number {
  return rows.filter((r) => r.enabled !== false && (r.key ?? '').trim()).length
}

/** KV 行 → 握手头映射（跳过停用行与空键行）。 */
function toHeaderMap(rows: KVRow[]): Record<string, string> {
  const out: Record<string, string> = {}
  for (const r of rows) {
    if (r.enabled === false) continue
    const k = (r.key ?? '').trim()
    if (!k) continue
    out[k] = r.value ?? ''
  }
  return out
}

/** 逗号分隔的子协议输入 → 数组（去空白、去空项）。 */
function toSubprotocols(raw: string): string[] {
  return raw
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
}

const wsLog = ref<FrameItem[]>([])

function wsPush(dir: FrameItem['dir'], kind: string, text: string): void {
  wsLog.value.push({ seq: logSeq++, t: nowTime(), dir, kind, text, size: text.length })
  cap(wsLog.value)
}

// ---------- 连接失败原因 / 自动重连提示（真实状态，无虚构） ----------
const wsLastError = ref('')
const sseLastError = ref('')
/** 错误后后端自动重连触发 connecting：徽章区分「连接中」与「自动重连中」。 */
const wsReconnecting = ref(false)

async function wsConnect(): Promise<void> {
  if (!wsUrl.value.trim() || wsConnecting.value || wsConnId.value) return
  wsConnecting.value = true
  wsLastError.value = ''
  wsReconnecting.value = false
  try {
    const id = await api.wsConnect({
      url: wsUrl.value.trim(),
      headers: toHeaderMap(wsHeaders.value),
      subprotocols: toSubprotocols(wsSubprotocols.value),
      auto_reconnect: wsAutoReconnect.value,
    })
    wsConnId.value = id
    wsState.value = 'connecting'
    wsPush('sys', 'sys', t('realtime.connecting', { v: wsUrl.value.trim() }))
    pushConnHistory('ws', wsUrl.value.trim(), false) // 结果由 open / error 状态事件回填
  } catch (err) {
    wsLastError.value = errorText(err)
    pushConnHistory('ws', wsUrl.value.trim(), false)
    toast.error(t('realtime.wsConnectFail'), { message: errorText(err) })
  } finally {
    wsConnecting.value = false
  }
}

async function wsDisconnect(silent = false): Promise<void> {
  if (!wsConnId.value) return
  const id = wsConnId.value
  wsConnId.value = null
  try {
    await api.wsDisconnect(id)
  } catch {
    // 连接可能已失效：本地状态照常清理
  }
  wsState.value = 'closed'
  if (!silent) wsPush('sys', 'sys', t('realtime.disconnected'))
}

type SendKind = 'text' | 'json' | 'binary'
const sendKind = ref<SendKind>('text')
const SEND_MODES = computed<SegmentOption[]>(() => [
  { value: 'text', label: t('realtime.frameText') },
  { value: 'json', label: t('realtime.frameJson') },
  { value: 'binary', label: t('realtime.frameBinaryShort') },
])
const wsSendText = ref('')
/** 发送历史（原文，最近 20 条；↑/↓ 回溯）。 */
const sendHistory = ref<string[]>([])
const historyIdx = ref(-1)

/** JSON 模式校验提示（不阻断发送——服务端可能接受任意文本）。 */
const jsonInvalid = computed(() => {
  if (sendKind.value !== 'json') return false
  const raw = wsSendText.value.trim()
  if (!raw) return false
  try {
    JSON.parse(raw)
    return false
  } catch {
    return true
  }
})

/** 发送状态：按钮转圈 + 行内错误详情（toast 同步弹出）。 */
const sending = ref(false)
const sendError = ref('')

/**
 * 发送帧：text / json 帧发原文；binary / ping 按后端 WsSendArgs 契约先编码为
 * base64（此前 Ping 直传原文会被后端按 base64 误解码——契约修复）。
 */
async function wsSend(frame: 'text' | 'binary' | 'ping'): Promise<void> {
  if (!wsConnId.value || wsState.value !== 'open') return
  const raw = wsSendText.value
  if (!raw) return
  const payload = frame === 'text' ? raw : toBase64Utf8(raw)
  sending.value = true
  sendError.value = ''
  try {
    await api.wsSend({ connection_id: wsConnId.value, frame, payload })
    wsPush('out', frame, frame === 'text' ? raw : payload)
    if (sendHistory.value[sendHistory.value.length - 1] !== raw) {
      sendHistory.value.push(raw)
      if (sendHistory.value.length > 20) sendHistory.value.shift()
    }
    historyIdx.value = -1
    wsSendText.value = ''
  } catch (err) {
    sendError.value = errorText(err)
    toast.error(t('realtime.sendFail'), { message: errorText(err) })
  } finally {
    sending.value = false
  }
}

/** 发送入口：JSON 模式按 text 帧发原文，其余按所选帧类型。 */
function sendCurrent(): Promise<void> {
  return wsSend(sendKind.value === 'json' ? 'text' : sendKind.value)
}

/**
 * 发送区快捷键（capture，抢在 CodeMirror keymap 前）：
 * - Cmd/Ctrl + Enter：发送；
 * - Cmd/Ctrl + L：清空编辑器（Tauri 无地址栏，不与浏览器冲突）。
 * 处理后 stopPropagation：避免 text 输入的 Enter 直发二次触发。
 */
function onSendCaptureKey(e: KeyboardEvent): void {
  const mod = e.metaKey || e.ctrlKey
  if (mod && e.key === 'Enter') {
    e.preventDefault()
    e.stopPropagation()
    if (!sending.value) void sendCurrent()
  } else if (mod && e.key.toLowerCase() === 'l') {
    e.preventDefault()
    e.stopPropagation()
    wsSendText.value = ''
    sendError.value = ''
  }
}

// ---------- 消息模板（localStorage，常用帧一键回填） ----------
const TEMPLATES_KEY = 'miyo.realtime.templates'
function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}
function saveJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* 存储不可用仅本会话生效 */
  }
}
const templates = ref<string[]>(loadJson(TEMPLATES_KEY, []))
watch(templates, (v) => saveJson(TEMPLATES_KEY, v), { deep: true })

const templateMenuRef = ref<InstanceType<typeof Menu> | null>(null)
function openTemplateMenu(e: MouseEvent): void {
  const items: MenuItem[] = templates.value.map((tpl, i) => ({
    key: String(i),
    label: tpl.length > 40 ? `${tpl.slice(0, 40)}…` : tpl,
    icon: 'file' as const,
  }))
  items.push(
    { key: '__save', label: t('realtime.templateSave'), icon: 'save' as const, iconAccent: true, dividerBefore: items.length > 0 },
    ...(templates.value.length
      ? [{ key: '__clear', label: t('realtime.templateClearAll'), icon: 'trash' as const, danger: true }]
      : []),
  )
  templateMenuRef.value?.openAt(e.currentTarget as HTMLElement, items)
}
function onTemplateSelect(item: MenuItem): void {
  if (item.key === '__save') {
    const raw = wsSendText.value.trim()
    if (!raw) return
    if (!templates.value.includes(raw)) {
      templates.value.push(raw)
      if (templates.value.length > 20) templates.value.shift()
    }
    toast.success(t('realtime.templateSaved'))
    return
  }
  if (item.key === '__clear') {
    templates.value = []
    return
  }
  const tpl = templates.value[Number(item.key)]
  if (tpl != null) {
    wsSendText.value = tpl
    sendError.value = ''
  }
}

// ---------- 连接历史（localStorage，一键回填重连） ----------
const HISTORY_KEY = 'miyo.realtime.conn-history'
interface ConnHistoryEntry {
  url: string
  proto: 'ws' | 'sse'
  at: number
  ok: boolean
}
const connHistory = ref<ConnHistoryEntry[]>(loadJson<ConnHistoryEntry[]>(HISTORY_KEY, []).slice(0, 12))
watch(connHistory, (v) => saveJson(HISTORY_KEY, v.slice(0, 12)), { deep: true })

/** 记录一次连接尝试（ok=false 占位，结果由状态事件回填）。 */
function pushConnHistory(proto: 'ws' | 'sse', url: string, ok: boolean): void {
  if (!url) return
  connHistory.value.unshift({ url, proto, at: Date.now(), ok })
  connHistory.value = connHistory.value.slice(0, 12)
}

/** 回填最近一条同 URL 记录的连接结果（open / error 状态事件调用）。 */
function markConnHistory(proto: 'ws' | 'sse', url: string, ok: boolean): void {
  const hit = connHistory.value.find((h) => h.proto === proto && h.url === url)
  if (hit) hit.ok = ok
}

const historyMenuRef = ref<InstanceType<typeof Menu> | null>(null)
function openHistoryMenu(e: MouseEvent): void {
  const items: MenuItem[] = connHistory.value.map((h, i) => ({
    key: String(i),
    label: `${h.ok ? '✓' : '✕'} ${formatTime(new Date(h.at), { hour12: false })} ${h.url}`,
    icon: h.proto === 'ws' ? ('globe' as const) : ('zap' as const),
    dividerBefore: i === 0 ? false : connHistory.value[i - 1]?.proto !== h.proto,
  }))
  if (!items.length) items.push({ key: '__none', label: t('realtime.historyEmpty'), disabled: true })
  historyMenuRef.value?.openAt(e.currentTarget as HTMLElement, items)
}
function onHistorySelect(item: MenuItem): void {
  const entry = connHistory.value[Number(item.key)]
  if (!entry) return
  if (mainTab.value !== entry.proto) mainTab.value = entry.proto
  if (entry.proto === 'ws') {
    wsUrl.value = entry.url
    void wsConnect()
  } else {
    sseUrl.value = entry.url
    void sseConnect()
  }
}

/** ↑/↓ 回溯发送历史（↑ 从最新开始；↓ 反向，越过最早一条回到输入态）。 */
function onSendKeydown(e: KeyboardEvent): void {
  if (e.key === 'ArrowUp') {
    if (!sendHistory.value.length) return
    e.preventDefault()
    const idx = historyIdx.value === -1 ? sendHistory.value.length - 1 : Math.max(0, historyIdx.value - 1)
    historyIdx.value = idx
    wsSendText.value = sendHistory.value[idx] ?? ''
  } else if (e.key === 'ArrowDown') {
    if (historyIdx.value === -1) return
    e.preventDefault()
    const idx = historyIdx.value + 1
    if (idx >= sendHistory.value.length) {
      historyIdx.value = -1
      wsSendText.value = ''
    } else {
      historyIdx.value = idx
      wsSendText.value = sendHistory.value[idx] ?? ''
    }
  }
}

function onWsEvent(payload: WsEventPayload): void {
  if (payload.connection_id !== wsConnId.value) return
  if (payload.kind === 'state') {
    const s = payload.state
    const prev = wsState.value
    wsState.value =
      s === 'open' ? 'open' : s === 'connecting' ? 'connecting' : s === 'closed' ? 'closed' : 'error'
    if (s === 'open') {
      wsLastError.value = ''
      wsReconnecting.value = false
      markConnHistory('ws', wsUrl.value.trim(), true)
    } else if (s === 'error') {
      // state 载荷不带 message；具体原因由 kind='failed' 事件补充（那里才写 wsLastError）
      wsReconnecting.value = false
      if (!wsLastError.value) wsLastError.value = t('realtime.stateError')
      markConnHistory('ws', wsUrl.value.trim(), false)
    } else if (s === 'connecting' && (prev === 'error' || prev === 'closed')) {
      // 断开后重新建连：后端 auto_reconnect 在跑，徽章与错误提示联动
      wsReconnecting.value = true
    } else if (s === 'closed') {
      wsReconnecting.value = false
    }
    wsPush('sys', 'sys', t('realtime.stateChanged', { v: s }))
  } else if (payload.kind === 'message') {
    wsPush('in', payload.frame, payload.text)
  } else {
    wsState.value = 'error'
    wsReconnecting.value = false
    wsLastError.value = payload.message ?? ''
    markConnHistory('ws', wsUrl.value.trim(), false)
    wsPush('sys', 'sys', t('realtime.failed', { v: payload.message }))
  }
}

// ---------- 连接状态徽章 / 统计（只用真实数据） ----------
const BADGE_TONE = {
  open: 'ok',
  connecting: 'run',
  closed: 'warn',
  error: 'fail',
  idle: 'info',
} as const

const activeBadge = computed<{ tone: 'ok' | 'fail' | 'warn' | 'info' | 'run'; label: string }>(() => {
  if (mainTab.value === 'ws') {
    return { tone: BADGE_TONE[wsState.value], label: wsStateText.value }
  }
  switch (sseStatus.value) {
    case 'open':
      return { tone: 'ok', label: t('realtime.sseActive') }
    case 'closed':
      return { tone: 'warn', label: t('realtime.sseDone') }
    case 'error':
      return { tone: 'fail', label: t('realtime.stateError') }
    default:
      return { tone: 'info', label: t('realtime.sseIdle') }
  }
})

const wsStateText = computed(() => {
  switch (wsState.value) {
    case 'open':
      return t('realtime.stateOpen')
    case 'connecting':
      // 错误后的重连（后端 auto_reconnect）单独提示，与首次连接区分
      return wsReconnecting.value ? t('realtime.stateReconnecting') : t('realtime.stateConnecting')
    case 'closed':
      return t('realtime.stateClosed')
    case 'error':
      return t('realtime.stateError')
    default:
      return t('realtime.stateIdle')
  }
})

const wsInCount = computed(() => wsLog.value.filter((m) => m.dir === 'in').length)
const wsOutCount = computed(() => wsLog.value.filter((m) => m.dir === 'out').length)
const sseInCount = computed(() => sseLog.value.filter((m) => m.dir === 'in').length)

/** 数据量（按字符数求和；binary 为其 base64 文本长度，故单位为 B 只作量级参考）。 */
function sizeFmt(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(2)} MB`
}
const wsSize = computed(() => sizeFmt(wsLog.value.reduce((acc, m) => acc + m.size, 0)))
const sseSize = computed(() => sizeFmt(sseLog.value.reduce((acc, m) => acc + m.size, 0)))

// ---------- SSE ----------
const sseUrl = ref(saved.sseUrl ?? 'http://127.0.0.1:4010/sse')
const sseConnId = ref<string | null>(null)
const sseStatus = ref<'idle' | 'open' | 'closed' | 'error'>('idle')
const sseConnecting = ref(false)
const sseBuffer = ref('')
const sseLastId = ref('')

const sseLog = ref<FrameItem[]>([])

function ssePush(dir: FrameItem['dir'], event: string, text: string, id: string): void {
  sseLog.value.push({
    seq: logSeq++,
    t: nowTime(),
    dir,
    kind: 'sse',
    text,
    size: text.length,
    meta: { event, id: id || undefined },
  })
  cap(sseLog.value)
}

/** SSE 帧解析：按空行切帧，data 行换行拼接，id 行更新续传位点。 */
// （sseUrl 的持久化监听放在 sseUrl 声明之后，避免 use-before-assign）
watch([sseUrl, sseHeaders], savePersist, { deep: true })
function parseSseFrames(): void {
  const normalized = sseBuffer.value.replace(/\r\n/g, '\n')
  const parts = normalized.split('\n\n')
  sseBuffer.value = parts.pop() ?? ''
  for (const frame of parts) {
    const dataLines: string[] = []
    let event = 'message'
    let id = ''
    for (const line of frame.split('\n')) {
      if (!line || line.startsWith(':')) continue
      const colon = line.indexOf(':')
      const field = (colon === -1 ? line : line.slice(0, colon)).trim()
      const value = (colon === -1 ? '' : line.slice(colon + 1)).replace(/^ /, '')
      if (field === 'event') event = value || 'message'
      else if (field === 'data') dataLines.push(value)
      else if (field === 'id') id = value
    }
    if (dataLines.length === 0) continue
    if (id) sseLastId.value = id
    ssePush('in', event, dataLines.join('\n'), id)
  }
}

async function sseConnect(): Promise<void> {
  if (!sseUrl.value.trim() || sseConnecting.value || sseConnId.value) return
  sseConnecting.value = true
  sseLastError.value = ''
  try {
    const id = await api.sseConnect({
      url: sseUrl.value.trim(),
      headers: toHeaderMap(sseHeaders.value),
      last_event_id: sseLastId.value || null,
    })
    sseConnId.value = id
    sseStatus.value = 'open'
    pushConnHistory('sse', sseUrl.value.trim(), true)
  } catch (err) {
    sseLastError.value = errorText(err)
    pushConnHistory('sse', sseUrl.value.trim(), false)
    toast.error(t('realtime.sseConnectFail'), { message: errorText(err) })
  } finally {
    sseConnecting.value = false
  }
}

async function sseDisconnect(silent = false): Promise<void> {
  if (!sseConnId.value) return
  const id = sseConnId.value
  sseConnId.value = null
  try {
    await api.sseDisconnect(id)
  } catch {
    // 忽略：本地状态照常清理
  }
  sseStatus.value = 'closed'
  if (!silent) ssePush('sys', 'sys', t('realtime.sseUnsubscribed'), '')
}

function onSseEvent(payload: SseEventPayload): void {
  if (payload.connection_id !== sseConnId.value) return
  if (payload.kind === 'open') {
    sseStatus.value = 'open'
    ssePush('sys', 'sys', t('realtime.sseOpened'), '')
  } else if (payload.kind === 'chunk') {
    sseBuffer.value += payload.chunk
    // 单块超 1MB 未成帧：截断防内存膨胀（畸形流保护）。
    if (sseBuffer.value.length > 1_000_000) sseBuffer.value = sseBuffer.value.slice(-200_000)
    parseSseFrames()
  } else if (payload.kind === 'error') {
    sseStatus.value = 'error'
    sseLastError.value = payload.message
    ssePush('sys', 'sys', t('realtime.sseError', { v: payload.message }), '')
  } else {
    sseStatus.value = 'closed'
    sseConnId.value = null
    ssePush('sys', 'sys', t('realtime.sseClosedByServer'), '')
  }
  cap(sseLog.value)
}

// ---------- 连接状态徽章 / 统计（只用真实数据） ----------
/** 连接时长：open 起点的本地秒表（mm:ss），断开即冻结在最后读数。 */
const elapsedTick = ref(0)
let elapsedTimer: number | null = null
const wsOpenedAt = ref<number | null>(null)
const sseOpenedAt = ref<number | null>(null)
watch(wsState, (s) => {
  if (s === 'open' && wsOpenedAt.value === null) wsOpenedAt.value = Date.now()
  if (s !== 'open') wsOpenedAt.value = null
})
watch(
  () => sseStatus.value,
  (s) => {
    if (s === 'open' && sseOpenedAt.value === null) sseOpenedAt.value = Date.now()
    if (s !== 'open') sseOpenedAt.value = null
  },
)
function fmtElapsed(since: number | null): string {
  void elapsedTick.value
  if (since === null) return '—'
  const sec = Math.max(0, Math.floor((Date.now() - since) / 1000))
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`
}
const wsElapsed = computed(() => fmtElapsed(wsOpenedAt.value))
const sseElapsed = computed(() => fmtElapsed(sseOpenedAt.value))
watch(
  () => [wsState.value === 'open', sseStatus.value === 'open'],
  ([wsOpen, sseOpen]) => {
    const need = wsOpen || sseOpen
    if (need && elapsedTimer === null) {
      elapsedTimer = window.setInterval(() => {
        elapsedTick.value++
      }, 1000)
    } else if (!need && elapsedTimer !== null) {
      window.clearInterval(elapsedTimer)
      elapsedTimer = null
    }
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  if (elapsedTimer !== null) window.clearInterval(elapsedTimer)
})

// ---------- 事件订阅（scope 卸载自动解绑 + 晚到竞态兜底，见 useTauriEvent） ----------
if ('__TAURI_INTERNALS__' in window) {
  useTauriEvent<WsEventPayload>('miyo:ws-event', onWsEvent)
  useTauriEvent<SseEventPayload>('miyo:sse-event', onSseEvent)
}

// ---------- 帧详情（SplitPanes 右栏） ----------
const selectedSeq = ref<number | null>(null)
watch(mainTab, () => {
  selectedSeq.value = null
})
function clearLog(): void {
  if (mainTab.value === 'ws') wsLog.value = []
  else sseLog.value = []
  selectedSeq.value = null
}

/** 导出当前日志：JSON（全字段）或 TXT（时间线文本），经目录选择落盘。 */
async function onExportLog(format: 'json' | 'txt'): Promise<void> {
  const log = activeLog.value
  if (!log.length) return
  try {
    const dir = await openDialog({ directory: true, title: t('realtime.exportLog') })
    if (!dir || Array.isArray(dir)) return
    const base = `miyo-realtime-${mainTab.value}-${new Date().toISOString().replace(/[:.]/g, '-')}`
    const path = await api.joinPath(dir, `${base}.${format}`)
    const content =
      format === 'json'
        ? JSON.stringify(log, null, 2)
        : log.map((m) => `[${m.t}] ${m.dir}/${m.kind} (${m.size}) ${m.text}`).join('\n')
    await api.writeTextFile(path, content)
    toast.success(t('realtime.exported'), { message: path })
  } catch (err) {
    toast.error(t('realtime.exportFail'), { message: errorText(err) })
  }
}
const activeLog = computed<FrameItem[]>(() => (mainTab.value === 'ws' ? wsLog.value : sseLog.value))
const selectedFrame = computed<FrameItem | null>(
  () => activeLog.value.find((m) => m.seq === selectedSeq.value) ?? null,
)

// ---------- 帧详情（Pretty / Raw / Info 三页签） ----------
const detailTab = ref('pretty')
const DETAIL_TABS = computed<TabItem[]>(() => [
  { key: 'pretty', label: t('realtime.detailTabPretty') },
  { key: 'raw', label: t('realtime.detailTabRaw') },
  { key: 'info', label: t('realtime.detailTabInfo') },
])
watch(selectedSeq, () => {
  detailTab.value = 'pretty'
})

/** WS 帧 opcode / 帧类别（真实协议语义，按 kind 推导）。 */
function opcodeOf(f: FrameItem): string | null {
  if (mainTab.value !== 'ws') return null
  if (f.kind === 'text') return '0x1'
  if (f.kind === 'binary') return '0x2'
  if (f.kind === 'ping') return '0x9'
  return null
}
function frameTypeLabel(f: FrameItem): string {
  if (mainTab.value === 'sse') return 'SSE'
  if (f.kind === 'ping') return t('realtime.frameControl')
  if (f.kind === 'sys') return '—'
  return t('realtime.frameData')
}

/** binary 帧的 base64 解码预览（非法 base64 回退原文，不炸 UI）。 */
const decodedText = computed<string | null>(() => {
  const f = selectedFrame.value
  if (!f || f.kind !== 'binary') return null
  return fromBase64Utf8Safe(f.text)
})

/** 详情 JSON 解析（JSON → JsonTree；失败按纯文本展示）。 */
const detailJson = computed<unknown | null>(() => {
  const f = selectedFrame.value
  if (!f) return null
  const raw = f.kind === 'binary' ? (decodedText.value ?? '') : f.text
  if (!raw) return null
  try {
    return JSON.parse(raw) as unknown
  } catch {
    return null
  }
})

function selectFrame(item: FrameItem): void {
  selectedSeq.value = selectedSeq.value === item.seq ? null : item.seq
}

function copyDetail(): void {
  const f = selectedFrame.value
  if (f) void navigator.clipboard.writeText(f.text).catch(() => {})
}

onMounted(() => {
  void detectWindow()
})

onUnmounted(() => {
  // 视图卸载即断开：后台会话随之释放，避免泄漏。
  void wsDisconnect(true)
  void sseDisconnect(true)
})

// 测试挂载点（spec 驱动连接状态机 / 契约回归）：不进模板、无运行时开销
defineExpose({
  wsLog,
  sendHistory,
  wsSend,
  /** spec 注点：绕过 IPC 事件直接驱动连接状态机。 */
  __testSetConn(state: 'idle' | 'connecting' | 'open' | 'closed' | 'error', connId: string | null): void {
    wsConnId.value = connId
    wsState.value = state
  },
})
</script>

<template>
  <div class="rt-root">
    <!-- 顶栏：PageHeader 统一导航（macOS 交通灯落 Brand 区上方）；
         弹出子窗（非主窗口）无 Brand / 返回语境，仅保留工作台上下文与状态 -->
    <PageHeader
      :title="t('realtime.windowTitle')"
      :hide-brand="!isMainWindow"
      :hide-back="!isMainWindow"
      @back="goBack"
    >
      <SegmentedControl v-model="mainTab" :options="MAIN_TABS" size="sm" class="rt-seg" />
      <template #actions>
        <IconButton
          v-if="isMainWindow"
          name="monitor"
          :size="14"
          :title="t('realtime.popoutHint')"
          @click="popout"
        />
        <div class="rt-status-wrap">
          <Spinner v-if="wsConnecting || sseConnecting" />
          <StatusBadge :tone="activeBadge.tone" :label="activeBadge.label" :dot="activeBadge.tone === 'run'" />
        </div>
      </template>
    </PageHeader>

    <div class="rt-body">
    <!-- 连接条：URL + 动作 + 状态 / 耗时 / 收发统计 + 连接历史 -->
    <div class="rt-conn">
      <input
        v-if="mainTab === 'ws'"
        v-model="wsUrl"
        class="miyo-input rt-url"
        placeholder="ws://127.0.0.1:4010/socket"
        spellcheck="false"
        :disabled="!!wsConnId"
      />
      <input
        v-else
        v-model="sseUrl"
        class="miyo-input rt-url"
        placeholder="http://127.0.0.1:4010/events"
        spellcheck="false"
        :disabled="!!sseConnId"
      />
      <div class="rt-conn-actions">
        <button
          v-if="mainTab === 'ws' && !wsConnId"
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="wsConnecting || !wsUrl.trim()"
          @click="wsConnect"
        >
          {{ wsConnecting ? t('realtime.connectingShort') : t('realtime.connect') }}
        </button>
        <button
          v-else-if="mainTab === 'ws'"
          class="miyo-btn miyo-btn-sm miyo-btn-danger"
          type="button"
          @click="wsDisconnect()"
        >
          {{ t('realtime.disconnect') }}
        </button>
        <button
          v-else-if="!sseConnId"
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="sseConnecting || !sseUrl.trim()"
          @click="sseConnect"
        >
          {{ sseConnecting ? t('realtime.subscribing') : t('realtime.subscribe') }}
        </button>
        <button v-else class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="sseDisconnect()">
          {{ t('realtime.unsubscribe') }}
        </button>
        <IconButton
          name="history"
          :size="14"
          :title="t('realtime.historyHint')"
          @click="openHistoryMenu"
        />
        <Menu ref="historyMenuRef" @select="onHistorySelect" />
      </div>
      <!-- 实时统计（真实数据：秒表 + 收发条数 + 字节量；SSE 另有续传位点）。
           状态徽章在顶栏 rt-status-wrap，此处只放数字，避免重复。
           未连接时只显示收发计数，不做「已连接 —」这种自相矛盾的展示。 -->
      <div class="rt-conn-info">
        <template v-if="mainTab === 'ws' ? wsConnId : sseConnId">
          <span class="rt-stat num" :class="{ on: (mainTab === 'ws' ? wsState : sseStatus) === 'open' }">
            {{ t('realtime.statElapsed', { v: mainTab === 'ws' ? wsElapsed : sseElapsed }) }}
          </span>
          <span class="rt-sep">·</span>
        </template>
        <template v-if="mainTab === 'ws'">
          <span class="rt-stat stat-in num">{{ t('realtime.statIn', { n: wsInCount }) }}</span>
          <span class="rt-stat stat-out num">{{ t('realtime.statOut', { n: wsOutCount }) }}</span>
          <span class="rt-stat num">{{ t('realtime.statBytes', { v: wsSize }) }}</span>
        </template>
        <template v-else>
          <span class="rt-stat stat-in num">{{ t('realtime.statIn', { n: sseInCount }) }}</span>
          <span class="rt-stat num">{{ t('realtime.statBytes', { v: sseSize }) }}</span>
          <span v-if="sseLastId" class="rt-stat num">{{ t('realtime.lastEventId', { v: sseLastId }) }}</span>
        </template>
      </div>
    </div>

    <!-- 连接失败原因行（错误态才出现：具体原因 + 恢复建议） -->
    <div
      v-if="(mainTab === 'ws' && wsLastError) || (mainTab === 'sse' && sseLastError)"
      class="rt-conn-error"
      role="alert"
    >
      <Icon name="alert-triangle" :size="13" />
      <span class="rt-conn-error-text">{{ mainTab === 'ws' ? wsLastError : sseLastError }}</span>
      <span class="rt-conn-error-hint">{{ t('realtime.errorRecoverHint') }}</span>
    </div>

    <!-- 高级连接参数（请求头 / WS 子协议）：全宽折叠区 -->
    <CollapsibleSection
      :title="t('realtime.advanced')"
      :count="mainTab === 'ws' ? headerCount(wsHeaders) : headerCount(sseHeaders)"
      :open="mainTab === 'ws' ? wsHeadersOpen : sseHeadersOpen"
      dense
      class="rt-adv"
      @update:open="(v: boolean) => (mainTab === 'ws' ? (wsHeadersOpen = v) : (sseHeadersOpen = v))"
    >
      <template #actions>
        <label v-if="mainTab === 'ws'" class="rt-switch" @click.stop>
          <span>{{ t('realtime.autoReconnect') }}</span>
          <input v-model="wsAutoReconnect" type="checkbox" :disabled="!!wsConnId" />
        </label>
      </template>
      <KeyValueTable
        v-if="mainTab === 'ws'"
        v-model="wsHeaders"
        :show-description="false"
        :disabled="!!wsConnId"
      />
      <KeyValueTable
        v-else
        v-model="sseHeaders"
        :show-description="false"
        :disabled="!!sseConnId"
      />
      <label v-if="mainTab === 'ws'" class="rt-sub">
        <span class="rt-sub-label">{{ t('realtime.subprotocolsLabel') }}</span>
        <input
          v-model="wsSubprotocols"
          class="miyo-input miyo-input-sm rt-sub-input"
          :placeholder="t('realtime.subprotocolsPh')"
          :title="t('realtime.subprotocolsHint')"
          :disabled="!!wsConnId"
          spellcheck="false"
        />
      </label>
    </CollapsibleSection>

    <!-- 统计条已并入连接条（rt-conn-info），不再单独占一行 -->

    <!-- 帧日志 | 详情 分栏（窄容器自动上下堆叠） -->
    <SplitPanes
      v-model:width="splitWidth"
      class="rt-split"
      :min="360"
      :max="1100"
      :stack-below="880"
      @reset="splitWidth = 560"
    >
      <template #first>
        <FrameLog
          :items="activeLog"
          :selected-seq="selectedSeq"
          :empty-title="mainTab === 'ws' ? t('realtime.wsEmpty') : t('realtime.sseEmpty')"
          :reset-key="mainTab"
          @select="selectFrame"
          @clear="clearLog"
          @export="onExportLog"
        />
      </template>
      <template #second>
        <div class="rt-detail">
          <template v-if="selectedFrame">
            <div class="rt-detail-head">
              <Tabs v-model="detailTab" :tabs="DETAIL_TABS" size="sm" />
              <IconButton
                class="rt-detail-copy"
                name="copy"
                :size="13"
                :title="t('common.copyAction')"
                @click="copyDetail"
              />
            </div>
            <!-- Pretty：JSON → 树；其余 → 原文（binary 给解码预览） -->
            <div v-if="detailTab === 'pretty'" class="rt-detail-body">
              <JsonTree v-if="detailJson !== null" :data="detailJson" class="rt-detail-tree" />
              <pre v-else class="rt-detail-text">{{ decodedText ?? selectedFrame.text }}</pre>
            </div>
            <!-- Raw：原始内容（binary 为 base64 原文） -->
            <pre v-else-if="detailTab === 'raw'" class="rt-detail-text">{{ selectedFrame.text }}</pre>
            <!-- Info：帧元信息（方向 / 类型 / opcode / 序号 / 大小；SSE event / id） -->
            <dl v-else class="rt-detail-info">
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailDir') }}</dt>
                <dd>
                  {{ selectedFrame.dir === 'in' ? t('realtime.dirIn') : selectedFrame.dir === 'out' ? t('realtime.dirOut') : t('realtime.dirSys') }}
                </dd>
              </div>
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailKind') }}</dt>
                <dd>{{ selectedFrame.kind }}</dd>
              </div>
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailFrameType') }}</dt>
                <dd>{{ frameTypeLabel(selectedFrame) }}</dd>
              </div>
              <div v-if="opcodeOf(selectedFrame)" class="rt-info-row">
                <dt>{{ t('realtime.detailOpcode') }}</dt>
                <dd class="num">{{ opcodeOf(selectedFrame) }}</dd>
              </div>
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailSeq') }}</dt>
                <dd class="num">#{{ selectedFrame.seq }}</dd>
              </div>
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailTime') }}</dt>
                <dd class="num">{{ selectedFrame.t }}</dd>
              </div>
              <div class="rt-info-row">
                <dt>{{ t('realtime.detailChars') }}</dt>
                <dd class="num">{{ selectedFrame.size }}</dd>
              </div>
              <template v-if="selectedFrame.meta">
                <div v-if="selectedFrame.meta.event" class="rt-info-row">
                  <dt>event</dt>
                  <dd>{{ selectedFrame.meta.event }}</dd>
                </div>
                <div v-if="selectedFrame.meta.id" class="rt-info-row">
                  <dt>id</dt>
                  <dd class="num">{{ selectedFrame.meta.id }}</dd>
                </div>
              </template>
            </dl>
          </template>
          <div v-else class="rt-detail-empty">
            <Icon name="terminal" :size="18" />
            <p>{{ t('realtime.detailEmpty') }}</p>
          </div>
        </div>
      </template>
    </SplitPanes>

    <!-- 发送区（WS 专属；SSE 为服务端推送协议，无客户端发送）：帧类型 + 编辑器 + 模板 + 发送 -->
    <div v-if="mainTab === 'ws'" class="rt-send" @keydown.capture="onSendCaptureKey">
      <SegmentedControl
        v-model="sendKind"
        :options="SEND_MODES"
        size="sm"
        class="rt-send-kind"
      />
      <div class="rt-send-editor">
        <div v-if="sendKind === 'json'" class="rt-send-cm">
          <JsonCodeMirror
            v-model="wsSendText"
            language="json"
            :placeholder-text="t('realtime.sendPhJson')"
          />
        </div>
        <input
          v-else
          v-model="wsSendText"
          class="miyo-input rt-send-input"
          :placeholder="sendKind === 'binary' ? t('realtime.sendPhBinary') : t('realtime.sendPh')"
          spellcheck="false"
          :title="t('realtime.sendHistoryHint')"
          :disabled="!wsConnId || wsState !== 'open'"
          @keydown.enter="sendCurrent()"
          @keydown.up="onSendKeydown"
          @keydown.down="onSendKeydown"
        />
        <!-- JSON 校验提示（不阻断发送） -->
        <p v-if="jsonInvalid" class="rt-send-warn">
          <Icon name="alert-triangle" :size="12" /> {{ t('realtime.jsonInvalid') }}
        </p>
        <!-- 发送失败行内详情（toast 同步弹出） -->
        <p v-if="sendError" class="rt-send-error" role="alert">{{ sendError }}</p>
      </div>
      <IconButton
        name="star"
        :size="14"
        :title="t('realtime.templateMenuHint')"
        :disabled="!templates.length && !wsSendText"
        @click="openTemplateMenu"
      />
      <Menu ref="templateMenuRef" @select="onTemplateSelect" />
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-ghost"
        type="button"
        :disabled="sending || !wsConnId || wsState !== 'open' || !wsSendText"
        :title="t('realtime.pingHint')"
        @click="wsSend('ping')"
      >
        Ping
      </button>
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-primary"
        type="button"
        :disabled="sending || !wsConnId || wsState !== 'open' || !wsSendText"
        @click="sendCurrent()"
      >
        <Spinner v-if="sending" />
        <Icon v-else name="send" :size="13" />
        {{ t('editor.send') }}
      </button>
    </div>
    </div>
  </div>
</template>

<style scoped>
.rt-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

/* 顶栏之下主内容：原根容器内距与间距移入 body（顶栏通栏到底） */
.rt-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 14px 12px;
}
.rt-seg {
  margin-left: 8px;
}
.rt-status-wrap {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

/* ---- 连接条 ---- */
.rt-conn {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.rt-url {
  flex: 1;
  min-width: 200px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}
.rt-adv {
  flex-shrink: 0;
}
.rt-conn-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
/* 状态 / 统计簇：徽章 + 秒表 + 收发 + 字节量（真实数据，未连接时显示 0 与 —） */
.rt-conn-info {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  padding-left: 10px;
  border-left: 1px solid var(--border);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  color: var(--text-3);
  white-space: nowrap;
}
.rt-stat.on {
  color: var(--success);
}
.stat-in {
  color: var(--success);
}
.stat-out {
  color: var(--info);
}

/* 连接失败原因行：具体错误 + 恢复建议 */
.rt-conn-error {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  padding: 5px 10px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
  border-radius: var(--radius);
  background: var(--danger-tint);
  color: var(--danger);
  font-size: var(--fs-xs);
}
.rt-conn-error-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rt-conn-error-hint {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

/* 自动重连开关（自绘，Token 化） */
.rt-switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
}
.rt-switch input {
  appearance: none;
  width: 26px;
  height: 15px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  border: 1px solid var(--border-strong);
  position: relative;
  cursor: pointer;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
  flex-shrink: 0;
}
.rt-switch input::after {
  content: '';
  position: absolute;
  top: 1px;
  left: 1px;
  width: 11px;
  height: 11px;
  border-radius: var(--radius-pill);
  background: var(--text-3);
  transition: transform var(--dur) var(--ease), background var(--dur) var(--ease);
}
.rt-switch input:checked {
  background: var(--accent-tint);
  border-color: var(--accent);
}
.rt-switch input:checked::after {
  transform: translateX(11px);
  background: var(--accent);
}
.rt-switch input:disabled {
  opacity: 0.5;
  cursor: default;
}

/* 折叠区里的子协议行 */
.rt-sub {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}
.rt-sub-label {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.rt-sub-input {
  flex: 1;
  min-width: 0;
  max-width: 360px;
}

/* ---- 统计（已并入连接条 rt-conn-info，样式保留语义色） ---- */
.rt-sep {
  color: var(--border-strong);
}
.stat-in {
  color: var(--success);
}
.stat-out {
  color: var(--info);
}

/* ---- 分栏 ---- */
.rt-split {
  flex: 1;
  min-height: 0;
}

/* 详情栏 */
.rt-detail {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  overflow: hidden;
}
.rt-detail-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px 0;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border);
}
.rt-detail-copy {
  flex: none;
  margin-left: auto;
  margin-bottom: 6px;
}
/* Info 页签：键值行（dt 固定宽，dd 等宽数据） */
.rt-detail-info {
  flex: 1;
  min-height: 0;
  overflow: auto;
  margin: 0;
  padding: 8px 10px 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.rt-info-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 3px 6px;
  border-radius: var(--radius-sm);
}
.rt-info-row:nth-child(odd) {
  background: var(--bg-hover);
}
.rt-info-row dt {
  flex: none;
  width: 96px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.rt-info-row dd {
  margin: 0;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-1);
  word-break: break-all;
}
.rt-detail-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 4px 10px 10px;
}
.rt-detail-tree {
  font-size: var(--fs-xs);
}
.rt-detail-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-1);
}
.rt-detail-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-3);
}
.rt-detail-empty p {
  margin: 0;
  font-size: var(--fs-xs);
}

/* ---- 发送区 ---- */
.rt-send {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  flex-shrink: 0;
}
.rt-send-kind {
  flex-shrink: 0;
  margin-top: 1px;
}
.rt-send-editor {
  flex: 1;
  min-width: 120px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
/* JSON 模式：CodeMirror 编辑器（多行 JSON 编辑） */
.rt-send-cm {
  height: 110px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius);
  overflow: hidden;
}
.rt-send-input {
  width: 100%;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}
.rt-send-warn,
.rt-send-error {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xxs);
}
.rt-send-warn {
  color: var(--warning);
}
.rt-send-error {
  color: var(--danger);
  word-break: break-all;
}
/* 发送钮与模板钮对齐编辑器首行 */
.rt-send > .ib,
.rt-send > .miyo-btn {
  margin-top: 1px;
  flex-shrink: 0;
}
</style>
