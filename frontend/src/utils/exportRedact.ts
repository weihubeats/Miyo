/**
 * exportRedact：前端导出脱敏单源（P0 安全修复）。
 *
 * 与后端 miyo-sync/redact.rs 同口径：离开本机的数据（场景导出 JSON、
 * .mjs 脚本、运行报告）不得携带真实 Token / 密码 / API Key——敏感键值
 * 替换为占位符，导入方 / 接手者按提示自行回填。自由文本（text body）
 * 无法可靠判定，保守保留不整段替换。
 */
import type { AuthSpec } from '../types/miyoApi'

export const EXPORT_PLACEHOLDER = '__MIYO_REDACTED__'

/** 与 miyo-sync redact.rs `is_sensitive_key` 同口径的敏感键词根。 */
const SENSITIVE_TOKENS = [
  'authorization',
  'cookie',
  'token',
  'secret',
  'password',
  'passwd',
  'pwd',
  'api-key',
  'apikey',
  'api_key',
  'access_key',
  'accesskey',
  'private',
  'credential',
  'sessionid',
  'session_id',
  'x-auth',
  'signature',
  'x-sign',
] as const

/** 键名是否命中敏感词根（大小写不敏感子串匹配）。 */
export function isSensitiveKeyName(key: string): boolean {
  const k = key.toLowerCase()
  return SENSITIVE_TOKENS.some((t) => k.includes(t))
}

/** URL query 脱敏：命中敏感键的参数值替换占位符（access_token 类凭据）。 */
export function redactUrlQuery(url: string): string {
  const qIndex = url.indexOf('?')
  if (qIndex === -1) return url
  const base = url.slice(0, qIndex)
  const query = url.slice(qIndex + 1)
  const redacted = query
    .split('&')
    .map((pair) => {
      const eq = pair.indexOf('=')
      if (eq === -1) return pair
      const k = pair.slice(0, eq)
      const v = pair.slice(eq + 1)
      return v && isSensitiveKeyName(k) ? `${k}=${EXPORT_PLACEHOLDER}` : pair
    })
    .join('&')
  return `${base}?${redacted}`
}

interface KeyValueLite {
  key: string
  value: string
  [k: string]: unknown
}

/** 键值对脱敏（headers / params / path_variables / 表单字段）。 */
export function redactKeyValuePairs<T extends KeyValueLite>(items: T[]): T[] {
  return items.map((item) =>
    isSensitiveKeyName(item.key) && item.value && item.value !== EXPORT_PLACEHOLDER
      ? { ...item, value: EXPORT_PLACEHOLDER }
      : item,
  )
}

/** JSON 文本脱敏（保守策略）：可解析则递归替换敏感键字符串值，否则原文返回。 */
export function redactJsonText(raw: string): string {
  try {
    const value: unknown = JSON.parse(raw)
    return JSON.stringify(redactJsonValue(value))
  } catch {
    return raw
  }
}

function redactJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(redactJsonValue)
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(value)) {
      if (typeof v === 'string' && v && v !== EXPORT_PLACEHOLDER && isSensitiveKeyName(k)) {
        out[k] = EXPORT_PLACEHOLDER
      } else if (typeof v === 'string' && looksLikeJwt(v)) {
        out[k] = EXPORT_PLACEHOLDER
      } else {
        out[k] = redactJsonValue(v)
      }
    }
    return out
  }
  return value
}

/** JWT 形态（三段 base64url）：键名不敏感也脱敏（与后端 looks_like_secret 同口径）。 */
function looksLikeJwt(v: string): boolean {
  return (
    v.split('.').length === 3 &&
    v.length > 40 &&
    v.split('.').every((p) => p && /^[A-Za-z0-9_-]+$/.test(p))
  )
}

