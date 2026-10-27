/**
 * EndpointEditor 回归测试：请求/响应分割高度是**按接口独立**的。
 *
 * 用户实测的两条要求（历史上反复踩坑）：
 * 1. 拖动高后切到别的接口再切回来，本接口的高度不能回落到默认值；
 * 2. 一个接口改了高度，**不能**带着其他接口一起变——每个接口各自记自己的高度。
 *
 * 单独成文件的原因：分割态是模块级 Map（跨组件重建保留），vitest 默认按文件
 * isolate 模块，跨文件不可见；这里用同一文件串起「拖 A → 切 B → 回 A → 再拖」。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from './EndpointEditor.vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import { stubScrollIntoView } from '../testUtils/componentTest'
import type { Endpoint } from '../types/miyoApi'

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => new Proxy({}, { get: () => () => Promise.resolve(null) }),
}))

const endpointA: Endpoint = makeDraft({ id: 'ep-a', name: '接口 A', method: 'POST', path: '/a' })
const endpointB: Endpoint = makeDraft({ id: 'ep-b', name: '接口 B', method: 'POST', path: '/b' })

function setupStore() {
  setActivePinia(createPinia())
  stubScrollIntoView()
  localStorage.clear()
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.endpoints = [endpointA, endpointB]
  return store
}

interface BoxWrapper {
  find: (selector: string) => { attributes: (name: string) => string | undefined; classes: () => string[] }
}

function inlineHeight(wrapper: BoxWrapper): number {
  const style = wrapper.find('.config-box').attributes('style') ?? ''
  return Number(/height:\s*(\d+)px/.exec(style)?.[1] ?? NaN)
}

async function drag(wrapper: Awaited<ReturnType<typeof mount>>, dy: number): Promise<number> {
  const startY = 200
  await wrapper.find('.rp-splitter').trigger('mousedown', { button: 0, clientY: startY })
  document.dispatchEvent(new MouseEvent('mousemove', { clientY: startY + dy }))
  document.dispatchEvent(new MouseEvent('mouseup'))
  await nextTick()
  return inlineHeight(wrapper)
}

describe('EndpointEditor：分割高度按接口独立', () => {
  it('切到设计视图再切回调试：高度不回滚，且不重复挂载懒加载面板', async () => {
    const store = setupStore()
    const wrapper = mount(EndpointEditor, {
      global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
    })
    await nextTick()
    store.openEndpoint(endpointA)
    await nextTick()

    // 设计/文档/用例/Mock 是懒加载面板：调试视图下不该被提前挂载
    expect(wrapper.find('.editor > .design').exists()).toBe(false)

    // 请求卡盒高：jsdom 不排版，手动给一个真实值
    const box = wrapper.find('.config-box').element as HTMLElement
    box.getBoundingClientRect = () =>
      ({ height: 240, width: 500, top: 0, left: 0, right: 500, bottom: 240, x: 0, y: 0 }) as DOMRect

    const dragged = await drag(wrapper, 80)
    expect(dragged).toBe(320)

    store.setActiveView('design')
    await nextTick()
    store.setActiveView('debug')
    await nextTick()
    await nextTick()

    const back = wrapper.find('.config-box')
    expect(back.classes()).not.toContain('grow')
    expect(inlineHeight(wrapper)).toBe(320)

    wrapper.unmount()
  })

  it('拖 A → 切 B（B 不受影响，仍是贴内容布局）→ 回 A（保持拖动值）→ 再拖从当前值起算', async () => {
    const store = setupStore()
    const wrapper = mount(EndpointEditor, {
      global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
    })
    await nextTick()

    store.openEndpoint(endpointA)
    await nextTick()

    // jsdom 不排版：给请求卡一个真实盒高，模拟 grow 布局下的可视高度。
    // editorEl.clientHeight 在 jsdom 为 0 → 请求区上限走兜底 600-100=500。
    const box = wrapper.find('.config-box').element as HTMLElement
    box.getBoundingClientRect = () =>
      ({ height: 260, width: 500, top: 0, left: 0, right: 500, bottom: 260, x: 0, y: 0 }) as DOMRect

    expect(wrapper.find('.config-box').classes()).toContain('grow')

    // A：向下拖 100px → 260 + 100 = 360（固定高）
    const heightA = await drag(wrapper, 100)
    expect(heightA).toBe(360)
    expect(wrapper.find('.config-box').classes()).not.toContain('grow')

    // B：从未拖过 → 仍是 grow 贴内容布局，不继承 A 的高度
    store.openEndpoint(endpointB)
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('grow')
    expect(wrapper.find('.config-box').attributes('style')).toBeUndefined()

    // B 自己拖：向下 20px → 260 + 20 = 280
    expect(await drag(wrapper, 20)).toBe(280)

    // 回到 A：仍是 A 自己的 360，不受 B 影响
    store.openEndpoint(endpointA)
    await nextTick()
    expect(wrapper.find('.config-box').classes()).not.toContain('grow')
    expect(inlineHeight(wrapper)).toBe(360)

    // A 再拖 -10px：从 360 起算（350），不从默认值跳变
    expect(await drag(wrapper, -10)).toBe(350)

    // 再确认 B 仍是自己的 280
    store.openEndpoint(endpointB)
    await nextTick()
    expect(inlineHeight(wrapper)).toBe(280)

    wrapper.unmount()
  })
})