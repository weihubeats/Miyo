/**
 * 场景脚本 → 手动编排步骤 的反向解析器（scenarioScript.ts 的配套）。
 *
 * 解析对象是生成器产出的固定格式（Node fetch 风格）：步骤注释行 /
 * `let urlN` / fetch 块 / check(...) / `var x = ...; // 提取 ...`。
 * 用户在脚本里对断言、提取、URL、方法、头的修改都能读回来；超出该
 * 结构的自定义 JS（循环、自造函数等）或降级注释（复杂 JSONPath 的
 * TODO 行）会以带行号的 ScriptParseError 明确报错，绝不静默丢内容。
 *
 * 与「同步到编排」的边界：编排步骤是 endpoint_id 引用式，URL / 头 /
 * Body 的真源在接口——解析结果携带它们仅供比对与匹配，回写只覆盖
 * 步骤自有字段（extract / assertions / 步骤增删）。
 */
import type { ScenarioAssertionRule, ScenarioExtractRule } from '@/types/miyoApi'
import { tFallback } from '@/stores/locale'

/** 解析失败：message 携带行号与原因（toast 直接可读）。 */
export class ScriptParseError extends Error {
  /** 1-based 行号。 */
  readonly line: number
  constructor(message: string, line: number) {
    super(message)
    this.name = 'ScriptParseError'
    this.line = line
  }
}

/** 解析出的单步（URL / 头 / Body 为脚本里的具体形态，仅供比对）。 */
export interface ParsedScriptStep {
  name: string
  method: string
  /** {{base_url}} / {{var}} 模板形态（由 JS 模板串反向还原）。 */
  url: string
  params: { key: string; value: string }[]
  /** 启用中的头（含由鉴权并入的头，如 Authorization——鉴权无法与普通头区分）。 */
  headers: { key: string; value: string }[]
  /** json / text / graphql query 原文；urlencoded / multipart / none 为 null。 */
  body: string | null
  bodyKind: 'json' | 'text' | 'urlencoded' | 'multipart' | 'graphql' | 'none'
  /** urlencoded / multipart 的文本字段。 */
  bodyFields: { key: string; value: string }[]
  /** graphql variables JSON 文本（仅 bodyKind === 'graphql'）。 */
  bodyVariables: string
  extract: ScenarioExtractRule[]
  assertions: ScenarioAssertionRule[]
}

export interface ParsedScript {
  steps: ParsedScriptStep[]
  /** 脚本中不影响回写字段的降级提示（binary/grpc/不支持鉴权等「注意」行）。 */
  warnings: string[]
}

// ---------- 基础工具 ----------

