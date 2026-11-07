<script setup lang="ts">
/**
 * RunResultView：集成测试的独立执行结果视图（区别于编辑器内的实时 Dock）。
 *
 * 三层结构：
 * - 顶部执行概览：场景名 / 终态徽章 / 成功·失败·跳过计数 / 耗时 / 起止时间，
 *   操作：重新执行 · 返回编辑 · 导出报告；
 * - 左侧执行时间线：每步一行（序号/方法/名称/URL/状态/HTTP 码/耗时/断言数/
 *   提取数），点击选中；
 * - 右侧详情：Request · Response · Assertions · Variables · Logs 五个 Tab
 *   ——Request/Response 直接渲染落库快照（RequestDetail / ResponsePanel，
 *   绝不按当前接口配置重构历史）。
 *
 * 失败定位：result 变化时自动选中第一个失败步骤，详情头部给分类原因横幅。
 * 打开时机：运行完成后由宿主切换（或历史直达某次运行）。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import Tabs, { type TabItem } from '../ui/Tabs.vue'
import RequestDetail from '../RequestDetail.vue'
import ResponsePanel from '../ResponsePanel.vue'
import { methodBadgeTone } from '../../utils/methodTone'
import { formatDateTime } from '../../utils/dateTime'
import { formatDurationShort } from '../../utils/format'
import { isSensitiveVarName } from '../../utils/scenarioVars'
import { buildRunReportMarkdown } from '../../utils/runReport'
import { toneOf, type StepTone } from '../../composables/useScenarioRun'
import type { ScenarioRunResult, ScenarioStepResult } from '../../types/miyoApi'
import type { VarChainEntry } from '../../utils/scenarioVars'

const props = defineProps<{
  scenarioName: string
  runId: string
  /** 活跃运行进行中（时间线显示执行中状态）。 */
  running: boolean
  startedAt: string | null
  finishedAt: string | null
  result: ScenarioRunResult | null
  /** 变量链路（宿主按编辑态计算；来源步骤 / 引用步骤展示用）。 */
  varChain: VarChainEntry[]
  /** 正文 IPC 截断时的按需全文加载器（仅实时运行有）。 */
  fullBodyLoader?: (i: number) => (() => Promise<string | null>) | undefined
}>()

const emit = defineEmits<{
  (e: 'back'): void
  (e: 'rerun'): void
  (e: 'export', markdown: string): void
}>()

const t = useLocaleStore().t

// ---------- 汇总 ----------
const summary = computed(() => {
  const r = props.result
  if (!r) return null
  const ok = r.results.filter((x) => x.step_status === 'success').length
  const blocked = r.results.filter((x) => x.step_status === 'blocked').length
  const skipped = r.results.filter((x) => x.step_status === 'skipped').length
  const failed = r.results.filter(
    (x) => x.step_status === 'failed' || (x.cancelled && x.step_status !== 'success'),
  ).length
  const duration = r.results.reduce((acc, x) => acc + (x.duration_ms ?? 0), 0)
  return { total: r.results.length, ok, fail: failed, blocked, skipped, duration }
})

const finalTone = computed<'ok' | 'fail' | 'warn'>(() => {
  const r = props.result
  if (!r) return 'ok'
  if (r.cancelled) return 'warn'
  if ((summary.value?.fail ?? 0) > 0) return 'fail'
  return 'ok'
})

const finalText = computed(() => {
  const r = props.result
  if (!r) return ''
  if (r.cancelled) return t('scenario.cancelled')
  if ((summary.value?.fail ?? 0) > 0) return t('scenario.finalFail')
  return t('scenario.finalOk')
})

// ---------- 选中步骤 + 失败自动定位 ----------
const selectedIndex = ref(0)

const selected = computed<ScenarioStepResult | null>(() => {
  const r = props.result
  if (!r || !r.results.length) return null
  return r.results[Math.min(selectedIndex.value, r.results.length - 1)] ?? null
})

