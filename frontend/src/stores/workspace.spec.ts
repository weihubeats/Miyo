/**
 * workspace store 多项目快照切换单测：
 * 项目 A 的草稿/标签在切到 B 再切回后必须原样恢复；B 首次进入为全新态。
 * useMiyoApi 以模块级 mock 替换（内存假后端）。
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { CurlParsed, Endpoint, Environment, Project, TestCase } from '../types/miyoApi'

function makeProject(id: string, name: string): Project {
  const now = new Date().toISOString()
  return { id, name, description: '', variables: {}, created_at: now, updated_at: now }
}

function makeEndpoint(id: string, projectId: string, name: string, now = new Date().toISOString()): Endpoint {
  return {
    id,
    project_id: projectId,
    folder_id: null,
    name,
    method: 'GET',
    path: `/${name}`,
    description: '',
    status: 'designing',
    sort_order: 0,
    request: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'none' },
      active_tab: null,
      timeout_ms: 30000,
      follow_redirects: true,
      tests: null,
    },
    created_at: now,
    updated_at: now,
  }
}

const backend = vi.hoisted(() => {
  const projects: Project[] = []
  const endpointsByProject = new Map<string, Endpoint[]>()
  const envsByProject = new Map<string, Environment[]>()
  let activeId: string | null = null
  /** 历史接口行为控制（分页 / 失败提示用例）。 */
  const historyRows: { id: string }[] = []
  const historyCalls: (number | undefined)[] = []
  /** executeRequest 收到的调用参数（断言向后端送出的 URL）。 */
  const execCalls: { url: string; spec: unknown }[] = []
  /** cancelRequest 收到的 request_id（超时用例断言前端确实取消了在途请求）。 */
  const cancelCalls: string[] = []
  /** 注入 executeRequest 行为（超时用例让其悬挂直到 cancel）。 */
  let executeImpl: ((args: { url: string; spec: unknown }) => Promise<unknown>) | null = null
  /** 注入 cancelRequest 行为（通常用来 reject 在途的 executeRequest）。 */
  let cancelImpl: ((id: string) => void) | null = null
  let historyImpl:
    | ((projectId: string, limit?: number, endpointId?: string | null) => Promise<{ id: string }[]>)
    | null = null
  return {
    projects,
    endpointsByProject,
    envsByProject,
    setActive: (id: string | null) => {
      activeId = id
    },
    active: () => activeId,
    historyRows,
    historyCalls,
    execCalls,
    cancelCalls,
    setExecuteImpl: (fn: ((args: { url: string; spec: unknown }) => Promise<unknown>) | null) => {
      executeImpl = fn
    },
    executeImpl: () => executeImpl,
    setCancelImpl: (fn: ((id: string) => void) | null) => {
      cancelImpl = fn
    },
    cancelImpl: () => cancelImpl,
    setHistoryImpl: (
      fn: ((projectId: string, limit?: number, endpointId?: string | null) => Promise<{ id: string }[]>) | null,
    ) => {
      historyImpl = fn
    },
    historyImpl: () => historyImpl,
  }
})

/** useToast 单例句柄（断言失败提示只出现一次时用）。 */
const toastMock = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
  warning: vi.fn(),
}))

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => ({
    getProjects: vi.fn().mockResolvedValue(backend.projects),
    getActiveProject: vi.fn().mockImplementation(async () => backend.projects.find((p) => p.id === backend.active()) ?? null),
    setActiveProject: vi.fn().mockImplementation(async (id: string | null) => {
      backend.setActive(id)
      return backend.projects.find((p) => p.id === id) ?? null
    }),
    listFolders: vi.fn().mockResolvedValue([]),
    listEndpoints: vi.fn().mockImplementation(async (pid: string) => backend.endpointsByProject.get(pid) ?? []),
    listEnvironments: vi.fn().mockImplementation(async () => []),
    getActiveEnvironment: vi.fn().mockResolvedValue(null),
    setActiveEnvironment: vi.fn().mockResolvedValue(undefined),
    getGlobalVariables: vi.fn().mockResolvedValue([]),
    saveGlobalVariables: vi.fn().mockResolvedValue(undefined),
    getGlobalParams: vi.fn().mockResolvedValue([]),
    saveGlobalParams: vi.fn().mockResolvedValue(undefined),
    listExamples: vi.fn().mockResolvedValue([]),
    saveExample: vi.fn().mockImplementation(async (ex: unknown) => ex),
    listRequestExamples: vi.fn().mockResolvedValue([]),
    listTestCases: vi.fn().mockResolvedValue([]),
    // 全局超时：模拟「从未配置过」——后端返回 null，store 应回退内置默认
    getHttpTimeoutMs: vi.fn().mockResolvedValue(null),
    listRequestHistories: vi
      .fn()
      .mockImplementation((projectId: string, limit?: number, endpointId?: string | null) => {
        backend.historyCalls.push(limit)
        const impl = backend.historyImpl()
        if (impl) return impl(projectId, limit, endpointId)
        return Promise.resolve(backend.historyRows.slice(0, limit ?? 50))
      }),
    saveEndpoint: vi.fn().mockImplementation(async (ep: Endpoint) => {
      const list = backend.endpointsByProject.get(ep.project_id) ?? []
      const idx = list.findIndex((x) => x.id === ep.id)
      if (idx === -1) list.push(ep)
      else list[idx] = ep
      return ep
    }),
    /** 记录 executeRequest 收到的 url/spec，供「用例运行是否代入路径变量」断言。 */
    executeRequest: vi.fn().mockImplementation(async (args: { url: string; spec: unknown }) => {
      backend.execCalls.push(args)
      // 超时用例需悬挂请求直到 cancelRequest：由测试通过 setExecuteImpl 注入。
      const impl = backend.executeImpl()
      if (impl) return impl(args)
      return {
        status: 200,
        status_text: 'OK',
        headers: {},
        body: '{}',
        duration_ms: 1,
        size_bytes: 2,
        content_type: 'application/json',
        received_at: new Date().toISOString(),
      }
    }),
    cancelRequest: vi.fn().mockImplementation(async (id: string) => {
      backend.cancelCalls.push(id)
      const impl = backend.cancelImpl()
      impl?.(id)
    }),
  }),
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => toastMock,
}))

