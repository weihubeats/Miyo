/**
 * environment.ts：环境相关的前端工具（单一 Base URL / 结构化变量 / URL 拼接）。
 *
 * - envColorClass：按环境名称启发式归类颜色（开发绿 / 测试蓝 / 预发布琥珀 / 生产橙 / 全局紫）；
 * - envBaseUrl：环境声明的 Base URL（解析前的原始文本，保留 `{{变量}}`）；
 * - environmentVariableMap：结构化变量 → 扁平 `{{name}}` 注入表（enabled、本地值优先；
 *   环境 Base URL 自动以 `base_url` 注入，已有同名变量时不覆盖）；
 * - resolveRequestUrl：请求拼接核心 —— 最终 URL = 环境 Base URL + 相对路径；
 * - resolveVariables：镜像 miyo-core variable.rs 的 `{{name}}` 单层递归解析（深度上限 10）。
 */

import type { Endpoint, Environment } from '@/types/miyoApi'
import { extractVarRefs } from '@/utils/scenarioVars'

/** 环境名称 → 颜色类（映射类名，颜色值由调用方 scoped CSS 定义）。 */
export function envColorClass(name: string): string {
  const n = name.trim().toLowerCase()
  if (/(开发|development|dev)/.test(n)) return 'dev'
  if (/(测试|test|qa)/.test(n)) return 'test'
  if (/(预发布|staging|stage|pre)/.test(n)) return 'staging'
  if (/(生产|prod|production|live)/.test(n)) return 'prod'
  if (/(全局|global)/.test(n)) return 'global'
  return ''
}

/** 环境的 Base URL（解析前的原始文本，保留 `{{变量}}`）。 */
export function envBaseUrl(env: Environment | null | undefined): string {
  return env?.base_url?.trim() ?? ''
}

/** 变量生效值（镜像后端 `effective_value` 三态）：显式覆盖开关优先，未标注走旧推断。 */
export function effectiveVariable(v: {
  remote_value: string
  local_value: string
  local_override?: boolean | null
}): string {
  if (v.local_override === true) return v.local_value.trim()
  if (v.local_override === false) return v.remote_value.trim()
  return v.local_value.trim() || v.remote_value.trim()
}

/**
 * 敏感变量名启发式（值列遮蔽用）：key 含 token / secret / password /
 * api_key / access_key / authorization / credential 等词根即视为敏感。
 * 命中面宁窄勿宽——误伤普通变量会让值列莫名打点，比漏遮一个密码更恼人。
 */
const SENSITIVE_KEY_RE =
  /(token|secret|password|passwd|pwd|api[-_]?key|access[-_]?key|private[-_]?key|client[-_]?secret|authorization|credential|session[-_]?id)/i

export function isSensitiveVarKey(key: string | null | undefined): boolean {
  return SENSITIVE_KEY_RE.test(key ?? '')
}

/**
 * 变量敏感判定（镜像后端 `is_var_sensitive`）：显式 `sensitive` 标注优先，
 * 未标注（null / undefined）回退键名启发式。显式「非敏感」可豁免键名误报。
 */
export function resolveSensitive(v: {
  key: string
  sensitive?: boolean | null
}): boolean {
  if (v.sensitive != null) return v.sensitive
  return isSensitiveVarKey(v.key)
}

/**
 * Base URL 格式校验：空值合法（未配置）；其余须以 `http(s)://` 开头或为
 * `{{变量}}` 引用。返回错误信息键名，合法返回 null。
 */
