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
import { useRouter } from 'vue-router'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useTauriEvent } from '@/composables/useTauriEvent'
import { useLocaleStore } from '@/stores/locale'
import { formatTime } from '@/utils/dateTime'
import { toBase64Utf8, fromBase64Utf8Safe } from '@/utils/base64'
import Brand from '@/domains/workspace/components/Brand.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import JsonTree from '@/components/ui/JsonTree.vue'
import KeyValueTable, { type KVRow } from '@/components/ui/KeyValueTable.vue'
import SegmentedControl, { type SegmentOption } from '@/components/ui/SegmentedControl.vue'
import Spinner from '@/components/ui/Spinner.vue'
import SplitPanes from '@/components/ui/SplitPanes.vue'
import StatusBadge from '@/components/ui/StatusBadge.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import CollapsibleSection from '@/components/ui/CollapsibleSection.vue'
import FrameLog, { type FrameItem } from '@/domains/endpoint/components/realtime/realtime/FrameLog.vue'
import type { SseEventPayload, WsEventPayload } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const router = useRouter()
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
 * - 会话内有可回退历史 → `router.back()`，原路返回调用方页面；
 * - 直接深链打开（浅历史，回退会离开应用）→ 回仪表板兜底。
 * `history.state.back` 是 vue-router 写入的历史标识：顶层页面为 null。
 */
function goBack(): void {
  if (window.history.state?.back) router.back()
  else void router.push('/projects')
}

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

