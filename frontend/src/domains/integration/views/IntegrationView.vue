<script setup lang="ts">
/**
 * IntegrationView：集成测试视图（顶级路由 /integration）。
 *
 * 独立业务模块：把多接口业务流程（下单 → 支付预览 → 支付）编排成
 * 自动执行的测试——步骤间 {{var}} 传递变量、逐步断言、执行历史可回看。
 *
 * 结构：
 * - 顶栏：返回 + 标题 + 项目选择器 + 环境选择器（复用全局 EnvironmentSelector，
 *   数据与写入同源 workspace store——项目切换走 workspace.switchProject，
 *   环境列表 / 激活环境天然一致）；
 * - 主体：场景库列表（ScenarioLibrary）↔ 工作流编辑器（WorkflowEditor），
 *   由 integration store 的 activeScenarioId 驱动（跨路由返回不闪，见 store）。
 */
import { onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import Brand from '@/domains/workspace/components/Brand.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EnvironmentSelector from '@/domains/environment/components/EnvironmentSelector.vue'
import ScenarioLibrary from '@/domains/integration/components/integration/ScenarioLibrary.vue'
import WorkflowEditor from '@/domains/integration/components/integration/WorkflowEditor.vue'

const router = useRouter()
const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const store = useIntegrationStore()

/**
 * 返回来源页：可从仪表板导航或工作区工具进入，不写死回 /workspace——
 * 会话内有历史则原路返回，直接深链打开时回仪表板兜底。
 */
function goBack(): void {
  if (window.history.state?.back) router.back()
  else void router.push('/projects')
}

/** 项目下拉选项（名称排序与仪表板一致按原始顺序）。 */
const projectOptions = () =>
  store.projects.map((p) => ({ value: p.id, label: p.name }))

/** 切换项目：先走 workspace 的激活链路（持久化 + 环境加载），再同步本 store。 */
async function onSelectProject(value: string | number): Promise<void> {
  const id = String(value ?? '')
  if (!id) return
  try {
    await workspace.switchProject(id)
  } catch {
    // 项目已不存在：仅切换本 store（列表会以 404 空态呈现），不打断视图
  }
  store.setProject(id)
}

onMounted(async () => {
  // 首次进入：项目上下文默认跟随工作区激活项目；记忆的项目优先回切。
  await store.ensureProjectContext()
  const target = store.projectId
  if (target && workspace.project?.id !== target) {
    await onSelectProject(target)
  }
  void store.loadLibrary()
})

// 项目切换（含首次解析完成）→ 刷新场景库；跨路由返回时已加载则静默刷新。
watch(
  () => store.projectId,
  () => {
    void store.loadLibrary(!store.libraryLoaded)
  },
)
</script>

<template>
  <div class="it-root">
    <header class="it-topbar">
      <!-- Brand 打头：macOS 交通灯落在品牌区上方，可点元素不被遮挡（同 WorkspaceView 惯例） -->
      <Brand :title="t('integration.title')" class="it-brand" />
      <button class="miyo-btn miyo-btn-sm" type="button" :title="t('integration.backHint')" @click="goBack">
        ← {{ t('integration.back') }}
      </button>
      <div class="it-topbar-spacer"></div>
      <CustomSelect
        class="it-project"
        :model-value="store.projectId"
        :options="projectOptions()"
        :placeholder="t('integration.projectLabel')"
        size="sm"
        :pop-min-width="220"
        @update:model-value="onSelectProject"
      />
      <EnvironmentSelector variant="group" />
    </header>
    <main class="it-body">
      <!-- :key 强制换场景时重建实例：编辑器以 props.scenarioId 一次性捕获
           场景上下文，复用实例会串场景（运行历史记错归属）。 -->
      <WorkflowEditor
        v-if="store.activeScenarioId"
        :key="store.activeScenarioId"
        :scenario-id="store.activeScenarioId"
        @back="store.backToLibrary"
      />
      <ScenarioLibrary v-else />
    </main>
  </div>
</template>

<style scoped>
.it-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
  background: var(--bg-app);
  color: var(--text-1);
}

.it-topbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px 8px 8px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  -webkit-app-region: drag;
}

.it-topbar > * {
  -webkit-app-region: no-drag;
}

.it-topbar-spacer {
  flex: 1;
}


.it-project {
  min-width: 160px;
  max-width: 240px;
}

.it-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
</style>
