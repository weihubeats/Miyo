<script setup lang="ts">
/**
 * ScenarioRunReport：底部可折叠执行结果 Dock（从 ScenarioEditor 抽出重构）。
 *
 * 信息架构调整：旧版结果面板以普通 section 跟在步骤列表后面（折叠时仍是
 * 一段游离内容）；新版固化为编排区底部的 Dock——头部常驻摘要条（终态徽章
 * + 成功 / 失败 / 跳过 / 断言通过 / 总耗时），展开后正文限高内滚。
 *
 * 功能与旧版对齐（数据流不变）：
 * - 运行中逐步实时渲染（后端每步结束即回传，不必等整轮）；
 * - 运行结束换成 runResult（含汇总 / 变量池 / 重跑入口 / 历史回看）；
 * - 每步结果行：状态点 / 方法 / 名称 / 状态码 / 耗时 / 断言数 / 实际 URL /
 *   错误 / 断言明细 / 提取值 / 响应正文速览 / 可展开的请求响应详情；
 * - 点击结果行的定位按钮（或行首方法 + 名称）定位到编排区对应步骤卡。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import RequestDetail from '../RequestDetail.vue'
import ResponsePanel from '../ResponsePanel.vue'
import { formatDurationShort } from '../../utils/format'
import { formatDateTime } from '../../utils/dateTime'
import { methodBadgeTone } from '../../utils/methodTone'
import type { ScenarioRunResult, ScenarioStepResult } from '../../types/miyoApi'

/** Blocked / Skipped 行的展示文本（不发送请求的步骤给原因行）。 */
function holdText(r: ScenarioStepResult): { reason: string; blocked: boolean } | null {
  if (r.step_status === 'blocked') {
    return { reason: r.blocked_reason ?? r.request_error ?? '', blocked: true }
  }
  if (r.step_status === 'skipped' && r.request_error) {
    return { reason: r.request_error, blocked: false }
  }
  return null
}

/** 运行中的实时结果行（已结束步带 result，在途步占位）。 */
export interface LiveRow {
  result: ScenarioStepResult | null
  pos: number
  url: string | null
}

const props = defineProps<{
  open: boolean
  running: boolean
  progress: { done: number; total: number }
  progressPercent: number
  runResult: ScenarioRunResult | null
  runIsHistory: boolean
  currentRunAt: string | null
  liveRows: LiveRow[]
  livePending: boolean
  respOpenRows: Set<number>
  runSummary: { total: number; ok: number; fail: number; skipped: number; duration: number } | null
  finalState: { text: string; tone: 'ok' | 'fail' | 'warn' } | null
  rerunAvailable: boolean
  hasHistory: boolean
  /** 历史回看态的「按当时配置重跑」入口（meta 快照存在时才显示）。 */
  rerunHistoryAvailable?: boolean
  /** 在途步的接口名 / 方法（结果尚未回传时占位；宿主按运行位次反查）。 */
  stepName: (pos: number) => string
  stepMethod: (pos: number) => string
  /** 正文被 IPC 预览截断时的按需全文加载器（宿主持有 runId）。 */
  fullBodyLoader: (i: number) => (() => Promise<string | null>) | undefined
}>()

const emit = defineEmits<{
  (e: 'update:open', v: boolean): void
  (e: 'locate', pos: number): void
  (e: 'rerun'): void
  (e: 'rerun-history'): void
  (e: 'history', event: MouseEvent): void
  (e: 'toggle-row', i: number): void
}>()

const t = useLocaleStore().t

/** 变量展示形态（模板插值内不能写 {{ 字面量，收敛到函数）。 */
function varTag(n: string): string {
  return `{{${n}}}`
}

function toggleOpen(): void {
  emit('update:open', !props.open)
}

// ---------- 汇总补充：断言通过数（跨步累计） ----------
const assertSummary = computed(() => {
  const results = props.runResult?.results ?? []
  let passed = 0
  let total = 0
  for (const r of results) {
    for (const o of r.outcomes) {
      total += 1
      if (o.passed) passed += 1
    }
  }
  return { passed, total }
})

// ---------- 响应正文速览（原宿主逻辑平移） ----------
const PEEK_LINES = 8
const PEEK_CHARS = 800

/**
 * 把响应正文压成「可读的几行」：
 * - JSON 优先按结构重新序列化逐行（单行压缩 JSON 也能读）；
 * - 其它类型按原文分行；超长整行按字符截断。
 * 返回空串表示无可展示正文（204 / 空体）。
 */
