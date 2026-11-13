/**
 * integration：集成测试视图的应用级状态（路由级单例）。
 *
 * 存在意义：`/projects` ↔ `/integration` 来回切换时视图组件重挂载，
 * 没有 store 快照就会闪骨架屏 / 丢失「上次打开的场景」（同 dashboard
 * store 的防闪模式）。
 *
 * 职责边界：
 * - 项目上下文：独立于 workspace store 的 project（集成测试有自己的项目
 *   选择器），首次进入默认跟随工作区激活项目；
 * - 场景库缓存：listScenarioSummaries 一次 IPC 的结果缓存 + 静默刷新；
 * - 编辑目标：activeScenarioId（null = 列表页）。
 * 执行态（运行 / 进度 / 结果）不在本 store——归 WorkflowEditor 与
 * useScenarioRun，随编辑器实例存在。
 */
import { ref, type Ref } from 'vue'
import { defineStore } from 'pinia'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useWorkspaceStore } from '@/stores/workspace'
import type { Project, ScenarioLibraryItem } from '@/types/miyoApi.d'

export const useIntegrationStore = defineStore('integration', () => {
  const api = useMiyoApi()
  const workspace = useWorkspaceStore()

  // ---------- 项目上下文 ----------
  /** 当前项目（null = 未初始化；初始化时跟随工作区激活项目）。 */
  const projectId = ref<string | null>(null)
  /** 项目选择器数据（全量项目，进入视图时加载一次 + 切换时静默刷新）。 */
  const projects = ref<Project[]>([])

  /** 初始化项目上下文：默认跟随工作区激活项目（store 生命周期内仅一次）。 */
  async function ensureProjectContext(): Promise<string | null> {
    if (projectId.value && projects.value.length) return projectId.value
    try {
      projects.value = await api.getProjects()
    } catch {
      projects.value = []
    }
    if (!projectId.value) {
      projectId.value =
        workspace.project?.id ?? projects.value[0]?.id ?? null
    }
    return projectId.value
  }

  /** 切换项目（选择器；切换后库列表随 watch 刷新）。 */
  function setProject(id: string): void {
    if (projectId.value === id) return
    projectId.value = id
    activeScenarioId.value = null
  }

  // ---------- 场景库缓存 ----------
  // `as Ref<T>`：UnwrapRef 对该深嵌套类型会触发 TS2589 深度实例化爆炸（运行时行为不变）
  const libraryItems = ref<ScenarioLibraryItem[]>([]) as Ref<ScenarioLibraryItem[]>
  const libraryLoaded = ref(false)
  const libraryLoading = ref(false)

  /**
   * 加载场景库。`silent` = 后台刷新（已有缓存时不清空，列表不闪骨架）。
   */
  async function loadLibrary(silent = false): Promise<void> {
    const pid = projectId.value
    if (!pid) {
      libraryItems.value = []
      libraryLoaded.value = true
      return
    }
    if (!silent) libraryLoading.value = true
    try {
      const items = await api.listScenarioSummaries(pid)
      if (projectId.value === pid) {
        libraryItems.value = items
        libraryLoaded.value = true
      }
    } catch {
      if (projectId.value === pid && !libraryLoaded.value) {
        libraryItems.value = []
        libraryLoaded.value = true
      }
    } finally {
      libraryLoading.value = false
    }
  }

  // ---------- 编辑目标 ----------
  /** 当前编辑的场景 id（null = 列表页）。 */
  const activeScenarioId = ref<string | null>(null)
  /** 待直达的运行历史 id（列表「执行历史」入口 → 编辑器打开结果视图）。 */
  const pendingRunId = ref<string | null>(null)

  function openScenario(id: string): void {
    activeScenarioId.value = id
  }

  /** 从列表直达某次运行的结果视图。 */
  function openScenarioRun(id: string, runId: string): void {
    pendingRunId.value = runId
    activeScenarioId.value = id
  }

  /** 取走待直达 runId（编辑器消费一次）。 */
  function takePendingRun(): string | null {
    const v = pendingRunId.value
    pendingRunId.value = null
    return v
  }

  function backToLibrary(): void {
    activeScenarioId.value = null
  }

  /** 场景删除 / 移出后清编辑态，避免编辑器悬挂在已删除实体上。 */
  function forgetScenario(id: string): void {
    if (activeScenarioId.value === id) activeScenarioId.value = null
    libraryItems.value = libraryItems.value.filter((s) => s.id !== id)
  }

  return {
    projectId,
    projects,
    libraryItems,
    libraryLoaded,
    libraryLoading,
    activeScenarioId,
    pendingRunId,
    openScenarioRun,
    takePendingRun,
    ensureProjectContext,
    setProject,
    loadLibrary,
    openScenario,
    backToLibrary,
    forgetScenario,
  }
})
