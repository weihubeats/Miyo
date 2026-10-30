<script setup lang="ts">
/**
 * Tabs：标签页导航。激活 = accent 文本 + 2px 下划线；支持数量徽标。
 * 键盘：←/→ 循环切换、Home/End 跳首尾（切换即激活），roving tabindex
 * （仅激活项 tabindex=0，其余 -1），传 panelId 时渲染 aria-controls。
 */
import { computed } from 'vue'

export interface TabItem {
  key: string
  label: string
  count?: number
  disabled?: boolean
  /** 关联面板元素 id：存在时该页签渲染 aria-controls。 */
  panelId?: string
}

const props = withDefaults(
  defineProps<{
    modelValue?: string
    tabs: TabItem[]
    size?: 'sm' | 'md'
  }>(),
  { modelValue: '', size: 'md' },
)

const emit = defineEmits<{
  'update:modelValue': [key: string]
  change: [key: string]
}>()

function pick(key: string): void {
  emit('update:modelValue', key)
  emit('change', key)
}

/** roving tabindex：激活项可 Tab 进入；未命中（激活项缺失/禁用）时退到首个可用项。 */
const tabbableKey = computed(() => {
  const active = props.tabs.find((t) => t.key === props.modelValue)
  if (active && !active.disabled) return active.key
  return props.tabs.find((t) => !t.disabled)?.key ?? ''
})

const NAV_KEYS = ['ArrowLeft', 'ArrowRight', 'Home', 'End']

/** 方向键/Home/End：移动焦点并同步激活（自动激活模式，与点击等价）。 */
function onKeydown(event: KeyboardEvent): void {
  if (!NAV_KEYS.includes(event.key)) return
  const container = event.currentTarget as HTMLElement
  const items = [...container.querySelectorAll<HTMLButtonElement>('button.tab:not(:disabled)')]
  if (!items.length) return
  event.preventDefault()

  let index = items.indexOf(document.activeElement as HTMLButtonElement)
  if (index === -1) index = event.key === 'ArrowLeft' ? 0 : items.length - 1
  if (event.key === 'ArrowRight') index = (index + 1) % items.length
  else if (event.key === 'ArrowLeft') index = (index - 1 + items.length) % items.length
  else if (event.key === 'Home') index = 0
  else index = items.length - 1

  const next = items[index]
  const key = next.dataset.key
  if (key !== undefined && key !== props.modelValue) pick(key)
  next.focus()
}
</script>

<template>
  <div class="tabs" :class="`size-${size}`" role="tablist" @keydown="onKeydown">
    <button
      v-for="t in tabs"
      :key="t.key"
      type="button"
      class="tab"
      :class="{ active: modelValue === t.key }"
      role="tab"
      :data-key="t.key"
      :aria-selected="modelValue === t.key"
      :aria-controls="t.panelId"
      :tabindex="tabbableKey === t.key ? 0 : -1"
      :disabled="t.disabled"
      @click="pick(t.key)"
    >
      <span class="tab-label">{{ t.label }}</span>
      <span v-if="t.count !== undefined" class="tab-badge" :class="{ on: modelValue === t.key }">
        {{ t.count }}
      </span>
    </button>
  </div>
</template>

<style scoped>
.tabs {
  display: flex;
  align-items: stretch;
  gap: 2px;
  overflow-x: auto;
  overflow-y: hidden;
  border-bottom: 1px solid var(--border);
}
.tabs.size-md .tab {
  height: 34px;
  padding: 0 12px;
  font-size: 13px;
}
.tabs.size-sm .tab {
  height: 30px;
  padding: 0 10px;
  font-size: 12.5px;
}

.tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: none;
  background: none;
  color: var(--text-2);
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.tab:hover:not(:disabled) {
  color: var(--text-1);
  background: var(--bg-hover);
}
.tab:active:not(:disabled) {
  background: var(--bg-active);
}
.tab:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: -2px;
}
.tab.active {
  color: var(--accent);
  font-weight: 600;
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
.tab:disabled {
  opacity: 0.4;
  cursor: default;
}

.tab-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  border-radius: 999px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 600;
  line-height: 1;
  font-variant-numeric: tabular-nums;
  color: var(--text-2);
  background: color-mix(in srgb, var(--text-3) 30%, transparent);
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.tab-badge.on {
  color: var(--text-1);
  background: color-mix(in srgb, var(--text-3) 46%, transparent);
}
</style>