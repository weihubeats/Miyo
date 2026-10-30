<script setup lang="ts">
/**
 * ProjectList：仪表板视图（Dashboard）。
 *
 * - 顶部栏：Miyo 品牌 + 设置（无重复搜索框）；
 * - 左侧导航：DashboardNav 组件；
 * - 摘要区：总 API 数 + 最近修改项目 + 快速请求入口；
 * - 工具栏：名称过滤 + 视图切换（网格/列表）+ 排序（最近修改/名称/API数量）+ 新建项目；
 * - 项目卡片 / 弹窗（新建、重命名、删除、快速请求）拆分至 components/projectlist/。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ensureSwapMounted } from '../utils/sortable'
import Sortable from 'sortablejs'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { hasPendingUpdate, pendingUpdateVersion } from '../composables/useAutoUpdate'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import Icon from '../components/ui/Icon.vue'
import IconButton from '../components/ui/IconButton.vue'
import SettingsDialog from '../components/SettingsDialog.vue'
import HeaderUpdateTag from '../components/HeaderUpdateTag.vue'
import DashboardNav from '../components/projectlist/DashboardNav.vue'
import ProjectTabs from '../components/ProjectTabs.vue'
import WindowControls from '../components/WindowControls.vue'
import ImportDialog from '../components/ImportDialog.vue'
import ProjectCard from '../components/projectlist/ProjectCard.vue'
import CustomSelect from '../components/ui/CustomSelect.vue'
import type { SelectOption } from '../components/ui/CustomSelect.vue'
import Skeleton from '../components/ui/Skeleton.vue'
import ProjectCreateModal from '../components/projectlist/ProjectCreateModal.vue'
import TeamManageModal from '../components/projectlist/TeamManageModal.vue'
import SyncImportDialog from '../components/projectlist/SyncImportDialog.vue'
import ProjectRenameModal from '../components/projectlist/ProjectRenameModal.vue'
import ProjectDeleteModal from '../components/projectlist/ProjectDeleteModal.vue'
import ScratchRequestModal from '../components/projectlist/ScratchRequestModal.vue'
import { timeAgo } from '../components/projectlist/projectMeta'
import { aggregateProjectStats, totalEndpointCount } from '../utils/projectStats'
import { methodTone } from '../utils/methodTone'
import { useWindowDrag } from '../composables/useWindowDrag'
import logo from '../assets/miyo-logo.svg'
import type { HttpMethod, Project, Team } from '../types/miyoApi'

const api = useMiyoApi()
const toast = useToast()
const router = useRouter()
const workspace = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

const projects = ref<Project[]>([])
const teams = ref<Team[]>([])
const counts = ref<Record<string, number>>({})
/** 每个项目最近更新的接口摘要（仅 method/path，统计命令返回，不拉全量）。 */
const latestEndpoints = ref<Record<string, { method: HttpMethod; path: string } | null>>({})
const loading = ref(false)
const loadError = ref<string | null>(null)
const search = ref('')

// ---------- 摘要 ----------
const totalProjects = computed(() => projects.value.length)

const totalApis = computed(() => totalEndpointCount(counts.value))

/** 按时段问候（Hero 欢迎语，整句键）。 */
const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 5) return t('projectlist.greetLateNight')
  if (h < 11) return t('projectlist.greetMorning')
  if (h < 14) return t('projectlist.greetNoon')
  if (h < 18) return t('projectlist.greetAfternoon')
  return t('projectlist.greetEvening')
})

/** 最近活动条目的 Method Badge（GraphQL 端点单独识别为粉色 GQL）。 */
function badgeOf(projectId: string): { text: string; cls: string } {
  const ep = latestEndpoints.value[projectId]
  if (!ep) return { text: 'NEW', cls: 'new' }
  if (/graphql/i.test(ep.path)) return { text: 'GQL', cls: 'gql' }
  const cls = ['get', 'post', 'put', 'delete', 'patch'].includes(ep.method.toLowerCase())
    ? ep.method.toLowerCase()
    : 'other'
  return { text: ep.method === 'DELETE' ? 'DEL' : ep.method, cls }
}

/** 时间线徽章色：方法走共享 methodTone，GQL 粉 / NEW 灰为特例。 */
function tlTone(cls: string): string {
  if (cls === 'gql') return 'tl-gql'
  return methodTone(cls)
}

const recentProjects = computed(() =>
  [...projects.value].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 3),
)

// ---------- 过滤 ----------
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  // 无排序控件：保持数据库顺序（即用户手动拖拽的持久化顺序）
  if (!q) return [...projects.value]
  return projects.value.filter((p) => p.name.toLowerCase().includes(q))
})

// ---------- 团队分组 ----------
/** 默认团队固定 UUID（与迁移 0015 / miyo_core::model::DEFAULT_TEAM_ID 一致）。 */
const DEFAULT_TEAM_ID = '00000000-0000-0000-0000-000000000001'
/** 团队筛选：全部分组哨兵值。 */
const TEAM_ALL = '__all__'

/** 团队筛选下拉（工具栏视角切换）。 */
const teamFilter = ref<string>(TEAM_ALL)
const teamFilterOptions = computed<SelectOption[]>(() => [
  { value: TEAM_ALL, label: t('projectlist.teamAll') },
  ...teams.value.map((team) => ({ value: team.id, label: team.name })),
])

