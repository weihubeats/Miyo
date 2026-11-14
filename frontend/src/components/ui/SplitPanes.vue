<script setup lang="ts">
/**
 * SplitPanes：可拖拽双栏容器（请求|响应 左右分栏单源）。
 * - 宽度受控（v-model:width，px）：父级负责 localStorage 持久化；
 * - 拖拽 + 键盘（←/→ 16px，Shift 48px）+ 双击 reset 事件（父级恢复默认）；
 * - stackBelow：容器宽度低于阈值时自动改为上下堆叠（隐藏分割线），
 *   1366px 小窗与全屏抽屉两态都能正常落位；
 * - a11y：分割线 role=separator + aria-valuenow，可聚焦可键盘调宽。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 首栏宽度（px，v-model:width）。 */
    width: number
    min?: number
    /** 首栏最大宽度（px）；0 表示不限。 */
    max?: number
    /** 容器宽度低于该值时改为上下堆叠；0 表示不启用。 */
    stackBelow?: number
  }>(),
  { min: 220, max: 0, stackBelow: 0 },
)

const emit = defineEmits<{
  'update:width': [width: number]
  /** 双击分割线恢复默认（由父级决定默认值）。 */
  reset: []
}>()

const rootEl = ref<HTMLElement | null>(null)
const stacked = ref(false)

let ro: ResizeObserver | null = null
function measure(): void {
  if (!rootEl.value || !props.stackBelow) return
  stacked.value = rootEl.value.clientWidth < props.stackBelow
}
onMounted(() => {
  // 首帧先算一次（无 ResizeObserver 的环境只依赖这一次），
  // 之后交给 RO 跟随容器尺寸变化。
  measure()
  if (typeof ResizeObserver === 'undefined') return
  ro = new ResizeObserver(measure)
  ro.observe(rootEl.value!)
})
onBeforeUnmount(() => {
  ro?.disconnect()
  ro = null
  onDragEnd()
})

function clamp(v: number): number {
  return Math.min(Math.max(v, props.min), props.max || Number.POSITIVE_INFINITY)
}

/** 渲染宽度：夹紧到 [min, max]（超储的旧记忆宽度不撑破布局）。 */
const firstStyle = computed(() =>
  stacked.value ? undefined : { width: `${Math.round(clamp(props.width))}px` },
)

let dragStartX = 0
let dragStartWidth = 0
let dragging = false

function onDividerMousedown(e: MouseEvent): void {
  if (stacked.value) return
  dragging = true
  dragStartX = e.clientX
  dragStartWidth = props.width
  document.body.style.userSelect = 'none'
  document.body.style.cursor = 'col-resize'
  window.addEventListener('mousemove', onDragMove)
  window.addEventListener('mouseup', onDragEnd)
}

function onDragMove(e: MouseEvent): void {
  if (!dragging) return
  emit('update:width', clamp(dragStartWidth + (e.clientX - dragStartX)))
}

function onDragEnd(): void {
  if (!dragging) return
  dragging = false
  document.body.style.userSelect = ''
  document.body.style.cursor = ''
  window.removeEventListener('mousemove', onDragMove)
  window.removeEventListener('mouseup', onDragEnd)
}

function onDividerKeydown(e: KeyboardEvent): void {
  if (stacked.value) return
  const step = e.shiftKey ? 48 : 16
  if (e.key === 'ArrowLeft') {
    e.preventDefault()
    emit('update:width', clamp(props.width - step))
  } else if (e.key === 'ArrowRight') {
    e.preventDefault()
    emit('update:width', clamp(props.width + step))
  } else if (e.key === 'Enter') {
    e.preventDefault()
    emit('reset')
  }
}
</script>

<template>
  <div ref="rootEl" class="sp" :class="{ stacked }">
    <div class="sp-first" :style="firstStyle">
      <slot name="first" />
    </div>
    <div
      v-if="!stacked"
      class="sp-divider"
      role="separator"
      aria-orientation="vertical"
      :aria-valuenow="Math.round(clamp(width))"
      :aria-valuemin="min"
      :aria-valuemax="max || undefined"
      tabindex="0"
      @mousedown="onDividerMousedown"
      @keydown="onDividerKeydown"
      @dblclick="emit('reset')"
    ></div>
    <div class="sp-second">
      <slot name="second" />
    </div>
  </div>
</template>

<style scoped>
.sp {
  display: flex;
  align-items: stretch;
  min-width: 0;
  min-height: 0;
}
.sp.stacked {
  flex-direction: column;
}

.sp-first,
.sp-second {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}
.sp.stacked .sp-first,
.sp.stacked .sp-second {
  flex: 1;
}

.sp-divider {
  flex: none;
  width: 9px;
  margin: 0 -2px;
  cursor: col-resize;
  position: relative;
  z-index: 1;
  touch-action: none;
}
/* 视觉线 2px 居中，命中区 9px */
.sp-divider::after {
  content: '';
  position: absolute;
  top: 4px;
  bottom: 4px;
  left: 3.5px;
  width: 2px;
  border-radius: 1px;
  background: var(--border);
  transition: background var(--dur) var(--ease);
}
.sp-divider:hover::after,
.sp-divider:focus-visible::after {
  background: var(--accent);
}
.sp-divider:focus-visible {
  outline: none;
}
</style>
