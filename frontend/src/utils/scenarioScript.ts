/**
 * 场景 → 可运行脚本（Node 18+ 原生 fetch）生成器。
 *
 * 只读预览与 CI 导出用：手动编排是唯一编辑真源，脚本由步骤实时生成，
 * 与编辑内容始终一致。链路变量传递对应到 JS：前一步提取的变量在后续
 * 步骤的 URL / 断言期望中直接引用。
 *
 * 与真实执行的一致性口径（对齐 miyo-http 发送路径 / miyo-test runner）：
 * - URL：模板解析 {{var}}，启用的查询参数按 append_query 语义追加（已有 ? 用 & 续）；
 * - Auth：Bearer / Basic / ApiKey(header|query) 可静态映射为头或参数；
 *   OAuth2 / DynamicSignature / Digest / Hawk / AWS V4 / HMAC 需运行时
 *   签名或浏览器流，生成「不支持」注释明确报告，不静默丢失；
 * - Body：json / text / urlencoded / multipart(文本) / graphql 可还原；
 *   binary / grpc 生成明确注释（文件与 gRPC 无法内嵌进自包含脚本）；
 * - 环境变量：脚本内引用的 {{var}}（未被前步提取的）声明为
 *   `process.env.MIYO_VAR_*`；非敏感名可内嵌当前环境值作默认，
 *   敏感名（见 isSensitiveVarName）一律引用环境变量，不把本机真实
 *   密钥写进导出脚本。
 */
import type { ScenarioAssertionRule, ScenarioExtractRule } from '../types/miyoApi'
import { isSensitiveVarName } from './scenarioVars'
export { isSensitiveVarName }

/** 生成脚本的步骤输入（从编辑态物化：只带启用行）。 */
export interface ScriptStep {
  /** 步骤名（接口名）。 */
  name: string
  method: string
  /** URL 模板（含 {{base_url}} / {{var}} 占位；路径变量已物化进模板）。 */
  url: string
  /** 启用中的查询参数（发送时 append 到 URL）。缺省 []。 */
  params?: { key: string; value: string }[]
  /** 启用中的请求头。 */
  headers: { key: string; value: string }[]
  /** 可静态映射的鉴权（Bearer / Basic / ApiKey）；null = 无或不可映射。 */
  auth?: ScriptAuth | null
  /** 不可静态映射的鉴权方式名（OAuth2 / Hawk …），生成明确注释。 */
  authUnsupported?: string | null
  /** json / text / graphql-query 原文；其余 Body 类型为 null。 */
  body: string | null
  /** urlencoded / multipart 的启用文本字段。缺省 []。 */
  bodyFields?: { key: string; value: string }[]
  /** multipart 文件字段名（无法内嵌本地文件，生成注释）。缺省 []。 */
  bodyFileFields?: string[]
  /** graphql 变量 JSON 文本（空 = 无）。缺省 ''。 */
  bodyVariables?: string
  /** binary 文件路径（生成读取文件的注释行）。缺省 ''。 */
  bodyPath?: string
  bodyKind: 'json' | 'text' | 'urlencoded' | 'multipart' | 'graphql' | 'binary' | 'grpc' | 'none' | 'other'
  extract: ScenarioExtractRule[]
  assertions: ScenarioAssertionRule[]
}

/** 可静态映射到脚本的头 / 参数的鉴权子集。 */
export interface ScriptAuth {
  kind: 'bearer' | 'basic' | 'apikey_header' | 'apikey_query'
  /** bearer: token 模板。 */
  token?: string
  /** basic。 */
  username?: string
  password?: string
  /** apikey：键名 + 值模板。 */
  key?: string
  value?: string
}

/** 简单 JSONPath → JS 可选链（$.a.b / $[0].k）；复杂路径返回 null 交由注释说明。 */
export function jsonPathToJs(path: string): string | null {
  if (!path.startsWith('$')) return null
  const rest = path.slice(1)
  // `..`（递归下降）与过滤表达式不在可选链表达范围内。
  if (!/^[.[\]a-zA-Z0-9_-]*$/.test(rest) || rest.includes('..')) return null
  const parts: string[] = []
  const re = /\.([a-zA-Z_][a-zA-Z0-9_-]*)|\[(\d+)\]/g
  let m: RegExpExecArray | null
  while ((m = re.exec(rest))) {
    parts.push(m[1] ? `?.${m[1]}` : `?.[${m[2]}]`)
  }
  if (!parts.length) return rest === '' ? 'body' : null
  return `body${parts.join('')}`
}

