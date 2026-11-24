/** 拖动换位动画的统一时长（项目卡与接口树共用，用户要求两处手感一致）。 */
export const REORDER_ANIMATION_MS = 150

/** 拖动换位动画的统一缓动曲线（SortableJS easing + CSS 兜底共用）。 */
export const REORDER_EASING = 'cubic-bezier(0.2, 0, 0, 1)'

/** 拖拽期间给 body 挂的全局类（抓取光标 + 禁选，见 style.css `.dragging-dnd`）。 */
export const DRAGGING_BODY_CLASS = 'dragging-dnd'

/** 给 body 挂上全局拖拽态（幂等）。 */
export function beginDragCursor(): void {
  document.body.classList.add(DRAGGING_BODY_CLASS)
}

/** 摘掉全局拖拽态（幂等；卸载时兜底调用，防止异常退出留下抓取光标）。 */
export function endDragCursor(): void {
  document.body.classList.remove(DRAGGING_BODY_CLASS)
}