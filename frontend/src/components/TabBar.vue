<script setup lang="ts">
/**
 * TabBar：打开中的接口标签页。
 * - 每个标签：方法标签（GET 绿 / POST 黄…）+ 截断的接口名；宽 120–200px；
 * - 布局：末尾两个平级按钮——「+」新建、「⋯」标签管理；溢出时吸在
 *   右边缘（Chrome 式，sticky 实现，「+」比「⋯」多让出一枚按钮宽度避免
 *   同位重叠），tab 再多也不会把它们滚丢；
 * - 激活态 = text-1 + 底部主题色下划线；未保存草稿在标题旁显示小圆点，
 *   hover 时圆点被 ✕ 替换（两者不同时出现，避免挤占）；
 * - 关闭按钮 hover 才出现；脏标签关闭走 Popconfirm；
 * - 键盘：容器 role=tablist、页签 role=tab + roving tabindex，←/→ 与
 *   Home/End 切换，Enter/Space 激活，Delete 关闭（脏页签走既有确认）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Sortable from 'sortablejs'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { methodTone } from '../utils/methodTone'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import Popconfirm from './ui/Popconfirm.vue'
import Tooltip from './ui/Tooltip.vue'

const emit = defineEmits<{
  /** 点「+」：由父级在内容区打开新建选择页（独立页面，对标 Apifox）。 */
  create: []
}>()

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

function close(id: string): void {
  store.closeTab(id)
}

/** 中键直接关闭（对标浏览器标签页；mousedown 阶段拦截以抑制中键自动滚动）。
 *  经由关闭按钮同链路：干净页签立即关闭，脏页签弹 Popconfirm 二次确认——
 *  原先中键直连 close()，会绕过未保存确认静默丢草稿。 */
function onTabMouseDown(event: MouseEvent): void {
  if (event.button === 1) {
    event.preventDefault()
    closeViaButton(event.currentTarget as HTMLElement)
  }
}

// ---------- 页签横拖滚动 + Alt+拖拽排序 ----------
// 主操作是「横拖页签条查看被挡住的页签」（浏览器标签栏同款）：默认鼠标拖动
// 平移滚动区，不再触发重排——此前整条滚动区被 Sortable 接管，用户想横拖时
// 实际在重排，手感就是「拉不动」（v0.0.57 用户实测）。
// 排序退到 Alt+拖拽：Sortable 的 filter 在每次拖拽开始时求值，
// 读 event.altKey 即可让「不按 Alt」的拖拽全部被过滤掉。
const tabListEl = ref<HTMLElement | null>(null)
let tabSortable: Sortable | null = null

/** 排序期间松开 Alt 会中途误触发，用按下瞬间的修饰键状态判定更稳。 */
let altHeldOnPress = false

function initTabSortable(): void {
  if (!tabListEl.value || tabSortable) return
  tabSortable = Sortable.create(tabListEl.value, {
    animation: 150,
    easing: 'cubic-bezier(0.2, 0, 0, 1)',
    forceFallback: true,
    fallbackTolerance: 3,
    ghostClass: 'tab-ghost',
    chosenClass: 'tab-chosen',
    draggable: '.tab[data-tab-id]',
    // 关闭钮等交互元素永不参与；不按 Alt 时整个页签都被 filter 挡下，
    // 拖拽交给下面的 pointer 横拖处理。
    filter: (event: Event) => {
      const target = event.target as HTMLElement | null
      if (target?.closest('button, input, select, textarea')) return true
      return !altHeldOnPress
    },
    preventOnFilter: false,
    onEnd: () => {
      const root = tabListEl.value
      if (!root) return
      const ordered = [...root.querySelectorAll<HTMLElement>('.tab[data-tab-id]')]
        .map((el) => el.dataset.tabId)
        .filter((id): id is string => !!id)
      store.reorderTabs(ordered)
    },
  })
}

onMounted(() => nextTick(initTabSortable))
onBeforeUnmount(() => {
  tabSortable?.destroy()
  tabSortable = null
  stopDragPan()
})

// ---------- 横拖平移：按住页签条鼠标拖动左右滚动 ----------
/** 超过该位移才认作拖拽，否则当普通点击（切换页签）处理。 */
const DRAG_THRESHOLD = 4

const panning = ref(false)
/** active 期间临时关闭页签的过渡动画，拖动跟手才不会拖影。 */
let panPointerId: number | null = null
let panStartX = 0
let panStartScroll = 0
let panMoved = false
/** 曾在拖拽中移动过：用于吞掉紧随其后的 click，避免「拖完顺手切了页签」。 */
let panSwallowClick = false

