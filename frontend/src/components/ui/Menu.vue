<script setup lang="ts">
/**
 * Menu：通用下拉菜单（Teleport 到 body，固定定位）。
 * - openAt(el, side) 依触发元素定位，底部空间不足自动上翻；
 * - 项支持 icon / danger（红色）/ disabled / dividerBefore 分隔线；
 * - 项带 confirm 文案时先进入行内确认视图，确认后 emit('confirm')；
 * - 键盘导航：容器聚焦 + aria-activedescendant，↑↓/Home/End 漫游、Enter/Space
 *   激活、确认视图内 Enter 确认 / Esc 返回列表；关闭后焦点归还触发元素；
 * - 外部点击 / Esc / 滚动 / 窗口缩放自动关闭。
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import type { IconName } from '@/components/ui/Icon.vue'
import { pushEscHandler } from '@/utils/escStack'

export interface MenuItem {
  key: string
  label: string
  icon?: IconName
  /** 选中态：右侧显示 check 图标并整行高亮。 */
  checked?: boolean
  /** 图标使用主题色（如「新建项目」的 +）。 */
  iconAccent?: boolean
  /** 右侧快捷键提示（如 ⌘N）。 */
  shortcut?: string
  danger?: boolean
  disabled?: boolean
  dividerBefore?: boolean
  confirm?: string
  /** 确认视图按危险操作呈现（红色警示徽章 + 实心删除按钮 + 不可撤销提示）；默认跟随 danger。 */
  confirmDanger?: boolean
}

const emit = defineEmits<{
  select: [item: MenuItem]
  confirm: [item: MenuItem]
  open: []
  close: []
}>()

type View = { kind: 'list' } | { kind: 'confirm'; item: MenuItem }

/** 模块级实例计数（aria id 多实例隔离；每次 setup 递增）。 */
let MENU_SEQ = 0

const open = ref(false)
const view = ref<View>({ kind: 'list' })
const items = ref<MenuItem[]>([])
const pos = ref({ left: 0, top: 0 })
const menuEl = ref<HTMLElement | null>(null)

/** aria-activedescendant 实例前缀（同页多菜单实例 id 不冲突）。 */
const UID = `miyo-menu-${++MENU_SEQ}`
const activeKey = ref<string | null>(null)

const locale = useLocaleStore()
const t = locale.t

const menuStyle = computed(() => ({ left: `${pos.value.left}px`, top: `${pos.value.top}px` }))

/** 确认视图危险语义：confirmDanger 显式指定，否则跟随 item.danger（列表色与确认色解耦）。 */
const confirmDanger = computed(
  () => view.value.kind === 'confirm' && (view.value.item.confirmDanger ?? view.value.item.danger),
)

/** 触发元素矩形与弹出方向：openAt 记录，渲染后按实测尺寸校正定位时复用。 */
let triggerRect: DOMRect | null = null
let triggerSide: 'right' | 'left' = 'right'
/** 触发元素引用：关闭时归还焦点（键盘用户从原位置继续）。 */
let triggerEl: HTMLElement | null = null

/** 按实测宽高定位：默认贴按钮下方，底部放不下且上方放得下时紧贴按钮上翻。
 *  原实现用写死 220px 估高，两三项的小菜单上翻后与按钮间留出大片空隙（悬在半空）。 */
function applyPosition(): void {
  const rect = triggerRect
  const menu = menuEl.value
  if (!rect || !menu) return
  const width = menu.offsetWidth || 176
  const height = menu.offsetHeight || 220
  let left = triggerSide === 'right' ? rect.right - width : rect.left
  left = Math.max(8, Math.min(left, window.innerWidth - width - 8))
  let top = rect.bottom + 4
  if (top + height > window.innerHeight - 8 && rect.top - height - 4 > 8) {
    top = rect.top - height - 4
  }
  pos.value = { left, top }
}

function openAt(el: HTMLElement, menuItems: MenuItem[], side: 'right' | 'left' = 'right'): void {
  items.value = menuItems
  triggerRect = el.getBoundingClientRect()
  triggerSide = side
  triggerEl = el
  // 首帧先用估算位置渲染，nextTick 拿到实测尺寸后立即校正（微任务先于绘制，无跳动）
  const width = 176
  const height = 220
  let left = side === 'right' ? triggerRect.right - width : triggerRect.left
  left = Math.max(8, Math.min(left, window.innerWidth - width - 8))
  let top = triggerRect.bottom + 4
  if (top + height > window.innerHeight - 8 && triggerRect.top - height - 4 > 8) {
    top = triggerRect.top - height - 4
  }
  pos.value = { left, top }
  view.value = { kind: 'list' }
  open.value = true
  emit('open')
  // 键盘起点：首个可用项高亮 + 容器聚焦（↑↓ 立即可用）
  activeKey.value = items.value.find((it) => !it.disabled)?.key ?? null
  void nextTick(() => {
    applyPosition()
    menuEl.value?.focus()
  })
}

