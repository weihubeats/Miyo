<script setup lang="ts">
/**
 * JavaProcessPanel：本机 Java 进程列表（Attach 工作台左栏）。
 * 自动刷新开关 / 搜索 / 手动 pid / 状态徽章（attachable / attached / blocked）。
 * 不可 attach 的进程给出诚实原因（他人进程 / 非 HotSpot / 工具链缺失）。
 */
import { computed, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaProcessInfo, JavaProcessListSummary } from '@/types/miyoApi'

defineProps<{
  summary: JavaProcessListSummary | null
  processes: JavaProcessInfo[]
  loading: boolean
  search: string
  activePid: number | null
  attachedPids: number[]
  autoRefresh: boolean
  componentsDownloading: boolean
}>()

const emit = defineEmits<{
  (e: 'update:search', value: string): void
  (e: 'update:autoRefresh', value: boolean): void
  (e: 'refresh'): void
  (e: 'select', pid: number): void
  (e: 'attach', pid: number): void
  (e: 'manualPid', pid: number): void
  (e: 'downloadComponents'): void
}>()

const t = useLocaleStore().t
const manualPid = ref('')
const manualError = ref('')

const manualPidNumber = computed(() => {
  const raw = manualPid.value.trim()
  if (!raw) return null
  const n = Number(raw)
  return Number.isInteger(n) && n > 0 ? n : null
})

function submitManualPid(): void {
  if (manualPidNumber.value == null) {
    manualError.value = t('java2.invalidPid')
    return
  }
  manualError.value = ''
  emit('manualPid', manualPidNumber.value)
}

function statusTone(p: JavaProcessInfo): StatusTone {
  if (p.attached) return 'ok'
  if (p.attachable) return 'wait'
  return 'fail'
}

function statusLabel(p: JavaProcessInfo): string {
  if (p.attached) return t('java2.procAttached')
  if (p.attachable) return t('java2.procAttachable')
  return t('java2.procBlocked')
}

function kindLabel(p: JavaProcessInfo): string {
  if (p.kind === 'jdk') return 'JDK'
  if (p.kind === 'jre') return 'JRE'
  return p.java_version || 'Java'
}
</script>

