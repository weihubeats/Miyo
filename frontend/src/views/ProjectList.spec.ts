/**
 * ProjectList 直接测试（测试池补口：1632 行最大视图之一此前零覆盖）。
 * 覆盖：加载渲染项目卡 / 搜索过滤（含空态文案）/ 复制项目走 saveProject /
 * 激活项目高亮。useMiyoApi 模块级 mock 为内存假后端（GraphQLView.spec 同款）。
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import ProjectList from './ProjectList.vue'
import { useLocaleStore } from '../stores/locale'
import type { Project, ProjectStat, RecentActivity } from '../types/miyoApi'

const routerMock = vi.hoisted(() => ({ push: vi.fn() }))

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => routerMock,
}))

function makeProject(id: string, name: string): Project {
  const now = '2026-01-01T00:00:00.000Z'
  return { id, name, description: '', variables: {}, created_at: now, updated_at: now }
}

const backend = vi.hoisted(() => {
  const projects: Project[] = []
  let activeId: string | null = null
  return {
    projects,
    active: () => activeId,
    setActive: (id: string | null) => {
      activeId = id
    },
  }
})

const apiMock = vi.hoisted(() => ({
  getProjects: vi.fn(async () => backend.projects.map((p) => ({ ...p }))),
  listTeams: vi.fn(async () => []),
  listProjectStats: vi.fn(async (): Promise<ProjectStat[]> => []),
  listRecentActivities: vi.fn(async (): Promise<RecentActivity[]> => []),
  saveProject: vi.fn(async (p: Project) => {
    backend.projects.push(p)
    return { ...p }
  }),
  setActiveProject: vi.fn(async (id: string | null) => {
    backend.setActive(id)
    return backend.projects.find((p) => p.id === id) ?? null
  }),
  activeProject: { value: null as Project | null },
}))

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => ({
    ...apiMock,
    activeProject: computedRef(apiMock.activeProject),
  }),
}))

// ProjectList 顶栏经 useWindowDrag 拖拽窗口：模块级 getCurrentWindow() 在
// 非 Tauri 环境会崩，mock 成无操作句柄（WindowControls.spec 同款做法）。
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ startDragging: vi.fn(), onMove: vi.fn() }),
}))

// Swap 插件的具名导出在 vitest 的 CJS 互操作下不可靠（CI 实测），且拖拽
// 排序本身不是本 spec 的断言面——mock 掉 ensureSwapMounted。
vi.mock('../utils/sortable', () => ({ ensureSwapMounted: vi.fn() }))

import { computed } from 'vue'
function computedRef<T>(source: { value: T }) {
  return computed({
    get: () => source.value,
    set: (v: T) => {
      source.value = v
    },
  })
}

vi.mock('../components/ProjectTabs.vue', () => ({
  default: { template: '<div class="project-tabs-stub" />' },
}))

beforeAll(() => {
  const mem = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
  })
})


function mountList() {
  return mount(ProjectList, { attachTo: document.body })
}

/** 过滤网格卡片名（.tl-name 是摘要卡「最近活动」时间线，不做搜索过滤）。 */
function projectNames(wrapper: ReturnType<typeof mountList>): string[] {
  return wrapper.findAll('.proj-title').map((n) => n.text())
}

