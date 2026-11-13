<script setup lang="ts">
/**
 * IconButton：图标按钮（default/hover/focus/active/disabled × 深/浅双主题）。
 * 28×28 触点、圆角 6、text-2 常态 → hover 提升；tone=danger/accent 语义色。
 */
import Icon from '@/components/ui/Icon.vue'
import type { IconName } from '@/components/ui/Icon.vue'

withDefaults(
  defineProps<{
    name: IconName
    size?: number
    tone?: 'default' | 'danger' | 'accent'
    title?: string
    /** 无障碍名（只喂 aria-label，不渲染原生 title）：外层已有 <Tooltip> 时用它，避免双重气泡。 */
    label?: string
    disabled?: boolean
  }>(),
  { size: 14, tone: 'default' },
)

const emit = defineEmits<{ click: [event: MouseEvent] }>()
</script>

<template>
  <button
    type="button"
    class="ib"
    :class="`tone-${tone}`"
    :title="title"
    :aria-label="label ?? title ?? name"
    :disabled="disabled"
    @click="emit('click', $event)"
  >
    <Icon :name="name" :size="size" :stroke-width="1.5" />
  </button>
</template>

<style scoped>
.ib {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  background: transparent;
  color: var(--text-2);
  border-radius: var(--radius);
  cursor: pointer;
  padding: 0;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    transform var(--dur) var(--ease);
}
.ib:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-1);
}
.ib:active:not(:disabled) {
  transform: scale(0.92);
}
.ib:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.ib:disabled {
  opacity: 0.4;
  cursor: default;
}
.tone-danger:hover:not(:disabled) {
  background: var(--danger-tint);
  color: var(--danger);
}
.tone-accent:hover:not(:disabled) {
  background: var(--accent-tint);
  color: var(--accent);
}
</style>
