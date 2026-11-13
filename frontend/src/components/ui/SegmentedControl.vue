<script setup lang="ts">
/**
 * SegmentedControl：分段控件（Body 类型等）。激活项为填充 pill。
 * 五态：default / hover / focus / active / disabled × 双主题。
 * 键盘：←/→ 循环切换、Home/End 跳首尾（切换即激活）、roving tabindex；
 * 选项带 panelId 时渲染 aria-controls（有关联面板时）。
 */
import { computed } from 'vue'
import Icon, { type IconName } from '@/components/ui/Icon.vue'

export interface SegmentOption {
  value: string
  label: string
  icon?: IconName
  /** 关联面板元素 id：存在时该分段渲染 aria-controls。 */
  panelId?: string
}

const props = withDefaults(
  defineProps<{
    modelValue?: string | null
    options: SegmentOption[]
    disabled?: boolean
    size?: 'sm' | 'md'
  }>(),
  { modelValue: null, disabled: false, size: 'md' },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
  change: [value: string]
}>()

function pick(value: string): void {
  emit('update:modelValue', value)
  emit('change', value)
}

/** roving tabindex：激活项可 Tab 进入；未命中时退到第一个选项。 */
const tabbableValue = computed(() => {
  const active = props.options.find((o) => String(props.modelValue) === o.value)
  return (active ?? props.options[0])?.value ?? ''
})

const NAV_KEYS = ['ArrowLeft', 'ArrowRight', 'Home', 'End']

/** 方向键/Home/End：移动焦点并同步激活（自动激活模式，与点击等价）。 */
function onKeydown(event: KeyboardEvent): void {
  if (props.disabled || !NAV_KEYS.includes(event.key)) return
  const container = event.currentTarget as HTMLElement
  const items = [...container.querySelectorAll<HTMLButtonElement>('button.seg-item:not(:disabled)')]
  if (!items.length) return
  event.preventDefault()

  let index = items.indexOf(document.activeElement as HTMLButtonElement)
  if (index === -1) index = event.key === 'ArrowLeft' ? 0 : items.length - 1
  if (event.key === 'ArrowRight') index = (index + 1) % items.length
  else if (event.key === 'ArrowLeft') index = (index - 1 + items.length) % items.length
  else if (event.key === 'Home') index = 0
  else index = items.length - 1

  const next = items[index]
  const value = next.dataset.value
  if (value !== undefined && value !== String(props.modelValue)) pick(value)
  next.focus()
}
</script>

<template>
  <div
    class="seg"
    :class="[`size-${size}`, { disabled }]"
    role="tablist"
    @keydown="onKeydown"
  >
    <button
      v-for="o in options"
      :key="o.value"
      type="button"
      class="seg-item"
      :class="{ active: String(modelValue) === o.value }"
      role="tab"
      :data-value="o.value"
      :aria-selected="String(modelValue) === o.value"
      :aria-controls="o.panelId"
      :tabindex="tabbableValue === o.value ? 0 : -1"
      :disabled="disabled"
      @click="pick(o.value)"
    >
      <Icon v-if="o.icon" :name="o.icon" :size="13" />{{ o.label }}
    </button>
  </div>
</template>

<style scoped>
.seg {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}
.seg.size-md .seg-item {
  height: 26px;
  padding: 0 14px;
  font-size: 12.5px;
}
.seg.size-sm .seg-item {
  height: 22px;
  padding: 0 10px;
  font-size: 12px;
}

.seg-item {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border: none;
  background: transparent;
  color: var(--text-2);
  border-radius: var(--radius-sm);
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.seg-item:hover:not(:disabled) {
  color: var(--text-1);
  background: var(--bg-hover);
}
.seg-item:active:not(:disabled) {
  background: var(--bg-active);
}
.seg-item:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}
.seg-item.active {
  background: var(--accent);
  color: #fff;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
}
.seg-item.active:hover {
  background: var(--accent-hover);
}
.seg.disabled .seg-item {
  opacity: 0.45;
  cursor: default;
}
</style>