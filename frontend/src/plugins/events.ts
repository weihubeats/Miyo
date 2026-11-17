/**
 * 插件事件总线：极简发布订阅（插件间与宿主广播；std.events 权限）。
 *
 * - on 返回解绑函数（与 useTauriEvent 风格一致）；
 * - 插件 deactivate 时由 SDK 统一解绑该插件的全部监听（防泄漏）。
 */
export type PluginEventHandler = (payload: unknown) => void

export class PluginEventBus {
  private handlers = new Map<string, Set<PluginEventHandler>>()

  emit(event: string, payload?: unknown): void {
    const set = this.handlers.get(event)
    if (!set) return
    // 拷贝后遍历：handler 内再订阅 / 退订不破坏本轮遍历。
    for (const handler of [...set]) {
      try {
        handler(payload)
      } catch (err) {
        console.error(`[plugin.events] handler for "${event}" threw:`, err)
      }
    }
  }

  on(event: string, handler: PluginEventHandler): () => void {
    let set = this.handlers.get(event)
    if (!set) {
      set = new Set()
      this.handlers.set(event, set)
    }
    set.add(handler)
    return () => {
      set.delete(handler)
      if (set.size === 0) this.handlers.delete(event)
    }
  }

  /** 注销某插件注册的全部监听（deactivate 回收路径）。 */
  disposeAllOf(disposed: Iterable<PluginEventHandler>): void {
    for (const handler of disposed) {
      for (const [event, set] of this.handlers) {
        set.delete(handler)
        if (set.size === 0) this.handlers.delete(event)
      }
    }
  }

  /** 当前监听总数（测试 / 泄漏检查）。 */
  get size(): number {
    let n = 0
    for (const set of this.handlers.values()) n += set.size
    return n
  }
}

/** 宿主级单例总线（插件间广播共用；插件私有隔离靠 SDK 包一层）。 */
export const pluginEventBus = new PluginEventBus()
