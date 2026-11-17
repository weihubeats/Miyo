<script setup lang="ts">
/**
 * JavaHistoryDrawer：调用历史抽屉（时间倒序）。
 *
 * 过滤：类 / 方法名文本搜索 + 状态筛选（全部 / 成功 / 失败）。
 * 每条：状态徽标 + 方法签名 + 耗时 + 时间；操作：重新执行（回填参数）/
 * 复制参数 / 删除。展开详情的参数做敏感键脱敏展示（原始值不动）。
 * 支持清空项目历史（Popconfirm 确认）。
 */
import { computed, ref } from 'vue'
import Drawer from '@/components/ui/Drawer.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { maskArgs } from '@/domains/java-debug/utils/mask'
import type { JavaInvocationRecord } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const props = defineProps<{
  open: boolean
  records: JavaInvocationRecord[]
  loading: boolean
}>()

const emit = defineEmits<{
  close: []
  rerun: [record: JavaInvocationRecord]
  changed: []
}>()

const locale = useLocaleStore()
const t = locale.t
const toast = useToast()
const api = useMiyoApi()

/** 展开中记录 id（点击行展开完整参数 / 异常）。 */
const expandedId = ref<string | null>(null)

// ---------- 搜索与状态过滤 ----------
const search = ref('')
const statusFilter = ref<'all' | 'success' | 'failed'>('all')

const filteredRecords = computed(() => {
  const term = search.value.trim().toLowerCase()
  return props.records.filter((r) => {
    if (statusFilter.value === 'success' && r.status !== 'success') return false
    if (statusFilter.value === 'failed' && r.status === 'success') return false
    if (!term) return true
    return (
      r.class_name.toLowerCase().includes(term) ||
      r.method_id.toLowerCase().includes(term) ||
      r.method_signature.toLowerCase().includes(term)
    )
  })
})

function statusTone(status: string): StatusTone {
  switch (status) {
    case 'success':
      return 'ok'
    case 'failed':
      return 'fail'
    case 'timeout':
      return 'warn'
    case 'cancelled':
      return 'cancel'
    default:
      return 'fail'
  }
}

function statusLabel(status: string): string {
  switch (status) {
    case 'success':
      return t('java.statusSuccess')
    case 'failed':
      return t('java.statusFailed')
    case 'timeout':
      return t('java.statusTimeout')
    case 'cancelled':
      return t('java.statusCancelled')
    default:
      return status
  }
}

/** 展开详情的参数文本（对象键 / 值形状脱敏；仅展示层，复制走原文）。 */
function argsText(record: JavaInvocationRecord): string {
  try {
    const masked = maskArgs(record.arguments ?? [])
    return JSON.stringify(masked, null, 2)
  } catch {
    return String(record.arguments)
  }
}

async function copyArgs(record: JavaInvocationRecord): Promise<void> {
  try {
    await navigator.clipboard.writeText(argsText(record))
    toast.success(t('java.copied'))
  } catch {
    toast.error(t('java.copyFailed'))
  }
}

async function remove(record: JavaInvocationRecord): Promise<void> {
  try {
    await api.deleteJavaInvocation(record.id)
    toast.success(t('java.historyDeleted'))
    emit('changed')
  } catch (e) {
    toast.error(errorText(e))
  }
}

async function clearAll(): Promise<void> {
  const projectId = props.records[0]?.project_id
  if (!projectId) return
  try {
    await api.clearJavaHistory(projectId)
    toast.success(t('java.historyCleared'))
    emit('changed')
  } catch (e) {
    toast.error(errorText(e))
  }
}

function fmtTime(record: JavaInvocationRecord): string {
  try {
    return new Date(record.created_at).toLocaleString()
  } catch {
    return record.created_at
  }
}
</script>

