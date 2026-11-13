<script setup lang="ts">
/**
 * RunCompareModal：两次执行结果的比较视图。
 *
 * - 选基准运行与对比运行（历史下拉，新→旧）；
 * - 按步骤位次逐行对比：状态（差异高亮）/ 断言通过数 / 状态码 / 耗时；
 * - 数据从 get_scenario_run 回读（历史含响应快照，但比较只看摘要级字段）。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { useWorkspaceStore } from '../../stores/workspace'
import Modal from '../ui/Modal.vue'
import CustomSelect from '../ui/CustomSelect.vue'
import Icon from '../ui/Icon.vue'
import { formatDateTime } from '../../utils/dateTime'
import { formatDurationShort } from '../../utils/format'
import { toneOf } from '../../composables/useScenarioRun'
import type { ScenarioRunDetail, ScenarioRunSummary, ScenarioStepResult } from '../../types/miyoApi'
import { errorText } from '../../utils/errorText'

const props = defineProps<{ scenarioId: string }>()
const open = defineModel<boolean>('open', { default: false })

const t = useLocaleStore().t
const api = useMiyoApi()
const toast = useToast()
const workspace = useWorkspaceStore()

const runs = ref<ScenarioRunSummary[]>([])
const runAId = ref<string | null>(null)
const runBId = ref<string | null>(null)
const detailA = ref<ScenarioRunDetail | null>(null)
const detailB = ref<ScenarioRunDetail | null>(null)
const loading = ref(false)

/** 加载序号：快速切换下拉 / 弹窗关闭后，迟到响应不得覆盖新状态
 * （旧详情与选择代次错位）；loading 只反映最新一次加载。 */
let detailSeq = 0

async function loadRuns(): Promise<void> {
  if (!open.value) return
  try {
    runs.value = await api.listScenarioRuns(props.scenarioId)
    if (!runAId.value && runs.value[1]) runAId.value = runs.value[1].id
    if (!runBId.value && runs.value[0]) runBId.value = runs.value[0].id
  } catch {
    runs.value = []
  }
}

watch(open, (v) => {
  if (v) {
    void loadRuns()
  } else {
    // 关闭即作废在途加载：迟到的 getScenarioRun 不再 setState。
    detailSeq++
    loading.value = false
  }
})

watch([runAId, runBId], () => {
  void loadDetails()
})

async function loadDetails(): Promise<void> {
  const seq = ++detailSeq
  loading.value = true
  try {
    const a = runAId.value ? await api.getScenarioRun(runAId.value) : null
    const b = runBId.value ? await api.getScenarioRun(runBId.value) : null
    // 两次请求并行在途时，只有最新一次选择可以落状态（乱序丢弃）。
    if (seq !== detailSeq || !open.value) return
    detailA.value = a
    detailB.value = b
  } catch (err) {
    if (seq !== detailSeq || !open.value) return
    toast.error(t('integration.historyMissing'), {
      message: errorText(err),
    })
  } finally {
    if (seq === detailSeq) loading.value = false
  }
}

const runOptions = computed(() =>
  runs.value.map((r) => ({
    value: r.id,
    label: `${formatDateTime(r.finished_at)} · ${t('scenario.summaryOk', { n: r.ok_count })}${r.fail_count ? ` / ${t('scenario.summaryFail', { n: r.fail_count })}` : ''}`,
  })),
)

interface CompareRow {
  index: number
  nameA: string
  nameB: string
  statusA: string
  statusB: string
  diff: boolean
  assertA: string
  assertB: string
  codeA: number | null
  codeB: number | null
  timeA: number | null
  timeB: number | null
}

const rows = computed<CompareRow[]>(() => {
  const ra = detailA.value?.results ?? []
  const rb = detailB.value?.results ?? []
  const len = Math.max(ra.length, rb.length)
  const out: CompareRow[] = []
  for (let i = 0; i < len; i++) {
    const a = ra[i]
    const b = rb[i]
    const statusA = a ? toneOf(a) : 'none'
    const statusB = b ? toneOf(b) : 'none'
    out.push({
      index: i,
      nameA: a?.endpoint_name ?? '',
      nameB: b?.endpoint_name ?? '',
      statusA,
      statusB,
      diff: statusA !== statusB || (a?.status ?? null) !== (b?.status ?? null),
      assertA: assertText(a),
      assertB: assertText(b),
      codeA: a?.status ?? null,
      codeB: b?.status ?? null,
      timeA: a?.duration_ms ?? null,
      timeB: b?.duration_ms ?? null,
    })
  }
  return out
})

function assertText(r: ScenarioStepResult | undefined): string {
  if (!r) return '—'
  const passed = r.outcomes.filter((o) => o.passed).length
  return `${passed}/${r.outcomes.length}`
}

function statusLabel(s: string): string {
  if (s === 'none') return t('integration.compareNotRun')
  if (s === 'success') return t('integration.statusSuccess')
  if (s === 'failed') return t('integration.statusFailed')
  if (s === 'blocked') return t('integration.statusBlocked')
  if (s === 'skipped') return t('integration.statusSkipped')
  if (s === 'cancelled') return t('scenario.stepStatusCancelled')
  return s
}

