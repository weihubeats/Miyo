<script setup lang="ts">
/**
 * ArtifactPanel：右栏产物预览（场景草稿 / 用例草稿 / 修复 / 失败分析 /
 * 覆盖率 / 依赖关系）。Tabs 随产物出现；所有渲染只读 + 动作按钮。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useAiTestStore } from '@/domains/agent/stores/aiTest'
import { methodBadgeTone } from '@/utils/methodTone'
import Icon from '@/components/ui/Icon.vue'
import type {
  CoverageReport,
  DependencyReport,
  FixProposal,
  ScenarioDraftResult,
  TestCaseDraft,
} from '@/types/miyoApi.d'

const t = useLocaleStore().t
const ai = useAiTestStore()

const asScenarioDraft = (d: unknown) => d as ScenarioDraftResult
interface CaseDraftsData {
  endpointId: string | null
  drafts: TestCaseDraft[]
}
const asCases = (d: unknown) => d as CaseDraftsData
const asFix = (d: unknown) => d as FixProposal
const asCoverage = (d: unknown) => d as CoverageReport
const asDeps = (d: unknown) => d as DependencyReport

/** 用例草稿勾选（默认全选）。 */
const selectedCases = ref<Set<string>>(new Set())
watch(
  () => ai.activeArtifactType,
  (type) => {
    if (type === 'test_case_drafts') {
      const draft = ai.artifacts.find((a) => a.type === 'test_case_drafts')
      if (draft) {
        selectedCases.value = new Set(asCases(draft.data).drafts.map((c) => c.name))
      }
    }
  },
  { immediate: true },
)
function toggleCase(name: string): void {
  const next = new Set(selectedCases.value)
  if (next.has(name)) next.delete(name)
  else next.add(name)
  selectedCases.value = next
}
const selectedCasesArtifact = computed(() => {
  const a = ai.artifacts.find((x) => x.type === 'test_case_drafts')
  return a ? asCases(a.data) : null
})
const selectedCount = computed(
  () => selectedCasesArtifact.value?.drafts.filter((c) => selectedCases.value.has(c.name)).length ?? 0,
)

/** 用例分类 chip 色（存库分类直接展示）。 */
const CATEGORY_TONE: Record<string, string> = {
  正向: 'text-success',
  负向: 'text-danger',
  边界值: 'text-warning',
  安全性: 'text-info',
  其他: 'text-2',
}
/** 修复 change 的风险色。 */
function riskTone(risk: string): string {
  if (risk === 'high') return 'text-danger'
  if (risk === 'medium') return 'text-warning'
  return 'text-success'
}

/** 提取规则行（展示用）。 */
interface ExtractRow {
  name: string
  path: string
}
function extractsOf(raw: unknown): ExtractRow[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((r) => r as { name?: string; path?: string })
    .filter((r) => r.name?.trim() && r.path?.trim())
    .map((r) => ({ name: r.name!.trim(), path: r.path!.trim() }))
}
interface AssertionRow {
  type: string
  path: string | null
  op: string
  expected: unknown
}
function assertionsOf(raw: unknown): AssertionRow[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((r) => r as AssertionRow)
    .filter((r) => r.type)
}
</script>

