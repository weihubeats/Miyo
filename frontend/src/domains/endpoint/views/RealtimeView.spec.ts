/**
 * RealtimeView 单测：WS/SSE 双页签渲染与切换（不建真实连接）。
 * 契约回归：binary/ping 帧按后端 WsSendArgs 契约编码为 base64（历史 bug：
 * Ping 直传原文被误解码）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const routerMock = vi.hoisted(() => ({ push: vi.fn(), back: vi.fn() }))
const apiMock = vi.hoisted(() => ({
  wsConnect: vi.fn(async () => 'conn-1'),
  wsSend: vi.fn(async () => undefined),
  wsDisconnect: vi.fn(async () => true),
  sseConnect: vi.fn(async () => 'conn-sse'),
  sseDisconnect: vi.fn(async () => true),
}))

vi.mock('vue-router', () => ({ useRouter: () => routerMock }))
vi.mock('@/composables/useMiyoApi', () => ({ useMiyoApi: () => apiMock }))

import RealtimeView from '@/domains/endpoint/views/RealtimeView.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  routerMock.push.mockClear()
  routerMock.back.mockClear()
  apiMock.wsConnect.mockClear()
  apiMock.wsSend.mockClear()
  localStorage.clear()
})

function mountView() {
  // 复用 beforeEach 建好的 pinia（zh 已固定）：再建新 pinia 会让组件拿到
  // 默认语言的第二个 locale store（文案断言随系统漂移）
  return mount(RealtimeView, { attachTo: document.body })
}

describe('RealtimeView', () => {
  it('默认展示 WebSocket 页（未连接徽章 + 空日志提示 + 默认 URL）', () => {
    const wrapper = mountView()
    const badge = wrapper.find('.miyo-status')
    expect(badge.classes()).toContain('tone-info')
    expect(badge.text()).toContain('未连接')
    expect(wrapper.text()).toContain('连接后在此查看收发的帧')
    expect(
      (wrapper.find('input[placeholder="ws://127.0.0.1:4010/socket"]').element as HTMLInputElement)
        .value,
    ).toBe('ws://127.0.0.1:4010')
    // 帧日志容器：role=log + 位于分栏首栏
    const log = document.querySelector('.fl-list')
    expect(log?.getAttribute('role')).toBe('log')
    expect(log?.closest('.sp-first')).toBeTruthy()
    wrapper.unmount()
  })

  it('切换到 SSE 页签展示订阅入口与空日志提示', async () => {
    const wrapper = mountView()
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.text())).toEqual(['WebSocket', 'SSE'])
    await tabs[1]!.trigger('click')
    expect(wrapper.text()).toContain('未订阅')
    expect(wrapper.text()).toContain('订阅后在此查看事件流')
    wrapper.unmount()
  })

  it('高级连接参数可展开（请求头 + 仅 WS 有子协议），自动重连开关在 WS 侧', async () => {
    const wrapper = mountView()
    // 受控折叠：初始收起（内容常驻 grid 0fr，展开态看 .open 类）
    expect(wrapper.find('.rt-adv').classes()).not.toContain('open')

    await wrapper.find('.rt-adv .sec-head').trigger('click')
    expect(wrapper.find('.rt-adv').classes()).toContain('open')
    expect(wrapper.find('.rt-sub-input').exists()).toBe(true)
    expect(wrapper.find('.rt-switch').exists()).toBe(true)

    // SSE 页：同样可展开请求头，但没有子协议 / 自动重连（仅 WS 生效）
    await wrapper.findAll('[role="tab"]')[1]!.trigger('click')
    await wrapper.find('.rt-adv .sec-head').trigger('click')
    expect(wrapper.find('.rt-adv').classes()).toContain('open')
    expect(wrapper.find('.rt-sub-input').exists()).toBe(false)
    expect(wrapper.find('.rt-switch').exists()).toBe(false)
    wrapper.unmount()
  })

  it('返回按钮：有来路时走 router.back()，深链直开兜底回仪表板', async () => {
    window.history.replaceState({ back: '/projects' }, '')
    const wrapper = mountView()
    const back = wrapper.findAll('button').find((b) => b.attributes('aria-label') === '返回进入实时调试前的页面')
    expect(back).toBeTruthy()
    await back!.trigger('click')
    expect(routerMock.back).toHaveBeenCalledTimes(1)
    expect(routerMock.push).not.toHaveBeenCalled()
    wrapper.unmount()

    window.history.replaceState({ back: null }, '')
    routerMock.back.mockClear()
    routerMock.push.mockClear()
    const w2 = mountView()
    const back2 = w2.findAll('button').find((b) => b.attributes('aria-label') === '返回进入实时调试前的页面')
    await back2!.trigger('click')
    expect(routerMock.back).not.toHaveBeenCalled()
    expect(routerMock.push).toHaveBeenCalledWith('/projects')
    w2.unmount()
  })

  it('契约回归：binary / ping 帧的 payload 编码为 base64（text 帧发原文）', async () => {
    const wrapper = mountView()
    const vm = wrapper.vm as unknown as {
      __testSetConn: (state: 'idle' | 'connecting' | 'open' | 'closed' | 'error', connId: string | null) => void
      wsSend: (frame: 'text' | 'binary' | 'ping') => Promise<void>
      // expose 的 ref 在实例代理上已解包：直接读数组
      wsLog: unknown[]
      sendHistory: string[]
    }
    vm.__testSetConn('open', 'conn-1')

    const { toBase64Utf8 } = await import('@/utils/base64')
    const input = wrapper.find('.rt-send-input')
    await input.setValue('{"amount": 100}')

    // 文本帧：原文直发
    await input.trigger('keydown.enter')
    expect(apiMock.wsSend).toHaveBeenLastCalledWith({
      connection_id: 'conn-1',
      frame: 'text',
      payload: '{"amount": 100}',
    })

    // 二进制帧：UTF-8 base64 编码（经帧类型选择器切到 binary）
    wrapper.findComponent(CustomSelect).vm.$emit('update:modelValue', 'binary')
    await wrapper.vm.$nextTick()
    await input.setValue('中文二进制')
    await input.trigger('keydown.enter')
    expect(apiMock.wsSend).toHaveBeenLastCalledWith({
      connection_id: 'conn-1',
      frame: 'binary',
      payload: toBase64Utf8('中文二进制'),
    })

    // Ping：同样按契约编码（历史 bug：直传原文被后端按 base64 误解码）
    await input.setValue('heartbeat')
    const ping = wrapper.findAll('button').find((b) => b.text().trim() === 'Ping')
    await ping!.trigger('click')
    expect(apiMock.wsSend).toHaveBeenLastCalledWith({
      connection_id: 'conn-1',
      frame: 'ping',
      payload: toBase64Utf8('heartbeat'),
    })

    // 发送历史入栈（去重连续重复）
    expect(vm.sendHistory).toEqual(['{"amount": 100}', '中文二进制', 'heartbeat'])
    // 出站帧进日志（binary 帧日志显示编码后的 payload）
    expect((vm.wsLog as { dir: string }[]).filter((m) => m.dir === 'out')).toHaveLength(3)
    wrapper.unmount()
  })
})
