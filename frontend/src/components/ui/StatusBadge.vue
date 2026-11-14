<script setup lang="ts">
/**
 * StatusBadge：状态徽章单源组件。
 * - 色彩/排版走 style.css 的 .miyo-status.tone-*（深浅主题自动跟随）；
 * - tone 与 useScenarioRun 的 StepTone 对齐，另含通用 warn/info 态；
 * - label 由调用方传入（已翻译文案）；dot=true 渲染前置状态点，
 *   tone-run 时圆点自带呼吸动画（reduced-motion 下降级为脉冲）。
 */
export type StatusTone =
  | 'ok'
  | 'fail'
  | 'warn'
  | 'info'
  | 'run'
  | 'wait'
  | 'skip'
  | 'blocked'
  | 'cancel'

withDefaults(
  defineProps<{
    tone: StatusTone
    label: string
    /** 前置状态点（tone-run 自带呼吸）。 */
    dot?: boolean
  }>(),
  { dot: false },
)
</script>

<template>
  <span class="miyo-status" :class="`tone-${tone}`">
    <span v-if="dot" class="st-dot" aria-hidden="true"></span>
    <span class="st-label">{{ label }}</span>
  </span>
</template>

<style scoped>
.st-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
