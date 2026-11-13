/**
 * EnvironmentManager × workspace store 全链路保存测试（真实 store，IPC 用内存假后端）。
 *
 * 回归用户报告的 bug：环境编辑中输入 Base URL（如 http://localhost:8093）点保存
 * 「没有效果」。链路：组件 save() → store.updateEnvironment → api.saveEnvironment
 * （假后端按 save_environment 命令同款规则校验 project 归属并落内存库）→ 重开弹窗回显。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import EnvironmentManager from '@/domains/environment/components/EnvironmentManager.vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import type { Environment, Project } from '@/types/miyoApi'

const toastMock = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
  warning: vi.fn(),
}))

/** 内存假后端：saveEnvironment 记录调用并按 save_environment 同款校验落库。 */
const backend = vi.hoisted(() => {
  const projects: Project[] = []
  const envsByProject = new Map<string, Environment[]>()
  let activeProjectId: string | null = null
  const savedPayloads: Environment[] = []
  /** 模拟旧版本后端：不认识 base_url 字段（serde 忽略未知字段），回显时静默丢弃。 */
  let echoDropsBaseUrl = false
  return {
    projects,
    envsByProject,
    savedPayloads,
    setActive: (id: string) => void (activeProjectId = id),
    active: () => activeProjectId,
    setEchoDropsBaseUrl: (v: boolean) => void (echoDropsBaseUrl = v),
    echoDropsBaseUrl: () => echoDropsBaseUrl,
  }
})

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => toastMock,
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    getProjects: vi.fn().mockResolvedValue(backend.projects),
    getActiveProject: vi.fn().mockImplementation(async () => backend.projects.find((p) => p.id === backend.active()) ?? null),
    setActiveProject: vi.fn(),
    listFolders: vi.fn().mockResolvedValue([]),
    listEndpoints: vi.fn().mockResolvedValue([]),
    listEnvironments: vi.fn().mockImplementation(async (pid: string) =>
      structuredClone(backend.envsByProject.get(pid) ?? []),
    ),
    saveEnvironment: vi.fn().mockImplementation(async (env: Environment) => {
      backend.savedPayloads.push(structuredClone(env))
      // save_environment 命令同款校验：所属项目必须存在（不要求是激活项目，可跨项目管理）
      if (!backend.projects.some((p) => p.id === env.project_id)) {
        throw Object.assign(new Error('环境所属项目不存在'), { code: 'VALIDATION' })
      }
      const list = backend.envsByProject.get(env.project_id) ?? []
      const idx = list.findIndex((e) => e.id === env.id)
      if (idx === -1) list.push(structuredClone(env))
      else list[idx] = structuredClone(env)
      if (backend.echoDropsBaseUrl()) {
        // 旧后端：响应里没有 base_url 字段（模拟 serde 忽略未知字段）
        const echo = structuredClone(env)
        delete (echo as Partial<Environment>).base_url
        return echo
      }
      return structuredClone(env)
    }),
    getActiveEnvironment: vi.fn().mockResolvedValue(null),
    setActiveEnvironment: vi.fn().mockResolvedValue(undefined),
    getGlobalVariables: vi.fn().mockResolvedValue([]),
    saveGlobalVariables: vi.fn().mockResolvedValue(undefined),
    getGlobalParams: vi.fn().mockResolvedValue([]),
    saveGlobalParams: vi.fn().mockResolvedValue(undefined),
    readTextFile: vi.fn(),
    exportEnvironment: vi.fn(),
    importEnvironment: vi.fn(),
  }),
}))

function makeProject(id: string, name: string): Project {
  const now = new Date().toISOString()
  return { id, name, description: '', variables: {}, created_at: now, updated_at: now }
}

function makeEnv(id: string, projectId: string, name: string): Environment {
  return {
    id,
    project_id: projectId,
    name,
    base_url: '',
    variables: [],
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  }
}

async function mountDlg() {
  const wrapper = mount(EnvironmentManager, { props: { open: false }, attachTo: document.body })
  await wrapper.setProps({ open: true })
  await nextTick()
  return wrapper
}

beforeEach(async () => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  backend.projects.length = 0
  backend.envsByProject.clear()
  backend.savedPayloads.length = 0
  backend.projects.push(makeProject('p1', '演示项目'))
  backend.envsByProject.set('p1', [makeEnv('e-dev', 'p1', '开发环境')])
  backend.setActive('p1')
  toastMock.success.mockClear()
  toastMock.error.mockClear()
  toastMock.warning.mockClear()
  // 真实 store 初始化：加载当前项目的环境列表
  const store = useWorkspaceStore()
  await store.init()
  // 测试环境无完整 localStorage，stub 内存版（store 启动恢复逻辑需要）
  const mem = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
  })
})

