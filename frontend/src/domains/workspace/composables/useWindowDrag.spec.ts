/**
 * useWindowDrag 单测：顶栏哪些区域不该接管为「拖窗口」。
 *
 * 回归「按下去往右拖，标签条一点不动」（v0.0.57 用户实测）：顶栏整体绑了
 * mousedown → startDragging，指针一按下就被窗口管理器接管，标签条的
 * pointermove 再也收不到。项目标签是 `<div class="proj-tab">`、标签条是
 * `<div class="proj-tabs">`，都不在 button/input/a 这类选择器里，必须显式豁免。
 *
 * useWindowDrag 在组件内 watch 容器 ref，故用 mount 承载（裸调用不会触发 watch）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'

// vi.mock 工厂会被提升到 import 之前：共享 spy 必须走 vi.hoisted
const { startDragging, toggleMaximize } = vi.hoisted(() => ({
  startDragging: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
}))

vi.mock('@tauri-apps/api/window', () => ({
  // 稳定对象：useWindowDrag 内部惰性单例缓存窗口，每次调用返回新对象会拿不到 spy
  getCurrentWindow: () => ({ startDragging, toggleMaximize }),
}))

import { useWindowDrag } from '@/domains/workspace/composables/useWindowDrag'

const Host = defineComponent({
  setup() {
    const bar = ref<HTMLElement | null>(null)
    useWindowDrag(bar)
    return () => h('div', { ref: bar, class: 'bar' })
  },
})

describe('useWindowDrag：标签条豁免', () => {
  let wrapper: ReturnType<typeof mount>
  let bar: HTMLElement

  beforeEach(async () => {
    vi.clearAllMocks()
    // 让 isTauri() 为真：真实监听才会绑定
    ;(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
    wrapper = mount(Host, { attachTo: document.body })
    await nextTick()
    bar = wrapper.find('.bar').element as HTMLElement
  })

  function mouseDown(target: Element, detail = 1): void {
    target.dispatchEvent(
      new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0, detail }),
    )
  }

  it('空白处按下 → 拖动窗口', () => {
    mouseDown(bar)
    expect(startDragging).toHaveBeenCalledTimes(1)
  })

  it('项目标签 .proj-tab 上按下 → 不拖窗口（留给标签条横拖平移）', () => {
    bar.innerHTML = '<div class="proj-tabs"><div class="proj-tab"><span>项目</span></div></div>'
    const tab = bar.querySelector<HTMLElement>('.proj-tab')!
    mouseDown(tab)
    mouseDown(tab.firstElementChild!)
    expect(startDragging).not.toHaveBeenCalled()
  })

  it('标签条 .proj-tabs 空白处按下同样豁免', () => {
    bar.innerHTML = '<div class="proj-tabs"></div>'
    mouseDown(bar.querySelector<HTMLElement>('.proj-tabs')!)
    expect(startDragging).not.toHaveBeenCalled()
  })

  it('接口页签条 .tab-scroll 同样豁免', () => {
    bar.innerHTML =
      '<div class="tab-scroll"><div class="tab"><span class="tab-title">接口</span></div></div>'
    mouseDown(bar.querySelector<HTMLElement>('.tab-scroll')!)
    mouseDown(bar.querySelector<HTMLElement>('.tab-title')!)
    expect(startDragging).not.toHaveBeenCalled()
  })

  it('data-no-window-drag 标记的元素豁免（供新组件复用）', () => {
    bar.innerHTML = '<div data-no-window-drag><span>手势区</span></div>'
    mouseDown(bar.querySelector<HTMLElement>('[data-no-window-drag]')!)
    expect(startDragging).not.toHaveBeenCalled()
  })

  it('双击空白处 → 切换最大化而不是拖动', () => {
    mouseDown(bar, 2)
    expect(toggleMaximize).toHaveBeenCalledTimes(1)
    expect(startDragging).not.toHaveBeenCalled()
  })
})