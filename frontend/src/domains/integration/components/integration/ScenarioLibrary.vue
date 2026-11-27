<script setup lang="ts">
/**
 * ScenarioLibrary：集成测试的场景库（列表页）。
 *
 * 布局：左侧业务目录树（多级文件夹）+ 右侧场景列表（列表 / 卡片视图）。
 * - 数据走 integration store 缓存（list_scenario_summaries 单次 IPC）+
 *   list_scenario_folders；目录/场景变更后统一 refreshLibrary；
 * - 搜索跨目录（名称 / 描述 / 标签）；状态 / 分类 / 收藏 / 目录筛选；
 * - 排序：默认（收藏置顶 + 编排序）/ 名称 / 最近执行 / 更新时间；
 * - 多选 + 批量工具栏：批量执行 / 移动 / 删除 / 打标签 / 收藏；
 * - 拖拽：场景拖入目录（含批量）、目录拖到其他目录 / 根级（防环）；
 * - 目录删除三策略：仅空目录 / 场景移到未分类 / 连场景删除（二次确认）。
 */
import { computed, nextTick, onBeforeUnmount, provide, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useTestSuitesStore } from '@/domains/integration/stores/testSuites'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useTauriEvent } from '@/composables/useTauriEvent'
import { errorText } from '@/utils/errorText'
import { buildScenarioExport, parseScenarioImport } from '@/utils/scenarioIO'
import { formatDateTime } from '@/utils/dateTime'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import RunHistoryPanel from '@/domains/integration/components/integration/RunHistoryPanel.vue'
import ScenarioFolderTree, {
  type ScenarioDragState,
} from '@/domains/integration/components/integration/ScenarioFolderTree.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Modal from '@/components/ui/Modal.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import { formatDurationShort } from '@/utils/format'
import {
  buildFolderTree,
  collectSubtreeIds,
  folderPath as folderPathOf,
  lastRunResult,
  uncategorizedCount,
} from '@/domains/integration/utils/folderTree'
import type {
  Endpoint,
  MatrixRunOutcome,
  Scenario,
  ScenarioFolder,
  ScenarioFolderDeleteMode,
  ScenarioLibraryItem,
  ScenarioRunSummary,
  ScenarioStep,
} from '@/types/miyoApi.d'

const t = useLocaleStore().t
const store = useIntegrationStore()
const suites = useTestSuitesStore()
const workspace = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const router = useRouter()

// ---------- 过滤 / 排序 ----------
const view = store.libraryView
const search = computed({
  get: () => view.search,
  set: (v: string) => (view.search = v),
})
const categoryFilter = computed({
  get: () => view.categoryFilter,
  set: (v: string | null) => (view.categoryFilter = v),
})
const statusFilter = computed({
  get: () => view.statusFilter,
  set: (v: 'all' | 'ok' | 'fail' | 'none') => (view.statusFilter = v),
})
const sortBy = computed({
  get: () => view.sortBy,
  set: (v: 'default' | 'name' | 'recent' | 'updated') => (view.sortBy = v),
})
const folderFilter = computed({
  get: () => view.folderFilter,
  set: (v: string | null) => (view.folderFilter = v),
})
const favoritesOnly = computed({
  get: () => view.favoritesOnly,
  set: (v: boolean) => (view.favoritesOnly = v),
})
const viewMode = computed({
  get: () => view.viewMode,
  set: (v: 'list' | 'card') => (view.viewMode = v),
})

/** 全部分类（去重、保序）。 */
const categories = computed(() => {
  const set: string[] = []
  for (const s of store.libraryItems) {
    const c = s.category.trim()
    if (c && !set.includes(c)) set.push(c)
  }
  return set
})
const hasCategory = computed(() => store.libraryItems.some((s) => s.category.trim()))

const folderIds = computed(() => new Set(store.folders.map((f) => f.id)))

/** 当前目录筛选的子树（选中目录时含后代目录）。 */
const folderScope = computed<Set<string> | null>(() => {
  const id = folderFilter.value
  if (!id || id === '') return null
  return collectSubtreeIds(store.folders, id)
})

function inFolderScope(item: ScenarioLibraryItem): boolean {
  const fid = item.folder_id ?? null
  if (folderFilter.value === null) return true
  if (folderFilter.value === '') return !fid || !folderIds.value.has(fid)
  if (!fid || !folderIds.value.has(fid)) return false
  return folderScope.value?.has(fid) ?? false
}

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  const list = store.libraryItems.filter((s) => {
    if (!inFolderScope(s)) return false
    if (favoritesOnly.value && !s.favorite) return false
    if (categoryFilter.value === '' && s.category.trim()) return false
    if (categoryFilter.value && s.category.trim() !== categoryFilter.value) return false
    if (statusFilter.value !== 'all') {
      const res = lastRunResult(s)
      if (statusFilter.value === 'ok' && res !== 'ok') return false
      if (statusFilter.value === 'fail' && res !== 'fail') return false
      if (statusFilter.value === 'none' && res !== null) return false
    }
    if (!q) return true
    const folderName = folderPathOf(store.folders, s.folder_id ?? null).toLowerCase()
    return (
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q) ||
      folderName.includes(q) ||
      (s.tags ?? []).some((tag) => tag.toLowerCase().includes(q))
    )
  })
  if (sortBy.value === 'name') {
    return [...list].sort((a, b) => a.name.localeCompare(b.name))
  }
  if (sortBy.value === 'recent') {
    return [...list].sort((a, b) => {
      const ta = a.last_run ? Date.parse(a.last_run.finished_at) : 0
      const tb = b.last_run ? Date.parse(b.last_run.finished_at) : 0
      return tb - ta
    })
  }
  if (sortBy.value === 'updated') {
    return [...list].sort((a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at))
  }
  // 默认：收藏置顶（后端已按 favorite/sort_order 排序，这里保持稳定序）。
  return [...list].sort((a, b) => Number(!!b.favorite) - Number(!!a.favorite))
})

/** 目录树（场景数 / 执行状态汇总）。 */
const treeNodes = computed(() => buildFolderTree(store.folders, store.libraryItems))
const uncategorizedTotal = computed(() => uncategorizedCount(store.folders, store.libraryItems))

const expanded = computed(() => view.expandedFolders)
function toggleFolder(id: string): void {
  const set = new Set(view.expandedFolders)
  if (set.has(id)) set.delete(id)
  else set.add(id)
  view.expandedFolders = [...set]
}

const canMoveFolderFn = (folderId: string, parentId: string | null): boolean => {
  if (parentId === null) return true
  const ids = collectSubtreeIds(store.folders, folderId)
  return !ids.has(parentId)
}

// ---------- 多选 ----------
const selectedIds = computed(() => new Set(store.selectedScenarioIds))
const selectedCount = computed(() => store.selectedScenarioIds.length)
const allSelected = computed(
  () => filtered.value.length > 0 && filtered.value.every((s) => selectedIds.value.has(s.id)),
)

function toggleSelected(id: string): void {
  const ids = new Set(store.selectedScenarioIds)
  if (ids.has(id)) ids.delete(id)
  else ids.add(id)
  store.selectScenarios([...ids])
}

function toggleSelectAll(): void {
  if (allSelected.value) {
    store.clearSelection()
    return
  }
  store.selectScenarios(filtered.value.map((s) => s.id))
}

