<script setup lang="ts">
/**
 * RealtimeView：实时调试视图（WebSocket / SSE）。
 *
 * - WebSocket：后端 `miyo-http::ws_client` 早已完备（自动重连/心跳/离线补发），
 *   本视图经新增的 ws_* 命令接入：建连 → 收发帧 → 断开，事件经 `miyo:ws-event` 推送；
 * - SSE：`sse_connect` 拉流转发原始文本块，前端按帧解析（event/data/id），事件经
 *   `miyo:sse-event` 推送；断线续传带 `Last-Event-ID`。
 * - 消息日志上限 500 条（超限丢弃最旧）；卸载视图时自动断开，避免后台泄漏。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { formatTime } from '../utils/dateTime'
import Icon from '../components/ui/Icon.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import KeyValueTable, { type KVRow } from '../components/ui/KeyValueTable.vue'
import Tabs, { type TabItem } from '../components/ui/Tabs.vue'
import type { SseEventPayload, WsEventPayload } from '../types/miyoApi'

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
    toast.error(t('realtime.popoutFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

type MainTab = 'ws' | 'sse'
const mainTab = ref<MainTab>('ws')
const MAIN_TABS: TabItem[] = [
  { key: 'ws', label: 'WebSocket' },
  { key: 'sse', label: 'SSE' },
]

const MAX_LOG = 500
function cap<T>(arr: T[]): T[] {
  if (arr.length > MAX_LOG) arr.splice(0, arr.length - MAX_LOG)
  return arr
}
/**
 * 日志行单调 id：头部裁剪（cap）会使后续所有行的索引整体前移，
 * 若 v-for 用 index 作 key，列表满后每来一条消息 500 行全部重新 patch；
 * 稳定 key 下只卸载被裁掉的行。popout 新窗口各自从 0 计数，无跨窗共享。
 */
let logSeq = 0
function nowTime(): string {
  return formatTime(new Date(), { hour12: false })
}

// ---------- WebSocket ----------
const wsUrl = ref('ws://127.0.0.1:4010')
const wsAutoReconnect = ref(true)
const wsConnId = ref<string | null>(null)
const wsState = ref<'idle' | 'connecting' | 'open' | 'closed' | 'error'>('idle')
const wsConnecting = ref(false)

/**
 * 连接级自定义请求头 / WS 子协议：与 URL 同为组件本地状态（RealtimeView
 * 现有状态组织方式即本地 ref，不落库）——鉴权头是 WS/SSE 的真实需求，
 * 后端 ws_connect / sse_connect 早已接受 headers / subprotocols。
 */
const wsHeadersOpen = ref(false)
const wsHeaders = ref<KVRow[]>([])
const wsSubprotocols = ref('')
const sseHeadersOpen = ref(false)
const sseHeaders = ref<KVRow[]>([])

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

interface WsLogItem {
  seq: number
  t: string
  dir: 'in' | 'out' | 'sys'
  frame: string
  text: string
}
const wsLog = ref<WsLogItem[]>([])

