/**
 * Java 内置插件独立运行测试：经 PluginManager 全生命周期验证
 * 「安装 → 启用 → 激活（导航 / 路由可用）→ 停用（资源回收）」。
 *
 * 与 manager.spec 的差异：本 spec 不打桩 builtinModules，走真实插件模块
 * （domains/java-debug/plugin.ts → JavaDebugView 懒加载），验证 loader 表、
 * manifest 驱动的注册与 javaDebug store 资源回收的真实链路。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import router from '@/router'
import { useLocaleStore } from '@/stores/locale'
import { usePluginManager } from '@/plugins/manager'
import { builtinModules } from '@/plugins/builtinModules'
import { JAVA_DEBUG_PLUGIN_ID, javaView } from '@/testUtils/pluginFixture'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    toast: vi.fn(),
    dismiss: vi.fn(),
  }),
}))

beforeEach(async () => {
  vi.clearAllMocks()
  invokeMock.mockReset()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  for (const route of router.getRoutes()) {
    if (String(route.name).startsWith('plugin-')) router.removeRoute(String(route.name))
  }
})

describe('Java 内置插件', () => {
  it('loader 表包含 java-debug（未启用绝不 import——表内只有懒加载工厂）', () => {
    expect(typeof builtinModules[JAVA_DEBUG_PLUGIN_ID]).toBe('function')
  })

  it('全生命周期：激活注册 /java-debug → 停用回收 + store 资源释放', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      if (cmd.endsWith('plugin_set_enabled')) {
        return javaView({
          view: { enabled: args?.enabled as boolean, state: args?.enabled ? 'enabled' : 'installed' },
        })
      }
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))

    // 路由可达且视图 chunk 可加载（真实动态 import JavaDebugView）
    const resolved = router.resolve('/java-debug')
    expect(resolved.matched).toHaveLength(1)
    expect(resolved.meta.pluginId).toBe(JAVA_DEBUG_PLUGIN_ID)
    const component = (resolved.matched[0].components as { default: { (): Promise<unknown> } }).default
    const mod = (await component()) as { default: unknown }
    expect(mod.default).toBeDefined() // Vue 组件定义

    // 预置插件运行时状态 → 停用应触发 dispose（清空在途状态）
    const store = useJavaDebugStore()
    store.recentMethods.push({
      projectId: 'p1',
      methodId: 'm1',
      className: 'C',
      signature: 's',
      at: 1,
    })
    expect(store.recentMethods).toHaveLength(1)

    await manager.disable(JAVA_DEBUG_PLUGIN_ID)
    expect(router.hasRoute('plugin-java-debug-main')).toBe(false)
    expect(manager.activeNavItems).toHaveLength(0)
    // javaDebug store 资源已由插件 deactivate 回收
    expect(store.recentMethods).toHaveLength(0)
    expect(store.connection.state).toBe('idle')
  })

  it('重激活：停用后再次启用，注册表恢复', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      if (cmd.endsWith('plugin_set_enabled')) {
        return javaView({
          view: { enabled: args?.enabled as boolean, state: args?.enabled ? 'enabled' : 'installed' },
        })
      }
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))

    await manager.disable(JAVA_DEBUG_PLUGIN_ID)
    expect(router.hasRoute('plugin-java-debug-main')).toBe(false)

    await manager.enable(JAVA_DEBUG_PLUGIN_ID)
    await vi.waitFor(() => {
      expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true)
      expect(router.hasRoute('plugin-java-debug-main')).toBe(true)
    })
    // 累计激活 2 次（指标用于管理中心性能展示）
    expect(manager.entries.get(JAVA_DEBUG_PLUGIN_ID)?.activateCount).toBe(2)
  })

  it('卸载后 /java-debug 不再可解析（核心路由不受影响）', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) return [javaView()]
      if (cmd.endsWith('plugin_uninstall')) return null
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))
    await manager.uninstall(JAVA_DEBUG_PLUGIN_ID)

    expect(router.resolve('/java-debug').matched).toHaveLength(0)
    // 核心路由完好
    expect(router.resolve('/projects').matched).toHaveLength(1)
    expect(router.resolve('/plugins').matched).toHaveLength(1)
  })
})
