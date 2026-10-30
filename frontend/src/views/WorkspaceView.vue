<script setup lang="ts">
/**
 * WorkspaceView：工作区主视图。
 * 左侧接口树（文件夹 + 接口，含 CRUD），右侧标签页 + 编辑器 + 响应区。
 * 树操作全部走 workspace store（Pinia），点击接口打开草稿标签页。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { hasPendingUpdate, pendingUpdateVersion } from '../composables/useAutoUpdate'
import Brand from '../components/Brand.vue'
import HeaderUpdateTag from '../components/HeaderUpdateTag.vue'
import ProjectTabs from '../components/ProjectTabs.vue'
import WindowControls from '../components/WindowControls.vue'
import EndpointTree from '../components/EndpointTree.vue'
import ScenarioEditor from '../components/ScenarioEditor.vue'
import ScenarioPanel from '../components/ScenarioPanel.vue'
import EnvironmentBar from '../components/EnvironmentBar.vue'
import Icon from '../components/ui/Icon.vue'
import IconButton from '../components/ui/IconButton.vue'
import Skeleton from '../components/ui/Skeleton.vue'
import Menu, { type MenuItem } from '../components/ui/Menu.vue'
import CreatePanel, { type CreateActionKey } from '../components/CreatePanel.vue'
import CreateChoicePage from '../components/CreateChoicePage.vue'
import Modal from '../components/ui/Modal.vue'
import ShortcutsHelp from '../components/ShortcutsHelp.vue'
import TabBar from '../components/TabBar.vue'
import Tabs, { type TabItem } from '../components/ui/Tabs.vue'
import Tooltip from '../components/ui/Tooltip.vue'
import EndpointEditor from '../components/EndpointEditor.vue'
import { useShortcuts, shortcutDef } from '../composables/useShortcuts'
import { useWindowDrag } from '../composables/useWindowDrag'
import { lazyComponent } from '../composables/lazyComponent'

// 低频重型面板按需加载（composables/lazyComponent.ts：骨架 + 可翻译的错误重试层）。
// 侧栏历史 / Cookie 页签与四个弹窗都是 v-if 打开才渲染，首次点开才拉各自 chunk，
// 不再随首屏主 chunk 下发（SettingsDialog 近 1800 行是其中大头）。
const HistoryPanel = lazyComponent(() => import('../components/HistoryPanel.vue'))
const CookiePanel = lazyComponent(() => import('../components/CookiePanel.vue'))
const SettingsDialog = lazyComponent(() => import('../components/SettingsDialog.vue'))
const CurlImportDialog = lazyComponent(() => import('../components/CurlImportDialog.vue'))
const ImportDialog = lazyComponent(() => import('../components/ImportDialog.vue'))
const MockRuleDialog = lazyComponent(() => import('../components/MockRuleDialog.vue'))
const SyncDialog = lazyComponent(() => import('../components/SyncDialog.vue'))

const store = useWorkspaceStore()
const router = useRouter()
const route = useRoute()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const loading = ref(false)
const showCurlImport = ref(false)
const curlFolderId = ref<string | null>(null)
const showDocImport = ref(false)
const showMockRules = ref(false)
const showSettings = ref(false)

/** 激活页签是「新建」伪页签 → 内容区渲染新建选择页（否则渲染接口编辑器）。 */
const activeIsCreate = computed(
  () => !!store.activeTabId && store.isCreateTab(store.activeTabId),
)
/** 激活页签是「场景」伪页签 → 内容区渲染场景编排视图。 */
const activeScenarioId = computed(() => {
  const id = store.activeTabId
  return id && store.isScenarioTab(id) ? id.slice('scenario:'.length) : null
})
const showSync = ref(false)
const showShortcuts = ref(false)

/** 快捷键帮助（默认 Ctrl+/，可在设置 → 快捷键中自定义）：集中注册表驱动，列表自动生成。 */
useShortcuts([
  shortcutDef('workspace.shortcuts-help', () => {
    showShortcuts.value = true
  }),
])

/**
 * 侧栏搜索：输入即时回显（v-model），过滤词经 200ms 防抖下发。
 * EndpointTree 每个文件夹实例都会对全量 endpoints 做过滤，逐键触发
 * 会造成 O(文件夹数 × 接口数) 的重复计算风暴。
 */
const apiSearchInput = ref('')
const apiSearch = ref('')
let apiSearchTimer: ReturnType<typeof setTimeout> | undefined
watch(apiSearchInput, (v) => {
  if (apiSearchTimer) clearTimeout(apiSearchTimer)
  if (!v) {
    apiSearch.value = ''
    return
  }
  apiSearchTimer = setTimeout(() => {
    apiSearch.value = v
  }, 200)
})

function clearApiSearch(): void {
  if (apiSearchTimer) clearTimeout(apiSearchTimer)
  apiSearchInput.value = ''
  apiSearch.value = ''
}