export function validateBaseUrl(value: string): 'envmgr.baseUrlInvalid' | null {
  const s = value.trim()
  if (!s) return null
  if (/^https?:\/\//i.test(s)) return null
  if (s.startsWith('{{')) return null
  return 'envmgr.baseUrlInvalid'
}

/** 变量行（组件新增行时的最小形态，含三态扩展字段缺省）。 */
export interface ParsedVarRow {
  key: string
  remote_value: string
}

/**
 * 批量导入文本解析：JSON 对象 `{k: v}` / 每行 `k=v` / `k: v` 三种形态。
 * 返回解析结果与被跳过的无效行；空 key、`{{`/`$` 开头的行按无效处理
 * （与注入过滤口径一致）。key 重复时后者覆盖前者（保持出现顺序）。
 */
export function parseVarBatch(text: string): {
  rows: ParsedVarRow[]
  invalid: string[]
} {
  const rows: ParsedVarRow[] = []
  const invalid: string[] = []
  const byKey = new Map<string, string>()
  const push = (rawKey: string, rawValue: string): void => {
    const key = rawKey.trim()
    const value = rawValue.trim()
    if (!key || key.startsWith('{{') || key.startsWith('$')) {
      invalid.push(`${rawKey.trim()}=${rawValue.trim()}`)
      return
    }
    if (!byKey.has(key)) rows.push({ key, remote_value: '' })
    byKey.set(key, value)
  }
  const s = text.trim()
  if (s.startsWith('{')) {
    try {
      const obj = JSON.parse(s) as Record<string, unknown>
      for (const [k, v] of Object.entries(obj)) {
        push(k, typeof v === 'string' ? v : String(v ?? ''))
      }
      return { rows: rows.map((r) => ({ ...r, remote_value: byKey.get(r.key) ?? '' })), invalid }
    } catch {
      // 非法 JSON 落入行解析（多数会进 invalid），由调用方提示
    }
  }
  for (const line of s.split(/\r?\n/)) {
    const t = line.trim()
    if (!t) continue
    const m = /^(.*?)[:=](.*)$/.exec(t)
    if (!m || !m[1].trim()) {
      invalid.push(t)
      continue
    }
    push(m[1], m[2])
  }
  return { rows: rows.map((r) => ({ ...r, remote_value: byKey.get(r.key) ?? '' })), invalid }
}

/** 结构化变量列表 → 扁平注入表（enabled 才注入、覆盖三态生效）。 */
export function variableListToMap(
  vars:
    | {
        key: string
        remote_value: string
        local_value: string
        enabled: boolean
        local_override?: boolean | null
      }[]
    | null
    | undefined,
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const v of vars ?? []) {
    if (!v.enabled) continue
    const key = v.key.trim()
    if (!key || key.startsWith('{{') || key.startsWith('$')) continue
    const value = effectiveVariable(v)
    if (value) out[key] = value
  }
  return out
}

/**
 * 结构化变量 → 扁平注入表（enabled 才注入；本地值优先；未知键原样）。
 * 环境 Base URL 自动以 `base_url` 注入（已有同名变量时不覆盖），
 * 使 `{{base_url}}` 在旧语义下继续可用。
 */
export function environmentVariableMap(
  env: Environment | null | undefined,
): Record<string, string> {
  const out = variableListToMap(env?.variables)
  const base = envBaseUrl(env)
  if (base && !out.base_url) out.base_url = base
  return out
}

/** miyo-core variable.rs 的内置变量（{{$seq:key}} 形态属模板写法，不算未定义）。 */
export const BUILTIN_VARIABLES = ['$uuid', '$timestamp', '$isoTimestamp', '$randomInt', '$seq'] as const

/**
 * 调试页合并变量表（单源）：全局 < 项目 < 环境，环境 Base URL 注入 `base_url`。
 *
 * 原先这条合并链在 EndpointEditor / GraphQLView / workspace.caseVariableMap
 * 三处手写展开——优先级规则一旦调整（如新增层级）极易漏改一处。此处收口后，
 * 调试页、GraphQL、测试用例运行与变量补全候选拿到同一张表。
 */
export function mergedVariableMap(
  sources: {
    globalVariables:
      | {
          key: string
          remote_value: string
          local_value: string
          enabled: boolean
          local_override?: boolean | null
        }[]
      | null
      | undefined
    projectVariables: Record<string, string> | null | undefined
    env: Environment | null | undefined
  },
): Record<string, string> {
  return {
    ...variableListToMap(sources.globalVariables),
    ...(sources.projectVariables ?? {}),
    ...environmentVariableMap(sources.env),
  }
}

