<script setup lang="ts">
/**
 * TabBar：打开中的接口标签页。
 * - 每个标签：方法标签（GET 绿 / POST 黄…）+ 截断的接口名；宽 120–200px；
 * - 布局：末尾两个平级按钮——「+」新建、「⋯」标签管理；溢出时各自吸在
 *   右边缘（Chrome 式，sticky 实现），tab 再多也不会把它们滚丢；
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
import { ensureSwapMounted } from '../utils/sortable'
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

// ---------- 页签拖拽排序 ----------
// SortableJS + 落点按 DOM 顺序回写 store（与 ProjectList / ScenarioEditor 同款收口）。
// filter 挡住关闭钮等交互元素；空标签拖拽期间 click 激活不受影响（fallback 容差）。
const tabListEl = ref<HTMLElement | null>(null)
let tabSortable: Sortable | null = null

function initTabSortable(): void {
  if (!tabListEl.value || tabSortable) return
  ensureSwapMounted()
  tabSortable = Sortable.create(tabListEl.value, {
    animation: 150,
    easing: 'cubic-bezier(0.2, 0, 0, 1)',
    forceFallback: true,
    fallbackTolerance: 3,
    ghostClass: 'tab-ghost',
    chosenClass: 'tab-chosen',
    filter: 'button, input, select, textarea',
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
})

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
 * 标签行数据一次求值：原来模板每标签调 methodOf/titleOf/isDirty 3-4 次
 *（每次各自查 drafts/endpoints），此处单遍组装供模板复用。
 */
const tabs = computed(() =>
  store.openTabs.map((id) => ({
    id,
    create: store.isCreateTab(id),
    scenario: store.isScenarioTab(id),
    method: store.draftOf(id)?.method ?? 'GET',
    title: store.titleOf(id),
    dirty: store.isDirty(id),
  })),
)

// ---------- 标签管理菜单：关闭当前 / 其他 / 全部（有未保存时行内二次确认） ----------
// 菜单在触发器右侧展开（left 对齐），贴着 ⋯ 按钮往右下开，避免盖住左侧标签。
const tabMenu = ref<InstanceType<typeof Menu> | null>(null)
const tabMoreEl = ref<HTMLButtonElement | null>(null)

/** 含未保存标签时，二次确认才放行批量关闭（与单标签 Popconfirm 同语义）。 */
function unsavedConfirm(count: number): string {
  return t('tabbar.closeUnsavedConfirm', { n: count })
}

const TAB_MENU_ITEMS = computed<MenuItem[]>(() => {
  const ids = store.openTabs
  const active = store.activeTabId
  const others = ids.filter((id) => id !== active)
  const othersDirty = others.filter((id) => store.isDirty(id)).length
  const allDirty = ids.filter((id) => store.isDirty(id)).length
  return [
    {
      key: 'close-current',
      label: t('tabbar.closeCurrent'),
      disabled: !active,
      ...(active && store.isDirty(active) ? { confirm: t('tabbar.closeConfirm'), confirmDanger: true } : {}),
    },
    {
      key: 'close-others',
      label: t('tabbar.closeOthers'),
      disabled: others.length === 0,
      ...(othersDirty > 0 ? { confirm: unsavedConfirm(othersDirty), confirmDanger: true } : {}),
    },
    {
      key: 'close-all',
      label: t('tabbar.closeAll'),
      danger: true,
      dividerBefore: true,
      disabled: ids.length === 0,
      ...(allDirty > 0 ? { confirm: unsavedConfirm(allDirty), confirmDanger: true } : {}),
    },
  ]
})

function openTabMenu(): void {
  if (tabMoreEl.value) tabMenu.value?.openAt(tabMoreEl.value, TAB_MENU_ITEMS.value, 'left')
}

function runMenuAction(item: MenuItem): void {
  if (item.key === 'close-current' && store.activeTabId) {
    store.closeTab(store.activeTabId)
  } else if (item.key === 'close-others' && store.activeTabId) {
    store.closeOtherTabs(store.activeTabId)
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
    <div ref="tabListEl" class="tab-scroll" role="tablist" @keydown="onTabListKeydown">
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
      >
        <!-- 「新建」伪页签：无方法徽章，闪电指代“待创建” -->
        <span v-if="tab.create" class="tab-spark" aria-hidden="true"><Icon name="zap" :size="12" /></span>
        <!-- 「场景」伪页签：主题色烧瓶徽章区分于接口方法页签 -->
        <span
          v-else-if="tab.scenario"
          class="tab-scenario-tag"
          :title="t('tabbar.scenarioTab')"
          aria-hidden="true"
        ><Icon name="beaker" :size="12" /></span>
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

/* 内层滚动区：只有标签横滚 */
.tab-scroll {
  display: flex;
  gap: 2px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  overflow-y: hidden;
}

/* 新建「+」/ 管理「⋯」跟在末尾 tab 后面；溢出时吸在右边缘（Chrome 式），永远可见 */
.tab-scroll > .tt-trigger {
  position: sticky;
  right: 0;
  z-index: 1;
  flex-shrink: 0;
  align-self: center;
  margin-bottom: 2px;
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
  /* 溢出吸右时要盖住下方滚过的标签，需实底 */
  position: sticky;
  right: 0;
  z-index: 1;
  margin-left: 2px;
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
  box-shadow: 0 0 8px var(--accent);
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
.tab-scenario-tag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 22px;
  height: 18px;
  border-radius: 999px;
  background: var(--accent-tint);
  color: var(--accent);
}

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