function responsePeek(body: string | null | undefined): string {
  const raw = (body ?? '').trim()
  if (!raw) return ''
  let text = raw
  if (raw.startsWith('{') || raw.startsWith('[')) {
    try {
      text = JSON.stringify(JSON.parse(raw), null, 2)
    } catch {
      // 非法 JSON：保留原文
    }
  }
  if (text.length > PEEK_CHARS) text = `${text.slice(0, PEEK_CHARS)}\n…`
  const lines = text.split('\n')
  return lines.length > PEEK_LINES ? `${lines.slice(0, PEEK_LINES).join('\n')}\n…` : text
}

/** 折叠行里显示的正文速览（>0 才渲染）。 */
function peekOf(response: { body?: string | null } | null | undefined): string {
  return responsePeek(response?.body)
}
</script>

<template>
  <!-- ===== 执行结果 Dock（可折叠；支持历史回看 / 重开恢复） ===== -->
  <section v-if="running || runResult" class="se-report" :class="{ fail: finalState?.tone === 'fail' }">
    <div class="se-report-headrow">
      <button type="button" class="se-report-head" @click="toggleOpen">
        <Icon name="gauge" :size="13" />
        <span class="se-report-title">{{ t('scenario.resultTitle') }}</span>
        <span
          v-if="runIsHistory && currentRunAt"
          class="se-report-badge warn"
          :title="t('scenario.historyBadge')"
        >{{ t('scenario.historyBadge') }}</span>
        <template v-if="running">
          <span class="se-report-meta">{{ t('scenario.progress', { done: progress.done, total: progress.total }) }}</span>
        </template>
        <template v-else-if="runSummary && finalState">
          <span class="se-report-badge" :class="finalState.tone">{{ finalState.text }}</span>
          <span class="se-report-chips">
            <span class="se-chip ok" :title="t('scenario.summaryOk', { n: runSummary.ok })">{{ runSummary.ok }}</span>
            <span class="se-chip fail" :class="{ dim: runSummary.fail === 0 }" :title="t('scenario.summaryFail', { n: runSummary.fail })">{{ runSummary.fail }}</span>
            <span v-if="runSummary.skipped" class="se-chip skip" :title="t('scenario.summarySkip', { n: runSummary.skipped })">{{ runSummary.skipped }}</span>
            <span v-if="assertSummary.total" class="se-chip assert" :title="t('scenario.assertPassedHint')">
              {{ t('scenario.assertPassed', { n: assertSummary.passed, m: assertSummary.total }) }}
            </span>
            <span class="se-chip time">{{ formatDurationShort(runSummary.duration) }}</span>
          </span>
        </template>
        <span v-if="runResult?.stopped_on_failure && !running" class="se-report-warn">
          {{ t('scenario.stoppedAt') }}
        </span>
        <span v-if="runResult?.cancelled && !running" class="se-report-warn">
          {{ t('scenario.cancelled') }}
        </span>
        <span v-if="currentRunAt && !running" class="se-report-meta se-report-time">
          {{ formatDateTime(currentRunAt) }}
        </span>
        <Icon
          :name="open ? 'chevrons-down-up' : 'chevrons-up-down'"
          :size="13"
          class="se-report-caret"
        />
      </button>
      <IconButton
        v-if="rerunAvailable"
        name="refresh"
        :size="13"
        :title="t('scenario.rerunFailed')"
        @click.stop="emit('rerun')"
      />
      <IconButton
        v-if="rerunHistoryAvailable"
        name="play"
        :size="13"
        :title="t('integration.rerunFromHistory')"
        @click.stop="emit('rerun-history')"
      />
      <IconButton
        name="history"
        :size="13"
        :title="t('scenario.history')"
        :disabled="!hasHistory"
        @click.stop="emit('history', $event)"
      />
    </div>
    <div v-if="open" class="se-report-body">
      <div v-if="running" class="se-progress">
        <div class="se-progress-track">
          <div class="se-progress-bar" :style="{ width: `${progressPercent}%` }"></div>
        </div>
      </div>
      <div class="se-report-scroll">
      <!-- 运行中逐步实时渲染：后端每步结束即回传结果，不必等整轮跑完；
           运行结束后换成 runResult（含汇总 / 变量池 / 重跑入口）。 -->
      <template v-if="running">
        <div v-for="row in liveRows" :key="row.pos" class="se-rrow">
          <div class="se-rrow-head">
            <button type="button" class="se-rloc-btn" :title="t('scenario.locateStep')" @click.stop="emit('locate', row.pos)">
              <span
                class="se-rdot"
                :class="{ fail: row.result && !row.result.ok && !row.result.cancelled, cancelled: row.result?.cancelled, live: !row.result }"
                :aria-label="row.result ? undefined : t('scenario.stepStatusRun')"
              ></span>
              <span class="se-rmethod" :class="methodBadgeTone(row.result?.method ?? stepMethod(row.pos))">
                {{ row.result?.method ?? stepMethod(row.pos) }}
              </span>
              <span class="se-rname">
                {{ row.pos + 1 }}. {{ row.result?.endpoint_name ?? stepName(row.pos) }}
              </span>
            </button>
            <template v-if="row.result">
              <span v-if="row.result.status != null" class="se-rstatus">{{ row.result.status }}</span>
              <span v-if="row.result.duration_ms != null" class="se-rtime">
                {{ formatDurationShort(row.result.duration_ms) }}
              </span>
            </template>
            <span v-else class="se-rtime se-rtime-live">{{ t('scenario.stepStatusRun') }}…</span>
            <IconButton
              v-if="row.result?.response || row.result?.request_error"
              :name="respOpenRows.has(row.pos) ? 'chevrons-down-up' : 'chevrons-up-down'"
              :size="13"
              :title="t('scenario.viewRequestDetail')"
              @click.stop="emit('toggle-row', row.pos)"
            />
          </div>
          <!-- 请求参数：渲染后实际 URL + 实际发出的方法 / 头 / 体（请求快照） -->
          <code v-if="row.result?.url || row.url" class="se-rurl" :title="row.result?.url ?? row.url ?? undefined">
            {{ row.result?.url ?? row.url }}
          </code>
          <p v-if="row.result?.request_error" class="se-rerr">{{ row.result.request_error }}</p>
          <ul v-if="row.result?.outcomes.length" class="se-routcomes">
            <li v-for="(o, j) in row.result.outcomes" :key="j" :class="{ fail: !o.passed }">
              {{ o.description }}
              <span v-if="o.expected != null || o.actual != null" class="se-r-outcome-va">
                <template v-if="o.expected != null"> · {{ t('scenario.outcomeExpected', { v: o.expected }) }}</template>
                <template v-if="o.actual != null"> · {{ t('scenario.outcomeActual', { v: o.actual }) }}</template>
              </span>
              <span v-else-if="o.reason"> — {{ o.reason }}</span>
            </li>
          </ul>
          <p v-if="row.result?.response?.history_truncated" class="se-rtrunc">{{ t('integration.historyTruncated') }}</p>
          <!-- 响应正文速览：不展开也能看到返回了什么 -->
          <pre v-if="peekOf(row.result?.response)" class="se-rpeek">{{ peekOf(row.result?.response) }}</pre>
          <div v-if="row.result?.extracted && Object.keys(row.result.extracted).length" class="se-rext">
            <span class="se-rext-label">{{ t('scenario.extracted') }}</span>
            <code v-for="(v, k) in row.result.extracted" :key="k" class="se-var-chip">
              {{ varTag(k) }} = {{ v }}
            </code>
          </div>
          <div v-if="respOpenRows.has(row.pos) && row.result" class="se-rresp">
            <RequestDetail
              v-if="row.result.response"
              :response="row.result.response"
              :url="row.result.url ?? undefined"
            />
            <p v-else-if="row.result.request_error" class="se-rerr">{{ row.result.request_error }}</p>
            <ResponsePanel
              v-if="row.result.response"
              :response="row.result.response"
              :load-full-body="row.result.response.body_omitted ? fullBodyLoader(row.pos) : undefined"
            />
          </div>
        </div>
        <p v-if="livePending" class="se-pending-hint">
          <Icon name="dot" :size="11" /> {{ t('scenario.pendingHint') }}
        </p>
      </template>
      <template v-else>
      <div
        v-for="(r, i) in runResult?.results ?? []"
        :key="i"
        class="se-rrow"
        :class="{ fail: !r.ok && !r.cancelled, cancelled: r.cancelled }"
      >
        <div class="se-rrow-head">
          <!-- 行首定位：点击方法 + 名称定位到编排区对应步骤卡 -->
          <button type="button" class="se-rloc-btn" :title="t('scenario.locateStep')" @click.stop="emit('locate', i)">
            <span class="se-rdot" :class="{ fail: !r.ok && !r.cancelled, cancelled: r.cancelled }" aria-hidden="true"></span>
            <span class="se-rmethod" :class="methodBadgeTone(r.method)">{{ r.method }}</span>
            <span class="se-rname">{{ i + 1 }}. {{ r.endpoint_name }}</span>
          </button>
          <span v-if="r.status != null" class="se-rstatus">{{ r.status }}</span>
          <span v-if="r.duration_ms != null" class="se-rtime">{{ formatDurationShort(r.duration_ms) }}</span>
          <span class="se-rtime se-rtime-assert" :title="t('scenario.assertPassedHint')">
            {{ t('scenario.assertPassed', { n: r.outcomes.filter((o) => o.passed).length, m: r.outcomes.length }) }}
          </span>
          <IconButton
            v-if="r.response || r.request_error"
            :name="respOpenRows.has(i) ? 'chevrons-down-up' : 'chevrons-up-down'"
            :size="13"
            :title="t('scenario.viewRequestDetail')"
            @click.stop="emit('toggle-row', i)"
          />
        </div>
        <code v-if="r.url" class="se-rurl" :title="r.url">{{ r.url }}</code>
        <p v-if="holdText(r)" class="se-rblocked" :class="{ blocked: holdText(r)?.blocked }">
          <Icon name="alert-triangle" :size="11" />
          <template v-if="holdText(r)?.blocked">{{ t('integration.blockedReason') }}：</template>{{ holdText(r)?.reason }}
        </p>
        <p v-else-if="r.request_error" class="se-rerr">{{ r.request_error }}</p>
        <ul v-if="r.outcomes.length" class="se-routcomes">
          <li v-for="(o, j) in r.outcomes" :key="j" :class="{ fail: !o.passed }">
            {{ o.description }}
            <span v-if="o.expected != null || o.actual != null" class="se-r-outcome-va">
              <template v-if="o.expected != null"> · {{ t('scenario.outcomeExpected', { v: o.expected }) }}</template>
              <template v-if="o.actual != null"> · {{ t('scenario.outcomeActual', { v: o.actual }) }}</template>
            </span>
            <span v-else-if="o.reason"> — {{ o.reason }}</span>
          </li>
        </ul>
        <p v-if="r.response?.history_truncated" class="se-rtrunc">{{ t('integration.historyTruncated') }}</p>
        <!-- 响应正文速览：不展开也能看到返回了什么 -->
        <pre v-if="peekOf(r.response)" class="se-rpeek">{{ peekOf(r.response) }}</pre>
        <div v-if="r.extracted && Object.keys(r.extracted).length" class="se-rext">
          <span class="se-rext-label">{{ t('scenario.extracted') }}</span>
          <code v-for="(v, k) in r.extracted" :key="k" class="se-var-chip">
            {{ varTag(k) }} = {{ v }}
          </code>
        </div>
        <!-- 请求参数 + 响应详情：复用单请求的查看组件（请求快照 / 正文预览 /
             按需取全文）；请求级错误时也给出可展开的入口，避免只剩一行红字。 -->
        <div v-if="respOpenRows.has(i) && (r.response || r.request_error)" class="se-rresp">
          <RequestDetail v-if="r.response" :response="r.response" :url="r.url ?? undefined" />
          <p v-else-if="r.request_error" class="se-rerr">{{ r.request_error }}</p>
          <ResponsePanel
            v-if="r.response"
            :response="r.response"
            :load-full-body="r.response.body_omitted ? fullBodyLoader(i) : undefined"
          />
        </div>
      </div>
      <p v-if="runResult?.results.length" class="se-locate-hint">
        <Icon name="arrow-up-right" :size="11" /> {{ t('scenario.locateStep') }}
      </p>
      </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* ---- 执行结果 Dock ---- */
