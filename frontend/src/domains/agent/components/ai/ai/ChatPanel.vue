<script setup lang="ts">
/**
 * ChatPanel：AI 对话（消息流 / 工具调用卡 / 副作用确认卡 / 快捷提示）。
 * 流式中的 assistant 内容与工具卡为临时态（done 后以持久化消息为准）。
 */
import { computed, nextTick, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useAiTestStore } from '@/domains/agent/stores/aiTest'
import { renderMarkdownLite } from '@/utils/markdownLite'
import Icon from '@/components/ui/Icon.vue'
import type { AiMessageView } from '@/types/miyoApi.d'

const t = useLocaleStore().t
const ai = useAiTestStore()

const input = ref('')
const scrollEl = ref<HTMLElement | null>(null)

const QUICK_PROMPTS = [
  'ai.quick.scenario',
  'ai.quick.cases',
  'ai.quick.coverage',
  'ai.quick.analyze',
] as const

function send(): void {
  const text = input.value.trim()
  if (!text || ai.streaming) return
  input.value = ''
  void ai.send(text)
}

/** 消息流末尾跟随滚动。 */
watch(
  () => [ai.messages.length, ai.streamContent, ai.toolCards.length] as const,
  () => {
    void nextTick(() => {
      const el = scrollEl.value
      if (el) el.scrollTop = el.scrollHeight
    })
  },
)

/** tool 消息回显为紧凑卡片（结果 JSON 截断）。 */
function toolMsgOf(msg: AiMessageView): { name: string; ok: boolean } | null {
  const callId = msg.tool_call_id ?? ''
  // 前一个 assistant 消息的 tool_calls 里找同名工具。
  const idx = ai.messages.findIndex((m) => m.id === msg.id)
  for (let i = idx - 1; i >= 0; i -= 1) {
    const m = ai.messages[i]!
    if (m.role === 'assistant' && Array.isArray(m.tool_calls)) {
      const call = m.tool_calls.find(
        (c) => (c as { id?: string }).id === callId,
      ) as { function?: { name?: string } } | undefined
      if (call?.function?.name) return { name: call.function.name, ok: !msg.content?.includes('"error"') }
    }
    if (m.role === 'assistant') break
  }
  return { name: 'tool', ok: true }
}

const streamHtml = computed(() => renderMarkdownLite(ai.streamContent))

const sessionTitle = computed(
  () => ai.sessions.find((s) => s.id === ai.activeSessionId)?.title ?? t('ai.newChat'),
)
</script>