<template>
  <aside class="artifact-panel">
    <header class="ap-head">
      <h3 class="ap-title">{{ t('ai.artifacts') }}</h3>
      <button
        class="ap-toggle"
        :title="t('ai.artifacts')"
        @click="ai.artifactPanelOpen = !ai.artifactPanelOpen"
      >
        <Icon
        name="chevron-right"
        :size="13"
        :style="ai.artifactPanelOpen ? '' : 'transform: rotate(180deg)'"
      />
      </button>
    </header>

    <template v-if="ai.artifactPanelOpen">
      <nav v-if="ai.artifacts.length > 0" class="ap-tabs">
        <button
          v-for="a in ai.artifacts"
          :key="a.type"
          class="ap-tab"
          :class="{ active: ai.activeArtifactType === a.type }"
          @click="ai.activeArtifactType = a.type"
        >
          {{ t(`ai.artifact.${a.type}`) }}
        </button>
      </nav>

      <div v-if="ai.artifacts.length === 0" class="ap-empty">
        <p class="ap-empty-text">{{ t('ai.runDock.empty') }}</p>
      </div>

      <div v-else class="ap-body">
        <!-- ===== 场景草稿 ===== -->
        <template v-if="ai.activeArtifactType === 'scenario_draft'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'scenario_draft')"
            :key="a.type"
            class="ap-section"
          >
            <div
              v-for="(step, i) in asScenarioDraft(a.data).scenario.steps"
              :key="i"
              class="draft-step"
            >
              <span class="miyo-method method-badge" :class="methodBadgeTone(
                step.request ? step.request.method : undefined,
              )">
                {{
                  step.request
                    ? step.request.method
                    : ai.endpoints.find((e) => e.id === step.endpoint_id)?.method
                }}
              </span>
              <div class="step-main">
                <div class="step-name">
                  {{ step.name || ai.endpoints.find((e) => e.id === step.endpoint_id)?.name || `#${i + 1}` }}
                </div>
                <div class="step-url mono">
                  {{ step.request ? step.request.url : ai.endpoints.find((e) => e.id === step.endpoint_id)?.path }}
                </div>
                <div v-if="extractsOf(step.extract).length" class="step-chips">
                  <span
                    v-for="ex in extractsOf(step.extract)"
                    :key="`${ex.name}${ex.path}`"
                    class="chip chip-extract"
                  >
                    ⤓ {{ ex.name }} ← {{ ex.path }}
                  </span>
                </div>
                <div v-if="assertionsOf(step.assertions).length" class="step-chips">
                  <span
                    v-for="(asr, j) in assertionsOf(step.assertions)"
                    :key="j"
                    class="chip chip-assert"
                  >
                    ✓ {{ asr.type }}{{ asr.path ? ` ${asr.path}` : '' }} {{ asr.op }}
                    {{ asr.expected }}
                  </span>
                </div>
              </div>
            </div>

            <!-- 变量依赖 -->
            <div class="sub-block">
              <h4 class="sub-title">{{ t('ai.varDeps') }}</h4>
              <div
                v-for="vd in asScenarioDraft(a.data).var_dependencies"
                :key="vd.step_index"
                class="vardep-row"
              >
                <span class="vardep-step">{{ vd.step_index + 1 }}. {{ vd.step_name }}</span>
                <span v-if="vd.produces.length" class="vardep-vals">
                  {{ t('ai.dep.produces') }}: {{ vd.produces.join(', ') }}
                </span>
                <span v-if="vd.consumes.length" class="vardep-vals">
                  {{ t('ai.dep.consumes') }}: {{ vd.consumes.join(', ') }}
                </span>
              </div>
            </div>

            <!-- 执行计划 / 写操作 -->
            <div v-if="asScenarioDraft(a.data).write_operations.length" class="sub-block">
              <h4 class="sub-title">
                {{ t('ai.writeOps', { n: asScenarioDraft(a.data).write_operations.length }) }}
              </h4>
              <div
                v-for="w in asScenarioDraft(a.data).write_operations"
                :key="w.step_index"
                class="writeop-row"
              >
                <Icon name="alert-triangle" :size="12" class="text-warning" />
                <span class="mono">{{ w.method }}</span>
                <span class="writeop-url mono">{{ w.url }}</span>
              </div>
            </div>

            <!-- 自动调整 / 警告 -->
            <div v-if="asScenarioDraft(a.data).adjustments.length" class="sub-block">
              <h4 class="sub-title">{{ t('ai.adjustments') }}</h4>
              <ul class="note-list">
                <li v-for="(adj, i) in asScenarioDraft(a.data).adjustments" :key="i">
                  {{ adj }}
                </li>
              </ul>
            </div>
            <div v-if="asScenarioDraft(a.data).warnings.length" class="sub-block">
              <h4 class="sub-title text-warning">{{ t('ai.warnings') }}</h4>
              <ul class="note-list">
                <li
                  v-for="(w, i) in asScenarioDraft(a.data).warnings"
                  :key="i"
                  class="text-warning"
                >
                  {{ w }}
                </li>
              </ul>
            </div>

            <div class="action-row">
              <button
                class="miyo-btn miyo-btn-primary miyo-btn-sm"
                @click="ai.saveScenarioFromDraft(asScenarioDraft(a.data))"
              >
                <Icon name="save" :size="12" />
                {{ t('ai.saveScenario') }}
              </button>
            </div>
          </section>
        </template>

        <!-- ===== 用例草稿 ===== -->
        <template v-else-if="ai.activeArtifactType === 'test_case_drafts'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'test_case_drafts')"
            :key="a.type"
            class="ap-section"
          >
            <div v-for="c in asCases(a.data).drafts" :key="c.kind" class="case-row">
              <label class="case-check">
                <input
                  type="checkbox"
                  :checked="selectedCases.has(c.name)"
                  @change="toggleCase(c.name)"
                />
              </label>
              <div class="case-main">
                <div class="case-name">
                  {{ c.name }}
                  <span class="chip" :class="CATEGORY_TONE[c.category] ?? 'text-2'">
                    {{ c.category }}
                  </span>
                </div>
                <div class="case-desc">{{ c.description }}</div>
                <div v-if="c.notes.length" class="case-notes">{{ c.notes.join('；') }}</div>
              </div>
            </div>
            <div class="action-row">
              <button
                class="miyo-btn miyo-btn-primary miyo-btn-sm"
                :disabled="selectedCount === 0 || !selectedCasesArtifact?.endpointId"
                @click="
                  selectedCasesArtifact &&
                    ai.saveTestCaseDrafts(
                      selectedCasesArtifact.endpointId!,
                      selectedCasesArtifact.drafts.filter((c) => selectedCases.has(c.name)),
                    )
                "
              >
                <Icon name="save" :size="12" />
                {{ t('ai.selectCases', { n: selectedCount }) }}
              </button>
            </div>
          </section>
        </template>

        <!-- ===== 修复方案 ===== -->
        <template v-else-if="ai.activeArtifactType === 'fix_proposal'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'fix_proposal')"
            :key="a.type"
            class="ap-section"
          >
            <div v-if="asFix(a.data).analysis.root_cause_summary" class="fix-summary">
              {{ asFix(a.data).analysis.root_cause_summary }}
            </div>
            <div
              v-if="asFix(a.data).analysis.rerun_advice"
              class="rerun-advice"
              :class="asFix(a.data).analysis.rerun_advice!.should_rerun ? 'ok' : 'warn'"
            >
              <Icon name="refresh" :size="12" />
              {{
                asFix(a.data).analysis.rerun_advice!.should_rerun
                  ? t('ai.rerunYes')
                  : t('ai.rerunNo')
              }}
              — {{ asFix(a.data).analysis.rerun_advice!.reason }}
            </div>

            <h4 class="sub-title">{{ t('ai.changes') }}</h4>
            <div v-for="(c, i) in asFix(a.data).changes" :key="i" class="fix-change">
              <div class="fix-change-head">
                <span class="mono fix-step">{{ c.step_index + 1 }}. {{ c.step_name }}</span>
                <span class="chip" :class="riskTone(c.risk)">
                  {{ c.risk === 'low' ? t('ai.riskLow') : t('ai.riskMedium') }}
                </span>
              </div>
              <div class="fix-field mono">{{ c.field }}</div>
              <div class="fix-diff">
                <div class="diff-line before">{{ c.before }}</div>
                <div class="diff-line after">{{ c.after }}</div>
              </div>
              <div class="fix-reason">{{ c.reason }}</div>
            </div>

            <div class="action-row">
              <button
                v-if="asFix(a.data).draft_steps && ai.runScenario"
                class="miyo-btn miyo-btn-primary miyo-btn-sm"
                @click="ai.applyFix(asFix(a.data))"
              >
                <Icon name="check" :size="12" />
                {{ t('ai.fixApply') }}
              </button>
              <button
                v-if="ai.runScenario"
                class="miyo-btn miyo-btn-sm"
                @click="ai.revertFix(ai.runScenario.id)"
              >
                {{ t('ai.fixRevert') }}
              </button>
            </div>
          </section>
        </template>

        <!-- ===== 失败分析 ===== -->
        <template v-else-if="ai.activeArtifactType === 'failure_analysis'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'failure_analysis')"
            :key="a.type"
            class="ap-section"
          >
            <div v-for="(fs, i) in (a.data as FixProposal['analysis']).failed_steps" :key="i" class="fail-step">
              <div class="fail-head">
                <span class="mono">{{ fs.index + 1 }}. {{ fs.name }}</span>
                <span class="chip text-danger mono">{{ fs.error_type }}</span>
                <span v-if="fs.status" class="chip text-2 mono">HTTP {{ fs.status }}</span>
              </div>
              <ul class="note-list">
                <li v-for="(c, j) in fs.possible_causes" :key="j">{{ c }}</li>
              </ul>
              <ul class="note-list suggestions">
                <li v-for="(s, j) in fs.suggestions" :key="j" class="text-info">→ {{ s }}</li>
              </ul>
            </div>
          </section>
        </template>

        <!-- ===== 覆盖率报告 ===== -->
        <template v-else-if="ai.activeArtifactType === 'coverage_report'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'coverage_report')"
            :key="a.type"
            class="ap-section"
          >
            <div class="score-block">
              <span class="score-num">{{ asCoverage(a.data).score }}</span>
              <span class="score-label">{{ t('ai.coverage.score') }} / 100 · {{ asCoverage(a.data).summary }}</span>
            </div>
            <div class="dim-grid">
              <div v-for="d in asCoverage(a.data).dimensions" :key="d.key" class="dim-row">
                <span
                  class="dim-count"
                  :class="d.endpoint_count > 0 ? 'text-warning' : 'text-success'"
                >
                  {{ d.endpoint_count }}
                </span>
                <span class="dim-label">{{ t(`ai.coverage.${d.key}`) }}</span>
              </div>
            </div>
            <h4 class="sub-title">{{ t('ai.coverage.endpoints') }}</h4>
            <div
              v-for="e in asCoverage(a.data).endpoints.filter((x) => x.issues.length > 0)"
              :key="e.endpoint_id"
              class="cov-endpoint"
            >
              <div class="cov-name">
                <span class="miyo-method method-badge" :class="methodBadgeTone(e.method)">
                  {{ e.method }}
                </span>
                {{ e.name }}
              </div>
              <div class="cov-issues">
                <span v-for="iss in e.issues" :key="iss" class="chip chip-assert">
                  {{ t(`ai.coverage.${iss}`) }}
                </span>
              </div>
            </div>
          </section>
        </template>

        <!-- ===== 依赖关系 ===== -->
        <template v-else-if="ai.activeArtifactType === 'dependency_report'">
          <section
            v-for="a in ai.artifacts.filter((x) => x.type === 'dependency_report')"
            :key="a.type"
            class="ap-section"
          >
            <div class="fix-summary">{{ asDeps(a.data).summary }}</div>
            <div v-for="e in asDeps(a.data).edges" :key="`${e.from}-${e.to}`" class="dep-edge">
              <div class="dep-nodes">
                <span class="dep-node">{{ asDeps(a.data).nodes.find((n) => n.endpoint_id === e.from)?.name }}</span>
                <Icon name="arrow-up-right" :size="12" class="dep-arrow" />
                <span class="dep-node">{{ asDeps(a.data).nodes.find((n) => n.endpoint_id === e.to)?.name }}</span>
                <span
                  class="chip"
                  :class="e.confidence === 'high' ? 'text-success' : 'text-3'"
                >
                  {{ e.confidence }}
                </span>
              </div>
              <div class="dep-via mono">
                {{ t('ai.dep.via') }}: {{ e.via.join(', ') }}
              </div>
              <div class="dep-evidence">{{ e.evidence }}</div>
            </div>
            <div
              v-if="asDeps(a.data).unresolved.length"
              class="sub-block"
            >
              <h4 class="sub-title text-warning">{{ t('ai.dep.unresolved') }}</h4>
              <div
                v-for="u in asDeps(a.data).unresolved"
                :key="u.endpoint_id"
                class="vardep-row"
              >
                <span class="vardep-step">{{ u.name }}</span>
                <span class="vardep-vals mono">{{ u.vars.join(', ') }}</span>
              </div>
            </div>
          </section>
        </template>
      </div>
    </template>
  </aside>
