/**
 * integration store：项目上下文初始化 / 场景库缓存 / 编辑目标清理。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useIntegrationStore } from './integration'
import { useWorkspaceStore } from './workspace'
import { useLocaleStore } from './locale'
import type { ScenarioLibraryItem } from '../types/miyoApi'

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => ({
    pending: { value: false },
    getProjects: vi.fn(async () => [
      { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' },
      { id: 'p-2', name: 'B', description: '', variables: {}, created_at: '', updated_at: '' },
    ]),
    listScenarioSummaries: vi.fn(async (pid: string) =>
      pid === 'p-1'
        ? ([
            {
              id: 'sc-1',
              project_id: 'p-1',
              name: '下单链路',
              description: '',
              category: '电商',
              sort_order: 0,
              steps: [],
              created_at: '',
              updated_at: '',
              last_run: null,
            },
          ] as ScenarioLibraryItem[])
        : [],
    ),
  }),
}))

describe('integration store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('项目上下文初始化：跟随工作区激活项目；无激活项目时取第一个', async () => {
    const store = useIntegrationStore()
    const ws = useWorkspaceStore()
    ws.project = { id: 'p-2', name: 'B', description: '', variables: {}, created_at: '', updated_at: '' }
    const pid = await store.ensureProjectContext()
    expect(pid).toBe('p-2')
    // 幂等：再次调用不重置
    expect(await store.ensureProjectContext()).toBe('p-2')
  })

  it('场景库加载与切换项目清编辑态', async () => {
    const store = useIntegrationStore()
    await store.ensureProjectContext()
    store.setProject('p-1')
    await store.loadLibrary()
    expect(store.libraryItems).toHaveLength(1)
    expect(store.libraryItems[0].name).toBe('下单链路')
    // 打开 → 切项目 → 编辑态清空
    store.openScenario('sc-1')
    expect(store.activeScenarioId).toBe('sc-1')
    store.setProject('p-2')
    expect(store.activeScenarioId).toBeNull()
  })

  it('forgetScenario 清编辑态并从列表移除', async () => {
    const store = useIntegrationStore()
    await store.ensureProjectContext()
    store.setProject('p-1')
    await store.loadLibrary()
    store.openScenario('sc-1')
    store.forgetScenario('sc-1')
    expect(store.activeScenarioId).toBeNull()
    expect(store.libraryItems).toHaveLength(0)
  })
})