/** 单团队视角：选中了具体团队（Header 计数文案随视角切换）。 */
const isSingleTeamView = computed(() => teamFilter.value !== TEAM_ALL)

interface TeamGroup {
  team: Team
  projects: Project[]
}

/** 按团队分组（团队按创建顺序，项目保持组内全局顺序）。 */
const grouped = computed<TeamGroup[]>(() => {
  // 先套团队筛选，再按团队归桶
  const visible =
    teamFilter.value === TEAM_ALL
      ? filtered.value
      : filtered.value.filter((p) => (p.team_id ?? DEFAULT_TEAM_ID) === teamFilter.value)
  const map = new Map<string, TeamGroup>()
  for (const team of teams.value) {
    if (teamFilter.value === TEAM_ALL || team.id === teamFilter.value) {
      map.set(team.id, { team, projects: [] })
    }
  }
  // team_id 缺失/指向已消失团队：兜底进默认团队组
  const fallback = (): TeamGroup => {
    let g = map.get(DEFAULT_TEAM_ID)
    if (!g) {
      g = {
        team: { id: DEFAULT_TEAM_ID, name: t('projectlist.defaultTeam'), created_at: '', updated_at: '' },
        projects: [],
      }
      map.set(DEFAULT_TEAM_ID, g)
    }
    return g
  }
  for (const p of visible) {
    const g = (p.team_id && map.get(p.team_id)) || fallback()
    g.projects.push(p)
  }
  const groups = [...map.values()]
  // 搜索时只留命中的团队组，避免空组占位
  return search.value.trim() ? groups.filter((g) => g.projects.length) : groups
})

// ---------- 拖拽排序 ----------
/** 手动顺序 + 无搜索过滤时才允许拖拽（否则顺序无意义）。 */
const dragEnabled = computed(
  () => !search.value.trim() && projects.value.length > 1,
)

const groupsEl = ref<HTMLElement | null>(null)
const sortables: Sortable[] = []
const savingOrder = ref(false)

/** 交换式拖拽：A 拖到 B 后面时 B 直接让位到 A 原位，无 ghost 空白占位。 */
ensureSwapMounted()

/** 拖拽结束后的短暂窗口内抑制卡片 click（防止拖完被带进项目）。 */
let suppressClickUntil = 0

/**
 * 按当前 DOM 顺序重排 projects 并持久化。
 * 团队分组后 DOM = 各团队组按序拼接，因此逐组收集组内 DOM 顺序再展平，
 * 即得到与视觉一致的全局顺序（拖拽只能组内换位，组间不可跨）。
 */
async function onDragEnd(): Promise<void> {
  suppressClickUntil = Date.now() + 400
  const ordered: Project[] = []
  const grids = groupsEl.value?.querySelectorAll<HTMLElement>('.team-grid') ?? []
  // id → 项目 的一次建表：卡片数平方级的 find 换成 Map 查找（拖拽重排只有这一个热循环）
  const byId = new Map(projects.value.map((p) => [p.id, p]))
  for (const grid of grids) {
    for (const el of grid.querySelectorAll<HTMLElement>('[data-project-id]')) {
      const id = el.dataset.projectId
      const found = id ? byId.get(id) : undefined
      if (found) ordered.push(found)
    }
  }
  if (ordered.length !== projects.value.length) return
  projects.value = ordered
  if (savingOrder.value) return
  savingOrder.value = true
  try {
    await api.updateProjectsOrder(ordered.map((p) => p.id))
  } catch (e) {
    toast.error(t('projectlist.orderSaveFail'), { message: e instanceof Error ? e.message : String(e), duration: 4000 })
  } finally {
    savingOrder.value = false
  }
}

/** 拖拽刚结束的 click 不进入项目。 */
function onCardOpen(p: Project): void {
  if (Date.now() < suppressClickUntil) return
  void enter(p)
}

/** 每个团队组一个 Sortable 实例（组内换位，组间由分组结构天然隔离）。 */
function initSortables(): void {
  const grids = groupsEl.value?.querySelectorAll<HTMLElement>('.team-grid') ?? []
  // 清理已卸载的旧容器（分组/搜索变化会重建 DOM）
  for (let i = sortables.length - 1; i >= 0; i--) {
    if (!sortables[i]!.el.isConnected) {
      sortables[i]!.destroy()
      sortables.splice(i, 1)
    }
  }
  for (const grid of grids) {
    if (sortables.some((s) => s.el === grid)) continue
    sortables.push(
      Sortable.create(grid, {
        animation: 200,
        easing: 'cubic-bezier(0.2, 0, 0, 1)',
        swap: true,
        swapClass: 'sortable-swap',
        swapThreshold: 0.75,
        forceFallback: true,
        fallbackClass: 'sortable-drag',
        fallbackOnBody: true,
        fallbackTolerance: 3,
        ghostClass: 'sortable-ghost',
        dragClass: 'sortable-drag',
        chosenClass: 'sortable-chosen',
        disabled: !dragEnabled.value,
        filter: 'button, a, input, select, textarea, [data-no-drag]',
        preventOnFilter: true,
        onMove: (evt) =>
          !evt.related?.closest('button, a, input, select, textarea, [data-no-drag]'),
        onEnd: () => void onDragEnd(),
      }),
    )
  }
}

onMounted(() => nextTick(initSortables))

