/**
 * testCases.ts：测试用例快照的纯函数（可单测）。
 *
 * - 快照：从请求（RequestSpec + path）提取 URL 路径 / Query 参数 / 请求头 / Body 文本；
 * - 回填：把用例快照写回请求（params / headers / body 按 body_type 还原）。
 */
import type { BodySpec, KeyValue, RequestSpec, TestCase, TestCaseCategory } from '../types/miyoApi'
import { tFallback } from '../stores/locale'
import { deepClone } from './clone'

/** 用例分组（与后端 CATEGORIES 一致）。 */
export const TEST_CASE_CATEGORIES: TestCaseCategory[] = [
  '正向',
  '负向',
  '边界值',
  '安全性',
  '其他',
]

/** 分组主题色（暗色主题下可读）。 */
export const CATEGORY_TONE: Record<TestCaseCategory, string> = {
  正向: 'var(--ok)',
  负向: 'var(--danger)',
  边界值: 'var(--warning)',
  安全性: 'var(--accent)',
  其他: 'var(--text-3)',
}

/** 分组存库原值 → 展示文案键（面板 Tab / 下拉 label 用；原值本身保持中文不变）。 */
const CATEGORY_LABEL_KEYS: Record<TestCaseCategory, string> = {
  正向: 'cases.catPositive',
  负向: 'cases.catNegative',
  边界值: 'cases.catBoundary',
  安全性: 'cases.catSecurity',
  其他: 'cases.catOther',
}

/** 分组展示文案（未知原值原样返回；存储仍写中文原值）。 */
export function caseCategoryLabel(cat: string): string {
  const key = CATEGORY_LABEL_KEYS[cat as TestCaseCategory]
  return key ? tFallback(key) : cat
}

/** body_type 标识 → 展示文案。 */
const BODY_TYPE_LABELS: Record<string, string> = {
  json: 'JSON',
  'form-data': 'Form-Data',
  raw: 'Raw',
  urlencoded: 'x-www-form-urlencoded',
  graphql: 'GraphQL',
  binary: 'Binary',
}

/** HTTP 状态码 → 简短语义文案（未知回退数字本身）。 */
const STATUS_TEXT: Record<number, string> = {
  200: 'OK',
  201: 'Created',
  202: 'Accepted',
  204: 'No Content',
  301: 'Moved Permanently',
  302: 'Found',
  304: 'Not Modified',
  400: 'Bad Request',
  401: 'Unauthorized',
  403: 'Forbidden',
  404: 'Not Found',
  405: 'Method Not Allowed',
  409: 'Conflict',
  422: 'Unprocessable Entity',
  429: 'Too Many Requests',
  500: 'Internal Error',
  502: 'Bad Gateway',
  503: 'Service Unavailable',
  504: 'Gateway Timeout',
}

export function statusTextOf(status: number): string {
  return STATUS_TEXT[status] ?? String(status)
}

/** 状态码 → 语义色调（2xx 绿 / 3xx 蓝 / 4xx 琥珀 / 5xx 玫红）。 */
export type StatusTone = 'ok' | 'warn' | 'err' | 'info'

export function statusToneOf(status: number): StatusTone {
  if (status >= 200 && status < 300) return 'ok'
  if (status >= 400 && status < 500) return 'warn'
  if (status >= 500) return 'err'
  return 'info'
}