import { useWorkspaceStore, FALLBACK_GLOBAL_TIMEOUT_MS } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'

/** 测试环境的 localStorage 为残缺对象（真机 WebView 才有完整实现），stub 一个内存版。 */
beforeAll(() => {
  const mem = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => void mem.clear(),
  })
})

describe('workspace store 多项目快照切换', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive(null)
    const a = makeProject('p-a', '项目A')
    const b = makeProject('p-b', '项目B')
    backend.projects.push(a, b)
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.endpointsByProject.set('p-b', [makeEndpoint('ep-2', 'p-b', 'orders')])
    backend.envsByProject.set('p-a', [])
    backend.envsByProject.set('p-b', [])
  })

  it('切走再切回：草稿与标签原样恢复；新项目为全新态', async () => {
    const store = useWorkspaceStore()

    // 进入 A，打开接口并修改草稿（脏）
    backend.setActive('p-a')
    await store.init()
    expect(store.project?.id).toBe('p-a')
    store.openEndpoint(store.endpoints[0]!)
    store.draftOf('ep-1')!.name = '用户列表-改'

    // 切到 B：全新态（无标签），A 的草稿不丢
    await store.switchProject('p-b')
    expect(store.project?.id).toBe('p-b')
    expect(store.openTabs).toHaveLength(0)
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a', 'p-b'])

    // 切回 A：标签与脏草稿原样恢复
    await store.switchProject('p-a')
    expect(store.project?.id).toBe('p-a')
    expect(store.openTabs).toEqual(['ep-1'])
    expect(store.draftOf('ep-1')?.name).toBe('用户列表-改')
    expect(store.isDirty('ep-1')).toBe(true)

    // 关闭 A 标签：自动切到 B
    store.closeProjectTab('p-a')
    await vi.waitFor(() => expect(store.project?.id).toBe('p-b'))
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-b'])

    // 关闭最后一个标签：project 清空（视图负责跳转）
    store.closeProjectTab('p-b')
    expect(store.project).toBeNull()
  })

  it('切换到不存在的项目：移除标签并抛错，当前项目不受影响', async () => {
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    await expect(store.switchProject('ghost')).rejects.toThrow('项目不存在')
    expect(store.project?.id).toBe('p-a')
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a'])
  })

  it('重启后从项目列表直进（不经过 init）：持久化的标签仍恢复', async () => {
    // 模拟上次会话打开过 A、B 两个标签
    localStorage.setItem('miyo.open-projects', JSON.stringify(['p-a', 'p-b']))
    const store = useWorkspaceStore()

    // 用户在项目列表页点 B 进入（store 全新，未调 init）
    await store.switchProject('p-b')
    expect(store.project?.id).toBe('p-b')
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a', 'p-b'])
  })

  it('新建空接口默认干净：无改动关闭不确认；改动后才变脏', async () => {
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    store.openNewEndpoint(null)
    const id = store.activeTabId!
    // 刚 + 出来、一个字没改：干净（无小圆点、无关闭确认）
    expect(store.isDirty(id)).toBe(false)
    // 改名 → 脏（定版推进是 watcher 套 nextTick 的两跳，这里等两拍）
    const blankName = store.draftOf(id)!.name
    store.draftOf(id)!.name = '新建用户'
    await nextTick()
    await nextTick()
    expect(store.isDirty(id)).toBe(true)
    // 改回原样 → 干净
    store.draftOf(id)!.name = blankName
    await nextTick()
    await nextTick()
    expect(store.isDirty(id)).toBe(false)
    // 改路径 → 脏；保存后回归干净（创建快照使命完成）
    store.draftOf(id)!.path = '/users'
    await nextTick()
    await nextTick()
    expect(store.isDirty(id)).toBe(true)
    store.draftOf(id)!.name = '新建用户'
    expect(await store.saveActiveDraft()).toBe(true)
    expect(store.isDirty(id)).toBe(false)
  })

  it('批量关闭：closeOthers 保留当前，closeAll 清空并回退激活', async () => {
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    store.openEndpoint(store.endpoints[0]!)
    store.openNewEndpoint(null)
    store.openNewEndpoint(null)
    expect(store.openTabs).toHaveLength(3)
    const keep = store.activeTabId!
    expect(store.closeOtherTabs(keep)).toBe(2)
    expect(store.openTabs).toEqual([keep])
    expect(store.activeTabId).toBe(keep)
    expect(store.closeAllTabs()).toBe(1)
    expect(store.openTabs).toHaveLength(0)
    expect(store.activeTabId).toBeNull()
  })
})

