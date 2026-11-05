/**
 * EndpointEditor URL 栏增强单测：
 * - 路径变量缺失提示 chips：path 含 {id}/{{var}} 而 path_variables 未配置时出现，点击切到 Path 页签；
 * - 地址栏历史下拉：点选最近请求仅回填 Method + 地址（复用 applyUrlInput 归一化语义）。
 */
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import EndpointEditor from './EndpointEditor.vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { makeDraft } from '../testUtils/draftFixture'
import { stubScrollIntoView } from '../testUtils/componentTest'
import type { Endpoint, RequestHistory } from '../types/miyoApi'

const apiMock = vi.hoisted(() => ({
  executeRequest: vi.fn(async () => {
    throw new Error('not used')
  }),
  listExamples: vi.fn(async () => []),
  listRequestExamples: vi.fn(async () => []),
  listTestCases: vi.fn(async () => []),
  listHistories: vi.fn(async () => []),
  listEnvironments: vi.fn(async () => []),
  getActiveEnvironment: vi.fn(async () => null),
  getGlobalVariables: vi.fn(async () => []),
  getGlobalParams: vi.fn(async () => []),
}))

vi.mock('../composables/useMiyoApi', () => ({ useMiyoApi: () => apiMock }))

/** Menu 交互 stub：记录 openAt 传入的 items，渲染为可点击行（触发 select）。 */
const MenuStub = {
  name: 'MenuStub',
  props: [],
  emits: ['select', 'open', 'close'],
  data: () => ({ items: [] as { key: string; label: string }[] }),
  methods: {
    openAt(_el: Element, items: { key: string; label: string }[]): void {
      ;(this as unknown as { items: { key: string; label: string }[] }).items = items
    },
    close(): void {
      ;(this as unknown as { items: { key: string; label: string }[] }).items = []
    },
  },
  template: `<div class="menu-stub"><button v-for="i in items" :key="i.key" class="menu-item-stub" type="button" @click="$emit('select', i)">{{ i.label }}</button></div>`,
}

function history(partial: Partial<RequestHistory> & { id: string }): RequestHistory {
  return {
    project_id: 'proj-url-1',
    endpoint_id: null,
    method: 'POST',
    url: 'https://p.example.com/api/orders?x=1',
    status: 200,
    duration_ms: 10,
    request_summary_json: '{}',
    response_summary_json: '{}',
    created_at: '2026-01-01T00:00:00.000Z',
    ...partial,
  }
}

async function mountEditor(endpoint: Endpoint) {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-url-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.endpoints = [endpoint]
  const wrapper = mount(EndpointEditor, {
    shallow: true,
    global: { stubs: { Menu: MenuStub, Tooltip: false } },
  })
  await nextTick()
  store.openEndpoint(endpoint)
  await nextTick()
  return { wrapper, store }
}

beforeEach(() => {
  vi.clearAllMocks()
  stubScrollIntoView()
})

describe('EndpointEditor 路径变量缺失提示', () => {
  it('path 含未配置变量时出现 chips（{id} 与 {{var}} 两种写法），点击切到 Path 页签', async () => {
    const endpoint = makeDraft({ id: 'ep-pv', name: '路径', path: '/orders/{id}/items/{{itemId}}' })
    endpoint.request.path_variables = [
      { key: 'id', value: '5', enabled: true, description: '' },
    ]
    const { wrapper, store } = await mountEditor(endpoint)
    const chips = wrapper.findAll('.pathvar-chip')
    expect(chips).toHaveLength(1)
    expect(chips[0]!.text()).toBe('itemId')

    await chips[0]!.trigger('click')
    expect(store.draftOf('ep-pv')?.request.active_tab).toBe('path')
  })

  it('变量全部配置 / path 无变量 / gRPC 接口时不提示', async () => {
    const full = makeDraft({ id: 'ep-ok', name: '全配', path: '/orders/{id}' })
    full.request.path_variables = [{ key: 'id', value: '', enabled: true, description: '' }]
    const { wrapper: w1 } = await mountEditor(full)
    expect(w1.find('.pathvar-hint').exists()).toBe(false)

    const plain = makeDraft({ id: 'ep-plain', name: '无变量', path: '/health' })
    const { wrapper: w2 } = await mountEditor(plain)
    expect(w2.find('.pathvar-hint').exists()).toBe(false)

    const grpc = makeDraft({ id: 'ep-grpc', name: 'gRPC', method: 'GRPC', path: 'localhost:50051/h.G/S' })
    const { wrapper: w3 } = await mountEditor(grpc)
    expect(w3.find('.pathvar-hint').exists()).toBe(false)
  })
})

describe('EndpointEditor 地址栏历史下拉', () => {
  it('点选历史记录：Method + 地址回填，query 并入参数（origin 写会话前缀）', async () => {
    const endpoint = makeDraft({ id: 'ep-hist', name: '历史', path: '/' })
    const { wrapper, store } = await mountEditor(endpoint)
    store.histories = [
      history({ id: 'h-1', method: 'POST', url: 'https://p.example.com/api/orders?x=1' }),
      history({ id: 'h-2', method: 'GET', url: 'https://p.example.com/api/orders' }),
    ]

    await wrapper.find('button[aria-label="最近请求（回填地址）"]').trigger('click')
    const items = wrapper.findAll('.menu-item-stub')
    expect(items).toHaveLength(2)

    // 点选第一条：POST + 完整地址
    await items[0]!.trigger('click')
    const draft = store.draftOf('ep-hist')
    expect(draft?.method).toBe('POST')
    expect(draft?.path).toBe('/api/orders')
    expect(draft?.request.params.find((p) => p.key === 'x')?.value).toBe('1')
    expect(store.sessionBaseUrl).toBe('https://p.example.com')
  })

  it('历史去重：相同 method+url 只出现一次（取最新）', async () => {
    const endpoint = makeDraft({ id: 'ep-dup', name: '去重', path: '/' })
    const { wrapper, store } = await mountEditor(endpoint)
    store.histories = [
      history({ id: 'h-a', url: 'https://p.example.com/a' }),
      history({ id: 'h-b', url: 'https://p.example.com/a' }),
      history({ id: 'h-c', url: 'https://p.example.com/b' }),
    ]
    await wrapper.find('button[aria-label="最近请求（回填地址）"]').trigger('click')
    expect(wrapper.findAll('.menu-item-stub')).toHaveLength(2)
  })
})