/** 失败自动定位：新结果到达时选中第一个失败步（无失败选第 0 步）。 */
watch(
  () => props.result,
  (r) => {
    if (!r) return
    const failIdx = r.results.findIndex(
      (x) => x.step_status === 'failed' || (x.step_status !== 'success' && !x.ok && !x.cancelled),
    )
    selectedIndex.value = failIdx === -1 ? 0 : failIdx
    respView.value = 'pretty'
  },
  { immediate: true },
)

// ---------- 失败分类（与 analyze_run_failure 同口径的本地版） ----------
const failureInfo = computed(() => {
  const r = selected.value
  if (!r) return null
  const status = r.step_status
  let labelKey: string | null = null
  let raw: string | null = null
  if (status === 'blocked') {
    labelKey = 'integration.failType.blocked'
    raw = r.blocked_reason ?? r.request_error ?? null
  } else if (r.cancelled) {
    labelKey = 'integration.failType.cancelled'
    raw = r.request_error ?? null
  } else if (r.request_error?.includes(t('integration.failType.loopKeyword'))) {
    labelKey = 'integration.failType.loopTimeout'
    raw = r.request_error
  } else if (r.request_error) {
    labelKey = 'integration.failType.request'
    raw = r.request_error
  } else if (r.outcomes.some((o) => !o.passed)) {
    labelKey = 'integration.failType.assertion'
    raw = `${r.outcomes.filter((o) => !o.passed).length} / ${r.outcomes.length}`
  }
  if (!labelKey) return null
  return { labelKey, raw }
})

// ---------- 右侧 Tab ----------
const detailTabs: TabItem[] = [
  { key: 'request', label: t('integration.tabRequest') },
  { key: 'response', label: t('integration.resultTabResponse') },
  { key: 'assertions', label: t('integration.tabAssert') },
  { key: 'variables', label: t('integration.resultTabVariables') },
  { key: 'logs', label: t('integration.resultTabLogs') },
]
const detailTab = ref('request')

watch(selectedIndex, () => {
  detailTab.value = 'request'
})

// ---------- Response 面板适配（ScenarioStepResponse → ExecuteResponse 形状） ----------
const responseForPanel = computed(() => {
  const resp = selected.value?.response
  if (!resp) return null
  return {
    status: resp.status,
    headers: resp.headers,
    body: resp.body,
    content_type: resp.content_type,
    duration_ms: resp.duration_ms,
    size_bytes: resp.size_bytes,
    truncated: resp.truncated,
    body_omitted: false, // 历史 64KB 截断由 historyTruncated 静态提示
    request: null,
  }
})

const respView = ref<'pretty' | 'raw' | 'tree'>('pretty')

// ---------- Variables：本步提取 + 全链路池（带来源/引用） ----------
function maskVar(name: string, value: string): string {
  return isSensitiveVarName(name) ? '••••••' : value
}

const poolEntries = computed(() => {
  const vars = props.result?.variables ?? {}
  return Object.entries(vars)
    .map(([name, value]) => {
      const chain = props.varChain.find((c) => c.name === name)
      return {
        name,
        value: maskVar(name, value),
        sensitive: isSensitiveVarName(name),
        sources: chain?.sources.length ?? 0,
        usages: chain?.usages.length ?? 0,
      }
    })
    .sort((a, b) => a.name.localeCompare(b.name))
})

const selectedExtracted = computed(() => {
  const r = selected.value
  if (!r?.extracted) return []
  return Object.entries(r.extracted).map(([name, value]) => ({
    name,
    value: maskVar(name, value),
    sensitive: isSensitiveVarName(name),
  }))
})

// ---------- Logs：由落库结果确定性重建（快照已含全部事实） ----------
interface LogLine {
  tone: 'info' | 'ok' | 'fail' | 'warn'
  text: string
}