/** 过滤条件变化后清空选择（避免批量操作作用于不可见场景）。 */
watch(
  [folderFilter, search, statusFilter, favoritesOnly, categoryFilter],
  () => store.clearSelection(),
)

// ---------- 最近执行状态 ----------
type RunTone = 'ok' | 'fail' | 'cancel' | null

const envNameById = computed(() => {
  const m = new Map<string, string>()
  for (const e of workspace.environments) m.set(e.id, e.name)
  return m
})

function envBadgeName(item: ScenarioLibraryItem): string | null {
  const id = item.last_run?.environment_id
  if (!id) return null
  return envNameById.value.get(id) ?? id.slice(0, 8)
}

function successRate(item: ScenarioLibraryItem): number | null {
  const r = item.last_run
  if (!r) return null
  const total = r.ok_count + r.fail_count + r.skipped_count
  if (total <= 0) return null
  return Math.round((r.ok_count / total) * 100)
}

function runTone(item: ScenarioLibraryItem): RunTone {
  return lastRunResult(item)
}

function runSummaryText(r: ScenarioRunSummary | null | undefined): string {
  if (!r) return t('integration.libLastRunNone')
  const parts = [
    t('scenario.summaryOk', { n: r.ok_count }),
    r.fail_count > 0 ? t('scenario.summaryFail', { n: r.fail_count }) : null,
    r.skipped_count > 0 ? t('scenario.summarySkip', { n: r.skipped_count }) : null,
  ].filter(Boolean)
  return `${t('integration.libLastRun', { time: formatDateTime(r.finished_at) })} · ${parts.join(' / ')}`
}

function folderNameOf(item: ScenarioLibraryItem): string {
  return folderPathOf(store.folders, item.folder_id ?? null)
}

// ---------- AI 创建入口 ----------
function goAiCreate(): void {
  void router.push('/ai-test')
}

// ---------- 滚动位置记忆 ----------
const listEl = ref<HTMLElement | null>(null)
function saveScroll(): void {
  view.scrollTop = listEl.value?.scrollTop ?? 0
  view.projectIdAtScroll = store.projectId
}
/** 滚动容器随视图切换（列表 ↔ 卡片）与空库 → 有数据重建，监听随之迁移。 */
let scrollBound: HTMLElement | null = null
watch(
  listEl,
  async (el) => {
    if (scrollBound) scrollBound.removeEventListener('scroll', saveScroll)
    scrollBound = el
    if (!el) return
    el.addEventListener('scroll', saveScroll, { passive: true })
    const top = view.scrollTop
    if (top > 0 && view.projectIdAtScroll === store.projectId) {
      await nextTick()
      el.scrollTop = top
    }
  },
  { flush: 'post', immediate: true },
)
onBeforeUnmount(() => {
  if (scrollBound) scrollBound.removeEventListener('scroll', saveScroll)
  scrollBound = null
})

// ---------- 执行历史面板 ----------
const historyPanelOpen = ref(false)
function onHistoryOpenRun(payload: { scenarioId: string; runId: string }): void {
  store.openScenarioRun(payload.scenarioId, payload.runId)
}

// ---------- 场景 CRUD ----------
/** 列表项 → 可保存的场景（剥离 last_run / 统计字段）。 */
function toScenario(item: ScenarioLibraryItem): Scenario {
  return {
    id: item.id,
    project_id: item.project_id,
    name: item.name,
    description: item.description,
    category: item.category,
    folder_id: item.folder_id ?? null,
    tags: [...(item.tags ?? [])],
    favorite: !!item.favorite,
    sort_order: item.sort_order,
    steps: JSON.parse(JSON.stringify(item.steps ?? [])) as ScenarioStep[],
    script: item.script ?? null,
    setup: JSON.parse(JSON.stringify(item.setup ?? [])) as ScenarioStep[],
    teardown: JSON.parse(JSON.stringify(item.teardown ?? [])) as ScenarioStep[],
    teardown_enabled: item.teardown_enabled ?? true,
    data_sets: JSON.parse(JSON.stringify(item.data_sets ?? [])) as Scenario['data_sets'],
    created_at: item.created_at,
    updated_at: item.updated_at,
  }
}

/** 新建场景落到当前目录（选中具体目录时）。 */
function currentFolderId(): string | null {
  const id = folderFilter.value
  return id && id !== '' ? id : null
}

async function onCreate(): Promise<void> {
  const now = new Date().toISOString()
  const scenario: Scenario = {
    id: crypto.randomUUID(),
    project_id: store.projectId ?? '',
    name: t('integration.newName'),
    description: '',
    category: categoryFilter.value || '',
    folder_id: currentFolderId(),
    tags: [],
    favorite: false,
    sort_order: store.libraryItems.length,
    steps: [],
    script: null,
    created_at: now,
    updated_at: now,
  }
  try {
    const saved = await api.saveScenario(scenario)
    await store.refreshLibrary()
    store.openScenario(saved.id)
  } catch (err) {
    toast.error(t('integration.saveFail'), { message: errorText(err) })
  }
}

async function onDuplicate(s: ScenarioLibraryItem): Promise<void> {
  const now = new Date().toISOString()
  const copy: Scenario = {
    ...toScenario(s),
    id: crypto.randomUUID(),
    name: t('scenario.copyName', { name: s.name }),
    sort_order: store.libraryItems.length,
    created_at: now,
    updated_at: now,
  }
  try {
    const saved = await api.saveScenario(copy)
    toast.success(t('integration.duplicated', { name: saved.name }))
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.saveFail'), { message: errorText(err) })
  }
}

async function onDelete(id: string): Promise<void> {
  try {
    await api.deleteScenario(id)
    store.forgetScenario(id)
  } catch (err) {
    toast.error(t('integration.saveFail'), { message: errorText(err) })
  }
}

async function toggleFavorite(s: ScenarioLibraryItem): Promise<void> {
  try {
    await api.setScenariosFavorite([s.id], !s.favorite)
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.saveFail'), { message: errorText(err) })
  }
}

// ---------- 重命名场景 ----------
const renameOpen = ref(false)
const renameTarget = ref<ScenarioLibraryItem | null>(null)
const renameName = ref('')

function openRename(s: ScenarioLibraryItem): void {
  renameTarget.value = s
  renameName.value = s.name
  renameOpen.value = true
}

async function confirmRename(): Promise<void> {
  const target = renameTarget.value
  const name = renameName.value.trim()
  if (!target || !name) return
  try {
    await api.saveScenario({ ...toScenario(target), name, updated_at: new Date().toISOString() })
    renameOpen.value = false
    toast.success(t('integration.libRenamed'))
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.saveFail'), { message: errorText(err) })
  }
}

// ---------- 目录 CRUD ----------
const folderModalOpen = ref(false)
const folderModalMode = ref<'create' | 'rename'>('create')
const folderNameInput = ref('')
const folderParentId = ref<string | null>(null)
const folderRenameTarget = ref<ScenarioFolder | null>(null)

function openCreateFolder(parentId: string | null): void {
  folderModalMode.value = 'create'
  folderParentId.value = parentId
  folderRenameTarget.value = null
  folderNameInput.value = ''
  folderModalOpen.value = true
}

function openRenameFolder(folder: ScenarioFolder): void {
  folderModalMode.value = 'rename'
  folderRenameTarget.value = folder
  folderNameInput.value = folder.name
  folderModalOpen.value = true
}

