<script setup lang="ts">
/**
 * WindowControls：Windows 无边框窗口的自绘控制钮（最小化 / 最大化还原 / 关闭）。
 * - 仅在 Tauri + Windows 渲染：macOS 用系统红绿灯，Linux 保持原生标题栏；
 * - Rust 侧同步移除原生装饰（src-tauri main.rs），顶栏拖拽/双击最大化由 useWindowDrag 承担；
 * - 最大化态经 resize 事件跟随，切换 还原(copy)/最大化(square) 图标。
 */
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Icon from '@/components/ui/Icon.vue'
import { useLocaleStore } from '@/stores/locale'
import { showWindowControls } from '@/utils/platform'

const t = useLocaleStore().t
const visible = showWindowControls()
const maximized = ref(false)
let unlisten: (() => void) | null = null

/** resize 事件不带状态，重查一次 isMaximized（拖动还原/最大化/贴边分屏都会触发）。 */
async function syncMaximized(): Promise<void> {
  try {
    maximized.value = await getCurrentWindow().isMaximized()
  } catch {
    // 非 Tauri 环境：忽略
  }
}

onMounted(async () => {
  if (!visible) return
  void syncMaximized()
  try {
    unlisten = await getCurrentWindow().onResized(() => {
      void syncMaximized()
    })
  } catch {
    // 非 Tauri 环境：忽略
  }
})

onBeforeUnmount(() => {
  unlisten?.()
})

function minimize(): void {
  void getCurrentWindow().minimize()
}

function toggleMaximize(): void {
  void getCurrentWindow().toggleMaximize()
}

function close(): void {
  void getCurrentWindow().close()
}
</script>

<template>
  <div v-if="visible" class="win-controls">
    <button
      class="wc-btn"
      type="button"
      :title="t('window.minimize')"
      :aria-label="t('window.minimize')"
      @click="minimize"
    >
      <Icon name="minus" :size="14" />
    </button>
    <button
      class="wc-btn"
      type="button"
      :title="maximized ? t('window.restore') : t('window.maximize')"
      :aria-label="maximized ? t('window.restore') : t('window.maximize')"
      @click="toggleMaximize"
    >
      <Icon :name="maximized ? 'copy' : 'square'" :size="maximized ? 12 : 13" />
    </button>
    <button
      class="wc-btn wc-close"
      type="button"
      :title="t('common.close')"
      :aria-label="t('common.close')"
      @click="close"
    >
      <Icon name="x" :size="14" />
    </button>
  </div>
</template>

<style scoped>
/* Windows 惯例：贴顶栏右缘、通栏高、直角；关闭钮悬停系统红。 */
.win-controls {
  display: flex;
  align-items: stretch;
  align-self: stretch;
  flex-shrink: 0;
}

.wc-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  padding: 0;
  border: none;
  border-radius: 0;
  background: transparent;
  color: var(--text-2);
  cursor: default;
}

.wc-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.wc-btn:active {
  background: var(--bg-active);
}

.wc-close:hover {
  background: #e81123;
  color: #fff;
}

.wc-close:active {
  background: #f1707a;
  color: #fff;
}
</style>