const logs = computed<LogLine[]>(() => {
  const r = selected.value
  if (!r) return []
  const out: LogLine[] = []
  out.push({ tone: 'info', text: `${r.method} ${r.url ?? r.path}` })
  if (r.response) {
    out.push({
      tone: r.ok ? 'ok' : 'fail',
      text: `HTTP ${r.response.status} · ${Math.round(r.response.duration_ms)}ms · ${r.response.size_bytes}B`,
    })
  } else if (r.request_error) {
    out.push({ tone: 'fail', text: r.request_error })
  }
  for (const [k, v] of Object.entries(r.extracted ?? {})) {
    out.push({ tone: 'info', text: `${t('integration.logExtracted')} {{${k}}} = ${maskVar(k, v)}` })
  }
  for (const o of r.outcomes) {
    out.push({
      tone: o.passed ? 'ok' : 'fail',
      text: `${o.passed ? '✓' : '✗'} ${o.description}${o.expected != null ? ` · ${t('scenario.outcomeExpected', { v: o.expected })}` : ''}${o.actual != null ? ` · ${t('scenario.outcomeActual', { v: o.actual })}` : ''}${!o.passed && o.reason ? ` — ${o.reason}` : ''}`,
    })
  }
  if (r.step_status === 'blocked') {
    out.push({ tone: 'warn', text: `${t('integration.statusBlocked')}: ${r.blocked_reason ?? ''}` })
  } else if (r.step_status === 'skipped' && r.request_error) {
    out.push({ tone: 'warn', text: r.request_error })
  }
  if (r.cancelled) out.push({ tone: 'warn', text: t('scenario.stepStatusCancelled') })
  if ((r.loop_runs ?? 1) > 1) out.push({ tone: 'info', text: t('integration.loopRunsBadge', { n: r.loop_runs ?? 1 }) })
  return out
})

// ---------- 导出报告 ----------
function onExport(): void {
  const md = buildRunReportMarkdown({
    scenarioName: props.scenarioName,
    runId: props.runId,
    startedAt: props.startedAt,
    finishedAt: props.finishedAt,
    result: props.result!,
    t,
  })
  emit('export', md)
}

// ---------- 模板辅助 ----------
function toneClass(r: ScenarioStepResult | null): StepTone {
  if (!r) return 'wait'
  return toneOf(r)
}

const STEP_TEXT: Record<string, string> = {
  ok: 'integration.statusSuccess',
  fail: 'integration.statusFailed',
  skip: 'integration.statusSkipped',
  blocked: 'integration.statusBlocked',
  run: 'integration.statusRunning',
  wait: 'integration.statusPending',
  cancel: 'scenario.stepStatusCancelled',
}

const varTag = (n: string) => `{{${n}}}`
</script>

