<script setup lang="ts">
/**
 * MethodText：HTTP 方法彩色文本标识（mono、加粗、语义色；双主题跟随令牌）。
 */
import { methodTextTone } from '@/utils/methodTone'

withDefaults(defineProps<{ method: string }>(), {})
</script>

<template>
  <span class="mt" :class="methodTextTone(method)">{{ method }}</span>
</template>

<style scoped>
.mt {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.03em;
  line-height: 1.5;
  white-space: nowrap;
}
</style>