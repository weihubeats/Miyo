/**
 * testSuites store：
 * - 启动批次把运行配置（含环境 / 模式 / 失败策略）转成后端 snake_case 入参；
 * - 进度事件按 batch_id 隔离，跨批次事件丢弃；
 * - 失败重试新建批次（parent_batch_id 指向原批次，不覆盖历史）；
 * - 套件保存 / 重排 / 启停同步本地缓存。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const listTestSuites = vi.fn()
const saveTestSuite = vi.fn()
const deleteTestSuite = vi.fn()
const duplicateTestSuite = vi.fn()
const reorderTestSuiteItems = vi.fn()
const setTestSuiteItemEnabled = vi.fn()
const runTestBatch = vi.fn()
const resumeTestBatch = vi.fn()
const pauseTestBatch = vi.fn()
const cancelTestBatch = vi.fn()
const listTestBatches = vi.fn()
const getTestBatch = vi.fn()
const deleteTestBatch = vi.fn()

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    listTestSuites,
    saveTestSuite,
    deleteTestSuite,
    duplicateTestSuite,
    reorderTestSuiteItems,
    setTestSuiteItemEnabled,
    runTestBatch,
    resumeTestBatch,
    pauseTestBatch,
    cancelTestBatch,
    listTestBatches,
    getTestBatch,
    deleteTestBatch,
  }),
}))

import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useWorkspaceStore } from '@/stores/workspace'
import { useTestSuitesStore } from '@/domains/integration/stores/testSuites'
import type {
  TestBatchDetail,
  TestRunBatchItem,
  TestSuiteItem,
  TestSuiteWithItems,
} from '@/types/miyoApi.d'

function batchItem(
  id: string,
  status: TestRunBatchItem['status'],
  sortOrder: number,
  scenarioId = `s-${id}`,
): TestRunBatchItem {
  return {
    id,
    batch_id: 'b1',
    scenario_id: scenarioId,
    scenario_name: `场景 ${id}`,
    status,
    sort_order: sortOrder,
    attempt: 1,
    duration_ms: 12,
    error_message: status === 'failed' ? '断言失败' : null,
    execution_record_id: null,
    started_at: null,
    finished_at: null,
  }
}

function detail(
  id: string,
  items: TestRunBatchItem[],
  overrides: Partial<TestBatchDetail> = {},
): TestBatchDetail {
  const passed = items.filter((i) => i.status === 'passed').length
  const failed = items.filter((i) => i.status === 'failed').length
  return {
    id,
    project_id: 'p1',
    suite_id: null,
    parent_batch_id: null,
    environment_id: null,
    status: 'completed',
    mode: 'serial',
    concurrency: 1,
    failure_policy: 'continue',
    retry_count: 0,
    skip_disabled: true,
    total_count: items.length,
    passed_count: passed,
    failed_count: failed,
    skipped_count: 0,
    cancelled_count: 0,
    running_count: 0,
    pending_count: 0,
    started_at: null,
    finished_at: null,
    duration_ms: 100,
    created_at: new Date().toISOString(),
    items,
    ...overrides,
  }
}

function suite(id: string, items: TestSuiteItem[] = []): TestSuiteWithItems {
  return {
    id,
    project_id: 'p1',
    name: `套件 ${id}`,
    description: '',
    folder_id: null,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    items,
  }
}

function suiteItem(id: string, scenarioId: string, order: number, enabled = true): TestSuiteItem {
  return { id, suite_id: 'su1', scenario_id: scenarioId, sort_order: order, enabled }
}

function setup(): ReturnType<typeof useTestSuitesStore> {
  setActivePinia(createPinia())
  const integration = useIntegrationStore()
  integration.setProject('p1')
  const workspace = useWorkspaceStore()
  workspace.activeEnvId = 'env1'
  return useTestSuitesStore()
}

beforeEach(() => {
  vi.clearAllMocks()
  listTestBatches.mockResolvedValue([])
})

describe('testSuites store', () => {
  it('启动批次：转成 snake_case 入参并回填报告', async () => {
    const store = setup()
    const d = detail('b1', [batchItem('i1', 'passed', 0)], { environment_id: 'env1' })
    runTestBatch.mockResolvedValue(d)

    await store.startBatch(
      { suite_id: 'su1' },
      { mode: 'parallel', concurrency: 4, failure_policy: 'retry', retry_count: 2 },
    )

    expect(runTestBatch).toHaveBeenCalledWith(
      expect.objectContaining({
        project_id: 'p1',
        suite_id: 'su1',
        scenario_ids: [],
        folder_id: null,
        environment_id: 'env1',
        mode: 'parallel',
        concurrency: 4,
        failure_policy: 'retry',
        retry_count: 2,
        skip_disabled: true,
        parent_batch_id: null,
      }),
    )
    expect(store.activeBatchId).toBe('b1')
    expect(store.activeBatch?.id).toBe('b1')
    expect(store.panelOpen).toBe(true)
    expect(store.starting).toBe(false)
    expect(store.progress.done).toBe(1)
  })

  it('进度事件：跨批次丢弃，终态回读详情与历史', async () => {
    const store = setup()
    const d = detail('b1', [batchItem('i1', 'passed', 0)])
    getTestBatch.mockResolvedValue(d)

    store.applyProgress({
      batch_id: 'b1',
      status: 'running',
      total: 2,
      done: 1,
      passed: 1,
      failed: 0,
      skipped: 0,
      cancelled: 0,
      running: 1,
      current: null,
      item: null,
    })
    await flushPromises()
    expect(store.activeBatchId).toBe('b1')

    // 另一个批次的迟到事件不得污染当前批次。
    store.applyProgress({
      batch_id: 'b2',
      status: 'running',
      total: 9,
      done: 9,
      passed: 9,
      failed: 0,
      skipped: 0,
      cancelled: 0,
      running: 0,
      current: null,
      item: null,
    })
    expect(store.progress.total).toBe(2)

    store.applyProgress({
      batch_id: 'b1',
      status: 'completed',
      total: 2,
      done: 2,
      passed: 2,
      failed: 0,
      skipped: 0,
      cancelled: 0,
      running: 0,
      current: null,
      item: batchItem('i2', 'passed', 1),
    })
    await flushPromises()

    expect(store.progress.done).toBe(2)
    expect(store.activeItems.map((i) => i.id)).toEqual(['i1', 'i2'])
    expect(getTestBatch).toHaveBeenCalledWith('b1')
    expect(listTestBatches).toHaveBeenCalled()
  })

  it('失败重试：新建批次，parent_batch_id 指向原批次', async () => {
    const store = setup()
    store.activeBatch = detail('b1', [
      batchItem('i1', 'passed', 0),
      batchItem('i2', 'failed', 1, 'sc-fail'),
    ])
    runTestBatch.mockResolvedValue(detail('b2', [batchItem('i3', 'passed', 0)]))

    await store.retryFailed()

    expect(runTestBatch).toHaveBeenCalledWith(
      expect.objectContaining({
        scenario_ids: ['sc-fail'],
        parent_batch_id: 'b1',
        environment_id: 'env1',
      }),
    )
    expect(store.activeBatchId).toBe('b2')
  })

  it('整批重跑：套件批次复用 suite_id 并新建批次', async () => {
    const store = setup()
    store.activeBatch = detail('b1', [batchItem('i1', 'passed', 0)], {
      suite_id: 'su1',
      mode: 'parallel',
      concurrency: 5,
    })
    runTestBatch.mockResolvedValue(detail('b2', [batchItem('i2', 'passed', 0)]))

    await store.rerunBatch()

    expect(runTestBatch).toHaveBeenCalledWith(
      expect.objectContaining({
        suite_id: 'su1',
        scenario_ids: [],
        parent_batch_id: 'b1',
        mode: 'parallel',
        concurrency: 5,
      }),
    )
    expect(store.activeBatchId).toBe('b2')
  })

  it('批次详情返回执行历史：清空当前批次但保留面板', async () => {
    const store = setup()
    store.activeBatch = detail('b1', [batchItem('i1', 'passed', 0)])
    store.activeBatchId = 'b1'
    store.panelOpen = true

    store.showHistory()

    expect(store.activeBatchId).toBeNull()
    expect(store.activeBatch).toBeNull()
    expect(store.panelOpen).toBe(true)
  })

  it('套件选择状态可外部同步（路由 ?suite=）', () => {
    const store = setup()
    store.setActiveSuite('su9')
    expect(store.activeSuiteId).toBe('su9')
    store.setActiveSuite(null)
    expect(store.activeSuiteId).toBeNull()
  })

  it('套件保存 / 重排 / 启停同步本地缓存', async () => {
    const store = setup()
    const items = [
      suiteItem('i1', 'sc1', 0),
      suiteItem('i2', 'sc2', 1),
    ]
    saveTestSuite.mockResolvedValue(suite('su1', items))
    store.suites = [suite('su1', items)]

    await store.saveSuite(suite('su1'), items)
    expect(store.suites).toHaveLength(1)

    reorderTestSuiteItems.mockResolvedValue(undefined)
    await store.reorderItems('su1', ['i2', 'i1'])
    expect(store.suites[0].items.map((i) => i.id)).toEqual(['i2', 'i1'])

    setTestSuiteItemEnabled.mockResolvedValue(undefined)
    await store.toggleItem('su1', 'i1', false)
    expect(store.suites[0].items.find((i) => i.id === 'i1')?.enabled).toBe(false)
  })
})