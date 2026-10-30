<script setup lang="ts">
/**
 * CustomSelect：自绘下拉选择器。
 * - 触发按钮 + Teleport 到 body 的浮层（避免被 overflow 裁剪），滚动/尺寸变化自动收起；
 * - 键盘：↑/↓ 移动高亮（循环）、Enter 选中、Esc 关闭；外部点击关闭；
 * - 五态：default / hover / focus / active(open) / disabled × 深/浅主题；
 * - 作用域插槽 #display 定制触发区文案（如方法着色）、#option 定制选项行、
 *   #search 顶部搜索区（列表上方，带淡分隔线；过滤由父组件改写 options 完成）、
 *   #actions 定制选项右侧操作区（作为 .cs-opt 直接子项参与 flex 布局；
 *   点击需自行 .stop 防误选中）、#footer 底部固定操作栏（分割线下方，
 *   不参与列表滚动与键盘高亮循环）。
 * - 勾选状态固定在行尾（左侧不留勾选列，全部选项文本严格左对齐）；
 *   options[].tone 可注入语义色类（方法下拉的 text-method-*），选中态背景
 *   跟随语义色浅色高亮而非 --accent。
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from './Icon.vue'

export interface SelectOption {
  value: string | number
  label: string
  /** 分组分隔标题：不可选中、不参与键盘导航（如方法下拉里的「其他协议」）。 */
  header?: boolean
  /**
   * 选项行语义色类（如 methodTextTone('GET') 的 text-method-*，Tailwind utilities）。
   * 设置后整行文字按语义色渲染（Tailwind utilities 在 layer 内，会被组件 scoped 的
   * 无层样式覆盖，故 .cs-opt 基础色用 :not(.tone) 收窄）；选中态不再用 --accent
   * 着色，背景改用 currentColor 12% 浅色高亮（跟随语义色）。
   */
  tone?: string
}

const props = withDefaults(
  defineProps<{
    modelValue?: string | number | null
    options: SelectOption[]
    placeholder?: string
    disabled?: boolean
    size?: 'sm' | 'md'
    popClass?: string
    /** 弹窗最小宽度（px）：触发器太窄时（如请求栏模块选择）保证选项可读，默认跟触发器等宽。 */
    popMinWidth?: number
    /** 弹层宽度随内容自适应（w-auto，不窄于触发器/popMinWidth）：长文案选项（如 JavaScript）不再被触发器宽度截断。 */
    popAutoWidth?: boolean
  }>(),
  { modelValue: null, placeholder: '', disabled: false, size: 'md', popClass: '', popMinWidth: 0, popAutoWidth: false },
)

const locale = useLocaleStore()
const t = locale.t

/** 未传 placeholder 时按当前语言兜底。 */
const effectivePlaceholder = computed(() => props.placeholder || t('select.ph'))

const emit = defineEmits<{
  'update:modelValue': [value: string | number]
  change: [value: string | number]
  open: []
  close: []
}>()

const open = ref(false)
const highlight = ref(-1)
const triggerEl = ref<HTMLButtonElement | null>(null)
const popupEl = ref<HTMLDivElement | null>(null)
const pos = ref<{ left: number; top: number; width: number | 'auto'; minWidth: number; up: boolean }>({
  left: 0,
  top: 0,
  width: 0,
  minWidth: 0,
  up: false,
})

/** 可选中选项的索引列表（header 分隔项被排除，键盘导航只在其中循环）。 */
const selectableIndexes = computed(() =>
  props.options.map((o, i) => (o.header ? -1 : i)).filter((i) => i >= 0),
)

/** 从 from 出发按 step 相对移动到的可选中索引（from 无效时落到首/尾）。 */
function nextSelectable(from: number, step: 1 | -1): number {
  const list = selectableIndexes.value
  if (!list.length) return -1
  const pos = list.indexOf(from)
  if (pos === -1) return step === 1 ? list[0] : list[list.length - 1]
  return list[(pos + step + list.length) % list.length]
}

const selectedIndex = computed(() => {
  const idx = props.options.findIndex((o) => String(o.value) === String(props.modelValue))
  return idx === -1 ? -1 : idx
})

const displayLabel = computed(() => {
  const o = props.options[selectedIndex.value]
  return o ? o.label : ''
})

function measure(): void {
  const el = triggerEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  // 挂载后优先用浮层真实高度（两行选项 / 底部栏会让按行数的估算偏小）；未挂载回退估算。
  const estimated = Math.min(props.options.length * 30 + 8, 280)
  const height = popupEl.value?.offsetHeight || estimated
  const spaceBelow = window.innerHeight - rect.bottom - 8
  const up = spaceBelow < height && rect.top > height
  const minW = Math.max(rect.width, props.popMinWidth ?? 0)
  // 右缘夹取：浮层贴近窗口右缘时整体左移不越出视口（留 8px）。
  // 已挂载用实测宽；首测（v-if 挂载前）用声明宽估算，openPopup 的 nextTick 复测会校正。
  const popW = popupEl.value?.offsetWidth ?? minW
  const left = Math.max(8, Math.min(rect.left, window.innerWidth - popW - 8))
  pos.value = {
    left,
    top: up ? rect.top - height - 4 : rect.bottom + 4,
    // auto 模式：宽度随内容（w-auto），不窄于触发器——长选项文案不再截断
    width: props.popAutoWidth ? 'auto' : minW,
    minWidth: minW,
    up,
  }
}

