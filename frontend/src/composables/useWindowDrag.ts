/**
 * useWindowDrag：无边框/自定义标题栏窗口拖拽。
 * 绑定容器 mousedown：空白处（非交互元素）左键 → startDragging；双击 → 切换最大化。
 * 用法：给标题栏元素 ref，`useWindowDrag(topBarRef)`。
 *
 * 浏览器预览（vite dev / 非 Tauri 宿主）里没有窗口 API：getCurrentWindow()
 * 在 import 期就会抛 `reading 'metadata'`，连带初始路由导航失败整页白屏。
 * 因此按需惰性取窗口，且仅 Tauri 内绑定监听。
 */
import { onBeforeUnmount, watch, type Ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '../utils/platform'

type TauriWindow = ReturnType<typeof getCurrentWindow>

/** 惰性单例：只在真正需要拖拽时初始化（浏览器预览不会触碰）。 */
let appWindow: TauriWindow | null = null
function win(): TauriWindow {
  appWindow ??= getCurrentWindow()
  return appWindow
}

/**
 * 标题栏里的「自管手势」区域：标签条靠 pointer 事件做横向平移，窗口拖拽必须让位。
 * `data-no-window-drag` 同时用于新组件，避免每个宿主重复列选择器。
 */
const NO_DRAG_SELECTOR =
  'button, input, select, textarea, a, [contenteditable="true"], [data-no-window-drag], .proj-tabs, .tab-scroll'

function onMouseDown(event: MouseEvent): void {
  if (event.button !== 0) return
  const target = event.target as HTMLElement
  if (target.closest(NO_DRAG_SELECTOR)) return
  if (event.detail === 2) {
    void win().toggleMaximize()
  } else {
    void win().startDragging()
  }
}

export function useWindowDrag(container: Ref<HTMLElement | null>): void {
  // 浏览器预览无桌面窗口：不绑定 mousedown，保留原有标题栏交互
  if (!isTauri()) return
  watch(container, (el, old) => {
    old?.removeEventListener('mousedown', onMouseDown)
    el?.addEventListener('mousedown', onMouseDown)
  })
  onBeforeUnmount(() => {
    container.value?.removeEventListener('mousedown', onMouseDown)
  })
}