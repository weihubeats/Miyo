/**
 * useOverlayFullscreen：面板级 CSS 全屏（fixed 覆盖视口，非 requestFullscreen）。
 *
 * 从 ResponsePanel 的全屏模式抽出：开/关只翻转 `fullscreen`，Esc 关闭交给
 * escStack 仲裁（后开先关——抽屉里开全屏时按 Esc 只退全屏不关抽屉）。
 * body 滚动锁不在此管理（宿主 Modal/Drawer 已锁；嵌套场景各管各的会互相解锁）。
 *
 * 样式约定：宿主根元素加 `fullscreen` class，用
 * `position: fixed; inset: 14px; z-index: var(--z-overlay)` 落位。
 */
import { onBeforeUnmount, ref, watch } from 'vue'
import { pushEscHandler } from '@/utils/escStack'

export function useOverlayFullscreen() {
  const fullscreen = ref(false)

  let offEsc: (() => void) | null = null
  watch(fullscreen, (v) => {
    offEsc?.()
    offEsc = null
    if (v) {
      offEsc = pushEscHandler(() => {
        fullscreen.value = false
        return true
      })
    }
  })

  function toggle(): void {
    fullscreen.value = !fullscreen.value
  }

  onBeforeUnmount(() => {
    offEsc?.()
    offEsc = null
  })

  return { fullscreen, toggle }
}