function close(): void {
  if (!open.value) return
  open.value = false
  view.value = { kind: 'list' }
  activeKey.value = null
  triggerEl?.focus()
  triggerEl = null
  emit('close')
}

function onItemClick(item: MenuItem): void {
  if (item.disabled) return
  if (item.confirm) {
    view.value = { kind: 'confirm', item }
    void nextTick(applyPosition)
    return
  }
  close()
  emit('select', item)
}

function backToList(): void {
  view.value = { kind: 'list' }
  void nextTick(applyPosition)
}

function onConfirm(): void {
  const item = view.value.kind === 'confirm' ? view.value.item : null
  close()
  if (item) emit('confirm', item)
}

function onDocMouseDown(event: MouseEvent): void {
  const target = event.target as Node
  if (menuEl.value?.contains(target)) return
  close()
}

/* ---- 键盘导航：容器 tabindex=-1 聚焦 + aria-activedescendant 漫游 ---- */

/** 可激活项（disabled 不参与漫游）。 */
const enabledItems = computed(() => items.value.filter((it) => !it.disabled))

/** 漫游：1/-1 相对移动，'first'/'last' 跳边界；溢出时滚动跟随。 */
function moveActive(step: 1 | -1 | 'first' | 'last'): void {
  const list = enabledItems.value
  if (!list.length) return
  const idx = list.findIndex((it) => it.key === activeKey.value)
  let next: number
  if (step === 'first') next = 0
  else if (step === 'last') next = list.length - 1
  else if (idx === -1) next = step === 1 ? 0 : list.length - 1
  else next = (idx + step + list.length) % list.length
  activeKey.value = list[next].key
  void nextTick(() => {
    const el = menuEl.value?.querySelector<HTMLElement>('.miyo-menu-item.active')
    // jsdom 无 scrollIntoView（单测环境守卫）
    el?.scrollIntoView?.({ block: 'nearest' })
  })
}

function onMenuKeydown(e: KeyboardEvent): void {
  if (view.value.kind === 'confirm') {
    if (e.key === 'Enter') {
      e.preventDefault()
      onConfirm()
    } else if (e.key === 'Escape') {
      // 确认视图：Esc 回列表而非关菜单（容器先于 escStack 收到，stopPropagation 拦截）
      e.preventDefault()
      e.stopPropagation()
      backToList()
    }
    return
  }
  switch (e.key) {
    case 'ArrowDown':
      e.preventDefault()
      moveActive(1)
      break
    case 'ArrowUp':
      e.preventDefault()
      moveActive(-1)
      break
    case 'Home':
      e.preventDefault()
      moveActive('first')
      break
    case 'End':
      e.preventDefault()
      moveActive('last')
      break
    case 'Enter':
    case ' ': {
      e.preventDefault()
      const item = items.value.find((it) => it.key === activeKey.value)
      if (item && !item.disabled) onItemClick(item)
      break
    }
    default:
      break
  }
}

/** escStack 卸载函数（打开期间持有）。 */
let offEsc: (() => void) | null = null

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('mousedown', onDocMouseDown, true)
    document.addEventListener('scroll', close, true)
    window.addEventListener('resize', close)
    // ESC 层级仲裁：后开先关（Modal 里的菜单按 ESC 只关菜单）。
    offEsc = pushEscHandler(() => {
      close()
      return true
    })
  } else {
    document.removeEventListener('mousedown', onDocMouseDown, true)
    document.removeEventListener('scroll', close, true)
    window.removeEventListener('resize', close)
    offEsc?.()
    offEsc = null
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocMouseDown, true)
  document.removeEventListener('scroll', close, true)
  window.removeEventListener('resize', close)
  offEsc?.()
  offEsc = null
})

