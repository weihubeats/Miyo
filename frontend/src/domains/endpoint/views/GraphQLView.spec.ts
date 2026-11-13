/**
 * GraphQLView 单测：无上下文进入时的地址解析（props → route.query → 端点 →
 * localStorage → 活跃端点 → `${urlDomain}/graphql`）与发送成功后的地址记忆。
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import GraphQLView from '@/domains/endpoint/views/GraphQLView.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'

const routeMock = vi.hoisted(() => ({ query: {} as Record<string, unknown> }))
const apiMock = vi.hoisted(() => ({
  executeRequest: vi.fn(async () => ({
    status: 200,
    body: '{"data":{"hi":1}}',
    duration_ms: 3,
  })),
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: routeMock.query }),
  useRouter: () => ({ push: vi.fn() }),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))

/** 测试环境的 localStorage 为残缺对象（真机 WebView 才有完整实现），stub 一个内存版。 */
beforeAll(() => {
  const mem = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
  })
})

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  localStorage.clear()
  routeMock.query = {}
  apiMock.executeRequest.mockClear()
})

function mountView() {
  return mount(GraphQLView, { attachTo: document.body })
}

function urlInput(wrapper: ReturnType<typeof mountView>): HTMLInputElement {
  return wrapper.find('input.gql-url').element as HTMLInputElement
}

describe('GraphQLView 初始地址', () => {
  it('route.query.url 优先（覆盖 localStorage 记忆）', () => {
    localStorage.setItem('miyo_graphql_last_url', 'https://cached.example.com/graphql')
    routeMock.query = { url: 'https://from-query.example.com/graphql' }

    const wrapper = mountView()
    expect(urlInput(wrapper).value).toBe('https://from-query.example.com/graphql')
    wrapper.unmount()
  })

  it('route.query.endpointId 命中端点：相对路径拼会话 Base URL', () => {
    routeMock.query = { endpointId: 'ep-9' }
    const store = useWorkspaceStore()
    store.endpoints.push(makeDraft({ id: 'ep-9', path: '/api/graphql' }))

    const wrapper = mountView()
    expect(urlInput(wrapper).value).toBe('http://localhost/api/graphql')
    wrapper.unmount()
  })

  it('无上下文：回退上次发送地址；再无则 `${urlDomain}/graphql` 兜底', async () => {
    let wrapper = mountView()
    expect(urlInput(wrapper).value).toBe('http://localhost/graphql')
    wrapper.unmount()

    localStorage.setItem('miyo_graphql_last_url', 'https://cached.example.com/graphql')
    wrapper = mountView()
    expect(urlInput(wrapper).value).toBe('https://cached.example.com/graphql')
    wrapper.unmount()
  })
})

describe('GraphQLView 地址记忆', () => {
  it('发送成功后把地址写入 localStorage（下次无上下文直接可用）', async () => {
    const wrapper = mountView()
    const input = wrapper.find('input.gql-url')
    ;(input.element as HTMLInputElement).value = 'https://api.example.com/gql'
    await input.trigger('input')

    await wrapper.findAll('textarea.hl-ta')[0]!.setValue('query { hi }')
    await wrapper.find('.row.miyo-mb-2 button.miyo-btn-primary').trigger('click')
    await flushPromises()

    expect(apiMock.executeRequest).toHaveBeenCalledTimes(1)
    expect(localStorage.getItem('miyo_graphql_last_url')).toBe('https://api.example.com/gql')
    wrapper.unmount()
  })
})
