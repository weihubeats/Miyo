import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  DRAG_LOCK_ATTR,
  REORDER_ANIMATION_MS,
  REORDER_EASING,
  bindDragGuards,
  endDragCursor,
} from '@/utils/sortable'
import type Sortable from 'sortablejs'

/** 只保留 option() 的 Sortable 替身（guard 只改 onEnd 一个回调）。 */
function fakeSortable() {
  const option = vi.fn<(name: string, value?: unknown) => void>()
  return { option } as unknown as { option: typeof option } & Sortable
}

/** 造一个容器 + 一张「卡片」，记录卡片上补发的事件。 */
function setup() {
  const container = document.createElement('div')
  const card = document.createElement('span')
  card.textContent = '小奏技术 · 用户服务'
  container.appendChild(card)
  document.body.appendChild(container)
  const seen: string[] = []
  for (const type of ['click', 'dblclick']) {
    card.addEventListener(type, () => seen.push(type))
  }
  const instance = fakeSortable()
  const unbind = bindDragGuards(instance, { container })
  return { container, card, seen, instance, unbind }
}

const at = (x: number, y: number) => ({ clientX: x, clientY: y })

afterEach(() => {
  document.body.innerHTML = ''
  endDragCursor()
})

describe('bindDragGuards', () => {
  it('按下即加禁选标记（指针还没动就已禁选，杜绝 WKWebView 反白整页）', () => {
    const { container, card } = setup()
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(true)
  })

  it('纯点击：mouseup 时补发 click 与 dblclick，随后解锁', () => {
    vi.useFakeTimers()
    const { container, card, seen } = setup()
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    card.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, button: 0, ...at(11, 12) }))
    expect(seen).toEqual(['click', 'dblclick'])
    // 解锁延后到双击间隔之后，保证「点两下」的第二次点击不被前一次的禁选吃掉
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(true)
    vi.advanceTimersByTime(500)
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(false)
    vi.useRealTimers()
  })

  it('指针位移超过阈值即视为拖动：不补发 click', () => {
    const { card, seen } = setup()
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    card.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, ...at(60, 40) }))
    card.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, button: 0, ...at(60, 40) }))
    expect(seen).toEqual([])
  })

  it('按钮 / 链接 / 输入框不接管（它们自己要收点击）', () => {
    const container = document.createElement('div')
    const button = document.createElement('button')
    container.appendChild(button)
    document.body.appendChild(container)
    const unbind = bindDragGuards(fakeSortable(), { container })
    button.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(5, 5) }))
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(false)
    unbind()
  })

  it('拖拽结束时（onEnd）摘掉禁选标记并解锁全局光标', () => {
    const { container, card, instance } = setup()
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    card.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, ...at(80, 80) }))
    const onEnd = (
      instance as unknown as { option: { mock: { calls: unknown[][] } } }
    ).option.mock.calls.find(([k]) => k === 'onEnd')?.[1] as () => void
    expect(onEnd).toBeTypeOf('function')
    onEnd()
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(false)
    expect(document.body.classList.contains('dragging-dnd')).toBe(false)
  })

  it('解绑后按下不再禁选（容器卸载不留残留标记）', () => {
    const { container, card, unbind } = setup()
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(true)
    unbind()
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(false)
    card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, ...at(10, 10) }))
    expect(container.hasAttribute(DRAG_LOCK_ATTR)).toBe(false)
  })
})

describe('换位动画常量', () => {
  it('时长与缓动是两处共用值（接口树 / 项目卡一致）', () => {
    expect(REORDER_ANIMATION_MS).toBe(150)
    expect(REORDER_EASING).toBe('cubic-bezier(0.2, 0, 0, 1)')
  })
})