<template>
  <Drawer :open="open" :title="t('java.historyTitle')" width="480px" @close="emit('close')">
    <div class="jhd-toolbar">
      <span class="jhd-count">{{ t('java.historyCount', { count: records.length }) }}</span>
      <Popconfirm v-if="records.length > 0" :confirm-text="t('java.historyClearConfirm')" @confirm="clearAll">
        <button type="button" class="miyo-btn">
          <Icon name="trash" :size="12" />
          {{ t('java.historyClear') }}
        </button>
      </Popconfirm>
    </div>

    <!-- 搜索 + 状态过滤 -->
    <div v-if="records.length > 0" class="jhd-filters">
      <input
        v-model="search"
        class="miyo-input jhd-search"
        type="search"
        :placeholder="t('java.historySearch')"
        data-testid="java-history-search"
      />
      <select v-model="statusFilter" class="miyo-input jhd-status" data-testid="java-history-status">
        <option value="all">{{ t('java.historyFilterAll') }}</option>
        <option value="success">{{ t('java.historyFilterSuccess') }}</option>
        <option value="failed">{{ t('java.historyFilterFailed') }}</option>
      </select>
    </div>

    <EmptyState
      v-if="!loading && records.length === 0"
      icon="history"
      :title="t('java.historyEmptyTitle')"
      :description="t('java.historyEmptyDesc')"
    />
    <EmptyState
      v-else-if="records.length > 0 && filteredRecords.length === 0"
      icon="search"
      :title="t('java.historyEmptyTitle')"
      :description="t('java.historySearch')"
    />

    <ul class="jhd-list" data-testid="java-history-list">
      <li v-for="r in filteredRecords" :key="r.id">
        <article
          class="jhd-item"
          :class="{ expanded: expandedId === r.id }"
          @click="expandedId = expandedId === r.id ? null : r.id"
        >
          <div class="jhd-row">
            <StatusBadge :tone="statusTone(r.status)" :label="statusLabel(r.status)" />
            <span class="jhd-method" :title="r.method_id">{{ r.method_signature || r.method_id }}</span>
            <span v-if="r.duration_ms != null" class="jhd-duration">{{ r.duration_ms }} ms</span>
          </div>
          <div class="jhd-row jhd-meta">
            <span class="jhd-time">{{ fmtTime(r) }}</span>
            <span class="jhd-actions" @click.stop>
              <IconButton name="play" :title="t('java.historyRerun')" @click="emit('rerun', r)" />
              <IconButton name="copy" :title="t('java.historyCopyArgs')" @click="copyArgs(r)" />
              <IconButton name="trash" tone="danger" :title="t('common.delete')" @click="remove(r)" />
            </span>
          </div>
          <div v-if="expandedId === r.id" class="jhd-detail">
            <pre class="jhd-json">{{ argsText(r) }}</pre>
            <template v-if="r.exception">
              <p class="jhd-exc-type">{{ r.exception.exception_type }}: {{ r.exception.message ?? '' }}</p>
              <pre class="jhd-json jhd-json-exc">{{ r.exception.stack_trace.join('\n') }}</pre>
            </template>
          </div>
        </article>
      </li>
    </ul>
  </Drawer>
</template>

<style scoped>
.jhd-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 0 10px;
}
.jhd-count {
  font-size: 12px;
  color: var(--text-tertiary, var(--text-secondary));
}
.jhd-filters {
  display: flex;
  gap: 8px;
  padding: 0 0 10px;
}
.jhd-search {
  flex: 1;
  min-width: 0;
}
.jhd-status {
  width: 96px;
  flex-shrink: 0;
}
.jhd-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.jhd-item {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jhd-item:hover {
  background: var(--bg-hover);
}
.jhd-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.jhd-method {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
}
.jhd-duration {
  font-size: 11px;
  color: var(--text-tertiary, var(--text-secondary));
  font-family: var(--font-mono, monospace);
}
.jhd-meta {
  justify-content: space-between;
}
.jhd-time {
  font-size: 11px;
  color: var(--text-tertiary, var(--text-secondary));
}
.jhd-actions {
  display: inline-flex;
  gap: 2px;
}
.jhd-detail {
  border-top: 1px solid var(--border);
  padding-top: 6px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jhd-json {
  margin: 0;
  padding: 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 220px;
  overflow: auto;
  color: var(--text-secondary);
}
.jhd-json-exc {
  color: var(--danger, #ef4444);
}
.jhd-exc-type {
  margin: 0;
  font-size: 12px;
  color: var(--danger, #ef4444);
  font-family: var(--font-mono, monospace);
}
</style>
