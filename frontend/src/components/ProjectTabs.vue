<script setup lang="ts">
/**
 * ProjectTabs：顶栏多项目标签条（工作区 / 项目首页共用）。
 * - 点击标签：切换项目并进入工作区（首页点击同样跳转）；
 * - × 关闭标签（快照丢弃；关闭当前项目时 store 自动切相邻标签）；
 * - ⋯ 菜单按三组固定分隔：未打开的项目（点击打开并切换）｜新建项目 + 项目列表｜
 *   当前项目的重命名（短标签）/ 删除（工作区顶栏用，替代原侧栏头部 ⋯）。
 */
import { ref, withDefaults } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import Icon from './ui/Icon.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'

const props = withDefaults(defineProps<{ projectActions?: boolean }>(), { projectActions: false })

const emit = defineEmits<{
  'new-project': []
  'rename-project': []
  'delete-project': []
}>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const router = useRouter()
const route = useRoute()

const menuRef = ref<InstanceType<typeof Menu> | null>(null)
const moreBtn = ref<HTMLButtonElement | null>(null)

async function openMore(): Promise<void> {
  if (!moreBtn.value) return
  let items: MenuItem[] = []
  try {
    const openIds = new Set(store.openProjects.map((t) => t.id))
    items = (await api.getProjects())
      .filter((p) => !openIds.has(p.id))
      .map((p) => ({
        key: `switch-project:${p.id}`,
        label: p.name,
        icon: 'folder' as const,
      }))
  } catch {
    toast.error(t('projectTabs.loadFail'))
  }
  // 第一组：切换到未打开的项目（无则整组省略）；第二组固定组前分隔
  items.push(
    { key: 'new-project', label: t('projectTabs.newProject'), icon: 'plus', iconAccent: true, dividerBefore: items.length > 0 },
    { key: 'go-projects', label: t('projectTabs.projectList'), icon: 'layout-grid' },
  )
  // 第三组（工作区模式）：当前项目管理——短标签重命名 + 红色删除（二次确认）
  if (props.projectActions && store.project) {
    const name = store.project.name
    items.push(
      { key: 'rename-project', label: t('projectTabs.rename'), icon: 'pencil', dividerBefore: true },
      { key: 'delete-project', label: t('projectTabs.delete'), icon: 'trash', danger: true, confirm: t('projectTabs.deleteConfirm', { name }) },
    )
  }
  menuRef.value?.openAt(moreBtn.value, items, 'left')
}

function onSelect(item: MenuItem): void {
  if (item.key.startsWith('switch-project:')) void switchTo(item.key.slice('switch-project:'.length))
  else if (item.key === 'new-project') emit('new-project')
  else if (item.key === 'go-projects') router.push('/projects')
  else if (item.key === 'rename-project') emit('rename-project')
}