function onListPointerDown(event: PointerEvent): void {
  // 只认鼠标左键 / 触控笔 / 触摸；Alt+左键留给排序
  if (event.button !== 0) return
  altHeldOnPress = event.altKey
  if (event.altKey) return
  const el = tabListEl.value
  if (!el || el.scrollWidth <= el.clientWidth) return
  // 关闭钮等交互元素上按下不进入拖拽
  if ((event.target as HTMLElement | null)?.closest('button, input, select, textarea')) return
  panPointerId = event.pointerId
  panStartX = event.clientX
  panStartScroll = el.scrollLeft
  panMoved = false
  panning.value = true
  el.setPointerCapture(event.pointerId)
}

function onListPointerMove(event: PointerEvent): void {
  if (!panning.value || event.pointerId !== panPointerId) return
  const el = tabListEl.value
  if (!el) return
  const dx = event.clientX - panStartX
  if (!panMoved && Math.abs(dx) < DRAG_THRESHOLD) return
  panMoved = true
  el.scrollLeft = panStartScroll - dx
}

function stopDragPan(): void {
  const el = tabListEl.value
  if (panPointerId !== null && el?.hasPointerCapture?.(panPointerId)) {
    el.releasePointerCapture(panPointerId)
  }
  panPointerId = null
  panning.value = false
}

function onListPointerUp(event: PointerEvent): void {
  if (event.pointerId !== panPointerId) return
  panSwallowClick = panMoved
  stopDragPan()
}

/** 拖拽结束时浏览器仍会派发一次 click：吞掉它，避免误切换页签。 */
function onListClickCapture(event: MouseEvent): void {
  if (!panSwallowClick) return
  panSwallowClick = false
  event.preventDefault()
  event.stopPropagation()
}

// ---------- 激活标签滚动到可视区 ----------
const barEl = ref<HTMLElement | null>(null)

watch(
  () => store.activeTabId,
  async () => {
    await nextTick()
    barEl.value
      ?.querySelector('.tab.active')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  },
)

/**
 * 竖向滚轮转横向平移：页签条只横滚，但 overflow-y:hidden 下鼠标滚轮
 * （deltaY）原生无事可做，用户只能 shift+滚轮或触控板横扫。
 * 页签溢出时把纵向增量接到 scrollLeft 上（Chrome 页签条同款行为）；
 * 触控板的横向分量（deltaX 占优）仍走原生滚动，不劫持。
 */
function onTabWheel(event: WheelEvent): void {
  const el = tabListEl.value
  if (!el || el.scrollWidth <= el.clientWidth) return
  if (Math.abs(event.deltaY) < Math.abs(event.deltaX)) return
  event.preventDefault()
  el.scrollLeft += event.deltaY
}

/**
 * 标签行数据一次求值：原来模板每标签调 methodOf/titleOf/isDirty 3-4 次
 *（每次各自查 drafts/endpoints），此处单遍组装供模板复用。
 */
const tabs = computed(() =>
  store.openTabs.map((id) => ({
    id,
    create: store.isCreateTab(id),
    method: store.draftOf(id)?.method ?? 'GET',
    title: store.titleOf(id),
    dirty: store.isDirty(id),
  })),
)

// ---------- 标签管理菜单：关闭当前 / 其他 / 右侧 / 全部（有未保存时行内二次确认） ----------
// 菜单在触发器右侧展开（left 对齐），贴着 ⋯ 按钮往右下开，避免盖住左侧标签。
const tabMenu = ref<InstanceType<typeof Menu> | null>(null)
const tabMoreEl = ref<HTMLButtonElement | null>(null)

/** 含未保存标签时，二次确认才放行批量关闭（与单标签 Popconfirm 同语义）。 */
function unsavedConfirm(count: number): string {
  return t('tabbar.closeUnsavedConfirm', { n: count })
}

function dirtyCount(ids: string[]): number {
  return ids.filter((id) => store.isDirty(id)).length
}

