<script setup lang="ts">
/**
 * JavaResultPanel：右侧执行结果区。
 *
 * Tab：返回结果（JsonTree 展开 + Raw 切换）/ 异常堆栈 / 执行信息 /
 * 输入参数（敏感参数脱敏展示）/ 调用历史（最近 10 条快速重跑）。
 * 异常状态头部醒目展示异常类型；截断结果给出明确提示；支持复制 / 导出。
 */
import { computed, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import JsonTree from '@/components/ui/JsonTree.vue'
import StatusBadge from '@/components/ui/StatusBadge.vue'
import Tabs, { type TabItem } from '@/components/ui/Tabs.vue'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { maskArgs } from '@/domains/java-debug/utils/mask'
import type { JavaInvokeResponse, JavaInvocationRecord, JavaParamInfo } from '@/types/miyoApi.d'

const props = defineProps<{
  response: JavaInvokeResponse | null
  startedAt: number | null
  argsJson: string
  /** 形参元数据（脱敏按名匹配用）。 */
  params: JavaParamInfo[]
  invoking: boolean
  /** 最近调用记录（历史 Tab 快速重跑）。 */
  history: JavaInvocationRecord[]
}>()

const emit = defineEmits<{
  /** 快速重跑（回填参数 + 选类选方法）。 */
  rerun: [record: JavaInvocationRecord]
}>()

const locale = useLocaleStore()
const t = locale.t
const toast = useToast()

const activeTab = ref('result')
const rawMode = ref(false)

const tabs = computed<TabItem[]>(() => {
  const hasException = !!props.response?.exception
  return [
    { key: 'result', label: t('java.tabResult') },
    { key: 'exception', label: t('java.tabException'), count: hasException ? 1 : 0 },
    { key: 'info', label: t('java.tabInfo') },
    { key: 'args', label: t('java.tabArgs') },
    { key: 'history', label: t('java.tabHistory') },
  ]
})

/** 输入参数展示：按形参名 + 对象键脱敏。 */
const maskedArgsText = computed(() => {
  let parsed: unknown = []
  try {
    parsed = JSON.parse(props.argsJson || '[]')
  } catch {
    return props.argsJson
  }
  const masked = maskArgs(parsed, props.params.map((p) => p.name))
  const hasMasked = JSON.stringify(masked) !== JSON.stringify(parsed)
  const text = JSON.stringify(masked, null, 2)
  return hasMasked ? `${text}\n\n${t('java.argsMasked')}` : text
})

const statusTone = computed(() => {
  switch (props.response?.status) {
    case 'success':
      return 'ok' as const
    case 'failed':
      return 'fail' as const
    case 'timeout':
      return 'warn' as const
    case 'cancelled':
      return 'cancel' as const
    case 'runtime_disconnected':
      return 'fail' as const
    default:
      return 'info' as const
  }
})

const statusLabel = computed(() => {
  switch (props.response?.status) {
    case 'success':
      return t('java.statusSuccess')
    case 'failed':
      return t('java.statusFailed')
    case 'timeout':
      return t('java.statusTimeout')
    case 'cancelled':
      return t('java.statusCancelled')
    case 'runtime_disconnected':
      return t('java.statusDisconnected')
    default:
      return t('java.statusIdle')
  }
})

/** 返回值 JSON 文本（Raw 模式 / 复制 / 导出用）。 */
const resultText = computed(() => {
  const result = props.response?.result
  if (result === undefined || result === null) return ''
  try {
    return typeof result === 'string' ? result : JSON.stringify(result, null, 2)
  } catch {
    return String(result)
  }
})

/** 导出结果文件。 */
async function exportResult(): Promise<void> {
  if (!resultText.value) return
  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const path = await save({
      defaultPath: `java-result-${Date.now()}.json`,
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!path) return
    await import('@/composables/useMiyoApi').then(({ useMiyoApi }) =>
      useMiyoApi().writeTextFile(path, resultText.value),
    )
    toast.success(t('java.exported'))
  } catch {
    toast.error(t('java.exportFailed'))
  }
}

async function copy(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('java.copied'))
  } catch {
    toast.error(t('java.copyFailed'))
  }
}

