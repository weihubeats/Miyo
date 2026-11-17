/**
 * PluginManager（Pinia store）：插件生命周期编排的唯一入口。
 *
 * 职责：
 * - 启动：拉取已安装插件 → 逐个激活「已启用」的内置插件（懒加载：模块经
 *   builtinModules 动态 import，未启用插件零加载）；
 * - 生命周期动作：installBuiltin / setEnabled / uninstall / updateBuiltin /
 *   retryActivate（激活失败恢复）——全部经后端 IPC 持久化，前端只做运行时编排；
 * - 导航 / 路由注册表：插件经 SDK 声明的 nav 与 route 在此集中持有，
 *   DashboardNav / WorkspaceView / vue-router 按注册表响应式渲染；
 * - 隔离与回收：deactivate 顺序 = 实例钩子 → SDK dispose（事件解绑）→
 *   导航 / 路由注销 → 当前路由兜底重定向；
 * - 性能：activate 计时（performance.now）落在 PluginEntry 指标上，管理中心展示。
 *
 * 后端门禁兜底：java_* 等受控命令在后端按插件状态二次校验
 * （PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED），前端激活态只是第一道闸。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { tFallback, useLocaleStore } from '@/stores/locale'
import router from '@/router'
import { version as APP_VERSION } from '../../package.json'
import { builtinModules } from './builtinModules'
import { createPluginSession, type PluginHostBridge } from './sdk'
import { isCompatible, isSdkSupported } from './compatibility'
import { lookupPermission } from './permissions'
import type {
  PluginCatalogView,
  PluginEntry,
  PluginId,
  PluginNavItem,
  PluginView,
} from './types'

const IPC_PREFIX = 'plugin:miyo|'

async function callIpc<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(`${IPC_PREFIX}${command}`, args ?? {})
}

/** 核心路由（插件不得占用；DashboardNav 静态项同源）。 */
const CORE_ROUTE_PATHS = new Set([
  '/',
  '/projects',
  '/workspace',
  '/graphql',
  '/realtime',
  '/integration',
  '/ai-test',
  '/plugins',
])