/** 以某页签为基准的批量关闭菜单项：⋯ 菜单以激活页签为基准；右键菜单以被点页签为基准。 */
function tabMenuItemsFor(targetId: string | null): MenuItem[] {
  const ids = store.openTabs
  const idx = targetId ? ids.indexOf(targetId) : -1
  const others = ids.filter((id) => id !== targetId)
  const rightIds = targetId && idx !== -1 ? ids.slice(idx + 1) : []
  return [
    {
      key: 'close-current',
      label: t('tabbar.closeCurrent'),
      disabled: !targetId,
      ...(targetId && store.isDirty(targetId) ? { confirm: t('tabbar.closeConfirm'), confirmDanger: true } : {}),
    },
    {
      key: 'close-others',
      label: t('tabbar.closeOthers'),
      disabled: others.length === 0,
      ...(dirtyCount(others) > 0 ? { confirm: unsavedConfirm(dirtyCount(others)), confirmDanger: true } : {}),
    },
    {
      key: 'close-right',
      label: t('tabbar.closeRight'),
      disabled: rightIds.length === 0,
      ...(dirtyCount(rightIds) > 0 ? { confirm: unsavedConfirm(dirtyCount(rightIds)), confirmDanger: true } : {}),
    },
    {
      key: 'close-all',
      label: t('tabbar.closeAll'),
      danger: true,
      dividerBefore: true,
      disabled: ids.length === 0,
      ...(dirtyCount(ids) > 0 ? { confirm: unsavedConfirm(dirtyCount(ids)), confirmDanger: true } : {}),
    },
  ]
}

function openTabMenu(): void {
  contextTabId.value = store.activeTabId
  if (tabMoreEl.value) tabMenu.value?.openAt(tabMoreEl.value, tabMenuItemsFor(store.activeTabId), 'left')
}

/** 右键菜单目标：右键的页签（可能是非激活页签），批量动作围绕它展开。 */
const contextTabId = ref<string | null>(null)

function onTabContextMenu(event: MouseEvent, id: string): void {
  event.preventDefault()
  contextTabId.value = id
  tabMenu.value?.openAt(event.currentTarget as HTMLElement, tabMenuItemsFor(id), 'left')
}

function runMenuAction(item: MenuItem): void {
  const target = contextTabId.value ?? store.activeTabId
  if (item.key === 'close-current' && target) {
    store.closeTab(target)
  } else if (item.key === 'close-others' && target) {
    store.closeOtherTabs(target)
  } else if (item.key === 'close-right' && target) {
    store.closeTabsToRight(target)
  } else if (item.key === 'close-all') {
    store.closeAllTabs()
  }
}

function onTabMenuSelect(item: MenuItem): void {
  // 无 confirm 的项直接执行；有 confirm 的走行内确认视图
  runMenuAction(item)
}

function onTabMenuConfirm(item: MenuItem): void {
  // 二次确认通过：强制执行（与 onSelect 同分支）
  runMenuAction(item)
}

// ---------- 页签键盘导航：roving tabindex + ←/→ 切换、Enter/Space 激活、Delete 关闭 ----------
/** 页签容器内的 .tab 列表（DOM 顺序 = 视觉顺序）。 */
function tabElements(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>('.tab')]
}

/** roving tabindex：激活页签可 Tab 进入；无激活项时退到第一个页签。 */
function tabbableId(): string | null {
  const ids = store.openTabs
  if (store.activeTabId && ids.includes(store.activeTabId)) return store.activeTabId
  return ids[0] ?? null
}

/**
 * Delete 关闭当前页签：直接点页签上的关闭按钮——干净页签立即关闭，
 * 脏页签沿用既有 Popconfirm 二次确认（与鼠标点击完全同链路）。
 */
function closeViaButton(row: HTMLElement): void {
  row.querySelector<HTMLButtonElement>('.tab-close')?.click()
}

function onTabListKeydown(event: KeyboardEvent): void {
  const container = event.currentTarget as HTMLElement
  const rows = tabElements(container)
  if (!rows.length) return
  const current = (event.target as HTMLElement).closest<HTMLElement>('.tab')
  const index = current ? rows.indexOf(current) : -1
  if (index === -1) return

  if (event.key === 'Delete') {
    // 焦点已在关闭按钮上时不重复触发（按钮自身 Enter/Space 即关闭）
    if ((event.target as HTMLElement).closest('button')) return
    event.preventDefault()
    closeViaButton(rows[index])
    return
  }

  const NAV_KEYS = ['ArrowLeft', 'ArrowRight', 'Home', 'End']
  if (!NAV_KEYS.includes(event.key)) return
  event.preventDefault()

  let next = index
  if (event.key === 'ArrowRight') next = (index + 1) % rows.length
  else if (event.key === 'ArrowLeft') next = (index - 1 + rows.length) % rows.length
  else if (event.key === 'Home') next = 0
  else next = rows.length - 1

  const row = rows[next]
  const id = row.dataset.tabId
  if (id !== undefined && id !== store.activeTabId) store.activeTabId = id
  row.focus()
}

