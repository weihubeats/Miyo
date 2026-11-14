<script setup lang="ts">
/**
 * Drawer：右侧侧滑抽屉。
 * - v-model:open 受控；Esc / 遮罩点击 / 右上 ✕ 关闭（可经 guardClose 拦截）；
 * - 打开后焦点移入抽屉、Tab 圈定在内部（焦点陷阱），关闭后归还触发元素；
 * - 打开期间锁定 body 滚动；Teleport 到 body；
 * - resizable：左缘拖拽调宽 + 键盘调宽（宽度经 v-model:resizable-width 受控，
 *   持久化归父级；双击 / Enter 发 reset-width）；
 * - fullscreenable：头部显示全屏切换（CSS 覆盖视口）；
 * - keepMounted：首次打开后内容常驻（关闭只隐藏，不卸载）——切换对象 /
 *   重开抽屉不再丢页签与滚动状态；首开前不挂载，无额外启动成本；
 * - layer='drawer'：浮层层级用 --z-drawer（自绘大抽屉压过 Modal 基线）；
 * - #header 插槽：替换默认标题（关闭按钮仍由组件渲染）。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { pushEscHandler } from '@/utils/escStack'
import IconButton from '@/components/ui/IconButton.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    width?: string
    closable?: boolean
    /** 关闭拦截：返回 false 阻止（Esc / 遮罩 / ✕ 统一走它）。 */
    guardClose?: () => boolean
    /** 左缘拖拽调宽 + 键盘调宽。 */
    resizable?: boolean
    /** 拖拽模式宽度（px，v-model:resizable-width）。 */
    resizableWidth?: number
    minWidth?: number
    /** 拖拽最大宽度（px）；0 = 90vw。 */
    maxWidth?: number
    fullscreenable?: boolean
    keepMounted?: boolean
    /** 浮层层级：modal（默认，--z-modal）/ drawer（--z-drawer）。 */
    layer?: 'modal' | 'drawer'
  }>(),
  {
    title: '',
    width: '640px',
    closable: true,
    guardClose: undefined,
    resizable: false,
    resizableWidth: undefined,
    minWidth: 480,
    maxWidth: 0,
    fullscreenable: false,
    keepMounted: false,
    layer: 'modal',
  },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  close: []
  'update:resizableWidth': [width: number]
  'reset-width': []
}>()

const bodyEl = ref<HTMLElement | null>(null)

const titleId = `miyo-drawer-title-${Math.random().toString(36).slice(2, 9)}`

const locale = useLocaleStore()
const t = locale.t

/** keepMounted 下首次打开才真正渲染（此前不挂载内容，零启动成本）。 */
const rendered = ref(props.open)
watch(
  () => props.open,
  (open) => {
    if (open) rendered.value = true
  },
)
/** 外层 v-if（Transition 进出场由 v-show / v-if 共同驱动）。 */
const mounted = computed(() => (props.keepMounted ? rendered.value : props.open))
const visible = computed(() => (props.keepMounted ? props.open : true))

function close(): void {
  emit('update:open', false)
  emit('close')
}

function attemptClose(): void {
  if (props.guardClose && !props.guardClose()) return
  close()
}

// ---------- 全屏 ----------
const fullscreen = ref(false)

// ---------- 拖拽调宽 ----------
const dragMaxWidth = computed(() => props.maxWidth || Math.round(window.innerWidth * 0.9))

function clampWidth(w: number): number {
  return Math.min(Math.max(Math.round(w), props.minWidth), dragMaxWidth.value)
}

let dragStartX = 0
let dragStartW = 0
let resizing = false

function onResizeStart(e: MouseEvent): void {
  if (fullscreen.value || props.resizableWidth == null) return
  resizing = true
  dragStartX = e.clientX
  dragStartW = props.resizableWidth
  document.body.style.userSelect = 'none'
  document.body.style.cursor = 'ew-resize'
  window.addEventListener('mousemove', onResizeMove)
  window.addEventListener('mouseup', onResizeEnd)
}

function onResizeMove(e: MouseEvent): void {
  if (!resizing) return
  emit('update:resizableWidth', clampWidth(dragStartW + (dragStartX - e.clientX)))
}

function onResizeEnd(): void {
  if (!resizing) return
  resizing = false
  document.body.style.userSelect = ''
  document.body.style.cursor = ''
  window.removeEventListener('mousemove', onResizeMove)
  window.removeEventListener('mouseup', onResizeEnd)
}

function onResizeKeydown(e: KeyboardEvent): void {
  if (fullscreen.value || props.resizableWidth == null) return
  const step = e.shiftKey ? 48 : 16
  if (e.key === 'ArrowLeft') {
    e.preventDefault()
    emit('update:resizableWidth', clampWidth(props.resizableWidth + step))
  } else if (e.key === 'ArrowRight') {
    e.preventDefault()
    emit('update:resizableWidth', clampWidth(props.resizableWidth - step))
  } else if (e.key === 'Enter') {
    e.preventDefault()
    emit('reset-width')
  }
}

/** 实际渲染宽度：拖拽模式走受控 px，否则用字符串宽度。 */
const effectiveWidth = computed(() =>
  props.resizable && props.resizableWidth != null
    ? `${clampWidth(props.resizableWidth)}px`
    : props.width,
)

const layerZ = computed(() => (props.layer === 'drawer' ? 'var(--z-drawer)' : 'var(--z-modal)'))

function focusables(): HTMLElement[] {
  if (!bodyEl.value) return []
  return Array.from(
    bodyEl.value.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  )
}