describe('环境 Base URL 保存全链路', () => {
  /** 测试间卸载残留实例：弹窗 Teleport 到 body，残留实例的全局查询会串测。 */
  const mounted: { unmount: () => void }[] = []
  afterEach(() => {
    while (mounted.length) mounted.pop()?.unmount()
    document.body.innerHTML = ''
  })

  it('输入 Base URL 点击保存：落库并回显，重开弹窗不丢', async () => {
    const store = useWorkspaceStore()
    expect(store.environments).toHaveLength(1)
    expect(store.environments[0]!.base_url).toBe('')

    const wrapper = await mountDlg()
    mounted.push(wrapper)

    // 编辑 Base URL（模拟真实输入事件）
    const input = document.querySelector<HTMLInputElement>('.em-base-input')!
    expect(input).toBeTruthy()
    input.value = 'http://localhost:8093'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()

    // 点击底部「保存」
    const save = document.querySelector<HTMLButtonElement>('.em-savebar .miyo-btn-primary')!
    expect(save.disabled).toBe(false)
    save.click()
    await flushPromises()

    // IPC 收到的 payload 带上了 Base URL
    expect(backend.savedPayloads).toHaveLength(1)
    expect(backend.savedPayloads[0]!.base_url).toBe('http://localhost:8093')
    expect(backend.savedPayloads[0]!.project_id).toBe('p1')
    // store 列表已同步
    expect(store.environments[0]!.base_url).toBe('http://localhost:8093')
    // 成功提示（带环境名）恰好一条
    expect(toastMock.success).toHaveBeenCalledTimes(1)
    expect(toastMock.error).not.toHaveBeenCalled()

    // 关闭再重开：Base URL 回显不丢
    await wrapper.setProps({ open: false })
    await nextTick()
    await wrapper.setProps({ open: true })
    await nextTick()
    const reopened = document.querySelector<HTMLInputElement>('.em-base-input')!
    expect(reopened.value).toBe('http://localhost:8093')
  })

  it('Base URL 带尾部斜杠：保存时规范化去掉', async () => {
    await mountDlg()
    const input = document.querySelector<HTMLInputElement>('.em-base-input')!
    input.value = 'http://localhost:8093/'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()

    document.querySelector<HTMLButtonElement>('.em-savebar .miyo-btn-primary')!.click()
    await flushPromises()

    expect(backend.savedPayloads[0]!.base_url).toBe('http://localhost:8093')
  })

  it('旧版本后端回显丢失 base_url：报错而非静默假成功，列表不同步', async () => {
    backend.setEchoDropsBaseUrl(true)
    const store = useWorkspaceStore()
    const wrapper = await mountDlg()
    mounted.push(wrapper)

    const input = document.querySelector<HTMLInputElement>('.em-base-input')!
    input.value = 'http://localhost:8093'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()

    document.querySelector<HTMLButtonElement>('.em-savebar .miyo-btn-primary')!.click()
    await flushPromises()

    // 保存按失败处理：错误提示 + 无成功提示 + 本地列表不写入假值
    expect(toastMock.error).toHaveBeenCalledTimes(1)
    expect(toastMock.success).not.toHaveBeenCalled()
    expect(store.environments[0]!.base_url).toBe('')
    // 弹窗仍为脏态：关闭会触发未保存确认（修改还在，不会被假成功清掉）
    const close = document.querySelector<HTMLButtonElement>('.m-head button.ib')!
    close.click()
    await nextTick()
    expect(document.querySelector('.em-confirm')).toBeTruthy()
    expect(wrapper.emitted('update:open')).toBeUndefined()
  })

  it('独立模式（projectId 指向其他项目）：加载该项目环境，保存不回写工作区列表', async () => {
    backend.projects.push(makeProject('p2', '另一个项目'))
    backend.envsByProject.set('p2', [makeEnv('e-p2', 'p2', 'P2环境')])
    const store = useWorkspaceStore()

    // 不走 mountDlg（其无 props）：带 projectId/projectName 挂载
    const wrapper = mount(EnvironmentManager, {
      props: { open: false, projectId: 'p2', projectName: '另一个项目' },
      attachTo: document.body,
    })
    mounted.push(wrapper)
    await wrapper.setProps({ open: true })
    await flushPromises()
    await nextTick()

    // 标题带项目名（取最后挂载的实例，防残留实例干扰），列表是 p2 的环境
    const titles = document.querySelectorAll('.m-title')
    expect(titles[titles.length - 1]?.textContent).toContain('另一个项目')
    expect(document.querySelector('.em-row-name')?.textContent).toBe('P2环境')

    const input = document.querySelector<HTMLInputElement>('.em-base-input')!
    input.value = 'http://p2.example.com'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    document.querySelector<HTMLButtonElement>('.em-savebar .miyo-btn-primary')!.click()
    await flushPromises()

    // 保存归属 p2 并落库
    expect(backend.savedPayloads[0]!.project_id).toBe('p2')
    expect(backend.envsByProject.get('p2')![0]!.base_url).toBe('http://p2.example.com')
    // 工作区（p1）列表不受影响
    expect(store.environments.some((e) => e.id === 'e-p2')).toBe(false)
    expect(store.environments.every((e) => e.project_id === 'p1')).toBe(true)
  })
})