/** Enter / Space：div 页签无原生激活，补齐与鼠标点击等价的激活语义。 */
function onTabActivateKey(event: KeyboardEvent, id: string): void {
  if (event.key !== 'Enter' && event.key !== ' ' && event.key !== 'Spacebar') return
  event.preventDefault()
  store.activeTabId = id
}
</script>

<template>
  <div ref="barEl" class="tab-bar">
    <div
      ref="tabListEl"
      class="tab-scroll"
      :class="{ panning }"
      role="tablist"
      @keydown="onTabListKeydown"
      @wheel="onTabWheel"
      @pointerdown="onListPointerDown"
      @pointermove="onListPointerMove"
      @pointerup="onListPointerUp"
      @pointercancel="onListPointerUp"
      @click.capture="onListClickCapture"
    >
      <div
        v-for="tab in tabs"
        :key="tab.id"
        class="tab"
        :class="{ active: store.activeTabId === tab.id }"
        role="tab"
        :data-tab-id="tab.id"
        :aria-selected="store.activeTabId === tab.id"
        :tabindex="tabbableId() === tab.id ? 0 : -1"
        @click="store.activeTabId = tab.id"
        @mousedown="onTabMouseDown($event)"
        @keydown="onTabActivateKey($event, tab.id)"
        @contextmenu="onTabContextMenu($event, tab.id)"
      >
        <!-- 「新建」伪页签：无方法徽章，闪电指代“待创建” -->
        <span v-if="tab.create" class="tab-spark" aria-hidden="true"><Icon name="zap" :size="12" /></span>
        <span v-else class="method-tag" :class="methodTone(tab.method)">{{ tab.method }}</span>
        <span class="tab-title" v-tooltip-overflow="tab.title">{{ tab.title }}</span>
        <span v-if="tab.dirty" class="tab-dirty" :title="t('tabbar.unsaved')"><Icon name="dot" :size="8" /></span>
        <Popconfirm
          v-if="tab.dirty"
          :title="t('tabbar.closeConfirm')"
          @confirm="close(tab.id)"
        >
          <IconButton class="tab-close" name="x" :size="12" :title="t('common.close')" />
        </Popconfirm>
        <IconButton v-else class="tab-close" name="x" :size="12" :title="t('common.close')" @click.stop="close(tab.id)" />
      </div>
      <Tooltip :content="t('tabbar.newRequestHint')">
        <button
          class="tab-add"
          type="button"
          :aria-label="t('tabbar.newRequest')"
          @click="emit('create')"
        >
          <Icon name="plus" :size="15" />
        </button>
      </Tooltip>
      <!-- 标签页管理：独立「⋯」按钮（不再与「+」共用一枚胶囊）。
           原设计把 + 与 ⌄ 拼成一个组合按钮，下拉只占 16px、箭头又像在
           「展开 +」，用户第一眼读不出这里藏的是「关闭其他 / 关闭全部」。
           拆成两个平级按钮后，+ 只管新建、⋯ 只管管理，各司其职。 -->
      <button
        ref="tabMoreEl"
        class="tab-add tab-more"
        type="button"
        :title="t('tabbar.tabMenu')"
        :aria-label="t('tabbar.tabMenu')"
        aria-haspopup="menu"
        @click="openTabMenu"
      >
        <Icon name="more-horizontal" :size="15" :stroke-width="1.5" />
      </button>
    </div>
    <Menu ref="tabMenu" @select="onTabMenuSelect" @confirm="onTabMenuConfirm" />
  </div>
</template>

<style scoped>
.tab-bar {
  display: flex;
  gap: 2px;
  padding: 6px 8px 0;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  flex-shrink: 0;
}

/* 内层滚动区：只有标签横滚。
   滚动条必须自己画出来——macOS 叠加式滚动条会盖在页签下半截上、
   经典式（系统「始终显示」/Windows）则挤占行高把 32px 页签压扁，
   多开几枚页签后整条看起来像断裂（v0.0.56 用户实测）。
   与 ProjectTabs 的 .proj-tabs 同款：藏掉滚动条，平移交给滚轮/触控板。 */
.tab-scroll {
  display: flex;
  gap: 2px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
}
.tab-scroll::-webkit-scrollbar {
  display: none;
}