async function switchTo(projectId: string): Promise<void> {
  try {
    await store.switchProject(projectId)
    // 标签高亮 + 内容切换已是充分反馈，成功不弹 toast（避免频繁切换被打扰）
    if (route.path !== '/workspace') router.push('/workspace')
  } catch (err) {
    toast.error(t('projectTabs.switchFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function onTabClick(projectId: string): void {
  void switchTo(projectId)
}

function onClose(projectId: string): void {
  store.closeProjectTab(projectId)
}

// ---------- 横拖平移：项目标签条溢出后按住左右拖动查看被挡住的标签 ----------
// 顶栏可用宽度有限（右侧 Docs/Mock/环境/工具组按 min-width:max-content 不压缩），
// 项目一多 .proj-tabs 就被压窄、后面的项目被挡住。此前只有滚轮与触控板能滚，
// 鼠标用户没有任何可见入口——看起来就是「拖不动」（v0.0.57 用户实测）。
// 与 TabBar 同款手势：默认拖 = 平移，指到关闭钮不接管。
const tabsEl = ref<HTMLElement | null>(null)
const DRAG_THRESHOLD = 4

const panning = ref(false)
let panPointerId: number | null = null
let panStartX = 0
let panStartScroll = 0
let panMoved = false
/** 曾拖拽过：吞掉紧随的 click，避免「拖完顺手切了项目」。 */
let panSwallowClick = false

function onTabsPointerDown(event: PointerEvent): void {
  if (event.button !== 0) return
  const el = tabsEl.value
  if (!el || el.scrollWidth <= el.clientWidth) return
  if ((event.target as HTMLElement | null)?.closest('button, input, select, textarea')) return
  panPointerId = event.pointerId
  panStartX = event.clientX
  panStartScroll = el.scrollLeft
  panMoved = false
  panning.value = true
  el.setPointerCapture(event.pointerId)
  // 捕获失败或指针移出标签条时元素收不到 pointerup，panning 会永久卡住
  window.addEventListener('pointerup', stopPan)
  window.addEventListener('pointercancel', stopPan)
}

function onTabsPointerMove(event: PointerEvent): void {
  if (!panning.value || event.pointerId !== panPointerId) return
  const el = tabsEl.value
  if (!el) return
  const dx = event.clientX - panStartX
  if (!panMoved) {
    if (Math.abs(dx) < DRAG_THRESHOLD) return
    // 越过阈值时重锚起点（同 TabBar）：不重锚会把阈值内的位移一次性补上，
    // 首个 pointermove 直接跳 4px，手感是「先是死的、然后突然挣脱」。
    panStartX = event.clientX
    panStartScroll = el.scrollLeft
    panMoved = true
    return
  }
  el.scrollLeft = panStartScroll - dx
}

function stopPan(): void {
  const el = tabsEl.value
  if (panPointerId !== null && el?.hasPointerCapture?.(panPointerId)) {
    el.releasePointerCapture(panPointerId)
  }
  panPointerId = null
  panning.value = false
  window.removeEventListener('pointerup', stopPan)
  window.removeEventListener('pointercancel', stopPan)
}

function onTabsPointerUp(event: PointerEvent): void {
  if (event.pointerId !== panPointerId) return
  panSwallowClick = panMoved
  stopPan()
}

/** 拖拽结束时浏览器仍会派一次 click：吞掉它，避免误切项目。 */
function onTabsClickCapture(event: MouseEvent): void {
  if (!panSwallowClick) return
  panSwallowClick = false
  event.preventDefault()
  event.stopPropagation()
}

/**
 * 竖向滚轮转横向平移：标签条只横滚，鼠标滚轮（deltaY）原生无事可做。
 * 触控板的横向分量（deltaX 占优）仍走原生滚动，不劫持。
 */
function onTabsWheel(event: WheelEvent): void {
  const el = tabsEl.value
  if (!el || el.scrollWidth <= el.clientWidth) return
  if (Math.abs(event.deltaY) < Math.abs(event.deltaX)) return
  event.preventDefault()
  el.scrollLeft += event.deltaY
}
</script>

<template>
  <div class="ptabs">
    <div
      ref="tabsEl"
      class="proj-tabs"
      :class="{ panning }"
      @pointerdown="onTabsPointerDown"
      @pointermove="onTabsPointerMove"
      @pointerup="onTabsPointerUp"
      @pointercancel="onTabsPointerUp"
      @click.capture="onTabsClickCapture"
      @wheel="onTabsWheel"
    >
      <div
        v-for="tab in store.openProjects"
        :key="tab.id"
        class="proj-tab"
        :class="{ active: tab.id === store.project?.id }"
        :title="tab.name"
        @click="onTabClick(tab.id)"
      >
        <span class="pt-name">{{ tab.name }}</span>
        <button
          class="pt-close"
          type="button"
          :title="t('projectTabs.closeTab', { name: tab.name })"
          :aria-label="t('projectTabs.closeTab', { name: tab.name })"
          @click.stop="onClose(tab.id)"
        >
          <Icon name="x" :size="11" />
        </button>
      </div>
    </div>
    <button
      ref="moreBtn"
      class="proj-more"
      type="button"
      :title="t('projectTabs.more')"
      :aria-label="t('projectTabs.more')"
      @click="openMore"
    >
      <Icon name="more-horizontal" :size="14" />
    </button>
  </div>
  <Menu ref="menuRef" @select="onSelect" @confirm="emit('delete-project')" />
</template>

<style scoped>
.ptabs {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1 1 auto;
}

.proj-tabs {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.proj-tabs::-webkit-scrollbar {
  display: none;
}
/* 拖动平移中：切抓手光标、关掉标签过渡（跟手不拖影）、禁文本选中。
   仅在拖拽进行时生效，常态零影响。 */
.proj-tabs.panning {
  cursor: grabbing;
  user-select: none;
}
.proj-tabs.panning .proj-tab {
  transition: none;
}

.proj-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 180px;
  padding: 3px 8px 3px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius);
  font-size: 12.5px;
  color: var(--text-2);
  cursor: pointer;
  user-select: none;
  /* WebKit（Tauri 内核）按住可拖拽元素会启动原生 drag 并派 pointercancel
   * 掐断平移，表现为「拖着没反应」。标签只承载点击与横拖，不该可拖拽。 */
  -webkit-user-drag: none;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.proj-tab:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
/* 当前项目：accent 淡底 + 描边 + 左侧圆点，与其他已打开标签明确区分 */
.proj-tab.active {
  background: color-mix(in srgb, var(--accent) 12%, var(--bg-elevated));
  border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  color: var(--text-1);
  font-weight: 600;
}
.proj-tab.active::before {
  content: '';
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 4px var(--accent);
}

.pt-name {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.pt-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  flex-shrink: 0;
}
.pt-close:hover {
  background: var(--bg-active);
  color: var(--text-1);
}
.proj-tab:not(:hover) .pt-close {
  opacity: 0;
}
.proj-tab.active .pt-close {
  opacity: 1;
}

.proj-more {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: var(--radius);
  background: none;
  color: var(--text-2);
  cursor: pointer;
  flex-shrink: 0;
}
.proj-more:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
</style>