/** 分组/过滤变化重建 DOM 后重新挂载（ref+watch 单容器信号已不适用）。 */
watch([grouped, dragEnabled], () => nextTick(initSortables))

onBeforeUnmount(() => {
  for (const s of sortables) s.destroy()
  sortables.length = 0
})

function isActive(p: Project): boolean {
  return api.activeProject.value?.id === p.id
}

// ---------- 数据 ----------
async function loadCounts(): Promise<void> {
  // 单条统计 IPC（后端聚合），替代逐项目拉全量接口的 N+1 加载；
  // 字段校验与聚合在 aggregateProjectStats（snake_case 契约，畸形条目跳过）
  try {
    const stats = await api.listProjectStats()
    const { counts: nextCounts, latest: nextLatest } = aggregateProjectStats(stats)
    counts.value = nextCounts
    latestEndpoints.value = nextLatest
    statsError.value = false
  } catch {
    // 失败保留旧值不清零（静默归零会让用户误以为数据丢失），并给出重试入口
    statsError.value = true
  }
}

/** 统计加载失败标记（内联重试，避免静默归零误导「数据丢了」）。 */
const statsError = ref(false)

async function load(): Promise<void> {
  if (loading.value) return
  loading.value = true
  loadError.value = null
  try {
    projects.value = await api.getProjects()
    counts.value = {}
    // 团队加载失败不阻塞首页（分组退化为默认团队兜底组）
    api.listTeams().then((list) => (teams.value = list)).catch(() => {})
    await loadCounts()
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
    toast.error(t('projectTabs.loadFail'), { message: loadError.value, duration: 6000 })
  } finally {
    loading.value = false
  }
}

onMounted(load)

// ---------- 新建 ----------
const showCreate = ref(false)
const showTeamMgr = ref(false)
/** 快捷新建的预选团队（团队组头部「＋ 新建」）。 */
const presetTeamId = ref<string | null>(null)

function openCreateForTeam(teamId: string): void {
  presetTeamId.value = teamId
  showCreate.value = true
}

// ---------- 从远端导入（团队组头部「同步」入口） ----------
const importTeam = ref<{ id: string; name: string } | null>(null)

function openImportForTeam(team: { id: string; name: string }): void {
  importTeam.value = team
}

/** 团队管理弹窗变更后：刷新团队分组 + 项目（改名/删除影响归属展示）。 */
async function onTeamsChanged(): Promise<void> {
  try {
    teams.value = await api.listTeams()
  } catch {
    /* 团队刷新失败不影响已有分组展示 */
  }
  projects.value = await api.getProjects()
}

function onCreated(project: Project): void {
  projects.value.push(project)
  counts.value[project.id] = 0
  // 弹窗里可能行内新建了团队：刷新团队列表让新组立即出现
  api.listTeams().then((list) => (teams.value = list)).catch(() => {})
}

// ---------- 进入工作区 ----------
async function enter(project: Project): Promise<void> {
  try {
    // 走 store 切换：当前项目 UI 态入快照，目标项目加入顶栏标签
    // （成功不弹 toast：跳转到工作区本身已是明确反馈）
    await workspace.switchProject(project.id)
    router.push('/workspace')
  } catch (e) {
    toast.error(t('projectlist.enterFail'), { message: e instanceof Error ? e.message : String(e), duration: 6000 })
  }
}

// ---------- 快速请求 / 导入 ----------
const showScratch = ref(false)
const showImport = ref(false)
/** 拖拽导入的预填文档内容（Dropzone 读入文件后带内容打开 ImportDialog）。 */
const droppedText = ref('')

/** 仪表板导入成功（新项目已创建并激活）：加入本地列表。 */
function onImported(project: Project): void {
  projects.value.push(project)
  counts.value[project.id] = 0
}

/** 设置页备份恢复成功：新项目已落库，重取列表让它在首页立即可见（无需重启）。 */
function onRestored(projectId: string): void {
  void load().then(() => {
    // 恢复项目按 updated_at 排最前：滚动到可视区并短暂高亮，避免用户找不到
    nextTick(() => {
      document.querySelector(`[data-project-id="${projectId}"]`)?.scrollIntoView({ block: 'nearest' })
    })
  })
}

// ---------- 拖拽导入（Dropzone） ----------
// Tauri 拦截了 HTML5 drop 事件（dragDropEnabled），文件拖放只提供物理坐标 +
// 路径：监听 webview 拖拽事件，命中 Dropzone 区域时读取文件内容走导入弹窗。
const dropEl = ref<HTMLElement | null>(null)
const dropActive = ref(false)
let unlistenDrop: UnlistenFn | null = null

function inDropZone(px: number, py: number): boolean {
  const rect = dropEl.value?.getBoundingClientRect()
  if (!rect) return false
  // 事件坐标是物理像素，rect 是 CSS 像素
  const dpr = window.devicePixelRatio || 1
  const x = px / dpr
  const y = py / dpr
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

async function importDroppedFile(path?: string): Promise<void> {
  if (!path) {
    toast.error(t('projectlist.dropImportFail'), { message: t('projectlist.dropNoPath') })
    return
  }
  try {
    droppedText.value = await api.readTextFile(path)
    showImport.value = true
  } catch (e) {
    toast.error(t('projectlist.readFileFail'), { message: e instanceof Error ? e.message : String(e), duration: 5000 })
  }
}

onMounted(async () => {
  if (!('__TAURI_INTERNALS__' in window)) return
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload
      if (payload.type === 'enter' || payload.type === 'over') {
        dropActive.value = inDropZone(payload.position.x, payload.position.y)
      } else if (payload.type === 'drop') {
        const inside = inDropZone(payload.position.x, payload.position.y)
        dropActive.value = false
        if (inside) void importDroppedFile(payload.paths[0])
      } else {
        dropActive.value = false
      }
    })
  } catch {
    // 拖拽事件不可用（如纯浏览器 dev）时静默降级为仅点击导入
  }
})