function openPopup(): void {
  if (props.disabled || open.value) return
  measure()
  open.value = true
  highlight.value = selectedIndex.value
  emit('open')
  // 浮层挂载后按真实高度复测上下翻转（首测发生在 v-if 挂载前，只能用估算）。
  void nextTick(() => {
    if (open.value) measure()
  })
}

function close(): void {
  if (!open.value) return
  open.value = false
  emit('close')
}

function pick(option: SelectOption): void {
  emit('update:modelValue', option.value)
  emit('change', option.value)
  close()
}

function onKeydown(event: KeyboardEvent): void {
  if (props.disabled) return
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    if (!selectableIndexes.value.length) return
    if (!open.value) {
      openPopup()
      return
    }
    highlight.value = nextSelectable(highlight.value, 1)
    scrollToHighlight()
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    if (!selectableIndexes.value.length) return
    if (!open.value) {
      openPopup()
      return
    }
    highlight.value = nextSelectable(highlight.value, -1)
    scrollToHighlight()
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const target = open.value ? props.options[highlight.value] : undefined
    if (target && !target.header) {
      pick(target)
    } else {
      openPopup()
    }
  } else if (event.key === 'Escape') {
    event.preventDefault()
    close()
  } else if (event.key === 'Tab') {
    close()
  }
}

function scrollToHighlight(): void {
  requestAnimationFrame(() => {
    const el = popupEl.value?.querySelector<HTMLElement>('.cs-opt.hl')
    el?.scrollIntoView({ block: 'nearest' })
  })
}

function onDocMouseDown(event: MouseEvent): void {
  const target = event.target as Node
  if (triggerEl.value?.contains(target) || popupEl.value?.contains(target)) return
  close()
}

function onReposition(): void {
  if (open.value) measure()
}

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('mousedown', onDocMouseDown, true)
    window.addEventListener('scroll', onReposition, true)
    window.addEventListener('resize', onReposition)
  } else {
    document.removeEventListener('mousedown', onDocMouseDown, true)
    window.removeEventListener('scroll', onReposition, true)
    window.removeEventListener('resize', onReposition)
  }
})

// 搜索过滤等导致选项收缩时钳住高亮，避免 Enter 命中越界空项。
watch(
  () => props.options,
  (list) => {
    if (highlight.value >= list.length) highlight.value = list.length - 1
  },
)

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocMouseDown, true)
  window.removeEventListener('scroll', onReposition, true)
  window.removeEventListener('resize', onReposition)
})

defineExpose({ close })
</script>

<template>
  <div class="cs" :class="[`size-${size}`, { open, disabled }]">
    <button
      ref="triggerEl"
      type="button"
      class="cs-trigger"
      :disabled="disabled"
      aria-haspopup="listbox"
      :aria-expanded="open"
      @click="open ? close() : openPopup()"
      @keydown="onKeydown"
    >
      <span class="cs-value" :class="{ 'is-empty': !displayLabel }">
        <slot name="display" :label="displayLabel" :selected="options[selectedIndex] ?? null">
          {{ displayLabel || effectivePlaceholder }}
        </slot>
      </span>
      <Icon class="cs-caret" :name="open ? 'chevron-up' : 'chevron-down'" :size="12" />
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="popupEl"
        class="cs-pop"
        :class="[popClass, { up: pos.up }]"
        :style="{
          left: `${pos.left}px`,
          top: `${pos.top}px`,
          width: pos.width === 'auto' ? 'auto' : `${pos.width}px`,
          minWidth: `${pos.minWidth}px`,
        }"
        role="listbox"
      >
        <div v-if="$slots.search" class="cs-pop-search" role="search">
          <slot name="search" />
        </div>
        <div class="cs-pop-list">
          <template v-for="(o, i) in options" :key="String(o.value)">
          <div v-if="o.header" class="cs-opt-header" role="presentation">{{ o.label }}</div>
          <div
            v-else
            class="cs-opt"
            :class="[{ hl: highlight === i, sel: String(o.value) === String(modelValue), tone: !!o.tone }, o.tone]"
            role="option"
            :aria-selected="String(o.value) === String(modelValue)"
            @click="pick(o)"
            @mouseenter="highlight = i"
          >
            <span class="cs-opt-label" :title="o.label">
              <slot name="option" :option="o" :selected="String(o.value) === String(modelValue)">
                {{ o.label }}
              </slot>
            </span>
            <slot
              v-if="$slots.actions"
              name="actions"
              :option="o"
              :selected="String(o.value) === String(modelValue)"
            />
            <!-- 勾选状态统一放行尾：左侧不再留固定勾选区，全部选项文本严格左对齐 -->
            <span class="cs-opt-check">
              <Icon v-if="String(o.value) === String(modelValue)" name="check" :size="12" />
            </span>
          </div>
          </template>
        </div>
        <div v-if="$slots.footer" class="cs-pop-footer">
          <slot name="footer" />
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.cs {
  position: relative;
  display: inline-flex;
  flex: 0 0 auto;
}