<template>
  <section class="chat-panel">
    <!-- 会话头 -->
    <header class="chat-head">
      <div class="chat-title-row">
        <h3 class="chat-title">{{ sessionTitle }}</h3>
        <button class="miyo-btn miyo-btn-sm" @click="ai.newSession()">
          <Icon name="plus" :size="12" />
          {{ t('ai.newChat') }}
        </button>
      </div>
      <div v-if="ai.sessions.length > 0" class="chat-sessions">
        <button
          v-for="s in ai.sessions.slice(0, 6)"
          :key="s.id"
          class="chat-session-chip"
          :class="{ active: s.id === ai.activeSessionId }"
          :title="s.title"
          @click="ai.selectSession(s.id)"
        >
          {{ s.title || t('ai.newChat') }}
        </button>
      </div>
    </header>

    <!-- 消息流 -->
    <div ref="scrollEl" class="chat-scroll">
      <div v-if="ai.messages.length === 0 && !ai.streaming" class="chat-empty">
        <Icon name="zap" :size="28" class="empty-icon" />
        <p class="empty-text">{{ t('ai.empty') }}</p>
        <div class="empty-quick">
          <button
            v-for="key in QUICK_PROMPTS"
            :key="key"
            class="quick-chip"
            :disabled="!ai.projectId"
            @click="ai.send(t(key))"
          >
            {{ t(key) }}
          </button>
        </div>
      </div>

      <template v-for="msg in ai.messages" :key="msg.id">
        <!-- 用户 -->
        <div v-if="msg.role === 'user'" class="msg msg-user">
          <div class="bubble-user">{{ msg.content }}</div>
        </div>
        <!-- 助手文本 -->
        <div v-else-if="msg.role === 'assistant' && msg.content" class="msg msg-assistant">
          <!-- eslint-disable-next-line vue/no-v-html -->
          <div class="bubble-assistant" v-html="renderMarkdownLite(msg.content)" />
        </div>
        <!-- 工具调用卡（持久化 tool 消息） -->
        <div v-else-if="msg.role === 'tool'" class="msg msg-tool">
          <div class="tool-card done">
            <div class="tool-head">
              <Icon name="terminal" :size="12" />
              <span class="tool-name">{{ toolMsgOf(msg)?.name }}</span>
              <Icon
                :name="toolMsgOf(msg)?.ok ? 'check' : 'x'"
                :size="12"
                :class="toolMsgOf(msg)?.ok ? 'tool-ok' : 'tool-fail'"
              />
            </div>
          </div>
        </div>
        <!-- 产物提示条（详情在右栏） -->
        <div v-else-if="msg.role === 'artifact' && msg.artifact" class="msg msg-artifact">
          <button class="artifact-link" @click="ai.activeArtifactType = msg.artifact!.type; ai.artifactPanelOpen = true">
            <Icon name="layout-grid" :size="12" />
            {{ t(`ai.artifact.${msg.artifact.type}`) }}
            <Icon name="arrow-up-right" :size="11" />
          </button>
        </div>
      </template>

      <!-- 流式临时态 -->
      <template v-if="ai.streaming">
        <div v-for="card in ai.toolCards" :key="card.callId" class="msg msg-tool">
          <div class="tool-card" :class="{ done: card.ok !== null, fail: card.ok === false }">
            <div class="tool-head">
              <Icon name="terminal" :size="12" />
              <span class="tool-name">{{ card.name }}</span>
              <span v-if="card.ok === null" class="tool-spin" />
              <Icon v-else-if="card.ok" name="check" :size="12" class="tool-ok" />
              <Icon v-else name="x" :size="12" class="tool-fail" />
            </div>
          </div>
        </div>
        <div v-if="ai.streamContent" class="msg msg-assistant">
          <!-- eslint-disable-next-line vue/no-v-html -->
          <div class="bubble-assistant streaming" v-html="streamHtml" />
        </div>
        <div v-else-if="ai.toolCards.length === 0" class="msg msg-assistant">
          <div class="bubble-assistant streaming"><span class="dot-spin" />…</div>
        </div>
      </template>

      <!-- 副作用确认卡 -->
      <div v-if="ai.pendingApproval" class="msg msg-pending">
        <div class="pending-card" :class="`risk-${ai.pendingApproval.risk}`">
          <div class="pending-head">
            <Icon name="alert-triangle" :size="14" />
            <span class="pending-title">{{ t('ai.pendingTitle') }}</span>
            <span class="risk-chip">{{ t(`ai.risk.${ai.pendingApproval.risk}`) }}</span>
          </div>
          <div class="pending-tool">
            <span class="tool-name mono">{{ ai.pendingApproval.name }}</span>
          </div>
          <pre class="pending-args">{{ JSON.stringify(ai.pendingApproval.args, null, 2) }}</pre>
          <div class="pending-actions">
            <button class="miyo-btn miyo-btn-danger-solid miyo-btn-sm" @click="ai.approve(true)">
              {{ t('ai.approve') }}
            </button>
            <button class="miyo-btn miyo-btn-sm" @click="ai.approve(false)">
              {{ t('ai.deny') }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- 输入区 -->
    <footer class="chat-input-row">
      <textarea
        v-model="input"
        class="miyo-textarea chat-input"
        rows="2"
        :placeholder="t('ai.chatPlaceholder')"
        :disabled="!ai.projectId"
        @keydown.enter.exact.prevent="send"
      />
      <div class="chat-input-actions">
        <button
          v-if="ai.streaming"
          class="miyo-btn miyo-btn-sm miyo-btn-danger-solid"
          @click="ai.cancelChat()"
        >
          {{ t('ai.stop') }}
        </button>
        <button
          v-else
          class="miyo-btn miyo-btn-primary miyo-btn-sm"
          :disabled="!input.trim() || !ai.projectId"
          @click="send"
        >
          <Icon name="send" :size="12" />
          {{ t('ai.send') }}
        </button>
      </div>
    </footer>
  </section>
</template>

<style scoped>
.chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.chat-head {
  flex: none;
  padding: var(--sp-2) var(--sp-3);
  border-bottom: 1px solid var(--border);
}
.chat-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}
.chat-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chat-sessions {
  display: flex;
  gap: var(--sp-1);
  margin-top: var(--sp-2);
  overflow-x: auto;
}
.chat-session-chip {
  flex: none;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-panel);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  cursor: pointer;
}
.chat-session-chip.active {
  background: var(--bg-hover);
  color: var(--text-1);
  border-color: var(--border-strong);
}

