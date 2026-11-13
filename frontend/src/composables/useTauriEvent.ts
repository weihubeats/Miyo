/**
 * useTauriEvent：Tauri 事件订阅收敛（原 8 处各自维护 unlisten 生命周期）。
 *
 * - 组件卸载（scope dispose）自动解绑；
 * - 晚到竞态兜底：`listen` 是异步注册，路由在 IPC 返回前切走时监听器会晚于
 *   onUnmounted 到达——注册完成即检查 disposed，已卸载立即解绑
 *   （原 RealtimeView / WorkspaceView / WorkflowEditor 各自手写的
 *   disposed 标志模式收敛于此）；
 * - 注册失败仅记日志（事件监听失败不影响主流程，与原各站点行为一致）。
 */
import { onScopeDispose } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/**
 * 订阅一个由「subscribe 返回解绑函数」描述的事件源（Tauri listen /
 * onDragDropEvent 等同形 API），scope 卸载自动解绑。
 */
export function useEventSubscription(subscribe: () => Promise<UnlistenFn>): void {
  let unlisten: UnlistenFn | null = null
  let disposed = false
  onScopeDispose(() => {
    disposed = true
    unlisten?.()
    unlisten = null
  })
  void Promise.resolve()
    .then(subscribe)
    .then((off) => {
      if (disposed) {
        off()
        return
      }
      unlisten = off
    })
    .catch((e) => {
      console.error('[useEventSubscription] listen failed:', e)
    })
}

/** 订阅 Tauri 事件（payload 形式，等价 listen<T>(event, (e) => handler(e.payload))）。 */
export function useTauriEvent<T>(event: string, handler: (payload: T) => void): void {
  useEventSubscription(async () => {
    const unlisten = await listen<T>(event, (e) => handler(e.payload))
    return unlisten
  })
}
