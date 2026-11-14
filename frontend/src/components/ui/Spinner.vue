<script setup lang="ts">
/**
 * Spinner：加载环单源组件。
 * - 尺寸 1em 跟随父级 font-size，颜色 currentColor（放在有语义色的容器里自动同色）；
 * - 默认纯装饰（aria-hidden）；传 label 时升级为 role=status 的可感知状态；
 * - 旋转动画在 prefers-reduced-motion 下由全局例外层降级为透明度脉冲。
 */
withDefaults(
  defineProps<{
    /** 无障碍状态描述（如「运行中」）：传入后渲染 role=status。 */
    label?: string
  }>(),
  { label: '' },
)
</script>

<template>
  <span
    class="miyo-spinner"
    :role="label ? 'status' : undefined"
    :aria-label="label || undefined"
    :aria-hidden="label ? undefined : 'true'"
  ></span>
</template>
