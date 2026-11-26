/**
 * integration store：项目上下文初始化 / 场景库缓存 / 编辑目标清理。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import type { ScenarioLibraryItem } from '@/types/miyoApi'

// 提升为模块级 mock（工厂内每次 useMiyoApi() 新建对象，测试要 override 必须持稳引用）
const listScenarioSummariesMock = vi.fn(async (pid: string) =>
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
)

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    pending: { value: false },
    getProjects: vi.fn(async () => [
      { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' },
      { id: 'p-2', name: 'B', description: '', variables: {}, created_at: '', updated_at: '' },
    ]),
    listScenarioSummaries: (...args: Parameters<typeof listScenarioSummariesMock>) =>
      listScenarioSummariesMock(...args),
    listScenarioFolders: vi.fn(async (pid: string) =>
      pid === 'p-1'
        ? [
            {
              id: 'f-1',
              project_id: 'p-1',
              parent_id: null,
              name: '电商业务',
              sort_order: 0,
              created_at: '',
              updated_at: '',
            },
          ]
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

  it('业务目录加载 / 切换项目清批量选择', async () => {
    const store = useIntegrationStore()
    await store.ensureProjectContext()
    store.setProject('p-1')
    await store.loadFolders()
    expect(store.folders).toHaveLength(1)
    expect(store.folders[0].name).toBe('电商业务')

    store.selectScenarios(['sc-1', 'sc-2'])
    expect(store.selectedScenarioIds).toEqual(['sc-1', 'sc-2'])
    store.clearSelection()
    expect(store.selectedScenarioIds).toEqual([])

    store.selectScenarios(['sc-1'])
    store.setProject('p-2') // 切项目：选择清空（避免跨项目批量操作）
    expect(store.selectedScenarioIds).toEqual([])
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

  it('列表视图状态跨路由存活：进编辑器（换 activeScenarioId）不重置，setProject 也不清', async () => {
    const store = useIntegrationStore()
    // 模拟列表页的搜索 / 筛选 / 排序 / 滚动记忆
    store.libraryView.search = '下单'
    store.libraryView.categoryFilter = '电商'
    store.libraryView.statusFilter = 'fail'
    store.libraryView.sortBy = 'recent'
    store.libraryView.scrollTop = 260
    store.libraryView.projectIdAtScroll = 'p-1'

    store.openScenario('sc-1') // 进编辑器
    expect(store.activeScenarioId).toBe('sc-1')
    store.backToLibrary() // 返回列表
    expect(store.libraryView).toMatchObject({
      search: '下单',
      categoryFilter: '电商',
      statusFilter: 'fail',
      sortBy: 'recent',
      scrollTop: 260,
      projectIdAtScroll: 'p-1',
    })
    // 切项目：数据（列表 / 编辑态）清空，视图状态保留（搜索词是通用输入）
    store.setProject('p-2')
    expect(store.activeScenarioId).toBeNull()
    expect(store.libraryView.search).toBe('下单')
    // 但滚动记忆归 p-1：恢复逻辑不得把 p-1 的滚动位置套到 p-2
    expect(store.libraryView.projectIdAtScroll).toBe('p-1')
  })

  it('loadLibrary 竞态：旧项目迟到响应不覆盖新项目，loading 只由最新请求关闭', async () => {
    const store = useIntegrationStore()
    await store.ensureProjectContext()
    store.setProject('p-1')

    // p-1 慢响应（先发起）；切到 p-2 后 p-2 快响应后发先至
    let resolveP1!: (v: ScenarioLibraryItem[]) => void
    let resolveP2!: (v: ScenarioLibraryItem[]) => void
    listScenarioSummariesMock.mockImplementation((pid: string) => {
      if (pid === 'p-1') {
        return new Promise<ScenarioLibraryItem[]>((resolve) => (resolveP1 = resolve))
      }
      return new Promise<ScenarioLibraryItem[]>((resolve) => (resolveP2 = resolve))
    })

    const slow = store.loadLibrary() // p-1：在途
    store.setProject('p-2') // 切项目（编辑态清空；单元测试里手动触发新加载）
    const fast = store.loadLibrary() // p-2：在途
    expect(store.libraryLoading).toBe(true)

    // 旧项目响应先返回：不得应用，也不得把 loading 关掉（新请求仍在途）
    resolveP1([{ id: 'sc-old', project_id: 'p-1', name: '旧项目场景', description: '', category: '', sort_order: 0, steps: [], created_at: '', updated_at: '', last_run: null }])
    await slow
    expect(store.libraryLoading).toBe(true)
    expect(store.libraryLoaded).toBe(false) // 旧响应不得宣称「已加载」

    // 新项目响应返回：应用 + loading 关闭
    resolveP2([])
    await fast
    expect(store.libraryItems).toHaveLength(0)
    expect(store.libraryLoaded).toBe(true)
    expect(store.libraryLoading).toBe(false)
    listScenarioSummariesMock.mockRestore()
  })
})
