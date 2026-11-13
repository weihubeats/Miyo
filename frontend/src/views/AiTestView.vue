<script setup lang="ts">
/**
 * AiTestView：AI 自动化测试工作区（/ai-test）。
 *
 * 三栏：左=项目接口与业务流程（含依赖 / 覆盖率入口）；
 * 中=AI 对话与场景生成；右=产物预览 + 场景执行坞。
 * 保持 Miyo 现有深色设计语言（IntegrationView 同款壳）。
 */
import { computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useLocaleStore } from '../stores/locale'
import { useAiTestStore } from '../stores/aiTest'
import { useWorkspaceStore } from '../stores/workspace'
import Brand from '../components/Brand.vue'
import CustomSelect from '../components/ui/CustomSelect.vue'
import Icon from '../components/ui/Icon.vue'
import ContextPanel from '../components/ai/ContextPanel.vue'
import ChatPanel from '../components/ai/ChatPanel.vue'
import ArtifactPanel from '../components/ai/ArtifactPanel.vue'
import RunDock from '../components/ai/RunDock.vue'

const t = useLocaleStore().t
const router = useRouter()
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

function goBack(): void {
  if (history.state.back) router.back()
  else router.push('/projects')
}
</script>

<template>
  <div class="ai-root">
    <header class="ai-topbar">
      <Brand :title="t('ai.title')" />
      <button class="miyo-btn miyo-btn-sm no-drag" @click="goBack">
        <Icon name="chevron-right" :size="12" style="transform: rotate(180deg)" />
        ← {{ t('integration.back') }}
      </button>
      <h1 class="ai-topbar-title">{{ t('ai.title') }}</h1>
      <div class="topbar-spacer" />
      <div class="no-drag">
        <CustomSelect
          :model-value="ai.projectId ?? ''"
          :options="projectOptions"
          size="sm"
          :pop-min-width="220"
          :placeholder="t('ai.noProject')"
          @update:model-value="(v: string | number) => workspace.switchProject(String(v))"
        />
      </div>
    </header>

    <main v-if="!ai.projectId" class="ai-body-empty">
      <Icon name="zap" :size="32" class="empty-zap" />
      <p>{{ t('ai.noProject') }}</p>
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
.ai-topbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: 0 var(--sp-3);
  height: var(--h-lg);
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  -webkit-app-region: drag;
}
.no-drag {
  -webkit-app-region: no-drag;
}
.ai-topbar-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.topbar-spacer {
  flex: 1;
}

.ai-body-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--sp-3);
  color: var(--text-3);
}
.empty-zap {
  color: var(--accent);
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