.se-report {
  flex: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
}

.se-report.fail {
  border-color: var(--danger-border, var(--danger));
}

.se-report-headrow {
  display: flex;
  align-items: center;
  gap: 2px;
  padding-right: 6px;
}

.se-report-head {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  border: none;
  background: transparent;
  padding: 8px 10px;
  cursor: pointer;
  text-align: left;
  color: var(--text-2);
  border-radius: var(--radius-lg);
}

.se-report-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.se-report-meta,
.se-report-warn {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-report-warn {
  color: var(--danger);
}

.se-report-time {
  margin-left: auto;
}

.se-report-badge {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 8px;
  border-radius: 999px;
}

.se-report-badge.ok {
  background: var(--success-tint);
  color: var(--success);
}

.se-report-badge.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.se-report-badge.warn {
  background: var(--warning-tint);
  color: var(--warning);
}

/* 摘要统计 chips：数字优先，hover 出全量说明 */
.se-report-chips {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.se-chip {
  flex: none;
  display: inline-flex;
  align-items: center;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
}

.se-chip.ok {
  background: var(--success-tint);
  color: var(--success);
}

.se-chip.fail {
  background: var(--danger-tint);
  color: var(--danger);
}

/* 失败数为 0 时弱化（成功态下红色 chip 反而抢眼） */
.se-chip.fail.dim {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-chip.skip {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-chip.assert {
  background: var(--info-tint);
  color: var(--info);
}

.se-chip.time {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-report-caret {
  margin-left: auto;
  color: var(--text-3);
}

.se-report-time + .se-report-caret {
  margin-left: 0;
}

.se-report-body {
  border-top: 1px solid var(--border);
  padding: 8px 10px 10px;
  display: flex;
  flex-direction: column;
  gap: 7px;
}

/* Dock 正文限高 + 内部滚动：展开响应详情时整页不被撑出去，
   ResponsePanel 才能在界内拿到高度渲染正文。 */
.se-report-scroll {
  display: flex;
  flex-direction: column;
  gap: 7px;
  max-height: 40vh;
  min-height: 0;
  overflow-y: auto;
}

.se-progress {
  padding: 2px 0 4px;
}

.se-progress-track {
  height: 3px;
  border-radius: 999px;
  background: var(--bg-hover);
  overflow: hidden;
}

.se-progress-bar {
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width 0.3s var(--ease);
}

@media (prefers-reduced-motion: reduce) {
  .se-progress-bar {
    transition: none;
  }
}

.se-rrow {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.se-rrow:hover {
  border-color: var(--border-strong);
}

.se-rrow.fail {
  border-color: var(--danger-border, var(--danger));
}

.se-rrow.cancelled {
  opacity: 0.75;
}

.se-rdot.cancelled {
  background: var(--warning);
}

/* 渲染后的实际 URL（含变量替换结果），定位「跑的是哪个地址」 */
.se-rurl {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 结构化断言对比（期望 x · 实际 y），失败行红色承接 */
.se-r-outcome-va {
  font-family: var(--font-mono);
  word-break: break-all;
}

/* 响应详情：复用 ResponsePanel（弹性布局，需给界内固定高度）。 */
.se-rresp {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px dashed var(--border);
  padding-top: 8px;
}

.se-rresp > :deep(.response-panel),
.se-rresp > .response-panel {
  flex: 1;
  min-height: 320px;
}

/* 行内响应正文速览：常显一段（不展开也能看到返回了什么） */
.se-rpeek {
  margin: 2px 0 0;
  max-height: 120px;
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-2);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-left: 2px solid var(--success);
  border-radius: var(--radius-sm);
  padding: 5px 8px;
  white-space: pre-wrap;
  word-break: break-all;
}

.se-rrow.fail .se-rpeek {
  border-left-color: var(--danger);
}

.se-rrow-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

/* 行首定位按钮（点 / 方法 / 名称一体）：hover 提示定位语义 */
.se-rloc-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  border: none;
  background: transparent;
  padding: 0;
  cursor: pointer;
  text-align: left;
}

.se-rloc-btn:hover .se-rname {
  color: var(--accent);
}

.se-rdot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
}

.se-rdot.fail {
  background: var(--danger);
}

/* 在途步：呼吸点，配合「执行中」文案 */
.se-rdot.live {
  background: var(--accent);
  animation: se-rdot-pulse 1.2s var(--ease) infinite;
}

@keyframes se-rdot-pulse {
  0%,
  100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.35;
    transform: scale(0.75);
  }
}

@media (prefers-reduced-motion: reduce) {
  .se-rdot.live {
    animation: none;
  }
}

.se-rtime-live {
  color: var(--accent);
}

.se-rtime-assert {
  color: var(--text-3);
}

.se-pending-hint {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 步骤卡是定位入口：结果行行首可点定位，底部提示承接说明 */
.se-locate-hint {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  opacity: 0.75;
}

/* Method 徽章（.se-rmethod 全局样式，见 style.css 场景编排区） */

.se-rname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.se-rstatus {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.se-rtime {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-rerr {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--danger);
  white-space: pre-wrap;
  word-break: break-all;
}

.se-rtrunc {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--warning);
}

.se-rblocked {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  word-break: break-all;
}

.se-rblocked.blocked {
  color: var(--warning);
}

.se-routcomes {
  margin: 0;
  padding: 0 0 0 16px;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.se-routcomes li {
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-2);
  word-break: break-all;
}

.se-routcomes li.fail {
  color: var(--danger);
}

.se-rext {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.se-rext-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 变量 chip（.se-var-chip 全局样式，见 style.css 场景编排区） */
</style>
