<script setup lang="ts">
/**
 * RunHistoryPanel：执行历史面板（弹窗；场景级 / 项目级双模式）。
 *
 * 场景级（scenarioId）：某场景的历史（最多 100 条），支持对比与按筛选导出；
 * 项目级（projectId，不传 scenarioId）：项目内全部场景的最近运行流
 * （list_recent_scenario_runs + project_scenario_run_stats 聚合统计条），
 * 行内展示场景名，点击直达对应场景的结果视图（openScenarioRun）。
 *
 * 筛选：执行状态（通过 / 失败 / 取消）× 环境 × 时间范围（+ 项目级按场景）。
 * 历史是不可变执行快照：列表只读摘要（命令层已剥大 JSON 列），删除走
 * delete_scenario_run，删除后本地重载。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import { formatDateTime } from '@/utils/dateTime'
import { formatDurationShort } from '@/utils/format'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Modal from '@/components/ui/Modal.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import type { ScenarioRunSummary } from '@/types/miyoApi.d'

const props = defineProps<{
  open: boolean
  /** 场景级模式：场景 id（此时 projectId 被忽略）。 */
  scenarioId?: string | null
  /** 项目级模式：项目 id（scenarioId 缺省时生效）。 */
  projectId?: string | null
  /** 场景级模式的标题场景名。 */
  scenarioName?: string
}>()

const emit = defineEmits<{
  (e: 'update:open', v: boolean): void
  (e: 'open-run', payload: { scenarioId: string; runId: string }): void
  (e: 'compare'): void
  (e: 'export', payload: { format: 'html' | 'junit_xml'; runIds: string[] }): void
}>()

const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const store = useIntegrationStore()
const api = useMiyoApi()
const toast = useToast()

const isProject = computed(() => !!props.projectId && !props.scenarioId)

// ---------- 数据加载（open 时拉取；序号防乱序） ----------
const runs = ref<ScenarioRunSummary[]>([])
const stats = ref<{ totalRuns: number; okRuns: number; failRuns: number; cancelledRuns: number } | null>(null)
const loading = ref(false)
let loadSeq = 0

const HISTORY_LIMIT = 100