describe('备份恢复后刷新（回归：不重启也能看到新项目）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive(null)
    localStorage.removeItem?.('miyo.open-projects')
  })

  it('恢复出的全新项目追加为标签，当前项目保持激活并重载自身数据', async () => {
    const a = makeProject('p-a', '项目A')
    backend.projects.push(a)
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.envsByProject.set('p-a', [])

    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a'])

    // 模拟 backupRestore：新项目已落库（新 id），且当前项目新增了一个接口
    const restored = makeProject('p-restored', 'pay')
    backend.projects.push(restored)
    backend.endpointsByProject.set('p-a', [
      makeEndpoint('ep-1', 'p-a', 'users'),
      makeEndpoint('ep-2', 'p-a', 'orders'),
    ])

    await store.refreshAfterRestore()

    // 新项目进标签栏，当前项目未被切换
    expect(store.project?.id).toBe('p-a')
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a', 'p-restored'])
    // 当前项目数据重载（能看到恢复合并进来的新接口）
    expect(store.endpoints.map((e) => e.id)).toEqual(['ep-1', 'ep-2'])
  })

  it('恢复替换掉的旧项目从标签栏剔除', async () => {
    backend.projects.push(makeProject('p-a', '项目A'), makeProject('p-dead', '旧项目'))
    backend.endpointsByProject.set('p-a', [])
    backend.endpointsByProject.set('p-dead', [])
    backend.envsByProject.set('p-a', [])
    backend.envsByProject.set('p-dead', [])

    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    store.openProjects.push({ id: 'p-dead', name: '旧项目' })
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a', 'p-dead'])

    // 旧项目已不在后端项目集中（被恢复替换）
    backend.projects.splice(
      backend.projects.findIndex((p) => p.id === 'p-dead'),
      1,
    )
    await store.refreshAfterRestore()

    expect(store.project?.id).toBe('p-a')
    expect(store.openProjects.map((t) => t.id)).toEqual(['p-a'])
  })
})

describe('moveEndpoint：移动后打开草稿的 folder_id / sort_order 同步', () => {
  beforeEach(() => {
    // 文案经 i18n 取词，锁定中文（jsdom 默认语言为英文）

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.setActive('p-a')
    // 固定时钟：isDirty 依赖草稿/已存的全字段 eq 比较，时间戳漂移会误判「脏」
    const T = '2026-01-01T00:00:00.000Z'
    backend.endpointsByProject.set('p-a', [
      { ...makeEndpoint('ep-a', 'p-a', 'in-a', T), folder_id: 'f-a', sort_order: 0 },
      { ...makeEndpoint('ep-other', 'p-a', 'also-in-a', T), folder_id: 'f-a', sort_order: 1 },
      { ...makeEndpoint('ep-b', 'p-a', 'in-b', T), folder_id: 'f-b', sort_order: 0 },
    ])
  })

  it('跨文件夹移动后保存：folder_id 保持新归属，不退回旧文件夹（回归 bug#保存回退）', async () => {
    const store = useWorkspaceStore()
    await store.init()
    store.openEndpoint(store.endpoints.find((e) => e.id === 'ep-a')!)

    await store.moveEndpoint('ep-a', 'f-b', 0)

    // 打开草稿的 folder_id 随移动同步；保存写库不再用旧归属覆盖
    expect(store.draftOf('ep-a')?.folder_id).toBe('f-b')
    expect(await store.saveActiveDraft()).toBe(true)
    const saved = backend.endpointsByProject.get('p-a')!.find((e) => e.id === 'ep-a')!
    expect(saved.folder_id).toBe('f-b')
  })

  it('同组内移动：打开草稿的 sort_order 同步，不产生假「脏」', async () => {
    const store = useWorkspaceStore()
    await store.init()
    store.openEndpoint(store.endpoints.find((e) => e.id === 'ep-a')!)
    store.openEndpoint(store.endpoints.find((e) => e.id === 'ep-other')!)

    // 把 ep-a 移到组尾：ep-other 前移
    await store.moveEndpoint('ep-a', 'f-a', 1)
    // dirtyTick 由 nextTick 推进，等其落定后 isDirty 才是稳定值
    await nextTick()

    expect(store.draftOf('ep-a')?.sort_order).toBe(1)
    expect(store.draftOf('ep-other')?.sort_order).toBe(0)
    expect(store.isDirty('ep-a')).toBe(false)
    expect(store.isDirty('ep-other')).toBe(false)
  })

  it('编辑 body 内容并保存：草稿与保存态深克隆解耦，保存后内容稳定不丢失', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const ep = store.endpoints.find((e) => e.id === 'ep-a')!
    store.openEndpoint(ep)

    const draft = store.draftOf('ep-a')!
    draft.request.body = { mode: 'json', raw: '{"name":"miyo","added":"s"}' }

    expect(store.isDirty('ep-a')).toBe(true)
    expect(await store.saveActiveDraft()).toBe(true)

    // 保存后草稿保持最新内容
    expect(store.draftOf('ep-a')?.request.body).toEqual({
      mode: 'json',
      raw: '{"name":"miyo","added":"s"}',
    })
    expect(store.isDirty('ep-a')).toBe(false)
  })
})