// ---------- 侧栏宽度分割（拖拽调整，双击恢复默认） ----------
const SIDEBAR_MIN = 220
const SIDEBAR_MAX = 520
const SIDEBAR_DEFAULT = 300
/** 侧栏宽度持久化键：与请求区高度等「用户布局偏好」同一策略（localStorage）。 */
const SIDEBAR_WIDTH_KEY = 'miyo.sidebar-width'

function clampSidebarWidth(v: number): number {
  return Math.min(Math.max(v, SIDEBAR_MIN), SIDEBAR_MAX)
}

function persistSidebarWidth(): void {
  try {
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(Math.round(sidebarWidth.value)))
  } catch {
    // 存储不可用时静默：仅影响重启后的宽度恢复
  }
}

const sidebarWidth = ref(
  clampSidebarWidth(Number(localStorage.getItem(SIDEBAR_WIDTH_KEY)) || SIDEBAR_DEFAULT),
)
const sidebarResizing = ref(false)
let sidebarDragging = false
let resizeStartX = 0
let resizeStartWidth = 0

function onSidebarResizeDown(event: MouseEvent): void {
  if (event.button !== 0) return
  event.preventDefault()
  sidebarDragging = true
  sidebarResizing.value = true
  resizeStartX = event.clientX
  resizeStartWidth = sidebarWidth.value
  document.body.style.userSelect = 'none'
  document.addEventListener('mousemove', onSidebarResizeMove)
  document.addEventListener('mouseup', onSidebarResizeUp)
}

function onSidebarResizeMove(event: MouseEvent): void {
  if (!sidebarDragging) return
  const next = resizeStartWidth + (event.clientX - resizeStartX)
  sidebarWidth.value = Math.min(Math.max(next, SIDEBAR_MIN), SIDEBAR_MAX)
}

function onSidebarResizeUp(): void {
  if (!sidebarDragging) return
  sidebarDragging = false
  sidebarResizing.value = false
  persistSidebarWidth()
  document.body.style.userSelect = ''
  document.removeEventListener('mousemove', onSidebarResizeMove)
  document.removeEventListener('mouseup', onSidebarResizeUp)
}

/** 键盘调宽（分割条可 Tab 聚焦）：← → 每次 10px，Shift 40px，Home/End 到上下限，Enter 恢复默认。 */
function onSidebarKeydown(event: KeyboardEvent): void {
  const step = event.shiftKey ? 40 : 10
  let next = sidebarWidth.value
  if (event.key === 'ArrowLeft') next -= step
  else if (event.key === 'ArrowRight') next += step
  else if (event.key === 'Home') next = SIDEBAR_MIN
  else if (event.key === 'End') next = SIDEBAR_MAX
  else if (event.key === 'Enter') next = SIDEBAR_DEFAULT
  else return
  event.preventDefault()
  sidebarWidth.value = Math.min(Math.max(next, SIDEBAR_MIN), SIDEBAR_MAX)
  persistSidebarWidth()
}

// ---------- 侧栏页签：接口目录 / 请求历史 ----------
type SidebarTab = 'collections' | 'scenarios' | 'history' | 'cookies'
const sidebarTab = ref<SidebarTab>('collections')
const sidebarTabs = computed<TabItem[]>(() => [
  { key: 'collections', label: t('workspace.collections') },
  { key: 'scenarios', label: t('workspace.scenarios'), count: store.scenarios.length || undefined },
  { key: 'history', label: t('workspace.history'), count: store.histories.length || undefined },
  { key: 'cookies', label: t('workspace.cookies') },
])

// ---------- Mock 服务 ----------
const mockAddress = ref<string | null>(null)
const mockBusy = ref(false)

async function refreshMockStatus(): Promise<void> {
  try {
    mockAddress.value = (await api.mockStatus()) ?? null
  } catch {
    mockAddress.value = null
  }
}