defineExpose({ openAt, close })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      ref="menuEl"
      class="miyo-menu"
      :style="menuStyle"
      role="menu"
      tabindex="-1"
      :aria-activedescendant="activeKey ? `${UID}-item-${activeKey}` : undefined"
      @click.stop
      @keydown="onMenuKeydown"
    >
      <template v-if="view.kind === 'list'">
        <template v-for="item in items" :key="item.key">
          <div v-if="item.dividerBefore" class="miyo-menu-divider"></div>
          <button
            :id="`${UID}-item-${item.key}`"
            class="miyo-menu-item"
            :class="{
              danger: item.danger,
              disabled: item.disabled,
              checked: item.checked,
              active: view.kind === 'list' && item.key === activeKey,
            }"
            type="button"
            role="menuitem"
            :aria-disabled="item.disabled || undefined"
            :tabindex="-1"
            :disabled="item.disabled"
            @click="onItemClick(item)"
            @mouseenter="activeKey = item.key"
          >
            <span
              v-if="item.icon"
              class="miyo-menu-icon"
              :class="{ accent: item.iconAccent }"
            >
              <Icon :name="item.icon" :size="14" />
            </span>
            <span class="miyo-menu-label">{{ item.label }}</span>
            <kbd v-if="item.shortcut" class="miyo-menu-kbd">{{ item.shortcut }}</kbd>
            <Icon
              v-else-if="item.checked"
              class="miyo-menu-check"
              name="check"
              :size="13"
            />
          </button>
        </template>
      </template>
      <template v-else>
        <div class="miyo-menu-confirm" :class="{ danger: confirmDanger }">
          <div class="miyo-menu-confirm-head">
            <span class="miyo-menu-confirm-badge">
              <Icon name="alert-triangle" :size="13" />
            </span>
            <p class="miyo-menu-confirm-title">{{ view.item.confirm }}</p>
          </div>
          <p v-if="confirmDanger" class="miyo-menu-confirm-hint">{{ t('confirm.undone') }}</p>
          <div class="miyo-menu-confirm-actions">
            <button class="miyo-btn miyo-btn-sm" type="button" @click="backToList">
              {{ t('common.cancel') }}
            </button>
            <button
              class="miyo-btn miyo-btn-sm"
              :class="confirmDanger ? 'miyo-btn-danger-solid' : undefined"
              type="button"
              @click="onConfirm"
            >
              {{ t('confirm.ok') }}
            </button>
          </div>
        </div>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.miyo-menu {
  position: fixed;
  z-index: var(--z-popover);
  min-width: 176px;
  max-width: 240px;
  padding: 6px;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow:
    0 25px 50px -12px rgb(0 0 0 / 0.5),
    var(--shadow-lg);
  animation: menu-in 120ms var(--ease);
  transform-origin: top center;
}
.miyo-menu:focus-visible {
  outline: none;
}

.miyo-menu-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 6px 12px;
  border: none;
  background: none;
  border-radius: var(--radius-md);
  font-size: var(--fs-xs);
  font-family: inherit;
  color: var(--text-2);
  cursor: pointer;
  text-align: left;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.miyo-menu-item:hover,
.miyo-menu-item.active {
  background: var(--bg-hover);
  color: var(--text-1);
}
.miyo-menu-item.checked {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--text-1);
}
.miyo-menu-item.danger {
  color: var(--danger);
}
.miyo-menu-item.danger:hover {
  background: var(--danger-tint);
  color: var(--danger);
}
.miyo-menu-item.disabled {
  color: var(--text-3);
  cursor: default;
}
.miyo-menu-item.disabled:hover,
.miyo-menu-item.disabled.active {
  background: none;
}

.miyo-menu-icon {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  color: var(--text-3);
}
.miyo-menu-item:hover .miyo-menu-icon {
  color: var(--text-2);
}
.miyo-menu-icon.accent {
  color: var(--accent);
}
.miyo-menu-icon :deep(svg) {
  display: block;
}

.miyo-menu-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.miyo-menu-check {
  flex-shrink: 0;
  color: var(--accent);
}

.miyo-menu-kbd {
  flex-shrink: 0;
  margin-left: 8px;
  padding: 1px 5px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--bg-hover);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.3;
  color: var(--text-3);
}

.miyo-menu-divider {
  height: 1px;
  margin: 4px -6px;
  background: var(--border);
}

/* ---- 行内确认视图：警示徽章 + 标题 + 不可撤销提示 + 按钮行 ---- */
.miyo-menu-confirm {
  padding: 4px;
}

.miyo-menu-confirm-head {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.miyo-menu-confirm-badge {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-2);
}
.miyo-menu-confirm.danger .miyo-menu-confirm-badge {
  background: var(--danger-tint);
  color: var(--danger);
}

.miyo-menu-confirm-title {
  margin: 2px 0 0;
  font-size: 12.5px;
  font-weight: 600;
  line-height: 1.45;
  color: var(--text-1);
  word-break: break-word;
}

.miyo-menu-confirm-hint {
  /* 与标题左缘对齐：徽章 22 + gap 8 */
  margin: 5px 0 0 30px;
  font-size: var(--fs-xxs);
  line-height: 1.4;
  color: var(--text-3);
}

.miyo-menu-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 12px;
}

@keyframes menu-in {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>