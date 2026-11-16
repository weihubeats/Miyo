/**
 * EndpointEditor 配置页签回归：**保存过 active_tab 的接口，点页签必须切得动**。
 *
 * 用户实测 bug：新建接口（如 `/aa`）保存后点 Headers 没反应、其他页签也点不动。
 * 根因：`activeTab` 的 getter 把草稿保存的 `active_tab` 放在首位，setter 只写
 * 本地 `smartTab`——只要草稿里有保存值（改 Method 时 applyMethodDefaults 写入、
 * 保存即落库），getter 就永远回保存值，点击被静默吞掉。
 *
 * 修复契约：手动点击 > 保存值 > 智能默认；保存值只用作「打开接口时的开场页签」，
 * 且点击仍不写回草稿（不误判「有改动」）。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import { stubScrollIntoView } from '@/testUtils/componentTest'
import type { Endpoint } from '@/types/miyoApi'

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => new Proxy({}, { get: () => () => Promise.resolve(null) }),
}))

async function mountEditor(ep: Endpoint) {
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
  store.endpoints = [ep]
  const wrapper = mount(EndpointEditor, {
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await nextTick()
  store.openEndpoint(ep)
  await nextTick()
  return { wrapper, store }
}

/** 带保存页签的接口（模拟「改过 Method 再保存」后的活跃页签）。 */
function draftWithTab(tab: string, overrides: Partial<Endpoint> = {}): Endpoint {
  const ep = makeDraft({ id: 'ep-tabs', name: '新建接口', method: 'GET', path: '/aa', ...overrides })
  ep.request.active_tab = tab
  return ep
}

function tabOf(wrapper: Awaited<ReturnType<typeof mountEditor>>['wrapper'], label: string) {
  return wrapper.findAll('.tab').find((b) => b.text().startsWith(label))!
}

describe('EndpointEditor：配置页签点击（已保存 active_tab 的接口）', () => {
  it('保存的 active_tab=params 时点 Headers：切过去并渲染请求头面板', async () => {
    const { wrapper } = await mountEditor(draftWithTab('params'))

    expect(wrapper.findComponent({ name: 'HeadersPanel' }).exists()).toBe(false)
    await tabOf(wrapper, 'Headers').trigger('click')
    await nextTick()

    expect(tabOf(wrapper, 'Headers').classes()).toContain('active')
    expect(wrapper.findComponent({ name: 'HeadersPanel' }).exists()).toBe(true)
    wrapper.unmount()
  })

  it('保存的 active_tab=body 时 Params / Auth / Path / Body 互相切换都生效', async () => {
    const { wrapper } = await mountEditor(draftWithTab('body'))

    for (const [label, panel] of [
      ['Params', 'ParamsPanel'],
      ['Auth', 'AuthPanel'],
      ['Path', 'PathVariablesPanel'],
      ['Body', 'BodyPanel'],
    ] as const) {
      await tabOf(wrapper, label).trigger('click')
      await nextTick()
      expect(wrapper.findComponent({ name: panel }).exists()).toBe(true)
    }
    wrapper.unmount()
  })

  it('点击页签不写回草稿 active_tab（保存后点页签不该冒「有改动」圆点）', async () => {
    const ep = draftWithTab('params')
    const { wrapper, store } = await mountEditor(ep)

    await tabOf(wrapper, 'Headers').trigger('click')
    await nextTick()

    expect(store.draftOf(ep.id)?.request.active_tab).toBe('params')
    expect(store.isDirty(ep.id)).toBe(false)
    wrapper.unmount()
  })

  it('切换接口：页签回到新接口自己的保存值（不吃上一个接口的手动选择）', async () => {
    const a = draftWithTab('params', { id: 'ep-a', path: '/aa' })
    const b = draftWithTab('body', { id: 'ep-b', path: '/bb' })
    const { wrapper, store } = await mountEditor(a)

    await tabOf(wrapper, 'Headers').trigger('click')
    await nextTick()
    store.endpoints = [a, b]
    store.openEndpoint(b)
    await nextTick()

    expect(tabOf(wrapper, 'Body').classes()).toContain('active')
    wrapper.unmount()
  })

  it('同一接口内改 Method 仍跟随智能默认（手动选择让位给方法默认）', async () => {
    const ep = draftWithTab('params')
    const { wrapper, store } = await mountEditor(ep)

    await tabOf(wrapper, 'Headers').trigger('click')
    await nextTick()
    store.draftOf(ep.id)!.method = 'POST'
    await vi.waitFor(() => expect(store.draftOf(ep.id)!.request.active_tab).toBe('body'))
    await nextTick()

    expect(tabOf(wrapper, 'Body').classes()).toContain('active')
    wrapper.unmount()
  })
})