async function reload(): Promise<void> {
  const seq = ++loadSeq
  loading.value = true
  try {
    if (isProject.value) {
      const [list, s] = await Promise.all([
        api.listRecentScenarioRuns(props.projectId!, HISTORY_LIMIT),
        api.projectScenarioRunStats(props.projectId!),
      ])
      if (seq !== loadSeq) return
      runs.value = list
      stats.value = s
    } else {
      const list = await api.listScenarioRuns(props.scenarioId!, HISTORY_LIMIT)
      if (seq !== loadSeq) return
      runs.value = list
    }
  } catch (err) {
    toast.error(t('integration.histLoadFail'), { message: errorText(err) })
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

watch(
  () => props.open,
  (v) => {
    if (v) void reload()
  },
  // immediate：以 open=true 直挂（测试 / 复用）也能加载
  { immediate: true },
)

// ---------- 环境名映射（已删环境回退短 id） ----------
const envNameById = computed(() => {
  const m = new Map<string, string>()
  for (const e of workspace.environments) m.set(e.id, e.name)
  return m
})

function envName(id: string | null | undefined): string | null {
  if (!id) return null
  return envNameById.value.get(id) ?? id.slice(0, 8)
}

/** 场景名映射（项目级模式展示行归属；跟随场景库缓存）。 */
const scenarioNameById = computed(() => {
  const m = new Map<string, string>()
  for (const item of store.libraryItems) m.set(item.id, item.name)
  return m
})

function scenarioLabel(id: string | null | undefined): string {
  if (!id) return t('integration.histUnknownScenario')
  return scenarioNameById.value.get(id) ?? id.slice(0, 8)
}

// ---------- 筛选（状态 × 环境 × 时间（× 场景），客户端过滤；上限 100 条内全量判断） ----------
const statusFilter = ref<'all' | 'ok' | 'fail' | 'cancel'>('all')
const envFilter = ref<string>('all')
const timeFilter = ref<'all' | '24h' | '7d' | '30d'>('all')
/** 项目级模式：按场景过滤。 */
const scenarioFilter = ref<string>('all')

/** 出现过的环境 id（下拉选项；新→旧去重保序）。 */
const envOptions = computed(() => {
  const seen: string[] = []
  for (const r of runs.value) {
    if (r.environment_id && !seen.includes(r.environment_id)) seen.push(r.environment_id)
  }
  return seen
})

/** 项目级模式：出现过的场景 id（下拉选项）。 */
const scenarioOptions = computed(() => {
  const seen: string[] = []
  for (const r of runs.value) {
    if (r.scenario_id && !seen.includes(r.scenario_id)) seen.push(r.scenario_id)
  }
  return seen
})

const TIME_MS: Record<string, number> = {
  '24h': 24 * 3600_000,
  '7d': 7 * 24 * 3600_000,
  '30d': 30 * 24 * 3600_000,
}

function runResult(r: ScenarioRunSummary): 'ok' | 'fail' | 'cancel' {
  if (r.cancelled) return 'cancel'
  return r.fail_count > 0 ? 'fail' : 'ok'
}

const filtered = computed(() => {
  const now = Date.now()
  return runs.value.filter((r) => {
    if (statusFilter.value !== 'all' && runResult(r) !== statusFilter.value) return false
    if (envFilter.value === 'none' && r.environment_id) return false
    if (envFilter.value !== 'all' && envFilter.value !== 'none' && r.environment_id !== envFilter.value) {
      return false
    }
    if (isProject.value && scenarioFilter.value !== 'all' && r.scenario_id !== scenarioFilter.value) {
      return false
    }
    if (timeFilter.value !== 'all') {
      const span = TIME_MS[timeFilter.value] ?? 0
      if (Date.parse(r.finished_at) < now - span) return false
    }
    return true
  })
})

// ---------- 删除 ----------
async function onDelete(runId: string): Promise<void> {
  try {
    const ok = await api.deleteScenarioRun(runId)
    if (ok) {
      toast.success(t('integration.histDeleted'))
      await reload()
    }
  } catch (err) {
    toast.error(t('integration.histLoadFail'), { message: errorText(err) })
  }
}

/** 打开详情：场景级回宿主结果视图；项目级直达对应场景（openScenarioRun）。 */
function openRun(r: ScenarioRunSummary): void {
  emit('open-run', { scenarioId: r.scenario_id, runId: r.id })
  emit('update:open', false)
}
</script>

<template>
  <Modal
    :open="props.open"
    :title="isProject ? t('integration.histProjectTitle') : t('integration.histPanelTitle', { name: props.scenarioName ?? '' })"
    width="640px"
    @update:open="(v) => emit('update:open', v)"
  >
    <div class="hist-body">
      <!-- 筛选栏：状态 × 环境 × 时间（× 项目级场景） -->
      <div class="hist-filters">
        <CustomSelect
          class="hist-filter"
          :model-value="String(statusFilter)"
          :options="[
            { value: 'all', label: t('integration.histStatusAll') },
            { value: 'ok', label: t('integration.histStatusOk') },
            { value: 'fail', label: t('integration.histStatusFail') },
            { value: 'cancel', label: t('integration.histStatusCancel') },
          ]"
          size="sm"
          :pop-min-width="110"
          @update:model-value="(v) => (statusFilter = String(v ?? 'all') as 'all' | 'ok' | 'fail' | 'cancel')"
        />
        <CustomSelect
          class="hist-filter"
          :model-value="envFilter"
          :options="[
            { value: 'all', label: t('integration.histEnvAll') },
            { value: 'none', label: t('integration.histEnvNone') },
            ...envOptions.map((id) => ({ value: id, label: envName(id) ?? id })),
          ]"
          size="sm"
          :pop-min-width="130"
          @update:model-value="(v) => (envFilter = String(v ?? 'all'))"
        />
        <CustomSelect
          v-if="isProject"
          class="hist-filter"
          :model-value="scenarioFilter"
          :options="[
            { value: 'all', label: t('integration.histScenarioAll') },
            ...scenarioOptions.map((id) => ({ value: id, label: scenarioLabel(id) })),
          ]"
          size="sm"
          :pop-min-width="130"
          @update:model-value="(v) => (scenarioFilter = String(v ?? 'all'))"
        />
        <CustomSelect
          class="hist-filter"
          :model-value="String(timeFilter)"
          :options="[
            { value: 'all', label: t('integration.histTimeAll') },
            { value: '24h', label: t('integration.histTime24h') },
            { value: '7d', label: t('integration.histTime7d') },
            { value: '30d', label: t('integration.histTime30d') },
          ]"
          size="sm"
          :pop-min-width="110"
          @update:model-value="(v) => (timeFilter = String(v ?? 'all') as 'all' | '24h' | '7d' | '30d')"
        />
        <div class="hist-spacer"></div>
        <template v-if="!isProject">
          <button class="miyo-btn miyo-btn-sm" type="button" :disabled="filtered.length < 2" :title="t('integration.histCompareTip')" @click="emit('compare'); emit('update:open', false)">
            <Icon name="layout-grid" :size="12" /> {{ t('integration.compare') }}
          </button>
          <button
            class="miyo-btn miyo-btn-sm"
            type="button"
            :disabled="!filtered.length"
            :title="t('integration.histExportTip')"
            @click="emit('export', { format: 'html', runIds: filtered.map((r) => r.id) }); emit('update:open', false)"
          >
            <Icon name="download" :size="12" /> {{ t('integration.reportHtml') }}
          </button>
          <button
            class="miyo-btn miyo-btn-sm"
            type="button"
            :disabled="!filtered.length"
            :title="t('integration.histExportTip')"
            @click="emit('export', { format: 'junit_xml', runIds: filtered.map((r) => r.id) }); emit('update:open', false)"
          >
            <Icon name="download" :size="12" /> {{ t('integration.reportJunit') }}
          </button>
        </template>
      </div>

      <!-- 统计条：项目级聚合（总数 / 通过 / 失败 / 取消）；场景级为筛选计数 -->
      <p class="hist-count num">
        <template v-if="isProject && stats">
          {{ t('integration.histStatsLine', { total: stats.totalRuns, ok: stats.okRuns, fail: stats.failRuns, cancel: stats.cancelledRuns }) }} ·
        </template>
        {{ t('integration.histCount', { n: filtered.length, m: runs.length }) }}
      </p>

      <!-- 记录列表 -->
      <div v-if="filtered.length" class="hist-list">
        <div v-for="r in filtered" :key="r.id" class="hist-row" role="button" tabindex="0" @click="openRun(r)" @keydown.enter="openRun(r)">
          <span class="hist-dot" :class="runResult(r)" aria-hidden="true"></span>
          <div class="hist-main">
            <span v-if="isProject" class="hist-scenario" :title="scenarioLabel(r.scenario_id)">{{ scenarioLabel(r.scenario_id) }}</span>
            <span class="hist-time">{{ formatDateTime(r.finished_at) }}</span>
            <span v-if="envName(r.environment_id)" class="hist-env">{{ envName(r.environment_id) }}</span>
            <span class="hist-nums num">
              <span class="hist-ok">✓{{ r.ok_count }}</span>
              <span v-if="r.fail_count > 0" class="hist-fail">✗{{ r.fail_count }}</span>
              <span v-if="r.skipped_count > 0" class="hist-skip">{{ t('scenario.summarySkip', { n: r.skipped_count }) }}</span>
              <span v-if="r.cancelled" class="hist-skip">{{ t('scenario.cancelled') }}</span>
              <span v-if="r.stopped_on_failure" class="hist-skip">{{ t('scenario.stoppedAt') }}</span>
            </span>
          </div>
          <span class="hist-dur num">{{ formatDurationShort(r.duration_ms ?? 0) }}</span>
          <span class="hist-actions" @click.stop>
            <Popconfirm :title="t('integration.histDeleteConfirm')" :confirm-text="t('common.delete')" @confirm="onDelete(r.id)">
              <IconButton name="trash" :size="13" :title="t('integration.histRunDelete')" @click.stop />
            </Popconfirm>
          </span>
        </div>
      </div>
      <p v-else-if="loading" class="hist-empty">{{ t('common.loading') }}</p>
      <p v-else class="hist-empty">{{ t('integration.historyEmpty') }}</p>
    </div>
  </Modal>
