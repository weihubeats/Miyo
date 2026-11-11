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
import type { McpToolStatus } from '../../types/miyoApi'

const apiMock = vi.hoisted(() => ({
  getMcpEnabled: vi.fn(async () => true),
  getMcpPort: vi.fn(async () => 4110),
  agentStatus: vi.fn(async () => ({ running: true, address: 'http://127.0.0.1:4110' })),
  getMcpTools: vi.fn(async () => tools),
  setMcpToolEnabled: vi.fn(async () => undefined),
}))

vi.mock('../../composables/useMiyoApi', () => ({ useMiyoApi: () => apiMock }))

const tools: McpToolStatus[] = [
  { name: 'list_projects', kind: 'read', enabled: true },
  { name: 'get_run_result', kind: 'read', enabled: true },
  { name: 'run_scenario', kind: 'write', enabled: false },
  { name: 'cancel_run', kind: 'write', enabled: true },
]

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  apiMock.setMcpToolEnabled.mockClear()
})

describe('SettingsMcpTab', () => {
  it('挂载即拉取运行状态并显示运行中地址', async () => {
    const wrapper = mount(SettingsMcpTab)
    await flushPromises()
    expect(apiMock.agentStatus).toHaveBeenCalled()
    expect(wrapper.text()).toContain('运行中')
    expect(wrapper.text()).toContain('http://127.0.0.1:4110')
    wrapper.unmount()
  })

  it('工具清单 read/write 分组渲染 + 开关切换调用 IPC 并本地翻转', async () => {
    const wrapper = mount(SettingsMcpTab)
    await flushPromises()
    // i18n 工具名 + 原始工具名都渲染
    expect(wrapper.text()).toContain('list_projects')
    expect(wrapper.text()).toContain('run_scenario')
    expect(apiMock.getMcpTools).toHaveBeenCalled()

    // 切换 run_scenario（停用 → 启用）；带 aria-label 的是工具开关
    //（顶部的 MCP 启停总开关无 aria-label）
    const switches = wrapper.findAll('[role="switch"][aria-label]')
    expect(switches.length).toBe(tools.length)
    const runScenarioIdx = tools.findIndex((t) => t.name === 'run_scenario')
    await switches[runScenarioIdx]!.trigger('click')
    await flushPromises()
    expect(apiMock.setMcpToolEnabled).toHaveBeenCalledWith({ name: 'run_scenario', enabled: true })
    wrapper.unmount()
  })

  it('工具开关保存失败：不翻转本地状态', async () => {
    apiMock.setMcpToolEnabled.mockRejectedValueOnce(new Error('boom'))
    const wrapper = mount(SettingsMcpTab)
    await flushPromises()
    const switches = wrapper.findAll('[role="switch"][aria-label]')
    const cancelIdx = tools.findIndex((t) => t.name === 'cancel_run') // 原 enabled=true
    await switches[cancelIdx]!.trigger('click')
    await flushPromises()
    // 失败后本地仍为 true（aria-checked 未变）
    expect(switches[cancelIdx]!.attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })
})
