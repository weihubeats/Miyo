import { afterEach, describe, expect, it } from 'vitest'
import {
  DRAGGING_BODY_CLASS,
  beginDragCursor,
  endDragCursor,
  REORDER_ANIMATION_MS,
  REORDER_EASING,
} from '@/utils/sortable'

afterEach(() => {
  endDragCursor()
})

describe('拖拽全局光标态', () => {
  it('挂上与摘除 body 类，重复调用幂等', () => {
    beginDragCursor()
    beginDragCursor()
    expect(document.body.classList.contains(DRAGGING_BODY_CLASS)).toBe(true)
    endDragCursor()
    endDragCursor()
    expect(document.body.classList.contains(DRAGGING_BODY_CLASS)).toBe(false)
  })
})

describe('换位动画常量', () => {
  it('时长与缓动是两处共用值（接口树 / 项目卡一致）', () => {
    expect(REORDER_ANIMATION_MS).toBe(150)
    expect(REORDER_EASING).toBe('cubic-bezier(0.2, 0, 0, 1)')
  })
})