.chat-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.chat-empty {
  margin: auto;
  text-align: center;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  max-width: 420px;
}
.empty-icon {
  color: var(--accent);
}
.empty-text {
  margin: 0;
  color: var(--text-3);
  font-size: var(--fs-sm);
  line-height: 1.6;
}
.empty-quick {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  justify-content: center;
}
.quick-chip {
  padding: 4px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  background: var(--bg-panel);
  color: var(--text-2);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.quick-chip:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-1);
}

.msg {
  display: flex;
  flex-direction: column;
}
.msg-user {
  align-items: flex-end;
}
.bubble-user {
  max-width: 85%;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--radius-lg);
  background: var(--primary-tint);
  color: var(--text-1);
  font-size: var(--fs-sm);
  white-space: pre-wrap;
  word-break: break-word;
}
.msg-assistant {
  align-items: flex-start;
}
.bubble-assistant {
  max-width: 92%;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-1);
  font-size: var(--fs-sm);
  line-height: 1.65;
  word-break: break-word;
}
.bubble-assistant :deep(p) {
  margin: 0 0 var(--sp-2);
}
.bubble-assistant :deep(p:last-child) {
  margin-bottom: 0;
}
.bubble-assistant :deep(ul),
.bubble-assistant :deep(ol) {
  margin: 0 0 var(--sp-2);
  padding-left: var(--sp-4);
}
.bubble-assistant :deep(code) {
  font-family: var(--font-mono, monospace);
  font-size: 0.92em;
  background: var(--bg-code);
  padding: 1px 4px;
  border-radius: var(--radius-sm);
}
.bubble-assistant :deep(pre) {
  background: var(--bg-code);
  padding: var(--sp-2);
  border-radius: var(--radius-md);
  overflow-x: auto;
}

.msg-tool {
  align-items: flex-start;
}
.tool-card {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-1) var(--sp-2);
  border-radius: var(--radius-md);
  border: 1px dashed var(--border);
  background: var(--bg-panel);
  font-size: var(--fs-xs);
  color: var(--text-3);
  min-width: 200px;
}
.tool-card.done {
  border-style: solid;
}
.tool-card.fail {
  border-color: var(--danger);
}
.tool-head {
  display: flex;
  align-items: center;
  gap: var(--sp-1);
}
.tool-name {
  color: var(--text-2);
  font-size: var(--fs-xxs);
}
.tool-name.mono {
  font-family: var(--font-mono, monospace);
}
.tool-ok {
  color: var(--success);
}
.tool-fail {
  color: var(--danger);
}
.tool-spin {
  width: 10px;
  height: 10px;
  border: 2px solid var(--border-strong);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
.dot-spin {
  display: inline-block;
  width: 8px;
  height: 8px;
  border: 2px solid var(--border-strong);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.msg-artifact {
  align-items: flex-start;
}
.artifact-link {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-1);
  padding: 3px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border-strong);
  background: var(--bg-panel);
  color: var(--accent);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.artifact-link:hover {
  background: var(--bg-hover);
}

.msg-pending {
  align-items: stretch;
}
.pending-card {
  border-radius: var(--radius-lg);
  border: 1px solid var(--warning);
  background: var(--bg-card);
  padding: var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
.pending-card.risk-high {
  border-color: var(--danger);
}
.pending-head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-1);
  font-size: var(--fs-sm);
}
.pending-title {
  font-weight: 600;
}
.risk-chip {
  margin-left: auto;
  padding: 1px 8px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  background: var(--warning-tint);
  color: var(--warning);
}
.risk-high .risk-chip {
  background: var(--danger-tint);
  color: var(--danger);
}
.pending-tool {
  font-size: var(--fs-sm);
  color: var(--text-2);
}
.pending-args {
  margin: 0;
  max-height: 180px;
  overflow: auto;
  padding: var(--sp-2);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  color: var(--text-2);
  font-family: var(--font-mono, monospace);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
}
.pending-actions {
  display: flex;
  gap: var(--sp-2);
  justify-content: flex-end;
}

.chat-input-row {
  flex: none;
  padding: var(--sp-2) var(--sp-3);
  border-top: 1px solid var(--border);
  display: flex;
  gap: var(--sp-2);
  align-items: flex-end;
}
.chat-input {
  flex: 1;
  resize: none;
  min-height: 44px;
  max-height: 140px;
}
.chat-input-actions {
  flex: none;
}
</style>