</template>

<style scoped>
.hist-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.hist-filters {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.hist-filter {
  width: 110px;
}

.hist-spacer {
  flex: 1;
}

.hist-count {
  margin: 0;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.hist-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 380px;
  overflow-y: auto;
}

.hist-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  cursor: pointer;
  min-width: 0;
}

.hist-row:hover {
  border-color: var(--border-strong);
}

.hist-dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.5;
}

.hist-dot.ok {
  background: var(--success);
  opacity: 1;
}

.hist-dot.fail {
  background: var(--danger);
  opacity: 1;
}

.hist-dot.cancel {
  background: var(--warning);
  opacity: 1;
}

.hist-main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.hist-scenario {
  font-size: var(--fs-xxs);
  font-weight: 500;
  color: var(--text-1);
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hist-time {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-1);
}

.hist-env {
  flex: none;
  font-size: 10px;
  padding: 1px 8px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-2);
  font-family: var(--font-mono);
  max-width: 110px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hist-nums {
  display: inline-flex;
  gap: 6px;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.hist-ok {
  color: var(--success);
}

.hist-fail {
  color: var(--danger);
}

.hist-dur {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.hist-actions {
  flex: none;
  display: none;
  align-items: center;
  gap: 2px;
}

.hist-row:hover .hist-actions {
  display: inline-flex;
}

.hist-empty {
  margin: 0;
  padding: 16px 8px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  text-align: center;
}
</style>
