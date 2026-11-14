<script setup lang="ts">
/**
 * ContextPanel：左栏项目上下文（接口 / 业务流程 / 依赖与覆盖率入口）。
 * 接口行点击 → 让 AI 生成该接口的测试用例；流程行点击 → 运行场景。
 */
import { useLocaleStore } from '@/stores/locale'
import { useAiTestStore } from '@/domains/agent/stores/aiTest'
import { methodBadgeTone } from '@/utils/methodTone'
import Icon from '@/components/ui/Icon.vue'

const t = useLocaleStore().t
const ai = useAiTestStore()

function askCases(endpointId: string, name: string): void {
  void ai.askCases(endpointId, name)
}

function runScenario(id: string): void {
  const sc = ai.scenarios.find((s) => s.id === id)
  if (sc) ai.runSavedScenario(sc)
}
</script>

<template>
  <aside class="ctx-panel">
    <!-- 分析入口 -->
    <div class="ctx-actions">
      <button class="ctx-action miyo-btn" :disabled="ai.analyzing" @click="ai.loadDependencies()">
        <Icon name="network" :size="13" />
        <span>{{ t('ai.dependencyBtn') }}</span>
        <span v-if="ai.dependencyReport" class="ctx-count">
          {{ ai.dependencyReport.edges.length }}
        </span>
      </button>
      <button class="ctx-action miyo-btn" :disabled="ai.analyzing" @click="ai.loadCoverage()">
        <Icon name="gauge" :size="13" />
        <span>{{ t('ai.coverageBtn') }}</span>
        <span v-if="ai.coverageReport" class="ctx-count">{{ ai.coverageReport.score }}</span>
      </button>
    </div>

    <!-- 接口 -->
    <div class="ctx-group">
      <h4 class="ctx-group-title">
        {{ t('ai.contextEndpoints', { n: ai.endpoints.length }) }}
      </h4>
      <div class="ctx-list">
        <button
          v-for="ep in ai.endpoints"
          :key="ep.id"
          class="ctx-row"
          :title="t('ai.quick.cases')"
          @click="askCases(ep.id, ep.name)"
        >
          <span class="miyo-method method-badge" :class="methodBadgeTone(ep.method)">
            {{ ep.method }}
          </span>
          <span class="ctx-row-name">{{ ep.name }}</span>
        </button>
      </div>
    </div>

    <!-- 业务流程（场景） -->
    <div class="ctx-group">
      <h4 class="ctx-group-title">
        {{ t('ai.contextScenarios', { n: ai.scenarios.length }) }}
      </h4>
      <div class="ctx-list">
        <div v-if="ai.scenarios.length === 0" class="ctx-empty">
          {{ t('ai.runDock.empty') }}
        </div>
        <button v-for="sc in ai.scenarios" :key="sc.id" class="ctx-row" @click="runScenario(sc.id)">
          <Icon name="beaker" :size="12" class="text-accent" />
          <span class="ctx-row-name">{{ sc.name }}</span>
          <span class="ctx-count">{{ sc.steps.length }}</span>
        </button>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.ctx-panel {
  height: 100%;
  overflow-y: auto;
  padding: var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.ctx-actions {
  display: flex;
  gap: var(--sp-2);
}
/* 视觉（边框/底色/字号/高度/hover）由 miyo-btn 全局类单源提供；此处只留布局职责。 */
.ctx-action {
  flex: 1;
  gap: var(--sp-2);
}
.ctx-count {
  margin-left: auto;
  padding: 0 6px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-1);
  font-size: var(--fs-xxs);
  font-variant-numeric: tabular-nums;
}
.ctx-group {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.ctx-group-title {
  margin: 0;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.ctx-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.ctx-row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: 4px var(--sp-2);
  border-radius: var(--radius-sm);
  border: none;
  background: none;
  color: var(--text-2);
  font-size: var(--fs-xs);
  cursor: pointer;
  text-align: left;
}
.ctx-row:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.ctx-row-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.method-badge {
  flex: none;
  font-size: var(--fs-xxs);
}
.ctx-empty {
  padding: var(--sp-2);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.text-accent {
  color: var(--accent);
}
</style>