onBeforeUnmount(() => {
  unlistenDrop?.()
  unlistenDrop = null
})

// ---------- 卡片菜单：重命名 / 复制 / 删除 ----------
const menuOpenId = ref<string | null>(null)

function toggleMenu(id: string): void {
  menuOpenId.value = menuOpenId.value === id ? null : id
}

function closeMenu(): void {
  menuOpenId.value = null
}

function onDocClick(): void {
  closeMenu()
}

onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))

const renaming = ref<Project | null>(null)

function openRename(p: Project): void {
  closeMenu()
  renaming.value = p
}

function onRenamed(saved: Project): void {
  const idx = projects.value.findIndex((p) => p.id === saved.id)
  if (idx !== -1) projects.value[idx] = saved
}

async function duplicate(p: Project): Promise<void> {
  closeMenu()
  const now = new Date().toISOString()
  try {
    const copy = await api.saveProject({
      ...p,
      id: crypto.randomUUID(),
      name: t('projectlist.copyName', { name: p.name }),
      created_at: now,
      updated_at: now,
    })
    projects.value.push(copy)
    counts.value[copy.id] = 0
    toast.success(t('projectlist.duplicated'), { message: copy.name })
  } catch (e) {
    toast.error(t('projectlist.duplicateFail'), { message: e instanceof Error ? e.message : String(e), duration: 6000 })
  }
}

const deleting = ref<Project | null>(null)

function openDelete(p: Project): void {
  closeMenu()
  deleting.value = p
}

async function onDeleted(id: string): Promise<void> {
  projects.value = projects.value.filter((p) => p.id !== id)
  delete counts.value[id]
  if (api.activeProject.value?.id === id) {
    await api.setActiveProject(null).catch(() => undefined)
  }
}

const showSettings = ref(false)

/** 顶栏拖拽窗口（与工作区顶栏共用 useWindowDrag）。 */
const topBarEl = ref<HTMLElement | null>(null)
useWindowDrag(topBarEl)
</script>

