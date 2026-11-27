<script setup lang="ts">
/**
 * BatchReportPanel：批量执行面板（实时进度 + 最终报告 + 历史批次）。
 *
 * - 实时区：计数 / 百分比 / 当前场景 / 逐项状态（数据来自
 *   `miyo:test-batch-progress` 事件，终态后回读 DB 报告）；
 * - 操作：暂停（只停调度未开始项）、继续、取消整批、失败项重试、整批重跑
 *   （重试 / 重跑都新建批次，绝不覆盖历史）；
 * - 历史区：项目最近批次，点击回看报告。
 *
 * 批次明细里的 `execution_record_id` 指向单场景运行历史，点击可跳转回看。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import {
  useTestSuitesStore,
} from '@/domains/integration/stores/testSuites'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import { formatDateTime } from '@/utils/dateTime'
import { formatDurationShort } from '@/utils/format'
import Drawer from '@/components/ui/Drawer.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import IntegrationNavBar from '@/domains/integration/components/integration/IntegrationNavBar.vue'
import type { TestBatchItemStatus, TestBatchStatus } from '@/types/miyoApi.d'

const t = useLocaleStore().t
const store = useTestSuitesStore()
const toast = useToast()

const emit = defineEmits<{
  (e: 'open-run', payload: { scenarioId: string; runId: string }): void
}>()

const batch = computed(() => store.activeBatch)

/** 统一导航层级：集成测试 / 执行历史 / 批次 #xxxx。 */
const navCrumbs = computed(() => {
  const crumbs = [
    { label: t('integration.title') },
    { label: t('integration.navHistory') },
  ]
  if (batch.value) {
    crumbs.push({ label: t('integration.navBatch', { id: batch.value.id.slice(0, 8) }) })
  }
  return crumbs
})

/** 批次详情 → 返回执行历史（面板留在历史列表）；历史态返回 = 关闭面板。 */
function onNavBack(): void {
  if (batch.value) store.showHistory()
  else close()
}

/** 批次状态 → 徽章色调 + 文案。 */
function batchTone(status: TestBatchStatus | undefined): StatusTone {
  switch (status) {
    case 'completed':
      return 'ok'
    case 'failed':
      return 'fail'
    case 'cancelled':
      return 'cancel'
    case 'paused':
      return 'warn'
    case 'interrupted':
      return 'blocked'
    case 'running':
      return 'run'
    default:
      return 'wait'
  }
}

function batchLabel(status: TestBatchStatus | undefined): string {
  switch (status) {
    case 'completed':
      return t('testSuite.statusCompleted')
    case 'failed':
      return t('testSuite.statusFailed')
    case 'cancelled':
      return t('testSuite.statusCancelled')
    case 'paused':
      return t('testSuite.statusPaused')
    case 'interrupted':
      return t('testSuite.statusInterrupted')
    case 'running':
      return t('testSuite.statusRunning')
    default:
      return t('testSuite.statusPending')
  }
}

/** 明细状态 → 徽章色调 + 文案。 */
function itemTone(status: TestBatchItemStatus): StatusTone {
  switch (status) {
    case 'passed':
      return 'ok'
    case 'failed':
      return 'fail'
    case 'cancelled':
      return 'cancel'
    case 'skipped':
      return 'skip'
    case 'interrupted':
      return 'blocked'
    case 'running':
      return 'run'
    default:
      return 'wait'
  }
}

function itemLabel(status: TestBatchItemStatus): string {
  switch (status) {
    case 'passed':
      return t('testSuite.statusPassed')
    case 'failed':
      return t('testSuite.statusFailedItem')
    case 'cancelled':
      return t('testSuite.statusCancelledItem')
    case 'skipped':
      return t('testSuite.statusSkipped')
    case 'interrupted':
      return t('testSuite.statusInterruptedItem')
    case 'running':
      return t('testSuite.statusRunningItem')
    default:
      return t('testSuite.statusPendingItem')
  }
}

/** 当前在途场景（事件优先，其次明细里的 running 行）。 */
const currentName = computed(() => {
  const event = store.liveProgress
  if (event?.current?.scenario_name) return event.current.scenario_name
  return store.activeItems.find((i) => i.status === 'running')?.scenario_name ?? ''
})

async function onPause(): Promise<void> {
  try {
    const ok = await store.pauseBatch()
    if (ok) toast.info(t('testSuite.batchPaused'))
  } catch (err) {
    toast.error(t('testSuite.pauseFail'), { message: errorText(err) })
  }
}

async function onResume(): Promise<void> {
  try {
    await store.resumeBatch()
    toast.success(t('testSuite.batchResumed'))
  } catch (err) {
    toast.error(t('testSuite.resumeFail'), { message: errorText(err) })
  }
}

async function onCancel(): Promise<void> {
  try {
    await store.cancelBatch()
    toast.info(t('testSuite.batchCancelled'))
  } catch (err) {
    toast.error(t('testSuite.cancelFail'), { message: errorText(err) })
  }
}