function onKeydown(event: KeyboardEvent): void {
  // ESC 交给 escStack 层级仲裁（子弹层打开时后开先关）；这里只承担 Tab 焦点陷阱。
  if (event.key !== 'Tab') return
  const root = bodyEl.value
  if (!root) return
  const items = focusables()
  if (!items.length) {
    event.preventDefault()
    root.focus()
    return
  }
  const first = items[0]
  const last = items[items.length - 1]
  const active = document.activeElement
  const inside = !!active && root.contains(active)
  if (event.shiftKey) {
    if (!inside || active === first) {
      event.preventDefault()
      last.focus()
    }
  } else if (!inside || active === last) {
    event.preventDefault()
    first.focus()
  }
}

let lastFocused: HTMLElement | null = null
/** escStack 卸载函数（打开期间持有）。 */
let offEsc: (() => void) | null = null

watch(
  () => props.open,
  (open) => {
    if (open) {
      lastFocused = document.activeElement as HTMLElement | null
      document.body.style.overflow = 'hidden'
      document.addEventListener('keydown', onKeydown)
      offEsc = pushEscHandler(() => {
        attemptClose()
        return true
      })
      // 首个可聚焦元素可能随内容异步渲染，先兜底聚焦容器
      setTimeout(() => {
        ;(focusables()[0] ?? bodyEl.value)?.focus()
      }, 0)
    } else {
      document.body.style.overflow = ''
      document.removeEventListener('keydown', onKeydown)
      offEsc?.()
      offEsc = null
      lastFocused?.focus?.()
      lastFocused = null
    }
  },
  // TestCaseDrawer 等「挂载即开」的宿主：immediate 保证首开也压 Esc 栈/锁滚动
  { immediate: true },
)

onBeforeUnmount(() => {
  document.body.style.overflow = ''
  document.removeEventListener('keydown', onKeydown)
  onResizeEnd()
  offEsc?.()
  offEsc = null
})
</script>

<template>
  <Teleport to="body">
    <Transition name="d">
      <div
        v-if="mounted"
        v-show="visible"
        class="d-mask"
        :style="{ zIndex: layerZ }"
        @mousedown.self="closable && attemptClose()"
      >
        <aside
          ref="bodyEl"
          class="d-drawer"
          :class="{ 'is-fullscreen': fullscreen }"
          role="dialog"
          aria-modal="true"
          :aria-labelledby="title ? titleId : undefined"
          tabindex="-1"
          :style="{ width: effectiveWidth }"
          @mousedown.stop
        >
          <span
            v-if="resizable && !fullscreen"
            class="d-resizer"
            role="separator"
            aria-orientation="vertical"
            :aria-valuenow="resizableWidth"
            :aria-valuemin="minWidth"
            :aria-valuemax="maxWidth || undefined"
            :title="t('common.resizeHint')"
            tabindex="0"
            @mousedown="onResizeStart"
            @keydown="onResizeKeydown"
            @dblclick="emit('reset-width')"
          ></span>
          <div class="d-head">
            <slot name="header">
              <h3 :id="titleId" class="d-title">{{ title }}</h3>
            </slot>
            <div class="d-head-actions">
              <IconButton
                v-if="fullscreenable"
                :name="fullscreen ? 'minimize-2' : 'maximize-2'"
                :size="14"
                :title="fullscreen ? t('common.exitFullscreen') : t('common.fullscreen')"
                @click="fullscreen = !fullscreen"
              />
              <IconButton v-if="closable" name="x" :size="14" :title="t('common.close')" @click="attemptClose" />
            </div>
          </div>
          <div class="d-body">
            <slot />
          </div>
          <div v-if="$slots.footer" class="d-footer">
            <slot name="footer" />
          </div>
        </aside>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.d-mask {
  position: fixed;
  inset: 0;
  display: flex;
  justify-content: flex-end;
  background: var(--mask);
}

.d-drawer {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  max-width: 96vw;
  border-left: 1px solid var(--border);
  border-radius: var(--radius-lg) 0 0 var(--radius-lg);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-drawer);
  overflow: hidden;
}
.d-drawer.is-fullscreen {
  width: 100vw !important;
  max-width: none;
  border-radius: 0;
  border-left: none;
}

/* 左缘拖拽把手：视觉 2px 线，命中区 10px */
.d-resizer {
  position: absolute;
  left: -4px;
  top: 0;
  bottom: 0;
  width: 10px;
  cursor: ew-resize;
  z-index: 2;
  touch-action: none;
}
.d-resizer::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: 4px;
  width: 2px;
  background: transparent;
  transition: background var(--dur) var(--ease);
}
.d-resizer:hover::after,
.d-resizer:focus-visible::after {
  background: var(--accent);
}
.d-resizer:focus-visible {
  outline: none;
}

.d-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.d-head > :first-child {
  flex: 1;
  min-width: 0;
}

.d-head-actions {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex: none;
}

.d-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}

.d-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px;
}

/* 底部操作条：固定于抽屉底（不随内容滚动） */
.d-footer {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
  background: var(--bg-elevated);
}

.d-enter-active,
.d-leave-active {
  transition: transform 220ms var(--ease), opacity 220ms var(--ease);
}

.d-mask {
  transition: background 220ms var(--ease);
}

.d-enter-from,
.d-leave-to {
  background: transparent;
}

.d-enter-from .d-drawer,
.d-leave-to .d-drawer {
  transform: translateX(100%);
}
</style>
