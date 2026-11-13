<script setup lang="ts">
/**
 * LoadTestPanel：压测（并发基准）面板。
 * - 实时面积图（Chart.js）：x=已耗时(s)，y=累计成功/失败请求数，压测中逐事件刷新；
 * - 3 列 KPI 指标网格：成功率 / 平均耗时 / RPS / P50 / P90 / P99。
 */
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { ChartOptions } from 'chart.js'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import CustomNumberInput from './ui/CustomNumberInput.vue'
import CustomSelect from './ui/CustomSelect.vue'
import Icon from './ui/Icon.vue'
import { formatDuration } from '../utils/format'
import type { Endpoint, LoadProgress, LoadResult, LoadSchedule } from '../types/miyoApi'
import { errorText } from '../utils/errorText'

// chart.js 体积较大（约 200KB gz）且仅压测面板使用，懒加载以缩减首屏 bundle
const Line = defineAsyncComponent(async () => {
  const [
    { Chart: ChartJS, Decimation, Filler, Legend, LinearScale, LineElement, PointElement, Tooltip },
    { Line },
  ] = await Promise.all([import('chart.js'), import('vue-chartjs')])
  ChartJS.register(Decimation, LinearScale, PointElement, LineElement, Filler, Tooltip, Legend)
  return Line
})

const props = defineProps<{
  draft: Endpoint | null
  url: string
}>()

const api = useMiyoApi()
const toast = useToast()
const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

// ---------- 运行控制 ----------
/** 调度模式：fixed 固定总量（原行为）/ fixed_qps 恒定速率 / ramp 阶梯加压 / duration 持续压测。 */
const loadMode = ref<'fixed' | 'fixed_qps' | 'ramp' | 'duration'>('fixed')
const loadConcurrency = ref('20')
const loadTotal = ref('200')
const loadQps = ref('50')
const loadDurationSecs = ref('10')
const loadStartQps = ref('10')
const loadEndQps = ref('100')
const loadSteps = ref('4')
const loadStageSecs = ref('1')
/** 错误率熔断阈值（空串 = 关闭）。 */
const loadMaxErrorRate = ref('')
const loading = ref(false)
const loadResult = ref<LoadResult | null>(null)

const progress = ref<LoadProgress | null>(null)
const running = computed(() => loading.value || progress.value !== null)

/** 模式下拉选项（computed：切语言后文案跟随）。 */
const modeOptions = computed(() => [
  { value: 'fixed', label: t('loadtest.modeFixed') },
  { value: 'fixed_qps', label: t('loadtest.modeFixedQps') },
  { value: 'ramp', label: t('loadtest.modeRamp') },
  { value: 'duration', label: t('loadtest.modeDuration') },
])

/** 组装当前模式的后端调度参数（非法输入在 runLoadTest 中拦截）。 */
function buildSchedule(): LoadSchedule | null | 'invalid' {
  const num = (v: string): number | null => {
    const n = Number(v)
    return Number.isFinite(n) && n > 0 ? n : null
  }
  switch (loadMode.value) {
    case 'fixed':
      return null
    case 'fixed_qps': {
      const qps = num(loadQps.value)
      const secs = num(loadDurationSecs.value)
      return qps && secs ? { mode: 'fixed_qps', qps, duration_secs: secs } : 'invalid'
    }
    case 'ramp': {
      const startQ = num(loadStartQps.value)
      const endQ = num(loadEndQps.value)
      const steps = num(loadSteps.value)
      const stage = num(loadStageSecs.value)
      if (!startQ || !endQ || !steps || !stage || endQ < startQ) return 'invalid'
      return { mode: 'ramp', start_qps: startQ, end_qps: endQ, steps, stage_secs: stage }
    }
    case 'duration': {
      const secs = num(loadDurationSecs.value)
      return secs ? { mode: 'duration', duration_secs: secs } : 'invalid'
    }
  }
}

// ---------- 实时图表 ----------
interface ChartPoint {
  x: number
  ok: number
  failed: number
}

const points = ref<ChartPoint[]>([])
let startTs: number | null = null

function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}

const okColor = cssVar('--success', '#22c55e')
const errColor = cssVar('--danger', '#ef4444')

const chartData = computed(() => ({
  datasets: [
    {
      label: t('loadtest.legendOk'),
      data: points.value.map((p) => ({ x: p.x, y: p.ok })),
      borderColor: okColor,
      backgroundColor: `${okColor}2e`,
      fill: true,
      tension: 0.35,
      borderWidth: 1.5,
      pointRadius: 0,
    },
    {
      label: t('loadtest.legendFail'),
      data: points.value.map((p) => ({ x: p.x, y: p.failed })),
      borderColor: errColor,
      backgroundColor: `${errColor}2e`,
      fill: true,
      tension: 0.35,
      borderWidth: 1.5,
      pointRadius: 0,
    },
  ],
}))

