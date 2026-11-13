/**
 * WindowControls 单测：Windows 无边框窗口的自绘控制钮。
 * 锁定：非 Windows/Tauri 不渲染 / 三钮文案与点击动作 / 最大化态图标切换
 * （resize 事件跟随 isMaximized 状态）。
 */
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import WindowControls from '@/domains/workspace/components/WindowControls.vue'
import { useLocaleStore } from '@/stores/locale'

const mocks = vi.hoisted(() => ({
  /** showWindowControls() 的桩：逐用例切换可见性 */
  visible: true,
}))

/** getCurrentWindow() 桩：resize 回调留存以便手动触发。 */
let resizeCb: (() => void) | null = null
const unlisten = vi.fn()
const winMock = {
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async (cb: () => void) => {
    resizeCb = cb
    return unlisten
  }),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => winMock,
}))

vi.mock('@/utils/platform.ts', () => ({
  showWindowControls: () => mocks.visible,
}))

function mountControls() {
  return mount(WindowControls)
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  mocks.visible = true
  resizeCb = null
  vi.clearAllMocks()
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('WindowControls：Windows 自绘窗口控制钮', () => {
  it('非 Windows/Tauri 不渲染', () => {
    mocks.visible = false
    const wrapper = mountControls()

    expect(wrapper.find('.win-controls').exists()).toBe(false)

    wrapper.unmount()
  })

  it('渲染三钮：文案走 i18n，点击触发对应窗口动作', async () => {
    const wrapper = mountControls()
    await vi.waitFor(() => expect(winMock.onResized).toHaveBeenCalled())

    const btns = wrapper.findAll('.wc-btn')
    expect(btns.length).toBe(3)
    expect(btns[0].attributes('title')).toBe('最小化')
    expect(btns[1].attributes('title')).toBe('最大化')
    expect(btns[2].attributes('title')).toBe('关闭')

    await btns[0].trigger('click')
    await btns[1].trigger('click')
    await btns[2].trigger('click')
    expect(winMock.minimize).toHaveBeenCalledTimes(1)
    expect(winMock.toggleMaximize).toHaveBeenCalledTimes(1)
    expect(winMock.close).toHaveBeenCalledTimes(1)

    wrapper.unmount()
  })

  it('最大化态：中间钮切换为「还原」，resize 跟随状态回落', async () => {
    winMock.isMaximized.mockResolvedValue(true)
    const wrapper = mountControls()
    await vi.waitFor(() => expect(wrapper.find('.wc-btn:nth-child(2)').attributes('title')).toBe('还原'))

    // 窗口被拖离最大化 → resize 事件 → 重查后切回最大化图标
    winMock.isMaximized.mockResolvedValue(false)
    resizeCb?.()
    await vi.waitFor(() => expect(wrapper.find('.wc-btn:nth-child(2)').attributes('title')).toBe('最大化'))

    wrapper.unmount()
  })
})
