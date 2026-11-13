/**
 * 「测试中」标记回归：AI 测试入口（导航 + 页面顶栏）必须常驻可见，
 * 提示页面仍在测试、结果可能不可靠。标记不可关闭（无 close 按钮）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const routerMock = vi.hoisted(() => ({ push: vi.fn(), back: vi.fn(), path: '/ai-test' }))

vi.mock('vue-router', () => ({ useRouter: () => routerMock, useRoute: () => routerMock }))
// AiTestView 的 onMounted 会打 IPC：整体打桩，只验证徽标渲染。
vi.mock('../stores/aiTest', () => ({
  useAiTestStore: () => ({
    projectId: null,
    projects: [],
    runScenario: null,
    runToken: 0,
    bindEvents: vi.fn(),
    initContext: vi.fn(),
    loadSessions: vi.fn(),
    loadScenarios: vi.fn(),
  }),
}))
vi.mock('../stores/workspace', () => ({ useWorkspaceStore: () => ({ switchProject: vi.fn() }) }))

import AiTestView from './AiTestView.vue'
import DashboardNav from '../components/projectlist/DashboardNav.vue'
import { useLocaleStore } from '../stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  routerMock.push.mockClear()
  routerMock.back.mockClear()
})

describe('AI 测试中心「测试中」标记', () => {
  it('页面顶栏常驻徽标 + 长文案提示，且无关闭按钮', () => {
    const wrapper = mount(AiTestView, { attachTo: document.body })
    const badge = wrapper.find('.ai-beta-badge')
    expect(badge.exists()).toBe(true)
    expect(badge.text()).toBe('测试中')
    // 常驻：不是可关闭的提示条，徽标内不含关闭控件
    expect(badge.find('button').exists()).toBe(false)
    // 悬停文案由 Tooltip 承载（content 传入即可，jsdom 不做 hover 定位）
    const tip = wrapper.findComponent({ name: 'Tooltip' })
    expect(tip.exists()).toBe(true)
    expect(String(tip.props('content'))).toContain('测试中')
    wrapper.unmount()
  })

  it('仪表板导航的 AI 测试入口带同名徽标与短提示', () => {
    const wrapper = mount(DashboardNav, { attachTo: document.body })
    const badges = wrapper.findAll('.nav-beta')
    expect(badges).toHaveLength(1)
    expect(badges[0].text()).toBe('测试中')
    expect(badges[0].attributes('title')).toContain('测试中')
    wrapper.unmount()
  })
})