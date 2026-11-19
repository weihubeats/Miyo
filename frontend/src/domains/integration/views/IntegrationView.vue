<script setup lang="ts">
/**
 * IntegrationView：集成测试视图（顶级路由 /integration）。
 *
 * 独立业务模块：把多接口业务流程（下单 → 支付预览 → 支付）编排成
 * 自动执行的测试——步骤间 {{var}} 传递变量、逐步断言、执行历史可回看。
 *
 * 结构：
 * - 顶栏：PageHeader 统一导航——列表态返回钮（useGoBack 统一策略），
 *   编辑器态面包屑（集成测试 / 场景名，父级可点 = 回列表 + 脏确认）；
 *   右侧项目选择器 + 环境选择器（复用全局 EnvironmentSelector，
 *   数据与写入同源 workspace store——项目切换走 workspace.switchProject，
 *   环境列表 / 激活环境天然一致）；
 * - 主体：场景库列表（ScenarioLibrary）↔ 工作流编辑器（WorkflowEditor），
 *   由 integration store 的 activeScenarioId 驱动（跨路由返回不闪，见 store）。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import PageHeader from '@/components/navigation/PageHeader.vue'
import Breadcrumbs, { type BreadcrumbItem } from '@/components/navigation/Breadcrumbs.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Modal from '@/components/ui/Modal.vue'
import Icon from '@/components/ui/Icon.vue'
import { useGoBack } from '@/composables/useGoBack'
import EnvironmentSelector from '@/domains/environment/components/EnvironmentSelector.vue'
import ScenarioLibrary from '@/domains/integration/components/integration/ScenarioLibrary.vue'
import WorkflowEditor from '@/domains/integration/components/integration/WorkflowEditor.vue'

const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const store = useIntegrationStore()

/** 编辑器实例引用：编辑器内返回要经过它的脏数据确认（requestBack）。 */
const editorRef = ref<InstanceType<typeof WorkflowEditor> | null>(null)

/** 统一返回策略（第四阶段）：有会话历史原路返回，深链直开回仪表板兜底。 */
const goBackFrom = useGoBack()

/**
 * 返回：按当前视图分层——
 * - 编辑器内（activeScenarioId 非空）：回场景列表，不离开集成测试页；
 *   未保存的修改由编辑器的 requestBack 弹确认，避免误丢改动；
 * - 列表页：返回来源页——可从仪表板导航或工作区工具进入，不写死回
 *   /workspace——统一走 useGoBack（会话内有历史则原路返回，直接深链
 *   打开时回仪表板兜底）。
 */
function goBack(): void {
  if (store.activeScenarioId) {
    editorRef.value?.requestBack()
    return
  }
  goBackFrom()
}

/** 返回钮提示随视图切换：编辑器内提示回列表，列表页提示回上一页。 */
const backHint = computed(() =>
  store.activeScenarioId ? t('integration.backToLib') : t('nav.backHint'),
)

/**
 * 编辑器态面包屑（集成测试 / 场景名）：父级「集成测试」可点 = requestBack
 * （先过脏确认回列表）；当前场景名不可点。场景未入库（新建流程）时以
 * 「未命名场景」占位。
 */
const crumbs = computed<BreadcrumbItem[]>(() => [
  {
    label: t('integration.title'),
    onSelect: () => editorRef.value?.requestBack(),
  },
  {
    label: store.libraryItems.find((s) => s.id === store.activeScenarioId)?.name ?? t('integration.untitledScenario'),
  },
])

/** 项目下拉选项（名称排序与仪表板一致按原始顺序）。 */
const projectOptions = () =>
  store.projects.map((p) => ({ value: p.id, label: p.name }))

/** 切换项目：先走 workspace 的激活链路（持久化 + 环境加载），再同步本 store。 */
async function doSwitchProject(id: string): Promise<void> {
  try {
    await workspace.switchProject(id)
  } catch {
    // 项目已不存在：仅切换本 store（列表会以 404 空态呈现），不打断视图
  }
  store.setProject(id)
}

/**
 * 编辑器态切换项目的脏保护：未保存修改不得被静默丢弃——先确认
 * （保存并切换 / 放弃并切换 / 留在编辑器）；保存失败不得切换。
 * 列表态（activeScenarioId 为空）直接切换。
 */
const switchDirtyOpen = ref(false)
const pendingProjectId = ref<string | null>(null)

async function onSelectProject(value: string | number): Promise<void> {
  const id = String(value ?? '')
  if (!id || id === store.projectId) return
  if (store.activeScenarioId && editorRef.value?.isDirty()) {
    pendingProjectId.value = id
    switchDirtyOpen.value = true
    return
  }
  await doSwitchProject(id)
}

/** 保存并切换：保存失败留在编辑器（弹窗保留，toast 已提示原因）。 */
async function saveAndSwitch(): Promise<void> {
  const target = pendingProjectId.value
  if (!target) return
  const ok = await editorRef.value?.save()
  if (!ok) return
  switchDirtyOpen.value = false
  pendingProjectId.value = null
  await doSwitchProject(target)
}

/** 放弃修改并切换：不落库直接切（与返回确认的「放弃」同语义）。 */
function discardAndSwitch(): void {
  const target = pendingProjectId.value
  switchDirtyOpen.value = false
  pendingProjectId.value = null
  if (target) void doSwitchProject(target)
}

onMounted(async () => {
  // 首次进入：项目上下文默认跟随工作区激活项目；记忆的项目优先回切。
  await store.ensureProjectContext()
  const target = store.projectId
  if (target && workspace.project?.id !== target) {
    await onSelectProject(target)
  }
  // 跨路由返回不重拉整库：已有缓存时静默刷新（导航规范场景 A——
  // 返回恢复搜索/筛选/滚动，数据仅后台更新）。
  void store.loadLibrary(!store.libraryLoaded)
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
    <PageHeader
      :title="t('integration.title')"
      :hide-back="!!store.activeScenarioId"
      :back-title="backHint"
      @back="goBack"
    >
      <!-- 编辑器态：面包屑替代返回钮（父级可点 = 回列表 + 脏确认；
           规范：面包屑与返回钮不堆叠同一层级入口） -->
      <Breadcrumbs v-if="store.activeScenarioId" :items="crumbs" />
      <template #actions>
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
      </template>
    </PageHeader>
    <main class="it-body">
      <!-- :key 强制换场景时重建实例：编辑器以 props.scenarioId 一次性捕获
           场景上下文，复用实例会串场景（运行历史记错归属）。 -->
      <WorkflowEditor
        v-if="store.activeScenarioId"
        ref="editorRef"
        :key="store.activeScenarioId"
        :scenario-id="store.activeScenarioId"
        @back="store.backToLibrary"
      />
      <ScenarioLibrary v-else />
    </main>

    <!-- 编辑器态切换项目的脏保护：未保存修改不得被静默丢弃；
         保存并切换失败时弹窗保留（不切换），与返回确认同语义 -->
    <Modal v-model:open="switchDirtyOpen" :title="t('integration.switchDirtyTitle')" width="420px">
      <p class="it-switch-body">{{ t('integration.switchDirtyBody') }}</p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="switchDirtyOpen = false">
          {{ t('integration.backDirtyStay') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger-solid" type="button" @click="discardAndSwitch">
          {{ t('integration.switchDirtyDiscard') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="saveAndSwitch">
          <Icon name="save" :size="12" /> {{ t('integration.switchDirtySave') }}
        </button>
      </template>
    </Modal>
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

.it-switch-body {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
  line-height: 1.6;
}
</style>