/** 期望值字面量：数字 / 布尔裸写；含 {{var}} 的字符串转模板串插值；其余 JSON 引号。 */
function literal(v: unknown): string {
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  const s = v == null ? '' : String(v)
  if (s.includes('{{')) return urlTemplateToJs(s)
  return JSON.stringify(s)
}

/** 断言 → check(...) 一行。 */
function assertLine(a: ScenarioAssertionRule, actual: string): string {
  const label = a.name?.trim() || `${a.type}${a.path ? ` ${a.path}` : ''} ${a.op ?? 'eq'} ${a.expected ?? ''}`.trim()
  // label 用模板串：链路变量在运行时插出解析值，失败信息更可读。
  const labelArg = a.name?.trim() ? JSON.stringify(label) : urlTemplateToJs(label)
  const call = (expr: string): string => `check(${labelArg}, ${expr});`
  switch (a.op) {
    case 'exists':
      return call(`${actual} != null`)
    case 'not_exists':
      return call(`${actual} == null`)
    case 'empty':
      return call(
        `${actual} == null || ${actual} === '' || (Array.isArray(${actual}) && ${actual}.length === 0)`,
      )
    case 'not_empty':
      return call(`${actual} != null && ${actual} !== ''`)
    case 'contains':
      return call(`String(${actual}).includes(${literal(a.expected)})`)
    case 'not_contains':
      return call(`!String(${actual}).includes(${literal(a.expected)})`)
    case 'gt':
      return call(`Number(${actual}) > ${literal(a.expected)}`)
    case 'gte':
      return call(`Number(${actual}) >= ${literal(a.expected)}`)
    case 'lt':
      return call(`Number(${actual}) < ${literal(a.expected)}`)
    case 'lte':
      return call(`Number(${actual}) <= ${literal(a.expected)}`)
    default:
      return call(`${actual} === ${literal(a.expected)}`)
  }
}