/** 认证配置脱敏（Rust `AuthSpec` tag="type" 形状；ApiKey 的 `key` 是头名不掩）。 */
export function redactAuthForExport(auth: AuthSpec): AuthSpec {
  const PH = EXPORT_PLACEHOLDER
  switch (auth.type) {
    case 'bearer':
      return { ...auth, token: PH }
    case 'basic':
      return { ...auth, password: PH }
    case 'apikey':
      return { ...auth, value: PH }
    case 'oauth2':
      return {
        ...auth,
        client_secret: PH,
        // 令牌属个人会话，导出一律清除（undefined = 未授权）。
        token: undefined,
        password: auth.password ? PH : auth.password,
      }
    case 'digest':
      return { ...auth, password: PH }
    case 'hawk':
      return { ...auth, key: PH }
    case 'awsv4':
      return {
        ...auth,
        secret_key: PH,
        session_token: auth.session_token ? PH : auth.session_token,
      }
    case 'hmac':
      return { ...auth, secret_key: PH }
    case 'dynamic_signature':
      return { ...auth, config: { ...auth.config, app_secret: PH } }
    default:
      return auth
  }
}

/** 内联请求 spec（宽形状：兼容旧数据缺字段）。 */
export interface ExportSpec {
  url?: string
  params?: KeyValueLite[]
  headers?: KeyValueLite[]
  path_variables?: KeyValueLite[]
  auth?: AuthSpec | null
  body?:
    | { mode: 'none' }
    | { mode: 'json' | 'text'; raw: string }
    | { mode: 'urlencoded'; fields: KeyValueLite[] }
    | {
        mode: 'multipart'
        fields: { key: string; value: string; value_type?: string; enabled?: boolean }[]
      }
    | { mode: 'graphql'; spec: { variables: string; query: string } }
    | { mode: 'binary' | 'grpc' }
    | Record<string, unknown>
}

/**
 * 内联请求 spec 脱敏（场景导出 / 脚本生成共用）：
 * headers / params / path_variables 键值对 + auth + body（json raw /
 * urlencoded / multipart 文本字段 / graphql variables）+ url query。
 * 返回 [脱敏后的 spec, 被脱敏的字段清单]。
 */
export function redactSpecForExport(
  spec: ExportSpec,
  label: string,
): [ExportSpec, string[]] {
  const masked: string[] = []
  const out: ExportSpec = { ...spec }

  if (typeof out.url === 'string') {
    const redacted = redactUrlQuery(out.url)
    if (redacted !== out.url) {
      out.url = redacted
      masked.push(`${label} URL query`)
    }
  }
  for (const listKey of ['params', 'headers', 'path_variables'] as const) {
    const list = out[listKey]
    if (Array.isArray(list)) {
      const before = JSON.stringify(list)
      const after = redactKeyValuePairs(list)
      if (JSON.stringify(after) !== before) {
        out[listKey] = after
        for (const kv of after) {
          if (kv.value === EXPORT_PLACEHOLDER) masked.push(`${label} ${listKey}.${kv.key}`)
        }
      }
    }
  }
  if (out.auth && typeof out.auth === 'object' && out.auth.type !== 'none') {
    out.auth = redactAuthForExport(out.auth)
    masked.push(`${label} auth`)
  }
  const body = out.body
  if (body && typeof body === 'object' && 'mode' in body) {
    if (body.mode === 'json' && typeof body.raw === 'string') {
      const redacted = redactJsonText(body.raw)
      if (redacted !== body.raw) {
        out.body = { ...body, raw: redacted }
        masked.push(`${label} body(json)`)
      }
    } else if (body.mode === 'urlencoded' && Array.isArray(body.fields)) {
      const after = redactKeyValuePairs(body.fields)
      if (JSON.stringify(after) !== JSON.stringify(body.fields)) {
        out.body = { ...body, fields: after }
        masked.push(`${label} body(urlencoded)`)
      }
    } else if (body.mode === 'multipart' && Array.isArray(body.fields)) {
      let changed = false
      const after = body.fields.map((f) => {
        if ((!f.value_type || f.value_type === 'text') && isSensitiveKeyName(f.key) && f.value) {
          changed = true
          return { ...f, value: EXPORT_PLACEHOLDER }
        }
        return f
      })
      if (changed) {
        out.body = { ...body, fields: after }
        masked.push(`${label} body(multipart)`)
      }
    } else if (body.mode === 'graphql') {
      const gspec = body.spec as { variables: string; query: string } | undefined
      if (gspec && typeof gspec.variables === 'string') {
        const redacted = redactJsonText(gspec.variables)
        if (redacted !== gspec.variables) {
          out.body = { ...body, spec: { ...gspec, variables: redacted } }
          masked.push(`${label} body(graphql)`)
        }
      }
    }
  }
  return [out, masked]
}
