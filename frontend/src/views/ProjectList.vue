<script setup lang="ts">
/**
 * ProjectList：仪表板视图（Dashboard）。
 *
 * - 顶部栏：Miyo 品牌 + 多项目标签 + 设置（ProjectTabs）；
 * - 左侧导航：DashboardNav（仪表板 / 实时调试）；
 * - 摘要区：欢迎卡（4 项真实统计，可点击跳转）+ 最近活动（真实请求历史）+
 *   快速开始（快速请求 / 新建项目 / 导入项目 / 实时调试）；
 * - 工具栏：名称过滤 + 分组筛选 + 排序（手动/名称/更新时间/API 数）+
 *   网格/列表切换 + 结果统计 + 清空筛选 + 团队管理 + 新建项目；
 * - 项目分组：可折叠（localStorage 持久化），网格支持组内拖拽（仅手动排序）；
 * - 弹窗（新建、重命名、删除、快速请求、导入）拆分至 components/projectlist/。
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
import ProjectListRow from '../components/projectlist/ProjectListRow.vue'
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
import {
  aggregateProjectStats,
  sumCounts,
  totalEndpointCount,
  type AggregatedProjectStats,
} from '../utils/projectStats'
import { methodBadgeTone } from '../utils/methodTone'
import { formatDurationShort } from '../utils/format'
import { useWindowDrag } from '../composables/useWindowDrag'
import { useDashboardStore, emptyProjectStats } from '../stores/dashboard'
import logo from '../assets/miyo-logo.svg'
import type { Project, ProjectStat, RecentActivity, Team } from '../types/miyoApi'

const api = useMiyoApi()
const toast = useToast()
const router = useRouter()
const workspace = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

/**
 * 项目 / 团队 / 统计：初值取 pinia 里的首页快照，返回首页时直接复用上次内容
 * （`<script setup>` 里的 ref 随挂载重建，跨路由切换必须靠这层缓存）。
 */
const dashboard = useDashboardStore()
const projects = ref<Project[]>([...(dashboard.projects ?? [])])
const teams = ref<Team[]>([...(dashboard.teams ?? [])])

/** 首页全部计数（接口 / 场景 / 请求 / 成功失败）都出自这一个聚合结果。 */
const stats = ref<AggregatedProjectStats>(dashboard.stats ?? emptyProjectStats())
/** 模板与排序用的计数视图（场景 / 请求数由总览 computed 直接读 stats）。 */
const counts = computed(() => stats.value.counts)
const scenarioCounts = computed(() => stats.value.scenarioCounts)

/** 统计是否仍未落定（首次加载 / 失败后未重试成功）：用于区分「真 0」与「未加载」。 */
const statsReady = ref(dashboard.stats !== null)
const loading = ref(false)
const loadError = ref<string | null>(null)
const search = ref('')

// ---------- 摘要统计 ----------
const totalProjects = computed(() => projects.value.length)

const totalApis = computed(() => totalEndpointCount(stats.value.counts))

const totalScenarios = computed(() => sumCounts(stats.value.scenarioCounts))

/** 请求总数（接口全量：含失败，与最近活动的记录来源同一张 request_histories 表）。 */
const totalRequests = computed(() => sumCounts(stats.value.requestCounts))

/** 请求明细：2xx/3xx 成功、4xx/5xx 与网络错误失败（两者相加等于总数）。 */
const requestBreakdown = computed(() => {
  const ok = sumCounts(stats.value.okCounts)
  const fail = sumCounts(stats.value.failCounts)
  return { ok, fail }
})

/** 计数展示：未就绪时给占位而不是误导性的 0（项目数来自项目列表，与统计 IPC 无关）。 */
function statText(n: number, fromStats = true): string {
  if (fromStats && !statsReady.value) return '—'
  return String(n)
}

/** 按时段问候（Hero 欢迎语，整句键）。 */
const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 5) return t('projectlist.greetLateNight')
  if (h < 11) return t('projectlist.greetMorning')
  if (h < 14) return t('projectlist.greetNoon')
  if (h < 18) return t('projectlist.greetAfternoon')
  return t('projectlist.greetEvening')
})

// ---------- 最近活动（真实请求历史，list_recent_activities 单条 IPC） ----------
const activities = ref<RecentActivity[]>([])
/** 三态：loading（加载中）/ error（失败可重试）/ ready（已拿到结果，可能为空）。 */
const activitiesState = ref<'loading' | 'error' | 'ready'>('loading')

/** 最多展示 4 条：摘要卡高度固定，多出的记录留给历史页。 */
const recentActivities = computed(() => activities.value.slice(0, 4))

