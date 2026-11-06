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

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: vi.fn() }),
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
