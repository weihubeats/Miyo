<script setup lang="ts">
/**
 * MockPanel：Mock 管理（接口页内嵌）。
 * - 展示项目级 Mock 规则，高亮与当前接口关联的规则；
 * - 「打开 Mock 管理」复用完整 MockRuleDialog（新建 / 编辑 / 删除）；
 * - 请求记录（最近 100 条，环形缓冲）：命中可见化 + 未匹配诊断。
 */
import { onMounted, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { methodTextTone } from '../utils/methodTone'
import { formatTime } from '../utils/dateTime'
import type { Endpoint, MockRequestLog, MockRule } from '../types/miyoApi'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'

const props = defineProps<{ draft: Endpoint | null }>()
const emit = defineEmits<{ openManager: [] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const rules = ref<MockRule[]>([])
const reloading = ref(false)
const logs = ref<MockRequestLog[]>([])
const logsOpen = ref(false)
const logsLoading = ref(false)

async function load(): Promise<void> {
  if (!store.project) return
  try {
    rules.value = (await api.listMockRules(store.project.id)) ?? []
  } catch {
    rules.value = []
  }
}

onMounted(load)

/** 热重载：运行中原子替换定义，无需重启服务（未运行会提示先启动）。 */
async function reload(): Promise<void> {
  if (reloading.value) return
  reloading.value = true
  try {
    const n = await api.mockReload()
    toast.success(t('mockpanel.reloaded', { n }))
  } catch (err) {
    toast.error(t('mockpanel.reloadFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    reloading.value = false
  }
}

/** 与当前接口关联的规则（endpoint_id 匹配）。 */
const related = (): MockRule[] =>
  props.draft ? rules.value.filter((r) => r.endpoint_id === props.draft?.id) : []

// ---------- 请求记录（命中可见化 + 未匹配诊断） ----------
async function loadLogs(): Promise<void> {
  if (logsLoading.value) return
  logsLoading.value = true
  try {
    logs.value = (await api.mockRequestLogs()) ?? []
  } catch {
    logs.value = []
  } finally {
    logsLoading.value = false
  }
}

async function clearLogs(): Promise<void> {
  try {
    await api.mockClearLogs()
    logs.value = []
  } catch (err) {
    toast.error(t('mocklog.clearFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

function toggleLogs(): void {
  logsOpen.value = !logsOpen.value
  if (logsOpen.value) void loadLogs()
}

/** 命中来源文案（后端稳定键 → 本地语言）。 */
function sourceLabel(entry: MockRequestLog): string {
  if (!entry.matched) return t('mocklog.unmatched')
  if (entry.source === 'rule') return entry.source_name
  if (entry.source === 'example') return t('mocklog.sourceExample')
  return t('mocklog.sourceDefault')
}

const others = (): MockRule[] =>
  props.draft ? rules.value.filter((r) => r.endpoint_id !== props.draft?.id) : []
</script>

<template>
  <div class="mkp">
    <div class="mkp-bar">
      <span class="mkp-title">{{ t('mockpanel.title', { n: rules.length }) }}</span>
      <span class="mkp-actions">
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="reloading"
          :title="t('mockpanel.reloadHint')"
          @click="reload"
        >
          <Icon name="refresh" :size="13" /> {{ reloading ? t('mockpanel.reloading') : t('mockpanel.reload') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('openManager')">
          <Icon name="settings" :size="13" /> {{ t('mockpanel.openManager') }}
        </button>
      </span>
    </div>

    <div v-if="related().length" class="mkp-sec">
      <span class="mkp-sec-label">{{ t('mockpanel.thisEndpoint') }}</span>
      <div class="mkp-list">
        <div v-for="r in related()" :key="r.id" class="mkp-row">
          <span class="mkp-method" :class="methodTextTone(r.method)">{{ r.method }}</span>
          <code class="mkp-path">{{ r.path }}</code>
          <span class="mkp-name">{{ r.name }}</span>
          <span class="mkp-status" :class="{ on: r.enabled }">{{ r.enabled ? t('mockpanel.enabled') : t('mockpanel.disabled') }}</span>
          <span class="mkp-code">{{ r.response_status }}</span>
        </div>
      </div>
    </div>

    <div v-if="others().length" class="mkp-sec">
      <span class="mkp-sec-label">{{ t('mockpanel.otherRules') }}</span>
      <div class="mkp-list">
        <div v-for="r in others()" :key="r.id" class="mkp-row">
          <span class="mkp-method" :class="methodTextTone(r.method)">{{ r.method }}</span>
          <code class="mkp-path">{{ r.path }}</code>
          <span class="mkp-name">{{ r.name }}</span>
          <span class="mkp-status" :class="{ on: r.enabled }">{{ r.enabled ? t('mockpanel.enabled') : t('mockpanel.disabled') }}</span>
        </div>
      </div>
    </div>

    <EmptyState
      v-if="!rules.length"
      icon="server"
      compact
      :title="t('mockpanel.empty')"
      :description="t('mockpanel.emptyHint')"
    />

    <!-- 请求记录：环形缓冲最近 100 条（服务运行期驻内存），命中规则与未匹配诊断 -->
    <div class="mkp-sec mkp-logs">
      <button type="button" class="mkp-logs-head" @click="toggleLogs">
        <Icon
          :name="logsOpen ? 'chevrons-down-up' : 'chevrons-up-down'"
          :size="13"
        />
        <span class="mkp-sec-label">{{ t('mocklog.title') }}</span>
        <span v-if="logs.length" class="mkp-logs-count">{{ logs.length }}</span>
        <span class="mkp-logs-actions">
          <IconButton
            v-if="logsOpen && logs.length"
            name="trash"
            :size="12"
            tone="danger"
            :title="t('mocklog.clear')"
            @click.stop="clearLogs"
          />
          <IconButton
            v-if="logsOpen"
            name="refresh"
            :size="12"
            :title="t('common.refresh')"
            @click.stop="loadLogs"
          />
        </span>
      </button>
      <div v-if="logsOpen" class="mkp-list mkp-log-list">
        <div v-if="logsLoading && !logs.length" class="mkp-log-empty">{{ t('common.loading') }}</div>
        <div v-else-if="!logs.length" class="mkp-log-empty">{{ t('mocklog.empty') }}</div>
        <div v-for="(entry, i) in logs.slice(0, 30)" :key="i" class="mkp-row mkp-log-row">
          <span class="mkp-method" :class="methodTextTone(entry.method)">{{ entry.method }}</span>
          <code class="mkp-path">{{ entry.path }}</code>
          <span
            class="mkp-log-source"
            :class="{ miss: !entry.matched }"
            :title="entry.source_name || undefined"
          >{{ sourceLabel(entry) }}</span>
          <span class="mkp-code" :class="{ miss: entry.status >= 400 }">{{ entry.status }}</span>
          <span class="mkp-log-time">{{ formatTime(entry.at, { hour12: false }) }}</span>
        </div>
        <p v-if="logs.length > 30" class="mkp-log-more">{{ t('mocklog.more', { n: logs.length - 30 }) }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mkp {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-width: 720px;
}

.mkp-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.mkp-actions {
  display: inline-flex;
  gap: 8px;
}

.mkp-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
}

.mkp-sec {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.mkp-sec-label {
  font-size: var(--fs-xs);
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

.mkp-list {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.mkp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border-bottom: 1px solid var(--border);
  font-size: 12.5px;
}
.mkp-row:last-child {
  border-bottom: none;
}

.mkp-method {
  flex-shrink: 0;
  width: 46px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
}

.mkp-path {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
}

.mkp-name {
  flex: 1;
  min-width: 0;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mkp-status {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.mkp-status.on {
  color: var(--ok);
}

.mkp-code {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* ---- 请求记录 ---- */
.mkp-logs-head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 2px 0;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
}

.mkp-logs-head:hover .mkp-sec-label {
  color: var(--text-1);
}

.mkp-logs-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  background: var(--bg-hover);
  border-radius: 999px;
  padding: 0 7px;
}

.mkp-logs-actions {
  margin-left: auto;
  display: inline-flex;
  gap: 2px;
}

.mkp-log-list {
  margin-top: 6px;
}

.mkp-log-row {
  opacity: 0.92;
}

.mkp-log-source {
  flex: none;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.mkp-log-source.miss {
  color: var(--danger);
}

.mkp-code.miss {
  color: var(--danger);
}

.mkp-log-time {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-3);
}

.mkp-log-empty,
.mkp-log-more {
  margin: 4px 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>