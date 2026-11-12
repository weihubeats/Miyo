/**
 * useScenarioRun：历史恢复链路回归。
 *
 * 锁定三个已修复的时序缺陷：
 * 1. 挂载链路 loadHistory → restoreLatestRun 必须串行（此前并行导致
 *    restoreLatestRun 恒读空列表，「恢复最近一次运行」整体失效）；
 * 2. loadHistory 乱序返回不得用过期数据覆盖新数据（序号守卫）；
 * 3. 运行中不接受历史回填（applyHistoryDetail / onHistorySelect 守卫），
 *    否则 runId 被历史覆盖、实时进度事件全被过滤。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { computed } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import type { ScenarioRunDetail, ScenarioRunSummary } from '../types/miyoApi'
import { useScenarioRun, type BuiltRunStep } from './useScenarioRun'

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function summary(id: string): ScenarioRunSummary {
  return {
    id,
    finished_at: `2026-01-01T00:0${id.length}:00Z`,
    duration_ms: 100,
    total_steps: 2,
    ok_count: 2,
    fail_count: 0,
    skipped_count: 0,
    cancelled: false,
    stopped_on_failure: false,
  }
}

function detail(id: string): ScenarioRunDetail {
  const okStep = (stepIndex: number): ScenarioRunDetail['results'][number] => ({
    endpoint_id: `ep-${stepIndex}`,
    endpoint_name: `步骤 ${stepIndex + 1}`,
    method: 'GET',
    path: `/x/${stepIndex}`,
    ok: true,
    status: 200,
    duration_ms: 50,
    url: `http://x/${stepIndex}`,
    step_index: stepIndex,
    step_status: 'success',
    outcomes: [],
  })
  return {
    ...summary(id),
    results: [okStep(0), okStep(1)],
    variables: {},
    meta: null,
  }
}

/** 组装 composable：api 可逐用例覆盖 mock。 */
function setup(apiOverrides: Record<string, unknown> = {}) {
  const pinia = createPinia()
  setActivePinia(pinia)
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  const toast = { info: vi.fn(), warning: vi.fn(), error: vi.fn(), success: vi.fn() }
  const api = {
    listScenarioRuns: vi.fn(async () => [] as ScenarioRunSummary[]),
    getScenarioRun: vi.fn(async () => null as ScenarioRunDetail | null),
    runScenario: vi.fn(),
    cancelScenarioRun: vi.fn(async () => undefined),
    getResponseBody: vi.fn(async () => ''),
    ...apiOverrides,
  }
  const run = useScenarioRun({
    store,
    api: api as never,
    toast: toast as never,
    t: (key: string) => key,
    scenario: computed(() => ({ id: 'sc-1', name: '场景一' })),
    scenarioId: 'sc-1',
    buildRunSteps: () =>
      [
        {
          index: 0,
          endpoint: {} as BuiltRunStep['endpoint'],
          url: 'http://x/1',
          enabled: true,
          extract: [],
          assertions: [],
        },
        {
          index: 1,
          endpoint: {} as BuiltRunStep['endpoint'],
          url: 'http://x/2',
          enabled: true,
          extract: [],
          assertions: [],
        },
      ] satisfies BuiltRunStep[],
    dirty: computed(() => false),
    save: async () => true,
    openMenuAt: vi.fn(),
  })
  return { run, api, toast }
}

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
})

describe('useScenarioRun 历史恢复', () => {
  it('loadHistory 完成后 restoreLatestRun 恢复最近一次运行（挂载链路时序）', async () => {
    const { run, api } = setup({
      listScenarioRuns: vi.fn(async () => [summary('r-old'), summary('r-newest')]),
      getScenarioRun: vi.fn(async (id: string) => detail(id)),
    })
    // 模拟挂载链路：列表先行，恢复随后（WorkflowEditor 的 .then 串联）。
    await run.loadHistory()
    await run.restoreLatestRun()
    expect(api.getScenarioRun).toHaveBeenCalledWith('r-old') // 列表首项（新→旧）
    expect(run.runIsHistory.value).toBe(true)
    expect(run.runId.value).toBe('r-old')
    expect(run.runResult.value?.results).toHaveLength(2)
    expect(run.progress.value).toEqual({ done: 2, total: 2 })
  })

  it('restoreLatestRun 语义锁死：列表为空时保持空态不误挂历史', async () => {
    const { run } = setup({
      getScenarioRun: vi.fn(async (id: string) => detail(id)),
    })
    await run.loadHistory()
    await run.restoreLatestRun()
    expect(run.runIsHistory.value).toBe(false)
    expect(run.runResult.value).toBeNull()
  })

  it('loadHistory 乱序返回：过期结果丢弃，不覆盖新数据', async () => {
    const slow = deferred<ScenarioRunSummary[]>()
    const { run, api } = setup()
    api.listScenarioRuns.mockImplementationOnce(() => slow.promise)
    // 第一次调用挂在受控 Promise 上（旧请求，慢）。
    const first = run.loadHistory()
    // 第二次调用立即返回新数据（新请求，快）。
    api.listScenarioRuns.mockImplementationOnce(async () => [summary('r-2')])
    await run.loadHistory()
    expect(run.runHistory.value.map((s) => s.id)).toEqual(['r-2'])
    // 旧请求此刻才返回：不得覆盖新数据。
    slow.resolve([summary('r-stale')])
    await first
    expect(run.runHistory.value.map((s) => s.id)).toEqual(['r-2'])
  })

  it('loadHistory 失败保留现有列表（不清空、不误伤并发新数据）', async () => {
    const { run, api } = setup({
      listScenarioRuns: vi.fn(async () => [summary('r-1')]),
    })
    await run.loadHistory()
    expect(run.runHistory.value).toHaveLength(1)
    api.listScenarioRuns.mockRejectedValueOnce(new Error('ipc down'))
    await run.loadHistory()
    expect(run.runHistory.value).toHaveLength(1)
  })

  it('运行中 applyHistoryDetail 为 no-op，运行结束恢复生效', async () => {
    const { run } = setup({
      getScenarioRun: vi.fn(async () => detail('r-hist')),
    })
    run.running.value = true
    run.applyHistoryDetail(detail('r-hist'))
    expect(run.runResult.value).toBeNull()
    expect(run.runId.value).toBe('')
    run.running.value = false
    run.applyHistoryDetail(detail('r-hist'))
    expect(run.runIsHistory.value).toBe(true)
    expect(run.runId.value).toBe('r-hist')
  })

  it('运行中 onHistorySelect 拦截并提示，不发起详情请求', async () => {
    const { run, api, toast } = setup({
      getScenarioRun: vi.fn(async () => detail('r-hist')),
    })
    run.running.value = true
    await run.onHistorySelect({ key: 'r-hist', label: 'x' })
    expect(toast.info).toHaveBeenCalled()
    expect(api.getScenarioRun).not.toHaveBeenCalled()
    expect(run.runResult.value).toBeNull()
  })

  it('进度事件 runId 过滤：runId 不匹配时丢弃（历史覆盖破坏实时性的防线仍在）', () => {
    const { run } = setup()
    run.running.value = true
    run.runId.value = 'live-run'
    run.applyProgressEvent({
      runId: 'other-run',
      done: 1,
      total: 2,
      result: detail('x').results[0],
    })
    expect(run.liveResults.value).toHaveLength(0)
    run.applyProgressEvent({
      runId: 'live-run',
      done: 1,
      total: 2,
      result: detail('x').results[0],
    })
    expect(run.liveResults.value[0]?.url).toBe('http://x/0')
  })
})
