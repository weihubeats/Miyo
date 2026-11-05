/**
 * useSidebarResize：侧栏宽度分割状态机（自 WorkspaceView 抽出）。
 *
 * 职责（原 WorkspaceView「侧栏宽度分割」整段，行为不变）：
 * - 宽度持久化（localStorage，键 `miyo.sidebar-width`；不可用时仅丢持久化）；
 * - 拖拽（mousedown/move/up 三段，拖拽期间禁用文本选中）；
 * - 键盘调宽（← → 每次 10px，Shift 40px，Home/End 到上下限，Enter 恢复默认）。
 *
 * 常量导出供宿主模板使用（aria-valuemin / 双击恢复默认等）。
 */
import { ref } from 'vue'

export const SIDEBAR_MIN = 220
export const SIDEBAR_MAX = 520
export const SIDEBAR_DEFAULT = 300
/** 侧栏宽度持久化键：与请求区高度等「用户布局偏好」同一策略（localStorage）。 */
const SIDEBAR_WIDTH_KEY = 'miyo.sidebar-width'
/** 侧栏整体折叠持久化键（折叠 ≠ 宽度 0：折叠时侧栏连同分割条一起移出布局）。 */
const SIDEBAR_COLLAPSED_KEY = 'miyo.sidebar-collapsed'

function clampSidebarWidth(v: number): number {
  return Math.min(Math.max(v, SIDEBAR_MIN), SIDEBAR_MAX)
}

/** 拖拽 / 键盘后的宽度状态 + 持久化与交互句柄。 */
export function useSidebarResize() {
  function persistSidebarWidth(): void {
    try {
      localStorage.setItem(SIDEBAR_WIDTH_KEY, String(Math.round(sidebarWidth.value)))
    } catch {
      // 存储不可用时静默：仅影响重启后的宽度恢复
    }
  }

  const sidebarWidth = ref(
    clampSidebarWidth(Number(localStorage.getItem(SIDEBAR_WIDTH_KEY)) || SIDEBAR_DEFAULT),
  )
  const sidebarResizing = ref(false)
  let sidebarDragging = false
  let resizeStartX = 0
  let resizeStartWidth = 0

  /** 侧栏折叠（隐藏整栏，主区独占宽度）：持久化，重启后保持。 */
  const sidebarCollapsed = ref(
    (() => {
      try {
        return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === '1'
      } catch {
        return false
      }
    })(),
  )

  function toggleSidebarCollapsed(): void {
    sidebarCollapsed.value = !sidebarCollapsed.value
    try {
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, sidebarCollapsed.value ? '1' : '0')
    } catch {
      // 存储不可用时静默：仅影响重启后的折叠恢复
    }
  }

  function onSidebarResizeDown(event: MouseEvent): void {
    if (event.button !== 0) return
    event.preventDefault()
    sidebarDragging = true
    sidebarResizing.value = true
    resizeStartX = event.clientX
    resizeStartWidth = sidebarWidth.value
    document.body.style.userSelect = 'none'
    document.addEventListener('mousemove', onSidebarResizeMove)
    document.addEventListener('mouseup', onSidebarResizeUp)
  }

  function onSidebarResizeMove(event: MouseEvent): void {
    if (!sidebarDragging) return
    const next = resizeStartWidth + (event.clientX - resizeStartX)
    sidebarWidth.value = Math.min(Math.max(next, SIDEBAR_MIN), SIDEBAR_MAX)
  }

  function onSidebarResizeUp(): void {
    if (!sidebarDragging) return
    sidebarDragging = false
    sidebarResizing.value = false
    persistSidebarWidth()
    document.body.style.userSelect = ''
    document.removeEventListener('mousemove', onSidebarResizeMove)
    document.removeEventListener('mouseup', onSidebarResizeUp)
  }

  /** 键盘调宽（分割条可 Tab 聚焦）：← → 每次 10px，Shift 40px，Home/End 到上下限，Enter 恢复默认。 */
  function onSidebarKeydown(event: KeyboardEvent): void {
    const step = event.shiftKey ? 40 : 10
    let next = sidebarWidth.value
    if (event.key === 'ArrowLeft') next -= step
    else if (event.key === 'ArrowRight') next += step
    else if (event.key === 'Home') next = SIDEBAR_MIN
    else if (event.key === 'End') next = SIDEBAR_MAX
    else if (event.key === 'Enter') next = SIDEBAR_DEFAULT
    else return
    event.preventDefault()
    sidebarWidth.value = Math.min(Math.max(next, SIDEBAR_MIN), SIDEBAR_MAX)
    persistSidebarWidth()
  }

  /** 直接写宽度（双击恢复默认等场景），并落持久化。 */
  function setSidebarWidth(v: number): void {
    sidebarWidth.value = clampSidebarWidth(v)
    persistSidebarWidth()
  }

  /** 卸载清理：拖拽在途时摘除 document 级监听并复位选中态（宿主 onBeforeUnmount 调用）。 */
  function dispose(): void {
    if (!sidebarDragging) return
    sidebarDragging = false
    sidebarResizing.value = false
    document.body.style.userSelect = ''
    document.removeEventListener('mousemove', onSidebarResizeMove)
    document.removeEventListener('mouseup', onSidebarResizeUp)
  }

  return {
    SIDEBAR_MIN,
    SIDEBAR_MAX,
    SIDEBAR_DEFAULT,
    sidebarWidth,
    sidebarResizing,
    sidebarCollapsed,
    toggleSidebarCollapsed,
    onSidebarResizeDown,
    onSidebarKeydown,
    setSidebarWidth,
    dispose,
  }
}
