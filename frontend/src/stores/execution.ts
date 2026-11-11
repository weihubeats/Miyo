/**
 * execution store：请求执行的**按接口 id 分桶**状态。
 *
 * 为什么是 store 不是组件 ref：EndpointEditor 单实例常驻只在 Tab 级成立
 * （路由 /workspace → /projects → /workspace 往返会卸载组件）——组件级
 * ref 会把全部响应、在途请求与计时丢掉；提升到 store 后路由往返不丢
 * （在途请求返回仍按发起时的接口 id 落桶，不错位）。
 *
 * 分桶语义（与 AGENTS.md「EndpointEditor 单实例常驻」规范一致）：
 * - responses / sendErrors / lastRequestIds / grpcResponses：每接口只见
 *   自己的最后一次结果；
 * - sendingMap：逐接口隔离在途态（A 发送中切到 B，B 仍可独立发送）；
 * - 桶淘汰：每桶持全量响应体（后端上限 20MB），超上限按插入序淘汰最旧
 *   （当前接口除外），防长会话内存无限增长。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { ExecuteResponse, GrpcResponse } from '../types/miyoApi'

/** 每桶上限（响应 / 错误 / 最近 request_id 同规格）。 */
export const RESPONSE_BUCKET_MAX = 10

export const useExecutionStore = defineStore('execution', () => {
  /** 接口 id → 在途请求（requestId 用于取消，startedAt 驱动计时）。 */
  const sendingMap = ref<Map<string, { requestId: string; startedAt: number }>>(new Map())
  /** 接口 id → 最近一次 HTTP 响应（null = 上一次失败清空）。 */
  const responses = ref<Map<string, ExecuteResponse | null>>(new Map())
  /** 接口 id → 最近一次发送错误文案（null = 无错）。 */
  const sendErrors = ref<Map<string, string | null>>(new Map())
  /** 接口 id → 最近一次 request_id（响应被 IPC 截断时按它取全文）。 */
  const lastRequestIds = ref<Map<string, string>>(new Map())
  /** 接口 id → 最近一次 gRPC unary 响应（口径同 responses）。 */
  const grpcResponses = ref<Map<string, GrpcResponse | null>>(new Map())

  /** 按插入序淘汰最旧（keepId 除外）。 */
  function evictOldest<T>(map: Map<string, T>, keepId: string): void {
    while (map.size > RESPONSE_BUCKET_MAX) {
      const oldest = map.keys().next().value
      if (oldest === undefined || oldest === keepId) break
      map.delete(oldest)
    }
  }

  return {
    sendingMap,
    responses,
    sendErrors,
    lastRequestIds,
    grpcResponses,
    evictOldest,
  }
})