</template>

<style scoped>
.artifact-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  border-left: 1px solid var(--border);
  background: var(--bg-panel);
}
.ap-head {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--sp-2) var(--sp-3);
  border-bottom: 1px solid var(--border);
}
.ap-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.ap-toggle {
  border: none;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: var(--radius-sm);
}
.ap-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.ap-tabs {
  flex: none;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
  padding: var(--sp-2) var(--sp-3);
  border-bottom: 1px solid var(--border);
}
.ap-tab {
  padding: 2px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  background: var(--bg-panel);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  cursor: pointer;
}
.ap-tab.active {
  background: var(--bg-hover);
  border-color: var(--border-strong);
  color: var(--text-1);
}
.ap-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--sp-4);
}
.ap-empty-text {
  color: var(--text-3);
  font-size: var(--fs-xs);
  text-align: center;
}
.ap-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.ap-section {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

/* 场景草稿步骤 */
.draft-step {
  display: flex;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}
.method-badge {
  flex: none;
  align-self: flex-start;
  font-size: var(--fs-xxs);
}
.step-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.step-name {
  font-size: var(--fs-sm);
  color: var(--text-1);
  font-weight: 500;
}
.step-url {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  word-break: break-all;
}
.mono {
  font-family: var(--font-mono, monospace);
}
.step-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}
.chip {
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  background: var(--bg-hover);
  color: var(--text-2);
}
.chip-extract {
  background: var(--info-tint);
  color: var(--info);
}
.chip-assert {
  background: var(--bg-hover);
}