async function toggleMock(): Promise<void> {
  if (mockBusy.value) return
  mockBusy.value = true
  try {
    if (mockAddress.value) {
      await api.mockStop()
      mockAddress.value = null
      toast.success(t('workspace.mockStopped'))
    } else {
      mockAddress.value = await api.mockStart()
      toast.success(t('workspace.mockStarted', { v: mockAddress.value }))
    }
  } catch (err) {
    toast.error(t('workspace.mockFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    mockBusy.value = false
  }
}

async function load(): Promise<void> {
  loading.value = true
  try {
    if (!store.project) {
      const p = await store.init()
      if (!p) {
        router.replace('/projects')
        return
      }
    } else {
      await store.refresh()
    }
  } catch {
    // loadError 已在 store 内写入，界面展示重试
  } finally {
    loading.value = false
  }
}

/** 同步拉取覆盖成功：远端快照已重建项目数据，全量重载工作区。 */
async function onSyncApplied(): Promise<void> {
  try {
    await store.reloadAfterOverwrite()
  } catch {
    // loadError 已在 store 内写入（如有）；界面展示重试
  }
}

/** 设置页备份恢复成功：恢复为全新项目，重载工作区让新项目标签立即出现（无需重启）。 */
async function onRestored(projectId: string): Promise<void> {
  try {
    await store.refreshAfterRestore()
  } catch {
    // loadError 已在 store 内写入（如有）；界面展示重试
  }
  const name = store.openProjects.find((t) => t.id === projectId)?.name
  toast.info(t('workspace.backupRestored', { name: name ?? t('workspace.untitled') }))
}

async function exportOpenapi(): Promise<void> {
  if (!store.project) return
  try {
    const text = await api.exportOpenapi(store.project.id)
    const blob = new Blob([text], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${store.project.name}-openapi.json`
    a.click()
    URL.revokeObjectURL(url)
    toast.success(t('workspace.exportedOpenapi'))
  } catch (err) {
    toast.error(t('workspace.exportFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function openCurlImport(folderId: string | null): void {
  curlFolderId.value = folderId
  showCurlImport.value = true
}

// ---------- 顶栏下拉菜单（文档 / Mock） ----------
const docsBtn = ref<HTMLButtonElement | null>(null)
const mockBtn = ref<HTMLButtonElement | null>(null)
const menuRef = ref<InstanceType<typeof Menu> | null>(null)
const createPanelRef = ref<InstanceType<typeof CreatePanel> | null>(null)
/** 新建面板是否打开：打开时禁用 + 按钮上的悬浮提示，避免与面板重叠。 */
const createOpen = ref(false)
/** 共享菜单是否打开：打开时禁用触发器上的悬浮提示，避免与菜单重叠。 */
const menuOpen = ref(false)

// ---------- 项目标签（顶栏多项目切换，共享组件） ----------
const treeRef = ref<InstanceType<typeof EndpointTree> | null>(null)

// ---------- 项目创建 / 重命名 / 删除 ----------
const showCreateProject = ref(false)
const newProjectName = ref('')
const newProjectDesc = ref('')
const projectFormError = ref<string | null>(null)

function openCreateProject(): void {
  newProjectName.value = ''
  newProjectDesc.value = ''
  projectFormError.value = null
  showCreateProject.value = true
}

async function confirmCreateProject(): Promise<void> {
  const name = newProjectName.value.trim()
  if (!name) {
    projectFormError.value = t('workspace.projectNameRequired')
    return
  }
  const now = new Date().toISOString()
  try {
    const p = await api.saveProject({
      id: crypto.randomUUID(),
      name,
      description: newProjectDesc.value.trim(),
      variables: {},
      created_at: now,
      updated_at: now,
    })
    showCreateProject.value = false
    await store.switchProject(p.id)
    toast.success(t('workspace.projectCreated'), { message: name })
  } catch (err) {
    toast.error(t('workspace.projectCreateFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

const renamingProject = ref(false)
const renameProjectName = ref('')

function openRenameProject(): void {
  renameProjectName.value = store.project?.name ?? ''
  projectFormError.value = null
  renamingProject.value = true
}

async function confirmRenameProject(): Promise<void> {
  const name = renameProjectName.value.trim()
  if (!name) {
    projectFormError.value = t('workspace.projectNameRequired')
    return
  }
  const current = store.project
  if (!current) return
  try {
    const saved = await api.saveProject({ ...current, name, updated_at: new Date().toISOString() })
    store.project = saved
    renamingProject.value = false
    toast.success(t('workspace.projectRenamed'))
  } catch (err) {
    toast.error(t('workspace.projectRenameFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

async function confirmDeleteProject(): Promise<void> {
  const target = store.project
  if (!target) return
  try {
    await api.deleteProject(target.id)
    await api.setActiveProject(null).catch(() => undefined)
    store.closeProjectTab(target.id)
    toast.success(t('workspace.projectDeleted'), { message: target.name })
    router.replace('/projects')
  } catch (err) {
    toast.error(t('workspace.projectDeleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function createFolderAtRoot(): void {
  treeRef.value?.startEdit('create-folder', { parentId: null })
}

// ---------- 侧栏目录工具栏：「+ 新建」面板 + 全部展开 / 折叠 ----------
const addBtn = ref<HTMLElement | null>(null)
const expandTick = ref(0)
const collapseTick = ref(0)

function openCreateMenu(): void {
  if (addBtn.value) createPanelRef.value?.openAt(addBtn.value)
}

function onCreateSelect(key: CreateActionKey): void {
  if (key === 'new-request') store.openNewEndpoint(null)
  else if (key === 'new-folder') createFolderAtRoot()
  else if (key === 'import-curl') openCurlImport(null)
  else if (key === 'import-doc') showDocImport.value = true
}

/** 新建选择页动作：两个入口都消耗当前「新建」页签——
 *  新建请求原地替换为接口页签；导入 cURL 先关页签再开对话框
 *  （取消导入则停在相邻页签，+ 可再开新的选择页）。 */
function onCreatePick(key: CreateActionKey): void {
  if (key === 'new-request') store.openNewEndpointFromCreateTab()
  else if (key === 'import-curl') {
    onCreatePageClose()
    openCurlImport(null)
  }
}

/** 选择页 Esc：关掉当前「新建」页签（激活态自动落到相邻页签）。 */
function onCreatePageClose(): void {
  const id = store.activeTabId
  if (id && store.isCreateTab(id)) store.closeTab(id)
}

const DOCS_ITEMS = computed<MenuItem[]>(() => [
  { key: 'import', label: t('workspace.importDocFile'), icon: 'upload' },
  { key: 'export', label: t('workspace.exportOpenapi'), icon: 'download' },
])

function openDocsMenu(): void {
  if (docsBtn.value) menuRef.value?.openAt(docsBtn.value, DOCS_ITEMS.value, 'right')
}

function onDocsSelect(item: MenuItem): void {
  if (item.key === 'import') showDocImport.value = true
  else void exportOpenapi()
}

function openMockMenu(): void {
  if (!mockBtn.value) return
  menuRef.value?.openAt(mockBtn.value, [
    {
      key: 'toggle',
      label: mockBusy.value ? t('workspace.processing') : mockAddress.value ? t('workspace.mockStop') : t('workspace.mockStart'),
      icon: mockAddress.value ? 'stop' : 'play',
      disabled: mockBusy.value,
    },
    { key: 'rules', label: t('workspace.mockRules'), icon: 'shield', dividerBefore: true },
  ], 'right')
}

function onMockSelect(item: MenuItem): void {
  if (item.key === 'toggle') void toggleMock()
  else if (item.key === 'rules') showMockRules.value = true
}

/** 顶栏直达工具按钮（GraphQL/实时）只在常规宽度展开：窄窗口右栏放不下，
 *  收进溢出菜单（TOOLS_ITEMS 同步增删，两个入口互斥出现）。 */
const WIDE_TOOLS_QUERY = '(min-width: 1180px)'
const wideTools = ref(window.matchMedia(WIDE_TOOLS_QUERY).matches)
const wideToolsMql = window.matchMedia(WIDE_TOOLS_QUERY)
function onWideToolsChange(event: MediaQueryListEvent): void {
  wideTools.value = event.matches
}
wideToolsMql.addEventListener('change', onWideToolsChange)
onBeforeUnmount(() => wideToolsMql.removeEventListener('change', onWideToolsChange))

/** 顶栏溢出菜单：窄窗口承载 GraphQL/实时（视图切换）+ 快捷键 + 团队同步。 */
const TOOLS_ITEMS = computed<MenuItem[]>(() => {
  const items: MenuItem[] = []
  if (!wideTools.value) {
    items.push(
      { key: 'tools-graphql', label: t('workspace.graphql'), icon: 'code', checked: route.path === '/graphql' },
      { key: 'tools-realtime', label: t('workspace.realtime'), icon: 'zap', checked: route.path === '/realtime' },
    )
  }
  items.push(
    { key: 'tools-shortcuts', label: t('workspace.shortcutsHint'), icon: 'keyboard', dividerBefore: !wideTools.value },
    { key: 'tools-sync', label: t('sync.title'), icon: 'refresh' },
  )
  return items
})

const toolsBtn = ref<HTMLButtonElement | null>(null)

function openToolsMenu(): void {
  if (toolsBtn.value) menuRef.value?.openAt(toolsBtn.value, TOOLS_ITEMS.value, 'right')
}

function onMenuSelect(item: MenuItem): void {
  if (item.key === 'new-project') openCreateProject()
  else if (item.key === 'import' || item.key === 'export') onDocsSelect(item)
  else if (item.key === 'tools-graphql') router.push('/graphql')
  else if (item.key === 'tools-realtime') router.push('/realtime')
  else if (item.key === 'tools-shortcuts') showShortcuts.value = true
  else if (item.key === 'tools-sync') showSync.value = true
  else onMockSelect(item)
}

/** Agent 控制面事件载荷（miyo-agent::server::AgentEvent，字段 snake_case）。 */
interface AgentEventPayload {
  type: string
  endpoint_id?: string
  project_id?: string
  name?: string
}

let unlistenAgent: UnlistenFn | null = null
let agentListenerDisposed = false

// 关闭最后一个项目标签（或项目被删）后回到项目列表
watch(
  () => store.project,
  (p) => {
    if (!p) void router.replace('/projects')
  },
)

onMounted(() => {
  load()
  refreshMockStatus()
  void store.loadHistories()
  // AI Agent 经控制面导入接口后刷新侧栏并提示（仅当前激活项目时）
  if ('__TAURI_INTERNALS__' in window) {
    void listen<AgentEventPayload>('miyo:agent-event', async (event) => {
      const payload = event.payload
      if (payload.type !== 'endpoint-imported') return
      if (!store.project || payload.project_id !== store.project.id) return
      await store.refresh()
      toast.info(t('workspace.agentImported', { name: payload.name ?? t('workspace.untitled') }))
    }).then((unlisten) => {
      // 组件在 IPC 返回前已卸载：立即解绑，避免泄漏
      if (agentListenerDisposed) {
        unlisten()
        return
      }
      unlistenAgent = unlisten
    })
  }
})

/** 顶栏拖拽窗口：空白处 mousedown → startDragging；交互元素（按钮/输入等）跳过；双击切换最大化。 */
const topBarEl = ref<HTMLElement | null>(null)
useWindowDrag(topBarEl)

onBeforeUnmount(() => {
  agentListenerDisposed = true
  menuRef.value?.close()
  unlistenAgent?.()
  if (apiSearchTimer) clearTimeout(apiSearchTimer)
  document.removeEventListener('mousemove', onSidebarResizeMove)
  document.removeEventListener('mouseup', onSidebarResizeUp)
})
</script>

<template>
  <div class="workspace">
    <div v-if="store.project" ref="topBarEl" class="top-bar">
      <div class="tb-region tb-left">
        <Brand title="Miyo" class="tb-brand" />
      </div>
      <span class="tb-divider" aria-hidden="true"></span>

      <div class="tb-region tb-projects">
        <ProjectTabs
          project-actions
          @new-project="openCreateProject"
          @rename-project="openRenameProject"
          @delete-project="confirmDeleteProject"
        />
      </div>
      <span class="tb-divider" aria-hidden="true"></span>

      <div class="tb-region tb-right">
        <button
          ref="docsBtn"
          class="miyo-btn miyo-btn-sm tb-action"
          type="button"
          aria-haspopup="menu"
          @click="openDocsMenu"
        >
          <Icon name="file" :size="13" /> {{ t('workspace.docs') }} <Icon name="chevron-down" :size="12" />
        </button>
        <button
          ref="mockBtn"
          class="miyo-btn miyo-btn-sm tb-action"
          type="button"
          aria-haspopup="menu"
          @click="openMockMenu"
        >
          <span class="mock-dot" :class="{ on: mockAddress }"></span>
          Mock <Icon name="chevron-down" :size="12" />
        </button>
        <EnvironmentBar />
        <div class="tb-tool-group" role="toolbar" :aria-label="t('workspace.tools')">
          <!-- 直达按钮：GraphQL/实时（常规宽度展示，窄窗口由溢出菜单接管） -->
          <template v-if="wideTools">
            <IconButton
              class="tb-tool"
              :class="{ active: route.path === '/graphql' }"
              name="code"
              :size="15"
              :title="t('workspace.graphql')"
              @click="router.push('/graphql')"
            />
            <IconButton
              class="tb-tool"
              :class="{ active: route.path === '/realtime' }"
              name="zap"
              :size="15"
              :title="t('workspace.realtime')"
              @click="router.push('/realtime')"
            />
          </template>
          <!-- 溢出菜单：窄窗口承载 GraphQL/实时，常驻快捷键/同步 -->
          <button
            ref="toolsBtn"
            class="tb-tool tb-more"
            type="button"
            :title="t('workspace.moreTools')"
            :aria-label="t('workspace.moreTools')"
            aria-haspopup="menu"
            @click="openToolsMenu"
          >
            <Icon name="more-horizontal" :size="15" :stroke-width="1.5" />
          </button>
          <HeaderUpdateTag />
          <span class="miyo-update-wrap">
            <IconButton
              class="tb-tool"
              name="settings"
              :size="15"
              :title="hasPendingUpdate ? t('app.updateFound', { v: pendingUpdateVersion() ?? '' }) : t('settings.title')"
              @click="showSettings = true"
            />
          </span>
        </div>
      </div>
      <WindowControls />
    </div>

    <div class="workspace-body">
      <aside class="miyo-sidebar" :style="{ width: `${sidebarWidth}px` }">
        <div data-tauri-drag-region class="sidebar-drag" aria-hidden="true"></div>
        <Tabs v-model="sidebarTab" :tabs="sidebarTabs" size="sm" class="sidebar-tabs" />
        <div v-show="sidebarTab === 'collections'" class="sidebar-collections">
          <div class="sidebar-toolbar">
            <div class="sidebar-search">
              <Icon name="search" :size="13" class="ss-icon" />
              <input
                v-model="apiSearchInput"
                class="ss-input"
                type="text"
                :placeholder="t('workspace.searchPh')"
                spellcheck="false"
              />
              <button
                v-if="apiSearchInput"
                class="ss-clear"
                type="button"
                :title="t('common.clearSearch')"
                :aria-label="t('common.clearSearch')"
                @click="clearApiSearch"
              >
                <Icon name="x" :size="12" />
              </button>
            </div>
            <div class="sidebar-tools">
              <Tooltip :content="t('workspace.newHint')" :disabled="createOpen">
                <button
                  ref="addBtn"
                  class="tool-add"
                  type="button"
                  :aria-label="t('workspace.newAria')"
                  @click="openCreateMenu"
                >
                  <Icon name="plus" :size="14" />
                </button>
              </Tooltip>
              <Tooltip :content="t('workspace.collapseAll')">
                <IconButton name="chevrons-down-up" :size="14" :label="t('workspace.collapseAll')" @click="collapseTick++" />
              </Tooltip>
              <Tooltip :content="t('workspace.expandAll')">
                <IconButton name="chevrons-up-down" :size="14" :label="t('workspace.expandAll')" @click="expandTick++" />
              </Tooltip>
            </div>
          </div>
          <div v-if="store.loadError" class="miyo-inline-error" role="alert">
            <span class="miyo-inline-error-text">{{ t('workspace.loadFail', { v: store.loadError }) }}</span>
            <button class="miyo-btn miyo-btn-sm" type="button" :disabled="loading" @click="load">
              {{ loading ? t('common.retrying') : t('common.retry') }}
            </button>
          </div>
          <div
            v-else-if="loading && !store.folders.length && !store.endpoints.length"
            class="sk-tree"
            aria-busy="true"
            :aria-label="t('common.loading')"
          >
            <Skeleton width="64%" height="14px" />
            <Skeleton width="48%" height="14px" />
            <Skeleton width="72%" height="14px" />
            <Skeleton width="42%" height="14px" />
          </div>
          <div v-else class="tree-wrap" :aria-busy="loading">
            <EndpointTree
              ref="treeRef"
              :folder-id="null"
              :search="apiSearch"
              :expand-tick="expandTick"
              :collapse-tick="collapseTick"
              @import-curl="openCurlImport"
            />
          </div>
        </div>
        <ScenarioPanel v-if="sidebarTab === 'scenarios'" class="sidebar-history" />
        <HistoryPanel v-if="sidebarTab === 'history'" class="sidebar-history" />
        <CookiePanel v-if="sidebarTab === 'cookies'" class="sidebar-history" />
      </aside>
      <div
        class="sidebar-resizer"
        :class="{ active: sidebarResizing }"
        role="separator"
        tabindex="0"
        aria-orientation="vertical"
        :aria-label="t('workspace.resizeSidebar')"
        :aria-valuenow="Math.round(sidebarWidth)"
        :aria-valuemin="SIDEBAR_MIN"
        :aria-valuemax="SIDEBAR_MAX"
        :title="t('workspace.resizeHint')"
        @mousedown="onSidebarResizeDown"
        @dblclick="sidebarWidth = SIDEBAR_DEFAULT"
        @keydown="onSidebarKeydown"
      ></div>
      <main class="miyo-main">
        <TabBar
          v-if="store.openTabs.length"
          @create="store.openCreateTab()"
        />
        <ScenarioEditor v-if="activeScenarioId" :scenario-id="activeScenarioId" />
        <CreateChoicePage
          v-else-if="activeIsCreate"
          @pick="onCreatePick"
          @close="onCreatePageClose"
        />
        <EndpointEditor v-else />
      </main>
    </div>

    <Menu ref="menuRef" @select="onMenuSelect" @open="menuOpen = true" @close="menuOpen = false" />
    <CreatePanel
      ref="createPanelRef"
      @select="onCreateSelect"
      @open="createOpen = true"
      @close="createOpen = false"
    />

    <Modal v-model:open="showCreateProject" :title="t('workspace.createProject')" width="420px" @close="showCreateProject = false">
      <div class="form-field">
        <label class="form-label">{{ t('workspace.projectName') }}</label>
        <input
          v-model="newProjectName"
          class="miyo-input"
          :class="{ 'miyo-input-error': projectFormError }"
          :placeholder="t('workspace.projectNamePh')"
          maxlength="60"
          spellcheck="false"
          @input="projectFormError = null"
          @keyup.enter="confirmCreateProject"
        />
      </div>
      <div class="form-field">
        <label class="form-label">{{ t('workspace.projectDesc') }}</label>
        <textarea
          v-model="newProjectDesc"
          class="miyo-textarea"
          :maxlength="200"
          :placeholder="t('workspace.projectDescPh')"
          rows="3"
        ></textarea>
      </div>
      <p v-if="projectFormError" class="miyo-field-error" role="alert">{{ projectFormError }}</p>
      <template #footer>
        <button class="miyo-btn" type="button" @click="showCreateProject = false">{{ t('common.cancel') }}</button>
        <button class="miyo-btn miyo-btn-primary" type="button" :disabled="api.pending.value" @click="confirmCreateProject">
          {{ t('workspace.create') }}
        </button>
      </template>
    </Modal>

    <Modal v-model:open="renamingProject" :title="t('workspace.renameProject')" width="420px" @close="renamingProject = false">
      <div class="form-field">
        <label class="form-label">{{ t('workspace.projectName') }}</label>
        <input
          v-model="renameProjectName"
          class="miyo-input"
          v-focus-end
          :class="{ 'miyo-input-error': projectFormError }"
          :placeholder="t('workspace.projectName')"
          maxlength="60"
          spellcheck="false"
          @input="projectFormError = null"
          @keyup.enter="confirmRenameProject"
        />
      </div>
      <p v-if="projectFormError" class="miyo-field-error" role="alert">{{ projectFormError }}</p>
      <template #footer>
        <button class="miyo-btn" type="button" @click="renamingProject = false">{{ t('common.cancel') }}</button>
        <button class="miyo-btn miyo-btn-primary" type="button" :disabled="api.pending.value" @click="confirmRenameProject">
          {{ t('common.save') }}
        </button>
      </template>
    </Modal>

    <CurlImportDialog
      v-if="showCurlImport"
      :folder-id="curlFolderId"
      @close="showCurlImport = false"
    />
    <ImportDialog v-if="showDocImport" @close="showDocImport = false" />
    <MockRuleDialog v-if="showMockRules" @close="showMockRules = false" />
    <SettingsDialog v-if="showSettings" @close="showSettings = false" @restored="onRestored" />
    <SyncDialog
      v-if="showSync"
      :open="showSync"
      :project-id="store.project?.id ?? null"
      :project-name="store.project?.name ?? ''"
      :team-id="store.project?.team_id ?? null"
      @update:open="showSync = $event"
      @applied="onSyncApplied"
    />
    <ShortcutsHelp :open="showShortcuts" @update:open="showShortcuts = $event" />
  </div>
</template>

<style scoped>
.workspace {
  display: flex;
  flex-direction: column;
  height: 100%;
  box-sizing: border-box;
}

.workspace-body {
  flex: 1;
  display: flex;
  min-height: 0;
}

/* ---- 顶栏：左（品牌）/ 中（状态）/ 右（操作 + 环境）三区 ---- */
.top-bar {
  display: flex;
  align-items: center;
  height: 48px;
  padding: 0 12px;
  gap: 12px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--miyo-border);
  background: var(--miyo-bg-panel);
  cursor: grab;
  user-select: none;
}

/* ---- 窄窗回退（<1080px）：侧栏随视口收窄，把宽度让给右侧请求/响应编辑区；
   顶栏允许换行，避免 Docs/Mock/环境/工具组横向溢出（≥1080px 布局不变） ---- */
@media (max-width: 1080px) {
  .miyo-sidebar {
    max-width: 34vw;
  }
  .top-bar {
    flex-wrap: wrap;
    height: auto;
    min-height: 48px;
    padding: 4px 12px;
    row-gap: 4px;
  }
}

.tb-region {
  display: flex;
  align-items: center;
  min-width: 0;
}

.tb-left {
  flex: 0 1 auto;
}

.tb-brand {
  width: 140px;
}

/* ---- 顶栏项目标签条（ProjectTabs 组件） ---- */
.tb-projects {
  flex: 1 1 auto;
  gap: 6px;
  overflow: hidden;
}

.tb-right {
  margin-left: auto;
  justify-content: flex-end;
  gap: 8px;
  flex: 1;
}

.tb-divider {
  width: 1px;
  height: 20px;
  flex-shrink: 0;
  background: var(--miyo-border);
  opacity: 0.55;
}

/* Windows 自绘窗口控制钮贴窗口右缘（抵消顶栏右内边距） */
.top-bar :deep(.win-controls) {
  margin-right: -12px;
}

.tb-inner-divider {
  width: 1px;
  height: 20px;
  flex-shrink: 0;
  margin: 0 2px;
  background: var(--miyo-border);
  opacity: 0.55;
}

/* ---- 顶栏右区操作：统一 h-8、rounded-lg、gap-2（边框走变量，深浅主题可见） ---- */
.tb-right .tb-action {
  height: 32px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  font-size: 12px;
  gap: 6px;
  color: var(--text-2);
}
.tb-right .tb-action:hover:not(:disabled) {
  background: var(--bg-active);
  border-color: var(--border-strong);
  color: var(--text-1);
}
/* ---- 右区工具组：连体分段控件（单边框 + 内部分隔），替代四个独立方块 ---- */
.tb-tool-group {
  display: flex;
  align-items: stretch;
  flex-shrink: 0;
  height: 32px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  overflow: hidden;
}
.tb-tool-group .tb-tool {
  width: 36px;
  height: auto;
  border-radius: 0;
}
.tb-tool-group .tb-tool + .tb-tool {
  border-left: 1px solid var(--border);
}
.tb-tool-group .tb-tool:hover:not(:disabled) {
  background: var(--bg-active);
  color: var(--text-1);
}
.tb-tool-group .tb-tool:active:not(:disabled) {
  transform: none;
}
/* 当前所在工作台高亮（路由驱动） */
.tb-tool-group .tb-tool.active {
  color: var(--accent);
  background: var(--accent-tint);
}

/* 溢出菜单触发器：原生 button，补齐 IconButton 的基础态（group 规则给宽高/悬停） */
.tb-more {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}

/* ---- Mock 状态圆点（按钮内指示） ---- */
.mock-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--miyo-text-muted);
}

.mock-dot.on {
  background: var(--miyo-success);
  box-shadow: 0 0 0 3px var(--miyo-success-tint);
}

.miyo-sidebar {
  /* 宽度由内联 style 驱动（拖拽分割条），保留 min/max 约束兜底 */
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--sidebar-border);
  background: var(--sidebar-bg);
  overflow: hidden;
}

/* ---- 顶部拖拽区：扩大窗口拖拽面积（内无交互元素，避免吞点击） ---- */
.sidebar-drag {
  flex-shrink: 0;
  height: 12px;
  cursor: default;
}

/* ---- 拖拽分割条：透明叠加在边框上，悬停/拖拽中高亮 ---- */
.sidebar-resizer {
  flex-shrink: 0;
  width: 7px;
  margin-left: -4px;
  cursor: col-resize;
  z-index: 10;
}
.sidebar-resizer:hover,
.sidebar-resizer.active,
.sidebar-resizer:focus-visible {
  background: var(--accent);
}
.sidebar-resizer:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -1px;
}

.miyo-heading {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 目录工具栏：搜索框占满 + 右侧「+ 新建」主按钮 / 折叠展开 ---- */
.sidebar-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 12px 0;
}

.sidebar-toolbar .sidebar-search {
  flex: 1;
  min-width: 0;
  margin: 0;
}

.sidebar-tools {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

/* 主操作「+」：accent 微光方块，页面侧栏唯一强调按钮 */
.tool-add {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
  border-radius: var(--radius);
  background: var(--accent-tint);
  color: var(--accent);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.tool-add:hover {
  border-color: color-mix(in srgb, var(--accent) 85%, transparent);
  background: color-mix(in srgb, var(--accent) 26%, transparent);
  color: var(--accent-hover);
  box-shadow: 0 0 10px color-mix(in srgb, var(--accent) 45%, transparent);
}
.tool-add:active {
  transform: translateY(1px);
}

/* ---- 接口搜索栏：内嵌凹陷质感，聚焦时紫色柔光 ---- */
.sidebar-search {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  margin: 8px 12px 0;
  padding: 0 10px;
  flex-shrink: 0;
  border: 1px solid var(--sidebar-border);
  border-radius: 12px;
  background: var(--sidebar-search-bg);
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.sidebar-search:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.ss-icon {
  flex-shrink: 0;
  color: var(--text-3);
  opacity: 0.7;
}

.ss-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-1);
}
.ss-input::placeholder {
  color: var(--text-3);
}

.ss-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 4px;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  padding: 0;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.ss-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.tree-wrap {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px 12px;
}

/* ---- 加载骨架：首屏目录拉取中占位（与 ProjectList 的 sk-grid 同语言） ---- */
.sk-tree {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 8px 12px 12px;
}

/* ---- 侧栏页签（接口目录 / 请求历史 / Cookie）：分段胶囊，与顶栏工具组同语言 ---- */
.sidebar-tabs {
  flex-shrink: 0;
  margin: 2px 12px 0;
}
.sidebar-tabs :deep(.tabs) {
  border-bottom: none;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-card);
  padding: 2px;
  gap: 2px;
}
.sidebar-tabs :deep(.tab) {
  flex: 1 1 0;
  justify-content: center;
  height: 28px;
  border-radius: var(--radius);
  font-size: 12px;
}
.sidebar-tabs :deep(.tab.active) {
  /* 微弱浅色底 + 微亮内描边（分段胶囊内的激活态，取代重阴影） */
  background: var(--bg-active);
  color: var(--text-1);
  box-shadow: inset 0 0 0 1px var(--border-strong);
}
.sidebar-tabs :deep(.tab.active::after) {
  display: none;
}

.sidebar-collections {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.sidebar-history {
  flex: 1;
  min-height: 0;
}

.miyo-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--miyo-bg);
  overflow: hidden;
}

.miyo-inline-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  border-radius: var(--miyo-radius-sm);
  background: var(--miyo-danger-tint);
  border: 1px solid var(--danger-border);
}

.miyo-inline-error-text {
  font-size: 12.5px;
  color: var(--miyo-danger);
  word-break: break-all;
}

/* ---- 项目表单（新建 / 重命名） ---- */
.form-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
}

.form-label {
  font-size: 12px;
  color: var(--text-2);
}

.miyo-input-error {
  border-color: var(--danger) !important;
}
.miyo-input-error:focus {
  border-color: var(--danger) !important;
  box-shadow: 0 0 0 2px var(--danger-tint) !important;
}
</style>