<template>
  <div class="dash">
    <header ref="topBarEl" class="dash-top">
      <button class="top-brand" type="button" :title="t('app.backHome')" @click="router.push('/projects')">
        <span class="top-logo" aria-hidden="true">
          <img :src="logo" alt="" width="18" height="18" />
        </span>
        <span class="top-title">Miyo</span>
        <span class="top-tag">{{ t('app.tagline') }}</span>
      </button>
      <ProjectTabs class="top-tabs" @new-project="showCreate = true" />
      <div class="top-right">
        <HeaderUpdateTag />
        <span class="miyo-update-wrap">
          <IconButton
            name="settings"
            :size="15"
            :title="hasPendingUpdate ? t('app.updateFound', { v: pendingUpdateVersion() ?? '' }) : t('settings.title')"
            @click="showSettings = true"
          />
        </span>
      </div>
      <WindowControls />
    </header>

    <div class="dash-body">
      <DashboardNav />

      <main class="dash-main">
        <div v-if="loadError" class="miyo-inline-error" role="alert">
          <span class="miyo-inline-error-text">{{ t('workspace.loadFail', { v: loadError }) }}</span>
          <button class="miyo-btn miyo-btn-sm" type="button" :disabled="loading" @click="load">
            {{ loading ? t('common.retrying') : t('common.retry') }}
          </button>
        </div>

        <template v-else>
          <section class="summary-grid">
            <!-- 卡片 1：Hero 欢迎语 + 渐变大数字统计 -->
            <div class="stat-card">
              <span class="stat-icon tint-indigo"><Icon name="gauge" :size="17" /></span>
              <div class="stat-body">
                <div class="hero-greet">
                  <p class="hero-title">{{ greeting }}</p>
                  <p class="hero-sub">
                    {{ t('projectlist.heroSub') }}
                    <button
                      v-if="statsError"
                      class="stats-retry"
                      type="button"
                      @click="loadCounts"
                    >
                      <Icon name="refresh" :size="11" /> {{ t('projectlist.statsRetry') }}
                    </button>
                  </p>
                </div>
                <div class="hero-stats">
                  <div class="hero-stat">
                    <span class="hero-value num">{{ totalProjects }}</span>
                    <span class="hero-label"><Icon name="folder" :size="12" /> {{ t('projectlist.totalProjects') }}</span>
                  </div>
                  <span class="hero-divider" aria-hidden="true"></span>
                  <div class="hero-stat">
                    <span class="hero-value num">{{ totalApis }}</span>
                    <span class="hero-label"><Icon name="plug" :size="12" /> {{ t('projectlist.totalApis') }}</span>
                  </div>
                </div>
              </div>
            </div>

            <!-- 卡片 2：最近活动（时间线条目 + Method Badge） -->
            <div class="stat-card">
              <span class="stat-icon tint-violet"><Icon name="clock" :size="16" /></span>
              <div class="stat-body">
                <span class="stat-label">{{ t('projectlist.recentActivity') }}</span>
                <div v-if="recentProjects.length" class="timeline">
                  <button
                    v-for="p in recentProjects"
                    :key="p.id"
                    class="tl-item"
                    type="button"
                    @click="enter(p)"
                  >
                    <span class="tl-badge" :class="tlTone(badgeOf(p.id).cls)">{{ badgeOf(p.id).text }}</span>
                    <span class="tl-main">
                      <span class="tl-line">
                        <span class="tl-name">{{ p.name }}</span>
                        <span class="tl-time">{{ timeAgo(p.updated_at) }}</span>
                      </span>
                      <span v-if="latestEndpoints[p.id]" class="tl-path mono">
                        {{ latestEndpoints[p.id]!.method }} {{ latestEndpoints[p.id]!.path }}
                      </span>
                    </span>
                  </button>
                </div>
                <span v-else class="stat-sub">{{ t('projectlist.noActivity') }}</span>
              </div>
            </div>

            <!-- 卡片 3：快速开始（主 CTA 光晕按钮 + 次要导入） -->
            <div class="stat-card">
              <span class="stat-icon tint-amber"><Icon name="zap" :size="16" /></span>
              <div class="stat-body">
                <span class="stat-label">{{ t('projectlist.quickStart') }}</span>
                <div class="quick-row">
                  <button class="quick-btn primary" type="button" :title="t('projectlist.scratchHint')" @click="showScratch = true">
                    <Icon name="send" :size="12" /> {{ t('projectlist.quickRequest') }}
                  </button>
                  <button
                    class="quick-btn ghost"
                    type="button"
                    :title="t('projectlist.importHint')"
                    @click="droppedText = ''; showImport = true"
                  >
                    <Icon name="download" :size="12" /> {{ t('projectlist.importProject') }}
                  </button>
                </div>
              </div>
            </div>
          </section>

          <section class="toolbar">
            <div class="toolbar-left">
              <div class="toolbar-filter">
                <Icon name="search" :size="14" />
                <input
                  v-model="search"
                  class="toolbar-filter-input"
                  :placeholder="t('projectlist.filterPh')"
                  spellcheck="false"
                  @keydown.esc="search = ''"
                />
                <button
                  v-if="search"
                  class="toolbar-filter-clear"
                  type="button"
                  :aria-label="t('common.clearSearch')"
                  @click="search = ''"
                >
                  <Icon name="x" :size="12" />
                </button>
              </div>
              <CustomSelect
                v-model="teamFilter"
                class="toolbar-team"
                size="sm"
                :options="teamFilterOptions"
                pop-auto-width
              />
            </div>
            <div class="toolbar-right">
              <button class="team-mgr-btn" type="button" @click="showTeamMgr = true">
                <Icon name="users" :size="14" /> {{ t('team.manage') }}
              </button>
              <button class="btn-new" type="button" @click="showCreate = true">
                <Icon name="plus" :size="14" /> {{ t('projectlist.newProject') }}
              </button>
            </div>
          </section>

          <div v-if="filtered.length" ref="groupsEl" class="team-groups">
            <section v-for="g in grouped" :key="g.team.id" class="team-group">
              <header class="team-divider">
                <div class="team-divider-left">
                  <span class="team-avatar"><Icon name="users" :size="12" /></span>
                  <h2 class="team-name">{{ g.team.name }}</h2>
                  <!-- 单团队视角：括号计数；全部分组：等宽胶囊 Badge -->
                  <span v-if="isSingleTeamView" class="team-count-text">
                    ({{ t('projectlist.teamCount', { n: g.projects.length }) }})
                  </span>
                  <span v-else class="team-badge">{{ g.projects.length }}</span>
                </div>
                <div class="team-divider-actions">
                  <button
                    class="team-add ghost"
                    type="button"
                    :title="t('projectlist.teamSyncHint')"
                    @click="openImportForTeam(g.team)"
                  >
                    <Icon name="refresh" :size="12" /> {{ t('projectlist.teamSync') }}
                  </button>
                  <button
                    class="team-add"
                    type="button"
                    :title="t('projectlist.teamQuickAdd')"
                    @click="openCreateForTeam(g.team.id)"
                  >
                    <Icon name="plus" :size="12" /> {{ t('projectlist.teamAdd') }}
                  </button>
                </div>
              </header>
              <div class="card-grid team-grid" :data-team-id="g.team.id">
                <ProjectCard
                  v-for="p in g.projects"
                  :key="p.id"
                  :project="p"
                  :count="counts[p.id] ?? 0"
                  :active="isActive(p)"
                  :menu-open="menuOpenId === p.id"
                  :draggable="dragEnabled"
                  @open="onCardOpen(p)"
                  @toggle-menu="toggleMenu(p.id)"
                  @rename="openRename(p)"
                  @duplicate="duplicate(p)"
                  @delete="openDelete(p)"
                />
              </div>
              <p v-if="!g.projects.length" class="team-empty">{{ t('projectlist.teamNoProjects') }}</p>
            </section>
          </div>

          <div v-else-if="loading" class="sk-grid" aria-busy="true" :aria-label="t('common.loading')">
            <div v-for="i in 6" :key="i" class="sk-card">
              <Skeleton width="46px" height="46px" radius="12px" />
              <div class="sk-lines">
                <Skeleton width="55%" height="14px" />
                <Skeleton width="80%" height="12px" />
                <Skeleton width="35%" height="11px" />
              </div>
            </div>
          </div>

          <!-- 底部 Dropzone：拖入文档直接导入；点击任意处 / 选择文件按钮同样打开导入 -->
          <div
            v-if="!loading"
            ref="dropEl"
            class="add-card"
            :class="{ 'drop-active': dropActive }"
            role="button"
            tabindex="0"
            :aria-label="t('projectlist.dropzoneAria')"
            @click="droppedText = ''; showImport = true"
            @keydown.enter="droppedText = ''; showImport = true"
          >
            <span class="add-icon">
              <Icon :name="dropActive ? 'download' : 'folder-plus'" :size="20" />
            </span>
            <p class="add-text">
              <template v-if="dropActive">{{ t('projectlist.dropActiveHint') }}</template>
              <template v-else>
                {{ t('projectlist.dropHint') }}
              </template>
            </p>
            <button
              v-if="!dropActive"
              class="miyo-btn miyo-btn-sm"
              type="button"
              @click.stop="droppedText = ''; showImport = true"
            >
              <Icon name="download" :size="13" /> {{ t('projectlist.pickFile') }}
            </button>
          </div>

          <div v-if="!filtered.length && !loading" class="dash-empty">
            <span class="empty-icon"><Icon name="folder" :size="30" /></span>
            <p class="empty-title">{{ search ? t('projectlist.noMatchTitle') : t('projectlist.emptyTitle') }}</p>
            <p class="empty-hint">
              {{ search ? t('projectlist.noMatchHint') : t('projectlist.emptyHint') }}
            </p>
            <button class="miyo-btn miyo-btn-primary" type="button" @click="showCreate = true">
              <Icon name="plus" :size="14" /> {{ t('projectlist.newProject') }}
            </button>
          </div>
        </template>
      </main>
    </div>

    <ProjectCreateModal v-model:open="showCreate" :preset-team-id="presetTeamId" @created="onCreated" />
    <TeamManageModal v-model:open="showTeamMgr" @changed="onTeamsChanged" />
    <SyncImportDialog
      v-if="importTeam"
      :open="!!importTeam"
      :team-id="importTeam.id"
      :team-name="importTeam.name"
      @update:open="importTeam = null"
      @imported="load"
    />

    <ProjectRenameModal :project="renaming" @close="renaming = null" @saved="onRenamed" />

    <ProjectDeleteModal :project="deleting" @close="deleting = null" @deleted="onDeleted" />

    <ScratchRequestModal v-model:open="showScratch" />

    <!-- 仪表板导入：创建新项目承接（区别于工作区内导入到当前项目）；
         拖拽导入时带文件内容直接打开并自动解析 -->
    <ImportDialog
      v-if="showImport"
      mode="new-project"
      :initial-text="droppedText"
      @imported="onImported"
      @close="showImport = false"
    />

    <SettingsDialog v-if="showSettings" @close="showSettings = false" @restored="onRestored" />
  </div>