// computed：切语言后重渲染时坐标轴标题跟随更新
const chartOptions = computed<ChartOptions<'line'>>(() => ({
  responsive: true,
  maintainAspectRatio: false,
  animation: false,
  interaction: { mode: 'index', intersect: false },
  scales: {
    x: {
      type: 'linear',
      title: { display: true, text: t('loadtest.axisTime'), font: { size: 11 } },
      ticks: { precision: 0 },
      grid: { color: 'rgba(128,128,128,0.12)' },
    },
    y: {
      beginAtZero: true,
      title: { display: true, text: t('loadtest.axisDone'), font: { size: 11 } },
      ticks: { precision: 0 },
      grid: { color: 'rgba(128,128,128,0.12)' },
    },
  },
  plugins: {
    legend: { labels: { boxWidth: 12, boxHeight: 12, font: { size: 11 } } },
    // 高频事件下只渲染抽样点（LTTB），避免逐事件全量重绘掉帧。
    decimation: { enabled: true, algorithm: 'lttb', samples: 200 },
  },
}))

/**
 * 图表节流：后端已按 100ms 合并事件，前端再按 150ms 窗口收点
 *（终态必收，保证收尾准确）。原来每事件 push + chartData 全量 map ×2
 * + Chart.js 全量重绘，高并发短请求下掉帧。
 */
let lastPointTs = 0
function onProgress(p: LoadProgress): void {
  progress.value = p
  const now = performance.now()
  // total=0 为不定总量模式（按时长跑）：全量收点（后端已 100ms 节流）
  if (p.total > 0 && p.done < p.total && now - lastPointTs < 150) return
  lastPointTs = now
  if (startTs === null) startTs = now
  const x = Math.round(((now - startTs) / 1000) * 10) / 10
  const last = points.value[points.value.length - 1]
  if (last && Math.abs(x - last.x) < 0.05) {
    last.ok = p.ok
    last.failed = p.failed
  } else {
    points.value.push({ x, ok: p.ok, failed: p.failed })
  }
  if (points.value.length > 400) points.value.splice(0, points.value.length - 400)
}

let unlistenLoad: UnlistenFn | null = null

onMounted(async () => {
  unlistenLoad = await listen<LoadProgress>('miyo:load-progress', (event) =>
    onProgress(event.payload),
  )
})

onUnmounted(() => {
  unlistenLoad?.()
})

// ---------- 启动 / 取消压测 ----------
/** 本轮运行标识：取消按钮持有，结束后清空。 */
const activeRunId = ref<string | null>(null)

async function runLoadTest(): Promise<void> {
  if (!props.draft) return
  const concurrency = Number(loadConcurrency.value)
  const total = Number(loadTotal.value)
  if (!Number.isFinite(concurrency) || !Number.isFinite(total) || total < 1) {
    toast.error(t('loadtest.invalidInput'))
    return
  }
  const schedule = buildSchedule()
  if (schedule === 'invalid') {
    toast.error(t('loadtest.invalidInput'))
    return
  }
  const maxErrorRate = loadMaxErrorRate.value.trim()
    ? Number(loadMaxErrorRate.value)
    : null
  if (maxErrorRate !== null && (!Number.isFinite(maxErrorRate) || maxErrorRate <= 0 || maxErrorRate > 1)) {
    toast.error(t('loadtest.invalidInput'))
    return
  }
  loading.value = true
  progress.value = null
  loadResult.value = null
  points.value = []
  startTs = null
  lastPointTs = 0
  const runId = crypto.randomUUID()
  activeRunId.value = runId
  try {
    const result = await api.loadTest({
      url: props.url,
      method: props.draft.method,
      spec: props.draft.request,
      environment_id: store.activeEnvId,
      concurrency,
      total,
      schedule,
      max_error_rate: maxErrorRate,
      run_id: runId,
    })
    loadResult.value = result
    if (result.aborted_reason === 'error_rate') {
      toast.warning(t('loadtest.abortedErrorRate'))
    } else if (result.cancelled) {
      toast.info(t('loadtest.cancelled', { v: `${result.total}/${total}` }))
    }
  } catch (err) {
    toast.error(t('loadtest.runFail'), { message: errorText(err) })
  } finally {
    if (activeRunId.value === runId) activeRunId.value = null
    loading.value = false
    progress.value = null
  }
}

/** 取消在途压测：后端中止 worker，已完成样本保留并返回。 */
function cancelLoadTest(): void {
  if (!activeRunId.value) return
  void api.cancelLoadTest(activeRunId.value).catch(() => {})
  toast.info(t('loadtest.cancelling'))
}