/** 同级下一个排序号（新建目录追加到末尾）。 */
function nextFolderSortOrder(parentId: string | null): number {
  const siblings = store.folders.filter((f) => (f.parent_id ?? null) === parentId)
  return siblings.reduce((max, f) => Math.max(max, f.sort_order + 1), 0)
}

async function confirmFolderModal(): Promise<void> {
  const name = folderNameInput.value.trim()
  if (!name) return
  const now = new Date().toISOString()
  try {
    if (folderModalMode.value === 'create') {
      const folder: ScenarioFolder = {
        id: crypto.randomUUID(),
        project_id: store.projectId ?? '',
        parent_id: folderParentId.value,
        name,
        sort_order: nextFolderSortOrder(folderParentId.value),
        created_at: now,
        updated_at: now,
      }
      await api.saveScenarioFolder(folder)
      // 新建后展开父级，便于看到新目录。
      if (folderParentId.value && !view.expandedFolders.includes(folderParentId.value)) {
        view.expandedFolders = [...view.expandedFolders, folderParentId.value]
      }
      toast.success(t('integration.folderCreated', { name }))
    } else {
      const target = folderRenameTarget.value
      if (!target) return
      await api.saveScenarioFolder({ ...target, name, updated_at: now })
      toast.success(t('integration.folderRenamed'))
    }
    folderModalOpen.value = false
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.folderMoveFail'), { message: errorText(err) })
  }
}

// ---------- 目录删除（三策略） ----------
const folderDeleteOpen = ref(false)
const folderDeleteTarget = ref<ScenarioFolder | null>(null)
const folderDeleteMode = ref<ScenarioFolderDeleteMode>('keep')
const folderDeleteCount = ref(0)
const cascadeConfirmed = ref(false)

const folderDeleteSubtreeCount = computed(() => {
  const target = folderDeleteTarget.value
  if (!target) return 0
  const ids = collectSubtreeIds(store.folders, target.id)
  return store.libraryItems.filter((s) => s.folder_id && ids.has(s.folder_id)).length
})

function openDeleteFolder(folder: ScenarioFolder): void {
  folderDeleteTarget.value = folder
  folderDeleteCount.value = 0
  folderDeleteMode.value = 'keep'
  cascadeConfirmed.value = false
  folderDeleteOpen.value = true
}

watch(folderDeleteOpen, (open) => {
  if (!open || !folderDeleteTarget.value) return
  folderDeleteCount.value = folderDeleteSubtreeCount.value
  if (folderDeleteCount.value === 0) folderDeleteMode.value = 'empty'
})

async function confirmDeleteFolder(): Promise<void> {
  const target = folderDeleteTarget.value
  if (!target) return
  if (folderDeleteMode.value === 'cascade' && !cascadeConfirmed.value) return
  // 删除前先记下子树：选中的是后代目录时也要回到「全部场景」（否则筛选悬空）。
  const deletedIds = collectSubtreeIds(store.folders, target.id)
  try {
    const outcome = await api.deleteScenarioFolder(target.id, folderDeleteMode.value)
    folderDeleteOpen.value = false
    const selected = folderFilter.value
    if (selected && selected !== '' && deletedIds.has(selected)) folderFilter.value = null
    if (outcome.deletedScenarios > 0) {
      toast.success(
        t('integration.folderDeletedScenarios', {
          n: outcome.deletedFolders,
          scenarios: outcome.deletedScenarios,
        }),
      )
    } else {
      toast.success(
        t('integration.folderDeleted', {
          n: outcome.deletedFolders,
          moved: outcome.movedScenarios,
        }),
      )
    }
    store.clearSelection()
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.folderMoveFail'), { message: errorText(err) })
  }
}

async function moveFolderTo(id: string, parentId: string | null): Promise<void> {
  try {
    await api.moveScenarioFolder(id, parentId, null)
    if (parentId && !view.expandedFolders.includes(parentId)) {
      view.expandedFolders = [...view.expandedFolders, parentId]
    }
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.folderMoveFail'), { message: errorText(err) })
  }
}

// ---------- 拖拽（场景 → 目录） ----------
const dragState = reactive<ScenarioDragState>({ kind: null, ids: [] })
provide('scenarioDrag', dragState)

function onScenarioDragStart(e: DragEvent, s: ScenarioLibraryItem): void {
  const ids = selectedIds.value.has(s.id) ? store.selectedScenarioIds : [s.id]
  dragState.kind = 'scenario'
  dragState.ids = ids
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('application/x-miyo-scenario', ids.join(','))
  }
}

function onScenarioDragEnd(): void {
  dragState.kind = null
  dragState.ids = []
}

async function onDropScenarios(folderId: string | null): Promise<void> {
  const ids = dragState.ids.length ? [...dragState.ids] : []
  dragState.kind = null
  dragState.ids = []
  if (!ids.length) return
  try {
    const moved = await api.moveScenarios(ids, folderId)
    toast.success(t('integration.folderMoved', { n: moved }))
    store.clearSelection()
    if (folderId && !view.expandedFolders.includes(folderId)) {
      view.expandedFolders = [...view.expandedFolders, folderId]
    }
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.folderMoveFail'), { message: errorText(err) })
  }
}

// ---------- 批量操作 ----------
const batchBusy = ref(false)
const batchDeleteOpen = ref(false)
const batchTagOpen = ref(false)
const batchTagInput = ref('')
const batchMoveOpen = ref(false)
const batchMoveTarget = ref<string | null>(null)

/** 批量移动目录下拉（全量目录 + 未分类）。 */
const folderOptions = computed(() => [
  { value: '', label: t('integration.libNoFolder') },
  ...store.folders.map((f) => ({
    value: f.id,
    label: folderPathOf(store.folders, f.id) || f.name,
  })),
])

function batchDelete(): void {
  batchDeleteOpen.value = false
  const ids = [...store.selectedScenarioIds]
  if (!ids.length) return
  void (async () => {
    batchBusy.value = true
    try {
      const n = await api.deleteScenarios(ids)
      toast.success(t('integration.libBatchDeleted', { n }))
      store.clearSelection()
      await store.refreshLibrary()
    } catch (err) {
      toast.error(t('integration.saveFail'), { message: errorText(err) })
    } finally {
      batchBusy.value = false
    }
  })()
}

function confirmBatchMove(): void {
  const ids = [...store.selectedScenarioIds]
  if (!ids.length) return
  batchMoveOpen.value = false
  const target = batchMoveTarget.value || null
  void (async () => {
    batchBusy.value = true
    try {
      const n = await api.moveScenarios(ids, target)
      toast.success(t('integration.folderMoved', { n }))
      store.clearSelection()
      if (target && !view.expandedFolders.includes(target)) {
        view.expandedFolders = [...view.expandedFolders, target]
      }
      await store.refreshLibrary()
    } catch (err) {
      toast.error(t('integration.folderMoveFail'), { message: errorText(err) })
    } finally {
      batchBusy.value = false
    }
  })()
}

