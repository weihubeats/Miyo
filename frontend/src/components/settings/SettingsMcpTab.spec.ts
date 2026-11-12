/**
 * SettingsMcpTab 冒烟：MCP 页签从 SettingsDialog 拆出后的行为锁定。
 * 覆盖：read/write 分组渲染（工具名走 i18n 键）+ 工具开关切换调用 IPC
 * 并本地翻转 + 保存失败不翻转。
 */
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import SettingsMcpTab from './SettingsMcpTab.vue'
import { useLocaleStore } from '../../stores/locale'
import type { McpPermissionStatus, McpToolStatus } from '../../types/miyoApi'

const perms: McpPermissionStatus[] = [
  { level: 'read', label: '读取数据', description: 'r', toolCount: 17, highRisk: false, enabled: true },
  { level: 'write', label: '创建与修改数据', description: 'w', toolCount: 10, highRisk: false, enabled: true },
  { level: 'execute', label: '执行测试用例与场景', description: 'e', toolCount: 3, highRisk: false, enabled: true },
  { level: 'network_execute', label: '发起真实网络请求', description: 'n', toolCount: 1, highRisk: true, enabled: false },
  { level: 'destructive', label: '删除与清理（不可恢复）', description: 'd', toolCount: 2, highRisk: true, enabled: false },
]

const apiMock = vi.hoisted(() => ({
  getMcpEnabled: vi.fn(async () => true),
  getMcpPort: vi.fn(async () => 4110),
  agentStatus: vi.fn(async () => ({ running: true, address: 'http://127.0.0.1:4110' })),
  getMcpTools: vi.fn(async () => tools),
  setMcpToolEnabled: vi.fn(async () => undefined),
  getMcpPermissions: vi.fn(async () => perms),
  setMcpPermissions: vi.fn(async () => undefined),
}))

vi.mock('../../composables/useMiyoApi', () => ({ useMiyoApi: () => apiMock }))

const tools: McpToolStatus[] = [
  { name: 'list_projects', kind: 'read' as const, permission: ('read' as const), module: 'system', title: 'list_projects', enabled: true },
  { name: 'get_run_result', kind: 'read' as const, permission: ('read' as const), module: 'system', title: 'get_run_result', enabled: true },
  { name: 'run_scenario', kind: 'write' as const, permission: ('write' as const), module: 'system', title: 'run_scenario', enabled: false },
  { name: 'cancel_run', kind: 'write' as const, permission: ('write' as const), module: 'system', title: 'cancel_run', enabled: true },
]

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  apiMock.setMcpToolEnabled.mockClear()
  apiMock.setMcpPermissions.mockClear()
  apiMock.getMcpPermissions.mockClear()
  apiMock.getMcpPermissions.mockImplementation(async () => perms.map((x) => ({ ...x })))
})

/** 统一挂载：Modal stub 成内联渲染（真实组件走 Teleport，测试树外不可见）。 */
function mountTab() {
  return mount(SettingsMcpTab, {
    global: {
      stubs: {
        Modal: { template: '<div data-test="modal"><slot /><slot name="footer" /></div>' },
        Icon: true,
        CustomNumberInput: true,
      },
    },
  })
}

describe('SettingsMcpTab', () => {
  it('挂载即拉取运行状态并显示运行中地址', async () => {
    const wrapper = mountTab()
    await flushPromises()
    expect(apiMock.agentStatus).toHaveBeenCalled()
    expect(wrapper.text()).toContain('运行中')
    expect(wrapper.text()).toContain('http://127.0.0.1:4110')
    wrapper.unmount()
  })

  it('工具清单 read/write 分组渲染 + 开关切换调用 IPC 并本地翻转', async () => {
    const wrapper = mountTab()
    await flushPromises()
    // i18n 工具名 + 原始工具名都渲染
    expect(wrapper.text()).toContain('list_projects')
    expect(wrapper.text()).toContain('run_scenario')
    expect(apiMock.getMcpTools).toHaveBeenCalled()

    // 工具开关的 aria-label 是 i18n 工具名（与权限等级开关不冲突）。
    const runLabel = useLocaleStore().t('settings.mcpTool.run_scenario')
    const runSwitch = wrapper
      .findAll('[role="switch"][aria-label]')
      .find((sw) => sw.attributes('aria-label') === runLabel)
    expect(runSwitch).toBeTruthy()
    await runSwitch!.trigger('click')
    await flushPromises()
    expect(apiMock.setMcpToolEnabled).toHaveBeenCalledWith({ name: 'run_scenario', enabled: true })
    wrapper.unmount()
  })

  it('工具开关保存失败：不翻转本地状态', async () => {
    apiMock.setMcpToolEnabled.mockRejectedValueOnce(new Error('boom'))
    const wrapper = mountTab()
    await flushPromises()
    const cancelLabel = useLocaleStore().t('settings.mcpTool.cancel_run')
    const cancelSwitch = wrapper
      .findAll('[role="switch"][aria-label]')
      .find((sw) => sw.attributes('aria-label') === cancelLabel) // 原 enabled=true
    expect(cancelSwitch).toBeTruthy()
    await cancelSwitch!.trigger('click')
    await flushPromises()
    // 失败后本地仍为 true（aria-checked 未变）
    expect(cancelSwitch!.attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })

  // ---------- P0 权限等级 UI ----------

  it('权限矩阵渲染五个等级，高风险标记可见', async () => {
    const wrapper = mountTab()
    await flushPromises()
    expect(apiMock.getMcpPermissions).toHaveBeenCalled()
    expect(wrapper.text()).toContain('发起真实网络请求')
    expect(wrapper.text()).toContain('高风险')
    wrapper.unmount()
  })

  it('启用高风险等级：先弹确认，确认后带 acknowledgeHighRisk 调 IPC', async () => {
    const wrapper = mountTab()
    await flushPromises()
    // network_execute 当前 disabled → 点击 = 启用 → 必须先弹确认，不直接调 IPC。
    const switches = wrapper.findAll('[role="switch"][aria-label]')
    // 权限开关的 aria-label 是等级说明（等级名），工具开关的 aria-label 是工具名。
    const permSwitch = switches.find((sw) => sw.attributes('aria-label') === '发起真实网络请求')!
    expect(permSwitch).toBeTruthy()
    await permSwitch.trigger('click')
    await flushPromises()
    expect(apiMock.setMcpPermissions).not.toHaveBeenCalled()
    // 确认对话框出现（Modal stub 不渲染 title，断言正文与确认按钮）
    expect(wrapper.text()).toContain('即将启用「发起真实网络请求」权限')
    // 点击确认按钮
    const confirmBtn = wrapper.findAll('button').find((b) => b.text() === '确认启用')!
    await confirmBtn.trigger('click')
    await flushPromises()
    expect(apiMock.setMcpPermissions).toHaveBeenCalledWith({
      levels: ['read', 'write', 'execute', 'network_execute'],
      acknowledgeHighRisk: true,
    })
    wrapper.unmount()
  })

  it('非高风险等级开关直接保存（无需确认）', async () => {
    const wrapper = mountTab()
    await flushPromises()
    const switches = wrapper.findAll('[role="switch"][aria-label]')
    // execute 已启用 → 点击 = 停用 → 无需确认直接保存。
    const execSwitch = switches.find((sw) => sw.attributes('aria-label') === '执行测试用例与场景')!
    await execSwitch.trigger('click')
    await flushPromises()
    expect(apiMock.setMcpPermissions).toHaveBeenCalledWith({
      levels: ['read', 'write'],
      acknowledgeHighRisk: false,
    })
    wrapper.unmount()
  })
})
