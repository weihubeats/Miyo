/**
 * escStack：ESC 层级仲裁。
 * 锁定「后开先关」语义——Modal 里开子弹层（菜单/下拉/确认气泡）按一次
 * ESC 只关子弹层，不连外层一起关（此前 document 双监听无仲裁即此缺陷）。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { pushEscHandler } from './escStack'

function esc(target: Window | Document = window): void {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
}

const offs: (() => void)[] = []
afterEach(() => {
  while (offs.length) offs.pop()!()
})

describe('escStack', () => {
  it('后开先关：一次 ESC 只关最顶层', () => {
    const closeModal = vi.fn()
    const closeMenu = vi.fn()
    offs.push(pushEscHandler(closeModal))
    offs.push(pushEscHandler(closeMenu))
    esc()
    expect(closeMenu).toHaveBeenCalledTimes(1)
    expect(closeModal).not.toHaveBeenCalled()
  })

  it('顶层关闭后再按 ESC 关下一层', () => {
    const closeModal = vi.fn()
    const closeMenu = vi.fn()
    offs.push(pushEscHandler(closeModal))
    const offMenu = pushEscHandler(closeMenu)
    esc()
    offMenu()
    esc()
    expect(closeMenu).toHaveBeenCalledTimes(1)
    expect(closeModal).toHaveBeenCalledTimes(1)
  })

  it('处理器返回 false 时放行给更低层', () => {
    const closeLow = vi.fn()
    offs.push(pushEscHandler(closeLow))
    offs.push(
      pushEscHandler(() => {
        return false // 本层不响应
      }),
    )
    esc()
    expect(closeLow).toHaveBeenCalledTimes(1)
  })

  it('卸载函数弹出处理器：泄漏会挡住更低层收 ESC', () => {
    const closeLow = vi.fn()
    offs.push(pushEscHandler(closeLow))
    const offHigh = pushEscHandler(vi.fn())
    offHigh()
    esc()
    expect(closeLow).toHaveBeenCalledTimes(1)
  })

  it('window 与 document 双路径派发只消费一次', () => {
    const close = vi.fn()
    offs.push(pushEscHandler(close))
    esc(document)
    esc(window)
    expect(close).toHaveBeenCalledTimes(2) // 两次独立按键各消费一次
    // 同一事件对象经 document→window 冒泡不重复消费
    const ev = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })
    document.dispatchEvent(ev)
    window.dispatchEvent(ev)
    expect(close).toHaveBeenCalledTimes(3)
  })
})
