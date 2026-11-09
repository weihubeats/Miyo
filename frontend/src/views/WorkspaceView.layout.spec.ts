/**
 * WorkspaceView 布局回归：「新建」选择页与 TabBar / EndpointEditor 同处一列。
 *
 * 用户实测 bug（v0.0.55，多项目标签 + 多接口标签场景）：
 * 打开「创建支付订单」页签后内容区整块空白——面包屑、二级导航、请求地址栏
 * 全部消失，只剩一条分割线和黑色底；切到相邻页签又一切正常。
 *
 * 根因（本 spec 锁定的三个结构前提）：
 * 1. 宿主把 `activeIsCreate` 与「有页签」写成 `v-if` / `v-else-if` 兄弟：选择页
 *    关闭后 `activeTabId` 仍指向已关闭的「新建」id，第一个分支恒为真，TabBar
 *    与 EndpointEditor 两个分支永远走不到；
 * 2. CreateChoicePage 在 openTabs 为空时内部返回注释节点（自身不占位），但宿主
 *    给它的强调度 `flex: 1` 还在——`miyo-main` 是 column flex，空闲高度照旧
 *    分给这具空壳，编辑器只剩 ~0 高；
 * 3. `.editor` 的子项走 flex 默认 `flex-shrink: 1`，高度预算一旦算错就被压成 0，
 *    只剩能自撑高度的分割线（1px）与空响应低条（120px）。
 *
 * jsdom 不排版：这里断言「谁被渲染、谁被允许吃掉空闲高度」的结构前提，
 * 像素高度交给真机验证。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

const routerMock = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn(), back: vi.fn() }))

vi.mock('vue-router', () => ({
  useRoute: () => ({ path: '/workspace', query: {} }),
  useRouter: () => routerMock,
}))

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

const endpoint = makeDraft({ id: 'ep-order', name: '创建支付订单', method: 'POST', path: '/orders' })

const fakeApi = {
  pending: { value: false },
  getProjects: async () => [],
  getActiveProject: async () => null,
  listFolders: async () => [],
  listEndpoints: async () => [endpoint],
  listEnvironments: async () => [],
  listExamples: async () => [],
  listRequestExamples: async () => [],
  listTestCases: async () => [],
  listGlobalVariables: async () => [],
  listGlobalParams: async () => [],
  mockStatus: async () => null,
}

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () =>
    new Proxy(fakeApi, {
      get: (target, prop) => (prop in target ? Reflect.get(target, prop) : async () => null),
    }),
}))

vi.mock('../composables/useWindowDrag', () => ({ useWindowDrag: vi.fn() }))

/** vi.mock 工厂会被提升到 import 之前执行：桩工厂必须留在 hoisted 区。 */
const stub = vi.hoisted(() => (name: string, cls: string) => ({
  default: { name, template: `<div class="${cls}" />` },
}))

vi.mock('../components/EndpointTree.vue', () => stub('EndpointTreeStub', 'tree-stub'))
vi.mock('../components/HistoryPanel.vue', () => stub('HistoryPanelStub', 'history-stub'))
vi.mock('../components/CookiePanel.vue', () => stub('CookiePanelStub', 'cookie-stub'))
vi.mock('../components/ProjectTabs.vue', () => stub('ProjectTabsStub', 'ptabs-stub'))
vi.mock('../components/WindowControls.vue', () => stub('WindowControlsStub', 'win-stub'))
vi.mock('../components/EnvironmentBar.vue', () => stub('EnvironmentBarStub', 'env-stub'))
vi.mock('../components/HeaderUpdateTag.vue', () => stub('HeaderUpdateTagStub', 'upd-stub'))
vi.mock('../components/QuickSearch.vue', () => stub('QuickSearchStub', 'qs-stub'))
vi.mock('../components/TabBar.vue', () => stub('TabBarStub', 'tabbar-stub'))
vi.mock('../components/EndpointEditor.vue', () => stub('EndpointEditorStub', 'editor-stub'))

/** 选择页桩：真实组件在 openTabs 为空时返回注释节点（这正是排查难点），
 *  这里给带标记的根元素——断言只看「宿主是否还给它让位」。 */
vi.mock('../components/CreateChoicePage.vue', () => stub('CreateChoicePageStub', 'cc-page'))

import WorkspaceView from './WorkspaceView.vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import { stubScrollIntoView } from '../testUtils/componentTest'

/** jsdom 未实现 matchMedia：顶栏「宽窗直达按钮」开关依赖它。 */
function stubMatchMedia(): void {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: true,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }))
}

function setupStore() {
  setActivePinia(createPinia())
  stubScrollIntoView()
  stubMatchMedia()
  localStorage.clear()
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: '小奏技术 · 支付中心',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.endpoints = [endpoint]
  return store
}

async function mountView() {
  const wrapper = mount(WorkspaceView, {
    attachTo: document.body,
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await flushPromises()
  await nextTick()
  return wrapper
}

const choicePage = { name: 'CreateChoicePageStub' }
const tabbar = { name: 'TabBarStub' }
const editor = { name: 'EndpointEditorStub' }

describe('WorkspaceView 布局：选择页不再是编辑器的高度竞争者', () => {
  beforeEach(() => {
    setupStore()
    vi.clearAllMocks()
  })

  it('存在「新建」页签但激活的是接口页签：渲染 TabBar + 编辑器，选择页不入布局', async () => {
    const store = useWorkspaceStore()
    const wrapper = await mountView()
    store.openCreateTab()
    store.openEndpoint(endpoint)
    await nextTick()

    expect(store.activeTabId).toBe('ep-order')
    expect(wrapper.findComponent(tabbar).exists()).toBe(true)
    expect(wrapper.findComponent(editor).exists()).toBe(true)
    // 选择页若仍在 DOM 里，column flex 会把空闲高度分给它 → 编辑器归零
    expect(wrapper.findComponent(choicePage).exists()).toBe(false)
    wrapper.unmount()
  })

  it('激活「新建」页签：展示选择页，编辑器让位', async () => {
    const store = useWorkspaceStore()
    const wrapper = await mountView()
    store.openEndpoint(endpoint)
    store.openCreateTab()
    await nextTick()

    expect(wrapper.findComponent(choicePage).exists()).toBe(true)
    expect(wrapper.findComponent(editor).exists()).toBe(false)
    wrapper.unmount()
  })

  it('「新建」页签已关闭：相邻接口页签接管，空壳选择页不得留在布局里', async () => {
    const store = useWorkspaceStore()
    const wrapper = await mountView()
    store.openEndpoint(endpoint)
    store.openCreateTab()
    await nextTick()
    expect(wrapper.findComponent(choicePage).exists()).toBe(true)

    // 对齐 onCreatePageClose 的真实路径：关掉「新建」页签后导入框弹出。
    // closeTab 会把激活态让给相邻页签（这里是接口页签），旧实现的 v-if 分支
    // 顺序却让「已关闭的选择页」继续占着整块内容区。
    store.closeTab(store.activeTabId!)
    await nextTick()

    expect(store.activeTabId).toBe('ep-order')
    expect(wrapper.findComponent(choicePage).exists()).toBe(false)
    expect(wrapper.findComponent(tabbar).exists()).toBe(true)
    expect(wrapper.findComponent(editor).exists()).toBe(true)
    wrapper.unmount()
  })
})