async function wsConnect(): Promise<void> {
  if (!wsUrl.value.trim() || wsConnecting.value || wsConnId.value) return
  wsConnecting.value = true
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
  } catch (err) {
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

const sendKind = ref<'text' | 'binary'>('text')
const wsSendText = ref('')
/** 发送历史（原文，最近 20 条；↑/↓ 回溯）。 */
const sendHistory = ref<string[]>([])
const historyIdx = ref(-1)

/**
 * 发送帧：text 帧发原文；binary / ping 按后端 WsSendArgs 契约先编码为
 * base64（此前 Ping 直传原文会被后端按 base64 误解码——契约修复）。
 */
async function wsSend(frame: 'text' | 'binary' | 'ping'): Promise<void> {
  if (!wsConnId.value || !wsSendText.value) return
  const raw = wsSendText.value
  const payload = frame === 'text' ? raw : toBase64Utf8(raw)
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
    toast.error(t('realtime.sendFail'), { message: errorText(err) })
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
    wsState.value =
      s === 'open' ? 'open' : s === 'connecting' ? 'connecting' : s === 'closed' ? 'closed' : 'error'
    wsPush('sys', 'sys', t('realtime.stateChanged', { v: s }))
  } else if (payload.kind === 'message') {
    wsPush('in', payload.frame, payload.text)
  } else {
    wsState.value = 'error'
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
      return t('realtime.stateConnecting')
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
  try {
    const id = await api.sseConnect({
      url: sseUrl.value.trim(),
      headers: toHeaderMap(sseHeaders.value),
      last_event_id: sseLastId.value || null,
    })
    sseConnId.value = id
    sseStatus.value = 'open'
  } catch (err) {
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

const sseInCount = computed(() => sseLog.value.filter((m) => m.dir === 'in').length)

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
const activeLog = computed<FrameItem[]>(() => (mainTab.value === 'ws' ? wsLog.value : sseLog.value))
const selectedFrame = computed<FrameItem | null>(
  () => activeLog.value.find((m) => m.seq === selectedSeq.value) ?? null,
)

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
    <!-- 顶栏：macOS 交通灯落在品牌区上方（同 WorkspaceView / IntegrationView 惯例）；
         整行 data-tauri-drag-region 可拖动窗口，交互子元素不受影响 -->
    <div class="rt-top" data-tauri-drag-region>
      <Brand v-if="isMainWindow" :title="t('realtime.windowTitle')" class="rt-brand" />
      <template v-if="isMainWindow">
        <IconButton
          class="rt-back"
          name="chevron-right"
          :size="14"
          :title="t('realtime.backHint')"
          @click="goBack"
        />
        <IconButton
          name="monitor"
          :size="14"
          :title="t('realtime.popoutHint')"
          @click="popout"
        />
      </template>
      <SegmentedControl v-model="mainTab" :options="MAIN_TABS" size="sm" class="rt-seg" />
      <div class="rt-status-wrap">
        <Spinner v-if="wsConnecting || sseConnecting" />
        <StatusBadge :tone="activeBadge.tone" :label="activeBadge.label" :dot="activeBadge.tone === 'run'" />
      </div>
    </div>

    <!-- 连接条 -->
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
        <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" type="button" @click="clearLog">
          {{ t('realtime.clearLog') }}
        </button>
      </div>
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

    <!-- 统计条（连接中显示；全部来自真实状态，无虚构指标） -->
    <div v-if="mainTab === 'ws' && wsConnId" class="rt-stats num">
      <span :class="{ on: wsState === 'open' }">{{ t('realtime.statElapsed', { v: wsElapsed }) }}</span>
      <span class="rt-sep">·</span>
      <span class="stat-in">{{ t('realtime.statIn', { n: wsInCount }) }}</span>
      <span class="rt-sep">·</span>
      <span class="stat-out">{{ t('realtime.statOut', { n: wsOutCount }) }}</span>
    </div>
    <div v-else-if="mainTab === 'sse' && sseConnId" class="rt-stats num">
      <span :class="{ on: sseStatus === 'open' }">{{ t('realtime.statElapsed', { v: sseElapsed }) }}</span>
      <span class="rt-sep">·</span>
      <span class="stat-in">{{ t('realtime.statIn', { n: sseInCount }) }}</span>
      <template v-if="sseLastId">
        <span class="rt-sep">·</span>
        <span>{{ t('realtime.lastEventId', { v: sseLastId }) }}</span>
      </template>
    </div>

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
          @select="selectFrame"
        />
      </template>
      <template #second>
        <div class="rt-detail">
          <template v-if="selectedFrame">
            <div class="rt-detail-head">
              <span class="rt-detail-title">{{ t('realtime.detailTime') }} {{ selectedFrame.t }}</span>
              <span class="rt-detail-meta">
                {{ t('realtime.detailDir') }}
                {{ selectedFrame.dir === 'in' ? t('realtime.dirIn') : selectedFrame.dir === 'out' ? t('realtime.dirOut') : t('realtime.dirSys') }}
                · {{ t('realtime.detailKind') }} {{ selectedFrame.kind }}
                · {{ t('realtime.detailChars', { n: selectedFrame.size }) }}
              </span>
              <IconButton
                class="rt-detail-copy"
                name="copy"
                :size="13"
                :title="t('common.copyAction')"
                @click="copyDetail"
              />
            </div>
            <p v-if="selectedFrame.kind === 'binary'" class="rt-detail-hint">
              {{ t('realtime.detailBinary') }}
            </p>
            <div class="rt-detail-body">
              <JsonTree v-if="detailJson !== null" :data="detailJson" class="rt-detail-tree" />
              <pre v-else class="rt-detail-text">{{ decodedText ?? selectedFrame.text }}</pre>
            </div>
          </template>
          <div v-else class="rt-detail-empty">
            <Icon name="terminal" :size="18" />
            <p>{{ t('realtime.detailEmpty') }}</p>
          </div>
        </div>
      </template>
    </SplitPanes>

    <!-- 发送条（WS 专属）：帧类型 + 输入（↑/↓ 历史）+ Ping + 发送 -->
    <div v-if="mainTab === 'ws'" class="rt-send">
      <CustomSelect
        v-model="sendKind"
        class="rt-send-kind"
        :options="[
          { value: 'text', label: t('realtime.frameText') },
          { value: 'binary', label: t('realtime.frameBinary') },
        ]"
        size="sm"
        :pop-min-width="150"
        :disabled="!wsConnId || wsState !== 'open'"
      />
      <input
        v-model="wsSendText"
        class="miyo-input rt-send-input"
        :placeholder="sendKind === 'binary' ? t('realtime.sendPhBinary') : t('realtime.sendPh')"
        spellcheck="false"
        :title="t('realtime.sendHistoryHint')"
        :disabled="!wsConnId || wsState !== 'open'"
        @keydown.enter="wsSend(sendKind)"
        @keydown.up="onSendKeydown"
        @keydown.down="onSendKeydown"
      />
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-ghost"
        type="button"
        :disabled="!wsConnId || wsState !== 'open' || !wsSendText"
        :title="t('realtime.pingHint')"
        @click="wsSend('ping')"
      >
        Ping
      </button>
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-primary"
        type="button"
        :disabled="!wsConnId || wsState !== 'open' || !wsSendText"
        @click="wsSend(sendKind)"
      >
        <Icon name="send" :size="13" /> {{ t('editor.send') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.rt-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 10px 14px 12px;
  gap: 8px;
  min-height: 0;
}

/* ---- 顶栏 ---- */
.rt-top {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.rt-brand {
  margin-right: 4px;
}
/* 返回钮：chevron-right 旋转 180° 指向左（仓库内既有做法） */
.rt-back :deep(svg) {
  transform: rotate(180deg);
}
.rt-seg {
  margin-left: 8px;
}
.rt-status-wrap {
  margin-left: auto;
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
  margin-left: auto;
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

/* ---- 统计条 ---- */
.rt-stats {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.rt-stats .on {
  color: var(--success);
}
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
  padding: 8px 10px 0;
  flex-shrink: 0;
}
.rt-detail-title {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}
.rt-detail-meta {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.rt-detail-copy {
  flex: none;
}
.rt-detail-hint {
  margin: 0;
  padding: 0 10px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  flex-shrink: 0;
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

/* ---- 发送条 ---- */
.rt-send {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.rt-send-kind {
  width: 150px;
  flex-shrink: 0;
}
.rt-send-input {
  flex: 1;
  min-width: 120px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}
</style>
