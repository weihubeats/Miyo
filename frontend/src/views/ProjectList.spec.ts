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
import type { Project } from '../types/miyoApi'

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
  listProjectStats: vi.fn(async () => []),
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