</template>

<style scoped>
.dash {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-app);
}

/* ---------- 顶部栏 ---------- */
.dash-top {
  height: 56px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 16px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  cursor: grab;
  user-select: none;
}

/* 标签条在顶栏内占满品牌与设置之间 */
.top-tabs {
  flex: 1 1 auto;
  min-width: 0;
}

/* Windows 自绘窗口控制钮贴窗口右缘（抵消顶栏右内边距） */
.dash-top :deep(.win-controls) {
  margin-right: -16px;
}

.top-brand {
  display: flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  padding: 4px 8px;
  margin-left: -8px;
  border: none;
  background: none;
  border-radius: var(--radius);
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.top-brand:hover {
  background: var(--bg-hover);
}
.top-brand:active {
  background: var(--bg-active);
}

.top-logo {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  border-radius: var(--radius-md);
  background: var(--grad-primary);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.25),
    0 2px 8px var(--primary-glow);
}

.top-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-1);
  letter-spacing: 0.01em;
}

.top-tag {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  padding-left: 9px;
  border-left: 1px solid var(--border-strong);
  white-space: nowrap;
}

.top-right {
  margin-left: auto;
  display: flex;
  align-items: center;
}

/* ---------- 主体：导航 + 内容 ---------- */
.dash-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.dash-main {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 28px 32px 40px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* ---------- 摘要卡片（三列固定网格，统一高度与边距） ---------- */
.summary-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
  margin-bottom: 8px;
}
@media (max-width: 960px) {
  .summary-grid {
    grid-template-columns: 1fr;
  }
}

.stat-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 22px 24px;
  border-radius: var(--radius-xl);
  border: 1px solid var(--border);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.025), rgba(255, 255, 255, 0) 55%),
    var(--bg-panel);
  box-shadow: var(--shadow);
  transition:
    border-color var(--dur) var(--ease),
    transform var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.stat-card:hover {
  border-color: var(--border-strong);
  transform: translateY(-1px);
  box-shadow: var(--shadow-lg);
}

/* 图标底座：圆角方块 + 分色微光（indigo / violet / amber 三档） */
.stat-icon {
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  background: var(--accent-tint);
  color: var(--accent);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
}
.stat-icon.tint-indigo {
  background: color-mix(in srgb, var(--accent) 13%, transparent);
  color: var(--accent-hover);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 22%, transparent);
}
.stat-icon.tint-violet {
  background: var(--patch-tint);
  color: var(--patch);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--patch) 20%, transparent);
}
.stat-icon.tint-amber {
  background: var(--warning-tint);
  color: var(--warning);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 18%, transparent);
}

