<script setup lang="ts">
/**
 * GrpcResponsePanel：gRPC 响应区（unary 与服务端流两种形态）。
 *
 * - unary：grpc-status 徽标（非零为失败色）+ 耗时/大小 + 响应消息 JSON +
 *   元数据表（tonic 0.14 已把 trailers 并入同一 map，grpc-status 在此）；
 * - 服务端流：消息时间线（序号 + 相对开流毫秒 + JSON），运行中可手动关闭，
 *   正常结束展示 grpc-status，失败展示错误文案。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { methodTextTone } from '../utils/methodTone'
import type { GrpcMessageEvent } from '../types/miyoApi'
import JsonTree from './JsonTree.vue'
import Icon from './ui/Icon.vue'
import Tooltip from './ui/Tooltip.vue'
import type { GrpcResponse } from '../types/miyoApi'
import type { GrpcStreamState } from '../stores/workspace'

const props = defineProps<{
  method: string
  /** unary 响应（send 返回 unary 时非空）。 */
  response: GrpcResponse | null
  /** 服务端流运行态（服务端流调用后非空）。 */
  stream: GrpcStreamState | null
}>()

const locale = useLocaleStore()
const t = locale.t
const store = useWorkspaceStore()
const api = useMiyoApi()

/** grpc-status → 展示文案（gRPC 标准状态码；0=OK 绿，非零红）。 */
const STATUS_LABELS: Record<number, string> = {
  0: 'OK',
  1: 'CANCELLED',
  2: 'UNKNOWN',
  3: 'INVALID_ARGUMENT',
  4: 'DEADLINE_EXCEEDED',
  5: 'NOT_FOUND',
  6: 'ALREADY_EXISTS',
  7: 'PERMISSION_DENIED',
  8: 'RESOURCE_EXHAUSTED',
  9: 'FAILED_PRECONDITION',
  10: 'ABORTED',
  11: 'OUT_OF_RANGE',
  12: 'UNIMPLEMENTED',
  13: 'INTERNAL',
  14: 'UNAVAILABLE',
  15: 'DATA_LOSS',
  16: 'UNAUTHENTICATED',
}

const unaryOk = computed(() => (props.response ? props.response.grpc_status === 0 : false))
const unaryStatusLabel = computed(() =>
  props.response ? (STATUS_LABELS[props.response.grpc_status] ?? `CODE ${props.response.grpc_status}`) : '',
)

const messageTree = ref<unknown>(null)
watch(
  () => props.response?.message_json,
  (raw) => {
    if (!raw) {
      messageTree.value = null
      return
    }
    try {
      messageTree.value = JSON.parse(raw)
    } catch {
      messageTree.value = raw
    }
  },
  { immediate: true },
)

/** 展示口径：JSON 紧凑长度（与后端 size_bytes 同源）。 */
function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}

/** 当前查看的流消息序号（0 = 尚未选择，跟随最新）。 */
const activeStreamMessage = ref(0)
const pinned = ref(false)
watch(
  // 盯最后一条的 sequence 而非 length：store 对流消息有 cap（截断最旧），
  // 达到上限后 length 恒定，按 length 无法继续跟随最新
  () => {
    const msgs = props.stream?.messages
    return msgs?.length ? msgs[msgs.length - 1].sequence : 0
  },
  (lastSeq) => {
    if (lastSeq > 0 && !pinned.value) activeStreamMessage.value = lastSeq
  },
  { immediate: true },
)

/** 当前查看的消息：按 sequence 精确匹配（pinMessage 存的即 sequence，
 *  旧实现把 sequence 当索引用，错位一条且最后一条显示空）；
 *  pinned 的消息被 cap 截断淘汰时回落到最新一条。 */
const activeMessage = computed(() => {
  const msgs = props.stream?.messages
  if (!msgs?.length) return undefined
  return msgs.find((m) => m.sequence === activeStreamMessage.value) ?? msgs[msgs.length - 1]
})

/** 消息预览缓存：prettyJson 解析开销大，流式高频追加 + 时间线重渲染时
 *  避免对全部历史消息重复解析（WeakMap 随消息对象一起回收）。 */
