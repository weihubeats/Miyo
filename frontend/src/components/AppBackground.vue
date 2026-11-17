/**
 * AppBackground：自定义主题背景图固定层（data-theme-id="custom" 且背景类型
 * 为图片时渲染）。纯色/渐变背景直接写 --bg-app（theme/apply.ts），无需本层。
 *
 * - 层级：fixed z-index 0，主内容（#app/.miyo-app，relative z-index 1）恒在其上；
 * - blur > 0 时层体向四周出血 32px，避免模糊在视口边缘产生透明羽化带；
 * - pointer-events: none，不拦截任何交互；随 store 配置实时更新（免刷新）。
 */
<script setup lang="ts">
import { computed } from 'vue'
import { useThemeStore } from '@/stores/theme'

const theme = useThemeStore()

const image = computed(() => {
  if (theme.themeId !== 'custom' || !theme.custom) return null
  const bg = theme.custom.background
  return bg.type === 'image' && bg.image ? bg.image : null
})

const layerStyle = computed(() => {
  const img = image.value
  if (!img) return null
  return {
    backgroundImage: `url("${img.data}")`,
    backgroundSize: img.fit,
    backgroundPosition: img.position || 'center',
    backgroundRepeat: 'no-repeat',
    opacity: String(img.opacity),
    filter: img.blur > 0 ? `blur(${img.blur}px)` : undefined,
  }
})
</script>

<template>
  <div v-if="layerStyle" class="app-bg" :class="{ 'app-bg-bleed': (image?.blur ?? 0) > 0 }" :style="layerStyle" aria-hidden="true" />
</template>

<style scoped>
.app-bg {
  position: fixed;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}
.app-bg-bleed {
  inset: -32px;
}
</style>