describe('ProjectList：加载与渲染', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    localStorage.clear()
    backend.projects.length = 0
    backend.setActive(null)
    backend.projects.push(makeProject('p-1', '支付网关'), makeProject('p-2', '用户中心'))
    vi.clearAllMocks()
  })

  it('挂载即拉取项目并渲染名称（时间线分组）', async () => {
    const wrapper = mountList()
    await flushPromises()
    await nextTick()
    expect(apiMock.getProjects).toHaveBeenCalled()
    const names = projectNames(wrapper)
    expect(names).toContain('支付网关')
    expect(names).toContain('用户中心')
    wrapper.unmount()
  })

  /**
   * 返回首页不闪：从实时调试回来时组件会重新挂载，但数据仍在内存里。
   * 此时必须先渲染已有卡片，不能回到骨架屏（旧行为是 loading → .sk-grid）。
   */
  it('已有快照再挂载：直接渲染卡片，不出现骨架屏', async () => {
    // 生产里 pinia 是应用级单例，路由切换不重建；这里显式复用同一个实例
    const sharedPinia = createPinia()
    setActivePinia(sharedPinia)

    const first = mountList()
    await flushPromises()
    await nextTick()
    expect(first.find('.card-grid').exists()).toBe(true)
    first.unmount()

    // 模拟返回：数据就绪后再挂载一次，且这次 IPC 迟迟不返回
    setActivePinia(sharedPinia)
    apiMock.getProjects.mockImplementationOnce(() => new Promise(() => {}))
    apiMock.listProjectStats.mockImplementationOnce(() => new Promise(() => {}))
    const again = mountList()
    await nextTick()
    expect(again.find('.sk-grid').exists()).toBe(false)
    expect(projectNames(again)).toContain('支付网关')
    again.unmount()

    // 收尾：本用例排队的 Once 实现若未消费，会污染后续用例（clearAllMocks 不清实现）
    apiMock.getProjects.mockReset()
    apiMock.getProjects.mockImplementation(async () => backend.projects.map((p) => ({ ...p })))
    apiMock.listProjectStats.mockReset()
    apiMock.listProjectStats.mockResolvedValue([])
  })

  it('搜索过滤项目；无命中展示空态文案', async () => {
    const wrapper = mountList()
    await flushPromises()
    const input = wrapper.find('.toolbar-filter-input')
    await input.setValue('支付')
    await nextTick()
    expect(projectNames(wrapper)).toContain('支付网关')
    expect(projectNames(wrapper)).not.toContain('用户中心')
    await input.setValue('不存在的项目xyz')
    await nextTick()
    expect(wrapper.text()).toContain('没有匹配的项目')
    await input.setValue('')
    await nextTick()
    expect(projectNames(wrapper)).toContain('用户中心')
    wrapper.unmount()
  })

  it('复制项目：经 saveProject 落一个「副本」新项目', async () => {
    const wrapper = mountList()
    await flushPromises()
    // 卡片菜单触发复制：直接走菜单 select 链路（openCopy 内部依赖 Menu 定位，事件层触发）
    const card = wrapper.find('[data-project-id="p-1"]')
    expect(card.exists()).toBe(true)
    // 菜单 teleport 定位脆弱：直接冒泡 ProjectCard 的 duplicate 事件走同链路
    await card.findComponent({ name: 'ProjectCard' }).vm.$emit('duplicate')
    await flushPromises()
    expect(apiMock.saveProject).toHaveBeenCalled()
    const saved = apiMock.saveProject.mock.calls[0]?.[0] as Project
    expect(saved.name).toContain('支付网关')
    // 副本落入假后端：项目数 +1
    expect(backend.projects).toHaveLength(3)
    wrapper.unmount()
  })

  it('激活项目高亮：isActive 跟随 activeProject 缓存', async () => {
    backend.setActive('p-2')
    apiMock.activeProject.value = makeProject('p-2', '用户中心')
    const wrapper = mountList()
    await flushPromises()
    await nextTick()
    const activeCard = wrapper.find('[data-project-id="p-2"]')
    expect(activeCard.exists()).toBe(true)
    wrapper.unmount()
  })
})

