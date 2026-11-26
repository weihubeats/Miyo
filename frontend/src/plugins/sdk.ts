/**
 * PluginSDK：createPluginSession 为插件构造受控的宿主上下文（SDK 根对象）。
 *
 * 边界规则（SDK 层强制，后端插件门禁兜底）：
 * - `ctx.commands.invoke` 只放行「命令 → 权限」映射表内的命令，且权限必须在
 *   用户批准快照（granted_permissions）内；
 * - `ctx.env` / `ctx.notify` / `ctx.events` / `ctx.clipboard` 分别对应
 *   std.env.read / std.notify / std.events / std.clipboard.write 权限，越权抛错；
 * - `ctx.storage` / `ctx.config` 由后端按 plugin_id 行级隔离（键名白名单校验）；
 * - `ctx.ui` 的导航 / 路由注册经 PluginHostBridge 委托给 PluginManager 统一落地，
 *   插件不直接触碰 vue-router 实例或核心组件。
 *
 * dispose()：deactivate 时由 manager 调用——解绑该插件注册的全部事件监听，
 * 保证「卸载后释放事件监听和运行时资源」。
 */
import { invoke } from '@tauri-apps/api/core'
import { useToast } from '@/composables/useToast'
import { tFallback } from '@/stores/locale'
import { pluginEventBus, type PluginEventHandler } from './events'
import { effectiveGranted, requiredPermissionFor } from './permissions'
import type {
  PluginContext,
  PluginNavItem,
  PluginView,
} from './types'

const IPC_PREFIX = 'plugin:miyo|'

/** manager 提供的宿主桥（导航 / 路由落地点；sdk 不直接 import manager 防环）。 */
export interface PluginHostBridge {
  registerNav(pluginId: string, item: PluginNavItem): void
  unregisterNav(pluginId: string, navItemId: string): void
  registerCommand(
    pluginId: string,
    cmd: { id: string; label: string; icon?: string },
  ): void
  unregisterCommand(pluginId: string, cmdId: string): void
  registerRoute(
    pluginId: string,
    route: { path: string; name?: string; load: () => Promise<unknown> },
  ): void
  unregisterRoute(pluginId: string, path: string): void
}

/** IPC 调用（错误透传为携带 code 的对象——与 useMiyoApi.call 同一约定）。 */
async function callIpc<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(`${IPC_PREFIX}${command}`, args ?? {})
}

function permissionError(permission: string): Error {
  return new Error(
    `PLUGIN_PERMISSION_DENIED: ${tFallback('plugins.err.permissionDenied', { permission })}`,
  )
}

/** 判断 IPC 错误是否为权限 / 会话类拒绝（FORBIDDEN 语义）。 */
function isForbiddenError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    (err as { code?: string }).code === 'FORBIDDEN'
  )
}

/** 一个已激活插件会话：context 给插件，dispose 由宿主回收。 */
export interface PluginSession {
  context: PluginContext
  dispose(): void
}

