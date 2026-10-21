<script setup lang="ts">
/**
 * ToolChip：单个工具在某个 skill 上的状态徽章。
 * 只显示「状态色点 + 工具名」，完整状态（含 detail）进 title / aria-label。
 * 样式自带（scoped），供卡片内可见徽章与「未启用」弹出层复用，避免两处样式漂移。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import type { SkillLinkState, SkillTargetState } from '../../types/foxApi'

const props = defineProps<{
  target: SkillTargetState
  busy?: boolean
}>()

const emit = defineEmits<{ toggle: [target: SkillTargetState] }>()

const t = useLocaleStore().t

const stateLabelKey: Record<SkillLinkState, string> = {
  linked: 'skills.stateLinked',
  missing: 'skills.stateMissing',
  drifted: 'skills.stateDrifted',
  broken: 'skills.stateBroken',
}

const title = computed(() => {
  const base = `${props.target.toolName} · ${t(stateLabelKey[props.target.state])}`
  return props.target.detail ? `${base} · ${props.target.detail}` : base
})
</script>

<template>
  <button
    type="button"
    class="tool-chip"
    :class="[`state-${target.state}`, { busy }]"
    :title="title"
    :aria-label="title"
    @click.stop="emit('toggle', target)"
  >
    <span class="dot" />
    <span class="chip-label">{{ target.toolName }}</span>
  </button>
</template>

<style scoped>
.tool-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-2);
  font: inherit;
  font-size: 11px;
  font-weight: 500;
  line-height: 18px;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    color var(--dur) var(--ease),
    opacity var(--dur) var(--ease);
}

.tool-chip.busy {
  opacity: 0.6;
  pointer-events: none;
}

.tool-chip .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-3);
  flex: 0 0 auto;
}

/* 已启用：绿底 */
.tool-chip.state-linked {
  border-color: color-mix(in srgb, var(--success) 35%, transparent);
  background: var(--success-tint);
  color: var(--text-1);
}
.tool-chip.state-linked .dot {
  background: var(--success);
}

/* 已分叉：琥珀底，需人工处理 */
.tool-chip.state-drifted {
  border-color: color-mix(in srgb, var(--warning) 40%, transparent);
  background: var(--warning-tint);
  color: var(--text-1);
}
.tool-chip.state-drifted .dot {
  background: var(--warning);
}

/* 链接失效：红点 + 中性底 */
.tool-chip.state-broken {
  border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  background: var(--danger-tint);
}
.tool-chip.state-broken .dot {
  background: var(--danger);
}

/* 未启用：低灰，仅在「未启用工具」弹出层里出现 */
.tool-chip.state-missing {
  border-color: transparent;
  background: var(--bg-2);
  color: var(--text-3);
  opacity: 0.75;
}
.tool-chip.state-missing:hover {
  opacity: 1;
  color: var(--text-2);
}
</style>