describe('cURL 导入：环境前缀优先时不覆写环境，path 存完整 URL', () => {
  const parsed: CurlParsed = {
    url: 'https://httpbin.org/post?x=1',
    method: 'POST',
    headers: [{ key: 'Content-Type', value: 'application/json', enabled: true, description: '' }],
    body: null,
    auth: { type: 'none' },
  }

  beforeEach(async () => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.projects.push(makeProject('p-a', '项目A'))
    backend.endpointsByProject.set('p-a', [])
    backend.envsByProject.set('p-a', [])
    backend.setActive('p-a')
  })

  /** 注入「有 base_url 的激活环境」：urlDomain 应为 {{base_url}}。 */
  function activateEnvWithBase(store: ReturnType<typeof useWorkspaceStore>): Environment {
    const now = new Date().toISOString()
    const env: Environment = {
      id: 'env-1',
      project_id: 'p-a',
      name: '测试',
      base_url: 'https://env.example.com',
      variables: [],
      created_at: now,
      updated_at: now,
    }
    store.environments.push(env)
    store.activeEnvId = 'env-1'
    return env
  }

  it('激活环境导入 curl（弹窗路径 openCurlDraft）：path 为完整 URL，环境 base_url 与会话前缀都不写', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const env = activateEnvWithBase(store)
    expect(store.urlDomain).toBe('{{base_url}}')
    const sessionBefore = store.sessionBaseUrl

    store.openCurlDraft(parsed, null)
    const id = store.activeTabId!

    // 地址栏应直达 httpbin（完整 URL 存 path），而非显示环境前缀 + /post
    expect(store.draftOf(id)!.path).toBe('https://httpbin.org/post')
    expect(store.draftOf(id)!.request.params).toEqual([
      { key: 'x', value: '1', enabled: true, description: '' },
    ])
    // 共享环境不被导入污染；展示前缀仍是环境变量
    expect(env.base_url).toBe('https://env.example.com')
    expect(store.urlDomain).toBe('{{base_url}}')
    // 会话前缀同样不写：被环境前缀遮蔽的写入对用户不可见，事后切「无环境」会冒出
    // 从没设过的域名（实测困惑），故只在展示前缀就是会话前缀时才预填
    expect(store.sessionBaseUrl).toBe(sessionBefore)
  })

  it('无环境 base 导入 curl：path 相对，会话 Base URL 预填 origin', async () => {
    const store = useWorkspaceStore()
    await store.init()

    store.openCurlDraft(parsed, null)
    const id = store.activeTabId!

    expect(store.draftOf(id)!.path).toBe('/post')
    expect(store.sessionBaseUrl).toBe('https://httpbin.org')
    expect(store.urlDomain).toBe('https://httpbin.org')
  })

  it('地址栏粘贴回填已有草稿（环境激活）：同规则，path 为完整 URL、环境与会话前缀都不写', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const env = activateEnvWithBase(store)
    store.openNewEndpoint(null)
    const id = store.activeTabId!
    const sessionBefore = store.sessionBaseUrl

    store.applyCurlToDraft(id, parsed)

    expect(store.draftOf(id)!.path).toBe('https://httpbin.org/post')
    expect(store.draftOf(id)!.method).toBe('POST')
    expect(env.base_url).toBe('https://env.example.com')
    expect(store.sessionBaseUrl).toBe(sessionBefore)
  })
})