const previewCache = new WeakMap<GrpcMessageEvent, string>()
function streamPreview(msg: GrpcMessageEvent): string {
  let p = previewCache.get(msg)
  if (p === undefined) {
    p = prettyJson(msg.message_json).replace(/\s+/g, ' ').slice(0, 120)
    previewCache.set(msg, p)
  }
  return p
}
watch(
  () => props.stream?.streamId,
  () => {
    // 换流重置查看状态
    pinned.value = false
    activeStreamMessage.value = 0
  },
)

function pinMessage(seq: number): void {
  pinned.value = true
  activeStreamMessage.value = seq
}

async function closeStream(): Promise<void> {
  if (!props.stream) return
  try {
    await api.grpcStreamClose(props.stream.streamId)
  } finally {
    store.dropGrpcStream(props.stream.streamId)
  }
}

/** 元数据行（unary 与流的 end 事件共用渲染）。 */
function metaRows(metadata: [string, string][]): Array<{ key: string; value: string }> {
  return (metadata ?? []).filter(([k]) => k.trim()).map(([k, v]) => ({ key: k, value: v }))
}
</script>

<template>
  <div class="grpc-result">
    <!-- ---------- unary ---------- -->
    <template v-if="response">
      <div class="grpc-head">
        <span class="status-pill" :class="{ ok: unaryOk }">
          {{ unaryStatusLabel }}
          <span class="status-code">grpc-status {{ response.grpc_status }}</span>
        </span>
        <span class="grpc-meta">{{ t('grpc.duration') }} {{ response.duration_ms.toFixed(1) }}ms</span>
        <span class="grpc-meta">{{ sizeLabel(response.size_bytes) }}</span>
        <span v-if="method" class="grpc-meta method-chip" :class="methodTextTone(method)">{{ method }}</span>
      </div>

      <p v-if="response.grpc_message && !unaryOk" class="grpc-error-msg">{{ response.grpc_message }}</p>

      <div class="grpc-body">
        <p class="section-label">{{ t('grpc.responseMessage') }}</p>
        <JsonTree :data="messageTree" />
      </div>

      <div v-if="metaRows(response.metadata).length" class="grpc-metadata">
        <p class="section-label">{{ t('grpc.metadata') }}</p>
        <div class="meta-table">
          <div v-for="row in metaRows(response.metadata)" :key="row.key" class="meta-row">
            <span class="meta-key">{{ row.key }}</span>
            <span class="meta-value">{{ row.value }}</span>
          </div>
        </div>
      </div>
    </template>

    <!-- ---------- 服务端流 ---------- -->
    <template v-else-if="stream">
      <div class="grpc-head">
        <span class="status-pill" :class="{ ok: stream.status === 'running' || (stream.end?.grpc_status ?? 0) === 0 }">
          {{ stream.status === 'running' ? t('grpc.streamRunning') : STATUS_LABELS[stream.end?.grpc_status ?? 0] ?? `CODE ${stream.end?.grpc_status ?? 0}` }}
        </span>
        <span class="grpc-meta">{{ stream.service }}/{{ stream.method }}</span>
        <span class="grpc-meta">{{ t('grpc.messageCount', { n: stream.messages.length }) }}</span>
        <span v-if="stream.status === 'running'" class="stream-dot" aria-hidden="true"></span>
        <span class="grpc-head-spacer"></span>
        <Tooltip v-if="stream.status === 'running'" :content="t('grpc.closeStream')" placement="top">
          <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="void closeStream()">
            <Icon name="stop" :size="12" /> {{ t('grpc.stopStream') }}
          </button>
        </Tooltip>
      </div>

      <p v-if="stream.error" class="grpc-error-msg">{{ stream.error }}</p>
      <p v-else-if="stream.end?.grpc_message && stream.end.grpc_status !== 0" class="grpc-error-msg">
        {{ stream.end.grpc_message }}
      </p>

      <div v-if="stream.messages.length" class="stream-timeline">
        <button
          v-for="msg in stream.messages"
          :key="msg.sequence"
          class="stream-item"
          type="button"
          :class="{ active: activeStreamMessage === msg.sequence }"
          @click="pinMessage(msg.sequence)"
        >
          <span class="stream-seq">#{{ msg.sequence }}</span>
          <span class="stream-elapsed">{{ msg.elapsed_ms.toFixed(0) }}ms</span>
          <span class="stream-preview">{{ streamPreview(msg) }}</span>
        </button>
      </div>
      <div v-else-if="stream.status === 'running'" class="stream-waiting">
        <span class="stream-dot" aria-hidden="true"></span>
        {{ t('grpc.waitingMessages') }}
      </div>

      <div v-if="stream.messages.length" class="grpc-body">
        <p class="section-label">{{ t('grpc.messageDetail', { n: Math.max(1, activeMessage?.sequence ?? 0) }) }}</p>
        <pre class="stream-detail">{{ prettyJson(activeMessage?.message_json ?? '{}') }}</pre>
      </div>

      <div
        v-if="stream.end && metaRows(stream.end.metadata).length"
        class="grpc-metadata"
      >
        <p class="section-label">{{ t('grpc.metadata') }}</p>
        <div class="meta-table">
          <div v-for="row in metaRows(stream.end.metadata)" :key="row.key" class="meta-row">
            <span class="meta-key">{{ row.key }}</span>
            <span class="meta-value">{{ row.value }}</span>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.grpc-result {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  height: 100%;
  overflow-y: auto;
  /* 与 HTTP 响应面板同款卡片浮层（顶边由分割条承接） */
  background: var(--bg-card);
  border: 1px solid var(--border-panel);
  border-top: none;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-panel);
}
.grpc-head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.grpc-head-spacer {
  flex: 1;
}
/* 状态：小圆点 + 同色文本（与 HTTP 响应区同款，去色块降负担） */
.status-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  font-family: var(--font-mono);
  color: var(--danger);
}
.status-pill.ok {
  color: var(--success);
}
.status-code {
  font-weight: 400;
  opacity: 0.75;
}
.grpc-meta {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.method-chip {
  font-weight: 600;
}
.grpc-error-msg {
  margin: 0;
  font-size: 12px;
  color: var(--danger);
  padding: 6px 10px;
  border: 1px solid var(--danger-border);
  border-radius: var(--radius);
  background: var(--danger-tint);
  overflow-wrap: anywhere;
}
.section-label {
  margin: 0 0 4px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  letter-spacing: 0.02em;
}
.grpc-body {
  min-height: 0;
}
.grpc-metadata {
  margin-top: 4px;
}
.meta-table {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}
.meta-row {
  display: flex;
  gap: 10px;
  padding: 4px 10px;
  font-size: 12px;
}
.meta-row + .meta-row {
  border-top: 1px solid var(--border);
}
.meta-key {
  color: var(--text-2);
  font-family: var(--font-mono, monospace);
  flex: none;
}
.meta-value {
  color: var(--text-1);
  font-family: var(--font-mono, monospace);
  overflow-wrap: anywhere;
}
.stream-timeline {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow-y: auto;
  max-height: 40%;
}
.stream-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 10px;
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
  font-size: 12px;
  color: var(--text-2);
}
.stream-item + .stream-item {
  border-top: 1px solid var(--border);
}
.stream-item:hover {
  background: var(--bg-hover);
}
.stream-item.active {
  background: color-mix(in srgb, var(--grpc) 10%, transparent);
}
.stream-seq {
  color: var(--grpc);
  font-weight: 600;
  flex: none;
}
.stream-elapsed {
  color: var(--text-3);
  flex: none;
  width: 64px;
}
.stream-preview {
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.stream-waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-3);
  padding: 10px;
}
.stream-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: var(--grpc);
  animation: grpc-pulse 1.2s ease-in-out infinite;
}
@keyframes grpc-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}
@media (prefers-reduced-motion: reduce) {
  .stream-dot {
    animation: none;
  }
}
.stream-detail {
  margin: 0;
  padding: 8px 10px;
  border: 1px solid var(--border-editor);
  border-radius: var(--radius);
  font-size: 12px;
  font-family: var(--font-mono, monospace);
  color: var(--text-1);
  overflow-x: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