<template>
  <div class="jpp" data-testid="java-process-panel">
    <div class="jpp-toolbar">
      <input
        class="miyo-input jpp-search"
        type="text"
        :value="search"
        :placeholder="t('java2.procSearchPlaceholder')"
        data-testid="java-process-search"
        @input="emit('update:search', ($event.target as HTMLInputElement).value)"
      />
      <IconButton
        name="refresh"
        :title="t('java2.refresh')"
        :disabled="loading"
        data-testid="java-process-refresh"
        @click="emit('refresh')"
      />
    </div>

    <label class="jpp-autorefresh">
      <input
        type="checkbox"
        :checked="autoRefresh"
        data-testid="java-process-autorefresh"
        @change="emit('update:autoRefresh', ($event.target as HTMLInputElement).checked)"
      />
      <span>{{ t('java2.autoRefresh') }}</span>
    </label>

    <!-- 工具链诊断（无 JDK / jar 缺失时给出可操作提示） -->
    <div v-if="summary && !summary.launcher_ready" class="jpp-hint danger">
      {{ t('java2.jarMissing') }}
      <button
        class="miyo-btn miyo-btn-sm jpp-download-btn"
        type="button"
        :disabled="componentsDownloading"
        @click="emit('downloadComponents')"
      >
        {{
          componentsDownloading
            ? t('java2.componentsDownloading')
            : t('java2.componentsDownload')
        }}
      </button>
    </div>
    <div v-else-if="summary && summary.toolchain_count === 0" class="jpp-hint danger">
      {{ t('java2.noToolchain') }}
    </div>
    <div v-for="(hint, i) in summary?.hints ?? []" :key="i" class="jpp-hint">{{ hint }}</div>

    <div class="jpp-list" data-testid="java-process-list">
      <div v-if="loading && processes.length === 0" class="jpp-empty">
        {{ t('java2.scanning') }}
      </div>
      <div v-else-if="processes.length === 0" class="jpp-empty">
        {{ t('java2.noProcesses') }}
      </div>
      <button
        v-for="p in processes"
        :key="p.pid"
        type="button"
        class="jpp-item"
        :class="{ active: p.pid === activePid }"
        :data-testid="`java-process-${p.pid}`"
        :title="p.attach_block_reason ?? ''"
        @click="emit('select', p.pid)"
      >
        <div class="jpp-item-top">
          <span class="jpp-name">{{ p.display_name || `PID ${p.pid}` }}</span>
          <StatusBadge :tone="statusTone(p)" :label="statusLabel(p)" size="sm" />
        </div>
        <div class="jpp-item-meta">
          <span class="jpp-chip">PID {{ p.pid }}</span>
          <span class="jpp-chip">{{ kindLabel(p) }} {{ p.java_version }}</span>
          <span v-if="p.elapsed" class="jpp-chip">{{ p.elapsed }}</span>
        </div>
        <div v-if="p.attach_block_reason" class="jpp-block-reason">
          {{ p.attach_block_reason }}
        </div>
        <div v-if="!p.attached && p.attachable" class="jpp-item-actions">
          <button
            type="button"
            class="miyo-btn miyo-btn-sm miyo-btn-primary"
            data-testid="java-attach-btn"
            @click.stop="emit('attach', p.pid)"
          >
            <Icon name="plug" :size="12" />
            {{ t('java2.attach') }}
          </button>
        </div>
      </button>
    </div>

    <div class="jpp-manual">
      <input
        v-model="manualPid"
        class="miyo-input jpp-manual-input"
        type="text"
        inputmode="numeric"
        :placeholder="t('java2.manualPidPlaceholder')"
        data-testid="java-manual-pid"
        @keydown.enter="submitManualPid"
      />
      <button type="button" class="miyo-btn miyo-btn-sm" :disabled="manualPidNumber == null" @click="submitManualPid">
        {{ t('java2.manualProbe') }}
      </button>
      <span v-if="manualError" class="jpp-manual-error">{{ manualError }}</span>
    </div>
  </div>
</template>

<style scoped>
.jpp {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  gap: 8px;
  padding: 10px;
}
.jpp-toolbar {
  display: flex;
  gap: 6px;
  align-items: center;
}
.jpp-search {
  flex: 1;
  min-width: 0;
  height: 26px;
  font-size: 12px;
}
.jpp-autorefresh {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-secondary);
  cursor: pointer;
}
.jpp-hint {
  font-size: 11px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-secondary);
  line-height: 1.5;
}
.jpp-hint.danger {
  color: var(--danger, #ef4444);
}
.jpp-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jpp-empty {
  font-size: 12px;
  color: var(--text-secondary);
  padding: 16px 8px;
  text-align: center;
}
.jpp-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, transparent);
  cursor: pointer;
  text-align: left;
  color: var(--text);
}
.jpp-item:hover {
  background: var(--bg-hover);
}
.jpp-item.active {
  border-color: var(--primary, #6366f1);
}
.jpp-item-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}
.jpp-name {
  font-size: 12.5px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jpp-item-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.jpp-chip {
  font-size: 10.5px;
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  white-space: nowrap;
}
.jpp-block-reason {
  font-size: 10.5px;
  color: var(--text-secondary);
  line-height: 1.4;
}
.jpp-item-actions {
  display: flex;
  justify-content: flex-end;
}
.jpp-item-actions .miyo-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  width: auto;
}
.jpp-manual {
  display: flex;
  gap: 6px;
  align-items: center;
  flex-shrink: 0;
}
.jpp-manual-input {
  flex: 1;
  min-width: 0;
  height: 26px;
  font-size: 12px;
}
.jpp-manual-error {
  font-size: 10.5px;
  color: var(--danger, #ef4444);
  white-space: nowrap;
}
</style>