describe('无环境前缀按项目持久化（回归：重启丢自定义域名回 localhost）', () => {
  const parsed: CurlParsed = {
    url: 'https://api.example.com/v1/users',
    method: 'GET',
    headers: [],
    body: null,
    auth: { type: 'none' },
  }

  beforeEach(async () => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.projects.push(makeProject('p-prefix', '前缀项目'))
    backend.endpointsByProject.set('p-prefix', [])
    backend.envsByProject.set('p-prefix', [])
    backend.setActive('p-prefix')
    // 隔离：内存 localStorage 跨用例共享，清掉本项目可能残留的前缀记录
    localStorage.removeItem('miyo.base-url.p-prefix')
  })

  it('导入 cURL 后整个 app 重启（新 store 重新 init）：自定义前缀恢复，不回 localhost', async () => {
    const store = useWorkspaceStore()
    await store.init()
    expect(store.sessionBaseUrl).toBe('http://localhost')

    store.openCurlDraft(parsed, null)
    expect(store.sessionBaseUrl).toBe('https://api.example.com')

    // 模拟重启：换新 pinia（新 store 实例）重新 init——前缀必须仍在。
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    const reopened = useWorkspaceStore()
    await reopened.init()
    expect(reopened.sessionBaseUrl).toBe('https://api.example.com')
    expect(reopened.urlDomain).toBe('https://api.example.com')

    localStorage.removeItem('miyo.base-url.p-prefix')
  })

  it('从未导入过的项目：回退默认 localhost', async () => {
    const store = useWorkspaceStore()
    await store.init()
    expect(store.sessionBaseUrl).toBe('http://localhost')
  })
})

describe('请求历史：增量分页与失败提示', () => {
  beforeEach(() => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive(null)
    backend.projects.push(makeProject('p-a', '项目A'), makeProject('p-b', '项目B'))
    backend.endpointsByProject.set('p-a', [])
    backend.endpointsByProject.set('p-b', [])
    backend.envsByProject.set('p-a', [])
    backend.envsByProject.set('p-b', [])
    backend.historyRows.length = 0
    backend.historyCalls.length = 0
    backend.setHistoryImpl(null)
    toastMock.error.mockClear()
    toastMock.success.mockClear()
  })

  function seedHistory(n: number): void {
    backend.historyRows.length = 0
    for (let i = 0; i < n; i++) backend.historyRows.push({ id: `h-${i}` })
  }

  it('首屏 50，加载更多窗口 +50；返回不足一屏即到底', async () => {
    seedHistory(120)
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()

    await store.loadHistories()
    expect(backend.historyCalls.at(-1)).toBe(50)
    expect(store.histories).toHaveLength(50)
    expect(store.historyHasMore).toBe(true)

    await store.loadMoreHistories()
    expect(backend.historyCalls.at(-1)).toBe(100)
    expect(store.histories).toHaveLength(100)
    expect(store.historyHasMore).toBe(true)

    // 后端无 offset：窗口 150 但只剩 120 条 → 不足一屏，判定到底
    await store.loadMoreHistories()
    expect(backend.historyCalls.at(-1)).toBe(150)
    expect(store.histories).toHaveLength(120)
    expect(store.historyHasMore).toBe(false)
  })

  it('切换项目重置分页窗口（新项目从首屏 50 条重新拉）', async () => {
    seedHistory(120)
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    await store.loadHistories()
    await store.loadMoreHistories()
    expect(backend.historyCalls.at(-1)).toBe(100)

    await store.switchProject('p-b')
    await store.loadHistories()
    expect(backend.historyCalls.at(-1)).toBe(50)
    expect(store.historyHasMore).toBe(true)
  })

  it('加载失败只提示一次；成功加载后再次失败重新提示', async () => {
    seedHistory(10)
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()

    backend.setHistoryImpl(() => Promise.reject(new Error('backend down')))
    await store.loadHistories()
    await store.loadHistories()
    expect(toastMock.error).toHaveBeenCalledTimes(1)
    expect(toastMock.error.mock.calls[0]?.[0]).toBe('加载请求历史失败')

    backend.setHistoryImpl(null)
    await store.loadHistories()
    expect(toastMock.error).toHaveBeenCalledTimes(1)

    backend.setHistoryImpl(() => Promise.reject(new Error('down again')))
    await store.loadHistories()
    expect(toastMock.error).toHaveBeenCalledTimes(2)
  })
})