function onRetryFailed(): void {
  void store
    .retryFailed()
    .catch((err) => toast.error(t('testSuite.startFail'), { message: errorText(err) }))
}

function onRerun(): void {
  void store
    .rerunBatch()
    .catch((err) => toast.error(t('testSuite.startFail'), { message: errorText(err) }))
}

/** 单场景重试：新建只含该场景的批次（不覆盖原记录）。 */
function onRetryItem(scenarioId: string): void {
  const source = batch.value
  if (!source) return
  void store
    .startBatch(
      { scenario_ids: [scenarioId], parent_batch_id: source.id },
      {
        mode: source.mode,
        concurrency: source.concurrency,
        failure_policy: source.failure_policy,
        retry_count: source.retry_count,
        skip_disabled: source.skip_disabled,
      },
    )
    .catch((err) => toast.error(t('testSuite.startFail'), { message: errorText(err) }))
}

function openRun(item: { scenario_id: string; execution_record_id?: string | null }): void {
  if (!item.execution_record_id) return
  emit('open-run', { scenarioId: item.scenario_id, runId: item.execution_record_id })
}

function openHistory(id: string): void {
  void store.openBatch(id).catch(() => {})
}

async function onDeleteBatch(id: string): Promise<void> {
  try {
    await store.removeBatch(id)
    toast.success(t('testSuite.batchDeleted'))
  } catch (err) {
    toast.error(t('testSuite.deleteBatch'), { message: errorText(err) })
  }
}

function close(): void {
  store.closePanel()
}
</script>

<template>
  <Drawer
    :open="store.panelOpen"
    :title="t('testSuite.panelTitle')"
    width="640px"
    @update:open="(v: boolean) => !v && close()"
  >
    <template #header>
      <IntegrationNavBar
        :back-label="batch ? t('integration.navBackHistory') : t('integration.navClose')"
        :crumbs="navCrumbs"
        @back="onNavBack"
      />
    </template>
    <div class="brp-root">
      <template v-if="batch">
        <!-- 汇总 -->
        <section class="brp-summary">
          <div class="brp-summary-head">
            <StatusBadge :tone="batchTone(batch.status)" :label="batchLabel(batch.status)" dot />
            <span class="brp-summary-count">{{ t('testSuite.itemCount', { n: batch.total_count }) }}</span>
            <span class="brp-summary-time">{{ formatDateTime(batch.created_at) }}</span>
          </div>

          <div class="brp-bar" role="progressbar" :aria-valuenow="store.percent" aria-valuemin="0" aria-valuemax="100">
            <div
              class="brp-bar-fill"
              :class="{ fail: store.failedCount > 0 }"
              :style="{ width: `${store.percent}%` }"
            ></div>
          </div>

          <div class="brp-counts">
            <span class="brp-count">{{ t('testSuite.progress', { done: store.progress.done, total: store.progress.total }) }}</span>
            <span class="brp-count ok">{{ t('testSuite.statusPassed') }} {{ store.progress.passed }}</span>
            <span class="brp-count fail">{{ t('testSuite.statusFailedItem') }} {{ store.progress.failed }}</span>
            <span class="brp-count skip">{{ t('testSuite.statusSkipped') }} {{ store.progress.skipped }}</span>
            <span class="brp-count cancel">{{ t('testSuite.statusCancelledItem') }} {{ store.progress.cancelled }}</span>
          </div>

          <p v-if="store.isRunning && currentName" class="brp-current">
            {{ t('testSuite.runningCurrent', { name: currentName }) }}
          </p>
          <p v-else-if="batch.status === 'interrupted'" class="brp-current warn">
            {{ t('testSuite.batchInterrupted') }}
          </p>

          <!-- 操作 -->
          <div class="brp-actions">
            <button
              v-if="store.isRunning && !store.isPaused"
              class="miyo-btn miyo-btn-sm"
              type="button"
              @click="onPause"
            >
              <Icon name="pause" :size="12" />{{ t('testSuite.pause') }}
            </button>
            <button
              v-if="store.canResume"
              class="miyo-btn miyo-btn-sm miyo-btn-primary"
              type="button"
              @click="onResume"
            >
              <Icon name="play" :size="12" />{{ t('testSuite.resume') }}
            </button>
            <button
              v-if="store.isRunning || store.canResume"
              class="miyo-btn miyo-btn-sm miyo-btn-danger-solid"
              type="button"
              @click="onCancel"
            >
              <Icon name="stop" :size="12" />{{ t('testSuite.cancelBatch') }}
            </button>
            <button
              v-if="store.hasReport && store.failedCount > 0"
              class="miyo-btn miyo-btn-sm"
              type="button"
              @click="onRetryFailed"
            >
              <Icon name="refresh" :size="12" />{{ t('testSuite.retryFailed', { n: store.failedCount }) }}
            </button>
            <button
              v-if="store.hasReport"
              class="miyo-btn miyo-btn-sm"
              type="button"
              @click="onRerun"
            >
              <Icon name="refresh" :size="12" />{{ t('testSuite.rerun') }}
            </button>
          </div>
        </section>

        <!-- 逐项明细 -->
        <section class="brp-items">
          <div
            v-for="item in store.activeItems"
            :key="item.id"
            class="brp-item"
            :class="item.status"
          >
            <StatusBadge :tone="itemTone(item.status)" :label="itemLabel(item.status)" />
            <div class="brp-item-main">
              <span class="brp-item-name" :title="item.scenario_name">{{ item.scenario_name }}</span>
              <span v-if="item.attempt > 1" class="brp-item-meta">
                {{ t('testSuite.itemAttempt', { n: item.attempt }) }}
              </span>
              <span v-if="item.duration_ms != null" class="brp-item-meta">
                {{ formatDurationShort(item.duration_ms) }}
              </span>
              <span v-if="item.error_message" class="brp-item-error" :title="item.error_message">
                {{ item.error_message }}
              </span>
            </div>
            <span class="brp-item-tools">
              <IconButton
                v-if="item.execution_record_id"
                name="history"
                :size="13"
                :title="t('testSuite.itemRun')"
                @click="openRun(item)"
              />
              <IconButton
                v-if="item.status === 'failed' || item.status === 'interrupted'"
                name="refresh"
                :size="13"
                :title="t('testSuite.retryOne')"
                @click="onRetryItem(item.scenario_id)"
              />
            </span>
          </div>
        </section>
      </template>

      <p v-else class="brp-empty">{{ t('testSuite.noHistory') }}</p>

      <!-- 历史批次 -->
      <section class="brp-history">
        <h3 class="brp-history-title">{{ t('testSuite.history') }}</h3>
        <p v-if="!store.batches.length" class="brp-empty">{{ t('testSuite.noHistory') }}</p>
        <template v-else>
          <div
            v-for="b in store.batches"
            :key="b.id"
            class="brp-history-row"
            :class="{ active: b.id === store.activeBatchId }"
            role="button"
            tabindex="0"
            @click="openHistory(b.id)"
            @keydown.enter="openHistory(b.id)"
          >
            <StatusBadge :tone="batchTone(b.status)" :label="batchLabel(b.status)" />
            <span class="brp-history-time">{{ formatDateTime(b.created_at) }}</span>
            <span class="brp-history-counts">
              {{ t('testSuite.statusPassed') }} {{ b.passed_count }} ·
              {{ t('testSuite.statusFailedItem') }} {{ b.failed_count }}
            </span>
            <Popconfirm
              :title="t('testSuite.deleteBatch')"
              :confirm-text="t('common.delete')"
              @confirm="onDeleteBatch(b.id)"
            >
              <IconButton name="trash" :size="12" :title="t('testSuite.deleteBatch')" @click.stop />
            </Popconfirm>
          </div>
        </template>
      </section>
    </div>
  </Drawer>