async function loadActivities(): Promise<void> {
  activitiesState.value = 'loading'
  try {
    activities.value = await api.listRecentActivities(6)
    activitiesState.value = 'ready'
  } catch {
    // 失败保留旧数据（首屏则为空）并给出重试入口，不把「加载失败」说成「没有记录」
    activitiesState.value = 'error'
  }
}

/** 活动行 Method Badge：GraphQL 请求单独识别为 GQL，其余走 methodBadgeTone。 */
function actTone(a: RecentActivity): string {
  if (/graphql/i.test(a.url)) return 'graphql'
  return a.method
}

function actBadgeText(a: RecentActivity): string {
  if (/graphql/i.test(a.url)) return 'GQL'
  return a.method === 'DELETE' ? 'DEL' : a.method
}

/** 状态码语义色：2xx/3xx 绿、4xx 橙、5xx 红、无状态中性。 */
function statusCls(status: number | null): string {
  if (status == null) return 'st-none'
  if (status < 400) return 'st-ok'
  if (status < 500) return 'st-warn'
  return 'st-err'
}

/**
 * 活动行第二行的路径展示：只保留 pathname + query（去掉协议与域名）。
 * 域名对「哪条请求」没有区分度，反而把真正有用的 path 挤到省略号里；
 * 解析失败（非标准 URL）时原样返回。
 */
function activityPath(a: RecentActivity): string {
  try {
    const u = new URL(a.url)
    return `${u.pathname}${u.search}`
  } catch {
    return a.url
  }
}

/**
 * 点击活动 → 进入归属项目并恢复该次请求（工作区按 `history` 把记录回填到
 * 编辑器，接口不存在时退化为只定位到 `focus` 接口）。
 * 项目已被删除时给出提示，而不是静默无反应。
 */
async function enterActivity(a: RecentActivity): Promise<void> {
  const target = projects.value.find((p) => p.id === a.project_id)
  if (!target) {
    toast.warning(t('projectlist.activityProjectGone'), { message: a.project_name })
    return
  }
  await enter(target, { endpointId: a.endpoint_id ?? undefined, historyId: a.id })
}

// ---------- 视图偏好（localStorage 持久化：视图模式 / 排序 / 分组折叠） ----------
type ViewMode = 'grid' | 'list'

const VIEW_KEY = 'miyo.dash.viewMode'
const SORT_KEY = 'miyo.dash.sortBy'
const COLLAPSE_KEY = 'miyo.dash.collapsedTeams'

function loadEnum<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key)
    return allowed.includes(v as T) ? (v as T) : fallback
  } catch {
    return fallback
  }
}

function persistPref(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* 写失败（隐私模式等）不影响功能 */
  }
}

const viewMode = ref<ViewMode>(loadEnum<ViewMode>(VIEW_KEY, ['grid', 'list'], 'grid'))
watch(viewMode, (v) => persistPref(VIEW_KEY, v))

/** 排序维度：manual = 数据库顺序（即拖拽持久化顺序）。 */
const sortBy = ref<string>(loadEnum<string>(SORT_KEY, ['manual', 'name', 'updated', 'apis'], 'manual'))
watch(sortBy, (v) => persistPref(SORT_KEY, v))

const sortOptions = computed<SelectOption[]>(() => [
  { value: 'manual', label: t('projectlist.sortManual') },
  { value: 'name', label: t('projectlist.sortName') },
  { value: 'updated', label: t('projectlist.sortUpdated') },
  { value: 'apis', label: t('projectlist.sortApis') },
])

function loadCollapsed(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(COLLAPSE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, boolean>) : {}
  } catch {
    return {}
  }
}

const collapsedTeams = ref<Record<string, boolean>>(loadCollapsed())
watch(collapsedTeams, (v) => persistPref(COLLAPSE_KEY, JSON.stringify(v)), { deep: true })

function isCollapsed(teamId: string): boolean {
  return !!collapsedTeams.value[teamId]
}

function toggleTeam(teamId: string): void {
  collapsedTeams.value = { ...collapsedTeams.value, [teamId]: !collapsedTeams.value[teamId] }
}

// ---------- 过滤 / 排序 ----------
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return [...projects.value]
  return projects.value.filter((p) => p.name.toLowerCase().includes(q))
})

