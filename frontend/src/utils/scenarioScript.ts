/**
 * 场景 → 可运行脚本（Node 18+ 原生 fetch）生成器。
 *
 * 只读预览与 CI 导出用：手动编排是唯一编辑真源，脚本由步骤实时生成，
 * 与编辑内容始终一致。链路变量传递对应到 JS：前一步提取的变量在后续
 * 步骤的 URL / 断言期望中直接引用。
 */
import type { ScenarioAssertionRule, ScenarioExtractRule } from '../types/miyoApi'

/** 生成脚本的步骤输入（从编辑态物化：只带启用行）。 */
export interface ScriptStep {
  /** 步骤名（接口名）。 */
  name: string
  method: string
  /** URL 模板（含 {{base_url}} / {{var}} 占位）。 */
  url: string
  /** 启用中的请求头。 */
  headers: { key: string; value: string }[]
  /** json / text 原文；其余 Body 类型为 null（生成占位注释）。 */
  body: string | null
  bodyKind: 'json' | 'text' | 'other'
  extract: ScenarioExtractRule[]
  assertions: ScenarioAssertionRule[]
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

const SAMPLE_BODY_LIMIT = 60

/**
 * 生成可运行脚本。`base` 为当前环境的 Base URL 兜底值（脚本内仍可用
 * BASE_URL 环境变量覆盖）。
 */
export function generateScenarioScript(
  scenarioName: string,
  steps: ScriptStep[],
  opts: { base: string; stopOnFailure: boolean },
): string {
  const out: string[] = []
  out.push('/**')
  out.push(` * 场景「${scenarioName}」— 由 Miyo 生成，可直接运行：node scenario.mjs`)
  out.push(' * 运行环境：Node 18+（原生 fetch）。BASE_URL 环境变量可覆盖服务地址。')
  out.push(' * 链路变量：前一步提取的值在后续步骤的 URL / 断言期望中直接引用。')
  if (opts.stopOnFailure) out.push(' * 失败即停：任一断言不过则退出（MIYO_STOP_ON_FAILURE=0 关闭）。')
  out.push(' */')
  out.push(`const BASE = process.env.BASE_URL ?? ${JSON.stringify(opts.base)};`)
  out.push('')
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
    const headers = s.bodyKind === 'json' && !s.headers.some((h) => h.key.toLowerCase() === 'content-type')
      ? [...s.headers, { key: 'Content-Type', value: 'application/json' }]
      : s.headers
    if (headers.length) {
      out.push(`let res${n} = await fetch(${urlTemplateToJs(s.url)}, {`)
      out.push(`  method: ${JSON.stringify(s.method)},`)
      out.push('  headers: {')
      for (const h of headers) {
        // 含 {{var}} 的值转模板串插值；纯字面量保持普通字符串。
        const value = h.value.includes('{{') ? urlTemplateToJs(h.value) : JSON.stringify(h.value)
        out.push(`    ${JSON.stringify(h.key)}: ${value},`)
      }
      out.push('  },')
      if (s.body != null) {
        out.push(`  body: ${JSON.stringify(s.body)},`)
      }
      out.push('});')
    } else if (s.body != null) {
      out.push(`let res${n} = await fetch(${urlTemplateToJs(s.url)}, {`)
      out.push(`  method: ${JSON.stringify(s.method)},`)
      out.push(`  body: ${JSON.stringify(s.body)},`)
      out.push('});')
    } else {
      out.push(`let res${n} = await fetch(${urlTemplateToJs(s.url)}, { method: ${JSON.stringify(s.method)} });`)
    }
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
    if (s.bodyKind === 'other') {
      out.push(`// 注意：该步骤的 Body 类型在脚本中未生成，完整行为以 Miyo 运行为准。`)
    }
    if (s.body != null && s.body.length > SAMPLE_BODY_LIMIT) {
      out.push(`// （Body 共 ${s.body.length} 字符，已完整写入上方 body 字段）`)
    }
    out.push('')
  })

  out.push('console.log(`共 ${results.length} 项检查，通过 ${results.filter((r) => r.ok).length} 项`);')
  const failed = 'results.some((r) => !r.ok)'
  out.push(`if (${failed}) process.exit(1);`)
  return out.join('\n') + '\n'
}
