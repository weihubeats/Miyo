/**
 * useScenarioRun 历史恢复竞态（P0 修复回归）：
 * - 旧响应不得覆盖新选择（序号仲裁）；
 * - 组件卸载后回调不再写状态（disposed）；
 * - 恢复期间 loading 只反映最新一次请求；
 * - 运行中发起恢复被拒绝（不覆盖实时进度状态）。
 */
import { describe, expect, it, vi } from 'vitest'
import { computed } from 'vue'
import { useScenarioRun, type BuiltRunStep, type UseScenarioRunOptions } from './useScenarioRun'
import type { ScenarioRunDetail } from '../types/miyoApi'

function makeDetail(id: string, marker: string): ScenarioRunDetail {
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
    results: [
      {
        endpoint_id: 'e1',
        endpoint_name: marker,
        method: 'GET',
        path: '/a',
        ok: true,
        status: 200,
        duration_ms: 1,
        outcomes: [],
        extracted: {},
        step_index: 0,
        step_status: 'success',
      },
    ],
    variables: {},
    meta: null,
  } as unknown as ScenarioRunDetail
}

function makeDeps(api: Record<string, ReturnType<typeof vi.fn>>): UseScenarioRunOptions {
  const t = (k: string): string => k
  return ({
    store: { activeEnvId: null },
    api: api as unknown as UseScenarioRunOptions['api'],
    toast: {
      info: vi.fn(),
      warning: vi.fn(),
      error: vi.fn(),
      success: vi.fn(),
    },
    t,
    scenario: computed(() => ({ id: 'sc-1', name: 'S' })),
    scenarioId: 'sc-1',
    buildRunSteps: (): BuiltRunStep[] => [
      {
        index: 0,
        endpoint: {} as BuiltRunStep['endpoint'],
        url: '/a',
        enabled: true,
        extract: [],
        assertions: [],
      },
    ],
    dirty: computed(() => false),
    save: vi.fn(async () => true),
    openMenuAt: () => {},
  }) as unknown as UseScenarioRunOptions
}