describe('全局请求超时：未配置时回退后端内置默认', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('getHttpTimeoutMs 返回 null（从未配置）时，globalTimeoutMs 取内置默认 300s', async () => {
    const store = useWorkspaceStore()
    await vi.waitFor(() => {
      expect(store.globalTimeoutMs).toBe(FALLBACK_GLOBAL_TIMEOUT_MS)
    })
  })

  /**
   * 回归：send() 的超时解析必须与后端 resolve_timeout_ms 一致（接口级 → 全局）。
   *
   * 旧 bug：`const ms = timeoutMs ?? endpoint.request.timeout_ms ?? null`，从不看
   * 全局设置。后果一：接口留空（跟随全局）时前端不设 timer；后果二：接口残留旧
   * 默认 30000 时按 30s 提前取消，与设置页展示的 300s 不符（用户实测）。
   */
  it('接口未设超时：前端按全局值取消（悬挂请求触发 cancelRequest）', async () => {
    vi.useFakeTimers()
    try {
      const store = useWorkspaceStore()
      await vi.waitFor(() => expect(store.globalTimeoutMs).toBe(FALLBACK_GLOBAL_TIMEOUT_MS))

      // 悬挂请求，直到 cancelRequest 才 reject（模拟弱网挂起）
      backend.cancelCalls.length = 0
      backend.setExecuteImpl(
        (args) =>
          new Promise((_res, rej) => {
            backend.setCancelImpl(() => {
              const e = new Error('cancelled') as Error & { code?: string }
              e.code = 'CANCELLED'
              rej(e)
            })
            void args
          }),
      )
      const ep = makeEndpoint('ep-global', 'p-a', 'global')
      // makeEndpoint 夹具默认 30000（旧夹具），这里显式置空以走全局兜底
      ep.request.timeout_ms = null
      const p = store.send(ep, 'http://x/global')
      p.catch(() => {})

      // 未到全局值：不取消
      await vi.advanceTimersByTimeAsync(FALLBACK_GLOBAL_TIMEOUT_MS - 1)
      expect(backend.cancelCalls).toHaveLength(0)

      // 到达全局值：取消
      await vi.advanceTimersByTimeAsync(1)
      expect(backend.cancelCalls).toHaveLength(1)
      await expect(p).rejects.toThrow(String(FALLBACK_GLOBAL_TIMEOUT_MS))
    } finally {
      backend.setExecuteImpl(null)
      backend.setCancelImpl(null)
      vi.useRealTimers()
    }
  })

  it('接口级遗留 30000：仍按 30s 取消（显式覆盖优先，与后端一致）', async () => {
    vi.useFakeTimers()
    try {
      const store = useWorkspaceStore()
      await vi.waitFor(() => expect(store.globalTimeoutMs).toBe(FALLBACK_GLOBAL_TIMEOUT_MS))

      backend.cancelCalls.length = 0
      backend.setExecuteImpl(
        () =>
          new Promise((_res, rej) => {
            backend.setCancelImpl(() => {
              const e = new Error('cancelled') as Error & { code?: string }
              e.code = 'CANCELLED'
              rej(e)
            })
          }),
      )
      const ep = makeEndpoint('ep-30k', 'p-a', 'legacy')
      ep.request.timeout_ms = 30000
      const p = store.send(ep, 'http://x/legacy')
      p.catch(() => {})

      await vi.advanceTimersByTimeAsync(30_000)
      expect(backend.cancelCalls).toHaveLength(1)
      await expect(p).rejects.toThrow('30000')
    } finally {
      backend.setExecuteImpl(null)
      backend.setCancelImpl(null)
      vi.useRealTimers()
    }
  })
})

/**
 * 用例运行代入路径变量（回归）。
 *
 * 旧 bug：`TestCase` 没有 `path_variables` 字段，`buildCaseRequest` 写死
 * `path_variables: []` 且不代入 URL，于是 `/users/{id}` 这种带路径传参的用例
 * 单跑时把字面量 `{id}` 原样发给后端。此处断言真正送到 executeRequest 的 URL。
 */
describe('用例运行：URL 路径变量代入', () => {
  const baseCase = (over: Partial<TestCase>): TestCase => ({
    id: 'c1',
    request_id: 'ep-1',
    name: '用例',
    category: '正向',
    method: 'GET',
    url_path: '/users/{id}',
    path_variables: [],
    params: [],
    headers: [],
    body_type: 'none',
    body_content: '',
    last_run_status: 'Untested',
    created_at: '2026-01-01T00:00:00.000Z',
    ...over,
  })

  beforeEach(() => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.execCalls.length = 0
    backend.setActive(null)
    backend.projects.push(makeProject('p-a', '项目A'))
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.envsByProject.set('p-a', [])
  })

  it('已填值的路径变量在发出前代入 URL', async () => {
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    await store.runTestCase(
      'ep-1',
      baseCase({ path_variables: [{ key: 'id', value: '42', enabled: true, description: '' }] }),
      null,
    )
    expect(backend.execCalls.at(-1)?.url).toContain('/users/42')
    expect(backend.execCalls.at(-1)?.url).not.toContain('{id}')
  })

  it('未填值时路径原样发出（保持历史行为）', async () => {
    const store = useWorkspaceStore()
    backend.setActive('p-a')
    await store.init()
    await store.runTestCase('ep-1', baseCase({ path_variables: [] }), null)
    expect(backend.execCalls.at(-1)?.url).toContain('/users/{id}')
  })
})

