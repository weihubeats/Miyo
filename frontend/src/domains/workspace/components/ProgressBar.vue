<script setup lang="ts">
/**
 * ProgressBar：全局顶部加载进度条（配合 composables/useProgress.ts）。
 *
 * 固定悬浮在窗口顶部，2px 高，miyo- 品牌蓝渐变 + 前进流光动画（NProgress 风格）。
 * 在 App 根组件挂载一次：
 * ```html
 * <ProgressBar />
 * ```
 * useMiyoApi 的 `run()` 已自动 start/done，无需业务代码介入。
 */
import { useProgress } from '@/composables/useProgress'
import { useLocaleStore } from '@/stores/locale'

const { visible, progress } = useProgress()
const locale = useLocaleStore()
const t = locale.t
</script>

<template>
  <Transition name="miyo-progress">
    <div v-if="visible" class="miyo-progress" role="progressbar" :aria-label="t('common.loading')">
      <div class="miyo-progress-bar" :style="{ width: `${progress}%` }"></div>
    </div>
  </Transition>
</template>

<style scoped>
.miyo-progress {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  z-index: var(--z-progress);
  pointer-events: none;
}

.miyo-progress-bar {
  height: 100%;
  border-radius: 0 2px 2px 0;
  background: linear-gradient(90deg, var(--miyo-accent-weak), var(--miyo-accent) 60%, var(--miyo-info));
  box-shadow: 0 0 8px color-mix(in srgb, var(--info) 70%, transparent);
  transition: width 0.2s ease;
}

.miyo-progress-bar::after {
  content: '';
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 96px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.35), transparent);
  animation: miyo-progress-shine 1s linear infinite;
}

@keyframes miyo-progress-shine {
  from {
    transform: translateX(-96px);
  }
  to {
    transform: translateX(100vw);
  }
}

.miyo-progress-enter-active,
.miyo-progress-leave-active {
  transition: opacity 0.24s ease;
}

.miyo-progress-enter-from,
.miyo-progress-leave-to {
  opacity: 0;
}
</style>