function confirmBatchTag(): void {
  const ids = [...store.selectedScenarioIds]
  const tags = batchTagInput.value
    .split(/[,，]/)
    .map((s) => s.trim())
    .filter(Boolean)
  if (!ids.length || !tags.length) return
  batchTagOpen.value = false
  batchTagInput.value = ''
  void (async () => {
    batchBusy.value = true
    try {
      const n = await api.addScenarioTags(ids, tags)
      toast.success(t('integration.libBatchTagDone', { n }))
      await store.refreshLibrary()
    } catch (err) {
      toast.error(t('integration.saveFail'), { message: errorText(err) })
    } finally {
      batchBusy.value = false
    }
  })()
}

function batchFavorite(favorite: boolean): void {
  const ids = [...store.selectedScenarioIds]
  if (!ids.length) return
  void (async () => {
    batchBusy.value = true
    try {
      await api.setScenariosFavorite(ids, favorite)
      if (favorite) toast.success(t('integration.libBatchFavorited', { n: ids.length }))
      await store.refreshLibrary()
    } catch (err) {
      toast.error(t('integration.saveFail'), { message: errorText(err) })
    } finally {
      batchBusy.value = false
    }
  })()
}

/**
 * 批量执行（勾选场景入口）：统一交给后端批次引擎（串行 / 并行、失败策略、
 * 暂停 / 取消 / 恢复、批次持久化都在 Rust）。打开运行配置面板后由用户确认。
 */
function batchRun(): void {
  // 无启用步骤的场景跳过（空场景不该让整批停下来）。
  const targets = store.libraryItems.filter(
    (s) => selectedIds.value.has(s.id) && (s.steps ?? []).some((st) => st.enabled),
  )
  if (!targets.length) {
    toast.warning(t('scenario.noEnabledSteps'))
    return
  }
  suites.openRunDialog({ scenario_ids: targets.map((s) => s.id) })
}

/** 文件夹批量执行入口：目录（含子目录）内全部场景走同一批次表。 */
function runFolder(folderId: string): void {
  suites.openRunDialog({ folder_id: folderId })
}

// ---------- 单场景直接运行（列表快捷执行） ----------
/** 步骤物化（引用式 → 内联请求；与矩阵运行同一套组装）。 */
function materializeSteps(steps: ScenarioStep[] | undefined): unknown[] {
  return (steps ?? [])
    .filter((st) => st.enabled)
    .map((st) => {
      if (st.request) {
        const endpoint: Endpoint = {
          id: st.request.url,
          project_id: '',
          folder_id: null,
          name: st.name || 'step',
          method: st.request.method as Endpoint['method'],
          path: st.request.url,
          description: '',
          status: 'developing',
          sort_order: 0,
          request: st.request.spec,
          created_at: '',
          updated_at: '',
        }
        return {
          endpoint,
          url: st.request.url,
          enabled: true,
          extract: st.extract ?? [],
          assertions: st.assertions ?? [],
          condition: st.condition ?? null,
          loop: st.loop ?? null,
          wait_ms: st.wait_ms ?? null,
          parallel: st.parallel ?? false,
          retry: st.retry ?? null,
        }
      }
      const ep = workspace.endpoints.find((e) => e.id === st.endpoint_id)
      if (!ep) return null
      return {
        endpoint: ep,
        url: `{{base_url}}${ep.path.startsWith('/') ? ep.path : `/${ep.path}`}`,
        enabled: true,
        extract: st.extract ?? [],
        assertions: st.assertions ?? [],
        condition: st.condition ?? null,
        loop: st.loop ?? null,
        wait_ms: st.wait_ms ?? null,
        parallel: st.parallel ?? false,
        retry: st.retry ?? null,
      }
    })
    .filter((x): x is NonNullable<typeof x> => x !== null)
}

/** 运行单个场景（落运行历史；返回是否全部通过）。 */
async function runScenarioNow(item: ScenarioLibraryItem): Promise<boolean> {
  const s = toScenario(item)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const steps = materializeSteps(s.steps) as any[]
  if (!steps.length) {
    toast.warning(t('scenario.noEnabledSteps'))
    return false
  }
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const setup = materializeSteps(s.setup) as any[]
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const teardown = materializeSteps(s.teardown) as any[]
    const result = await api.runScenario({
      steps,
      setup,
      teardown,
      teardownEnabled: s.teardown_enabled ?? true,
      stopOnFailure: false,
      runId: crypto.randomUUID(),
      scenarioId: s.id,
    })
    return result.results.every((r) => r.ok || r.cancelled)
  } catch (err) {
    toast.error(t('scenario.runFail'), { message: errorText(err) })
    return false
  }
}

async function onRunOne(item: ScenarioLibraryItem): Promise<void> {
  const passed = await runScenarioNow(item)
  toast[passed ? 'success' : 'warning'](
    t('integration.libRunOneDone', { name: item.name, ok: passed ? 1 : 0, fail: passed ? 0 : 1 }),
  )
  await store.refreshLibrary()
}

// ---------- 矩阵运行 ----------
const matrixProgress = ref<{
  groupId: string
  done: number
  total: number
  label: string
  ok: boolean
} | null>(null)
const matrixResult = ref<MatrixRunOutcome | null>(null)
const matrixResultOpen = ref(false)
const runningMatrixScenarioId = ref<string | null>(null)
useTauriEvent<{
  groupId: string
  index: number
  total: number
  label: string
  ok: boolean
  runId: string
}>('miyo:matrix-progress', (p) => {
  matrixProgress.value = {
    groupId: p.groupId,
    done: p.index,
    total: p.total,
    label: p.label,
    ok: p.ok,
  }
})

async function buildMatrixRows(
  s: Scenario,
  mode: 'data' | 'environment',
): Promise<{ key: string; label: string; environmentId?: string | null; vars?: Record<string, string> | null }[]> {
  if (mode === 'data') {
    const rows: { key: string; label: string; vars: Record<string, string> }[] = []
    for (const ds of s.data_sets ?? []) {
      const parsed: Record<string, string>[] = []
      if (ds.source.kind === 'inline') {
        for (const r of ds.source.rows) {
          parsed.push(
            Object.fromEntries(
              Object.entries(r).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)]),
            ),
          )
        }
      } else if (ds.source.kind === 'json') {
        const arr = JSON.parse(ds.source.text) as Record<string, unknown>[]
        for (const r of arr) {
          parsed.push(
            Object.fromEntries(
              Object.entries(r).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)]),
            ),
          )
        }
      } else if (ds.source.kind === 'csv') {
        const lines = ds.source.text.split('\n').filter((l) => l.trim())
        if (lines.length < 2) continue
        const header = lines[0]!.split(',').map((h) => h.trim())
        for (const line of lines.slice(1)) {
          const cells = line.split(',')
          parsed.push(Object.fromEntries(header.map((h, i) => [h, (cells[i] ?? '').trim()])))
        }
      }
      if (!parsed.length) {
        toast.warning(t('integration.matrixEmptySet', { name: ds.name }))
        continue
      }
      parsed.forEach((vars, i) => {
        rows.push({
          key: `${ds.id}-${i}`,
          label: parsed.length > 1 ? `${ds.name}#${i + 1}` : ds.name,
          vars,
        })
      })
    }
    return rows
  }
  const envs = await api.listEnvironments(s.project_id)
  return envs.map((e) => ({ key: e.id, label: e.name, environmentId: e.id }))
}