const summary = computed(() => {
  const a = detailA.value
  const b = detailB.value
  if (!a || !b) return null
  const diffs = rows.value.filter((r) => r.diff).length
  return {
    durationA: a.duration_ms ?? 0,
    durationB: b.duration_ms ?? 0,
    diffs,
  }
})

void workspace
</script>

<template>
  <Modal v-model:open="open" :title="t('integration.compareTitle')" width="720px">
    <div class="cmp-picks">
      <div class="cmp-pick">
        <span class="cmp-label">{{ t('integration.compareRunA') }}</span>
        <CustomSelect
          v-model="runAId"
          :options="runOptions"
          size="sm"
          :pop-min-width="260"
        />
      </div>
      <Icon name="arrow-up-right" :size="14" class="cmp-arrow" />
      <div class="cmp-pick">
        <span class="cmp-label">{{ t('integration.compareRunB') }}</span>
        <CustomSelect
          v-model="runBId"
          :options="runOptions"
          size="sm"
          :pop-min-width="260"
        />
      </div>
    </div>

    <div v-if="summary" class="cmp-summary">
      <span>{{ t('integration.metaDuration') }}: {{ formatDurationShort(summary.durationA) }} → {{ formatDurationShort(summary.durationB) }}</span>
      <span :class="summary.diffs ? 'cmp-diff-count' : 'cmp-same-count'">
        {{ summary.diffs ? t('integration.compareChanged') : t('integration.compareSame') }} · {{ summary.diffs }}
      </span>
    </div>

    <div class="cmp-table">
      <div class="cmp-row cmp-head">
        <span class="c-idx">#</span>
        <span class="c-name">{{ t('integration.compareStepCol') }}</span>
        <span class="c-status">{{ t('integration.compareStatusCol') }}</span>
        <span class="c-code">{{ t('integration.assertType.status') }}</span>
        <span class="c-assert">{{ t('integration.compareAssertCol') }}</span>
        <span class="c-time">{{ t('integration.compareTimeCol') }}</span>
      </div>
      <div
        v-for="r in rows"
        :key="r.index"
        class="cmp-row"
        :class="{ diff: r.diff }"
      >
        <span class="c-idx num">{{ r.index + 1 }}</span>
        <span class="c-name" :title="r.nameB || r.nameA">{{ r.nameB || r.nameA }}</span>
        <span class="c-status">
          <span class="cmp-status" :class="{ bad: r.statusA !== r.statusB }">{{ statusLabel(r.statusA) }}</span>
          <span class="cmp-sep">→</span>
          <span class="cmp-status" :class="{ bad: r.statusA !== r.statusB }">{{ statusLabel(r.statusB) }}</span>
        </span>
        <span class="c-code num" :class="{ bad: r.codeA !== r.codeB }">
          {{ r.codeA ?? '—' }} → {{ r.codeB ?? '—' }}
        </span>
        <span class="c-assert num" :class="{ bad: r.assertA !== r.assertB }">
          {{ r.assertA }} → {{ r.assertB }}
        </span>
        <span class="c-time num">
          {{ r.timeA != null ? formatDurationShort(r.timeA) : '—' }} →
          {{ r.timeB != null ? formatDurationShort(r.timeB) : '—' }}
        </span>
      </div>
      <p v-if="!rows.length && !loading" class="cmp-empty">{{ t('integration.compareEmpty') }}</p>
    </div>
  </Modal>
</template>

<style scoped>
.cmp-picks {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.cmp-pick {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.cmp-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.cmp-arrow {
  flex: none;
  color: var(--text-3);
  margin-top: 14px;
}

.cmp-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: var(--fs-xxs);
  color: var(--text-2);
  margin-bottom: 8px;
}

.cmp-diff-count {
  color: var(--warning);
}

.cmp-same-count {
  color: var(--success);
}

.cmp-table {
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-height: 46vh;
  overflow: auto;
}

.cmp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  min-width: 0;
}

.cmp-row.diff {
  background: var(--warning-tint);
}

.cmp-head {
  color: var(--text-3);
  position: sticky;
  top: 0;
  background: var(--bg-elevated);
  z-index: 1;
}

.c-idx {
  flex: none;
  width: 24px;
  color: var(--text-3);
}

.c-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-1);
}

.c-status {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.cmp-status.bad {
  color: var(--warning);
  font-weight: 600;
}

.cmp-sep {
  color: var(--text-3);
}

.c-code,
.c-assert,
.c-time {
  flex: none;
  font-family: var(--font-mono);
  color: var(--text-2);
}

.c-code.bad,
.c-assert.bad {
  color: var(--warning);
}

.c-time {
  width: 110px;
  text-align: right;
}

.cmp-empty {
  margin: 0;
  padding: 16px;
  text-align: center;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