export const usePluginManager = defineStore('pluginManager', () => {
  const locale = useLocaleStore()

  /** 活跃会话表（activate/deactivate 配对维护）。 */
  const activeSessions = new Map<
    PluginId,
    {
      session: ReturnType<typeof createPluginSession>
      instance: { deactivate?: () => void | Promise<void> } | null
    }
  >()

  // ---------- 状态 ----------
  /** 插件注册表（id → entry）。 */
  const entries = ref(new Map<PluginId, PluginEntry>())
  const booted = ref(false)
  /** 启动激活中的插件数（首屏不阻塞，仅指示）。 */
  const activatingCount = ref(0)

  /** 插件注册的导航项（pluginId → navItemId → item），宿主桥写入。 */
  const navRegistry = ref(new Map<PluginId, Map<string, PluginNavItem>>())
  /** 插件注册的路由（pluginId → path → routeName），宿主桥写入。 */
  const routeRegistry = ref(new Map<PluginId, Map<string, string>>())

  // ---------- 展示名 / 排序 ----------

  function displayName(manifest: PluginView['manifest']): string {
    return manifest.name_key ? locale.t(manifest.name_key) : manifest.name
  }

  /** 已激活插件的导航项（按 order 升序；order 相同按 id 稳定序）。 */
  const activeNavItems = computed<{ item: PluginNavItem; pluginId: PluginId }[]>(() => {
    const out: { item: PluginNavItem; pluginId: PluginId }[] = []
    for (const [pluginId, items] of navRegistry.value) {
      const entry = entries.value.get(pluginId)
      if (!entry || entry.runtime !== 'active') continue
      for (const item of items.values()) out.push({ item, pluginId })
    }
    return out.sort(
      (a, b) =>
        (a.item.order ?? 100) - (b.item.order ?? 100) ||
        a.item.id.localeCompare(b.item.id),
    )
  })

  /** 视图列表（稳定排序：启用 → 已安装 → 错误，再按 id）。 */
  const installedList = computed<PluginEntry[]>(() => {
    const rank = (e: PluginEntry) => (e.runtime === 'active' || e.view.enabled ? 0 : e.view.state === 'error' ? 2 : 1)
    return [...entries.value.values()].sort(
      (a, b) => rank(a) - rank(b) || a.view.id.localeCompare(b.view.id),
    )
  })

  // ---------- 宿主桥（SDK 导航 / 路由落地点） ----------

  const hostBridge: PluginHostBridge = {
    registerNav(pluginId, item) {
      let items = navRegistry.value.get(pluginId)
      if (!items) {
        items = new Map()
        navRegistry.value.set(pluginId, items)
      }
      items.set(item.id, item)
    },
    unregisterNav(pluginId, navItemId) {
      navRegistry.value.get(pluginId)?.delete(navItemId)
      if (navRegistry.value.get(pluginId)?.size === 0) navRegistry.value.delete(pluginId)
    },
    registerRoute(pluginId, route) {
      // 冲突防护：核心路由与其他插件路由不可占用。
      if (CORE_ROUTE_PATHS.has(route.path)) {
        throw new Error(
          `PLUGIN_ROUTE_CONFLICT: ${tFallback('plugins.err.routeConflictCore', { path: route.path })}`,
        )
      }
      for (const [owner, paths] of routeRegistry.value) {
        if (owner !== pluginId && paths.has(route.path)) {
          throw new Error(
            `PLUGIN_ROUTE_CONFLICT: ${tFallback('plugins.err.routeConflictTaken', { path: route.path, owner })}`,
          )
        }
      }
      const name = route.name ?? `plugin-${pluginId}-${route.path.replaceAll('/', '-')}`
      // route.load 是 () => import('…vue') 形态，即 vue-router 的懒加载组件形态；
      // meta.pluginId 供路由守卫识别归属（停用插件的路由直接拒绝导航）。
      router.addRoute({
        path: route.path,
        name,
        component: route.load as unknown as () => Promise<unknown>,
        meta: { pluginId },
      })
      let paths = routeRegistry.value.get(pluginId)
      if (!paths) {
        paths = new Map()
        routeRegistry.value.set(pluginId, paths)
      }
      paths.set(route.path, name)
    },
    unregisterRoute(pluginId, path) {
      const name = routeRegistry.value.get(pluginId)?.get(path)
      if (!name) return
      // 正在该路由上时先兜底重定向（removeRoute 后当前路由解析失效）。
      if (router.currentRoute.value.path === path) {
        void router.replace('/projects')
      }
      if (router.hasRoute(name)) router.removeRoute(name)
      routeRegistry.value.get(pluginId)?.delete(path)
      if (routeRegistry.value.get(pluginId)?.size === 0) routeRegistry.value.delete(pluginId)
    },
  }

  // ---------- 内部：注册表维护 ----------

  function upsertEntry(view: PluginView): PluginEntry {
    const existing = entries.value.get(view.id)
    const entry: PluginEntry = {
      view,
      runtime: existing?.runtime ?? 'inactive',
      displayName: displayName(view.manifest),
      lastActivateMs: existing?.lastActivateMs ?? null,
      activateCount: existing?.activateCount ?? 0,
      lastActivatedAt: existing?.lastActivatedAt ?? null,
    }
    entries.value.set(view.id, entry)
    return entry
  }

  function dropEntry(id: PluginId): void {
    entries.value.delete(id)
    navRegistry.value.delete(id)
    routeRegistry.value.delete(id)
  }

  // ---------- 激活 / 停用 ----------

  /** 激活一个「已启用」插件：动态 import 模块 → 构造 SDK 会话 → 调用工厂。 */
  async function activate(id: PluginId): Promise<void> {
    const entry = entries.value.get(id)
    if (!entry) return
    if (entry.runtime === 'active' || entry.runtime === 'activating') return
    entry.runtime = 'activating'
    activatingCount.value++
    const started = performance.now()
    /** 激活成功后的公共收尾（运行时态 + 指标）。 */
    const markActive = (): void => {
      entry.runtime = 'active'
      entry.lastActivateMs = Math.round(performance.now() - started)
      entry.activateCount++
      entry.lastActivatedAt = Date.now()
    }
    try {
      const loader = builtinModules[id]
      if (loader) {
        // 内置插件：动态 import 仓内模块 → 工厂拿到 SDK context。
        const mod = await loader()
        const session = createPluginSession(entry.view, hostBridge)
        const instance = await mod.default(session.context)
        activeSessions.set(id, { session, instance })
        markActive()
      } else if (entry.view.kind === 'ui' && entry.view.manifest.entry) {
        // 第三方 UI 插件：沙箱 iframe 宿主（host/，opaque origin + postMessage
        // RPC）；模块代码永远不进主 WebView，会话由宿主组件经 sessionOf 取用。
        const session = createPluginSession(entry.view, hostBridge)
        activeSessions.set(id, { session, instance: null })
        for (const item of entry.view.manifest.nav ?? []) {
          hostBridge.registerNav(id, item)
          hostBridge.registerRoute(id, {
            path: item.route,
            name: `plugin-${id}-${item.id}`,
            load: () => import('@/plugins/host/PluginIframeHost.vue'),
          })
        }
        markActive()
      } else {
        // 无加载器且无 entry：不可激活。
        entry.runtime = 'error'
        await reportError(id, tFallback('plugins.err.noLoader', { id })).catch(() => {})
        unregisterAllUi(id)
      }
    } catch (err) {
      entry.runtime = 'error'
      const message = err instanceof Error ? err.message : String(err)
      console.error(`[plugin:${id}] activate failed:`, err)
      await reportError(id, message).catch(() => {})
      // 激活失败的插件不得残留半注册状态：回收导航 / 路由。
      unregisterAllUi(id)
    } finally {
      activatingCount.value--
    }
  }

  /** 停用运行时（导航 / 路由回收 + 实例钩子 + SDK dispose）。 */
  async function deactivate(id: PluginId): Promise<void> {
    const active = activeSessions.get(id)
    unregisterAllUi(id)
    activeSessions.delete(id)
    const entry = entries.value.get(id)
    if (entry) entry.runtime = 'inactive'
    if (!active) return
    try {
      await active.instance?.deactivate?.()
    } catch (err) {
      console.error(`[plugin:${id}] deactivate hook failed:`, err)
    }
    active.session.dispose()
  }

  /** 注销某插件全部导航 / 路由（激活失败回收与停用共用）。 */
  function unregisterAllUi(id: PluginId): void {
    const paths = routeRegistry.value.get(id)
    if (paths) {
      for (const path of [...paths.keys()]) hostBridge.unregisterRoute(id, path)
    }
    navRegistry.value.delete(id)
  }

  /** 激活失败上报（后端落 error_json；state → error）。 */
  async function reportError(id: PluginId, message: string): Promise<void> {
    try {
      await callIpc('plugin_report_error', { id, message })
      const entry = entries.value.get(id)
      if (entry) {
        entry.view = { ...entry.view, state: 'error', enabled: false }
      }
    } catch {
      // 后端不可达时保持前端本地错误态
    }
  }

  // ---------- 生命周期动作（管理中心调用） ----------

  /** 启动引导：拉取已安装插件；启用态的逐个激活（不阻塞首屏）。 */
  async function boot(): Promise<void> {
    if (booted.value) return
    booted.value = true
    // 路由守卫（注册一次）：插件路由仅在所属插件激活时可导航——停用 / 卸载
    // 竞态窗口内直接访问插件路由时给出可解释的重定向，而不是白屏。
    router.beforeEach((to) => {
      const pluginId = to.meta?.pluginId as PluginId | undefined
      if (!pluginId || isActive(pluginId)) return true
      return { path: '/plugins', query: { blocked: pluginId } }
    })
    let installed: PluginView[] = []
    try {
      installed = await callIpc<PluginView[]>('plugin_list_installed')
    } catch {
      // 非 Tauri（浏览器预览 / 部分单测）：插件系统以空目录降级运行
      return
    }
    for (const view of installed) {
      upsertEntry(view)
      if (view.enabled) void activate(view.id)
    }
  }

  /** 刷新已安装列表（管理中心操作后调用）。 */
  async function refreshInstalled(): Promise<void> {
    try {
      const installed = await callIpc<PluginView[]>('plugin_list_installed')
      const seen = new Set<PluginId>()
      for (const view of installed) {
        seen.add(view.id)
        upsertEntry(view)
      }
      // 后端已消失的插件（他端卸载 / 数据回滚）：本地回收。
      for (const id of [...entries.value.keys()]) {
        if (!seen.has(id)) {
          await deactivate(id)
          dropEntry(id)
        }
      }
    } catch {
      // 非 Tauri 环境：忽略
    }
  }

  /** 市场目录（内置目录 + 安装状态 + 兼容性）。 */
  async function loadCatalog(): Promise<PluginCatalogView[]> {
    try {
      return await callIpc<PluginCatalogView[]>('plugin_list_catalog')
    } catch {
      return []
    }
  }

  /** 安装内置插件（权限在安装确认页展示后调用）。 */
  async function installBuiltin(id: PluginId): Promise<void> {
    const view = await callIpc<PluginView>('plugin_install_builtin', { id })
    upsertEntry(view)
    await refreshInstalled()
  }

  /** 启用（含兼容性 + SDK 协议检查的后端校验）并激活。 */
  async function enable(id: PluginId): Promise<void> {
    const entry = entries.value.get(id)
    if (entry) {
      const m = entry.view.manifest
      // 前端预检（后端强制为真源）：不兼容直接抛，避免无效 IPC 往返。
      const appVersion = await appVersionCached()
      if (!isCompatible(m.min_app_version, appVersion)) {
        throw new Error(
          `PLUGIN_INCOMPATIBLE: ${tFallback('plugins.err.incompatibleVersion', { v: m.min_app_version })}`,
        )
      }
      if (!isSdkSupported(m.sdk_version)) {
        throw new Error(
          `PLUGIN_INCOMPATIBLE: ${tFallback('plugins.err.incompatibleSdk', { v: String(m.sdk_version) })}`,
        )
      }
    }
    const view = await callIpc<PluginView>('plugin_set_enabled', { id, enabled: true })
    upsertEntry(view)
    await activate(id)
  }

  /** 停用：先回收运行时，再落状态。 */
  async function disable(id: PluginId): Promise<void> {
    await deactivate(id)
    const view = await callIpc<PluginView>('plugin_set_enabled', { id, enabled: false })
    upsertEntry(view)
  }

  /** 卸载：回收运行时 + 清注册表，后端级联清理存储。 */
  async function uninstall(id: PluginId): Promise<void> {
    await deactivate(id)
    await callIpc('plugin_uninstall', { id })
    dropEntry(id)
  }

  /** 更新内置插件到随应用打包的新版本。 */
  async function updateBuiltin(id: PluginId): Promise<void> {
    const view = await callIpc<PluginView>('plugin_update_builtin', { id })
    upsertEntry(view)
    // 若处于启用态，重新激活（manifest 可能变化）。
    if (view.enabled) {
      await deactivate(id)
      await activate(id)
    }
  }

  /** 激活失败恢复：重试 activate（error → activating → active / error）。 */
  async function retryActivate(id: PluginId): Promise<void> {
    const entry = entries.value.get(id)
    if (!entry) return
    entry.runtime = 'inactive'
    // 错误态在启用成功激活后被后端覆盖为 enabled；若后端仍是 error，先恢复启用。
    if (!entry.view.enabled) {
      const view = await callIpc<PluginView>('plugin_set_enabled', { id, enabled: true })
      upsertEntry(view)
    }
    await activate(id)
  }

  // ---------- 查询辅助 ----------

  /** 插件是否处于激活态（路由守卫用）。 */
  function isActive(id: PluginId): boolean {
    return entries.value.get(id)?.runtime === 'active'
  }

  /** 取插件活跃 SDK 会话（沙箱宿主建 RPC 桥用；未激活返回 null）。 */
  function sessionOf(id: PluginId) {
    return activeSessions.get(id)?.session ?? null
  }

  /** 权限展示数据（管理中心）。 */
  function permissionMeta(id: string) {
    return lookupPermission(id)
  }

  /**
   * 当前应用版本（package.json，与 tauri.conf.json / 发版流程同源）。
   * 兼容性前端仅做 UX 预检，启用/更新以后端强制校验为准。
   */
  function appVersionCached(): string {
    return APP_VERSION
  }

  return {
    entries,
    booted,
    activatingCount,
    activeNavItems,
    installedList,
    hostBridge,
    boot,
    refreshInstalled,
    loadCatalog,
    installBuiltin,
    enable,
    disable,
    uninstall,
    updateBuiltin,
    retryActivate,
    reportError,
    isActive,
    sessionOf,
    permissionMeta,
  }
})
