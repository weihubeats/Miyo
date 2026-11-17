/**
 * HostRpcBridge：宿主侧 RPC 桥——把沙箱 iframe 的 RPC 请求分发到插件
 * SDK 会话（createPluginSession 的 context），把总线事件推回订阅方。
 *
 * 安全模型（为何 postMessage 桥 + sandbox iframe 是真实隔离）：
 * - iframe 用 `sandbox="allow-scripts"`（**不带** allow-same-origin）：插件
 *   脚本运行在 opaque origin，无 DOM / localStorage / cookie / 父窗口访问；
 * - 唯一通道是 postMessage；桥只接受「消息源 === 本宿主 iframe 的
 *   contentWindow」的帧，其他窗口的帧一律忽略；
 * - 方法白名单 + 参数校验在 protocol.ts；权限在 SDK context（用户批准快照）；
 * - 插件 UI 不经 vue-router / pinia，不 import 主程序模块。
 */
import { pluginEventBus, type PluginEventHandler } from '@/plugins/events'
import {
  RPC_ERRORS,
  RPC_VERSION,
  isRpcRequestFrame,
  validateMethod,
  type RpcOutboundFrame,
} from './protocol'
import type { PluginSession } from '@/plugins/sdk'

/** 请求超时（宿主侧等待 SDK 完成；SDK 内部 IPC 自带错误通道）。 */
export const REQUEST_TIMEOUT_MS = 15_000

export interface BridgeOptions {
  /** 消息源校验：仅接受该窗口发来的帧。 */
  sourceWindow: Window
  /** 宿主 → 插件出帧通道（通常是 iframe.contentWindow.postMessage）。 */
  postMessage: (frame: RpcOutboundFrame) => void
  /** 插件 SDK 会话（权限 / 存储 / 日志 / 通知的执行层）。 */
  session: PluginSession
  /** 请求超时（测试可缩短）。 */
  timeoutMs?: number
}

export class HostRpcBridge {
  /** 消息源窗口（宿主转发模式下由宿主按 event.source 过滤；直连模式用于桥内校验）。 */
  private readonly sourceWindow: Window
  private readonly post: (frame: RpcOutboundFrame) => void
  private readonly session: PluginSession
  private readonly timeoutMs: number
  /** 已订阅总线事件的插件监听（dispose 统一回收）。 */
  private busHandlers = new Set<PluginEventHandler>()
  private disposed = false

  constructor(opts: BridgeOptions) {
    this.sourceWindow = opts.sourceWindow
    this.post = opts.postMessage
    this.session = opts.session
    this.timeoutMs = opts.timeoutMs ?? REQUEST_TIMEOUT_MS
  }

  /** 消息源（直连模式下用于丢弃非宿主 iframe 的帧）。 */
  get acceptsFrom(): Window {
    return this.sourceWindow
  }

  /** 处理一条来自 iframe 的 message 事件（PluginIframeHost 转发）。 */
  handleMessage(data: unknown): void {
    if (this.disposed) return
    if (!isRpcRequestFrame(data)) return // 未知帧静默丢弃
    const { id, method, params } = data
    const invalid = validateMethod(method, params)
    if (invalid) {
      this.fail(id, RPC_ERRORS.BAD_REQUEST, invalid)
      return
    }
    // 超时兜底：SDK 调用挂起时不让插件侧永久等待。
    const timer = setTimeout(() => this.fail(id, RPC_ERRORS.TIMEOUT, 'host request timeout'), this.timeoutMs)
    void this.dispatch(method, params)
      .then((result) => {
        clearTimeout(timer)
        this.post({ v: RPC_VERSION, type: 'res', id, ok: true, result })
      })
      .catch((err: unknown) => {
        clearTimeout(timer)
        const code = err instanceof Error && err.message.startsWith('PLUGIN_') ? RPC_ERRORS.PERMISSION_DENIED : RPC_ERRORS.INTERNAL
        this.fail(id, code, err instanceof Error ? err.message : String(err))
      })
  }

  /** 订阅总线事件（宿主监听 → 插件收 evt 帧）。 */
  subscribe(event: string): void {
    if (this.disposed) return
    const prefixed = `${this.session.context.manifest.id}:${event}`
    const handler: PluginEventHandler = (payload) => {
      this.post({ v: RPC_VERSION, type: 'evt', event, payload })
    }
    pluginEventBus.on(prefixed, handler)
    this.busHandlers.add(handler)
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    this.session.dispose()
    pluginEventBus.disposeAllOf(this.busHandlers)
    this.busHandlers.clear()
  }

  private fail(id: string, code: string, message: string): void {
    this.post({ v: RPC_VERSION, type: 'res', id, ok: false, error: { code, message } })
  }

  /** 方法分发（SDK context 执行；权限在 context 内强制）。 */
  private async dispatch(method: string, params: Record<string, unknown> | undefined): Promise<unknown> {
    const ctx = this.session.context
    switch (method) {
      case 'storage.get':
        return ctx.storage.get(params!.key as string)
      case 'storage.set':
        await ctx.storage.set(params!.key as string, params!.value)
        return null
      case 'storage.delete':
        await ctx.storage.delete(params!.key as string)
        return null
      case 'storage.keys':
        return ctx.storage.keys()
      case 'log.write':
        this.log(params!.level as string, params!.message as string)
        return null
      case 'notify.show': {
        const kind = params!.kind as string
        const title = params!.title as string | undefined
        const message = params!.message as string | undefined
        if (kind === 'success') ctx.notify.success(message ?? '', title)
        else if (kind === 'warn') ctx.notify.warn(message ?? '', title)
        else if (kind === 'error') ctx.notify.error(message ?? '', title)
        else ctx.notify.info(message ?? '', title)
        return null
      }
      case 'config.get':
        return ctx.config.get()
      case 'config.set':
        await ctx.config.set(params!.values as Record<string, unknown>)
        return null
      case 'env.map':
        return ctx.env.getVariableMap()
      case 'commands.invoke':
        return ctx.commands.invoke(params!.command as string, params!.args as Record<string, unknown>)
      case 'events.emit':
        ctx.events.emit(params!.event as string, params!.payload)
        return null
      case 'events.subscribe':
        this.subscribe(params!.event as string)
        return null
      default:
        // validateMethod 已挡住未知方法；此处兜底保持穷尽性
        throw new Error(`${RPC_ERRORS.NOT_FOUND}: ${method}`)
    }
  }

  private log(level: string, message: string): void {
    const logger = this.session.context.logger
    if (level === 'error') logger.error(message)
    else if (level === 'warn') logger.warn(message)
    else if (level === 'audit') logger.audit(message)
    else if (level === 'debug') logger.debug(message)
    else logger.info(message)
  }
}