/** 耗时格式化：<1s 取整毫秒，≥1s 转秒（2 位小数），空值显示 -。 */
export function formatDuration(ms?: number | null): string {
  if (ms === undefined || ms === null || !Number.isFinite(ms)) return '-'
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(2)}s`
}

export function bodyTypeLabel(t: string): string {
  if (t === 'none') return tFallback('cases.bodyNone')
  return BODY_TYPE_LABELS[t] ?? t
}

/** BodySpec → body_type 标识。 */
export function bodyTypeOf(body: BodySpec): string {
  switch (body.mode) {
    case 'json':
      return 'json'
    case 'text':
      return 'raw'
    case 'multipart':
      return 'form-data'
    case 'urlencoded':
      return 'urlencoded'
    case 'graphql':
      return 'graphql'
    case 'binary':
      return 'binary'
    case 'none':
      return 'none'
    // gRPC 不参与测试用例（本期排除）；快照标识保留区分
    case 'grpc':
      return 'grpc'
  }
}

/** BodySpec → 文本快照（用于落库 / 展示）。 */
export function bodyContentOf(body: BodySpec): string {
  switch (body.mode) {
    case 'json':
    case 'text':
      return body.raw
    case 'urlencoded':
      return JSON.stringify(body.fields)
    case 'multipart':
      return JSON.stringify(body.fields)
    case 'graphql':
      return body.spec.query
    case 'binary':
      return body.path
    case 'none':
      return ''
    case 'grpc':
      return body.spec.message
  }
}

/** 从请求提取用例快照（不含 path：path 由调用方单独传）。 */
export function snapshotRequest(request: RequestSpec): {
  path_variables: KeyValue[]
  params: KeyValue[]
  headers: KeyValue[]
  body_type: string
  body_content: string
} {
  return {
    // 路径变量必须随快照走：用例 url_path 里可能含 `/users/{id}`，
    // 缺了它单跑就只能把字面量 `{id}` 发出去。
    path_variables: deepClone(request.path_variables ?? []),
    params: deepClone(request.params),
    headers: deepClone(request.headers),
    body_type: bodyTypeOf(request.body),
    body_content: bodyContentOf(request.body),
  }
}

/** 把用例快照回填到请求：path_variables / params / headers 整体替换，body 按 body_type 还原。 */
export function applyCaseToRequest(
  request: RequestSpec,
  snapshot: {
    path_variables?: KeyValue[]
    params: KeyValue[]
    headers: KeyValue[]
    body_type: string
    body_content: string
  },
): void {
  request.path_variables = deepClone(snapshot.path_variables ?? [])
  request.params = deepClone(snapshot.params)
  request.headers = deepClone(snapshot.headers)
  request.body = restoreBody(snapshot.body_type, snapshot.body_content)
}

/** body_type + 内容 → BodySpec。JSON 解析失败按 raw 文本降级（保证可回填）。 */
export function restoreBody(bodyType: string, content: string): BodySpec {
  switch (bodyType) {
    case 'json': {
      const raw = content.trim()
      if (!raw) return { mode: 'json', raw: '' }
      return { mode: 'json', raw }
    }
    case 'raw':
      return { mode: 'text', raw: content }
    case 'urlencoded':
      return { mode: 'urlencoded', fields: parseFields(content) }
    case 'form-data':
      return { mode: 'multipart', fields: parseFields(content).map((f) => ({ key: f.key, value: f.value, value_type: 'text' as const, enabled: f.enabled })) }
    case 'graphql':
      return { mode: 'graphql', spec: { query: content, variables: '', operation_name: '' } }
    case 'binary':
      return { mode: 'binary', path: content }
    case 'none':
      return { mode: 'none' }
    case 'grpc':
      return {
        mode: 'grpc',
        spec: { service: '', method: '', message: content, use_tls: false, proto_ids: [] },
      }
    default:
      return { mode: 'text', raw: content }
  }
}

/** 容错解析字段 JSON（损坏时返回空）。 */
function parseFields(json: string): KeyValue[] {
  try {
    const v = JSON.parse(json)
    if (Array.isArray(v)) {
      return (v as KeyValue[]).map((f) => ({
        key: f.key ?? '',
        value: f.value ?? '',
        enabled: f.enabled !== false,
        description: f.description ?? '',
        field_type: f.field_type ?? 'string',
        required: f.required ?? true,
        example: f.example ?? '',
      }))
    }
  } catch {
    /* 忽略损坏快照 */
  }
  return []
}

/**
 * 用例关键字匹配（测试用例面板搜索用）：名称 / 方法 / 路径 /
 * 请求 Body 文本 / 参数与请求头键值均参与匹配（大小写不敏感）。
 * 空关键字恒为 true。
 */
export function matchTestCaseKeyword(c: TestCase, keyword: string): boolean {
  const q = keyword.trim().toLowerCase()
  if (!q) return true
  if (c.name.toLowerCase().includes(q)) return true
  if (c.method.toLowerCase().includes(q)) return true
  if (c.url_path.toLowerCase().includes(q)) return true
  if (c.body_content.toLowerCase().includes(q)) return true
  for (const p of [...c.params, ...c.headers]) {
    if (p.key.toLowerCase().includes(q) || p.value.toLowerCase().includes(q)) return true
  }
  return false
}

/** 相对时间文案（列表「最近执行」列）：刚刚 / N 分钟前 / N 小时前 / N 天前，超过 30 天显示日期。 */
export function relativeRunTime(iso?: string | null): string {
  if (!iso) return ''
  const t = new Date(iso).getTime()
  if (!Number.isFinite(t)) return ''
  const diff = Date.now() - t
  const min = Math.floor(diff / 60_000)
  if (min < 1) return '<1min'
  if (min < 60) return `${min}min`
  const hour = Math.floor(min / 60)
  if (hour < 24) return `${hour}h`
  const day = Math.floor(hour / 24)
  if (day < 30) return `${day}d`
  const d = new Date(t)
  return `${d.getMonth() + 1}/${d.getDate()}`
}