/** 规范化基础 URL：去掉尾部斜杠，避免与路径拼接出双斜杠（`https://x.com//posts`）。 */
export function normalizeBaseUrl(value: string): string {
  const s = value.trim()
  const stripped = s.replace(/\/+$/, '')
  // 保留协议完整性（避免 `https://` 被削成 `https:`）
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/+$/.test(s)) return s
  return stripped
}

/** 变量递归解析（镜像后端：单次扫描 + 深度上限；未知变量原样保留）。 */
export function resolveVariables(
  input: string,
  vars: Record<string, string>,
  depth = 0,
): string {
  if (depth >= 10) return input
  return input.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (full, name: string) => {
    const value = vars[name]
    if (value == null || value === '') return full
    return resolveVariables(value, vars, depth + 1)
  })
}

function isAbsolute(s: string): boolean {
  return s.startsWith('http://') || s.startsWith('https://')
}

/** 基址 + 相对路径拼接（与 miyo-core util::build_url 一致：完整 URL 直用，否则去斜杠后拼接）。 */
export function joinBaseUrl(base: string, path: string): string {
  if (isAbsolute(path)) return path
  const p = path.trim().replace(/^\/+/, '')
  const b = normalizeBaseUrl(base)
  if (!b) return `/${p}`
  return `${b}/${p}`
}

export interface ResolvedRequestUrl {
  /** 最终请求 URL。 */
  url: string
}

/**
 * 请求拼接核心：
 * 1. `path` 为完整 http(s) 地址 → 直接使用；
 * 2. 否则以环境 Base URL 为基址拼接相对路径（无基址时仅返回路径）。
 * 基址 / 路径中的 `{{变量}}` 以「环境变量 + 调用方变量」合并表解析。
 */
export function resolveRequestUrl(
  env: Environment | null | undefined,
  path: string,
  extraVars: Record<string, string> = {},
): ResolvedRequestUrl {
  const vars = { ...extraVars, ...environmentVariableMap(env) }
  const rendered = resolveVariables(path, vars)
  if (isAbsolute(rendered)) return { url: rendered }
  const base = resolveVariables(envBaseUrl(env), vars)
  return { url: joinBaseUrl(base, rendered) }
}

/**
 * 收集一个请求里参与 {{变量}} 引用扫描的模板文本：URL + 启用中的
 * 头（键与值）/ 参数（键与值）+ Body（json/text 原文、表单值、GraphQL
 * query 与 variables）。发送前未定义变量警示的数据源。
 */
export function requestTemplateTexts(endpoint: Endpoint, url: string): string[] {
  const texts = [url]
  for (const h of endpoint.request.headers) if (h.enabled) texts.push(h.key, h.value)
  for (const p of endpoint.request.params) if (p.enabled) texts.push(p.key, p.value)
  const body = endpoint.request.body
  switch (body.mode) {
    case 'json':
    case 'text':
      texts.push(body.raw)
      break
    case 'urlencoded':
    case 'multipart':
      for (const f of body.fields) if (f.enabled) texts.push(f.value)
      break
    case 'graphql':
      texts.push(body.spec.query, body.spec.variables)
      break
    default:
      break
  }
  return texts
}

/**
 * 找出 texts 中引用了、但不在已知变量池（环境/项目/全局合并表 + 内置变量）
 * 中的 {{变量}} 名（保序去重）。发送前汇总警示用：后端对未知占位原样发出，
 * 「服务端收到 {{token}} 字面量」这类问题应在发送前暴露。
 */
export function findUnknownVars(
  texts: string[],
  known: Iterable<string>,
): string[] {
  const knownNames = new Set(known)
  const out: string[] = []
  for (const text of texts) {
    for (const name of extractVarRefs(text ?? '')) {
      if (!knownNames.has(name) && !out.includes(name)) out.push(name)
    }
  }
  return out
}