async function runMatrix(item: ScenarioLibraryItem, mode: 'data' | 'environment'): Promise<void> {
  const s = toScenario(item)
  runningMatrixScenarioId.value = s.id
  matrixScenarioId.value = s.id
  try {
    const rows = await buildMatrixRows(s, mode)
    if (!rows.length) {
      toast.warning(
        mode === 'data' ? t('integration.matrixNoData') : t('integration.matrixNoEnv'),
      )
      return
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const steps = materializeSteps(s.steps) as any[]
    if (!steps.length) {
      toast.warning(t('scenario.noEnabledSteps'))
      return
    }
    const outcome = await api.runScenarioMatrix({
      steps,
      setup: [],
      teardown: [],
      teardownEnabled: s.teardown_enabled ?? true,
      rows,
      stopOnFailure: false,
      groupId: crypto.randomUUID(),
      scenarioId: s.id,
    })
    const failed = outcome.total - outcome.okRows
    matrixProgress.value = null
    runningMatrixScenarioId.value = null
    matrixResult.value = outcome
    matrixResultOpen.value = true
    toast.success(
      t('integration.matrixDone', {
        total: outcome.total,
        ok: outcome.okRows,
        fail: failed,
      }),
      { duration: 8000 },
    )
    await store.refreshLibrary()
  } catch (err) {
    toast.error(t('integration.matrixFail'), { message: errorText(err) })
  }
}

function openMatrixRow(i: number): void {
  const row = matrixResult.value?.rows[i]
  if (!row) return
  const sc = matrixScenarioId.value
  if (sc) store.openScenarioRun(sc, row.runId)
  matrixResultOpen.value = false
}
const matrixScenarioId = ref<string | null>(null)

// ---------- 导出 / 导入 ----------
async function onExport(item: ScenarioLibraryItem): Promise<void> {
  const s = toScenario(item)
  try {
    const dir = await openDialog({ directory: true, title: t('integration.export') })
    if (!dir || Array.isArray(dir)) return
    const base = s.name.replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await api.joinPath(dir, `${base}.miyo-scenario.json`)
    const { json, redactedFields } = buildScenarioExport(
      s,
      (id) => workspace.endpoints.find((e) => e.id === id),
    )
    await api.writeTextFile(path, json)
    if (redactedFields.length) {
      toast.info(t('integration.exportRedacted'), {
        message: redactedFields.slice(0, 10).join('；'),
        duration: 8000,
      })
    }
    toast.success(t('integration.exported', { path }))
  } catch (err) {
    toast.error(t('integration.exportFail'), { message: errorText(err) })
  }
}

async function onImport(): Promise<void> {
  try {
    const file = await openDialog({
      multiple: false,
      title: t('integration.importFileTitle'),
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!file || Array.isArray(file)) return
    const text = await api.readTextFile(file)
    const parsed = parseScenarioImport(text)
    const now = new Date().toISOString()
    const scenario: Scenario = {
      id: crypto.randomUUID(),
      project_id: store.projectId ?? '',
      name: parsed.name,
      description: parsed.description,
      category: parsed.category,
      folder_id: currentFolderId(),
      tags: [],
      favorite: false,
      sort_order: store.libraryItems.length,
      steps: parsed.steps,
      script: parsed.script,
      created_at: now,
      updated_at: now,
    }
    const saved = await api.saveScenario(scenario)
    toast.success(t('integration.imported', { name: saved.name }))
    await store.refreshLibrary()
  } catch (err) {
    const raw = errorText(err)
    toast.error(
      raw === 'invalid-json' || raw === 'invalid-shape'
        ? t('integration.importInvalid')
        : t('integration.importFail'),
      { message: raw },
    )
  }
}
</script>
<template>
  <div class="lib-root">
    <!-- 左侧：业务目录树（多级文件夹） -->
    <ScenarioFolderTree
      :nodes="treeNodes"
      :selected-id="folderFilter"
      :expanded="expanded"
      :total-count="store.libraryItems.length"
      :uncategorized="uncategorizedTotal"
      :filtered-count="filtered.length"
      :can-move-folder="canMoveFolderFn"
      @select="(id) => (folderFilter = id)"
      @toggle="toggleFolder"
      @create="openCreateFolder"
      @rename="openRenameFolder"
      @remove="openDeleteFolder"
      @run="runFolder"
      @move-folder="moveFolderTo"
      @drop-scenarios="onDropScenarios"
    />

    <!-- 右侧：工具栏 + 场景列表 -->
    <div class="lib-content">
      <div class="lib-toolbar">
        <SearchInput v-model="search" class="lib-search" :placeholder="t('integration.libSearchPh')" />
        <IconButton name="upload" :size="14" :title="t('integration.libImport')" @click="onImport" />
        <IconButton
          name="history"
          :size="14"
          :title="t('integration.histProjectTitle')"
          @click="historyPanelOpen = true"
        />
        <button
          type="button"
          class="miyo-btn miyo-btn-sm"
          :class="{ 'lib-toggle-on': favoritesOnly }"
          :title="t('integration.libFavoritesOnly')"
          @click="favoritesOnly = !favoritesOnly"
        >
          <Icon name="star" :size="13" />
        </button>
        <CustomSelect
          class="lib-sort"
          :model-value="String(sortBy)"
          :options="[
            { value: 'default', label: t('integration.libSortDefault') },
            { value: 'name', label: t('integration.libSortName') },
            { value: 'recent', label: t('integration.libSortRecent') },
            { value: 'updated', label: t('integration.libSortUpdated') },
          ]"
          size="sm"
          :pop-min-width="140"
          @update:model-value="
            (v) => (sortBy = String(v ?? 'default') as 'default' | 'name' | 'recent' | 'updated')
          "
        />
        <CustomSelect
          class="lib-status-filter"
          :model-value="String(statusFilter)"
          :options="[
            { value: 'all', label: t('integration.libStatusAll') },
            { value: 'ok', label: t('integration.libStatusOk') },
            { value: 'fail', label: t('integration.libStatusFail') },
            { value: 'none', label: t('integration.libStatusNone') },
          ]"
          size="sm"
          :pop-min-width="110"
          @update:model-value="
            (v) => (statusFilter = String(v ?? 'all') as 'all' | 'ok' | 'fail' | 'none')
          "
        />
        <div class="lib-view-toggle">
          <IconButton
            name="list"
            :size="13"
            :title="t('integration.libViewList')"
            :class="{ active: viewMode === 'list' }"
            @click="viewMode = 'list'"
          />
          <IconButton
            name="layout-grid"
            :size="13"
            :title="t('integration.libViewCard')"
            :class="{ active: viewMode === 'card' }"
            @click="viewMode = 'card'"
          />
        </div>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="openCreateFolder(null)">
          <Icon name="folder-plus" :size="13" /> {{ t('integration.folderNew') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="onCreate">
          <Icon name="plus" :size="13" /> {{ t('integration.libNew') }}
        </button>
      </div>

      <!-- 分类过滤 chips -->
      <div v-if="hasCategory" class="lib-cats">
        <button
          class="lib-cat"
          :class="{ active: categoryFilter === null }"
          type="button"
          @click="categoryFilter = null"
        >
          {{ t('integration.libAllCategories') }}
        </button>
        <button
          v-for="c in categories"
          :key="c"
          class="lib-cat"
          :class="{ active: categoryFilter === c }"
          type="button"
          @click="categoryFilter = categoryFilter === c ? null : c"
        >
          {{ c }}
        </button>
        <button
          class="lib-cat"
          :class="{ active: categoryFilter === '' }"
          type="button"
          @click="categoryFilter = categoryFilter === '' ? null : ''"
        >
          {{ t('integration.libUncategorized') }}
        </button>
      </div>

      <!-- 批量操作条（选中 > 0 时显示） -->
      <div v-if="selectedCount" class="lib-batch">
        <label class="lib-batch-all">
          <input type="checkbox" :checked="allSelected" @change="toggleSelectAll" />
          <span>{{ t('integration.libSelectAll') }}</span>
        </label>
        <span class="lib-batch-count">{{ t('integration.libBatchSelected', { n: selectedCount }) }}</span>
        <span class="lib-batch-spacer"></span>
        <button
          type="button"
          class="miyo-btn miyo-btn-sm"
          :disabled="batchBusy"
          @click="batchRun"
        >
          <Icon name="play" :size="13" /> {{ t('integration.libBatchRun') }}
        </button>
        <button
          type="button"
          class="miyo-btn miyo-btn-sm"
          :disabled="batchBusy"
          @click="batchMoveTarget = currentFolderId(); batchMoveOpen = true"
        >
          <Icon name="folder" :size="13" /> {{ t('integration.libBatchMove') }}
        </button>
        <button
          type="button"
          class="miyo-btn miyo-btn-sm"
          :disabled="batchBusy"
          @click="batchTagInput = ''; batchTagOpen = true"
        >
          <Icon name="pin" :size="13" /> {{ t('integration.libBatchTag') }}
        </button>
        <button type="button" class="miyo-btn miyo-btn-sm" :disabled="batchBusy" @click="batchFavorite(true)">
          <Icon name="star" :size="13" /> {{ t('integration.libBatchFavorite') }}
        </button>
        <button type="button" class="miyo-btn miyo-btn-sm" :disabled="batchBusy" @click="batchFavorite(false)">
          <Icon name="star-fill" :size="13" /> {{ t('integration.libBatchUnfavorite') }}
        </button>
        <button
          type="button"
          class="miyo-btn miyo-btn-sm lib-danger"
          :disabled="batchBusy"
          @click="batchDeleteOpen = true"
        >
          <Icon name="trash" :size="13" /> {{ t('integration.libBatchDelete') }}
        </button>
        <IconButton
          name="x"
          :size="13"
          :title="t('integration.libClearSelection')"
          @click="store.clearSelection()"
        />
      </div>

      <!-- 列表视图 -->
      <div v-if="store.libraryItems.length && viewMode === 'list'" ref="listEl" class="lib-list">
        <div
          v-for="s in filtered"
          :key="s.id"
          class="lib-row"
          :class="{ selected: selectedIds.has(s.id) }"
          role="button"
          tabindex="0"
          draggable="true"
          @click="store.openScenario(s.id)"
          @keydown.enter="store.openScenario(s.id)"
          @dragstart="onScenarioDragStart($event, s)"
          @dragend="onScenarioDragEnd"
        >
          <input
            type="checkbox"
            class="lib-check"
            :checked="selectedIds.has(s.id)"
            :title="t('integration.libSelectAll')"
            @click.stop
            @change="toggleSelected(s.id)"
          />
          <span class="lib-run-dot" :class="runTone(s)" :title="runSummaryText(s.last_run)" aria-hidden="true"></span>
          <IconButton
            :name="s.favorite ? 'star-fill' : 'star'"
            :size="12"
            :title="t('integration.libBatchFavorite')"
            class="lib-fav"
            :class="{ on: s.favorite }"
            @click.stop="toggleFavorite(s)"
          />
          <div class="lib-main">
            <span class="lib-name" :title="s.name">{{ s.name }}</span>
            <span v-if="s.folder_id" class="lib-cat-tag lib-folder-tag" :title="folderNameOf(s)">
              {{ folderNameOf(s) }}
            </span>
            <span v-if="s.category" class="lib-cat-tag" :title="s.category">{{ s.category }}</span>
            <span
              v-for="tag in (s.tags ?? []).slice(0, 3)"
              :key="tag"
              class="lib-tag"
              :title="tag"
            >{{ tag }}</span>
          </div>
          <span class="lib-meta">{{ t('integration.libSteps', { n: s.steps.length }) }}</span>
          <span
            v-if="successRate(s) !== null"
            class="lib-rate"
            :class="{ 'lib-rate-bad': (successRate(s) ?? 100) < 100 }"
            :title="runSummaryText(s.last_run)"
          >
            {{ successRate(s) }}%
          </span>
          <span v-else class="lib-meta">—</span>
          <span v-if="s.last_run" class="lib-meta num" :title="runSummaryText(s.last_run)">
            {{ formatDurationShort(s.last_run.duration_ms ?? 0) }}
          </span>
          <span v-else class="lib-meta">—</span>
          <span class="lib-meta lib-runcount" :title="t('integration.libRunCount', { n: s.run_count ?? 0 })">
            {{ s.run_count ?? 0 }}
          </span>
          <span v-if="envBadgeName(s)" class="lib-env" :title="t('integration.libEnvBadge', { name: envBadgeName(s) ?? '' })">
            {{ envBadgeName(s) }}
          </span>
          <span v-else class="lib-meta">—</span>
          <span class="lib-meta lib-time" :title="runSummaryText(s.last_run)">
            {{ s.last_run ? formatDateTime(s.last_run.finished_at) : t('integration.libLastRunNone') }}
          </span>
          <span class="lib-actions">
            <IconButton
              name="history"
              :size="13"
              :title="t('integration.libHistory')"
              :disabled="!s.last_run"
              @click.stop="s.last_run && store.openScenarioRun(s.id, s.last_run.id)"
            />
            <span
              v-if="matrixProgress && runningMatrixScenarioId === s.id"
              class="num text-xxs font-medium text-accent"
              :title="t('integration.matrixRunning', { done: matrixProgress.done, total: matrixProgress.total, label: matrixProgress.label })"
            >
              {{ matrixProgress.done }}/{{ matrixProgress.total }}
            </span>
            <IconButton name="play" :size="13" :title="t('integration.run')" @click.stop="onRunOne(s)" />
            <IconButton
              name="list"
              :size="13"
              :title="t('integration.matrixDataRun')"
              :disabled="!(s.data_sets?.length ?? 0)"
              @click.stop="runMatrix(s, 'data')"
            />
            <IconButton
              name="globe"
              :size="13"
              :title="t('integration.matrixEnvRun')"
              @click.stop="runMatrix(s, 'environment')"
            />
            <IconButton name="pencil" :size="13" :title="t('integration.libRename')" @click.stop="openRename(s)" />
            <IconButton name="copy" :size="13" :title="t('integration.duplicate')" @click.stop="onDuplicate(s)" />
            <IconButton name="download" :size="13" :title="t('integration.export')" @click.stop="onExport(s)" />
            <Popconfirm
              :title="t('integration.confirmDelete', { name: s.name })"
              :confirm-text="t('common.delete')"
              @confirm="onDelete(s.id)"
            >
              <IconButton name="trash" :size="13" :title="t('scenario.delete')" @click.stop />
            </Popconfirm>
          </span>
        </div>
        <p v-if="!filtered.length" class="lib-no-match">{{ t('common.noResults') }}</p>
      </div>

      <!-- 卡片视图 -->
      <div v-else-if="store.libraryItems.length && viewMode === 'card'" ref="listEl" class="lib-cards">
        <div
          v-for="s in filtered"
          :key="s.id"
          class="lib-card"
          :class="{ selected: selectedIds.has(s.id) }"
          role="button"
          tabindex="0"
          draggable="true"
          @click="store.openScenario(s.id)"
          @keydown.enter="store.openScenario(s.id)"
          @dragstart="onScenarioDragStart($event, s)"
          @dragend="onScenarioDragEnd"
        >
          <div class="lib-card-head">
            <input
              type="checkbox"
              class="lib-check"
              :checked="selectedIds.has(s.id)"
              @click.stop
              @change="toggleSelected(s.id)"
            />
            <span class="lib-run-dot" :class="runTone(s)" aria-hidden="true"></span>
            <span class="lib-name" :title="s.name">{{ s.name }}</span>
            <IconButton
              :name="s.favorite ? 'star-fill' : 'star'"
              :size="12"
              class="lib-fav"
              :class="{ on: s.favorite }"
              @click.stop="toggleFavorite(s)"
            />
          </div>
          <p v-if="s.description" class="lib-card-desc">{{ s.description }}</p>
          <div class="lib-card-meta">
            <span v-if="s.folder_id" class="lib-cat-tag lib-folder-tag">{{ folderNameOf(s) }}</span>
            <span v-if="s.category" class="lib-cat-tag">{{ s.category }}</span>
            <span class="lib-meta">{{ t('integration.libSteps', { n: s.steps.length }) }}</span>
            <span class="lib-meta">{{ t('integration.libRunCount', { n: s.run_count ?? 0 }) }}</span>
          </div>
          <div class="lib-card-foot">
            <span class="lib-meta lib-time">{{ runSummaryText(s.last_run) }}</span>
            <span class="lib-actions always">
              <IconButton name="play" :size="13" :title="t('integration.run')" @click.stop="onRunOne(s)" />
              <IconButton name="pencil" :size="13" :title="t('integration.libRename')" @click.stop="openRename(s)" />
              <Popconfirm
                :title="t('integration.confirmDelete', { name: s.name })"
                :confirm-text="t('common.delete')"
                @confirm="onDelete(s.id)"
              >
                <IconButton name="trash" :size="13" :title="t('scenario.delete')" @click.stop />
              </Popconfirm>
            </span>
          </div>
        </div>
        <p v-if="!filtered.length" class="lib-no-match">{{ t('common.noResults') }}</p>
      </div>

      <EmptyState
        v-else-if="!store.libraryLoading"
        icon="beaker"
        :title="t('integration.libEmptyTitle')"
        :description="t('integration.libEmptyDesc')"
      >
        <div class="lib-empty-actions">
          <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="onCreate">
            <Icon name="plus" :size="13" /> {{ t('integration.libNew') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="goAiCreate">
            <Icon name="zap" :size="13" /> {{ t('integration.libEmptyAi') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="onImport">
            <Icon name="upload" :size="13" /> {{ t('integration.libImport') }}
          </button>
        </div>
        <p class="lib-empty-hint">{{ t('integration.libEmptyFromEndpoint') }}</p>
      </EmptyState>
    </div>

    <!-- 项目级执行历史：全部场景的最近运行流（筛选 / 直达详情 / 删除） -->
    <RunHistoryPanel
      v-model:open="historyPanelOpen"
      :project-id="store.projectId"
      @open-run="onHistoryOpenRun"
    />

    <!-- 目录新建 / 重命名 -->
    <Modal
      v-model:open="folderModalOpen"
      :title="folderModalMode === 'create' ? t('integration.folderNew') : t('integration.folderRename')"
      width="380px"
    >
      <input
        v-model="folderNameInput"
        class="miyo-input"
        :placeholder="t('integration.folderNamePh')"
        @keydown.enter="confirmFolderModal"
      />
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="folderModalOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!folderNameInput.trim()"
          @click="confirmFolderModal"
        >
          {{ t('common.save') }}
        </button>
      </template>
    </Modal>

    <!-- 目录删除（三策略：仅空目录 / 场景移到未分类 / 连场景删除） -->
    <Modal
      v-model:open="folderDeleteOpen"
      :title="t('integration.folderDeleteTitle', { name: folderDeleteTarget?.name ?? '' })"
      width="440px"
    >
      <p class="lib-dialog-text">
        {{ t('integration.folderDeleteBody', { count: folderDeleteCount }) }}
      </p>
      <div class="lib-radios">
        <label class="lib-radio" :class="{ disabled: folderDeleteCount > 0 }">
          <input v-model="folderDeleteMode" type="radio" value="empty" :disabled="folderDeleteCount > 0" />
          <span>{{ t('integration.folderDeleteEmpty') }}</span>
        </label>
        <label class="lib-radio">
          <input v-model="folderDeleteMode" type="radio" value="keep" />
          <span>{{ t('integration.folderDeleteKeep') }}</span>
        </label>
        <label class="lib-radio" :class="{ disabled: folderDeleteCount === 0 }">
          <input v-model="folderDeleteMode" type="radio" value="cascade" :disabled="folderDeleteCount === 0" />
          <span>{{ t('integration.folderDeleteCascade', { count: folderDeleteCount }) }}</span>
        </label>
      </div>
      <div v-if="folderDeleteMode === 'cascade'" class="lib-cascade">
        <p class="lib-cascade-warn">
          <Icon name="alert-triangle" :size="13" />
          {{ t('integration.folderDeleteCascadeWarn') }}
        </p>
        <label class="lib-radio">
          <input v-model="cascadeConfirmed" type="checkbox" />
          <span>{{ t('integration.folderDeleteCascadeWarn') }}</span>
        </label>
      </div>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="folderDeleteOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm"
          :class="folderDeleteMode === 'cascade' ? 'lib-btn-danger' : 'miyo-btn-primary'"
          type="button"
          :disabled="folderDeleteMode === 'cascade' && !cascadeConfirmed"
          @click="confirmDeleteFolder"
        >
          {{ t('common.delete') }}
        </button>
      </template>
    </Modal>

    <!-- 场景重命名 -->
    <Modal v-model:open="renameOpen" :title="t('integration.libRename')" width="380px">
      <input
        v-model="renameName"
        class="miyo-input"
        :placeholder="t('integration.libNewNamePh')"
        @keydown.enter="confirmRename"
      />
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="renameOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!renameName.trim()"
          @click="confirmRename"
        >
          {{ t('common.save') }}
        </button>
      </template>
    </Modal>

    <!-- 批量移动到目录 -->
    <Modal v-model:open="batchMoveOpen" :title="t('integration.libBatchMove')" width="380px">
      <CustomSelect
        :model-value="batchMoveTarget ?? ''"
        :options="folderOptions"
        size="sm"
        :pop-min-width="220"
        @update:model-value="(v) => (batchMoveTarget = String(v ?? '') || null)"
      />
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="batchMoveOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="confirmBatchMove">
          {{ t('common.confirm') }}
        </button>
      </template>
    </Modal>

    <!-- 批量添加标签 -->
    <Modal v-model:open="batchTagOpen" :title="t('integration.libBatchTagTitle')" width="380px">
      <input
        v-model="batchTagInput"
        class="miyo-input"
        :placeholder="t('integration.libBatchTagPh')"
        @keydown.enter="confirmBatchTag"
      />
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="batchTagOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!batchTagInput.trim()"
          @click="confirmBatchTag"
        >
          {{ t('common.save') }}
        </button>
      </template>
    </Modal>

    <!-- 批量删除（二次确认） -->
    <Modal v-model:open="batchDeleteOpen" :title="t('integration.libBatchDelete')" width="400px">
      <p class="lib-dialog-text">{{ t('integration.libBatchDeleteConfirm', { n: selectedCount }) }}</p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="batchDeleteOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button class="miyo-btn miyo-btn-sm lib-btn-danger" type="button" @click="batchDelete">
          {{ t('common.delete') }}
        </button>
      </template>
    </Modal>

    <!-- 矩阵结果：逐行对比（行点击 → 打开该行运行详情） -->
    <Modal v-model:open="matrixResultOpen" :title="t('integration.matrixResultTitle')" width="520px">
      <div class="flex flex-col gap-1.5">
        <div v-for="(row, i) in matrixResult?.rows ?? []" :key="row.key" class="mx-row">
          <button type="button" class="min-w-0 flex-1 text-left" @click="openMatrixRow(i)">
            <div class="flex items-center gap-2">
              <span class="mx-dot" :class="row.ok ? 'mx-dot-ok' : 'mx-dot-fail'" aria-hidden="true"></span>
              <span class="text-xs font-medium text-ink1">{{ row.label }}</span>
            </div>
            <p class="mt-0.5 text-xxs text-ink3">
              {{
                t('scenario.assertPassed', {
                  n: row.okCount,
                  m: row.okCount + row.failCount + row.skippedCount,
                })
              }}
              <template v-if="row.durationMs != null"> · {{ Math.round(row.durationMs) }}ms</template>
            </p>
          </button>
          <code class="font-mono text-xxs text-ink3">{{ row.runId.slice(0, 8) }}</code>
        </div>
      </div>
    </Modal>
  </div>
</template>

<style scoped>
.mx-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 12px;
}
.mx-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.mx-dot-ok {
  background: var(--success);
}
.mx-dot-fail {
  background: var(--danger);
}

/* 双栏：左树 + 右列表（树自滚，列表区自滚） */
.lib-root {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.lib-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 14px 18px;
  gap: 10px;
  overflow: hidden;
}

.lib-toolbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.lib-search {
  flex: 1;
  min-width: 160px;
  max-width: 360px;
}

.lib-sort {
  flex: none;
  width: 132px;
}

.lib-status-filter {
  flex: none;
  width: 108px;
}

.lib-view-toggle {
  flex: none;
  display: flex;
  align-items: center;
  gap: 1px;
  padding: 1px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.lib-view-toggle :deep(button.active) {
  color: var(--accent);
  background: var(--accent-tint);
}

.lib-toggle-on {
  color: var(--accent);
  border-color: var(--accent);
}

.lib-cats {
  flex: none;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.lib-cat {
  height: 24px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-family: inherit;
  cursor: pointer;
}

.lib-cat:hover {
  border-color: var(--border-strong);
  color: var(--text-1);
}

.lib-cat.active {
  background: var(--accent-tint);
  border-color: var(--accent);
  color: var(--accent);
}

/* 批量操作条 */
.lib-batch {
  flex: none;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  padding: 6px 10px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  background: var(--accent-tint);
}

.lib-batch-all {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-2);
  cursor: pointer;
}

.lib-batch-count {
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--accent);
}

.lib-batch-spacer {
  flex: 1;
}

.lib-list {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
  overflow-y: auto;
  padding-right: 2px;
}

.lib-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  cursor: pointer;
  min-width: 0;
  content-visibility: auto;
  contain-intrinsic-size: auto 42px;
}

.lib-row:hover {
  border-color: var(--border-strong);
  background: var(--bg-card);
}

.lib-row.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

.lib-check {
  flex: none;
  width: 13px;
  height: 13px;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.lib-fav {
  flex: none;
  opacity: 0.45;
}

.lib-fav.on,
.lib-row:hover .lib-fav {
  opacity: 1;
  color: var(--warning);
}

.lib-run-dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.5;
}

.lib-run-dot.ok {
  background: var(--success);
  opacity: 1;
}

.lib-run-dot.fail {
  background: var(--danger);
  opacity: 1;
}

.lib-run-dot.cancel {
  background: var(--warning);
  opacity: 1;
}

.lib-main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
}

.lib-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  font-weight: 500;
}

.lib-row:hover .lib-name {
  color: var(--accent);
}

.lib-cat-tag {
  flex: none;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  background: var(--accent-tint);
  color: var(--accent);
}

.lib-folder-tag {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-2);
}

.lib-tag {
  flex: none;
  max-width: 90px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xxs);
  padding: 1px 7px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-3);
}

