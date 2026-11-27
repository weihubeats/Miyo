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
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import PageHeader from '@/components/navigation/PageHeader.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Modal from '@/components/ui/Modal.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import Icon from '@/components/ui/Icon.vue'
import { useGoBack } from '@/composables/useGoBack'
import { useTauriEvent } from '@/composables/useTauriEvent'
import EnvironmentSelector from '@/domains/environment/components/EnvironmentSelector.vue'
import ScenarioLibrary from '@/domains/integration/components/integration/ScenarioLibrary.vue'
import TestSuitePanel from '@/domains/integration/components/integration/TestSuitePanel.vue'
import BatchRunDialog from '@/domains/integration/components/integration/BatchRunDialog.vue'
import BatchReportPanel from '@/domains/integration/components/integration/BatchReportPanel.vue'
import WorkflowEditor from '@/domains/integration/components/integration/WorkflowEditor.vue'
import { useTestSuitesStore } from '@/domains/integration/stores/testSuites'
import type { TestBatchProgressEvent } from '@/types/miyoApi.d'

const t = useLocaleStore().t
const route = useRoute()
const router = useRouter()
const workspace = useWorkspaceStore()
const store = useIntegrationStore()
const suites = useTestSuitesStore()

/** 列表态页签：场景库 / 测试套件（存 store，返回列表时保持离开前的页签）。 */
const listTab = computed({
  get: () => store.listTab,
  set: (v: 'library' | 'suites') => (store.listTab = v),
})

const listTabOptions = computed(() => [
  { value: 'library', label: t('integration.tabLibrary') },
  { value: 'suites', label: t('integration.tabSuites') },
])

// 批量执行进度：事件驱动（所有入口统一从 Rust 批量引擎推送），按批次隔离。
useTauriEvent<TestBatchProgressEvent>('miyo:test-batch-progress', (payload) => {
  suites.applyProgress(payload)
})

/** 从批次明细进入单场景执行详情时记住来源批次，返回时回到批次详情。 */
const batchOriginId = ref<string | null>(null)
const batchOriginLabel = ref('')

/** 批次明细跳转单场景执行记录（回看当时的步骤结果）。 */
function openBatchRun(payload: { scenarioId: string; runId: string }): void {
  const origin = suites.activeBatchId
  if (origin) {
    batchOriginId.value = origin
    batchOriginLabel.value = t('integration.navBatch', { id: origin.slice(0, 8) })
  }
  suites.closePanel()
  void store.openScenarioRun(payload.scenarioId, payload.runId)
}

/** 编辑器返回：来自批次 → 回到批次详情；否则回场景列表。 */
function onEditorBack(): void {
  const origin = batchOriginId.value
  batchOriginId.value = null
  store.backToLibrary()
  if (origin) void suites.openBatch(origin)
}

/** 情境导航：默认「返回测试场景」；批次来源「返回批次详情」+ 祖先层级。 */
const editorBackLabel = computed(() =>
  batchOriginId.value ? t('integration.navBackBatch') : undefined,
)
const editorParentCrumbs = computed(() =>
  batchOriginId.value
    ? [{ label: t('integration.navHistory') }, { label: batchOriginLabel.value }]
    : [],
)

// ---------- 路由同步（深链 / 刷新恢复 / 返回不丢状态） ----------
/** 路由 query：project + scenario / suite / batch（同一时刻只保留一个详情层级）。 */
const LAST_DETAIL_KEY = 'miyo.integration.last-detail'
let routeRestored = false
/** 自身写入 query 时忽略回环（replace 会触发 route.query watch）。 */
let applyingQuery = false

function desiredQuery(): Record<string, string> {
  const q: Record<string, string> = {}
  if (store.activeScenarioId) {
    q.scenario = store.activeScenarioId
    if (batchOriginId.value) q.batch = batchOriginId.value
  } else if (suites.panelOpen && suites.activeBatchId) {
    // 批次详情优先于套件层级（从套件打开批次时地址栏指向批次）。
    q.batch = suites.activeBatchId
  } else if (listTab.value === 'suites' && suites.activeSuiteId) {
    q.suite = suites.activeSuiteId
  }
  if (q.scenario || q.suite || q.batch) {
    if (store.projectId) q.project = store.projectId
  }
  return q
}

