<script setup lang="ts">
/**
 * CollapsibleSection：可折叠分区（编辑器/抽屉内容分区统一外壳）。
 * - 头部 = 箭头 + 标题 + 可选计数徽章 + 右侧 actions 插槽；
 * - 受控/非受控两用：传 open 走受控（v-model:open），否则用内部态（defaultOpen）；
 * - 展开动画走 grid-template-rows 0fr→1fr（内容高度自适应），
 *   reduced-motion 下由全局基线块瞬时切换；
 * - a11y：头部是真实 button + aria-expanded/aria-controls。
 */
import { computed, ref } from 'vue'
import Icon from '@/components/ui/Icon.vue'

const props = withDefaults(
  defineProps<{
    title: string
    /** 计数徽章（undefined 不渲染）。 */
    count?: number
    /** 非受控时的初始展开态。 */
    defaultOpen?: boolean
    /** 受控展开态（v-model:open）。 */
    open?: boolean
    /** 紧凑模式（抽屉内嵌小分区）。 */
    dense?: boolean
  }>(),
  { count: undefined, defaultOpen: true, open: undefined, dense: false },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
}>()

const innerOpen = ref(props.defaultOpen)
const isOpen = computed({
  get: () => props.open ?? innerOpen.value,
  set: (v: boolean) => {
    innerOpen.value = v
    emit('update:open', v)
  },
})

function toggle(): void {
  isOpen.value = !isOpen.value
}

const bodyId = `miyo-sec-${Math.random().toString(36).slice(2, 9)}`
</script>

<template>
  <section class="sec" :class="{ dense, open: isOpen }">
    <button
      type="button"
      class="sec-head"
      :aria-expanded="isOpen"
      :aria-controls="bodyId"
      @click="toggle"
    >
      <Icon name="chevron-right" :size="12" class="sec-arrow" />
      <span class="sec-title">{{ title }}</span>
      <span v-if="count !== undefined" class="sec-count num">{{ count }}</span>
      <span v-if="$slots.actions" class="sec-actions" @click.stop>
        <slot name="actions" />
      </span>
    </button>
    <div :id="bodyId" class="sec-body" role="region" :aria-label="title">
      <div class="sec-body-inner">
        <slot />
      </div>
    </div>
  </section>
</template>

<style scoped>
.sec {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  overflow: hidden;
}
.sec.dense {
  border-radius: var(--radius);
}

.sec-head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  background: none;
  color: var(--text-2);
  font-family: inherit;
  font-size: var(--fs-xs);
  font-weight: 600;
  text-align: left;
  cursor: pointer;
  user-select: none;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.sec-head:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.sec.dense .sec-head {
  padding: 5px 8px;
  font-size: var(--fs-xxs);
}

.sec-arrow {
  flex: none;
  color: var(--text-3);
  transition: transform 160ms var(--ease);
}
.sec.open .sec-arrow {
  transform: rotate(90deg);
}

.sec-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sec-count {
  flex: none;
  min-width: 18px;
  padding: 0 5px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.5;
  text-align: center;
}

.sec-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

.sec-body {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 180ms var(--ease);
}
.sec.open .sec-body {
  grid-template-rows: 1fr;
}

.sec-body-inner {
  min-height: 0;
  overflow: hidden;
}
.sec.open .sec-body-inner {
  padding: 0 10px 10px;
}
.sec.dense.open .sec-body-inner {
  padding: 0 8px 8px;
}
/* 收起态不留内边距，避免 0fr 时仍占空 */
.sec:not(.open) .sec-body-inner {
  padding: 0;
}
</style>