.sub-block {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.sub-title {
  margin: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}
.vardep-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}
.vardep-step {
  color: var(--text-1);
  min-width: 90px;
}
.vardep-vals {
  color: var(--text-3);
}
.writeop-row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}
.writeop-url {
  color: var(--text-3);
  word-break: break-all;
}
.note-list {
  margin: 0;
  padding-left: var(--sp-4);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  line-height: 1.7;
}
.action-row {
  display: flex;
  gap: var(--sp-2);
  padding-top: var(--sp-1);
}

/* 用例草稿 */
.case-row {
  display: flex;
  gap: var(--sp-2);
  padding: var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}
.case-check {
  flex: none;
  padding-top: 2px;
}
.case-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.case-name {
  font-size: var(--fs-sm);
  color: var(--text-1);
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
}
.case-desc {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.case-notes {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-style: italic;
}

/* 修复 */
.fix-summary {
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--radius-md);
  background: var(--danger-tint);
  color: var(--text-1);
  font-size: var(--fs-xs);
}
.rerun-advice {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xs);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--radius-md);
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-2);
}
.rerun-advice.ok {
  border-color: var(--success);
}
.rerun-advice.warn {
  border-color: var(--warning);
}
.fix-change {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--sp-2);
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  background: var(--bg-card);
}
.fix-change-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}
.fix-step {
  font-size: var(--fs-xs);
  color: var(--text-1);
}
.fix-field {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.fix-diff {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.diff-line {
  font-size: var(--fs-xxs);
  font-family: var(--font-mono, monospace);
  padding: 3px var(--sp-2);
  border-radius: var(--radius-sm);
  word-break: break-all;
}
.diff-line.before {
  background: var(--danger-tint);
  color: var(--text-1);
}
.diff-line.after {
  background: var(--success-tint);
  color: var(--text-1);
}
.fix-reason {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 失败分析 */
.fail-step {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--sp-2);
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.fail-head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
  font-size: var(--fs-xs);
  color: var(--text-1);
}
.suggestions li {
  list-style: none;
}

/* 覆盖率 */
.score-block {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
}
.score-num {
  font-size: 28px;
  font-weight: 700;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}
.score-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.dim-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--sp-1) var(--sp-3);
}
.dim-row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xxs);
}
.dim-count {
  min-width: 18px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.dim-label {
  color: var(--text-2);
}
.cov-endpoint {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-2) 0;
  border-bottom: 1px dashed var(--border);
}
.cov-name {
  font-size: var(--fs-xs);
  color: var(--text-1);
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.cov-issues {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
  padding-left: 40px;
}

/* 依赖 */
.dep-edge {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--sp-2);
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  background: var(--bg-card);
}
.dep-nodes {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
}
.dep-node {
  font-size: var(--fs-xs);
  color: var(--text-1);
}
.dep-arrow {
  color: var(--accent);
}
.dep-via {
  font-size: var(--fs-xxs);
  color: var(--text-2);
}
.dep-evidence {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