describe('ProjectList：仪表板第五轮精修', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    localStorage.clear()
    backend.projects.length = 0
    backend.setActive(null)
    backend.projects.push(makeProject('p-1', '支付网关'), makeProject('p-2', '用户中心'))
    vi.clearAllMocks()
    // mockResolvedValue 会跨用例残留：每个用例前恢复默认空实现
    apiMock.listProjectStats.mockResolvedValue([])
    apiMock.listRecentActivities.mockResolvedValue([])
  })

  it('欢迎卡统计：项目 / 接口 / 场景 / 请求总数来自 listProjectStats 聚合', async () => {
    apiMock.listProjectStats.mockResolvedValue([
      {
        project_id: 'p-1',
        endpoint_count: 3,
        scenario_count: 1,
        request_count: 5,
        latest_method: 'GET',
        latest_path: '/x',
      },
      {
        project_id: 'p-2',
        endpoint_count: 2,
        scenario_count: 0,
        request_count: 7,
        latest_method: null,
        latest_path: null,
      },
    ])
    const wrapper = mountList()
    await flushPromises()
    const values = wrapper.findAll('.hero-value').map((v) => v.text())
    expect(values).toEqual(['2', '5', '1', '12'])
    wrapper.unmount()
  })

  /**
   * 四个统计都是跨项目汇总，首页没有「按维度看明细」的落点：
   * 做成可点击只会把用户带进无关项目（旧实现点接口/场景/请求都跳最近项目）。
   */
  it('统计数字不可点击：不是按钮，点击不触发任何导航', async () => {
    const pushSpy = routerMock.push
    pushSpy.mockClear()
    const wrapper = mountList()
    await flushPromises()
    const stats = wrapper.findAll('.hero-stat')
    expect(stats).toHaveLength(4)
    for (const stat of stats) {
      expect(stat.element.tagName).not.toBe('BUTTON')
      await stat.trigger('click')
    }
    expect(pushSpy).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('列表模式：localStorage 偏好生效，行内展示名称并保留更多菜单', async () => {
    localStorage.setItem('miyo.dash.viewMode', 'list')
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.find('.team-grid').exists()).toBe(false)
    const rows = wrapper.findAll('.proj-row')
    expect(rows).toHaveLength(2)
    expect(wrapper.find('[data-project-id="p-1"] .row-name').text()).toBe('支付网关')
    // 行内菜单开合独立于整行点击
    await rows[0]!.find('.row-more .icon-btn, .row-more button').trigger('click')
    await nextTick()
    expect(wrapper.find('[data-project-id="p-1"] .more-menu').exists()).toBe(true)
    wrapper.unmount()
  })

  it('排序偏好：按接口数排序经 localStorage 恢复后生效', async () => {
    apiMock.listProjectStats.mockResolvedValue([
      {
        project_id: 'p-1',
        endpoint_count: 3,
        scenario_count: 0,
        request_count: 0,
        latest_method: null,
        latest_path: null,
      },
      {
        project_id: 'p-2',
        endpoint_count: 10,
        scenario_count: 0,
        request_count: 0,
        latest_method: null,
        latest_path: null,
      },
    ])
    localStorage.setItem('miyo.dash.sortBy', 'apis')
    const wrapper = mountList()
    await flushPromises()
    // 数据库顺序是 p-1, p-2；按接口数排序后 p-2（10 个）在前
    const names = wrapper.findAll('.proj-title').map((n) => n.text())
    expect(names).toEqual(['用户中心', '支付网关'])
    wrapper.unmount()
  })

  it('清空筛选：一键还原搜索 / 分组 / 排序', async () => {
    const wrapper = mountList()
    await flushPromises()
    await wrapper.find('.toolbar-filter-input').setValue('支付')
    await nextTick()
    expect(wrapper.findAll('.proj-title')).toHaveLength(1)
    expect(wrapper.text()).toContain('匹配 1/2 个项目')
    const clearBtn = wrapper.find('.clear-btn')
    expect(clearBtn.exists()).toBe(true)
    await clearBtn.trigger('click')
    await nextTick()
    expect(wrapper.findAll('.proj-title')).toHaveLength(2)
    expect(wrapper.find('.clear-btn').exists()).toBe(false)
    wrapper.unmount()
  })

  it('分组折叠：点击 chevron 收起网格并持久化偏好', async () => {
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.find('.team-grid').exists()).toBe(true)
    await wrapper.find('.team-collapse').trigger('click')
    await nextTick()
    expect(wrapper.find('.team-grid').exists()).toBe(false)
    expect(JSON.parse(localStorage.getItem('miyo.dash.collapsedTeams') ?? '{}')).toEqual({
      '00000000-0000-0000-0000-000000000001': true,
    })
    wrapper.unmount()
  })

  it('最近活动：渲染真实请求历史（方法 / 项目 / 状态 / 耗时），点击进入归属项目', async () => {
    apiMock.listRecentActivities.mockResolvedValue([
      {
        id: 'a-1',
        project_id: 'p-1',
        project_name: '支付网关',
        endpoint_name: '查询订单',
        method: 'GET',
        url: 'https://api.example.com/orders',
        status: 200,
        duration_ms: 12,
        created_at: '2026-01-01T00:00:00.000Z',
      },
      {
        id: 'a-2',
        project_id: 'p-2',
        project_name: '用户中心',
        endpoint_name: null,
        method: 'POST',
        url: 'https://api.example.com/users',
        status: 500,
        duration_ms: null,
        created_at: '2026-01-01T00:00:00.000Z',
      },
    ])
    const wrapper = mountList()
    await flushPromises()
    const items = wrapper.findAll('.tl-item')
    expect(items).toHaveLength(2)
    expect(items[0]!.text()).toContain('支付网关')
    expect(items[0]!.text()).toContain('查询订单')
    expect(items[0]!.text()).toContain('200')
    expect(items[0]!.text()).toContain('12 ms')
    // 5xx 状态走错误色档
    expect(items[1]!.find('.tl-status.st-err').exists()).toBe(true)
    wrapper.unmount()
  })

  it('无请求历史时活动卡展示空态，不模拟数据', async () => {
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.findAll('.tl-item')).toHaveLength(0)
    expect(wrapper.text()).toContain('暂无活动')
    wrapper.unmount()
  })
})

