/**
 * RequestExamplesPanel 单测：用例列表渲染 + 保存/回填/复制/删除的 store 接线。
 * workspace store 以模块级 mock 替换；locale 需真实 Pinia（组件 setup 调 useLocaleStore）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { reactive } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import RequestExamplesPanel from './RequestExamplesPanel.vue'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import type { Endpoint, RequestExample } from '../types/miyoApi'

const requestExamples = vi.hoisted(() => {
  const map = new Map<string, RequestExample[]>()
  return {
    map,
    set: (entries: [string, RequestExample[]][]) => {
      map.clear()
      for (const [k, v] of entries) map.set(k, v)
    },
    save: vi.fn(),
    apply: vi.fn(),
    remove: vi.fn(),
  }
})

vi.mock('../stores/workspace', () => ({
  useWorkspaceStore: () => ({
    requestExamples: requestExamples.map,
    saveRequestAsExample: requestExamples.save,
    applyRequestExample: requestExamples.apply,
    deleteRequestExample: requestExamples.remove,
  }),
}))

function draft(): Endpoint {
  const d = makeDraft({ id: 'ep-1', method: 'POST', path: '/funds/transfer' })
  return reactive(d)
}

function makeExample(name: string, overrides: Partial<RequestExample> = {}): RequestExample {
  return {
    id: crypto.randomUUID(),
    endpoint_id: 'ep-1',
    name,
    request: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'json', raw: '{"amount":100}' },
      active_tab: 'body',
      timeout_ms: 30000,
      follow_redirects: true,
      tests: null,
      disable_cookies: false,
    },
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    ...overrides,
  }
}

function mountPanel(d: Endpoint) {
  return mount(RequestExamplesPanel, { props: { draft: d } })
}

describe('RequestExamplesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    requestExamples.set([["ep-1", [makeExample("资金调拨-100")]]])
    requestExamples.save.mockReset()
    requestExamples.apply.mockReset()
    requestExamples.remove.mockReset()
  })

  it('渲染用例列表（最新在前）', () => {
    requestExamples.set([
      [
        'ep-1',
        [makeExample('资金调拨-100'), makeExample('资金调拨-200')],
      ],
    ])
    const wrapper = mountPanel(draft())
    const titles = wrapper.findAll('.rex-title').map((n) => n.text())
    expect(titles).toEqual(['资金调拨-100', '资金调拨-200'])
  })

  it('空列表展示 EmptyState', () => {
    requestExamples.set([])
    const wrapper = mountPanel(draft())
    expect(wrapper.text()).toContain('暂无请求用例')
  })

  it('名称默认 = method + path，点「保存当前请求」以快照调用 store', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    expect((wrapper.find('.rex-name').element as HTMLInputElement).value).toBe('POST /funds/transfer')

    await wrapper.find('.rex-save-btn').trigger('click')
    expect(requestExamples.save).toHaveBeenCalledTimes(1)
    const [endpointId, name, request] = requestExamples.save.mock.calls[0]
    expect(endpointId).toBe('ep-1')
    expect(name).toBe('POST /funds/transfer')
    expect(request).toBe(d.request)
    // 保存成功后名称复位为默认
    expect((wrapper.find('.rex-name').element as HTMLInputElement).value).toBe('POST /funds/transfer')
  })

  it('回填：调用 applyRequestExample，缺 active_tab 时按 Method 智能默认（POST → body）', async () => {
    const d = draft()
    const ex = makeExample('无tab', { request: { ...makeExample('x').request, active_tab: null } })
    requestExamples.set([['ep-1', [ex]]])
    const wrapper = mountPanel(d)

    await wrapper.find('.rex-row').trigger('dblclick')
    expect(requestExamples.apply).toHaveBeenCalledWith('ep-1', ex)
    expect(d.request.active_tab).toBe('body')
  })

  it('回填保留已保存的 active_tab', async () => {
    const d = draft()
    const ex = makeExample('带tab', { request: { ...makeExample('x').request, active_tab: 'headers' } })
    requestExamples.set([['ep-1', [ex]]])
    // 模拟真实 store：apply 把用例 request 深拷贝进草稿
    requestExamples.apply.mockImplementation((_id: string, e: RequestExample) => {
      d.request = JSON.parse(JSON.stringify(e.request))
    })
    const wrapper = mountPanel(d)

    await wrapper.find('.rex-row').trigger('dblclick')
    expect(d.request.active_tab).toBe('headers')
  })

  it('复制：以该用例的请求另存「名称 副本」', async () => {
    requestExamples.save.mockResolvedValue(true)
    const wrapper = mountPanel(draft())
    await wrapper.findAll('.rex-actions button')[1].trigger('click')
    const [endpointId, name, request] = requestExamples.save.mock.calls[0]
    expect(endpointId).toBe('ep-1')
    expect(name).toBe('资金调拨-100 副本')
    expect((request as Endpoint['request']).body).toEqual({ mode: 'json', raw: '{"amount":100}' })
  })

  it('删除：Popconfirm 确认后调用 deleteRequestExample', async () => {
    const expectedId = requestExamples.map.get('ep-1')![0].id
    const wrapper = mountPanel(draft())
    const buttons = wrapper.findAll('.rex-actions button')
    // 第三个按钮是 trash（download / copy / trash）
    await buttons[2].trigger('click')
    // Popconfirm 弹层 Teleport 到 body
    const confirm = Array.from(document.querySelectorAll('.pc-pop button')).find(
      (b) => b.textContent === '删除',
    )
    expect(confirm).toBeDefined()
    ;(confirm as HTMLButtonElement).click()
    await vi.waitFor(() => {
      expect(requestExamples.remove).toHaveBeenCalledWith('ep-1', expectedId)
    })
  })
})