/** 组内排序：manual 保持数据库顺序（拖拽持久化），其余按所选维度。 */
const sorted = computed<Project[]>(() => {
  const list = filtered.value
  if (sortBy.value === 'name') return [...list].sort((a, b) => a.name.localeCompare(b.name, 'zh-Hans-CN'))
  if (sortBy.value === 'updated') return [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at))
  if (sortBy.value === 'apis')
    return [...list].sort((a, b) => (counts.value[b.id] ?? 0) - (counts.value[a.id] ?? 0))
  return list
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

/** 按团队分组（团队按创建顺序，项目保持组内排序后顺序）。 */
const grouped = computed<TeamGroup[]>(() => {
  // 先套团队筛选，再按团队归桶
  const visible =
    teamFilter.value === TEAM_ALL
      ? sorted.value
      : sorted.value.filter((p) => (p.team_id ?? DEFAULT_TEAM_ID) === teamFilter.value)
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

/**
 * 搜索命中落在已折叠的分组里：该组临时展开，否则「匹配 1/2」却一片空白
 * 像丢了数据。折叠偏好不被改写，原文是用户手动的选择。
 */
const searchActive = computed(() => search.value.trim() !== '')

function isGroupCollapsed(teamId: string): boolean {
  if (!isCollapsed(teamId)) return false
  if (!searchActive.value) return true
  return !grouped.value.some((g) => g.team.id === teamId && g.projects.length)
}

/** 是否有生效中的筛选/排序（工具栏显示「清空筛选」与匹配计数）。 */
const hasActiveFilters = computed(
  () => search.value.trim() !== '' || teamFilter.value !== TEAM_ALL || sortBy.value !== 'manual',
)

function closeFilters(): void {
  search.value = ''
  teamFilter.value = TEAM_ALL
  sortBy.value = 'manual'
}

// ---------- 搜索快捷键（⌘K / Ctrl+K 聚焦；Esc 清空） ----------
const searchEl = ref<HTMLInputElement | null>(null)

function onGlobalKeydown(e: KeyboardEvent): void {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'k') return
  // 只在仪表板内生效：其它输入框（弹窗/编辑器）里的 ⌘K 交给组件自己处理
  e.preventDefault()
  searchEl.value?.focus()
  searchEl.value?.select()
}

onMounted(() => window.addEventListener('keydown', onGlobalKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onGlobalKeydown))

/** 结果统计：无筛选时给全量概览，筛选中给匹配/总数。 */
const resultSummary = computed(() => {
  if (!hasActiveFilters.value) {
    return t('projectlist.totalCount', { n: totalProjects.value, groups: grouped.value.length })
  }
  return t('projectlist.shownCount', { shown: filtered.value.length, total: totalProjects.value })
})

// ---------- 拖拽排序 ----------
/** 手动顺序 + 网格视图 + 无搜索过滤时才允许拖拽（列表模式不拖）。 */
const dragEnabled = computed(
  () => viewMode.value === 'grid' && sortBy.value === 'manual' && !search.value.trim() && projects.value.length > 1,
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
  // 拖拽不可用（列表视图 / 非手动排序 / 搜索中）：同步禁用已挂实例，
  // 否则排序切回手动前旧实例仍可拖，顺序会被非持久化顺序覆盖
  if (!dragEnabled.value) {
    for (const s of sortables) s.option('disabled', true)
    return
  }
  for (const grid of grids) {
    const existing = sortables.find((s) => s.el === grid)
    if (existing) {
      existing.option('disabled', false)
      continue
    }
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
        disabled: false,
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
    const agg = aggregateProjectStats(await api.listProjectStats())
    stats.value = agg
    dashboard.setStats(agg)
    statsReady.value = true
    statsError.value = false
  } catch {
    // 失败保留旧值不清零（静默归零会让用户误以为数据丢失），并给出重试入口；
    // 首次失败时统计未就绪，数字显示「—」而不是 0
    statsError.value = true
  }
}

/** 统计加载失败标记（内联重试，避免静默归零误导「数据丢了」）。 */
const statsError = ref(false)

/**
 * 是否已有可用数据：从实时调试等页面返回时组件会重新挂载，
 * 若内存里仍是上一次的完整快照，直接复用并静默刷新——
 * 否则会先渲染骨架屏再切回卡片，视觉上「闪一下」。
 */
const hasSnapshot = computed(() => projects.value.length > 0 || statsReady.value)

/**
 * 自动同步快照（拖拽排序 / 删除 / 重命名 / 新建都会改 `projects`，
 * 逐个改动点手写提交容易漏；deep 监听一次开销很小）。
 */
watch(
  projects,
  (v) => dashboard.setProjects([...v]),
  { deep: true, flush: 'sync' },
)
watch(
  teams,
  (v) => dashboard.setTeams([...v]),
  { deep: true, flush: 'sync' },
)

async function load(): Promise<void> {
  if (loading.value) return
  loading.value = true
  loadError.value = null
  try {
    projects.value = await api.getProjects()
    // 项目列表重载后统计同样重取（旧计数可能属于已删项目）
    // 团队加载失败不阻塞首页（分组退化为默认团队兜底组）
    api
      .listTeams()
      .then((list) => (teams.value = list))
      .catch(() => {})
    await loadCounts()
    void loadActivities()
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
    toast.error(t('projectTabs.loadFail'), { message: loadError.value, duration: 6000 })
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  // 有快照：内容原地保留，只做静默刷新（不进入 loading 骨架态）
  if (!hasSnapshot.value) void load()
  else void refreshSilently()
})

/**
 * 静默刷新（返回首页时用）：内容原地保留，失败也不弹错、不清空，
 * 用户看到的是上一次的快照，等数据回来原地更新。
 */
async function refreshSilently(): Promise<void> {
  try {
    const list = await api.getProjects()
    if (Array.isArray(list)) projects.value = list
    api
      .listTeams()
      .then((l) => (teams.value = l))
      .catch(() => {})
    await loadCounts()
    void loadActivities()
  } catch {
    // 静默：保留快照，下一次进入或手动重试再拉
  }
}

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
  patchStat(project.id, { endpoint_count: 0, scenario_count: 0, request_count: 0 })
  // 弹窗里可能行内新建了团队：刷新团队列表让新组立即出现
  api.listTeams().then((list) => (teams.value = list)).catch(() => {})
}

/**
 * 本地增量更新某个项目的统计（新建/导入/复制/删除时不必重跑整条 IPC）。
 * 用「合并进 stats 原始形状」而不是直接改 counts，保证四组计数
 * （接口 / 场景 / 请求 / 成功失败）始终来自同一份快照。
 */
function patchStat(projectId: string, patch: Partial<ProjectStat>): void {
  const cur = stats.value
  stats.value = {
    ...cur,
    counts: { ...cur.counts, [projectId]: patch.endpoint_count ?? cur.counts[projectId] ?? 0 },
    scenarioCounts: {
      ...cur.scenarioCounts,
      [projectId]: patch.scenario_count ?? cur.scenarioCounts[projectId] ?? 0,
    },
    requestCounts: {
      ...cur.requestCounts,
      [projectId]: patch.request_count ?? cur.requestCounts[projectId] ?? 0,
    },
    okCounts: { ...cur.okCounts, [projectId]: patch.ok_count ?? cur.okCounts[projectId] ?? 0 },
    failCounts: { ...cur.failCounts, [projectId]: patch.fail_count ?? cur.failCounts[projectId] ?? 0 },
    latest: { ...cur.latest, [projectId]: cur.latest[projectId] ?? null },
  }
}

/** 从本地统计里剔除项目（删除项目后首页计数立即对齐）。 */
function dropStat(projectId: string): void {
  const cur = stats.value
  const without = <T>(m: Record<string, T>): Record<string, T> => {
    const { [projectId]: _gone, ...rest } = m
    return rest
  }
  stats.value = {
    counts: without(cur.counts),
    scenarioCounts: without(cur.scenarioCounts),
    requestCounts: without(cur.requestCounts),
    okCounts: without(cur.okCounts),
    failCounts: without(cur.failCounts),
    latest: without(cur.latest),
  }
}

// ---------- 进入工作区 ----------
/** 正在进入中的项目 id：双击 / 连点卡片时只发一次切换，其余忽略。 */
const enteringId = ref<string | null>(null)

/** 点击卡片 / 活动 → 工作区；带定位参数时交给工作区消费（见 WorkspaceView）。 */
async function enter(
  project: Project,
  focus?: { endpointId?: string; historyId?: string },
): Promise<void> {
  if (enteringId.value) return
  enteringId.value = project.id
  try {
    // 走 store 切换：当前项目 UI 态入快照，目标项目加入顶栏标签
    // （成功不弹 toast：跳转到工作区本身已是明确反馈）
    await workspace.switchProject(project.id)
    const query: Record<string, string> = {}
    if (focus?.endpointId) query.focus = focus.endpointId
    if (focus?.historyId) query.history = focus.historyId
    await router.push({ path: '/workspace', query })
  } catch (e) {
    toast.error(t('projectlist.enterFail'), { message: e instanceof Error ? e.message : String(e), duration: 6000 })
  } finally {
    enteringId.value = null
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
  patchStat(project.id, { endpoint_count: 0, scenario_count: 0, request_count: 0 })
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
/** 已放手并在读取文件：Dropzone 显示「正在导入」而不是闪回普通态。 */
const dropBusy = ref(false)
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
  dropBusy.value = true
  try {
    droppedText.value = await api.readTextFile(path)
    showImport.value = true
  } catch (e) {
    toast.error(t('projectlist.readFileFail'), { message: e instanceof Error ? e.message : String(e), duration: 5000 })
  } finally {
    dropBusy.value = false
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
    patchStat(copy.id, { endpoint_count: 0, scenario_count: 0, request_count: 0 })
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
  dropStat(id)
  // 活动里指向已删项目的行同步移除，避免留下点进去「项目已不存在」的死行
  activities.value = activities.value.filter((a) => a.project_id !== id)
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
            <!-- 卡片 1：Hero 欢迎语 + 真实统计（可点击跳转） -->
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
                <!--
                  四个数字都是跨项目汇总，首页没有「按维度看明细」的对应物，
                  故一律不可点击（做成按钮只会把用户带进无关项目）。
                  title 保留为悬停说明，解释数字口径。
                -->
                <div class="hero-stats">
                  <div class="hero-stat" :title="t('projectlist.statProjectsHint')">
                    <span class="hero-value num">{{ statText(totalProjects, false) }}</span>
                    <span class="hero-label"><Icon name="folder" :size="12" /> {{ t('projectlist.totalProjects') }}</span>
                  </div>
                  <span class="hero-divider" aria-hidden="true"></span>
                  <div class="hero-stat" :title="t('projectlist.statApisHint')">
                    <span class="hero-value num">{{ statText(totalApis) }}</span>
                    <span class="hero-label"><Icon name="plug" :size="12" /> {{ t('projectlist.totalApis') }}</span>
                  </div>
                  <span class="hero-divider" aria-hidden="true"></span>
                  <div class="hero-stat" :title="t('projectlist.statScenariosHint')">
                    <span class="hero-value num">{{ statText(totalScenarios) }}</span>
                    <span class="hero-label"><Icon name="play" :size="12" /> {{ t('projectlist.totalScenarios') }}</span>
                  </div>
                  <span class="hero-divider" aria-hidden="true"></span>
                  <div
                    class="hero-stat"
                    :title="t('projectlist.statRequestsHint', { ok: requestBreakdown.ok, fail: requestBreakdown.fail })"
                  >
                    <span class="hero-value num">{{ statText(totalRequests) }}</span>
                    <span class="hero-label"><Icon name="zap" :size="12" /> {{ t('projectlist.totalRequests') }}</span>
                  </div>
                </div>
              </div>
            </div>

            <!-- 卡片 2：最近活动（真实请求历史：方法 / 项目 / 接口 / 状态 / 耗时 / 时间） -->
            <div class="stat-card">
              <span class="stat-icon tint-violet"><Icon name="history" :size="16" /></span>
              <div class="stat-body">
                <div class="stat-head">
                  <span class="stat-label">{{ t('projectlist.recentActivity') }}</span>
                  <button
                    v-if="activitiesState === 'error'"
                    class="stats-retry"
                    type="button"
                    @click="loadActivities"
                  >
                    <Icon name="refresh" :size="11" /> {{ t('projectlist.activityRetry') }}
                  </button>
                </div>
                <!-- 三态：加载中给骨架，失败给重试，成功后才谈「暂无活动」 -->
                <div v-if="activitiesState === 'loading'" class="tl-skeleton" aria-busy="true">
                  <Skeleton v-for="i in 3" :key="i" width="100%" height="30px" radius="8px" />
                </div>
                <div v-else-if="recentActivities.length" class="timeline">
                  <button
                    v-for="a in recentActivities"
                    :key="a.id"
                    class="tl-item"
                    type="button"
                    :title="a.url"
                    @click="enterActivity(a)"
                  >
                    <span class="tl-badge" :class="methodBadgeTone(actTone(a))">{{ actBadgeText(a) }}</span>
                    <span class="tl-main">
                      <!-- 第 1 行：项目名（左）+ 状态 / 耗时 / 时间（右） -->
                      <span class="tl-line">
                        <span class="tl-name">{{ a.project_name }}</span>
                        <span class="tl-meta">
                          <span class="tl-status" :class="statusCls(a.status)">{{ a.status ?? '—' }}</span>
                          <span v-if="a.duration_ms != null" class="tl-dur num">{{ formatDurationShort(a.duration_ms) }}</span>
                          <span class="tl-time">{{ timeAgo(a.created_at) }}</span>
                        </span>
                      </span>
                      <!-- 第 2 行：接口名 + 路径（路径去掉域名只留 pathname） -->
                      <span class="tl-path mono">
                        <span v-if="a.endpoint_name" class="tl-epname">{{ a.endpoint_name }}</span>
                        <span class="tl-url">{{ activityPath(a) }}</span>
                      </span>
                    </span>
                  </button>
                </div>
                <span v-else-if="activitiesState === 'error'" class="stat-sub">
                  {{ t('projectlist.activityLoadFail') }}
                </span>
                <span v-else class="stat-sub">{{ t('projectlist.noActivity') }}</span>
              </div>
            </div>

            <!-- 卡片 3：快速开始（紧凑 2×2 操作入口，四按钮同尺寸同对齐） -->
            <div class="stat-card">
              <span class="stat-icon tint-amber"><Icon name="zap" :size="16" /></span>
              <div class="stat-body">
                <span class="stat-label">{{ t('projectlist.quickStart') }}</span>
                <div class="quick-grid">
                  <button class="quick-btn primary" type="button" :title="t('projectlist.scratchHint')" @click="showScratch = true">
                    <Icon name="send" :size="13" /> {{ t('projectlist.quickRequest') }}
                  </button>
                  <button class="quick-btn ghost" type="button" :title="t('projectlist.newProject')" @click="showCreate = true">
                    <Icon name="plus" :size="13" /> {{ t('projectlist.createProject') }}
                  </button>
                  <button
                    class="quick-btn ghost"
                    type="button"
                    :title="t('projectlist.importHint')"
                    @click="droppedText = ''; showImport = true"
                  >
                    <Icon name="download" :size="13" /> {{ t('projectlist.importProject') }}
                  </button>
                  <button class="quick-btn ghost" type="button" :title="t('pnav.realtimeHint')" @click="router.push('/realtime')">
                    <Icon name="globe" :size="13" /> {{ t('pnav.realtime') }}
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
                  ref="searchEl"
                  v-model="search"
                  class="toolbar-filter-input"
                  :placeholder="t('projectlist.filterPh')"
                  :aria-label="t('projectlist.filterPh')"
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
                <kbd v-else class="toolbar-kbd" aria-hidden="true">⌘K</kbd>
              </div>
              <CustomSelect
                v-model="teamFilter"
                class="toolbar-team"
                size="sm"
                :options="teamFilterOptions"
                pop-auto-width
              />
              <CustomSelect
                v-model="sortBy"
                class="toolbar-sort"
                size="sm"
                :options="sortOptions"
                pop-auto-width
              />
            </div>
            <span class="toolbar-count num" :class="{ filtered: hasActiveFilters }">{{ resultSummary }}</span>
            <div class="toolbar-right">
              <div class="view-toggle" role="group" :aria-label="t('projectlist.viewToggle')">
                <button
                  class="vt-btn"
                  :class="{ active: viewMode === 'grid' }"
                  type="button"
                  :title="t('projectlist.viewGrid')"
                  :aria-pressed="viewMode === 'grid'"
                  @click="viewMode = 'grid'"
                >
                  <Icon name="layout-grid" :size="14" />
                </button>
                <button
                  class="vt-btn"
                  :class="{ active: viewMode === 'list' }"
                  type="button"
                  :title="t('projectlist.viewList')"
                  :aria-pressed="viewMode === 'list'"
                  @click="viewMode = 'list'"
                >
                  <Icon name="list" :size="14" />
                </button>
              </div>
              <button v-if="hasActiveFilters" class="clear-btn" type="button" @click="closeFilters">
                <Icon name="x" :size="12" /> {{ t('projectlist.clearFilters') }}
              </button>
              <button class="team-mgr-btn" type="button" @click="showTeamMgr = true">
                <Icon name="users" :size="14" /> {{ t('team.manage') }}
              </button>
              <button class="btn-new" type="button" @click="showCreate = true">
                <Icon name="plus" :size="14" /> {{ t('projectlist.newProject') }}
              </button>
            </div>
          </section>

          <div v-if="filtered.length" ref="groupsEl" class="team-groups">
            <section
              v-for="g in grouped"
              :key="g.team.id"
              class="team-group"
              :class="{ collapsed: isGroupCollapsed(g.team.id) }"
            >
              <header class="team-divider">
                <div class="team-divider-left">
                  <button
                    class="team-collapse"
                    type="button"
                    :title="isGroupCollapsed(g.team.id) ? t('projectlist.expandGroup') : t('projectlist.collapseGroup')"
                    :aria-expanded="!isGroupCollapsed(g.team.id)"
                    @click="toggleTeam(g.team.id)"
                  >
                    <Icon name="chevron-down" :size="13" />
                  </button>
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
              <!-- 网格模式：卡片拖拽（仅手动排序） -->
              <div
                v-if="viewMode === 'grid' && !isGroupCollapsed(g.team.id)"
                class="card-grid team-grid"
                :data-team-id="g.team.id"
              >
                <ProjectCard
                  v-for="p in g.projects"
                  :key="p.id"
                  :project="p"
                  :count="counts[p.id] ?? 0"
                  :scenario-count="scenarioCounts[p.id] ?? 0"
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
              <!-- 列表模式：紧凑行（名称 / API 数 / 场景数 / 更新时间 / 状态） -->
              <div
                v-else-if="viewMode === 'list' && !isGroupCollapsed(g.team.id)"
                class="team-list"
                :data-team-id="g.team.id"
              >
                <ProjectListRow
                  v-for="p in g.projects"
                  :key="p.id"
                  :project="p"
                  :count="counts[p.id] ?? 0"
                  :scenario-count="scenarioCounts[p.id] ?? 0"
                  :active="isActive(p)"
                  :menu-open="menuOpenId === p.id"
                  @open="onCardOpen(p)"
                  @toggle-menu="toggleMenu(p.id)"
                  @rename="openRename(p)"
                  @duplicate="duplicate(p)"
                  @delete="openDelete(p)"
                />
              </div>
              <p v-if="!g.projects.length && !isGroupCollapsed(g.team.id)" class="team-empty">
                {{ t('projectlist.teamNoProjects') }}
              </p>
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

          <!-- Dropzone（有项目时才出现）与空态互斥：空列表下不给两条并排空盒 -->
          <div
            v-if="!loading && filtered.length"
            ref="dropEl"
            class="add-card"
            :class="{ 'drop-active': dropActive, busy: dropBusy }"
            role="button"
            tabindex="0"
            :aria-label="t('projectlist.dropzoneAria')"
            :aria-busy="dropBusy"
            @click="droppedText = ''; showImport = true"
            @keydown.enter="droppedText = ''; showImport = true"
          >
            <span class="add-icon">
              <Icon :name="dropActive ? 'download' : 'folder-plus'" :size="20" />
            </span>
            <p class="add-text">
              <template v-if="dropBusy">{{ t('projectlist.dropReading') }}</template>
              <template v-else-if="dropActive">{{ t('projectlist.dropActiveHint') }}</template>
              <template v-else>{{ t('projectlist.dropHint') }}</template>
            </p>
            <button
              v-if="!dropActive && !dropBusy"
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
/* 窄窗口收纳 tagline，给项目标签让位 */
@media (max-width: 1200px) {
  .top-tag {
    display: none;
  }
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

/* 统一左右内边距；纵向节奏由 flex gap 统一控制（不允许横向滚动）。
   窄窗口收紧内边距，把宽度让给卡片（卡片列数由 --card-min 与容器宽度算出）。 */
.dash-main {
  --dash-pad: 28px;
  --card-min: 290px;
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 24px var(--dash-pad) 32px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  scroll-behavior: smooth;
}
@media (max-width: 1280px) {
  .dash-main {
    --dash-pad: 22px;
  }
}
@media (max-width: 1024px) {
  .dash-main {
    --dash-pad: 16px;
  }
}
@media (max-width: 720px) {
  .dash-main {
    --dash-pad: 12px;
    --card-min: 240px;
  }
}

/* ---------- 摘要卡片（≥1560px 三列；以下两列 + 快速开始通栏） ----------
   欢迎卡 4 项统计约需 300px 内容宽：三列布局要到内容 ≥470px/列才放得下，
   即窗口 ≥1560（1920/1680 三列；1440/1280/1024 两列 + 通栏快速开始）。 */
.summary-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
}
@media (max-width: 1560px) {
  .summary-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .summary-grid > :last-child {
    grid-column: 1 / -1;
  }
}
@media (max-width: 860px) {
  .summary-grid {
    grid-template-columns: 1fr;
  }
}

/* 卡片统一：圆角 / 描边 / 无渐变无投影（hover 仅提亮描边） */
.stat-card {
  display: flex;
  align-items: flex-start;
  gap: 14px;
  padding: 18px 20px;
  border-radius: var(--radius-xl);
  border: 1px solid var(--border);
  background: var(--bg-panel);
  transition: border-color var(--dur) var(--ease);
}
.stat-card:hover {
  border-color: var(--border-strong);
}

/* 图标底座：圆角方块 + 分色微光（indigo / violet / amber 三档） */
.stat-icon {
  width: 38px;
  height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-lg);
  background: var(--accent-tint);
  color: var(--accent);
}
.stat-icon.tint-indigo {
  background: color-mix(in srgb, var(--accent) 13%, transparent);
  color: var(--accent-hover);
}
.stat-icon.tint-violet {
  background: var(--patch-tint);
  color: var(--patch);
}
.stat-icon.tint-amber {
  background: var(--warning-tint);
  color: var(--warning);
}

.stat-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

/* ---- Hero 欢迎语 + 四项统计 ---- */
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
  align-items: stretch;
  gap: 10px;
  margin-top: 12px;
}

/* 统计为纯展示：跨项目汇总数没有对应明细页，不可点击（不做按钮，避免误认为入口） */
.hero-stat {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  padding: 2px 4px;
}

.hero-divider {
  width: 1px;
  align-self: stretch;
  background: var(--border);
  flex-shrink: 0;
}

/* 大数字：实色（skill：禁止渐变文本做大标题） */
.hero-value {
  font-family: var(--font-mono);
  font-size: 24px;
  font-weight: 700;
  line-height: 1.15;
  font-variant-numeric: tabular-nums;
  color: var(--text-1);
}

.hero-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xxs);
  color: var(--text-2);
  white-space: nowrap;
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

/* 卡片标题行：标题左，状态/重试右 */
.stat-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 16px;
}