/** URL 模板 → JS 模板串：{{base_url}} → ${BASE}，{{var}} → ${var}。 */
export function urlTemplateToJs(url: string): string {
  // 先转义输入中的字面量（\\、`、${），再引入真正的插值——顺序反了会把
  // 刚生成的 ${BASE} 也转义掉。
  const escaped = url.replace(/\\/g, '\\\\').replace(/`/g, '\\`').replace(/\$\{/g, '\\${')
  const js = escaped
    .replace(/\{\{base_url\}\}/g, '${BASE}')
    .replace(/\{\{\s*([^}]+?)\s*\}\}/g, '${$1}')
  return '`' + js + '`'
}

/** 头 / 参数值模板：含 {{var}} 转模板串插值；纯字面量保持普通字符串。 */
function valueExpr(v: string): string {
  return v.includes('{{') ? urlTemplateToJs(v) : JSON.stringify(v)
}

const SAMPLE_BODY_LIMIT = 60

/** 收集模板文本中引用的全部 {{var}} 名（含 base_url）。 */
function collectVars(text: string, into: Set<string>): void {
  for (const m of text.matchAll(/\{\{\s*([^{}]+?)\s*\}\}/g)) {
    into.add(m[1])
  }
}

/** 步骤内所有可能携带 {{var}} 的文本（供变量收集）。 */
function stepTemplateTexts(s: ScriptStep): string[] {
  const texts = [s.url]
  for (const p of s.params ?? []) texts.push(p.value)
  for (const h of s.headers) texts.push(h.key, h.value)
  if (s.auth) {
    if (s.auth.token) texts.push(s.auth.token)
    if (s.auth.username) texts.push(s.auth.username)
    if (s.auth.password) texts.push(s.auth.password)
    if (s.auth.value) texts.push(s.auth.value)
  }
  if (s.body) texts.push(s.body)
  for (const f of s.bodyFields ?? []) texts.push(f.value)
  if (s.bodyVariables) texts.push(s.bodyVariables)
  for (const a of s.assertions) {
    if (typeof a.expected === 'string') texts.push(a.expected)
  }
  return texts
}

/** 生成未提取变量的声明行：敏感名只引用环境变量；非敏感名可内嵌默认值。 */
function varDeclarationLines(
  names: string[],
  envDefaults: Record<string, string>,
): { lines: string[]; requiredEnv: string[] } {
  const lines: string[] = []
  const requiredEnv: string[] = []
  for (const raw of names) {
    const name = raw.trim()
    if (!name || name === 'base_url') continue
    const varName = name.replace(/[^a-zA-Z0-9_$]/g, '_')
    const envKey = `MIYO_VAR_${name.replace(/[^a-zA-Z0-9]/g, '_').toUpperCase()}`
    if (isSensitiveVarName(name)) {
      lines.push(
        `var ${varName} = process.env.${envKey} ?? ''; // 敏感变量：仅从环境变量注入，不写入脚本`,
      )
      requiredEnv.push(envKey)
    } else {
      const def = envDefaults[name]
      lines.push(
        def != null
          ? `var ${varName} = process.env.${envKey} ?? ${JSON.stringify(def)};`
          : `var ${varName} = process.env.${envKey} ?? '';`,
      )
      if (def == null) requiredEnv.push(envKey)
    }
  }
  return { lines, requiredEnv }
}

/**
 * 生成可运行脚本。`base` 为当前环境的 Base URL 兜底值（脚本内仍可用
 * BASE_URL 环境变量覆盖）；`envDefaults` 为当前变量表（环境 > 项目 > 全局
 * 合并结果），仅非敏感名会被内嵌为默认值。
 */
export function generateScenarioScript(
  scenarioName: string,
  steps: ScriptStep[],
  opts: { base: string; stopOnFailure: boolean; envDefaults?: Record<string, string> },
): string {
  const envDefaults = opts.envDefaults ?? {}
  // 兼容未携带新增字段的调用方（旧测试 / 外部用法）：缺省视为无。
  steps = steps.map((s) => ({
    ...s,
    params: s.params ?? [],
    bodyFields: s.bodyFields ?? [],
    bodyFileFields: s.bodyFileFields ?? [],
    bodyVariables: s.bodyVariables ?? '',
    bodyPath: s.bodyPath ?? '',
  }))
  const out: string[] = []
  out.push('/**')
  out.push(` * 场景「${scenarioName}」— 由 Miyo 生成，可直接运行：node scenario.mjs`)
  out.push(' * 运行环境：Node 18+（原生 fetch）。BASE_URL 环境变量可覆盖服务地址。')
  out.push(' * 链路变量：前一步提取的值在后续步骤的 URL / 断言期望中直接引用。')
  if (opts.stopOnFailure) out.push(' * 失败即停：任一断言不过则退出（MIYO_STOP_ON_FAILURE=0 关闭）。')
  out.push(' */')
  if (steps.some((s) => s.bodyKind === 'binary')) {
    out.push("import { readFileSync } from 'node:fs';")
  }
  out.push(`const BASE = process.env.BASE_URL ?? ${JSON.stringify(opts.base)};`)
  out.push('')

  // 未被前步提取的 {{var}} → 声明区（敏感名仅环境变量；否则可内嵌默认）。
  const extracted = new Set<string>()
  for (const s of steps) {
    for (const e of s.extract) {
      const name = e.name.trim()
      if (name) extracted.add(name)
    }
  }
  const referenced = new Set<string>()
  for (const s of steps) {
    for (const t of stepTemplateTexts(s)) collectVars(t, referenced)
  }
  const undeclared = [...referenced].filter((n) => !extracted.has(n))
  const { lines: varLines, requiredEnv } = varDeclarationLines(undeclared, envDefaults)
  if (varLines.length) {
    out.push('// ---- 环境变量声明（Miyo 中由环境/项目/全局变量在运行时解析）----')
    out.push(...varLines)
    out.push('')
  }
  if (requiredEnv.length) {
    out.push(
      `// 提示：以下环境变量未提供时为空值，请按需设置 —— ${requiredEnv.join(', ')}`,
    )
    out.push('')
  }

  out.push('const results = [];')
  out.push('function check(label, ok, extra) {')
  out.push('  results.push({ label, ok });')
  out.push("  console.log(`${ok ? '✓' : '✗'} ${label}${extra ? ' — ' + extra : ''}`);")
  if (opts.stopOnFailure) {
    out.push('  if (!ok && process.env.MIYO_STOP_ON_FAILURE !== \'0\') {')
    out.push("    console.error('失败即停：' + label);")
    out.push('    process.exit(1);')
    out.push('  }')
  }
  out.push('}')
  out.push('')

  steps.forEach((s, i) => {
    const n = i + 1
    out.push(`// ---- 步骤 ${n}：${s.name}  ${s.method} ${urlTemplateToJs(s.url)} ----`)
    out.push(`let t${n} = Date.now();`)
    out.push(`let url${n} = ${urlTemplateToJs(s.url)};`)
    // 启用中的查询参数：与发送路径 append_query 同序追加（已有 ? 用 & 续）。
    const params = s.params ?? []
    if (params.length) {
      out.push(`url${n} += (url${n}.includes('?') ? '&' : '?') + new URLSearchParams([`)
      for (const p of params) {
        out.push(`  [${JSON.stringify(p.key)}, ${valueExpr(p.value)}],`)
      }
      out.push(']).toString();')
    }

    // 鉴权 → 头 / 参数（与 apply_auth 同语义）。expr = value 已是 JS 表达式
    //（Basic 需运行时 btoa，不能再经 valueExpr 字符串化）。
    const jsonish = s.bodyKind === 'json' || s.bodyKind === 'graphql'
    const headers: { key: string; value: string; expr?: boolean }[] = jsonish
      ? s.headers.some((h) => h.key.toLowerCase() === 'content-type')
        ? s.headers.map((h) => ({ ...h }))
        : [...s.headers.map((h) => ({ ...h })), { key: 'Content-Type', value: 'application/json' }]
      : s.headers.map((h) => ({ ...h }))
    if (s.auth?.kind === 'bearer') {
      headers.push({ key: 'Authorization', value: `Bearer ${s.auth.token}` })
    } else if (s.auth?.kind === 'basic') {
      headers.push({
        key: 'Authorization',
        value: `'Basic ' + btoa(${valueExpr(s.auth.username ?? '')} + ':' + ${valueExpr(s.auth.password ?? '')})`,
        expr: true,
      })
    } else if (s.auth?.kind === 'apikey_header') {
      headers.push({ key: s.auth.key ?? '', value: s.auth.value ?? '' })
    }
    if (s.auth?.kind === 'apikey_query') {
      out.push(`url${n} += (url${n}.includes('?') ? '&' : '?') + new URLSearchParams([`)
      out.push(`  [${JSON.stringify(s.auth.key ?? '')}, ${valueExpr(s.auth.value ?? '')}],`)
      out.push(']).toString();')
    }

    // body 表达式（按类型）；null = 无 body。
    let bodyExpr: string | null = null
    switch (s.bodyKind) {
      case 'json':
        bodyExpr = s.body != null ? JSON.stringify(s.body) : null
        break
      case 'text':
        bodyExpr = s.body != null ? JSON.stringify(s.body) : null
        break
      case 'graphql': {
        // variables 文本原样内嵌：调用方用 JSON.parse 在运行时校验（坏 JSON 抛错可见）。
        const varsText = s.bodyVariables ?? ''
        const varsArg = varsText.trim() ? `JSON.parse(${JSON.stringify(varsText)})` : '{}'
        bodyExpr = `JSON.stringify({ query: ${JSON.stringify(s.body ?? '')}, variables: ${varsArg} })`
        break
      }
      case 'urlencoded':
        const fields = s.bodyFields ?? []
        bodyExpr = fields.length
          ? `new URLSearchParams([${fields.map((f) => `[${JSON.stringify(f.key)}, ${valueExpr(f.value)}]`).join(', ')}]).toString()`
          : null
        break
      case 'multipart':
        bodyExpr = null // 以 FormData 形式单独生成（fetch 自动设 Content-Type boundary）
        break
      case 'binary':
      case 'grpc':
      case 'none':
      case 'other':
        bodyExpr = null
        break
    }

    const hasHeaders = headers.length > 0
    out.push(`let res${n} = await fetch(url${n}, {`)
    out.push(`  method: ${JSON.stringify(s.method)},`)
    if (hasHeaders) {
      out.push('  headers: {')
      for (const h of headers) {
        const v = h.expr ? h.value : valueExpr(h.value)
        out.push(`    ${JSON.stringify(h.key)}: ${v},`)
      }
      out.push('  },')
    }
    if (s.bodyKind === 'multipart') {
      out.push('  body: (() => { const fd = new FormData();')
      for (const f of s.bodyFields ?? []) {
        out.push(`    fd.append(${JSON.stringify(f.key)}, ${valueExpr(f.value)});`)
      }
      for (const name of s.bodyFileFields ?? []) {
        out.push(
          `    // 文件字段 ${JSON.stringify(name)}：请自备文件后 append（fd.append(${JSON.stringify(name)}, fileBlob, fileName)）`,
        )
      }
      out.push('    return fd; })(),')
    } else if (bodyExpr != null) {
      out.push(`  body: ${bodyExpr},`)
    }
    out.push('});')
    out.push('')

    for (const a of s.assertions) {
      if (a.type === 'status') {
        out.push(assertLine(a, `res${n}.status`))
      } else if (a.type === 'response_time_ms') {
        out.push(assertLine(a, `Date.now() - t${n}`))
      } else if (a.type === 'header') {
        const actual = `res${n}.headers.get(${JSON.stringify((a.path ?? '').toLowerCase())})`
        out.push(assertLine(a, actual))
      } else {
        const js = jsonPathToJs(a.path ?? '')
        if (js) {
          out.push(`let body${n} = body${n} ?? await res${n}.json().catch(() => null);`)
          out.push(assertLine(a, js.replace(/^body/, `body${n}`)))
        } else {
          out.push(`// TODO 复杂 JSONPath（${a.path}）请人工转换后断言`)
        }
      }
    }

    for (const e of s.extract) {
      const name = e.name.trim()
      if (!name) continue
      if (e.from === 'header') {
        out.push(
          `var ${JSON.stringify(name).slice(1, -1).replace(/[^a-zA-Z0-9_$]/g, '_')} = res${n}.headers.get(${JSON.stringify(e.path.toLowerCase())}); // 提取 ${e.path}`,
        )
      } else {
        const js = jsonPathToJs(e.path)
        if (js) {
          out.push(`let body${n} = body${n} ?? await res${n}.json().catch(() => null);`)
          const varName = name.replace(/[^a-zA-Z0-9_$]/g, '_')
          out.push(`var ${varName} = (${js.replace(/^body/, `body${n}`)}); // 提取 ${e.path}`)
        } else {
          out.push(`// TODO 提取 ${e.path}（复杂 JSONPath 请人工转换）`)
        }
      }
    }

    // 不支持 / 有损的 Body 类型：明确报告，不静默丢失。
    switch (s.bodyKind) {
      case 'binary':
        out.push(
          `// 注意：该步骤为二进制文件 Body（${s.bodyPath || '未指定路径'}），已还原为本地文件读取——请确认路径后取消注释：`,
        )
        out.push(`// body: readFileSync(${JSON.stringify(s.bodyPath)}),`)
        break
      case 'grpc':
        out.push('// 注意：gRPC 调用无法用 HTTP fetch 还原（原步骤走 gRPC 通道），此脚本不含该步骤的实际调用。')
        break
      case 'other':
        out.push('// 注意：该步骤的 Body 类型在脚本中未生成，完整行为以 Miyo 运行为准。')
        break
      default:
        break
    }
    if (s.authUnsupported) {
      out.push(
        `// 注意：该步骤使用 ${s.authUnsupported}，签名 / 令牌需运行时计算，脚本未内嵌——请在发送前自行补齐对应凭据。`,
      )
    }
    if (s.body != null && s.body.length > SAMPLE_BODY_LIMIT && (s.bodyKind === 'json' || s.bodyKind === 'text')) {
      out.push(`// （Body 共 ${s.body.length} 字符，已完整写入上方 body 字段）`)
    }
    out.push('')
  })

  out.push('console.log(`共 ${results.length} 项检查，通过 ${results.filter((r) => r.ok).length} 项`);')
  const failed = 'results.some((r) => !r.ok)'
  out.push(`if (${failed}) process.exit(1);`)
  return out.join('\n') + '\n'
}