.cs-trigger {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  border: 1px solid var(--border-strong);
  background: var(--bg-card);
  color: var(--text-1);
  border-radius: var(--radius);
  font-size: 13px;
  font-family: var(--font-mono);
  padding: 0 8px 0 10px;
  cursor: pointer;
  user-select: none;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.cs.size-md .cs-trigger {
  height: var(--h-md);
}
.cs.size-sm .cs-trigger {
  height: var(--h-sm);
  font-size: 12px;
}
.cs-trigger:hover:not(:disabled) {
  background: var(--bg-elevated);
  border-color: var(--border-strong);
}
.cs-trigger:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.cs.open .cs-trigger {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}
.cs.disabled .cs-trigger {
  opacity: 0.45;
  cursor: default;
}

.cs-value {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
  display: inline-flex;
  align-items: center;
}
.cs-value.is-empty {
  color: var(--text-3);
}

.cs-caret {
  flex-shrink: 0;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease);
}

.cs-pop {
  position: fixed;
  z-index: 1000;
  /* 病态超宽内容的兜底：任何情况下不越出视口两侧（与 measure() 的右缘夹取双保险） */
  max-width: calc(100vw - 16px);
  background: var(--bg-elevated);
  /* 发丝边框（深色近 border-white/10，浅色跟随变量不隐形）+ 深部大阴影拉开暗部层级 */
  border: 1px solid color-mix(in srgb, var(--border-strong) 80%, transparent);
  border-radius: var(--radius-lg);
  box-shadow:
    0 24px 48px -12px rgb(0 0 0 / 0.55),
    0 8px 20px rgb(0 0 0 / 0.45);
  padding: 4px;
  transform-origin: top;
  animation: cs-in 100ms ease-out;
}
.cs-pop.up {
  transform-origin: bottom;
}
/* 选项列表独立滚动：footer 固定在其下方，不随列表滚走 */
.cs-pop-list {
  max-height: 280px;
  overflow-y: auto;
}
/* 顶部搜索区：mb-1 px-1.5 pt-1 外边距 + 与列表间极淡分隔线（主题变量调透明，浅色下不隐形） */
.cs-pop-search {
  padding: 4px 6px;
  margin-bottom: 4px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
}
.cs-pop-footer {
  margin-top: 4px;
  padding-top: 6px;
  border-top: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
}

/* 分组标题：明确分隔线（mt-1 pt-1 档）+ 小字加粗，与选项文本列（12px）对齐 */
.cs-opt-header {
  padding: 4px 12px 5px;
  margin-top: 4px;
  border-top: 1px solid color-mix(in srgb, var(--border-strong) 80%, transparent);
  font-size: var(--fs-xxs);
  font-weight: 500;
  color: var(--text-3);
  letter-spacing: 0.06em;
  user-select: none;
  cursor: default;
}
.cs-pop-list > .cs-opt-header:first-child {
  border-top: 0;
  margin-top: 0;
}

.cs-opt {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  height: 30px;
  padding: 0 12px;
  cursor: pointer;
  font-size: 12.5px;
  font-family: var(--font-mono);
  white-space: nowrap;
  overflow: hidden;
  user-select: none;
  transition: background 100ms ease-out;
}
/* 基础文字色只作用于无语义色的行：tone 行的 text-method-* utilities（layer 内）
 * 才能生效（无层 scoped 样式优先级恒高于 layer，须用 :not(.tone) 收窄）。 */
.cs-opt:not(.tone) {
  color: var(--text-1);
}
.cs-opt.hl {
  background: var(--bg-hover);
}
.cs-opt.sel:not(.tone) {
  color: var(--accent);
}
/* 带语义色的选中项：文字/行尾勾选保持语义色，背景仅微亮浅色（深色 = white/10 档，
 * 浅色主题自动跟随 --bg-active 变量），不用语义色实底，保证任意方法色下都可读 */
.cs-opt.tone.sel {
  background: var(--bg-active);
}
.cs-opt:active {
  background: var(--accent-tint);
}
.cs-opt.tone:active {
  background: var(--bg-active);
}

.cs-opt-check {
  width: 14px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.cs-opt-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@keyframes cs-in {
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