describe('「新建」伪页签：+ 每次新开、选择不消耗、快照过滤', () => {
  beforeEach(() => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive(null)
    const a = makeProject('p-a', '项目A')
    const b = makeProject('p-b', '项目B')
    backend.projects.push(a, b)
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.endpointsByProject.set('p-b', [makeEndpoint('ep-2', 'p-b', 'orders')])
    backend.envsByProject.set('p-a', [])
    backend.envsByProject.set('p-b', [])
  })

  it('openCreateTab：每次 + 新开一枚伪页签并激活（回归：单例浮层时再点 + 是死按钮）', () => {
    const store = useWorkspaceStore()

    store.openCreateTab()
    const first = store.activeTabId!
    expect(store.isCreateTab(first)).toBe(true)
    expect(store.openTabs).toEqual([first])

    // 未选择再点 +：再开一枚，而不是毫无反应
    store.openCreateTab()
    expect(store.openTabs).toHaveLength(2)
    expect(store.activeTabId).not.toBe(first)

    // 伪页签：无草稿、不判脏、标题「新建…」
    expect(store.draftOf(store.activeTabId!)).toBeNull()
    expect(store.isDirty(store.activeTabId!)).toBe(false)
    expect(store.titleOf(store.activeTabId!)).toBe('新建…')
  })

  it('选择「新建 HTTP 请求」：伪页签原地替换为新接口页签（不是旁边多一枚）', async () => {
    backend.setActive('p-a')
    const store = useWorkspaceStore()
    await store.init()
    store.openEndpoint(store.endpoints[0]!)

    store.openCreateTab()
    const createId = store.activeTabId!
    expect(store.openTabs).toEqual(['ep-1', createId])

    // 页面上点「新建 HTTP 请求」→ 当前「新建」页签被替换成新接口页签
    store.openNewEndpointFromCreateTab()
    expect(store.openTabs).toHaveLength(2)
    expect(store.openTabs).not.toContain(createId)
    expect(store.isCreateTab(store.activeTabId!)).toBe(false)
    expect(store.activeTabId).not.toBe('ep-1')
    // 原地替换：新接口页签占据原「新建」页签的位置（栏位顺序不变）
    expect(store.openTabs).toEqual(['ep-1', store.activeTabId])
    expect(store.titleOf(store.activeTabId!)).toBe('未命名接口')
  })

  it('closeTab 关闭「新建」页签：激活态回退相邻页签', async () => {
    backend.setActive('p-a')
    const store = useWorkspaceStore()
    await store.init()
    store.openEndpoint(store.endpoints[0]!)
    expect(store.activeTabId).toBe('ep-1')

    store.openCreateTab()
    const createId = store.activeTabId!
    store.closeTab(createId)
    expect(store.openTabs).toEqual(['ep-1'])
    expect(store.activeTabId).toBe('ep-1')
  })

  it('快照过滤：切项目再切回，「新建」页签不复活、激活态不指向伪页签', async () => {
    backend.setActive('p-a')
    const store = useWorkspaceStore()
    await store.init()

    store.openCreateTab()
    await store.switchProject('p-b')
    expect(store.openTabs.every((id) => !store.isCreateTab(id))).toBe(true)

    await store.switchProject('p-a')
    expect(store.openTabs.every((id) => !store.isCreateTab(id))).toBe(true)
    expect(store.activeTabId === null || !store.isCreateTab(store.activeTabId)).toBe(true)
  })

  it('刷新校验豁免：endpoints 列表刷新不清掉「新建」页签', async () => {
    backend.setActive('p-a')
    const store = useWorkspaceStore()
    await store.init()

    store.openCreateTab()
    await store.refresh()
    expect(store.openTabs.some((id) => store.isCreateTab(id))).toBe(true)
  })
})

describe('页签与草稿跨实例持久化（localStorage）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())

    // 页签/草稿持久化是全局副作用：每例清空，避免跨用例串现场
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive('p-a')
    backend.projects.push(makeProject('p-a', '项目A'))
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.envsByProject.set('p-a', [])
    // 隔离其他用例残留的持久化键（stub 的 localStorage 无枚举能力，按已知键清理）
    localStorage.removeItem('miyo.open-tabs.p-a')
    localStorage.removeItem('miyo.open-projects')
    localStorage.removeItem('miyo.recent-endpoints')
  })

  it('重启（新 store 实例）后恢复页签顺序、草稿内容与激活态', async () => {
    const s1 = useWorkspaceStore()
    await s1.init()
    s1.openEndpoint(backend.endpointsByProject.get('p-a')![0]!)
    const draft = s1.draftOf('ep-1')!
    draft.name = '改名草稿'
    // 草稿编辑走 600ms 防抖落盘
    await new Promise((r) => setTimeout(r, 700))

    // 全新实例 = 模拟应用重启
    setActivePinia(createPinia())
    const s2 = useWorkspaceStore()
    await s2.init()
    expect(s2.openTabs).toEqual(['ep-1'])
    expect(s2.activeTabId).toBe('ep-1')
    expect(s2.draftOf('ep-1')?.name).toBe('改名草稿')
    // 恢复的草稿与保存态不同 → 脏标记成立（未保存改动不被静默洗白）
    expect(s2.isDirty('ep-1')).toBe(true)
  })

  it('最近打开记录随 openEndpoint 写入并去重置顶', async () => {
    backend.endpointsByProject.set('p-a', [
      makeEndpoint('ep-1', 'p-a', 'users'),
      makeEndpoint('ep-2', 'p-a', 'orders'),
    ])
    const s1 = useWorkspaceStore()
    await s1.init()
    s1.openEndpoint(backend.endpointsByProject.get('p-a')![0]!)
    s1.openEndpoint(backend.endpointsByProject.get('p-a')![1]!)
    s1.openEndpoint(backend.endpointsByProject.get('p-a')![0]!)
    expect(s1.recentEndpointIds).toEqual(['ep-1', 'ep-2'])
  })
})