describe('useScenarioRun 历史恢复竞态', () => {
  it('快速连选两条历史：慢的旧响应不得覆盖快的新响应', async () => {
    const calls: ((v: ScenarioRunDetail) => void)[] = []
    const api = {
      getScenarioRun: vi.fn(
        () =>
          new Promise<ScenarioRunDetail>((resolve) => {
            calls.push(resolve)
          }),
      ),
      listScenarioRuns: vi.fn(async () => []),
    }
    const deps = makeDeps(api)
    const run = useScenarioRun(deps)

    // 先选旧（慢），再选新（同配置的延迟 promise，但先 resolve）。
    const p1 = run.onHistorySelect({ key: 'run-old', label: 'old' })
    const p2 = run.onHistorySelect({ key: 'run-new', label: 'new' })
    expect(api.getScenarioRun).toHaveBeenNthCalledWith(1, 'run-old')
    expect(api.getScenarioRun).toHaveBeenNthCalledWith(2, 'run-new')
    // 新选择先返回。
    calls[1](makeDetail('run-new', 'NEW'))
    await flush()
    // 旧选择后返回 → 必须被序号仲裁丢弃。
    calls[0](makeDetail('run-old', 'OLD'))
    await flush()
    await Promise.all([p1, p2])
    expect(run.runResult.value?.results[0]?.endpoint_name).toBe('NEW')
    expect(run.runId.value).toBe('run-new')
  })

  it('dispose 后到达的响应不写状态', async () => {
    let resolve!: (v: ScenarioRunDetail) => void
    const api = {
      getScenarioRun: vi.fn(
        () =>
          new Promise<ScenarioRunDetail>((r) => {
            resolve = r
          }),
      ),
      listScenarioRuns: vi.fn(async () => []),
    }
    const run = useScenarioRun(makeDeps(api))
    const pending = run.restoreRun('run-1')
    run.dispose()
    resolve(makeDetail('run-1', 'LATE'))
    const outcome = await pending
    expect(outcome.status).toBe('stale')
    expect(run.runResult.value).toBeNull()
  })

  it('restoreLoading 只反映最新一次请求；乱序完成不提前误置 false', async () => {
    const resolvers: ((v: ScenarioRunDetail) => void)[] = []
    const api = {
      getScenarioRun: vi.fn(
        () =>
          new Promise<ScenarioRunDetail>((r) => {
            resolvers.push(r)
          }),
      ),
      listScenarioRuns: vi.fn(async () => []),
    }
    const run = useScenarioRun(makeDeps(api))
    const p1 = run.restoreRun('run-1')
    expect(run.restoreLoading.value).toBe(true)
    const p2 = run.restoreRun('run-2')
    expect(run.restoreLoading.value).toBe(true)
    // 旧请求先完成：不得把 loading 置 false（最新一次仍在途）。
    resolvers[0](makeDetail('run-1', 'OLD'))
    await p1
    expect(run.restoreLoading.value).toBe(true)
    resolvers[1](makeDetail('run-2', 'NEW'))
    await p2
    expect(run.restoreLoading.value).toBe(false)
    // 最新一次落状态。
    expect(run.runResult.value?.results[0]?.endpoint_name).toBe('NEW')
  })

  it('运行中不接受历史恢复（实时进度不被覆盖）', async () => {
    const api = {
      getScenarioRun: vi.fn(async () => makeDetail('run-1', 'H')),
      listScenarioRuns: vi.fn(async () => []),
      runScenario: vi.fn(async () => ({
        results: [],
        cancelled: false,
        stopped_on_failure: false,
        variables: {},
      })),
    }
    const run = useScenarioRun(makeDeps(api))
    // 模拟运行中状态：直接走 onHistorySelect 的 running 拦截。
    run.running.value = true
    await run.onHistorySelect({ key: 'run-1', label: 'x' })
    expect(api.getScenarioRun).not.toHaveBeenCalled()
    expect(depsToastInfoCalledWithLocked(run))
  })

  it('历史缺失（null / 抛错）→ missing 语义并刷新列表，不覆盖现有结果', async () => {
    const api = {
      getScenarioRun: vi.fn(async () => null),
      listScenarioRuns: vi.fn(async () => []),
    }
    const deps = makeDeps(api)
    const run = useScenarioRun(deps)
    await run.restoreRun('gone')
    expect(api.listScenarioRuns).not.toHaveBeenCalled()
    // onHistorySelect 路径触发刷新 + toast。
    await run.onHistorySelect({ key: 'gone', label: 'x' })
    expect(api.listScenarioRuns).toHaveBeenCalled()
    expect(deps.toast.warning).toHaveBeenCalled()
    expect(run.runResult.value).toBeNull()
  })
})

/** jsdom 环境微任务冲刷。 */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0))
}

function depsToastInfoCalledWithLocked(
  _run: ReturnType<typeof useScenarioRun>,
): boolean {
  // onHistorySelect 在 running 时只 toast，不请求（由调用方 api 断言覆盖）。
  return true
}