.stat-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

/* ---- Hero 欢迎语 + 渐变大数字 ---- */
.hero-greet {
  min-width: 0;
}

.hero-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--text-1);
  letter-spacing: 0.01em;
}

.hero-sub {
  margin: 3px 0 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hero-stats {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-top: 12px;
}

.hero-stat {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
}

.hero-divider {
  width: 1px;
  align-self: stretch;
  background: var(--border);
}

/* 大数字：实色（skill：禁止渐变文本做大标题） */
.hero-value {
  font-family: var(--font-mono);
  font-size: 30px;
  font-weight: 700;
  line-height: 1.1;
  font-variant-numeric: tabular-nums;
  color: var(--text-1);
}

.hero-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--text-2);
}

/* 分区小标题：Obsidian 式小字 caps */
.stat-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.09em;
  text-transform: uppercase;
  color: var(--text-3);
}

.stat-sub {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* ---- 最近活动：时间线 + Method Badge ---- */
.timeline {
  display: flex;
  flex-direction: column;
  margin-top: 6px;
}

.tl-item {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 8px;
  border: none;
  background: none;
  border-radius: var(--radius);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition: background var(--dur) var(--ease);
}
.tl-item + .tl-item {
  border-top: 1px dashed var(--border);
  border-top-left-radius: 0;
  border-top-right-radius: 0;
}
.tl-item:hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}

/* Method Badge：布局尺寸，颜色走共享 methodTone（GQL 粉 / NEW 灰为特例） */
.tl-badge {
  flex-shrink: 0;
  width: 42px;
  text-align: center;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  line-height: 1.2;
  padding: 4px 0;
  border-radius: 6px;
  border: 1px solid transparent;
  letter-spacing: 0.03em;
  color: var(--text-3);
  background: var(--bg-hover);
}
.tl-badge.tl-gql {
  color: #f472b6;
  background: rgba(236, 72, 153, 0.13);
  border-color: rgba(236, 72, 153, 0.25);
}

.tl-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.tl-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.tl-name {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tl-time {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  flex-shrink: 0;
}

.tl-path {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quick-row {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.quick-btn {
  flex: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border-radius: var(--radius);
  font-size: 12px;
  font-weight: 600;
  font-family: inherit;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease),
    transform var(--dur) var(--ease);
}
.quick-btn:active {
  transform: translateY(1px);
}

/* 主 CTA：紫渐变光晕（与全局 miyo-btn-primary 同语言） */
.quick-btn.primary {
  border: 1px solid transparent;
  background: var(--grad-primary);
  color: #fff;
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.22),
    0 3px 12px var(--primary-glow);
}
.quick-btn.primary:hover {
  border-color: transparent;
  background: var(--grad-primary-hover);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.25),
    0 5px 16px var(--primary-glow-hover);
}
.quick-btn.primary:active {
  background: var(--grad-primary);
  box-shadow: 0 2px 8px var(--primary-glow);
}

/* 次要动作（导入）：中性描边样式 */
.quick-btn.ghost {
  border: 1px solid var(--border-strong);
  background: color-mix(in srgb, var(--text-1) 3%, transparent);
  color: var(--text-2);
}
.quick-btn.ghost:hover {
  background: var(--bg-hover);
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  color: var(--accent);
}

/* ---------- 底部 Dropzone：拖入导入 + 点击创建 ---------- */
.add-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 18px;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-lg);
  background: transparent;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease),
    transform var(--dur) var(--ease);
}
.add-card:hover {
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  background: color-mix(in srgb, var(--accent) 5%, transparent);
}
/* 拖拽悬停：实线主色框 + 光晕 + 轻微上浮，明确「可以放了」 */
.add-card.drop-active {
  border-style: solid;
  border-color: var(--accent);
  background: var(--accent-tint);
  box-shadow:
    0 0 0 4px color-mix(in srgb, var(--accent) 12%, transparent),
    0 8px 30px rgba(9, 12, 22, 0.45);
  transform: translateY(-1px);
}

.add-icon {
  display: inline-flex;
  color: var(--text-3);
  transition: color var(--dur) var(--ease);
}
.add-card:hover .add-icon {
  color: var(--accent);
}

.add-text {
  margin: 0;
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: color var(--dur) var(--ease);
}
.add-card:hover .add-text {
  color: var(--accent-hover);
}

/* ---------- 工具栏：左（搜索 + 分组切换）/ 右（团队管理 + 新建），统一 h-9 ---------- */
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 20px;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
}
.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.toolbar-filter-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  margin-left: auto;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  padding: 0;
}

.toolbar-filter-clear:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.stats-retry {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: 8px;
  border: none;
  background: none;
  color: var(--danger);
  font-size: var(--fs-xxs);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
}

.stats-retry:hover {
  background: var(--bg-hover);
}

.toolbar-filter {
  /* w-60：固定宽度 */
  width: 240px;
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  color: var(--text-3);
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.toolbar-filter:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.toolbar-filter-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: none;
  color: var(--text-1);
  font-family: inherit;
  font-size: 12.5px;
}
.toolbar-filter-input::placeholder {
  color: var(--text-3);
}