const infoRows = computed(() => {
  const r = props.response
  if (!r) return []
  const rows: Array<{ label: string; value: string }> = []
  rows.push({ label: t('java.infoStatus'), value: statusLabel.value })
  rows.push({ label: t('java.infoMethod'), value: r.method_id })
  if (r.duration_ms != null) {
    rows.push({ label: t('java.infoDuration'), value: `${r.duration_ms} ms` })
  }
  if (props.startedAt != null) {
    rows.push({ label: t('java.infoStarted'), value: new Date(props.startedAt).toLocaleString() })
  }
  if (r.started_at) rows.push({ label: t('java.infoRuntimeStarted'), value: r.started_at })
  if (r.finished_at) rows.push({ label: t('java.infoFinished'), value: r.finished_at })
  if (r.invocation_id) rows.push({ label: t('java.infoInvocationId'), value: r.invocation_id })
  return rows
})
</script>

<template>
  <section class="jrp">
    <div class="jrp-head">
      <StatusBadge v-if="response" :tone="statusTone" :label="statusLabel" />
      <span v-else class="jrp-idle">{{ invoking ? t('java.running') : t('java.statusIdle') }}</span>
      <span v-if="response?.result_truncated" class="jrp-truncated">
        <Icon name="alert-triangle" :size="12" />
        {{ t('java.resultTruncated') }}
      </span>
      <span class="jrp-head-actions">
        <IconButton
          v-if="response"
          :name="rawMode ? 'list' : 'code'"
          :title="rawMode ? t('java.treeMode') : t('java.rawMode')"
          @click="rawMode = !rawMode"
        />
        <IconButton
          v-if="resultText"
          name="copy"
          :title="t('common.copy')"
          @click="copy(resultText)"
        />
        <IconButton
          v-if="resultText"
          name="download"
          :title="t('java.export')"
          @click="exportResult"
        />
      </span>
    </div>

    <Tabs v-model="activeTab" :tabs="tabs" size="sm" />

    <div class="jrp-body" data-testid="java-result-body">
      <!-- 返回结果 -->
      <template v-if="activeTab === 'result'">
        <div v-if="!response" class="jrp-empty">{{ t('java.noResultYet') }}</div>
        <div v-else-if="response.status === 'success'" class="jrp-result">
          <JsonTree v-if="!rawMode && response.result != null" :data="response.result" />
          <pre v-else class="jrp-raw">{{ resultText || t('java.voidResult') }}</pre>
        </div>
        <div v-else class="jrp-result">
          <pre v-if="resultText" class="jrp-raw">{{ resultText }}</pre>
          <div v-else class="jrp-empty">{{ t('java.noResultFailed') }}</div>
        </div>
      </template>

      <!-- 异常堆栈 -->
      <template v-else-if="activeTab === 'exception'">
        <div v-if="!response?.exception" class="jrp-empty">{{ t('java.noException') }}</div>
        <div v-else class="jrp-exception">
          <div class="jrp-exc-head" data-testid="java-exception-head">
            <Icon name="alert-triangle" :size="14" />
            <span class="jrp-exc-type">{{ response.exception.exception_type }}</span>
            <IconButton
              name="copy"
              :title="t('common.copy')"
              @click="
                copy(
                  response.exception.exception_type +
                    ': ' +
                    (response.exception.message ?? '') +
                    '\n' +
                    response.exception.stack_trace.join('\n'),
                )
              "
            />
          </div>
          <p v-if="response.exception.message" class="jrp-exc-message">
            {{ response.exception.message }}
          </p>
          <pre class="jrp-exc-stack">{{ response.exception.stack_trace.join('\n') }}</pre>
          <div v-if="response.exception.cause_chain.length > 0" class="jrp-exc-causes">
            <pre class="jrp-exc-stack">{{ response.exception.cause_chain.join('\n') }}</pre>
          </div>
        </div>
      </template>

      <!-- 执行信息 -->
      <template v-else-if="activeTab === 'info'">
        <div v-if="!response" class="jrp-empty">{{ t('java.noResultYet') }}</div>
        <dl v-else class="jrp-info">
          <template v-for="row in infoRows" :key="row.label">
            <dt>{{ row.label }}</dt>
            <dd>
              <span :title="row.value">{{ row.value }}</span>
            </dd>
          </template>
        </dl>
      </template>

      <!-- 输入参数（脱敏） -->
      <template v-else-if="activeTab === 'args'">
        <pre class="jrp-raw" data-testid="java-result-args">{{ maskedArgsText }}</pre>
      </template>

      <!-- 调用历史（最近 10 条快速重跑） -->
      <template v-else>
        <div v-if="history.length === 0" class="jrp-empty">{{ t('java.historyEmptyTitle') }}</div>
        <ul v-else class="jrp-history">
          <li v-for="h in history" :key="h.id" class="jrp-history-item">
            <StatusBadge :tone="h.status === 'success' ? 'ok' : 'fail'" :label="h.status === 'success' ? t('java.statusSuccess') : t('java.statusFailed')" />
            <span class="jrp-history-sig" :title="h.method_id">
              {{ h.method_signature || h.method_id }}
            </span>
            <span v-if="h.duration_ms != null" class="jrp-history-duration">{{ h.duration_ms }} ms</span>
            <span class="jrp-history-time">{{ new Date(h.created_at).toLocaleTimeString() }}</span>
            <button
              type="button"
              class="miyo-btn jrp-history-rerun"
              :title="t('java.historyRerun')"
              @click="emit('rerun', h)"
            >
              {{ t('java.historyRerunShort') }}
            </button>
          </li>
        </ul>
      </template>
    </div>
  </section>
