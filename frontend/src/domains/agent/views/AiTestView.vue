<script setup lang="ts">
/**
 * AiTestView：AI 自动化测试工作区（/ai-test）。
 *
 * 三栏：左=项目接口与业务流程（含依赖 / 覆盖率入口）；
 * 中=AI 对话与场景生成；右=产物预览 + 场景执行坞。
 * 保持 Miyo 现有深色设计语言（IntegrationView 同款壳）。
 */
import { computed, onMounted } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useAiTestStore } from '@/domains/agent/stores/aiTest'
import { useWorkspaceStore } from '@/stores/workspace'
import PageHeader from '@/components/navigation/PageHeader.vue'
import { useGoBack } from '@/composables/useGoBack'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import ContextPanel from '@/domains/agent/components/ai/ai/ContextPanel.vue'
import ChatPanel from '@/domains/agent/components/ai/ai/ChatPanel.vue'
import ArtifactPanel from '@/domains/agent/components/ai/ai/ArtifactPanel.vue'
import RunDock from '@/domains/agent/components/ai/ai/RunDock.vue'
import Tooltip from '@/components/ui/Tooltip.vue'

const t = useLocaleStore().t
const ai = useAiTestStore()
const workspace = useWorkspaceStore()

onMounted(async () => {
  await ai.bindEvents()
  await ai.initContext()
  await ai.loadSessions()
  await ai.loadScenarios()
})

const projectOptions = computed(() =>
  ai.projects.map((p) => ({ value: p.id, label: p.name })),
)

/** 统一返回策略（第四阶段）：有会话历史原路返回，深链直开回仪表板兜底。 */
const goBack = useGoBack()
</script>

<template>
  <div class="ai-root">
    <PageHeader :title="t('ai.title')" @back="goBack">
      <!-- 测试中标记：常驻且不可关闭——页面仍在测试，结果可能不可靠，需持续可见 -->
      <Tooltip :content="t('ai.betaTip')" placement="bottom">
        <span class="ai-beta-badge no-drag" role="status">
          <span class="ai-beta-dot"></span>
          {{ t('ai.beta') }}
        </span>
      </Tooltip>
      <template #actions>
        <CustomSelect
          :model-value="ai.projectId ?? ''"
          :options="projectOptions"
          size="sm"
          :pop-min-width="220"
          :placeholder="t('ai.noProject')"
          @update:model-value="(v: string | number) => workspace.switchProject(String(v))"
        />
      </template>
    </PageHeader>

    <main v-if="!ai.projectId" class="ai-body-empty">
      <EmptyState icon="zap" :title="t('ai.noProject')" />
    </main>

    <main v-else class="ai-body">
      <ContextPanel class="pane-left" />
      <div class="pane-mid">
        <div class="mid-chat">
          <ChatPanel />
        </div>
        <div v-if="ai.runScenario" class="mid-dock">
          <RunDock :key="ai.runScenario.id" :scenario="ai.runScenario" :run-token="ai.runToken" />
        </div>
      </div>
      <ArtifactPanel class="pane-right" />
    </main>
  </div>
</template>

<style scoped>
.ai-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--bg-app);
}
.no-drag {
  -webkit-app-region: no-drag;
}

/* 「测试中」标记：amber 描边 + 静态圆点，常驻不可关闭（主题差异走变量）；
   功能状态文字 11px 地板 + 静态指示（impeccable undersized-ui-text / pulsing-dot） */
.ai-beta-badge {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  line-height: 1.6;
  padding: 0 7px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--warning) 55%, transparent);
  background: var(--warning-tint);
  color: var(--warning);
  cursor: help;
  user-select: none;
  white-space: nowrap;
}
.ai-beta-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
}

.ai-body-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.ai-body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.pane-left {
  flex: none;
  width: 250px;
  border-right: 1px solid var(--border);
  background: var(--bg-panel);
}
.pane-mid {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.mid-chat {
  flex: 1;
  min-height: 0;
}
/* 执行坞占下方 40%（可收起时隐藏）。 */
.mid-dock {
  flex: none;
  height: 42%;
  min-height: 220px;
  border-top: 1px solid var(--border);
  padding: 0 var(--sp-3) var(--sp-2);
  background: var(--bg-app);
  overflow: hidden;
}
.pane-right {
  flex: none;
  width: 340px;
}
</style>
