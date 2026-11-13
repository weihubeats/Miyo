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
import Icon from './ui/Icon.vue'
import { formatDuration } from '../utils/format'
import type { Endpoint, LoadResult } from '../types/miyoApi'
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
const loadConcurrency = ref('20')
const loadTotal = ref('200')
const loading = ref(false)
const loadResult = ref<LoadResult | null>(null)

const progress = ref<{ done: number; total: number; ok: number; failed: number } | null>(null)
const running = computed(() => loading.value || progress.value !== null)

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
function onProgress(p: { done: number; total: number; ok: number; failed: number }): void {
  progress.value = p
  const now = performance.now()
  if (p.done < p.total && now - lastPointTs < 150) return
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
  unlistenLoad = await listen<{ done: number; total: number; ok: number; failed: number }>(
    'miyo:load-progress',
    (event) => onProgress(event.payload),
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
      run_id: runId,
    })
    loadResult.value = result
    if (result.cancelled) toast.info(t('loadtest.cancelled', { v: `${result.total}/${total}` }))
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
    return [
      { label: t('loadtest.metricDone'), value: p ? `${p.done}/${p.total}` : '—' },
      { label: t('loadtest.legendOk'), value: p ? String(p.ok) : '—', tone: 'ok' },
      { label: t('loadtest.legendFail'), value: p ? String(p.failed) : '—', tone: p && p.failed > 0 ? 'err' : undefined },
      { label: t('loadtest.metricAvg'), value: '—' },
      { label: 'RPS', value: '—' },
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
  ]
})
</script>

<template>
  <div class="load-panel">
    <div class="load-controls">
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
        :model-value="loadTotal"
        size="sm"
        :min="1"
        :placeholder="t('loadtest.totalPh')"
        class="load-num"
        :disabled="running"
        @update:model-value="loadTotal = String($event)"
      />
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="!draft || running" @click="runLoadTest">
        <Icon name="gauge" :size="13" />
        {{ loading ? t('loadtest.running', { v: progress ? `${progress.done}/${progress.total}` : '' }) : t('loadtest.start') }}
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