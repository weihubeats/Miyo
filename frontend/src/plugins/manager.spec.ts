/**
 * PluginManager 生命周期单测：安装 → 启用 → 激活（导航/路由注册）→ 停用（回收）→
 * 卸载；激活失败恢复；兼容性预检；路由冲突防护。
 *
 * 后端 IPC 整体打桩（invoke mock）：生命周期编排是本 store 的职责边界，
 * 持久化正确性由 Rust 侧 repository 测试保证。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import router from '@/router'
import { useLocaleStore } from '@/stores/locale'
import { usePluginManager } from '@/plugins/manager'
import { JAVA_DEBUG_PLUGIN_ID, javaView } from '@/testUtils/pluginFixture'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

beforeEach(async () => {
  vi.clearAllMocks()
  invokeMock.mockReset()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  // 清理真实 router：上一用例注册的插件路由不串场
  for (const route of router.getRoutes()) {
    if (String(route.name).startsWith('plugin-')) router.removeRoute(String(route.name))
  }
})

describe('PluginManager 生命周期', () => {
  it('boot：启用态插件自动激活，注册导航与路由', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    // 激活是异步链（动态 import 模块），等待运行时态落地
    await vi.waitFor(() => {
      expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true)
    })

    // 导航注册
    expect(manager.activeNavItems).toHaveLength(1)
    expect(manager.activeNavItems[0].item.route).toBe('/java-debug')
    expect(manager.activeNavItems[0].item.label_key).toBe('pnav.javaDebug')
    // 路由注册（可解析）
    expect(router.hasRoute('plugin-java-debug-main')).toBe(true)
    expect(router.resolve('/java-debug').matched.length).toBe(1)
    // 激活耗时指标
    const entry = manager.entries.get(JAVA_DEBUG_PLUGIN_ID)
    expect(entry?.activateCount).toBe(1)
    expect(entry?.lastActivateMs).toBeGreaterThanOrEqual(0)
  })

  it('disable：停用后导航 / 路由回收，路由跳转被守卫重定向', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      if (cmd.endsWith('plugin_set_enabled')) {
        return javaView({ view: { enabled: args?.enabled as boolean, state: args?.enabled ? 'enabled' : 'installed' } })
      }
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))

    await manager.disable(JAVA_DEBUG_PLUGIN_ID)
    // 导航清空 / 路由注销
    expect(manager.activeNavItems).toHaveLength(0)
    expect(router.hasRoute('plugin-java-debug-main')).toBe(false)
    expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(false)
    expect(invokeMock).toHaveBeenCalledWith(
      'plugin:miyo|plugin_set_enabled',
      expect.objectContaining({ id: JAVA_DEBUG_PLUGIN_ID, enabled: false }),
    )
  })

  it('uninstall：注册表清空，后端卸载命令被调用', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      if (cmd.endsWith('plugin_uninstall')) return null
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))

    await manager.uninstall(JAVA_DEBUG_PLUGIN_ID)
    expect(manager.entries.has(JAVA_DEBUG_PLUGIN_ID)).toBe(false)
    expect(manager.activeNavItems).toHaveLength(0)
    expect(router.hasRoute('plugin-java-debug-main')).toBe(false)
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_uninstall', { id: JAVA_DEBUG_PLUGIN_ID })
  })

  it('激活失败：状态转 error 并上报后端（恢复入口可见）', async () => {
    // 用不存在的插件 id 模拟 loader 缺失（非内置目录插件不可激活）
    invokeMock.mockImplementation(async (cmd: string, _args?: Record<string, unknown>) => {
      if (cmd.endsWith('plugin_list_installed')) {
        return [javaView({ manifest: { id: 'com.example.ghost', nav: [] } })]
      }
      if (cmd.endsWith('plugin_report_error')) return null
      if (cmd.endsWith('plugin_set_enabled')) return javaView({ manifest: { id: 'com.example.ghost', nav: [] } })
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => {
      expect(manager.entries.get('com.example.ghost')?.runtime).toBe('error')
    })
    expect(invokeMock).toHaveBeenCalledWith(
      'plugin:miyo|plugin_report_error',
      expect.objectContaining({ id: 'com.example.ghost' }),
    )
    // 视图状态同步为 error（管理中心展示恢复入口）
    expect(manager.entries.get('com.example.ghost')?.view.state).toBe('error')
  })

  it('enable：版本不兼容在前端预检拒绝（不发 IPC）', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) {
        return [javaView({ view: { enabled: false, state: 'installed' }, manifest: { min_app_version: '9.9.9' } })]
      }
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await expect(manager.enable(JAVA_DEBUG_PLUGIN_ID)).rejects.toThrow('PLUGIN_INCOMPATIBLE')
    // 预检失败：未发生启用 IPC
    const enableCalls = invokeMock.mock.calls.filter(([cmd]) => String(cmd).endsWith('plugin_set_enabled'))
    expect(enableCalls).toHaveLength(0)
  })

  it('SDK SDK 协议版本不受支持：拒绝启用', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) {
        return [javaView({ view: { enabled: false, state: 'installed' }, manifest: { sdk_version: 2 } })]
      }
      throw new Error(`unexpected ${cmd}`)
    })
    const manager = usePluginManager()
    await manager.boot()
    await expect(manager.enable(JAVA_DEBUG_PLUGIN_ID)).rejects.toThrow('PLUGIN_INCOMPATIBLE')
  })

  it('路由冲突防护：插件不得注册核心路由 / 他人路由', async () => {
    const manager = usePluginManager()
    expect(() =>
      manager.hostBridge.registerRoute(JAVA_DEBUG_PLUGIN_ID, {
        path: '/projects',
        load: async () => ({}),
      }),
    ).toThrow('PLUGIN_ROUTE_CONFLICT')

    manager.hostBridge.registerRoute('com.a.one', { path: '/p/a', load: async () => ({}) })
    expect(() =>
      manager.hostBridge.registerRoute('com.b.two', { path: '/p/a', load: async () => ({}) }),
    ).toThrow('PLUGIN_ROUTE_CONFLICT')
  })
})