</template>

<style scoped>
.brp-root {
  display: flex;
  flex-direction: column;
  gap: 20px;
  padding: 4px 2px 24px;
}

.brp-summary {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
}

.brp-summary-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.brp-summary-count,
.brp-summary-time {
  font-size: 12px;
  color: var(--text-3);
}

.brp-summary-time {
  margin-left: auto;
}

.brp-bar {
  height: 6px;
  border-radius: 999px;
  background: var(--bg-hover);
  overflow: hidden;
}

.brp-bar-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width 0.25s ease;
}

.brp-bar-fill.fail {
  background: var(--danger);
}

.brp-counts {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  font-size: 12px;
  color: var(--text-3);
}

.brp-count.ok {
  color: var(--success);
}

.brp-count.fail {
  color: var(--danger);
}

.brp-current {
  margin: 0;
  font-size: 12px;
  color: var(--text-1);
}

.brp-current.warn {
  color: var(--warning);
}

.brp-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.brp-items {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.brp-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}

.brp-item-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.brp-item-name {
  font-size: 12px;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brp-item-meta {
  flex: none;
  font-size: 11px;
  color: var(--text-3);
}

.brp-item-error {
  min-width: 0;
  font-size: 11px;
  color: var(--danger);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brp-item-tools {
  display: flex;
  gap: 2px;
  flex: none;
}

.brp-history {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.brp-history-title {
  margin: 0 0 2px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-3);
}

.brp-history-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
}

.brp-history-row:hover,
.brp-history-row.active {
  border-color: var(--border);
  background: var(--bg-hover);
}

.brp-history-time {
  font-size: 11px;
  color: var(--text-3);
}

.brp-history-counts {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-3);
}

.brp-empty {
  margin: 0;
  padding: 16px 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
}
</style>