.lib-meta {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono);
}

.lib-runcount {
  width: 30px;
  text-align: right;
}

.lib-time {
  width: 148px;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lib-rate {
  flex: none;
  width: 42px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--success);
}

.lib-rate-bad {
  color: var(--danger);
}

.lib-env {
  flex: none;
  max-width: 110px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border);
  color: var(--text-2);
  font-family: var(--font-mono);
}

.lib-actions {
  flex: none;
  display: none;
  align-items: center;
  gap: 2px;
}

.lib-row:hover .lib-actions,
.lib-card:hover .lib-actions,
.lib-actions.always {
  display: inline-flex;
}

/* 卡片视图 */
.lib-cards {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 10px;
  align-content: start;
  overflow-y: auto;
  padding-right: 2px;
}

.lib-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  cursor: pointer;
}

.lib-card:hover {
  border-color: var(--border-strong);
  background: var(--bg-card);
}

.lib-card.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

.lib-card-head {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.lib-card-head .lib-name {
  flex: 1;
}

.lib-card-desc {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.lib-card-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.lib-card-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.lib-card-foot .lib-time {
  width: auto;
  text-align: left;
  flex: 1;
  min-width: 0;
}

.lib-no-match {
  margin: 0;
  padding: 16px 8px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  text-align: center;
}

.lib-empty-actions {
  display: flex;
  gap: 8px;
}

.lib-empty-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* 对话框 */
.lib-dialog-text {
  margin: 0 0 10px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.6;
}

.lib-radios {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.lib-radio {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  line-height: 1.5;
}

.lib-radio.disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.lib-radio input {
  margin: 2px 0 0;
  flex: none;
  accent-color: var(--accent);
}

.lib-cascade {
  margin-top: 10px;
  padding: 8px 10px;
  border: 1px solid var(--danger);
  border-radius: var(--radius-md);
  background: var(--danger-tint);
}

.lib-cascade-warn {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 6px;
  font-size: var(--fs-xxs);
  color: var(--danger);
}

.lib-btn-danger {
  border-color: var(--danger);
  color: var(--danger);
}

.lib-btn-danger:hover {
  background: var(--danger-tint);
}

.lib-danger {
  color: var(--danger);
}

/* 窄窗口逐级收起次要列（窗口可拖窄，行内固定列不参与收缩） */
@media (max-width: 1180px) {
  .lib-runcount {
    display: none;
  }
}

@media (max-width: 1040px) {
  .lib-env {
    display: none;
  }
}

@media (max-width: 900px) {
  .lib-rate,
  .lib-time {
    display: none;
  }
}
</style>