// ---------- 指标 ----------
interface Metric {
  label: string
  value: string
  tone?: 'ok' | 'warn' | 'err'
}

const metrics = computed<Metric[]>(() => {
  const r = loadResult.value
  if (!r) {
    const p = progress.value
    const doneLabel = p ? (p.total > 0 ? `${p.done}/${p.total}` : t('loadtest.runningIndeterminate', { v: p.done })) : '—'
    return [
      { label: t('loadtest.metricDone'), value: doneLabel },
      { label: t('loadtest.legendOk'), value: p ? String(p.ok) : '—', tone: 'ok' },
      { label: t('loadtest.legendFail'), value: p ? String(p.failed) : '—', tone: p && p.failed > 0 ? 'err' : undefined },
      { label: t('loadtest.metricRpsLive'), value: p ? p.rps.toFixed(1) : '—' },
      {
        label: t('loadtest.metricErrorRate'),
        value: p ? `${(p.error_rate * 100).toFixed(1)}%` : '—',
        tone: p && p.error_rate > 0.5 ? 'err' : undefined,
      },
      { label: 'P50', value: '—' },
    ]
  }
  const rate = ((r.ok / Math.max(r.total, 1)) * 100).toFixed(1)
  return [
    {
      label: t('loadtest.metricRate'),
      value: `${rate}%`,
      tone: Number(rate) === 100 ? 'ok' : Number(rate) >= 95 ? 'warn' : 'err',
    },
    { label: t('loadtest.metricAvg'), value: formatDuration(r.avg_ms) },
    { label: 'RPS', value: r.rps.toFixed(1) },
    { label: 'P50', value: formatDuration(r.p50_ms) },
    { label: 'P90', value: formatDuration(r.p90_ms) },
    { label: 'P99', value: formatDuration(r.p99_ms) },
    {
      label: t('loadtest.resCpu'),
      value: `${r.res_cpu_avg.toFixed(0)} / ${r.res_cpu_peak.toFixed(0)}`,
    },
    {
      label: t('loadtest.resMem'),
      value: `${r.res_mem_avg_mb.toFixed(0)} / ${r.res_mem_peak_mb.toFixed(0)}`,
    },
  ]
})

/** 直方图条形归一化高度（相对最大桶）。 */
const histogramBars = computed(() => {
  const r = loadResult.value
  if (!r?.histogram?.length) return []
  const max = Math.max(...r.histogram.map((b) => b.count), 1)
  return r.histogram.map((b) => ({
    label: b.le_ms === null ? t('loadtest.histogramOverflow') : `≤${b.le_ms}`,
    count: b.count,
    pct: Math.round((b.count / max) * 100),
  }))
})
</script>

