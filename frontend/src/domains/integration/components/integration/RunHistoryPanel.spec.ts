/**
 * RunHistoryPanel：
 * - 打开时拉取历史列表（含 environment_id 新字段）；
 * - 状态 × 环境 × 时间三条件过滤（客户端）；
 * - 行点击 → open-run 事件（面板关闭）；
 * - 导出按钮 → 按筛选结果发 runIds；
 * - 删除成功后重载。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'

const listScenarioRuns = vi.fn()
const listRecentScenarioRuns = vi.fn()
const projectScenarioRunStats = vi.fn()
const deleteScenarioRun = vi.fn()

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    listScenarioRuns,
    listRecentScenarioRuns,
    projectScenarioRunStats,
    deleteScenarioRun,
  }),
}))

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ info: vi.fn(), warning: vi.fn(), error: vi.fn(), success: vi.fn() }),
}))

import RunHistoryPanel from '@/domains/integration/components/integration/RunHistoryPanel.vue'
import type { ScenarioRunSummary } from '@/types/miyoApi.d'

function run(
  id: string,
  opts: { env?: string | null; fail?: number; cancelled?: boolean; daysAgo?: number; scenarioId?: string } = {},
): ScenarioRunSummary {
  return {
    id,
    scenario_id: opts.scenarioId ?? 'sc-1',
    environment_id: opts.env ?? null,
    finished_at: new Date(Date.now() - (opts.daysAgo ?? 0) * 86_400_000).toISOString(),
    duration_ms: 100,
    total_steps: 3,
    ok_count: 3 - (opts.fail ?? 0),
    fail_count: opts.fail ?? 0,
    skipped_count: 0,
    cancelled: opts.cancelled ?? false,
    stopped_on_failure: false,
  }
}

interface SetupState {
  statusFilter: string
  envFilter: string
  timeFilter: string
  filtered: ScenarioRunSummary[]
  runs: ScenarioRunSummary[]
}

function mountPanel(
  open = true,
  popconfirmEmit = false,
  propsOverride: Record<string, unknown> = {},
) {
  const wrapper = mount(RunHistoryPanel, {
    props: { open, scenarioId: 'sc-1', scenarioName: '下单链路', ...propsOverride },
    global: {
      stubs: {
        Modal: { template: '<div><slot /></div>' },
        CustomSelect: true,
        Icon: true,
        // popconfirmEmit=true：点击默认槽即触发 confirm（模拟用户点「确认删除」）
        Popconfirm: popconfirmEmit
          ? { template: '<span data-testid="popconfirm" @click="$emit(\'confirm\')"><slot /></span>' }
          : { template: '<span><slot /></span>' },
      },
    },
  })
  const setup = (wrapper.vm.$ as unknown as { setupState: SetupState }).setupState
  return { wrapper, setup }
}

describe('RunHistoryPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    listScenarioRuns.mockReset()
    listRecentScenarioRuns.mockReset()
    projectScenarioRunStats.mockReset()
    deleteScenarioRun.mockReset()
  })

  it('打开时拉取历史（limit=100；含 environment_id）', async () => {
    listScenarioRuns.mockResolvedValue([run('r-1', { env: 'env-1' })])
    const { wrapper } = mountPanel()
    await flushPromises()
    expect(listScenarioRuns).toHaveBeenCalledWith('sc-1', 100)
    expect(wrapper.find('.hist-row').exists()).toBe(true)
    expect(wrapper.find('.hist-env').text()).toBe('env-1')
    wrapper.unmount()
  })

  it('状态过滤：失败 → 只留失败行', async () => {
    listScenarioRuns.mockResolvedValue([run('ok-1'), run('bad-1', { fail: 1 }), run('cx-1', { cancelled: true })])
    const { wrapper, setup } = mountPanel()
    await flushPromises()
    setup.statusFilter = 'fail'
    await flushPromises()
    expect(setup.filtered.map((r) => r.id)).toEqual(['bad-1'])
    wrapper.unmount()
  })

  it('环境过滤：none → 只留未选环境的运行', async () => {
    listScenarioRuns.mockResolvedValue([run('a', { env: 'env-1' }), run('b', { env: null })])
    const { wrapper, setup } = mountPanel()
    await flushPromises()
    setup.envFilter = 'none'
    await flushPromises()
    expect(setup.filtered.map((r) => r.id)).toEqual(['b'])
    wrapper.unmount()
  })

  it('时间过滤：7d → 排除 30 天前的旧记录', async () => {
    listScenarioRuns.mockResolvedValue([run('new', { daysAgo: 1 }), run('old', { daysAgo: 30 })])
    const { wrapper, setup } = mountPanel()
    await flushPromises()
    setup.timeFilter = '7d'
    await flushPromises()
    expect(setup.filtered.map((r) => r.id)).toEqual(['new'])
    wrapper.unmount()
  })

  it('行点击 → open-run 事件（带场景归属）+ 面板关闭', async () => {
    listScenarioRuns.mockResolvedValue([run('r-1')])
    const { wrapper } = mountPanel()
    await flushPromises()
    await wrapper.find('.hist-row').trigger('click')
    expect(wrapper.emitted('open-run')?.[0]).toEqual([{ scenarioId: 'sc-1', runId: 'r-1' }])
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
    wrapper.unmount()
  })

  it('项目级模式：拉全部运行 + 聚合统计；行点击带各自场景 id', async () => {
    listScenarioRuns.mockClear()
    listRecentScenarioRuns.mockResolvedValue([
      run('a', { scenarioId: 'sc-1' }),
      run('b', { scenarioId: 'sc-2' }),
    ])
    projectScenarioRunStats.mockResolvedValue({ totalRuns: 2, okRuns: 2, failRuns: 0, cancelledRuns: 0 })
    const { wrapper } = mountPanel(true, false, {
      scenarioId: null,
      projectId: 'p-1',
      scenarioName: undefined,
    })
    await flushPromises()
    expect(listRecentScenarioRuns).toHaveBeenCalledWith('p-1', 100)
    expect(listScenarioRuns).not.toHaveBeenCalled()
    expect(wrapper.find('.hist-scenario').exists()).toBe(true)
    await wrapper.find('.hist-row').trigger('click')
    expect(wrapper.emitted('open-run')?.[0]).toEqual([{ scenarioId: 'sc-1', runId: 'a' }])
    wrapper.unmount()
  })

  it('导出 → 按筛选结果携带 runIds', async () => {
    listScenarioRuns.mockResolvedValue([run('a', { fail: 1 }), run('b'), run('c', { fail: 2 })])
    const { wrapper, setup } = mountPanel()
    await flushPromises()
    setup.statusFilter = 'fail'
    await flushPromises()
    // .hist-filters 里的按钮顺序：对比 → HTML 报告 → JUnit XML
    const buttons = wrapper.findAll('.hist-filters button')
    await buttons[1]!.trigger('click')
    const evt = wrapper.emitted('export')?.[0]?.[0] as { format: string; runIds: string[] }
    expect(evt.format).toBe('html')
    expect(evt.runIds).toEqual(['a', 'c'])
    wrapper.unmount()
  })

  it('删除成功 → 调 API 并重载列表', async () => {
    listScenarioRuns.mockResolvedValue([run('r-1')])
    deleteScenarioRun.mockResolvedValue(true)
    const { wrapper } = mountPanel(true, true)
    await flushPromises()
    // 点击 Popconfirm 确认（动作区 @click.stop：不触发行打开详情）
    await wrapper.find('[data-testid="popconfirm"]').trigger('click')
    expect(wrapper.emitted('open-run')).toBeUndefined()
    expect(deleteScenarioRun).toHaveBeenCalledWith('r-1')
    expect(listScenarioRuns).toHaveBeenCalledTimes(2) // 打开时 + 删除后重载
    wrapper.unmount()
  })
})