/** 把当前层级写回地址栏（replace，不制造历史堆栈；深链可直接复制）。 */
async function syncRoute(): Promise<void> {
  if (!routeRestored) return
  const next = desiredQuery()
  const current = route.query as Record<string, string>
  const same =
    Object.keys(next).length === Object.keys(current).length &&
    Object.entries(next).every(([k, v]) => current[k] === v)
  // 记录「上次停留的详情层级」：应用重启 / 长时间离开后回到集成测试可恢复。
  try {
    if (next.scenario || next.suite || next.batch) {
      localStorage.setItem(LAST_DETAIL_KEY, JSON.stringify(next))
    } else {
      localStorage.removeItem(LAST_DETAIL_KEY)
    }
  } catch {
    /* 隐私模式等存储不可用：仅本次会话内生效 */
  }
  if (same) return
  applyingQuery = true
  try {
    await router.replace({ path: '/integration', query: next })
  } finally {
    applyingQuery = false
  }
}

watch(
  [
    () => store.activeScenarioId,
    () => listTab.value,
    () => suites.activeSuiteId,
    () => suites.panelOpen,
    () => suites.activeBatchId,
    () => batchOriginId.value,
  ],
  () => void nextTick(syncRoute),
)

// 外部路由变化（浏览器前进/后退、直接改地址）：按新 query 重放层级。
watch(
  () => route.query,
  async () => {
    if (!routeRestored || applyingQuery) return
    const q = route.query
    const hasDetail = !!(q.scenario || q.suite || q.batch)
    if (!hasDetail) {
      batchOriginId.value = null
      store.backToLibrary()
      suites.setActiveSuite(null)
      if (suites.panelOpen) suites.closePanel()
      return
    }
    await restoreFromRoute()
    await syncRoute()
  },
)

/** 从 URL 解释详情入口（刷新 / 直接打开 / 外部跳转）。 */
async function restoreFromRoute(): Promise<void> {
  const q = route.query
  const str = (v: unknown): string => (typeof v === 'string' ? v : '')
  const project = str(q.project)
  if (project && project !== store.projectId) await doSwitchProject(project)

  let scenarioId = str(q.scenario)
  let suiteId = str(q.suite)
  let batchId = str(q.batch)

  // 无 query 详情（应用重启 / 从其他模块回到集成测试）：恢复上次详情。
  if (!scenarioId && !suiteId && !batchId) {
    try {
      const raw = localStorage.getItem(LAST_DETAIL_KEY)
      const saved = raw ? (JSON.parse(raw) as Record<string, string>) : null
      if (saved?.scenario) scenarioId = saved.scenario
      else if (saved?.suite) suiteId = saved.suite
      else if (saved?.batch) batchId = saved.batch
      if (saved?.project && saved.project !== store.projectId) {
        await doSwitchProject(saved.project)
      }
    } catch {
      /* 损坏的本地记录忽略 */
    }
  }

  if (scenarioId) {
    if (batchId) {
      batchOriginId.value = batchId
      batchOriginLabel.value = t('integration.navBatch', { id: batchId.slice(0, 8) })
      void suites.refreshBatches()
    }
    store.openScenario(scenarioId)
  } else if (suiteId) {
    listTab.value = 'suites'
    suites.setActiveSuite(suiteId)
  } else if (batchId) {
    await suites.openBatch(batchId)
  }
  const runId = str(q.run)
  if (scenarioId && runId) store.openScenarioRun(scenarioId, runId)
}

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
  goBackFrom()
}

/** 返回钮提示（列表页回上一页；编辑器内的返回在编辑器导航栏）。 */
const backHint = computed(() => t('nav.backHint'))

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
  void store.loadFolders(!store.folders.length)
  // 深链 / 刷新：按 query 恢复详情层级（列表上下文仍由 store 持有）。
  await restoreFromRoute()
  routeRestored = true
  await syncRoute()
})

// 项目切换（含首次解析完成）→ 刷新场景库 + 业务目录；跨路由返回时已加载则静默刷新。
watch(
  () => store.projectId,
  () => {
    void store.loadLibrary(!store.libraryLoaded)
    void store.loadFolders(true)
  },
)
</script>

<template>
  <div class="it-root">
    <PageHeader
      v-if="!store.activeScenarioId"
      :title="t('integration.title')"
      :back-title="backHint"
      @back="goBack"
    >
      <template #actions>
        <SegmentedControl
          v-if="!store.activeScenarioId"
          :model-value="listTab"
          :options="listTabOptions"
          size="sm"
          @update:model-value="(v: string) => (listTab = v as 'library' | 'suites')"
        />
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
        :back-label="editorBackLabel"
        :parent-crumbs="editorParentCrumbs"
        @back="onEditorBack"
      />
      <TestSuitePanel v-else-if="listTab === 'suites'" />
      <ScenarioLibrary v-else />
    </main>

    <!-- 批量执行配置 / 进度报告：全局挂在视图层，场景库与套件页共用 -->
    <BatchRunDialog />
    <BatchReportPanel @open-run="openBatchRun" />

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