.stat-sub {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* ---- 最近活动：时间线 + Method Badge（真实请求历史） ---- */
.timeline {
  display: flex;
  flex-direction: column;
  margin-top: 6px;
}

/* 加载中：与行同高的骨架条，避免卡片高度跳变 */
.tl-skeleton {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
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
.tl-item:hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}

/* Method Badge：布局尺寸，颜色走共享 methodBadgeTone（未知方法兜底中性） */
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

/* 右侧元信息：状态码 / 耗时 / 时间 */
.tl-meta {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 状态码语义色（真实请求结果，非模拟） */
.tl-status {
  min-width: 24px;
  text-align: center;
  padding: 1px 5px;
  border-radius: var(--radius-sm);
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  background: var(--bg-hover);
  color: var(--text-2);
}
.tl-status.st-ok {
  color: var(--success);
  background: var(--success-tint);
}
.tl-status.st-warn {
  color: var(--warning);
  background: var(--warning-tint);
}
.tl-status.st-err {
  color: var(--danger);
  background: var(--danger-tint);
}

.tl-dur {
  font-variant-numeric: tabular-nums;
}

.tl-time {
  flex-shrink: 0;
}

/* 第 2 行：接口名（弱化）+ 路径（等宽），超长走省略号 */
.tl-path {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.4;
}

.tl-epname {
  flex-shrink: 0;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-2);
}

/* 分隔点在接口名与路径之间；接口名缺失时路径前不出现孤立的「·」 */
.tl-epname + .tl-url::before {
  content: '·';
  margin-right: 6px;
  color: var(--border-strong);
}

.tl-url {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
}

/* ---- 快速开始：紧凑 2×2 操作入口 ---- */
.quick-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  margin-top: 8px;
}