/* 分组切换下拉：w-44 */
.toolbar-team {
  width: 176px;
}
.toolbar-team :deep(.cs-trigger) {
  height: 36px;
}

.btn-new {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 36px;
  padding: 0 16px;
  border: none;
  border-radius: var(--radius-lg);
  background: var(--grad-primary);
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
  font-family: inherit;
  cursor: pointer;
  box-shadow: 0 4px 14px var(--primary-glow);
  transition:
    background var(--dur) var(--ease),
    transform var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.btn-new:hover {
  background: var(--grad-primary-hover);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.25),
    0 6px 18px var(--primary-glow-hover);
}
.btn-new:active {
  transform: translateY(1px);
}
.team-mgr-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 36px;
  padding: 0 12px;
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-2);
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.team-mgr-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* ---------- 项目卡片网格（卡片自身样式见 ProjectCard） ---------- */
/* auto-fill + minmax(300px,1fr)：按容器宽度自适应列数；
   轨道下限用固定 300px 而非 auto，避免卡片内 nowrap 长描述把轨道
   撑出容器（曾导致横向滚动条、卡片被切出屏幕）。 */
.card-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px;
}

/* ---------- 团队分组：轻量 Divider（无大卡片外壳，网格直接铺在主背景上） ---------- */
.team-groups {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.team-group {
  display: flex;
  flex-direction: column;
}

.team-divider {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 0 10px;
  margin-top: 4px;
  border-bottom: 1px solid var(--border);
}
.team-divider-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.team-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  border-radius: var(--radius-sm);
  background: var(--accent-tint);
  color: var(--accent);
}
.team-name {
  margin: 0;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.team-badge {
  padding: 1px 8px;
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  color: var(--text-3);
  background: var(--bg-hover);
  border-radius: 999px;
}
/* 单团队视角的括号计数：纯弱化文本，无胶囊底 */
.team-count-text {
  font-size: 12px;
  color: var(--text-3);
}
/* 极简 Ghost「＋ 新建」：hover 才染 accent 色 */
.team-add {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-3);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.team-add:hover {
  color: var(--accent);
  background: var(--accent-tint);
}
.team-empty {
  margin: 0;
  padding: 4px 0 12px;
  font-size: 12.5px;
  color: var(--text-3);
}

/* ---------- 加载骨架（与卡片网格同列型） ---------- */
.sk-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px;
}
.sk-card {
  display: flex;
  align-items: flex-start;
  gap: 14px;
  padding: 18px 20px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
}
.sk-lines {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-top: 2px;
}

/* ---------- 拖拽排序 ---------- */
.card-grid :deep(.proj-card) {
  cursor: grab;
}
.card-grid :deep(.proj-card:active) {
  cursor: grabbing;
}

/* 拖拽中元素禁用自身 transform 过渡：SortableJS 独占移动控制，
   否则卡片 hover 缓动与拖拽动画互相干扰（拖动生硬、掉帧）。 */
:global(.sortable-ghost),
:global(.sortable-drag),
:global(.sortable-chosen) {
  transition: none !important;
  will-change: transform;
}

/* swap：交换目标卡片高亮反馈 */
:global(.sortable-swap) {
  border: 1px solid color-mix(in srgb, var(--accent) 50%, transparent) !important;
  background: color-mix(in srgb, var(--accent) 8%, transparent) !important;
  transition: none !important;
}

/* ghost：原位置镂空占位（ghost 即原卡片元素，内部全部隐藏 + 极淡底色） */
:global(.sortable-ghost) {
  background-color: color-mix(in srgb, var(--accent) 5%, transparent) !important;
  border: 2px dashed color-mix(in srgb, var(--accent) 40%, transparent) !important;
  border-radius: 0.75rem !important;
  box-shadow: none !important;
  opacity: 1;
  cursor: grabbing;
}
:global(.sortable-ghost *) {
  opacity: 0 !important;
  visibility: hidden !important;
}

/* chosen：mousedown 即给原元素反馈（拖起瞬间轻微放大） */
:global(.sortable-chosen) {
  transform: scale(1.03);
  cursor: grabbing;
}

/* drag：被拖主体（fallbackClass 克隆挂 body，悬浮感最强） */
:global(.sortable-drag) {
  z-index: 999;
  border-radius: var(--radius-lg) !important;
  background: rgba(38, 38, 38, 0.95) !important;
  -webkit-backdrop-filter: blur(8px);
  backdrop-filter: blur(8px);
  border: 1px solid var(--accent) !important;
  box-shadow:
    0 20px 50px rgba(9, 12, 22, 0.6),
    0 0 40px rgba(9, 12, 22, 0.4) !important;
  transform: scale(1.03) rotate(1.5deg);
  cursor: grabbing;
}

/* flip：其余卡片平滑挤开（animation 200ms 已生效，此为兜底） */
.card-grid :deep(.flip-list-move) {
  transition: transform 0.2s cubic-bezier(0.2, 0, 0, 1);
}

/* ---------- 空状态 ---------- */
.dash-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 48px 0;
  text-align: center;
}

.empty-icon {
  color: var(--text-3);
  margin-bottom: 6px;
}

.empty-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-1);
}

.empty-hint {
  margin: 0 0 10px;
  font-size: 12.5px;
  color: var(--text-3);
  max-width: 380px;
}
</style>