/** JS 模板串字面量（含反引号）→ {{}} 模板（urlTemplateToJs 的严格逆变换）。 */
export function jsTemplateToTemplate(literal: string): string {
  const m = literal.match(/^`([\s\S]*)`$/)
  if (!m) throw new Error(tFallback('scenario.parse.notTemplate'))
  // 逆着转义顺序还原：\${ → ${、\` → `、\\ → \，再回填 {{}} 占位
  return m[1]
    .replace(/\\\$\{/g, '${')
    .replace(/\\`/g, '`')
    .replace(/\\\\/g, '\\')
    .replace(/\$\{BASE\}/g, '{{base_url}}')
    .replace(/\$\{\s*([^}]+?)\s*\}/g, '{{$1}}')
}

/** 值表达式 → 原始模板值（JSON 字符串字面量 / 模板串 / 裸数字布尔）。 */
function exprToValue(expr: string): string {
  const t = expr.trim()
  if (t.startsWith('`')) return jsTemplateToTemplate(t)
  if (t.startsWith('"')) {
    const v = JSON.parse(t)
    return typeof v === 'string' ? v : String(v)
  }
  return t
}

/** 按顶层逗号拆分（跳过引号 / 模板串 / 括号内，转义感知）。 */
function splitTopLevel(s: string): string[] {
  const out: string[] = []
  let depth = 0
  let quote: string | null = null
  let cur = ''
  for (let i = 0; i < s.length; i += 1) {
    const ch = s[i]
    if (quote) {
      cur += ch
      if (ch === '\\') {
        cur += s[i + 1] ?? ''
        i += 1
      } else if (ch === quote) {
        quote = null
      }
      continue
    }
    if (ch === '"' || ch === "'" || ch === '`') {
      quote = ch
      cur += ch
      continue
    }
    if (ch === '(' || ch === '[' || ch === '{') depth += 1
    if (ch === ')' || ch === ']' || ch === '}') depth -= 1
    if (ch === ',' && depth === 0) {
      out.push(cur)
      cur = ''
      continue
    }
    cur += ch
  }
  if (cur.trim()) out.push(cur)
  return out
}

/**
 * 断言实际值表达式的候选形状（与 assertLine 的 actual 一一对应）：
 * status / 耗时 / 响应头 / JSONPath 可选链（bodyN 裸引用 = 根路径 $）。
 */
const ACTUAL = `(res\\d+\\.status|Date\\.now\\(\\) - t\\d+|res\\d+\\.headers\\.get\\("[^"]*"\\)|body\\d+(?:\\?\\.[A-Za-z_][A-Za-z0-9_-]*|\\?\\.\\[\\d+\\])*|body\\d+)`

/** actual 表达式 → type / path。 */
function actualTypeOf(actual: string): { type: ScenarioAssertionRule['type']; path: string | null } {
  if (/^res\d+\.status$/.test(actual)) return { type: 'status', path: null }
  if (/^Date\.now\(\) - t\d+$/.test(actual)) return { type: 'response_time_ms', path: null }
  const header = actual.match(/^res\d+\.headers\.get\("([^"]*)"\)$/)
  if (header) return { type: 'header', path: header[1] }
  const js = actual.match(/^body\d+((?:\?\.[A-Za-z_][A-Za-z0-9_-]*|\?\.\[\d+\])*)$/)
  if (js) return { type: 'jsonpath', path: jsPathToPath(js[1]) }
  throw new Error(tFallback('scenario.parse.badActual', { v: actual }))
}

/** 可选链片段 → JSONPath（jsonPathToJs 的逆向）：`.user[0].id` → `$.user[0].id`。 */
function jsPathToPath(segments: string): string {
  if (!segments) return '$'
  const re = /\?\.([A-Za-z_][A-Za-z0-9_-]*)|\?\.\[(\d+)\]/g
  let out = '$'
  let consumed = 0
  for (const m of segments.matchAll(re)) {
    out += m[1] ? `.${m[1]}` : `[${m[2]}]`
    consumed += m[0].length
  }
  if (consumed !== segments.length) {
    throw new Error(tFallback('scenario.parse.badPath', { v: segments }))
  }
  return out
}

/** 断言表达式 → AssertionRule（assertLine 各形状的逆映射；结构异常抛 Error）。 */
function parseCheckExpr(expr: string, line: number): ScenarioAssertionRule {
  const t = expr.trim()
  const failMsg = tFallback('scenario.parse.badAssert', { v: t })
  const rules: Array<[RegExp, (m: RegExpMatchArray) => ScenarioAssertionRule]> = [
    [
      new RegExp(`^${ACTUAL} == null \\|\\| \\1 === '' \\|\\| \\(Array\\.isArray\\(\\1\\) && \\1\\.length === 0\\)$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'empty' }),
    ],
    [
      new RegExp(`^${ACTUAL} != null && \\1 !== ''$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'not_empty' }),
    ],
    [
      new RegExp(`^${ACTUAL} != null$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'exists' }),
    ],
    [
      new RegExp(`^${ACTUAL} == null$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'not_exists' }),
    ],
    [
      new RegExp(`^!String\\(${ACTUAL}\\)\\.includes\\(([\\s\\S]+)\\)$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'not_contains', expected: exprToValue(m[2]) }),
    ],
    [
      new RegExp(`^String\\(${ACTUAL}\\)\\.includes\\(([\\s\\S]+)\\)$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'contains', expected: exprToValue(m[2]) }),
    ],
    [
      new RegExp(`^Number\\(${ACTUAL}\\) (>=|<=|>|<) ([\\s\\S]+)$`),
      (m) => ({
        ...actualTypeOf(m[1]),
        op: ({ '>=': 'gte', '<=': 'lte', '>': 'gt', '<': 'lt' } as const)[m[2]],
        expected: exprToValue(m[3]),
      }),
    ],
    [
      new RegExp(`^${ACTUAL} !== ([\\s\\S]+)$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'neq', expected: exprToValue(m[2]) }),
    ],
    [
      new RegExp(`^${ACTUAL} === ([\\s\\S]+)$`),
      (m) => ({ ...actualTypeOf(m[1]), op: 'eq', expected: exprToValue(m[2]) }),
    ],
  ]
  for (const [re, build] of rules) {
    const m = t.match(re)
    if (m) {
      try {
        return build(m)
      } catch (e) {
        throw new ScriptParseError(tFallback('scenario.parse.atLine', { msg: (e as Error).message, line }), line)
      }
    }
  }
  throw new ScriptParseError(tFallback('scenario.parse.atLine', { msg: failMsg, line }), line)
}

// ---------- 主解析 ----------

const STEP_RE = /^\/\/ ---- 步骤 (\d+)：(.+?)  ([A-Za-z]+) (`[\s\S]*`) ----$/
const URL_LET_RE = /^let url(\d+) = (`[\s\S]*`);$/
const PARAMS_OPEN_RE = /^url(\d+) \+= \(url\1\.includes\('\?'\) \? '&' : '\?'\) \+ new URLSearchParams\(\[$/
const PARAM_ROW_RE = /^  \["((?:[^"\\]|\\.)*)", ([\s\S]+)\],$/
const PARAMS_CLOSE = ']).toString();'
const FETCH_RE = /^let res(\d+) = await fetch\(url\1, \{$/
const METHOD_RE = /^  method: ("(?:[^"\\]|\\.)*"),$/
const HEADERS_OPEN = '  headers: {'
const HEADER_ROW_RE = /^    "((?:[^"\\]|\\.)*)": ([\s\S]+),$/
const BODY_RE = /^  body: ([\s\S]+),$/
const FETCH_CLOSE = '});'
const BODY_MARK_RE = /^let body(\d+) = body\1 \?\? await res\1\.json\(\)\.catch\(\(\) => null\);$/
const CHECK_RE = /^check\(([\s\S]+)\);$/
const EXTRACT_BODY_RE = /^var ([A-Za-z0-9_$]+) = \(body(\d+)([\s\S]*)\); \/\/ 提取 (.+)$/
const EXTRACT_HEADER_RE = /^var ([A-Za-z0-9_$]+) = res(\d+)\.headers\.get\("([^"]*)"\); \/\/ 提取 (.+)$/
const GRAPHQL_RE = /^JSON\.stringify\(\{ query: ("(?:[^"\\]|\\.)*"), variables: (.+) \}\)$/
const URLENCODED_RE = /^new URLSearchParams\(\[([\s\S]*)\]\)\.toString\(\)$/
const MULTIPART_OPEN_RE = /^  body: \(\(\) => \{ const fd = new FormData\(\);$/
const FD_APPEND_RE = /^    fd\.append\("((?:[^"\\]|\\.)*)", ([\s\S]+)\);$/
const FD_CLOSE = '    return fd; })(),'
const RUN_SUMMARY_RE = /^console\.log\(`共 /

export function parseScenarioScript(script: string): ParsedScript {
  const lines = script.split('\n')
  const steps: ParsedScriptStep[] = []
  const warnings: string[] = []
  /** lines 游标（0-based；错误统一报 1-based 行号）。 */
  let i = 0

  const fail = (msg: string, line = i + 1): never => {
    throw new ScriptParseError(tFallback('scenario.parse.atLine', { msg, line }), line)
  }

  while (i < lines.length) {
    const line = lines[i]
    if (STEP_RE.test(line)) {
      steps.push(parseStep())
      continue
    }
    if (RUN_SUMMARY_RE.test(line)) break
    const warn = line.match(/^\/\/ 注意：(.+)$/)
    if (warn) warnings.push(warn[1].trim())
    i += 1
  }

  if (!steps.length) {
    throw new ScriptParseError(tFallback('scenario.parse.noSteps'), 1)
  }
  return { steps, warnings }

  /** 从当前行（步骤注释）开始解析一个完整步骤，结束时光标停在本步末尾。 */
  function parseStep(): ParsedScriptStep {
    const head = lines[i].match(STEP_RE) as RegExpMatchArray
    const n = Number(head[1])
    const step: ParsedScriptStep = {
      name: head[2].trim(),
      method: head[3],
      url: safeTemplate(head[4], n),
      params: [],
      headers: [],
      body: null,
      bodyKind: 'none',
      bodyFields: [],
      bodyVariables: '',
      extract: [],
      assertions: [],
    }
    i += 1

    let sawFetch = false
    while (i < lines.length) {
      const line = lines[i]
      if (STEP_RE.test(line) || RUN_SUMMARY_RE.test(line)) break

      let mm: RegExpMatchArray | null
      // 计时行（耗时断言用）：let tN = Date.now();
      if (/^let t\d+ = Date\.now\(\);$/.test(line)) {
        i += 1
        continue
      }
      if ((mm = line.match(URL_LET_RE)) && Number(mm[1]) === n) {
        // 与步骤注释里的 URL 一致（注释为兜底来源），以 let 行为准
        step.url = safeTemplate(mm[2], n)
        i += 1
        continue
      }
      if ((mm = line.match(PARAMS_OPEN_RE)) && Number(mm[1]) === n) {
        i += 1
        while (i < lines.length && lines[i] !== PARAMS_CLOSE) {
          const row = lines[i].match(PARAM_ROW_RE)
          if (!row) {
            fail(tFallback('scenario.parse.stepBadParam', { n, v: lines[i].trim() }))
          } else {
            step.params.push({ key: JSON.parse(`"${row[1]}"`), value: exprToValue(row[2]) })
          }
          i += 1
        }
        if (lines[i] !== PARAMS_CLOSE) fail(tFallback('scenario.parse.stepParamsUnclosed', { n }))
        i += 1
        continue
      }
      if ((mm = line.match(FETCH_RE)) && Number(mm[1]) === n) {
        sawFetch = true
        i += 1
        parseFetch(n, step)
        continue
      }
      if ((mm = line.match(BODY_MARK_RE)) && Number(mm[1]) === n) {
        i += 1
        continue
      }
      if ((mm = line.match(CHECK_RE))) {
        try {
          step.assertions.push(parseCheckExpr(splitCheckArgs(mm[1], n)[1], i + 1))
          const label = splitCheckArgs(mm[1], n)[0]
          const last = step.assertions[step.assertions.length - 1]
          if (label.startsWith('"')) last.name = JSON.parse(label)
        } catch (e) {
          if (e instanceof ScriptParseError) throw e
          fail(tFallback('scenario.parse.stepWrap', { n, v: (e as Error).message }), i + 1)
        }
        i += 1
        continue
      }
      if ((mm = line.match(EXTRACT_BODY_RE)) && Number(mm[2]) === n) {
        if (mm[3] && !/^(\?\.[A-Za-z_][A-Za-z0-9_-]*|\?\.\[\d+\])*$/.test(mm[3])) {
          fail(tFallback('scenario.parse.stepBadExtract', { n, v: `${mm[2]}${mm[3]}` }))
        }
        step.extract.push({ name: mm[1], from: 'body', path: mm[4].trim() })
        i += 1
        continue
      }
      if ((mm = line.match(EXTRACT_HEADER_RE)) && Number(mm[2]) === n) {
        step.extract.push({ name: mm[1], from: 'header', path: mm[4].trim() })
        i += 1
        continue
      }
      if (/^\/\/ TODO/.test(line)) {
        fail(tFallback('scenario.parse.stepTodo', { n }))
      }
      if (/^\/\/ 注意：/.test(line)) {
        warnings.push(line.replace(/^\/\/ 注意：/, '').trim())
        i += 1
        continue
      }
      if (!line.trim()) {
        i += 1
        continue
      }
      fail(tFallback('scenario.parse.stepBadLine', { n, v: line.trim() }))
    }

    if (!sawFetch) fail(tFallback('scenario.parse.stepNoFetch', { n }))
    return step
  }

  /** fetch 块内部（method / headers / body），直到 `});`。 */
  function parseFetch(n: number, step: ParsedScriptStep): void {
    let inHeaders = false
    let inMultipart = false
    while (i < lines.length) {
      const line = lines[i]
      if (line === FETCH_CLOSE) {
        i += 1
        return
      }
      let mm: RegExpMatchArray | null
      if ((mm = line.match(METHOD_RE))) {
        const method = JSON.parse(mm[1])
        if (method !== step.method) {
          fail(tFallback('scenario.parse.stepMethodMismatch', { n, a: step.method, b: method }))
        }
        i += 1
        continue
      }
      if (line === HEADERS_OPEN) {
        inHeaders = true
        i += 1
        continue
      }
      if (inHeaders && line === '  },') {
        inHeaders = false
        i += 1
        continue
      }
      if (inHeaders) {
        mm = line.match(HEADER_ROW_RE)
        if (!mm) {
          fail(tFallback('scenario.parse.stepBadHeader', { n, v: line.trim() }))
        } else {
          step.headers.push({ key: JSON.parse(`"${mm[1]}"`), value: headerValue(mm[2]) })
          i += 1
        }
        continue
      }
      if (MULTIPART_OPEN_RE.test(line)) {
        inMultipart = true
        step.bodyKind = 'multipart'
        i += 1
        continue
      }
      if (inMultipart) {
        if (line === FD_CLOSE) {
          inMultipart = false
          i += 1
          continue
        }
        mm = line.match(FD_APPEND_RE)
        if (!mm && /^    \/\/ 文件字段 /.test(line)) {
          i += 1
          continue
        }
        if (!mm) {
          fail(tFallback('scenario.parse.stepBadForm', { n, v: line.trim() }))
        } else {
          step.bodyFields.push({ key: JSON.parse(`"${mm[1]}"`), value: exprToValue(mm[2]) })
          i += 1
        }
        continue
      }
      if ((mm = line.match(BODY_RE))) {
        parseBody(mm[1], n, step)
        i += 1
        continue
      }
      fail(tFallback('scenario.parse.stepBadFetchLine', { n, v: line.trim() }))
    }
    fail(tFallback('scenario.parse.stepFetchUnclosed', { n }))
  }

  /** fetch 内 body 表达式 → body / bodyKind / bodyFields / bodyVariables。 */
  function parseBody(expr: string, n: number, step: ParsedScriptStep): void {
    const t = expr.trim()
    const gql = t.match(GRAPHQL_RE)
    if (gql) {
      step.body = JSON.parse(gql[1])
      step.bodyKind = 'graphql'
      const varsRaw = gql[2].trim()
      if (varsRaw === '{}') {
        step.bodyVariables = ''
      } else {
        const vm = varsRaw.match(/^JSON\.parse\(("(?:[^"\\]|\\.)*")\)$/)
        if (!vm) {
          fail(tFallback('scenario.parse.stepBadGraphqlVars', { n, v: varsRaw }))
        } else {
          step.bodyVariables = JSON.parse(vm[1])
        }
      }
      return
    }
    const urlenc = t.match(URLENCODED_RE)
    if (urlenc) {
      step.bodyKind = 'urlencoded'
      const inner = urlenc[1].trim()
      if (!inner) return
      for (const pair of splitTopLevel(inner)) {
        const pm = pair.trim().match(/^\["((?:[^"\\]|\\.)*)", ([\s\S]+)\]$/)
        if (!pm) {
          fail(tFallback('scenario.parse.stepBadUrlencoded', { n, v: pair.trim() }))
        } else {
          step.bodyFields.push({ key: JSON.parse(`"${pm[1]}"`), value: exprToValue(pm[2]) })
        }
      }
      return
    }
    if (t.startsWith('"')) {
      // json 与 text 的生成形态一致：按 Content-Type 头判别
      step.body = exprToValue(t)
      const ct = step.headers.find((h) => h.key.toLowerCase() === 'content-type')
      step.bodyKind = ct?.value.includes('application/json') ? 'json' : 'text'
      return
    }
    fail(tFallback('scenario.parse.stepBadBody', { n, v: t }))
  }

  /** 请求头值：模板串 → {{}} 模板；JSON 字符串 → 原文；其余（Basic 的 btoa 拼接）原样保留。 */
  function headerValue(expr: string): string {
    const t = expr.trim()
    if (t.startsWith('`')) return safeTemplate(t, i + 1)
    if (t.startsWith('"')) return exprToValue(t)
    return t
  }

  function safeTemplate(literal: string, n: number): string {
    try {
      return jsTemplateToTemplate(literal)
    } catch (e) {
      throw new ScriptParseError(tFallback('scenario.parse.atLine', { msg: tFallback('scenario.parse.stepUrlFail', { n, v: (e as Error).message }), line: i + 1 }), i + 1)
    }
  }

  /** check(label, expr) 的实参拆分：label / expr 各一段。 */
  function splitCheckArgs(argsSrc: string, n: number): [string, string] {
    const parts = splitTopLevel(argsSrc)
    if (parts.length !== 2) {
      fail(tFallback('scenario.parse.stepCheckArgs', { n, v: parts.length }))
    }
    return [parts[0].trim(), parts[1]]
  }
}
