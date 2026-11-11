/**
 * useRequestExecution：请求执行状态机（自 EndpointEditor 抽出）。
 *
 * 职责（原 EndpointEditor 82-186 + send/sendGrpc/cancelSend 整段，行为不变）：
 * - **按接口 id 分桶**的全部执行态：在途（sendingMap）/ 响应（responses）/
 *   错误（sendErrors）/ 最近 request_id（lastRequestIds）/ gRPC 响应——
 *   分桶 Map 由 stores/execution.ts 持有（路由往返不丢响应），本组合式
 *   只做读写编排；
 * - 桶淘汰（RESPONSE_BUCKET_MAX，按插入序淘汰最旧、当前接口除外）；
 * - 「发送中计时」心跳（0.1s tick，仅在途时推进）；
 * - 发送（HTTP / gRPC）/ 取消 / 响应全文加载 / 新响应 flash 动画锚点。
 *
 * 宿主只保留：草稿对象、URL 构建（buildUrl）与待提交输入提交
 * （commitPendingInput）两个回调——它们读写编辑器局部 UI 状态。
 */
import { computed, nextTick, ref, type ComputedRef, type Ref } from 'vue'
import { storeToRefs } from 'pinia'
import type { useWorkspaceStore } from '../stores/workspace'
import { useExecutionStore } from '../stores/execution'
import type { useMiyoApi } from '../composables/useMiyoApi'
import type { useToast } from '../composables/useToast'
import type { GrpcResponse, Endpoint, ExecuteResponse } from '../types/miyoApi'
import type { GrpcStreamState } from '../stores/workspace'
import type { I18nParams } from '../stores/locale'

export interface UseRequestExecutionOptions {
  /** 当前草稿接口（store.activeEndpoint）。 */
  draft: ComputedRef<Endpoint | null>
  store: ReturnType<typeof useWorkspaceStore>
  api: ReturnType<typeof useMiyoApi>
  toast: ReturnType<typeof useToast>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: I18nParams) => string
  /** 发送前把未提交的输入（正在打的字）写回草稿。 */
  commitPendingInput: () => void
  /** HTTP 请求 URL 构建（编辑器局部状态：Base URL 临时值等）。 */
  buildUrl: () => string
  /** 响应体动画锚点（DOM 归宿主持有，与 useEditorSplitter 的锚点同款）。 */
  flashEl: Ref<HTMLElement | null>
}

