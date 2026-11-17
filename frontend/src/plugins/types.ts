/**
 * Miyo 插件系统 —— 前端核心类型。
 *
 * 分层约定：
 * - IPC 传输类型（PluginManifest / PluginView / PluginCatalogView …）由
 *   Rust `miyo-plugin` / `miyo-tauri::commands::plugins` 经 ts-rs 导出到
 *   `types/generated/`（本文件 re-export，**不手写镜像**——CI bindings job 防漂移）；
 * - 运行时状态（PluginRuntimeState / PluginInstance / SDK 接口）是 WebView 内概念，
 *   在此手写定义；
 * - 第三方插件的 UI 跑在沙箱 iframe（`host/`），内置插件是同仓 Vue 模块
 *   （`builtinModules.ts` 懒加载表），两者共用同一套 SDK 接口。
 */
import type { PluginCatalogView } from '@/types/generated/PluginCatalogView'
import type { PluginManifest } from '@/types/generated/PluginManifest'
import type { PluginNavItem } from '@/types/generated/PluginNavItem'
import type { PluginView } from '@/types/generated/PluginView'

export type { PluginCatalogView, PluginManifest, PluginNavItem, PluginView }

/** 插件 ID（reverse-DNS，与 Rust 侧校验规则一致）。 */
export type PluginId = string

/** 后端持久化的插件生命周期状态（PluginView.state）。 */
export type PluginPersistedState = 'installed' | 'enabled' | 'error'

/** 前端插件运行时状态（不持久化；激活是 WebView 内概念）。 */
export type PluginRuntimeState = 'inactive' | 'activating' | 'active' | 'error'

/** 注册表内的插件记录：后端视图 + 运行时状态 + 性能指标。 */
export interface PluginEntry {
  view: PluginView
  runtime: PluginRuntimeState
  /** 展示名（nameKey 经 i18n 优先）。 */
  displayName: string
  /** 最近一次 activate 耗时（ms）。 */
  lastActivateMs: number | null
  /** 累计激活次数。 */
  activateCount: number
  lastActivatedAt: number | null
}

// ---------------------------------------------------------------------------
// 插件 SDK 接口（createPluginContext 产出的对象，插件的唯一宿主入口）
// ---------------------------------------------------------------------------

/** 插件私有存储（后端按 plugin_id 强制隔离，键名不允许跨插件）。 */
export interface PluginStorage {
  get<T = unknown>(key: string): Promise<T | null>
  set(key: string, value: unknown): Promise<void>
  delete(key: string): Promise<void>
  keys(): Promise<string[]>
}

/** 统一日志：落后端 plugin_logs（管理中心可查），同时 console 带前缀输出。 */
export interface PluginLogger {
  debug(message: string): void
  info(message: string): void
  warn(message: string): void
  error(message: string): void
  /** 审计条目：高危动作（如方法执行）必须走 audit，管理中心的日志页默认展示。 */
  audit(message: string): void
}

/** 通知（复用主程序 Toast；std.notify 权限）。 */
export interface PluginNotify {
  info(message: string, title?: string): void
  success(message: string, title?: string): void
  warn(message: string, title?: string): void
  error(message: string, title?: string): void
}

/** 插件事件总线（插件间与宿主广播；std.events 权限）。 */
export interface PluginEvents {
  emit(event: string, payload?: unknown): void
  on(event: string, handler: (payload: unknown) => void): () => void
}

/** 环境变量合并表只读访问（std.env.read 权限；值含敏感变量，按 medium 风险治理）。 */
export interface PluginEnvAccess {
  /**
   * 当前激活环境 + 项目 + 全局变量的合并结果（与工作区调试页同源）。
   * 异步：宿主按需懒加载 workspace store，不把其拖进首屏链。
   */
  getVariableMap(): Promise<Record<string, string>>
}

/** 受控主程序 API 调用（命令白名单按权限映射；越权直接抛错）。 */
export interface PluginCommands {
  invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>
}

/** 插件配置（configSchema 声明的键值；存插件私有存储 __config）。 */
export interface PluginConfig {
  get(): Record<string, unknown>
  set(values: Record<string, unknown>): Promise<void>
}

/** SDK：导航 / 路由注册（activate 期间注册，deactivate 由宿主统一回收）。 */
export interface PluginUiRegistry {
  nav: {
    register(item: PluginNavItem): void
    unregister(navItemId: string): void
  }
  route: {
    /** 注册路由（path 必须在 manifest.nav 声明集合内，或以 /p/<插件id> 开头）。 */
    register(route: { path: string; name?: string; load: () => Promise<unknown> }): void
    unregister(path: string): void
  }
}

/** 剪贴板写入（std.clipboard.write 权限）。 */
export interface PluginClipboard {
  writeText(text: string): Promise<void>
}

/** 插件激活上下文（SDK 根对象）。activate 返回的实例用于 deactivate 回收。 */
export interface PluginContext {
  readonly manifest: PluginManifest
  readonly storage: PluginStorage
  readonly logger: PluginLogger
  readonly notify: PluginNotify
  readonly events: PluginEvents
  readonly env: PluginEnvAccess
  readonly commands: PluginCommands
  readonly config: PluginConfig
  readonly clipboard: PluginClipboard
  readonly ui: PluginUiRegistry
}

/** 插件模块默认导出的工厂：activate 接收 SDK ctx，返回实例（deactivate 钩子）。 */
export interface PluginInstance {
  deactivate?: () => void | Promise<void>
}

export type PluginFactory = (ctx: PluginContext) => PluginInstance | Promise<PluginInstance>

/** 插件模块形态（builtin loader 的动态 import 产物）。 */
export interface PluginModule {
  default: PluginFactory
}

/** builtin loader 模块表：插件 id → 懒加载工厂（未启用的插件绝不 import）。 */
export type BuiltinModuleMap = Record<PluginId, () => Promise<PluginModule>>
