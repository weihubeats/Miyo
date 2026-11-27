/**
 * IntegrationView 导航状态：
 * - 深链 ?scenario= 直接打开场景详情（刷新 / 直接 URL）；
 * - 详情返回列表后 query 清理，列表上下文（store.libraryView）保留；
 * - 深链 ?suite= / ?batch= 恢复对应层级；
 * - 批次来源的单场景详情返回时回到批次详情。
 *
 * 子页面以轻量 stub 替换，聚焦路由 ↔ store 的状态同步。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'

const listScenarioSummaries = vi.fn(async () => [])
const listScenarioFolders = vi.fn(async () => [])
const getProjects = vi.fn(async () => [
  { id: 'p1', name: 'P', description: '', variables: {}, created_at: '', updated_at: '' },
])
const getTestBatch = vi.fn(async () => ({
  id: 'b1',
  project_id: 'p1',
  status: 'completed',
  mode: 'serial',
  concurrency: 1,
  failure_policy: 'continue',
  retry_count: 0,
  skip_disabled: true,
  total_count: 0,
  passed_count: 0,
  failed_count: 0,
  skipped_count: 0,
  cancelled_count: 0,
  running_count: 0,
  pending_count: 0,
  created_at: new Date().toISOString(),
  items: [],
}))
const listTestBatches = vi.fn(async () => [])

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    getProjects,
    listScenarioSummaries,
    listScenarioFolders,
    getTestBatch,
    listTestBatches,
    listTestSuites: vi.fn(async () => []),
  }),
}))

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

vi.mock('@/domains/integration/components/integration/ScenarioLibrary.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'ScenarioLibrary', setup: () => () => h('div', { class: 'lib-stub' }) }) }
})
vi.mock('@/domains/integration/components/integration/TestSuitePanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'TestSuitePanel', setup: () => () => h('div', { class: 'suite-stub' }) }) }
})
vi.mock('@/domains/integration/components/integration/WorkflowEditor.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'WorkflowEditor',
      props: ['scenarioId', 'backLabel', 'parentCrumbs'],
      emits: ['back'],
      setup: () => () => h('div', { class: 'editor-stub' }),
    }),
  }
})
vi.mock('@/domains/integration/components/integration/BatchRunDialog.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'BatchRunDialog', setup: () => () => h('div') }) }
})
vi.mock('@/domains/integration/components/integration/BatchReportPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ name: 'BatchReportPanel', setup: () => () => h('div') }) }
})

import IntegrationView from '@/domains/integration/views/IntegrationView.vue'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useTestSuitesStore } from '@/domains/integration/stores/testSuites'
import { useLocaleStore } from '@/stores/locale'

async function mountAt(path: string) {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/integration', name: 'integration', component: IntegrationView }],
  })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(IntegrationView, { global: { plugins: [router] } })
  await flushPromises()
  await flushPromises()
  return { wrapper, router }
}

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
})

describe('IntegrationView 导航状态', () => {
  it('深链 ?scenario= 直接打开场景详情，query 保留可复制的层级', async () => {
    const { wrapper, router } = await mountAt('/integration?project=p1&scenario=sc-1')
    const store = useIntegrationStore()
    expect(store.activeScenarioId).toBe('sc-1')
    expect(wrapper.find('.editor-stub').exists()).toBe(true)

    const editor = wrapper.findComponent({ name: 'WorkflowEditor' })
    expect(editor.props('scenarioId')).toBe('sc-1')
    expect(router.currentRoute.value.query.scenario).toBe('sc-1')
    expect(router.currentRoute.value.query.project).toBe('p1')
    wrapper.unmount()
  })

  it('详情返回列表：清理 query，保留列表视图状态', async () => {
    const { wrapper, router } = await mountAt('/integration?project=p1&scenario=sc-1')
    const store = useIntegrationStore()
    store.libraryView.search = '订单'
    store.libraryView.folderFilter = 'f-1'

    wrapper.findComponent({ name: 'WorkflowEditor' }).vm.$emit('back')
    await flushPromises()

    expect(store.activeScenarioId).toBeNull()
    expect(router.currentRoute.value.query.scenario).toBeUndefined()
    // 列表上下文不被返回清空（搜索 / 目录选中保留）
    expect(store.libraryView.search).toBe('订单')
    expect(store.libraryView.folderFilter).toBe('f-1')
    wrapper.unmount()
  })

  it('应用重启 / 从其他模块回来：无 query 时恢复上次详情', async () => {
    localStorage.setItem(
      'miyo.integration.last-detail',
      JSON.stringify({ project: 'p1', scenario: 'sc-9' }),
    )
    const { wrapper } = await mountAt('/integration')
    const store = useIntegrationStore()
    expect(store.activeScenarioId).toBe('sc-9')
    expect(wrapper.find('.editor-stub').exists()).toBe(true)
    wrapper.unmount()
  })

  it('深链 ?suite= 选中套件；?batch= 打开批次详情', async () => {
    const { wrapper } = await mountAt('/integration?project=p1&suite=su-1')
    const suites = useTestSuitesStore()
    expect(suites.activeSuiteId).toBe('su-1')
    wrapper.unmount()

    const second = await mountAt('/integration?project=p1&batch=b1')
    const batchSuites = useTestSuitesStore()
    expect(second.wrapper.find('.suite-stub').exists() || second.wrapper.find('.lib-stub').exists()).toBe(true)
    expect(batchSuites.activeBatchId).toBe('b1')
    expect(batchSuites.panelOpen).toBe(true)
    second.wrapper.unmount()
  })

  it('批次来源的单场景详情：返回目标为「返回批次详情」，返回后回到批次', async () => {
    const { wrapper } = await mountAt('/integration?project=p1&scenario=sc-1&batch=b1')
    const store = useIntegrationStore()
    const suites = useTestSuitesStore()

    const editor = wrapper.findComponent({ name: 'WorkflowEditor' })
    expect(editor.props('backLabel')).toBe('返回批次详情')
    expect((editor.props('parentCrumbs') as { label: string }[]).map((c) => c.label)).toEqual([
      '执行历史',
      expect.stringContaining('批次'),
    ])

    editor.vm.$emit('back')
    await flushPromises()
    expect(store.activeScenarioId).toBeNull()
    expect(suites.activeBatchId).toBe('b1')
    wrapper.unmount()
  })
})