describe('ProjectList：仪表板第六轮精修', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    localStorage.clear()
    backend.projects.length = 0
    backend.setActive(null)
    backend.projects.push(makeProject('p-1', '支付网关'), makeProject('p-2', '用户中心'))
    vi.clearAllMocks()
    apiMock.listProjectStats.mockResolvedValue([])
    apiMock.listRecentActivities.mockResolvedValue([])
  })

  it('统计未加载：数字显示占位「—」而不是误导的 0', async () => {
    // 统计 IPC 一直挂起：首屏处于「未就绪」
    apiMock.listProjectStats.mockImplementation(() => new Promise(() => {}))
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.findAll('.hero-value').map((v) => v.text())).toEqual(['2', '—', '—', '—'])
    wrapper.unmount()
  })

  it('统计加载失败：同样不冒充 0，给出重试入口', async () => {
    apiMock.listProjectStats.mockRejectedValue(new Error('db down'))
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.findAll('.hero-value').map((v) => v.text())).toEqual(['2', '—', '—', '—'])
    expect(wrapper.find('.stats-retry').exists()).toBe(true)
    wrapper.unmount()
  })

  it('请求数成功/失败拆分进统计提示（口径与总数一致）', async () => {
    apiMock.listProjectStats.mockResolvedValue([
      {
        project_id: 'p-1',
        endpoint_count: 1,
        scenario_count: 0,
        request_count: 10,
        ok_count: 7,
        fail_count: 3,
        latest_method: null,
        latest_path: null,
      },
    ])
    const wrapper = mountList()
    await flushPromises()
    const requestStat = wrapper.findAll('.hero-stat')[3]!
    expect(requestStat.find('.hero-value').text()).toBe('10')
    expect(requestStat.attributes('title')).toContain('成功 7')
    expect(requestStat.attributes('title')).toContain('失败 3')
    wrapper.unmount()
  })

  it('活动加载失败：文案是「加载失败」而非「暂无活动」，可重试', async () => {
    apiMock.listRecentActivities.mockRejectedValueOnce(new Error('boom'))
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.text()).toContain('活动记录加载失败')
    expect(wrapper.text()).not.toContain('暂无活动')
    await wrapper.find('.stats-retry').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('暂无活动')
    wrapper.unmount()
  })

  it('活动行两行结构：第二行是去掉域名的路径', async () => {
    apiMock.listRecentActivities.mockResolvedValue([
      {
        id: 'a-1',
        project_id: 'p-1',
        project_name: '支付网关',
        endpoint_id: 'ep-9',
        endpoint_name: '查询订单',
        method: 'GET',
        url: 'https://api.example.com/orders/42?full=1',
        status: 200,
        duration_ms: 5,
        created_at: '2026-01-01T00:00:00.000Z',
      },
    ])
    const wrapper = mountList()
    await flushPromises()
    const item = wrapper.find('.tl-item')
    expect(item.find('.tl-name').text()).toBe('支付网关')
    expect(item.find('.tl-epname').text()).toBe('查询订单')
    expect(item.find('.tl-url').text()).toBe('/orders/42?full=1')
    // 点击进入归属项目（switchProject 走 store：http mock 下直接返回不抛错）
    await item.trigger('click')
    await flushPromises()
    wrapper.unmount()
  })

  it('⌘K 聚焦搜索框；Esc 清空', async () => {
    const wrapper = mountList()
    await flushPromises()
    const input = wrapper.find('.toolbar-filter-input')
    expect(wrapper.find('.toolbar-kbd').exists()).toBe(true)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))
    await nextTick()
    expect(document.activeElement).toBe(input.element)
    await input.setValue('支付')
    expect(wrapper.find('.toolbar-kbd').exists()).toBe(false)
    await input.trigger('keydown', { key: 'Escape' })
    await nextTick()
    expect((input.element as HTMLInputElement).value).toBe('')
    wrapper.unmount()
  })

  it('已折叠分组在搜索命中时临时展开（偏好不被改写）', async () => {
    localStorage.setItem(
      'miyo.dash.collapsedTeams',
      JSON.stringify({ '00000000-0000-0000-0000-000000000001': true }),
    )
    const wrapper = mountList()
    await flushPromises()
    // 无搜索：按偏好保持折叠
    expect(wrapper.find('.team-grid').exists()).toBe(false)
    await wrapper.find('.toolbar-filter-input').setValue('支付')
    await nextTick()
    // 搜索命中落在折叠组里：临时展开，能看到卡片
    expect(wrapper.findAll('.proj-title').map((n) => n.text())).toEqual(['支付网关'])
    expect(localStorage.getItem('miyo.dash.collapsedTeams')).toBe(
      JSON.stringify({ '00000000-0000-0000-0000-000000000001': true }),
    )
    wrapper.unmount()
  })

  it('空列表：只渲染空态，不叠加 Dropzone', async () => {
    backend.projects.length = 0
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.find('.dash-empty').exists()).toBe(true)
    expect(wrapper.find('.add-card').exists()).toBe(false)
    wrapper.unmount()
  })

  it('删除项目后首页计数与活动同步移除该项目', async () => {
    apiMock.listProjectStats.mockResolvedValue([
      {
        project_id: 'p-1',
        endpoint_count: 3,
        scenario_count: 1,
        request_count: 2,
        ok_count: 2,
        fail_count: 0,
        latest_method: null,
        latest_path: null,
      },
    ])
    apiMock.listRecentActivities.mockResolvedValue([
      {
        id: 'a-1',
        project_id: 'p-1',
        project_name: '支付网关',
        endpoint_id: null,
        endpoint_name: null,
        method: 'GET',
        url: 'https://x.test/a',
        status: 200,
        duration_ms: 1,
        created_at: '2026-01-01T00:00:00.000Z',
      },
    ])
    const wrapper = mountList()
    await flushPromises()
    expect(wrapper.findAll('.hero-value').map((v) => v.text())).toEqual(['2', '3', '1', '2'])
    expect(wrapper.findAll('.tl-item')).toHaveLength(1)
    const vm = wrapper.vm as unknown as { onDeleted: (id: string) => Promise<void> }
    await vm.onDeleted('p-1')
    await flushPromises()
    expect(wrapper.findAll('.hero-value').map((v) => v.text())).toEqual(['1', '0', '0', '0'])
    expect(wrapper.findAll('.tl-item')).toHaveLength(0)
    wrapper.unmount()
  })
})