<template>
  <div class="load-panel">
    <div class="load-controls">
      <label class="load-mode">
        <span class="load-mode-label">{{ t('loadtest.modeLabel') }}</span>
        <CustomSelect
          v-model="loadMode"
          :options="modeOptions"
          :disabled="running"
          class="load-mode-select"
        />
      </label>
      <CustomNumberInput
        :model-value="loadConcurrency"
        size="sm"
        :min="1"
        :placeholder="t('loadtest.concurrencyPh')"
        class="load-num"
        :disabled="running"
        @update:model-value="loadConcurrency = String($event)"
      />
      <CustomNumberInput
        v-if="loadMode === 'fixed'"
        :model-value="loadTotal"
        size="sm"
        :min="1"
        :placeholder="t('loadtest.totalPh')"
        class="load-num"
        :disabled="running"
        @update:model-value="loadTotal = String($event)"
      />
      <CustomNumberInput
        v-if="loadMode === 'fixed_qps'"
        :model-value="loadQps"
        size="sm"
        :min="1"
        :placeholder="t('loadtest.qpsPh')"
        class="load-num-wide"
        :disabled="running"
        @update:model-value="loadQps = String($event)"
      />
      <template v-if="loadMode === 'ramp'">
        <CustomNumberInput
          :model-value="loadStartQps"
          size="sm"
          :min="1"
          :placeholder="t('loadtest.startQpsPh')"
          class="load-num"
          :disabled="running"
          @update:model-value="loadStartQps = String($event)"
        />
        <CustomNumberInput
          :model-value="loadEndQps"
          size="sm"
          :min="1"
          :placeholder="t('loadtest.endQpsPh')"
          class="load-num"
          :disabled="running"
          @update:model-value="loadEndQps = String($event)"
        />
        <CustomNumberInput
          :model-value="loadSteps"
          size="sm"
          :min="1"
          :placeholder="t('loadtest.stepsPh')"
          class="load-num-sm"
          :disabled="running"
          @update:model-value="loadSteps = String($event)"
        />
        <CustomNumberInput
          :model-value="loadStageSecs"
          size="sm"
          :min="1"
          :placeholder="t('loadtest.stageSecsPh')"
          class="load-num-sm"
          :disabled="running"
          @update:model-value="loadStageSecs = String($event)"
        />
      </template>
      <CustomNumberInput
        v-if="loadMode === 'fixed_qps' || loadMode === 'duration'"
        :model-value="loadDurationSecs"
        size="sm"
        :min="1"
        :placeholder="t('loadtest.durationPh')"
        class="load-num-sm"
        :disabled="running"
        @update:model-value="loadDurationSecs = String($event)"
      />
      <CustomNumberInput
        :model-value="loadMaxErrorRate"
        size="sm"
        :min="0"
        :placeholder="t('loadtest.errorRatePh')"
        class="load-num-wide"
        :disabled="running"
        @update:model-value="loadMaxErrorRate = String($event)"
      />
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="!draft || running" @click="runLoadTest">
        <Icon name="gauge" :size="13" />
        {{
          loading
            ? progress && progress.total > 0
              ? t('loadtest.running', { v: `${progress.done}/${progress.total}` })
              : t('loadtest.runningIndeterminate', { v: progress?.done ?? 0 })
            : t('loadtest.start')
        }}
      </button>
      <button
        v-if="running && activeRunId"
        class="miyo-btn miyo-btn-sm miyo-btn-danger"
        type="button"
        @click="cancelLoadTest"
      >
        <Icon name="x" :size="13" />
        {{ t('common.cancel') }}
      </button>
    </div>

    <div v-if="running || loadResult" class="chart-wrap">
      <Line :data="chartData" :options="chartOptions" />
    </div>

    <div v-if="running || loadResult" class="metrics">
      <div
        v-for="m in metrics"
        :key="m.label"
        class="metric"
        :class="{ [`tone-${m.tone}`]: m.tone }"
      >
        <span class="metric-label">{{ m.label }}</span>
        <span class="metric-value">{{ m.value }}</span>
      </div>
    </div>

    <div v-if="histogramBars.length" class="histogram">
      <span class="histogram-title">{{ t('loadtest.histogramTitle') }}</span>
      <div class="histogram-bars">
        <div v-for="b in histogramBars" :key="b.label" class="histogram-col">
          <div class="histogram-bar" :style="{ height: `${Math.max(b.pct, 2)}%` }" />
          <span class="histogram-count">{{ b.count }}</span>
          <span class="histogram-label">{{ b.label }}</span>
        </div>
      </div>
    </div>

    <ul v-if="loadResult?.errors.length" class="load-errors">
      <li v-for="(e, i) in loadResult.errors" :key="i">{{ e }}</li>
    </ul>

    <p v-if="!running && !loadResult" class="load-hint">
      {{ t('loadtest.hint') }}
    </p>
  </div>
</template>

<style scoped>
.load-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.load-controls {
  display: flex;
  gap: 8px;
  align-items: center;
}

.load-num {
  width: 110px;
}

.load-num-wide {
  width: 150px;
}

.load-num-sm {
  width: 76px;
}

.load-mode {
  display: flex;
  align-items: center;
  gap: 6px;
}

.load-mode-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.load-mode-select {
  width: 110px;
}

.histogram {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
}

.histogram-title {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.histogram-bars {
  display: flex;
  align-items: flex-end;
  gap: 4px;
  height: 88px;
}

.histogram-col {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
  height: 100%;
}

.histogram-bar {
  width: 100%;
  min-height: 2px;
  border-radius: 2px 2px 0 0;
  background: var(--accent);
  opacity: 0.85;
}

.histogram-count {
  font-size: 10px;
  font-family: var(--font-mono);
  color: var(--text-2);
}

.histogram-label {
  font-size: 10px;
  color: var(--text-3);
  white-space: nowrap;
}

.chart-wrap {
  position: relative;
  height: 220px;
  padding: 4px 2px 0;
}

.metrics {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.metric {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
}

.metric-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.metric-value {
  font-size: 15px;
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-1);
}

.metric.tone-ok .metric-value {
  color: var(--success);
}

.metric.tone-warn .metric-value {
  color: var(--warning);
}

.metric.tone-err .metric-value {
  color: var(--danger);
}

.load-errors {
  margin: 0;
  padding: 8px 12px 8px 28px;
  border: 1px solid var(--danger-border);
  border-radius: var(--radius);
  background: var(--danger-tint);
  color: var(--danger);
  font-size: 12px;
  word-break: break-all;
}

.load-hint {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}
</style>