function wsPush(dir: WsLogItem['dir'], frame: string, text: string): void {
  wsLog.value.push({ seq: logSeq++, t: nowTime(), dir, frame, text })
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
    toast.error(t('realtime.wsConnectFail'), { message: err instanceof Error ? err.message : String(err) })
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

const wsSendText = ref('')
async function wsSend(frame: 'text' | 'ping'): Promise<void> {
  if (!wsConnId.value || !wsSendText.value) return
  const payload = wsSendText.value
  try {
    await api.wsSend({ connection_id: wsConnId.value, frame, payload })
    wsPush('out', frame, payload)
    wsSendText.value = ''
  } catch (err) {
    toast.error(t('realtime.sendFail'), { message: err instanceof Error ? err.message : String(err) })
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

// ---------- SSE ----------
const sseUrl = ref('http://127.0.0.1:4010/sse')
const sseConnId = ref<string | null>(null)
const sseStatus = ref<'idle' | 'open' | 'closed' | 'error'>('idle')
const sseConnecting = ref(false)
const sseBuffer = ref('')
const sseLastId = ref('')

interface SseLogItem {
  seq: number
  t: string
  event: string
  data: string
  /** SSE `id:` 行的事件 id（续传位点），非列表 key。 */
  id: string
}
const sseLog = ref<SseLogItem[]>([])

/** SSE 帧解析：按空行切帧，data 行换行拼接，id 行更新续传位点。 */
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
    sseLog.value.push({ seq: logSeq++, t: nowTime(), event, data: dataLines.join('\n'), id })
    cap(sseLog.value)
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
    toast.error(t('realtime.sseConnectFail'), { message: err instanceof Error ? err.message : String(err) })
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
  if (!silent) {
    sseLog.value.push({ seq: logSeq++, t: nowTime(), event: 'sys', data: t('realtime.sseUnsubscribed'), id: '' })
    cap(sseLog.value)
  }
}

function onSseEvent(payload: SseEventPayload): void {
  if (payload.connection_id !== sseConnId.value) return
  if (payload.kind === 'open') {
    sseStatus.value = 'open'
    sseLog.value.push({ seq: logSeq++, t: nowTime(), event: 'sys', data: t('realtime.sseOpened'), id: '' })
  } else if (payload.kind === 'chunk') {
    sseBuffer.value += payload.chunk
    // 单块超 1MB 未成帧：截断防内存膨胀（畸形流保护）。
    if (sseBuffer.value.length > 1_000_000) sseBuffer.value = sseBuffer.value.slice(-200_000)
    parseSseFrames()
  } else if (payload.kind === 'error') {
    sseStatus.value = 'error'
    sseLog.value.push({ seq: logSeq++, t: nowTime(), event: 'sys', data: t('realtime.sseError', { v: payload.message }), id: '' })
  } else {
    sseStatus.value = 'closed'
    sseConnId.value = null
    sseLog.value.push({ seq: logSeq++, t: nowTime(), event: 'sys', data: t('realtime.sseClosedByServer'), id: '' })
  }
  cap(sseLog.value)
}

// ---------- 事件订阅 ----------
let unlistenWs: UnlistenFn | null = null
let unlistenSse: UnlistenFn | null = null
/** 卸载标志：listen 是异步注册，路由在 IPC 返回前切走时监听器会晚于
 *  onUnmounted 到达——此时立即解绑，避免事件打进已卸载组件。 */
let listenersDisposed = false

onMounted(async () => {
  void detectWindow()
  try {
    if ('__TAURI_INTERNALS__' in window) {
      unlistenWs = await listen<WsEventPayload>('miyo:ws-event', (e) => onWsEvent(e.payload))
      unlistenSse = await listen<SseEventPayload>('miyo:sse-event', (e) => onSseEvent(e.payload))
      if (listenersDisposed) {
        unlistenWs()
        unlistenSse()
        unlistenWs = unlistenSse = null
      }
    }
  } catch {
    // 非 Tauri（浏览器预览）环境：忽略
  }
})

onUnmounted(() => {
  listenersDisposed = true
  unlistenWs?.()
  unlistenSse?.()
  // 视图卸载即断开：后台会话随之释放，避免泄漏。
  void wsDisconnect(true)
  void sseDisconnect(true)
})
</script>

<template>
  <div class="rt-root">
    <div class="row miyo-mb-2 rt-top">
      <button
        v-if="isMainWindow"
        class="miyo-btn miyo-btn-sm"
        type="button"
        @click="router.push('/workspace')"
      >
        ← {{ t('realtime.backToWorkspace') }}
      </button>
      <button
        v-if="isMainWindow"
        class="miyo-btn miyo-btn-sm miyo-btn-ghost"
        type="button"
        :title="t('realtime.popoutHint')"
        @click="popout"
      >
        {{ t('realtime.popout') }}
      </button>
      <Tabs v-model="mainTab" :tabs="MAIN_TABS" size="sm" />
      <span class="rt-status" :class="mainTab === 'ws' ? `st-${wsState}` : `st-${sseStatus}`">
        {{
          mainTab === 'ws'
            ? wsStateText
            : sseStatus === 'open'
              ? t('realtime.sseActive')
              : sseStatus === 'closed'
                ? t('realtime.sseDone')
                : sseStatus === 'error'
                  ? t('realtime.stateError')
                  : t('realtime.sseIdle')
        }}
      </span>
    </div>

    <!-- WebSocket -->
    <div v-if="mainTab === 'ws'" class="rt-pane">
      <div class="row miyo-mb-2">
        <input
          v-model="wsUrl"
          class="miyo-input rt-url"
          placeholder="ws://127.0.0.1:4010/socket"
          spellcheck="false"
          :disabled="!!wsConnId"
        />
        <label class="rt-check">
          <input v-model="wsAutoReconnect" type="checkbox" :disabled="!!wsConnId" />
          {{ t('realtime.autoReconnect') }}
        </label>
        <button
          v-if="!wsConnId"
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="wsConnecting || !wsUrl.trim()"
          @click="wsConnect"
        >
          {{ wsConnecting ? t('realtime.connectingShort') : t('realtime.connect') }}
        </button>
        <button v-else class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="wsDisconnect()">
          {{ t('realtime.disconnect') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" type="button" @click="wsLog = []">{{ t('realtime.clearLog') }}</button>
      </div>

      <!-- 折叠区：自定义请求头（握手携带）+ WS 子协议 -->
      <div class="rt-adv">
        <button class="rt-adv-toggle" type="button" :aria-expanded="wsHeadersOpen" @click="wsHeadersOpen = !wsHeadersOpen">
          <Icon :name="wsHeadersOpen ? 'chevron-down' : 'chevron-right'" :size="12" />
          {{ t('realtime.headersToggle', { n: headerCount(wsHeaders) }) }}
        </button>
        <span v-if="wsHeadersOpen" class="rt-adv-hint">{{ t('realtime.headersHint') }}</span>
      </div>
      <div v-if="wsHeadersOpen" class="rt-adv-body">
        <KeyValueTable
          v-model="wsHeaders"
          :show-description="false"
          :disabled="!!wsConnId"
        />
        <label class="rt-sub">
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
      </div>

      <div class="rt-log">
        <div v-if="!wsLog.length"><EmptyState icon="terminal" :title="t('realtime.wsEmpty')" compact /></div>
        <div v-for="m in wsLog" :key="m.seq" class="rt-line" :class="`dir-${m.dir}`">
          <span class="rt-time">{{ m.t }}</span>
          <span class="rt-dir">{{ m.dir === 'in' ? '↓' : m.dir === 'out' ? '↑' : '•' }}</span>
          <span class="rt-frame">{{ m.frame }}</span>
          <span class="rt-text">{{ m.text }}</span>
        </div>
      </div>

      <div class="row miyo-mt-2">
        <input
          v-model="wsSendText"
          class="miyo-input rt-url"
          :placeholder="t('realtime.sendPh')"
          spellcheck="false"
          :disabled="!wsConnId || wsState !== 'open'"
          @keydown.enter="wsSend('text')"
        />
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!wsConnId || wsState !== 'open' || !wsSendText"
          @click="wsSend('text')"
        >
          <Icon name="send" :size="13" /> {{ t('editor.send') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-ghost"
          type="button"
          :disabled="!wsConnId || wsState !== 'open' || !wsSendText"
          :title="t('realtime.pingHint')"
          @click="wsSend('ping')"
        >
          Ping
        </button>
      </div>
    </div>

    <!-- SSE -->
    <div v-else class="rt-pane">
      <div class="row miyo-mb-2">
        <input
          v-model="sseUrl"
          class="miyo-input rt-url"
          placeholder="http://127.0.0.1:4010/events"
          spellcheck="false"
          :disabled="!!sseConnId"
        />
        <button
          v-if="!sseConnId"
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
        <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" type="button" @click="sseLog = []">{{ t('realtime.clearLog') }}</button>
        <span v-if="sseLastId" class="hint-inline">{{ t('realtime.lastEventId', { v: sseLastId }) }}</span>
      </div>

      <!-- 折叠区：自定义请求头（订阅请求携带） -->
      <div class="rt-adv">
        <button class="rt-adv-toggle" type="button" :aria-expanded="sseHeadersOpen" @click="sseHeadersOpen = !sseHeadersOpen">
          <Icon :name="sseHeadersOpen ? 'chevron-down' : 'chevron-right'" :size="12" />
          {{ t('realtime.headersToggle', { n: headerCount(sseHeaders) }) }}
        </button>
        <span v-if="sseHeadersOpen" class="rt-adv-hint">{{ t('realtime.headersHint') }}</span>
      </div>
      <div v-if="sseHeadersOpen" class="rt-adv-body">
        <KeyValueTable v-model="sseHeaders" :show-description="false" :disabled="!!sseConnId" />
      </div>

      <div class="rt-log">
        <div v-if="!sseLog.length"><EmptyState icon="terminal" :title="t('realtime.sseEmpty')" compact /></div>
        <div v-for="m in sseLog" :key="m.seq" class="rt-line dir-in">
          <span class="rt-time">{{ m.t }}</span>
          <span class="rt-frame">{{ m.event }}</span>
          <span v-if="m.id" class="rt-id">#{{ m.id }}</span>
          <span class="rt-text">{{ m.data }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.rt-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 12px 16px;
  gap: 4px;
  min-height: 0;
}
.row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.rt-top {
  justify-content: flex-start;
}
/* 页签内容区：flex 纵向撑满，日志区 flex:1 吃掉剩余高度（直接子节点必须是它，block 包裹会断掉传递） */
.rt-pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  gap: 4px;
}
.rt-url {
  flex: 1;
  font-family: var(--font-mono);
  font-size: 12.5px;
}
.rt-check {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--text-2);
  white-space: nowrap;
}
.rt-check input {
  width: 14px;
  height: 14px;
  accent-color: var(--accent);
  cursor: pointer;
}
.rt-status {
  font-size: 12px;
  font-weight: 600;
  padding: 2px 10px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
}
.rt-status.st-open {
  color: var(--success);
  background: var(--success-tint);
}
.rt-status.st-connecting {
  color: var(--warning);
  background: var(--warning-tint);
}
.rt-status.st-error {
  color: var(--danger);
  background: var(--danger-tint);
}
/* ---- 折叠区：自定义请求头 / 子协议 ---- */
.rt-adv {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  flex-shrink: 0;
}
.rt-adv-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.rt-adv-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.rt-adv-hint {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.rt-adv-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex-shrink: 0;
}
.rt-sub {
  display: flex;
  align-items: center;
  gap: 8px;
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
  font-family: var(--font-mono);
  font-size: 12px;
}

.rt-log {
  flex: 1;
  min-height: 200px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
  padding: 8px 0;
  font-family: var(--font-mono);
  font-size: 12px;
}
.rt-line {
  display: flex;
  gap: 8px;
  padding: 3px 12px;
  align-items: baseline;
  border-bottom: 1px dashed transparent;
}
.rt-line:hover {
  background: var(--bg-hover);
}

/* ---- 窄窗回退（<1080px）：日志行改为两行制（时间/方向/帧在上，正文独占整行），
   顶部 URL 输入保底宽度，根容器内边距收紧，避免横向溢出 ---- */
@media (max-width: 1080px) {
  .rt-root {
    padding: 10px;
  }
  .rt-url {
    min-width: 180px;
  }
  .rt-line {
    flex-wrap: wrap;
    row-gap: 2px;
  }
  .rt-line .rt-text {
    flex: 1 0 100%;
  }
}
.rt-time {
  flex-shrink: 0;
  color: var(--text-3);
}
.rt-dir {
  flex-shrink: 0;
  width: 14px;
  text-align: center;
  font-weight: 700;
}
.dir-in .rt-dir {
  color: var(--success);
}
.dir-out .rt-dir {
  color: var(--info);
}
.dir-sys .rt-dir {
  color: var(--text-3);
}
.rt-frame {
  flex-shrink: 0;
  font-size: var(--fs-xxs);
  font-weight: 700;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-2);
}
.rt-id {
  flex-shrink: 0;
  color: var(--text-3);
}
.rt-text {
  flex: 1;
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-1);
}
.hint-inline {
  font-size: var(--fs-xs);
  color: var(--text-3);
  white-space: nowrap;
}
</style>