describe('导入冲突策略（method + path 三选）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive('p-a')
    backend.projects.push(makeProject('p-a', '项目A'))
    backend.endpointsByProject.set('p-a', [makeEndpoint('ep-1', 'p-a', 'users')])
    backend.envsByProject.set('p-a', [])
  })

  const items = () => [
    { name: 'users-导入版', method: 'GET', path: '/users', request: makeEndpoint('x', 'p-a', 'x').request },
    {
      name: 'orders',
      method: 'POST',
      path: '/orders',
      request: makeEndpoint('x', 'p-a', 'x').request,
      examples: [{ name: 'ex1', status: 200, content_type: 'application/json', headers: {}, body: '{}' }],
    },
  ]

  it('skip：冲突项跳过，非冲突项照常导入', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const summary = await store.importEndpoints(items(), 'skip')
    expect(summary.skipped).toBe(1)
    expect(summary.endpoints).toBe(1)
    // 冲突接口保持原样（名称不变）
    expect(backend.endpointsByProject.get('p-a')!.some((e) => e.name === 'users-导入版')).toBe(false)
  })

  it('overwrite：沿用现有 id / 状态标记，覆盖名称与请求配置，不导入示例', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const before = backend.endpointsByProject.get('p-a')!.find((e) => e.id === 'ep-1')!
    const summary = await store.importEndpoints(items(), 'overwrite')
    expect(summary.overwritten).toBe(1)
    // endpoints = 写入总数（新建 1 + 覆盖 1），与导入完成 toast 口径一致
    expect(summary.endpoints).toBe(2)
    const after = backend.endpointsByProject.get('p-a')!.find((e) => e.id === 'ep-1')!
    expect(after.id).toBe(before.id)
    expect(after.name).toBe('users-导入版')
    expect(after.created_at).toBe(before.created_at)
    // POST /orders 是新接口：新建路径 + 示例照常导入
    expect(backend.endpointsByProject.get('p-a')!.some((e) => e.method === 'POST' && e.path === '/orders')).toBe(true)
  })

  it('duplicate（默认）：一律新建，冲突项也产生新接口', async () => {
    const store = useWorkspaceStore()
    await store.init()
    const summary = await store.importEndpoints(items(), 'duplicate')
    expect(summary.skipped).toBe(0)
    expect(summary.overwritten).toBe(0)
    expect(summary.endpoints).toBe(2)
    const users = backend.endpointsByProject.get('p-a')!.filter((e) => e.method === 'GET' && e.path === '/users')
    expect(users).toHaveLength(2)
  })
})

describe('页签拖拽排序回写（reorderTabs）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    useLocaleStore().setMode('zh')
    backend.projects.length = 0
    backend.endpointsByProject.clear()
    backend.envsByProject.clear()
    backend.setActive('p-a')
    backend.projects.push(makeProject('p-a', '项目A'))
    backend.endpointsByProject.set('p-a', [
      makeEndpoint('ep-1', 'p-a', 'users'),
      makeEndpoint('ep-2', 'p-a', 'orders'),
      makeEndpoint('ep-3', 'p-a', 'billing'),
    ])
    backend.envsByProject.set('p-a', [])
  })

  it('接受当前成员的完整排列；拒绝缺项 / 重复 / 未知 id', async () => {
    const store = useWorkspaceStore()
    await store.init()
    store.openEndpoint(backend.endpointsByProject.get('p-a')![0]!)
    store.openEndpoint(backend.endpointsByProject.get('p-a')![1]!)
    store.openEndpoint(backend.endpointsByProject.get('p-a')![2]!)
    store.reorderTabs(['ep-3', 'ep-1', 'ep-2'])
    expect(store.openTabs).toEqual(['ep-3', 'ep-1', 'ep-2'])
    // 缺项：不生效
    store.reorderTabs(['ep-1', 'ep-2'])
    expect(store.openTabs).toEqual(['ep-3', 'ep-1', 'ep-2'])
    // 未知 id：不生效
    store.reorderTabs(['ep-1', 'ep-2', 'ep-9'])
    expect(store.openTabs).toEqual(['ep-3', 'ep-1', 'ep-2'])
    // 长度对但重复：不生效
    store.reorderTabs(['ep-1', 'ep-1', 'ep-2'])
    expect(store.openTabs).toEqual(['ep-3', 'ep-1', 'ep-2'])
  })
})