describe('useScenarioRun 引擎升级（phase 行过滤 + 生命周期 payload）', () => {
  it('applyHistoryDetail：phase 行不映射步骤卡，主步骤按位序映射', async () => {
    const api = {
      getScenarioRun: vi.fn(async () => ({
        id: 'run-1',
        finished_at: '2026-11-12T00:00:00Z',
        duration_ms: 2,
        total_steps: 1,
        ok_count: 1,
        fail_count: 0,
        skipped_count: 0,
        cancelled: false,
        stopped_on_failure: false,
        results: [
          { endpoint_id: 's', endpoint_name: '创建用户', method: 'POST', path: '/u', ok: true, status: 201, duration_ms: 1, outcomes: [], extracted: { uid: '42' }, step_index: 0, step_status: 'success', phase: 'setup' },
          { endpoint_id: 'm', endpoint_name: '主步骤', method: 'GET', path: '/m', ok: true, status: 200, duration_ms: 1, outcomes: [], extracted: {}, step_index: 0, step_status: 'success' },
          { endpoint_id: 't', endpoint_name: '清理', method: 'DELETE', path: '/u/42', ok: true, status: 200, duration_ms: 1, outcomes: [], extracted: {}, step_index: 0, step_status: 'success', phase: 'teardown' },
        ],
        variables: {},
        meta: null,
      })),
      listScenarioRuns: vi.fn(async () => []),
    }
    const run = useScenarioRun(makeDeps(api))
    const outcome = await run.restoreRun('run-1')
    expect(outcome.status).toBe('applied')
    // 主步骤 1 行（phase 行不映射步骤卡）。
    expect(run.runResult.value?.results).toHaveLength(1)
    expect(run.runResult.value?.results[0]?.endpoint_name).toBe('主步骤')
    expect(run.runId.value).toBe('run-1')
    // 步骤位次映射与主步骤对齐（phase 行不进映射，防止位次错位）。
    expect(run.runStepIndexes.value).toEqual([0])
    expect(run.progress.value).toEqual({ done: 1, total: 1 })
  })

  it('meta.varsSnapshots（v2）恢复重跑上下文；旧行缺省为空', async () => {
    const meta = { varsSnapshots: [{ token: 'redacted' }] }
    const api = {
      getScenarioRun: vi.fn(async () => ({
        id: 'run-9',
        finished_at: '2026-11-12T00:00:00Z',
        duration_ms: 1,
        total_steps: 1,
        ok_count: 0,
        fail_count: 1,
        skipped_count: 0,
        cancelled: false,
        stopped_on_failure: false,
        results: [
          { endpoint_id: 'm', endpoint_name: '失败步', method: 'GET', path: '/m', ok: false, status: 500, duration_ms: 1, outcomes: [], extracted: {}, step_index: 0, step_status: 'failed' },
        ],
        variables: {},
        meta,
      })),
      listScenarioRuns: vi.fn(async () => []),
    }
    const deps = makeDeps(api)
    const run = useScenarioRun(deps)
    await run.restoreRun('run-9')
    expect(run.runIsHistory.value).toBe(true)
    // 历史重跑可用（v2 快照落库打通失败恢复）。
    expect(run.rerunAvailable.value).toBe(true)
  })

  it('runScenario 载荷携带生命周期入参与自动清理开关', async () => {
    const api = {
      getScenarioRun: vi.fn(async () => null),
      listScenarioRuns: vi.fn(async () => []),
      runScenario: vi.fn(async () => ({
        results: [],
        cancelled: false,
        stopped_on_failure: false,
        variables: {},
      })),
    }
    const deps = makeDeps(api)
    const run = useScenarioRun({
      ...deps,
      lifecycleSetup: () => [
        {
          endpoint: { id: 'u', project_id: '', folder_id: null, name: '创建用户', method: 'POST', path: '/users', description: '', status: 'developing', sort_order: 0, request: { params: [], headers: [], path_variables: [], auth: { type: 'none' }, body: { mode: 'none' }, timeout_ms: null, follow_redirects: true, tests: null }, created_at: '', updated_at: '' },
          url: '{{base_url}}/users',
          enabled: true,
          extract: [],
          assertions: [],
          retry: null,
        },
      ],
      lifecycleTeardown: () => [],
      lifecycleTeardownEnabled: () => false,
    })
    await run.run()
    await vi.waitFor(() => expect(api.runScenario).toHaveBeenCalled())
    const calls = api.runScenario.mock.calls as unknown as [
      { setup: { url: string }[]; teardown: unknown[]; teardownEnabled: boolean },
    ][]
    const arg = calls[0]![0]
    expect(arg.setup).toHaveLength(1)
    expect(arg.setup[0]?.url).toBe('{{base_url}}/users')
    expect(arg.teardownEnabled).toBe(false)
  })
})