export function useRequestExecution(options: UseRequestExecutionOptions) {
  const { draft, store, api, toast, t, commitPendingInput, buildUrl, flashEl } = options

  // 分桶状态：store 持有（路由往返不丢），storeToRefs 保持 Ref 用法不变。
  const execution = useExecutionStore()
  const { sendingMap, responses, sendErrors, lastRequestIds, grpcResponses } =
    storeToRefs(execution)
  const evictOldest = execution.evictOldest

  /** 当前接口是否在途（逐接口隔离：A 发送中切到 B，B 仍可独立发送）。 */
  const sending = computed(() => (draft.value ? sendingMap.value.has(draft.value.id) : false))
  /** 在途请求的取消标识（非空表示有请求可取消）。 */
  const activeRequestId = computed(() =>
    draft.value ? (sendingMap.value.get(draft.value.id)?.requestId ?? null) : null,
  )
  /** 驱动「发送中计时」重绘的心跳（Apifox 式实时计时，仅在途时推进）。 */
  const elapsedTick = ref(0)
  let elapsedTimer: ReturnType<typeof setInterval> | undefined

  function ensureElapsedTimer(): void {
    if (elapsedTimer) return
    elapsedTimer = setInterval(() => {
      if (sendingMap.value.size === 0) {
        clearInterval(elapsedTimer)
        elapsedTimer = undefined
        return
      }
      elapsedTick.value += 1
    }, 100)
  }

  /** 当前在途耗时（ms），靠 elapsedTick 心跳刷新。 */
  const elapsedMs = computed(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    elapsedTick.value
    if (!draft.value) return 0
    const f = sendingMap.value.get(draft.value.id)
    return f ? Math.max(0, Date.now() - f.startedAt) : 0
  })
  /** 发送中实时计时文案（Apifox 式：0.0s 起跳，持续跳动）。 */
  const elapsedText = computed(() => `${(elapsedMs.value / 1000).toFixed(1)}s`)
  function triggerFlash(): void {
    const el = flashEl.value
    if (!el) return
    el.classList.remove('flash')
    void el.offsetWidth
    el.classList.add('flash')
  }
  /**
   * 各接口的请求结果按 id 分桶（Map 在 stores/execution.ts）：切到另一个
   * 接口只会看到自己的响应；请求在途切走再返回也不错位；路由往返（离开
   * /workspace 再回来）响应与在途态保留。
   */
  const response = computed<ExecuteResponse | null>(() =>
    draft.value ? (responses.value.get(draft.value.id) ?? null) : null,
  )
  const sendError = computed<string | null>(() =>
    draft.value ? (sendErrors.value.get(draft.value.id) ?? null) : null,
  )

  /** ResponsePanel 全文加载器：按当前接口最近一次 request_id 取被截断的完整正文。 */
  async function loadFullBody(): Promise<string | null> {
    const id = draft.value?.id
    const rid = id ? lastRequestIds.value.get(id) : undefined
    return rid ? api.getResponseBody(rid) : null
  }

  /** gRPC 响应分桶（stores/execution.ts 持有，口径同 responses），这里只读派生。 */
  const grpcResponse = computed<GrpcResponse | null>(() =>
    draft.value ? (grpcResponses.value.get(draft.value.id) ?? null) : null,
  )
  /** 当前接口最近一次的服务端流状态（store 事件通道维护）。 */
  const grpcStream = computed<GrpcStreamState | null>(() =>
    draft.value ? store.grpcStreamOf(draft.value.id) : null,
  )
  const isGrpc = computed(() => draft.value?.method === 'GRPC')
  /** 响应区展示口径：运行中的流优先；否则最近 unary；否则已结束的流。 */
  const grpcDisplay = computed<{ response: GrpcResponse | null; stream: GrpcStreamState | null }>(() => {
    const st = grpcStream.value
    if (st && st.status === 'running') return { response: null, stream: st }
    if (grpcResponse.value) return { response: grpcResponse.value, stream: null }
    return { response: null, stream: st }
  })

  /** 正在发送或已有结果时展示响应区（发送中显示请求中占位，见模板）。 */
  const hasResponse = computed(
    () =>
      sending.value ||
      !!response.value ||
      !!sendError.value ||
      !!grpcResponse.value ||
      !!grpcStream.value,
  )

  /** 发送 HTTP 请求：结果按发起时的接口 id 落桶（在途切走再返回不错位）。 */
  async function send(): Promise<void> {
    if (!draft.value) return
    commitPendingInput()
    const targetId = draft.value.id
    if (sendingMap.value.has(targetId)) {
      toast.info(t('editor.sendingHint'))
      return
    }
    if (draft.value.method === 'GRPC') return sendGrpc()
    const snapshot = draft.value
    sendErrors.value.set(targetId, null)
    evictOldest(sendErrors.value, targetId)
    const url = buildUrl()
    const rid = crypto.randomUUID()
    lastRequestIds.value.set(targetId, rid)
    evictOldest(lastRequestIds.value, targetId)
    sendingMap.value.set(targetId, { requestId: rid, startedAt: Date.now() })
    ensureElapsedTimer()
    try {
      const resp = await store.send(snapshot, url, rid)
      // 结果按发起请求时的接口 id 落桶：请求在途时切走再返回，不会错位。
      responses.value.set(targetId, resp)
      evictOldest(responses.value, targetId)
      sendErrors.value.set(targetId, null)
      evictOldest(sendErrors.value, targetId)
      triggerFlash()
      // 历史已迁至侧栏「请求历史」页签，发送后由 store 统一刷新。
      void store.loadHistories()
    } catch (err) {
      const e = err as Error & { code?: string }
      if (e?.code === 'CANCELLED') {
        // 用户主动取消：不视为错误，保留上一次结果。
        toast.info(t('editor.cancelled'))
        sendErrors.value.set(targetId, null)
      } else {
        sendErrors.value.set(targetId, err instanceof Error ? err.message : String(err))
        evictOldest(sendErrors.value, targetId)
        responses.value.set(targetId, null)
        evictOldest(responses.value, targetId)
        triggerFlash()
      }
    } finally {
      sendingMap.value.delete(targetId)
      // 等响应分支挂载后再重启动画：完成瞬间仍是请求中占位，flashEl 为空。
      await nextTick()
      triggerFlash()
    }
  }

  /**
   * gRPC 发送：unary 直返响应入桶；服务端流注册运行态（消息经事件异步追加），
   * 发送按钮随即复位——流的关闭由响应区按钮控制。
   */
  async function sendGrpc(): Promise<void> {
    const d = draft.value
    if (!d) return
    const targetId = d.id
    if (sendingMap.value.has(targetId)) {
      toast.info(t('editor.sendingHint'))
      return
    }
    sendErrors.value.set(targetId, null)
    const rid = crypto.randomUUID()
    sendingMap.value.set(targetId, { requestId: rid, startedAt: Date.now() })
    ensureElapsedTimer()
    try {
      const result = await store.sendGrpc(d, rid)
      if (result.kind === 'unary') {
        grpcResponses.value.set(targetId, result.response)
        evictOldest(grpcResponses.value, targetId)
        void store.loadHistories()
      } else {
        // 服务端流：清掉旧 unary 响应，让流式时间线接管响应区
        grpcResponses.value.set(targetId, null)
        evictOldest(grpcResponses.value, targetId)
      }
      sendErrors.value.set(targetId, null)
      triggerFlash()
    } catch (err) {
      const e = err as Error & { code?: string }
      if (e?.code === 'CANCELLED') {
        toast.info(t('editor.cancelled'))
        sendErrors.value.set(targetId, null)
      } else {
        sendErrors.value.set(targetId, err instanceof Error ? err.message : String(err))
        grpcResponses.value.set(targetId, null)
        evictOldest(grpcResponses.value, targetId)
        triggerFlash()
      }
    } finally {
      sendingMap.value.delete(targetId)
      await nextTick()
      triggerFlash()
    }
  }

  /** 取消在途请求（后端中止连接，命令随即以 CANCELLED 返回）。 */
  function cancelSend(): void {
    const rid = activeRequestId.value
    if (!rid) return
    void api.cancelRequest(rid)
    toast.info(t('editor.cancelling'))
  }

  /** 卸载清理：停止计时心跳（宿主 onUnmounted 调用）。 */
  function dispose(): void {
    if (elapsedTimer) {
      clearInterval(elapsedTimer)
      elapsedTimer = undefined
    }
  }

  return {
    sending,
    activeRequestId,
    elapsedMs,
    elapsedText,
    dispose,
    lastRequestIds,
    responses,
    sendErrors,
    response,
    sendError,
    grpcResponse,
    grpcStream,
    isGrpc,
    grpcDisplay,
    hasResponse,
    send,
    sendGrpc,
    cancelSend,
    loadFullBody,
    triggerFlash,
  }
}
