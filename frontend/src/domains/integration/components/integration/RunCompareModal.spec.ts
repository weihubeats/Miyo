/**
 * RunCompareModal 加载竞态（P0 修复回归）：
 * - 快速切换下拉：旧代次的迟到响应不得覆盖新选择；
 * - 弹窗关闭后到达的响应不写状态；
 * - loading 只反映最新一次加载。
 *
 * 组件为 <script setup>：setupState 经 proxyRefs 解包，直接读属性；
 * 下拉交互不在本测试范围，直接改 runAId 触发 watch。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const getScenarioRun = vi.fn()
const listScenarioRuns = vi.fn(async () => [
  { id: 'run-1', finished_at: '2026-11-12T00:00:00Z', duration_ms: 1, total_steps: 1, ok_count: 1, fail_count: 0, skipped_count: 0, cancelled: false, stopped_on_failure: false },
  { id: 'run-2', finished_at: '2026-11-12T00:01:00Z', duration_ms: 1, total_steps: 1, ok_count: 1, fail_count: 0, skipped_count: 0, cancelled: false, stopped_on_failure: false },
])

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    listScenarioRuns,
    getScenarioRun,
  }),
}))

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ info: vi.fn(), warning: vi.fn(), error: vi.fn(), success: vi.fn() }),
}))

import RunCompareModal from '@/domains/integration/components/integration/RunCompareModal.vue'

function makeDetail(id: string, marker: string) {
  return {
    id,
    finished_at: '2026-11-12T00:00:00.000Z',
    duration_ms: 1,
    total_steps: 1,
    ok_count: 1,
    fail_count: 0,
    skipped_count: 0,
    cancelled: false,
    stopped_on_failure: false,
    results: [{ endpoint_id: 'e', endpoint_name: marker, method: 'GET', path: '/a', ok: true, status: 200, duration_ms: 1, outcomes: [], extracted: {}, step_index: 0 }],
    variables: {},
    meta: null,
  }
}

interface SetupState {
  runAId: string | null
  runBId: string | null
  detailA: { id?: string; results: { endpoint_name: string }[] } | null
  loading: boolean
}

function mountModal(open: boolean) {
  const wrapper = mount(RunCompareModal, {
    props: { scenarioId: 'sc-1', open },
    global: {
      stubs: { Modal: { template: '<div><slot /></div>' }, CustomSelect: true, Icon: true },
    },
  })
  const instance = wrapper.vm.$ as unknown as { setupState: SetupState }
  return { wrapper, setup: instance.setupState }
}

describe('RunCompareModal 加载竞态', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    getScenarioRun.mockReset()
    listScenarioRuns.mockClear()
  })

  it('快速切换 A 运行：迟到的一代响应不得覆盖新选择', async () => {
    const resolvers: ((v: ReturnType<typeof makeDetail>) => void)[] = []
    getScenarioRun.mockImplementation(
      () => new Promise((resolve) => resolvers.push(resolve)),
    )
    const { wrapper, setup } = mountModal(false)
    // open 置 true → loadRuns 默认选中 A=run-2 / B=run-1 → 详情加载（gen1）。
    await wrapper.setProps({ open: true })
    await flushPromises()
    expect(getScenarioRun).toHaveBeenNthCalledWith(1, 'run-2')
    // gen1 的 A 还挂着；此时切换 A 到 run-1（gen2 启动，序号 +1）。
    setup.runAId = 'run-1'
    await flushPromises()
    expect(getScenarioRun).toHaveBeenNthCalledWith(2, 'run-1')
    // gen2 的 A 回来 → gen2 继续发 B 请求。
    resolvers[1](makeDetail('run-1', 'GEN2A'))
    await flushPromises()
    expect(getScenarioRun).toHaveBeenCalledTimes(3)
    // gen1 的 A 此刻才回（迟到）→ 必须被序号仲裁丢弃。
    resolvers[0](makeDetail('run-2', 'GEN1A_LATE'))
    await flushPromises()
    expect(setup.detailA?.results[0].endpoint_name).not.toBe('GEN1A_LATE')
    // gen2 的 B 回来 → detailA/detailB 成对落位（同代次）。
    resolvers[2](makeDetail('run-1', 'GEN2B'))
    await flushPromises()
    expect(setup.detailA?.results[0].endpoint_name).toBe('GEN2A')
  })

  it('关闭弹窗后在途响应不写状态', async () => {
    const resolvers: ((v: ReturnType<typeof makeDetail>) => void)[] = []
    getScenarioRun.mockImplementation(
      () => new Promise((resolve) => resolvers.push(resolve)),
    )
    const { wrapper, setup } = mountModal(false)
    await wrapper.setProps({ open: true })
    await flushPromises()
    expect(setup.detailA).toBeNull()
    await wrapper.setProps({ open: false })
    resolvers[0](makeDetail('run-1', 'LATE'))
    await flushPromises()
    expect(setup.detailA).toBeNull()
  })

  it('loading 只反映最新一次加载（旧代次完成不提前置 false）', async () => {
    const resolvers: ((v: ReturnType<typeof makeDetail>) => void)[] = []
    getScenarioRun.mockImplementation(
      () => new Promise((resolve) => resolvers.push(resolve)),
    )
    const { wrapper, setup } = mountModal(false)
    await wrapper.setProps({ open: true })
    await flushPromises()
    expect(setup.loading).toBe(true)
    // gen1 的 A 完成（B 在途）：loading 保持。
    resolvers[0](makeDetail('run-2', 'g1A'))
    await flushPromises()
    expect(setup.loading).toBe(true)
    // 切 A → gen2（旧 gen1 的 B 被作废）。
    setup.runAId = 'run-1'
    await flushPromises()
    expect(setup.loading).toBe(true)
    // resolvers 顺序：[gen1-A(已回), gen1-B, gen2-A, gen2-B]。
    resolvers[1](makeDetail('run-1', 'g1B_LATE')) // gen1 的 B 迟到 → 丢弃
    await flushPromises()
    expect(setup.loading).toBe(true)
    resolvers[2](makeDetail('run-1', 'g2A'))
    await flushPromises()
    resolvers[3](makeDetail('run-1', 'g2B'))
    await flushPromises()
    expect(setup.loading).toBe(false)
  })
})