<template>
  <div class="rv-root">
    <!-- ===== 顶部执行概览 ===== -->
    <header class="rv-overview">
      <IconButton name="chevron-right" :size="14" :title="t('integration.resultBackToEdit')" @click="emit('back')" />
      <div class="rv-title">
        <span class="rv-name" :title="scenarioName">{{ scenarioName }}</span>
        <span class="rv-final" :class="finalTone">{{ finalText }}</span>
      </div>
      <span v-if="running" class="rv-final run">{{ t('integration.statusRunning') }}…</span>
      <template v-else-if="summary">
        <span class="rv-chips">
          <span class="rv-chip total">{{ t('integration.libSteps', { n: summary.total }) }}</span>
          <span class="rv-chip ok">{{ summary.ok }}</span>
          <span class="rv-chip fail" :class="{ dim: summary.fail === 0 }">{{ summary.fail }}</span>
          <span v-if="summary.skipped + summary.blocked" class="rv-chip skip">{{ summary.skipped + summary.blocked }}</span>
          <span class="rv-chip time">{{ formatDurationShort(summary.duration) }}</span>
        </span>
        <span v-if="startedAt || finishedAt" class="rv-times">
          {{ startedAt ? formatDateTime(startedAt) : '' }}{{ startedAt && finishedAt ? ' → ' : '' }}{{ finishedAt ? formatDateTime(finishedAt) : '' }}
        </span>
      </template>
      <div class="rv-spacer"></div>
      <IconButton name="refresh" :size="14" :title="t('integration.resultRerun')" @click="emit('rerun')" />
      <IconButton name="download" :size="14" :title="t('integration.resultExport')" :disabled="!result" @click="onExport" />
    </header>

    <div class="rv-body" v-if="result">
      <!-- ===== 左侧执行时间线 ===== -->
      <aside class="rv-timeline">
        <button
          v-for="(r, i) in result.results"
          :key="i"
          class="rv-step"
          :class="{ selected: selectedIndex === i }"
          type="button"
          @click="selectedIndex = i"
        >
          <span class="rv-step-rail" :class="toneClass(r)" aria-hidden="true"></span>
          <span class="rv-step-main">
            <span class="rv-step-head">
              <span class="num rv-step-no">{{ i + 1 }}</span>
              <span class="rv-step-method" :class="methodBadgeTone(r.method)">{{ r.method }}</span>
              <span class="rv-step-name" :title="r.endpoint_name">{{ r.endpoint_name }}</span>
              <span class="rv-step-status" :class="toneClass(r)">{{ t(STEP_TEXT[toneClass(r)]) }}</span>
            </span>
            <span class="rv-step-url" :title="r.url ?? r.path">{{ r.url ?? r.path }}</span>
            <span class="rv-step-meta">
              <span v-if="r.status != null" class="num">HTTP {{ r.status }}</span>
              <span v-if="r.duration_ms != null" class="num">{{ formatDurationShort(r.duration_ms) }}</span>
              <span v-if="r.outcomes.length" class="num">{{ t('scenario.assertPassed', { n: r.outcomes.filter((o) => o.passed).length, m: r.outcomes.length }) }}</span>
              <span v-if="r.extracted && Object.keys(r.extracted).length" class="num">{{ t('scenario.stepExtractBadge', { n: Object.keys(r.extracted).length }) }}</span>
              <span v-if="(r.loop_runs ?? 1) > 1" class="num">×{{ r.loop_runs }}</span>
            </span>
          </span>
        </button>
      </aside>

      <!-- ===== 右侧详情 ===== -->
      <section class="rv-detail">
        <!-- 失败定位横幅 -->
        <div v-if="failureInfo" class="rv-fail-banner" role="alert">
          <Icon name="alert-triangle" :size="14" />
          <div class="rv-fail-text">
            <div class="rv-fail-title">
              {{ t('integration.failTypeTitle') }} · {{ t(failureInfo.labelKey) }}
              <span v-if="failureInfo.raw" class="rv-fail-raw">{{ failureInfo.raw }}</span>
            </div>
            <div class="rv-fail-hint">{{ t('integration.failTypeHint') }}</div>
          </div>
        </div>

        <div v-if="selected?.response?.history_truncated" class="rv-trunc">
          <Icon name="alert-triangle" :size="12" /> {{ t('integration.historyTruncated') }}
        </div>

        <Tabs v-model="detailTab" :tabs="detailTabs" size="sm" />

        <div class="rv-detail-body">
          <!-- Request：落库快照 -->
          <div v-show="detailTab === 'request'" class="rv-pane">
            <RequestDetail
              v-if="selected?.response?.request"
              :response="selected.response"
              :url="selected.url ?? undefined"
            />
            <p v-else-if="selected" class="rv-empty">
              {{ selected.step_status === 'skipped' || selected.step_status === 'blocked' ? (selected.request_error ?? t('integration.resultNoRequest')) : t('integration.resultNoRequest') }}
            </p>
          </div>

          <!-- Response：完整面板（格式化/树/原始/搜索） -->
          <div v-show="detailTab === 'response'" class="rv-pane rv-pane-response">
            <ResponsePanel
              v-if="responseForPanel"
              :response="responseForPanel"
              class="rv-response-panel"
            />
            <p v-else class="rv-empty">{{ t('integration.resultNoResponse') }}</p>
          </div>

          <!-- Assertions：逐条明细 -->
          <div v-show="detailTab === 'assertions'" class="rv-pane">
            <table v-if="selected?.outcomes.length" class="rv-assert-table">
              <thead>
                <tr>
                  <th>{{ t('integration.assertNameCol') }}</th>
                  <th>{{ t('integration.assertResultCol') }}</th>
                  <th>{{ t('integration.assertExpectedCol') }}</th>
                  <th>{{ t('integration.assertActualCol') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(o, j) in selected.outcomes" :key="j" :class="{ fail: !o.passed }">
                  <td>{{ o.description }}</td>
                  <td class="rv-assert-result">{{ o.passed ? '✓' : '✗' }}</td>
                  <td class="num">{{ o.expected ?? '—' }}</td>
                  <td class="num">{{ o.actual ?? '—' }}</td>
                </tr>
              </tbody>
            </table>
            <p v-else class="rv-empty">{{ t('integration.resultNoAssertions') }}</p>
          </div>

          <!-- Variables：本步提取 + 全链路池 -->
          <div v-show="detailTab === 'variables'" class="rv-pane rv-vars">
            <div class="rv-vars-section">
              <div class="rv-vars-title">{{ t('integration.resultVarsExtracted') }}</div>
              <div v-if="selectedExtracted.length" class="rv-var-chips">
                <code v-for="e in selectedExtracted" :key="e.name" class="rv-var-chip" :title="e.sensitive ? t('integration.varMasked') : undefined">
                  {{ varTag(e.name) }} = {{ e.value }}
                </code>
              </div>
              <p v-else class="rv-empty">{{ t('integration.resultVarsNone') }}</p>
            </div>
            <div class="rv-vars-section">
              <div class="rv-vars-title">{{ t('integration.resultVarsPool') }}</div>
              <table v-if="poolEntries.length" class="rv-pool-table">
                <thead>
                  <tr>
                    <th>{{ t('integration.resultVarName') }}</th>
                    <th>{{ t('integration.resultVarValue') }}</th>
                    <th>{{ t('integration.resultVarSource') }}</th>
                    <th>{{ t('integration.resultVarUsed') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="e in poolEntries" :key="e.name">
                    <td class="num">{{ varTag(e.name) }}</td>
                    <td class="num" :title="e.sensitive ? t('integration.varMasked') : e.value">
                      {{ e.value }}
                    </td>
                    <td class="num">{{ e.sources ? t('integration.resultVarFromSteps', { n: e.sources }) : t('integration.resultVarExternal') }}</td>
                    <td class="num">{{ t('integration.resultVarUsedSteps', { n: e.usages }) }}</td>
                  </tr>
                </tbody>
              </table>
              <p v-else class="rv-empty">{{ t('integration.resultVarsNone') }}</p>
            </div>
          </div>

          <!-- Logs：确定性重建的执行日志 -->
          <div v-show="detailTab === 'logs'" class="rv-pane">
            <ol v-if="logs.length" class="rv-logs">
              <li v-for="(l, j) in logs" :key="j" :class="l.tone">{{ l.text }}</li>
            </ol>
            <p v-else class="rv-empty">{{ t('integration.resultNoLogs') }}</p>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.rv-root {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg-app);
  color: var(--text-1);
}

/* ---- 概览 ---- */
.rv-overview {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  min-width: 0;
}

.rv-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.rv-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 260px;
}

.rv-final {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 8px;
  border-radius: 999px;
}

.rv-final.ok {
  background: var(--success-tint);
  color: var(--success);
}

.rv-final.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.rv-final.warn,
.rv-final.run {
  background: var(--warning-tint);
  color: var(--warning);
}

.rv-chips {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.rv-chip {
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
}

.rv-chip.total {
  background: var(--bg-hover);
  color: var(--text-2);
}

.rv-chip.ok {
  background: var(--success-tint);
  color: var(--success);
}

.rv-chip.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.rv-chip.fail.dim {
  background: var(--bg-hover);
  color: var(--text-3);
}

.rv-chip.skip {
  background: var(--bg-hover);
  color: var(--text-3);
}

.rv-chip.time {
  background: var(--bg-hover);
  color: var(--text-3);
}

.rv-times {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rv-spacer {
  flex: 1;
}

/* ---- 主体两栏 ---- */
.rv-body {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 12px;
  padding: 10px 12px;
}

/* 时间线 */
.rv-timeline {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-right: 2px;
}

.rv-step {
  position: relative;
  display: flex;
  gap: 10px;
  padding: 7px 10px 7px 18px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  min-width: 0;
}

.rv-step:hover {
  border-color: var(--border-strong);
}

.rv-step.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

/* 时间线竖轨 */
.rv-step-rail {
  position: absolute;
  left: 7px;
  top: 0;
  bottom: 0;
  width: 2px;
  background: var(--border);
}

.rv-step:first-child .rv-step-rail {
  top: 50%;
}

.rv-step:last-child .rv-step-rail {
  bottom: 50%;
}

.rv-step-rail.ok {
  background: var(--success);
}

.rv-step-rail.fail {
  background: var(--danger);
}

.rv-step-rail.skip,
.rv-step-rail.blocked {
  background: var(--text-3);
  opacity: 0.4;
}

.rv-step-rail.cancel {
  background: var(--warning);
}

.rv-step-rail.run {
  background: var(--accent);
}

.rv-step-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
}

.rv-step-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.rv-step-no {
  flex: none;
  width: 16px;
  font-size: 10px;
  color: var(--text-3);
}

.rv-step-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}

.rv-step-name {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xs);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rv-step-status {
  flex: none;
  font-size: 10px;
  padding: 0 6px;
  border-radius: 999px;
}

.rv-step-status.ok {
  background: var(--success-tint);
  color: var(--success);
}

.rv-step-status.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.rv-step-status.skip,
.rv-step-status.blocked {
  background: var(--bg-hover);
  color: var(--text-3);
}

.rv-step-status.cancel {
  background: var(--warning-tint);
  color: var(--warning);
}

.rv-step-status.run {
  background: var(--accent-tint);
  color: var(--accent);
}

.rv-step-url {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rv-step-meta {
  display: flex;
  gap: 10px;
  font-size: 10px;
  color: var(--text-3);
  font-family: var(--font-mono);
}

/* 详情 */
.rv-detail {
  flex: none;
  width: 46%;
  min-width: 380px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  padding: 10px 12px;
  overflow: hidden;
}

.rv-fail-banner {
  display: flex;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--danger-border, var(--danger));
  border-radius: var(--radius-md);
  background: var(--danger-tint);
  color: var(--danger);
  align-items: flex-start;
}

.rv-fail-title {
  font-size: var(--fs-xs);
  font-weight: 600;
}

.rv-fail-raw {
  font-family: var(--font-mono);
  font-weight: 400;
  margin-left: 6px;
  word-break: break-all;
}

.rv-fail-hint {
  font-size: var(--fs-xxs);
  opacity: 0.8;
  margin-top: 2px;
}

.rv-trunc {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

.rv-detail-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.rv-pane {
  min-height: 0;
}

.rv-pane-response {
  display: flex;
  flex-direction: column;
}

.rv-response-panel {
  flex: 1;
  min-height: 320px;
}

.rv-empty {
  margin: 0;
  padding: 14px 6px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 断言表 */
.rv-assert-table,
.rv-pool-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-xxs);
}

.rv-assert-table th,
.rv-pool-table th {
  text-align: left;
  color: var(--text-3);
  font-weight: 500;
  padding: 4px 8px;
  border-bottom: 1px solid var(--border);
  position: sticky;
  top: 0;
  background: var(--bg-panel);
}

.rv-assert-table td,
.rv-pool-table td {
  padding: 5px 8px;
  border-bottom: 1px solid var(--border);
  word-break: break-all;
}

.rv-assert-table tr.fail td {
  color: var(--danger);
}

.rv-assert-result {
  font-family: var(--font-mono);
}

/* 变量 */
.rv-vars {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.rv-vars-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
  margin-bottom: 6px;
}

.rv-var-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.rv-var-chip {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-code);
  border: 1px solid var(--border);
  word-break: break-all;
}

/* 日志 */
.rv-logs {
  margin: 0;
  padding: 8px 10px;
  list-style: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.7;
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  word-break: break-all;
}

.rv-logs li.ok {
  color: var(--success);
}

.rv-logs li.fail {
  color: var(--danger);
}

.rv-logs li.warn {
  color: var(--warning);
}

.rv-logs li.info {
  color: var(--text-2);
}

@media (max-width: 1180px) {
  .rv-detail {
    min-width: 320px;
  }
}
</style>