/* 横拖平移中：关掉页签过渡动画（跟手不拖影）、禁掉文本选中、
   光标切成抓手；松手恢复。仅在拖拽进行时生效，常态零影响。 */
.tab-scroll.panning {
  cursor: grabbing;
  user-select: none;
}
.tab-scroll.panning .tab {
  transition: none;
}

/* 新建「+」/ 管理「⋯」跟在末尾 tab 后面；溢出时吸在右边缘（Chrome 式），永远可见 */
.tab-scroll > .tt-trigger {
  position: sticky;
  /* 比「⋯」（right:0，宽 28px）多让出 30px：两钮都写 right:0 会在页签溢出时
     被钉到同一位置叠成一枚按钮（v0.0.56 用户实测）。
     30 = 「⋯」宽 28 + 间隙 2，吸右后仍保持 [+][⋯] 的并排顺序。 */
  right: 30px;
  z-index: 1;
  flex-shrink: 0;
  align-self: center;
  margin-bottom: 2px;
  /* 与右侧「⋯」之间补实底：两枚按钮各自 sticky，中间那条缝若留空，
     滚过的页签标题会从缝里透出来，看起来像接口名挂在「+」右边
     （v0.0.57 用户实测）。用同底色 padding 填满这条缝，
     再用等宽负 margin 抵消，整体占位不变。 */
  padding-right: 2px;
  background: var(--bg-panel);
  border-radius: 6px 0 0 6px;
}

/* ---- 右侧两个平级动作按钮：+ 新建 / ⋯ 管理 ---- */
.tab-add {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  align-self: center;
  flex-shrink: 0;
  height: 28px;
  width: 28px;
  margin-bottom: 2px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: var(--bg-panel);
  color: var(--accent);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    transform var(--dur) var(--ease);
}
.tab-add:hover {
  background: var(--bg-active);
}
.tab-add:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.tab-add:active {
  transform: scale(0.92);
}

/* 管理按钮：与 + 同尺寸同底，仅图标颜色退到次级，避免与 + 抢焦点 */
.tab-more {
  color: var(--text-2);
  /* 溢出吸右时要盖住下方滚过的标签，需实底；right:0 + 上方 trigger 的
     right:30px 让两钮吸右时错开一枚按钮宽度，不再同位重叠。
     margin-left:-2px 吃掉与「+」之间的缝隙（由 + 的 padding-right 补位），
     两钮的实底连成一片，滚过的页签标题无从透出。 */
  position: sticky;
  right: 0;
  z-index: 1;
  margin-left: -2px;
}
.tab-more:hover {
  color: var(--text-1);
}

.tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  min-width: 120px;
  max-width: 200px;
  padding: 0 4px 0 10px;
  border-radius: var(--radius) var(--radius) 0 0;
  position: relative;
  font-size: var(--fs-sm);
  color: var(--text-2);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.tab:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.tab:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}
.tab.active {
  color: var(--text-1);
  background: var(--bg-app);
}
.tab.active::after {
  content: '';
  position: absolute;
  left: 10px;
  right: 10px;
  bottom: 0;
  height: 2px;
  border-radius: 1px;
  background: var(--accent);
}

/* 方法标签：胶囊布局，颜色走共享 methodTone（utils/methodTone.ts） */
.method-tag {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  line-height: 1.2;
  padding: 3px 6px;
  border-radius: 999px;
}

/* 「新建」伪页签的闪电：主题色小图标 */
.tab-spark {
  display: inline-flex;
  flex-shrink: 0;
  color: var(--accent);
}

/* 「场景」伪页签：主题色淡底烧瓶胶囊（区别于接口的方法色文字徽章） */

.tab-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 未保存圆点：常态显示，hover 时被 ✕ 顶替 */
.tab-dirty {
  display: inline-flex;
  color: var(--warning);
  flex-shrink: 0;
  transition: opacity var(--dur) var(--ease);
}
.tab:hover .tab-dirty {
  opacity: 0;
}

/* 拖拽排序反馈（Sortable ghost/chosen） */
.tab-ghost {
  opacity: 0.4;
}
.tab-chosen {
  box-shadow: 0 4px 14px rgb(0 0 0 / 0.3);
}

.tab-close {
  width: 20px;
  height: 20px;
  opacity: 0;
  flex-shrink: 0;
  transition:
    opacity var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.tab:hover .tab-close {
  opacity: 1;
}
.tab-close:hover {
  background: var(--danger-tint);
  color: var(--danger);
}
</style>
