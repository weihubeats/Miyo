/**
 * EndpointEditor 回归测试：请求/响应分割态 **按接口独立**，且跨视图切换不丢。
 *
 * 背景（用户实测 bug）：把请求区高度拖大后切到「设计 / 文档 / 用例 / Mock」
 * 再切回「调试」，高度回滚成默认值。历史上踩过三个坑：
 * 1. 切接口时手动分割态整份归零 → 每个接口都记不住自己拖过的高度；
 * 2. ui.activeView 切换整棵卸载请求/响应栈 → 组件级状态一并丢失（改 v-show）；
 * 3. 一刀切成「全局共享高度」→ 一个接口拖高，其他接口跟着变（用户实测否决）。
 * 现在的契约：高度/手动标记按接口 id 分桶（各自独立、模块级保留、落 localStorage），
 * 切视图不回滚；新接口从未拖过则仍是 grow 贴内容布局。
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

async function mountEditor() {
  setActivePinia(createPinia())
  stubScrollIntoView()
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
  const wrapper = mount(EndpointEditor, {
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await nextTick()
  return { wrapper, store }
}

/** 拖拽分割条（mousedown → mousemove → mouseup），返回落定后的内联高度。 */
async function dragSplitter(
  wrapper: Awaited<ReturnType<typeof mountEditor>>['wrapper'],
  dy: number,
): Promise<number> {
  const startY = 200
  await wrapper.find('.rp-splitter').trigger('mousedown', { button: 0, clientY: startY })
  document.dispatchEvent(new MouseEvent('mousemove', { clientY: startY + dy }))
  document.dispatchEvent(new MouseEvent('mouseup'))
  await nextTick()
  const style = wrapper.find('.config-box').attributes('style') ?? ''
  return Number(/height:\s*(\d+)px/.exec(style)?.[1] ?? NaN)
}

describe('EndpointEditor：分割态按接口独立 / 跨视图保留', () => {
  it('拖过分割条后打开 B：B 不继承 A 的高度（仍是 grow 贴内容布局）', async () => {
    const { wrapper, store } = await mountEditor()

    store.openEndpoint(endpointA)
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('grow')

    const height = await dragSplitter(wrapper, 60)
    expect(Number.isFinite(height)).toBe(true)
    expect(wrapper.find('.config-box').classes()).not.toContain('grow')

    store.openEndpoint(endpointB)
    await nextTick()
    const box = wrapper.find('.config-box')
    expect(box.classes()).toContain('grow')
    expect(box.attributes('style')).toBeUndefined()

    // 切回 A：A 自己的高度还在
    store.openEndpoint(endpointA)
    await nextTick()
    expect(wrapper.find('.config-box').attributes('style') ?? '').toContain(`height: ${height}px`)

    wrapper.unmount()
  })

  it('拖大后切到设计再切回调试：高度不回滚（用户实测 bug）', async () => {
    const { wrapper, store } = await mountEditor()

    store.openEndpoint(endpointA)
    await nextTick()
    const height = await dragSplitter(wrapper, 60)
    expect(Number.isFinite(height)).toBe(true)

    store.setActiveView('design')
    await nextTick()
    store.setActiveView('debug')
    await nextTick()

    const box = wrapper.find('.config-box')
    expect(box.classes()).not.toContain('grow')
    expect(box.attributes('style') ?? '').toContain(`height: ${height}px`)

    wrapper.unmount()
  })

  it('双击折叠后切走再切回：仍是折叠态；切到未拖过的接口则是 grow', async () => {
    const { wrapper, store } = await mountEditor()

    store.openEndpoint(endpointA)
    await nextTick()
    await wrapper.find('.rp-splitter').trigger('dblclick')
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('collapsed')

    store.setActiveView('mock')
    await nextTick()
    store.setActiveView('debug')
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('collapsed')

    // 未拖过的接口不受影响
    store.openEndpoint(endpointB)
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('grow')


    // 切回 A 仍折叠，再双击恢复默认高
    store.openEndpoint(endpointA)
    await nextTick()
    expect(wrapper.find('.config-box').classes()).toContain('collapsed')

    // 再双击恢复默认高：恢复了即「有手动固定高」且不是折叠态
    await wrapper.find('.rp-splitter').trigger('dblclick')
    await nextTick()
    const box = wrapper.find('.config-box')
    expect(box.classes()).not.toContain('collapsed')
    expect(box.attributes('style') ?? '').toContain('height: 200px')

    wrapper.unmount()
  })
})