</template>

<style scoped>
.jrp {
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
  min-height: 0;
  padding: 10px 12px;
  border-left: 1px solid var(--border);
}
.jrp-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
}
.jrp-idle {
  font-size: 12px;
  color: var(--text-tertiary, var(--text-secondary));
}
.jrp-truncated {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--warn, #d97706);
}
.jrp-head-actions {
  margin-left: auto;
  display: inline-flex;
  gap: 4px;
}
.jrp-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
.jrp-empty {
  padding: 24px 12px;
  text-align: center;
  font-size: 12px;
  color: var(--text-tertiary, var(--text-secondary));
}
.jrp-result {
  min-height: 0;
}
.jrp-raw {
  margin: 0;
  padding: 8px;
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  font-family: var(--font-mono, monospace);
  font-size: 11.5px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text);
}
.jrp-exception {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.jrp-exc-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 9px;
  border-radius: var(--radius-md);
  background: var(--danger-bg, rgba(239, 68, 68, 0.12));
  color: var(--danger, #ef4444);
  font-size: 12.5px;
  font-weight: 600;
}
.jrp-exc-type {
  font-family: var(--font-mono, monospace);
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.jrp-exc-message {
  margin: 0;
  font-size: 12px;
  color: var(--text);
  padding: 0 2px;
}
.jrp-exc-stack {
  margin: 0;
  padding: 8px;
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-secondary);
}
.jrp-info {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 6px 12px;
  margin: 0;
  font-size: 12px;
}
.jrp-info dt {
  color: var(--text-tertiary, var(--text-secondary));
}
.jrp-info dd {
  margin: 0;
  color: var(--text);
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jrp-history {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jrp-history-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: 11.5px;
}
.jrp-history-sig {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono, monospace);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jrp-history-duration,
.jrp-history-time {
  color: var(--text-tertiary, var(--text-secondary));
  font-size: 10.5px;
  flex-shrink: 0;
}
.jrp-history-rerun {
  flex-shrink: 0;
  font-size: 11px;
  padding: 2px 8px;
}
</style>
