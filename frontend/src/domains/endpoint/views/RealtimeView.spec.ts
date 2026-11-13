/**
 * RealtimeView 单测：WS/SSE 双页签渲染与切换（不建真实连接）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const routerMock = vi.hoisted(() => ({ push: vi.fn(), back: vi.fn() }))

vi.mock('vue-router', () => ({ useRouter: () => routerMock }))

import RealtimeView from '@/domains/endpoint/views/RealtimeView.vue'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  routerMock.push.mockClear()
  routerMock.back.mockClear()
})

function mountView() {
  setActivePinia(createPinia())
  return mount(RealtimeView, { attachTo: document.body })
}

describe('RealtimeView', () => {
  it('默认展示 WebSocket 页（未连接态 + 空日志提示）', () => {
    const wrapper = mountView()
    expect(wrapper.text()).toContain('未连接')
    expect(wrapper.text()).toContain('连接后在此查看收发的帧')
    expect(
      (wrapper.find('input[placeholder="ws://127.0.0.1:4010/socket"]').element as HTMLInputElement)
        .value,
    ).toBe('ws://127.0.0.1:4010')
    wrapper.unmount()
  })

  it('切换到 SSE 页签展示订阅入口与空日志提示', async () => {
    const wrapper = mountView()
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.text())).toEqual(['WebSocket', 'SSE'])
    await tabs[1].trigger('click')
    expect(wrapper.text()).toContain('未订阅')
    expect(wrapper.text()).toContain('订阅后在此查看事件流')
    wrapper.unmount()
  })

  it('WS/SSE 可展开自定义请求头，仅 WS 提供子协议输入', async () => {
    const wrapper = mountView()
    // 默认折叠：只露出「请求头 (n)」开关
    expect(wrapper.find('.rt-adv-toggle').text()).toBe('请求头 (0)')
    expect(wrapper.find('.rt-adv-body').exists()).toBe(false)

    await wrapper.find('.rt-adv-toggle').trigger('click')
    expect(wrapper.find('.rt-adv-body').exists()).toBe(true)
    expect(wrapper.find('.rt-sub-input').exists()).toBe(true)

    // SSE 页：同样可展开请求头，但没有子协议（Sec-WebSocket-Protocol 仅 WS 生效）
    await wrapper.findAll('[role="tab"]')[1]!.trigger('click')
    await wrapper.find('.rt-adv-toggle').trigger('click')
    expect(wrapper.find('.rt-adv-body').exists()).toBe(true)
    expect(wrapper.find('.rt-sub-input').exists()).toBe(false)
    wrapper.unmount()
  })

  it('日志区在 flex 页签容器内撑满（block 包裹会断掉高度传递）', async () => {
    const wrapper = mountView()
    for (const tab of [0, 1]) {
      await wrapper.findAll('[role="tab"]')[tab].trigger('click')
      const log = document.querySelector('.rt-log') as HTMLElement
      expect(log).toBeTruthy()
      // .rt-log 的 offsetParent 链上必须是 .rt-pane（flex 纵向），否则 flex:1 失效
      expect(log.parentElement?.classList.contains('rt-pane')).toBe(true)
    }
    wrapper.unmount()
  })

  /**
   * 返回按钮必须原路返回：实时调试可从仪表板左侧导航进入，
   * 旧实现写死 push('/workspace')，从主页进来会被弹到某个项目里。
   */
  it('返回按钮：有来路时走 router.back()，回到进入前的页面', async () => {
    window.history.replaceState({ back: '/projects' }, '')
    const wrapper = mountView()
    const back = wrapper.findAll('button').find((b) => b.text().includes('返回'))
    expect(back).toBeTruthy()
    await back!.trigger('click')
    expect(routerMock.back).toHaveBeenCalledTimes(1)
    expect(routerMock.push).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('返回按钮：深链直开（无来路）时兜底回仪表板，不离开应用', async () => {
    window.history.replaceState({ back: null }, '')
    const wrapper = mountView()
    const back = wrapper.findAll('button').find((b) => b.text().includes('返回'))
    await back!.trigger('click')
    expect(routerMock.back).not.toHaveBeenCalled()
    expect(routerMock.push).toHaveBeenCalledWith('/projects')
    wrapper.unmount()
  })
})