/** 为插件构造 SDK 会话。 */
export function createPluginSession(view: PluginView, host: PluginHostBridge): PluginSession {
  const pluginId = view.id
  const granted = effectiveGranted(view)
  const manifest = view.manifest

  const hasPermission = (permission: string): boolean => granted.has(permission)
  const requirePermission = (permission: string): void => {
    if (!hasPermission(permission)) throw permissionError(permission)
  }

  // ---- 会话凭证（std 族命令的后端强制凭据） ----
  // 惰性签发（首个 storage / log 调用时换取）；FORBIDDEN（收权 / 停用 /
  // 过期）时重建一次并重试——撤销即时生效且调用方无感，二次失败如实上抛。
  let sessionToken: string | null = null
  const ensureSessionToken = async (): Promise<string> => {
    if (sessionToken === null) {
      sessionToken = await callIpc<string>('plugin_session_create', { id: pluginId })
    }
    return sessionToken
  }
  const dropSessionToken = (): void => {
    sessionToken = null
  }
  const callWithSession = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    try {
      return await callIpc<T>(command, { ...args, sessionToken: await ensureSessionToken() })
    } catch (err) {
      if (!isForbiddenError(err)) throw err
      dropSessionToken()
      return callIpc<T>(command, { ...args, sessionToken: await ensureSessionToken() })
    }
  }

  // ---- 事件：记录 handler 以便 dispose 统一解绑 ----
  const myHandlers = new Set<PluginEventHandler>()
  const events = {
    emit(event: string, payload?: unknown): void {
      requirePermission('std.events')
      pluginEventBus.emit(`${pluginId}:${event}`, payload)
    },
    on(event: string, handler: PluginEventHandler): () => void {
      requirePermission('std.events')
      myHandlers.add(handler)
      return pluginEventBus.on(`${pluginId}:${event}`, handler)
    },
  }

  // ---- 日志：落后端（fire-and-forget）+ console 带前缀 ----
  const log = (level: string, message: string): void => {
    const line = `[plugin:${pluginId}] ${message}`
    if (level === 'error') console.error(line)
    else if (level === 'warn') console.warn(line)
    else console.info(line)
    // 敏感信息不得经普通日志外带：SDK 不做内容过滤（调用方契约 + 管理中心可见），
    // 但长度兜底截断，避免把大对象序列化进日志表。
    const trimmed = message.length > 2000 ? `${message.slice(0, 2000)}…` : message
    void callWithSession('plugin_record_log', { id: pluginId, level, message: trimmed }).catch(
      () => {
        // 日志通道失败不影响插件运行
      },
    )
  }
  const logger = {
    debug: (m: string) => log('debug', m),
    info: (m: string) => log('info', m),
    warn: (m: string) => log('warn', m),
    error: (m: string) => log('error', m),
    audit: (m: string) => log('audit', m),
  }

  // ---- 通知 ----
  const toast = useToast()
  const notify = {
    info: (m: string, title?: string) => {
      requirePermission('std.notify')
      toast.info(title ?? manifest.name, { message: m })
    },
    success: (m: string, title?: string) => {
      requirePermission('std.notify')
      toast.success(title ?? manifest.name, { message: m })
    },
    warn: (m: string, title?: string) => {
      requirePermission('std.notify')
      toast.toast({ type: 'warning', title: title ?? manifest.name, message: m })
    },
    error: (m: string, title?: string) => {
      requirePermission('std.notify')
      toast.error(title ?? manifest.name, { message: m })
    },
  }

  // ---- 存储（后端按 plugin_id 隔离 + 会话凭证逐调用校验；__config 为配置保留键） ----
  const storage = {
    async get<T = unknown>(key: string): Promise<T | null> {
      const raw = await callWithSession<unknown | null>('plugin_data_get', { id: pluginId, key })
      return (raw ?? null) as T | null
    },
    async set(key: string, value: unknown): Promise<void> {
      await callWithSession('plugin_data_set', { id: pluginId, key, value })
    },
    async delete(key: string): Promise<void> {
      await callWithSession('plugin_data_delete', { id: pluginId, key })
    },
    async keys(): Promise<string[]> {
      return callWithSession<string[]>('plugin_data_keys', { id: pluginId })
    },
  }

  // ---- 配置（__config 键；configSchema 校验由管理中心表单 + 后端存储约束兜底） ----
  let configCache: Record<string, unknown> | null = null
  const config = {
    get(): Record<string, unknown> {
      return configCache ?? {}
    },
    async set(values: Record<string, unknown>): Promise<void> {
      configCache = { ...(configCache ?? {}), ...values }
      await storage.set('__config', configCache)
    },
  }
  // activate 期间预取（fire-and-forget；失败保持空配置）。null 守卫：
  // 预取晚于用户写入到达时不得覆盖已合并的缓存（最终一致性：activate 后
  // 立即写配置的窗口内以写入为准）。
  void storage
    .get<Record<string, unknown>>('__config')
    .then((v) => {
      if (configCache === null) configCache = v ?? {}
    })
    .catch(() => undefined)

  // ---- 受控主程序命令（白名单 + 已批准权限） ----
  const commands = {
    async invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> {
      const required = requiredPermissionFor(command)
      if (!required) {
        throw new Error(
          `PLUGIN_COMMAND_DENIED: ${tFallback('plugins.err.commandDenied', { command })}`,
        )
      }
      requirePermission(required)
      // 后端插件门禁兜底（未安装/停用 → PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED）。
      return callIpc<T>(command, args)
    },
  }

  // ---- 环境变量合并表（std.env.read；动态 import 避免把 workspace store 拖进首屏链） ----
  const env = {
    async getVariableMap(): Promise<Record<string, string>> {
      requirePermission('std.env.read')
      const { useWorkspaceStore } = await import('@/stores/workspace')
      const { mergedVariableMap } = await import('@/utils/environment')
      const ws = useWorkspaceStore()
      return mergedVariableMap({
        globalVariables: ws.globalVariables,
        projectVariables: ws.project?.variables,
        env: ws.environments.find((e) => e.id === ws.activeEnvId) ?? null,
      })
    },
  }

  // ---- 剪贴板（std.clipboard.write） ----
  const clipboard = {
    async writeText(text: string): Promise<void> {
      requirePermission('std.clipboard.write')
      await callIpc('clipboard_write_text', { text })
    },
  }

  // ---- 导航 / 路由注册（委托宿主桥） ----
  const ui = {
    nav: {
      register(item: PluginNavItem): void {
        host.registerNav(pluginId, item)
      },
      unregister(navItemId: string): void {
        host.unregisterNav(pluginId, navItemId)
      },
    },
    route: {
      register(route: { path: string; name?: string; load: () => Promise<unknown> }): void {
        host.registerRoute(pluginId, route)
      },
      unregister(path: string): void {
        host.unregisterRoute(pluginId, path)
      },
    },
    registerCommand(cmd: { id: string; label: string; icon?: string }): void {
      if (!cmd.id || cmd.id.length > 48) {
        throw new Error(`PLUGIN_BAD_REQUEST: ${tFallback('plugins.err.badCommandId')}`)
      }
      host.registerCommand(pluginId, cmd)
    },
    unregisterCommand(cmdId: string): void {
      host.unregisterCommand(pluginId, cmdId)
    },
  }

  // ---- per-plugin 文件数据目录（Rust 懒创建；卸载「不保留数据」时删除） ----
  const paths = {
    async data() {
      return callIpc<{ root: string; data: string; cache: string; tmp: string }>('plugin_dir_get', {
        id: view.manifest.id,
      })
    },
  }

  const context: PluginContext = {
    manifest,
    storage,
    logger,
    notify,
    events,
    env,
    commands,
    config,
    clipboard,
    ui,
    paths,
  }

  return {
    context,
    dispose(): void {
      pluginEventBus.disposeAllOf(myHandlers)
      myHandlers.clear()
      // 导航 / 路由由 manager 在 deactivate 流程统一回收（保持单点状态）。
    },
  }
}