.quick-btn {
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

/* 次要动作：中性描边样式 */
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
  border-radius: var(--radius-xl);
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

/* 读取文件中：保留描边提示，点击暂不生效（避免重复打开导入弹窗） */
.add-card.busy {
  border-style: solid;
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  cursor: progress;
}
.add-card.busy .add-icon,
.add-card.busy .add-text {
  color: var(--accent);
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

/* ---------- 工具栏：搜索 / 分组 / 排序 | 计数 | 视图切换 / 清空 / 团队 / 新建 ---------- */
.toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

/* 结果统计：无筛选弱化展示；筛选中高亮提示当前是子集 */
.toolbar-count {
  font-size: var(--fs-xxs);
  font-family: var(--font-mono);
  color: var(--text-3);
  white-space: nowrap;
}
.toolbar-count.filtered {
  color: var(--accent);
}

/* 视图切换：分段控件（网格 / 列表） */
.view-toggle {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
}

.vt-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: var(--radius-md);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.vt-btn:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.vt-btn.active {
  background: var(--accent-tint);
  color: var(--accent);
}

.clear-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 32px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  color: var(--text-2);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.clear-btn:hover {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  background: var(--danger-tint);
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

/* ⌘K 提示：输入框空时占位显示快捷键，聚焦时隐藏避免抢注意力 */
.toolbar-kbd {
  flex-shrink: 0;
  margin-left: auto;
  padding: 1px 5px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: 10px;
  line-height: 1.4;
}
.toolbar-filter:focus-within .toolbar-kbd {
  display: none;
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

/* 分组 / 排序下拉：统一 36px 触发高度 */
.toolbar-team,
.toolbar-sort {
  width: 148px;
}
.toolbar-team :deep(.cs-trigger),
.toolbar-sort :deep(.cs-trigger) {
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
/* auto-fill + minmax(var(--card-min), 1fr)：列数按**内容区实际宽度**决定
   （--dash-pad 随窗口收紧，见 .dash-main）：宽屏 4 列、普通桌面 3 列、
   中等窗口 2 列、小屏 1 列。轨道下限用变量而非 auto，避免卡片内 nowrap
   长内容把轨道撑出容器（曾导致横向滚动条、卡片被切出屏幕）。 */
.card-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(var(--card-min, 290px), 1fr));
  gap: 16px;
}

/* ---------- 列表模式：紧凑行 ---------- */
.team-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
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

/* 分组折叠按钮：chevron 旋转反馈 */
.team-collapse {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.team-collapse:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.team-collapse svg {
  transition: transform var(--dur) var(--ease);
}
.team-group.collapsed .team-collapse svg {
  transform: rotate(-90deg);
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
  grid-template-columns: repeat(auto-fill, minmax(var(--card-min, 290px), 1fr));
  gap: 16px;
}
.sk-card {
  display: flex;
  align-items: flex-start;
  gap: 14px;
  padding: 18px 20px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xl);
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
