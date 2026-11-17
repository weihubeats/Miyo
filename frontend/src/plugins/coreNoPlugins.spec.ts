/**
 * 核心无插件回归：一个没有安装任何插件的 Miyo —— 导航只含核心项、
 * 插件路由不存在、核心路由全部可达；安装 java 插件后入口出现、卸载后恢复原状
 * （最终验收标准的自动化断言）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import router from '@/router'
import DashboardNav from '@/domains/workspace/components/projectlist/projectlist/DashboardNav.vue'
import { useLocaleStore } from '@/stores/locale'
import { usePluginManager } from '@/plugins/manager'
import { JAVA_DEBUG_PLUGIN_ID, javaView } from '@/testUtils/pluginFixture'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const CORE_NAV_ROUTES = ['/projects', '/integration', '/realtime', '/ai-test', '/plugins']

beforeEach(async () => {
  vi.clearAllMocks()
  invokeMock.mockReset()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  for (const route of router.getRoutes()) {
    if (String(route.name).startsWith('plugin-')) router.removeRoute(String(route.name))
  }
})

/** 挂载仪表板导航（真实 pinia + router，无组件桩）。 */
async function mountNav(): Promise<string[]> {
  if (!router.currentRoute.value.matched.length) {
    await router.push('/projects')
  }
  const wrapper = mount(DashboardNav, { global: { plugins: [router] } })
  const labels = wrapper.findAll('.nav-item .nav-label').map((n) => n.text())
  wrapper.unmount()
  return labels
}

describe('Core 无插件回归（最终验收标准）', () => {
  it('未安装插件：导航无插件项、/java-debug 不可解析、核心路由全可达', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) return []
      throw new Error(`unexpected ${cmd}`)
    })
    const manager = usePluginManager()
    await manager.boot()

    expect(manager.installedList).toHaveLength(0)
    expect(manager.activeNavItems).toHaveLength(0)
    expect(router.resolve('/java-debug').matched).toHaveLength(0)
    for (const path of CORE_NAV_ROUTES) {
      expect(router.resolve(path).matched, path).toHaveLength(1)
    }
  })

  it('未安装插件：仪表板导航只渲染核心项', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd.endsWith('plugin_list_installed')) return []
      throw new Error(`unexpected ${cmd}`)
    })
    const manager = usePluginManager()
    await manager.boot()

    const labels = await mountNav()
    expect(labels).toHaveLength(CORE_NAV_ROUTES.length)
    expect(labels).toContain('仪表板')
    expect(labels).toContain('插件中心')
    expect(labels.join('|')).not.toContain('Java')
  })

  it('安装并启用 Java 插件：导航出现入口；卸载后导航与路由复原', async () => {
    let installed: ReturnType<typeof javaView>[] = []
    invokeMock.mockImplementation(async (cmd: string, _args?: Record<string, unknown>) => {
      if (cmd.endsWith('plugin_list_installed')) return installed
      if (cmd.endsWith('plugin_install_builtin')) {
        installed = [javaView({ view: { enabled: false, state: 'installed' } })]
        return installed[0]
      }
      if (cmd.endsWith('plugin_set_enabled')) {
        installed = [javaView({ view: { enabled: true, state: 'enabled' } })]
        return installed[0]
      }
      if (cmd.endsWith('plugin_uninstall')) {
        installed = []
        return null
      }
      throw new Error(`unexpected ${cmd}`)
    })

    const manager = usePluginManager()
    await manager.boot()
    expect((await mountNav()).join('|')).not.toContain('Java')

    // 安装（市场动作）→ 启用（用户确认权限后）
    await manager.installBuiltin(JAVA_DEBUG_PLUGIN_ID)
    await manager.enable(JAVA_DEBUG_PLUGIN_ID)
    await vi.waitFor(() => expect(manager.isActive(JAVA_DEBUG_PLUGIN_ID)).toBe(true))

    const labels = await mountNav()
    expect(labels).toContain('Java 方法调试')
    expect(router.resolve('/java-debug').matched).toHaveLength(1)

    // 卸载：恢复未安装原状
    await manager.uninstall(JAVA_DEBUG_PLUGIN_ID)
    expect(router.resolve('/java-debug').matched).toHaveLength(0)
    expect((await mountNav()).join('|')).not.toContain('Java')
    expect(manager.installedList).toHaveLength(0)
  })
})
