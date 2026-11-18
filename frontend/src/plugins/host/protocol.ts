/**
 * 第三方插件 RPC 协议（宿主 ↔ 沙箱 iframe）。
 *
 * 传输：`window.postMessage`（沙箱 iframe 的 origin 是 opaque "null"，
 * 因此不做 origin 白名单比对，而是校验 **消息源窗口** === 宿主创建的 iframe
 * contentWindow + 方法白名单 + 参数形状校验）。帧格式：
 *
 *   请求  { v: 1, type: 'req',  id: string, method: string, params?: object }
 *   响应  { v: 1, type: 'res',  id: string, ok: true, result?: unknown }
 *         { v: 1, type: 'res',  id: string, ok: false, error: { code, message } }
 *   事件  { v: 1, type: 'evt',  event: string, payload?: unknown }   （宿主 → 插件）
 *
 * 安全面收口：
 * - 方法名白名单（METHODS）；参数按 methodSchema 校验（类型 / 键集）；
 * - 任何未知帧直接丢弃（不抛、不回错误帧，避免探针枚举）；
 * - 权限检查不在协议层——由 SDK context（granted_permissions）统一强制。
 */

/** 协议版本（破坏性变更递增；与 manifest.sdk_version 对应）。 */
export const RPC_VERSION = 1

/** 请求帧（插件 → 宿主）。 */
export interface RpcRequestFrame {
  v: number
  type: 'req'
  id: string
  method: string
  params?: Record<string, unknown>
}

/** 响应帧（宿主 → 插件）。 */
export interface RpcResponseFrame {
  v: number
  type: 'res'
  id: string
  ok: boolean
  result?: unknown
  error?: { code: string; message: string }
}

/** 事件帧（宿主 → 插件，总线广播转发）。 */
export interface RpcEventFrame {
  v: number
  type: 'evt'
  event: string
  payload?: unknown
}

export type RpcInboundFrame = RpcRequestFrame
export type RpcOutboundFrame = RpcResponseFrame | RpcEventFrame

/** 帧判别（严格形状校验，任何不符即丢弃）。 */
export function isRpcRequestFrame(data: unknown): data is RpcRequestFrame {
  if (typeof data !== 'object' || data === null) return false
  const f = data as Record<string, unknown>
  return (
    f.v === RPC_VERSION &&
    f.type === 'req' &&
    typeof f.id === 'string' &&
    f.id.length > 0 &&
    f.id.length <= 64 &&
    typeof f.method === 'string' &&
    (f.params === undefined || isPlainObject(f.params))
  )
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** 方法白名单（键 → 参数校验器）。 */
export type ParamValidator = (params: Record<string, unknown> | undefined) => string | null

function needString(params: Record<string, unknown> | undefined, key: string, maxLen = 4096): string | null {
  const v = params?.[key]
  if (typeof v !== 'string' || v.length === 0 || v.length > maxLen) return `${key} must be a string (1-${maxLen})`
  return null
}

function optionalString(params: Record<string, unknown> | undefined, key: string, maxLen = 8192): string | null {
  const v = params?.[key]
  if (v === undefined) return null
  if (typeof v !== 'string' || v.length > maxLen) return `${key} must be a string (<= ${maxLen})`
  return null
}

/** RPC 方法目录：宿主暴露给沙箱插件的全部能力面（SDK 权限在执行层强制）。 */
export const METHODS: Record<string, ParamValidator> = {
  'storage.get': (p) => needString(p, 'key', 128),
  'storage.set': (p) => {
    if (!p || typeof p.key !== 'string') return 'key must be a string'
    if (!('value' in p)) return 'value is required'
    try {
      JSON.stringify(p.value)
    } catch {
      return 'value must be JSON-serializable'
    }
    return null
  },
  'storage.delete': (p) => needString(p, 'key', 128),
  'storage.keys': () => null,
  'log.write': (p) => {
    const level = p?.level
    if (level !== 'debug' && level !== 'info' && level !== 'warn' && level !== 'error' && level !== 'audit') {
      return 'level must be debug|info|warn|error|audit'
    }
    return needString(p, 'message', 2000)
  },
  'notify.show': (p) => {
    const kind = p?.kind
    if (kind !== 'info' && kind !== 'success' && kind !== 'warn' && kind !== 'error') {
      return 'kind must be info|success|warn|error'
    }
    return optionalString(p, 'message') ?? needString(p, 'title', 96)
  },
  'config.get': () => null,
  'config.set': (p) => (isPlainObject(p?.values) ? null : 'values must be an object'),
  'env.map': () => null,
  'commands.invoke': (p) => {
    if (!p || typeof p.command !== 'string') return 'command must be a string'
    if (p.args !== undefined && !isPlainObject(p.args)) return 'args must be an object'
    return null
  },
  'events.emit': (p) => needString(p, 'event', 96),
  'events.subscribe': (p) => needString(p, 'event', 96),
}

/** 校验方法是否可用；返回错误信息或 null。 */
export function validateMethod(method: string, params: Record<string, unknown> | undefined): string | null {
  const validator = METHODS[method]
  if (!validator) return `unknown method: ${method}`
  return validator(params)
}

/** 错误码约定（插件侧可按 code 分支）。 */
export const RPC_ERRORS = {
  BAD_REQUEST: 'BAD_REQUEST',
  NOT_FOUND: 'NOT_FOUND',
  PERMISSION_DENIED: 'PERMISSION_DENIED',
  TIMEOUT: 'TIMEOUT',
  INTERNAL: 'INTERNAL',
  /** 宿主熔断 / 并发超限（插件应退避，不要立即重试）。 */
  RATE_LIMITED: 'RATE_LIMITED',
} as const

/**
 * RPC 安全限额（宿主侧强制；插件侧超限收到错误帧或被静默丢帧）。
 * - 整帧字符上限：防单帧拖垮主线程 / 内存（值序列化后度量）；
 * - JSON 深度上限：防深度嵌套的解析放大；
 * - 并发在途上限：防请求洪泛（超出即快速失败）；
 * - 订阅上限：防监听器泄漏；
 * - 熔断：错误率窗口——连续大量失败后暂停服务一段时间（防轰炸 / 防循环）。
 */
export const RPC_LIMITS = {
  /** 整帧 JSON 字符数上限（约 1 MiB）。 */
  MAX_FRAME_CHARS: 1_000_000,
  /** params 最大 JSON 嵌套深度。 */
  MAX_JSON_DEPTH: 16,
  /** 并发在途请求上限（超出立即返回 RATE_LIMITED）。 */
  MAX_INFLIGHT: 20,
  /** 每插件事件订阅上限。 */
  MAX_SUBSCRIPTIONS: 32,
  /** 已见请求 ID 去重表容量（重放防护；超限整表轮空——会话生命周期内足够）。 */
  MAX_SEEN_IDS: 4096,
  /** 熔断阈值：窗口内错误响应数。 */
  BREAKER_THRESHOLD: 50,
  /** 熔断统计窗口。 */
  BREAKER_WINDOW_MS: 30_000,
  /** 熔断打开时长。 */
  BREAKER_OPEN_MS: 60_000,
} as const

/** JSON 值深度（超限返回 true）。 */
export function exceedsJsonDepth(value: unknown, maxDepth: number): boolean {
  if (maxDepth < 0) return true
  if (Array.isArray(value)) {
    return value.some((v) => exceedsJsonDepth(v, maxDepth - 1))
  }
  if (typeof value === 'object' && value !== null) {
    return Object.values(value).some((v) => exceedsJsonDepth(v, maxDepth - 1))
  }
  return false
}
