/**
 * codeImport：从各语言 HTTP 客户端代码片段解析请求（cURL 之外的前端解析器）。
 *
 * 输出与后端 parse_curl_command 的 CurlParsed 同形状，导入路径复用
 * store.openCurlDraft。解析为启发式最佳努力：覆盖常见写法
 * （JS fetch/axios、Python requests、Java OkHttp/HttpURLConnection、
 * Go net/http、Rust reqwest、PHP cURL/Guzzle），
 * 无法识别的部分（变量引用等）跳过而不是报错；仅「找不到 URL」视为失败。
 */
import type { AuthSpec, BodySpec, CurlParsed, HttpMethod, KeyValue } from '../types/miyoApi'
import { tFallback } from '../stores/locale'

export type SnippetLang = 'curl' | 'java' | 'python' | 'javascript' | 'go' | 'rust' | 'php'

/** 语言名（cURL / Java…）为品牌名不翻译；「自动检测」为展示文案，值放字典键、由组件 t() 渲染。 */
export const SNIPPET_LANGS: Array<{ value: SnippetLang | 'auto'; label: string }> = [
  { value: 'auto', label: 'codeimport.langAuto' },
  { value: 'curl', label: 'cURL' },
  { value: 'java', label: 'Java (OkHttp / HttpURLConnection)' },
  { value: 'python', label: 'Python (requests)' },
  { value: 'javascript', label: 'JavaScript (fetch / axios)' },
  { value: 'go', label: 'Go (net/http)' },
  { value: 'rust', label: 'Rust (reqwest)' },
  { value: 'php', label: 'PHP (cURL / Guzzle)' },
]

const METHODS: HttpMethod[] = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']

function kv(key: string, value: string): KeyValue {
  return { key, value, enabled: true, description: '', field_type: 'string', required: true, example: '' }
}

// ---------- 通用工具 ----------

/** 提取第一个 http(s) URL（停在引号/空白/反引号/右括号处，去掉尾部标点）。 */
function findUrl(src: string): string | null {
  const m = src.match(/https?:\/\/[^\s"'`\\)\]}<>]+/i)
  return m ? m[0].replace(/[.,;:]+$/, '') : null
}

/** 解码字符串字面量里的常见转义（\n \t \r \" \' \\ \/）。 */
function unescapeLiteral(s: string): string {
  return s
    .replace(/\\n/g, '\n')
    .replace(/\\t/g, '\t')
    .replace(/\\r/g, '\r')
    .replace(/\\"/g, '"')
    .replace(/\\'/g, "'")
    .replace(/\\`/g, '`')
    .replace(/\\\//g, '/')
    .replace(/\\\\/g, '\\')
}

/** 匹配一段带引号的字符串字面量（含转义），返回内容；quote 为起始引号。 */
function readQuoted(src: string, start: number, quote: string): { text: string; end: number } | null {
  if (src[start] !== quote) return null
  let raw = ''
  for (let i = start + 1; i < src.length; i += 1) {
    const ch = src[i]
    if (ch === '\\' && i + 1 < src.length) {
      raw += ch + src[i + 1]
      i += 1
      continue
    }
    if (ch === quote) return { text: unescapeLiteral(raw), end: i + 1 }
    if (ch === '\n' && quote !== '`') return null
    raw += ch
  }
  return null
}

/** 从 fromIndex 起读取一个「值」：字符串字面量或平衡花括号对象字面量。 */
function readValue(src: string, from: number): { text: string; end: number; quoted: boolean } | null {
  let i = from
  while (i < src.length && /\s/.test(src[i])) i += 1
  const ch = src[i]
  if (ch === '"' || ch === "'" || ch === '`') {
    const lit = readQuoted(src, i, ch)
    if (lit) return { text: lit.text, end: lit.end, quoted: true }
    return null
  }
  if (ch === '{' || ch === '[') {
    const close = ch === '{' ? '}' : ']'
    let depth = 0
    let inStr: string | null = null
    for (let j = i; j < src.length; j += 1) {
      const c = src[j]
      if (inStr) {
        if (c === '\\') j += 1
        else if (c === inStr) inStr = null
        continue
      }
      if (c === '"' || c === "'" || c === '`') inStr = c
      else if (c === ch) depth += 1
      else if (c === close) {
        depth -= 1
        if (depth === 0) return { text: src.slice(i, j + 1), end: j + 1, quoted: false }
      }
    }
  }
  return null
}

/** 在 `key:` / `key =` 后读取值（JS/Python 对象与命名参数通用）。 */
function valueAfterKey(src: string, key: string): { text: string; quoted: boolean } | null {
  const re = new RegExp(`\\b${key}\\s*[:=]\\s*`)
  const m = re.exec(src)
  if (!m) return null
  const v = readValue(src, m.index + m[0].length)
  return v ? { text: v.text, quoted: v.quoted } : null
}

/** 解析对象/字典字面量里的 `键: 值` 字符串对（值仅取字符串字面量）。 */
function parseObjectPairs(objSrc: string): Array<[string, string]> {
  const pairs: Array<[string, string]> = []
  const re = /["'`]?([\w.$-]+)["'`]?\s*:\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|`((?:[^`\\]|\\.)*)`)/g
  let m: RegExpExecArray | null
  while ((m = re.exec(objSrc)) !== null) {
    const value = m[2] ?? m[3] ?? m[4] ?? ''
    pairs.push([m[1], unescapeLiteral(value)])
  }
  return pairs
}

/** 按内容推断 body 模式：JSON 外形 → json；k=v 串 + urlencoded 头 → urlencoded 字段。 */
function inferBody(raw: string, contentType: string): BodySpec {
  const trimmed = raw.trim()
  const ct = contentType.toLowerCase()
  if ((ct.includes('json') || /^[[{]/.test(trimmed)) && looksLikeJson(trimmed)) {
    return { mode: 'json', raw: trimmed }
  }
  if (ct.includes('urlencoded') && trimmed.includes('=')) {
    const fields = trimmed
      .split('&')
      .map((part) => {
        const eq = part.indexOf('=')
        return eq > 0
          ? kv(decodeURIComponent(part.slice(0, eq)), decodeURIComponent(part.slice(eq + 1).replace(/\+/g, ' ')))
          : null
      })
      .filter((x): x is KeyValue => x !== null)
    if (fields.length) return { mode: 'urlencoded', fields }
  }
  return { mode: 'text', raw }
}

function looksLikeJson(s: string): boolean {
  if (!s.startsWith('{') && !s.startsWith('[')) return false
  try {
    JSON.parse(s)
    return true
  } catch {
    return false
  }
}

// ---------- 语言检测 ----------

/** 按代码特征检测语言（cURL 优先，避免与 JS 里的 fetch 混淆）。 */
export function detectLang(src: string): SnippetLang | null {
  if (/^\s*curl\b/i.test(src) || /\bcurl\s+(-[A-Za-z]|https?:\/\/)/.test(src)) return 'curl'
  if (/new Request\.Builder\(|OkHttpClient|HttpURLConnection|openConnection\(\)/.test(src)) return 'java'
  if (/\brequests\s*[.(]\s*(get|post|put|patch|delete|head|request)\b/.test(src) || /^import requests\b/m.test(src)) return 'python'
  if (/\bfetch\s*\(|\baxios\s*[.(]|XMLHttpRequest/.test(src)) return 'javascript'
  if (/http\.NewRequest|["']net\/http["']/.test(src)) return 'go'
  if (/reqwest::|Client::builder|Client::new/.test(src)) return 'rust'
  if (/curl_setopt|CURLOPT_|new Client\(\[|GuzzleHttp/.test(src)) return 'php'
  return null
}

function detectMethod(src: string): HttpMethod {
  // 显式 method 配置优先：method: 'POST' / setRequestMethod / NewRequest / request("POST")
  const explicit = [
    /\bmethod\s*[:=]\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]/i,
    /\bsetRequestMethod\s*\(\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]\s*\)/i,
    /\bhttp\.NewRequest\s*\(\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]/i,
    /\brequests\.request\s*\(\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]/i,
    /\baxios\s*\(\s*\{[\s\S]*?\bmethod\s*:\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]/i,
  ]
  for (const re of explicit) {
    const m = re.exec(src)
    if (m) return m[1].toUpperCase() as HttpMethod
  }
  // 链式/速记调用：.post( / axios.post( / requests.post(
  const shorthand = new RegExp(
    `(?:\\.|\\b)(get|post|put|patch|delete|head)\\s*\\(`,
    'i',
  )
  const m = shorthand.exec(src)
  // 注意：Java 里 map.put/get 等链式调用可能误报，这里接受最佳努力结果，
  // 预览界面可见方法名，用户可在导入前修正。
  if (m && METHODS.includes(m[1].toUpperCase() as HttpMethod)) {
    return m[1].toUpperCase() as HttpMethod
  }
  return 'GET'
}

// ---------- 各语言解析 ----------

type PartialParsed = {
  url: string
  method: HttpMethod
  headers: KeyValue[]
  bodyRaw: string | null
  bodyFromObject: string | null
  /** `user:pass` 原文（Rust basic_auth 等可还原凭据的写法）。 */
  basicAuth?: string
}

function parseHeadersObject(src: string, key: string): KeyValue[] {
  const re = new RegExp(`\\b${key}\\s*[:=]\\s*`)
  const m = re.exec(src)
  if (!m) return []
  const v = readValue(src, m.index + m[0].length)
  if (!v || v.quoted) return []
  return parseObjectPairs(v.text).map(([k, val]) => kv(k, val))
}

function parseJavaScript(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers = parseHeadersObject(src, 'headers')
  // body: '字面量' / data: {...}（axios 配置式）/ body: JSON.stringify({...}) → 取内层对象
  let bodyRaw: string | null = null
  let bodyFromObject: string | null = null
  const bodyAssign = /\b(?:body|data)\s*:\s*/g
  let m: RegExpExecArray | null
  while ((m = bodyAssign.exec(src)) !== null) {
    const rest = src.slice(m.index + m[0].length)
    const stringify = /^\s*JSON\.stringify\s*\(\s*/.exec(rest)
    if (stringify) {
      const inner = readValue(rest, stringify[0].length)
      if (inner && !inner.quoted) {
        bodyFromObject = inner.text
        break
      }
    } else {
      const v = readValue(rest, 0)
      if (v?.quoted) {
        bodyRaw = v.text
        break
      }
      if (v && !v.quoted) {
        bodyFromObject = v.text
        break
      }
    }
  }
  // axios.post(url, data, …)：跳过第一个参数后取字面量参数作为 body
  if (bodyRaw === null && bodyFromObject === null) {
    const call = /\baxios\s*\.\s*\w+\s*\(\s*[^,()]*\s*,\s*/.exec(src)
    if (call) {
      const v = readValue(src, call.index + call[0].length)
      if (v?.quoted) bodyRaw = v.text
      else if (v) bodyFromObject = v.text
    }
  }
  return { url, method: detectMethod(src), headers, bodyRaw, bodyFromObject }
}

function parsePython(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers = parseHeadersObject(src, 'headers')
  let bodyRaw: string | null = null
  let bodyFromObject: string | null = null
  // json={...} / json=json.dumps({...}) → JSON body
  const jsonAssign = /\bjson\s*=\s*/g
  let m: RegExpExecArray | null
  while ((m = jsonAssign.exec(src)) !== null) {
    const rest = src.slice(m.index + m[0].length)
    const dumps = /^\s*json\.dumps\s*\(\s*/.exec(rest)
    const probe = dumps ? rest.slice(dumps[0].length) : rest
    const v = readValue(probe, 0)
    if (v && !v.quoted) {
      bodyFromObject = v.text
      break
    }
  }
  // data='k=v&…'（仅当没有 json= 时）
  if (bodyFromObject === null) {
    const data = valueAfterKey(src, 'data')
    if (data?.quoted) bodyRaw = data.text
  }
  return { url, method: detectMethod(src), headers, bodyRaw, bodyFromObject }
}

function parseJava(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers: KeyValue[] = []
  const headerRe =
    /(?:\.addHeader|\.header|\.setRequestProperty|\.addRequestProperty)\s*\(\s*("(?:[^"\\]|\\.)*")\s*,\s*("(?:[^"\\]|\\.)*")\s*\)/g
  let m: RegExpExecArray | null
  while ((m = headerRe.exec(src)) !== null) {
    const key = unescapeLiteral(m[1].slice(1, -1))
    const value = unescapeLiteral(m[2].slice(1, -1))
    if (key) headers.push(kv(key, value))
  }
  // RequestBody.create：两种参数序都支持——
  //   OkHttp 3.x: create("body", MediaType.parse("mime"))
  //   OkHttp 4.x: create(mediaType, "body")，mediaType 可为变量或内联 MediaType.parse
  let bodyRaw: string | null = null
  const mediaVars = new Map<string, string>()
  const varRe = /([A-Za-z_]\w*)\s*=\s*MediaType\.parse\s*\(\s*"((?:[^"\\]|\\.)*)"\s*\)/g
  let vm: RegExpExecArray | null
  while ((vm = varRe.exec(src)) !== null) {
    mediaVars.set(vm[1], unescapeLiteral(vm[2]))
  }
  const rbRe = /RequestBody\s*\.\s*create\s*\(/g
  let rb: RegExpExecArray | null
  while ((rb = rbRe.exec(src)) !== null && bodyRaw === null) {
    const args = src.slice(rb.index + rb[0].length)
    const litFirst = readValue(args, 0)
    if (litFirst?.quoted) {
      const rest = args.slice(litFirst.end).replace(/^\s*,\s*/, '')
      if (/^MediaType/.test(rest)) {
        const inline = /MediaType\.parse\s*\(\s*"((?:[^"\\]|\\.)*)"/.exec(rest)
        if (inline) headers.push(kv('Content-Type', unescapeLiteral(inline[1])))
        bodyRaw = litFirst.text
      }
      continue
    }
    let mime: string | null = null
    let afterMedia = -1
    const inline = /^MediaType\.parse\s*\(\s*"((?:[^"\\]|\\.)*)"\s*\)\s*,/.exec(args)
    if (inline) {
      mime = unescapeLiteral(inline[1])
      afterMedia = inline[0].length
    } else {
      const idm = /^\s*([A-Za-z_]\w*)\s*,/.exec(args)
      if (idm && mediaVars.has(idm[1])) {
        mime = mediaVars.get(idm[1]) ?? null
        afterMedia = idm[0].length
      }
    }
    if (mime !== null && afterMedia >= 0) {
      const body = readValue(args, afterMedia)
      if (body?.quoted) {
        headers.push(kv('Content-Type', mime))
        bodyRaw = body.text
      }
    }
  }
  // HttpURLConnection: conn.getOutputStream().write("...".getBytes())
  if (bodyRaw === null) {
    const write = /\.write\s*\(\s*("(?:[^"\\]|\\.)*")\s*\.getBytes/.exec(src)
    if (write) bodyRaw = unescapeLiteral(write[1].slice(1, -1))
  }
  return { url, method: detectMethod(src), headers, bodyRaw, bodyFromObject: null }
}

function parseGo(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers: KeyValue[] = []
  const setRe = /req\.Header\.(?:Set|Add)\s*\(\s*("(?:[^"\\]|\\.)*")\s*,\s*("(?:[^"\\]|\\.)*")\s*\)/g
  let m: RegExpExecArray | null
  while ((m = setRe.exec(src)) !== null) {
    headers.push(kv(unescapeLiteral(m[1].slice(1, -1)), unescapeLiteral(m[2].slice(1, -1))))
  }
  // bytes.NewBufferString("…") / strings.NewReader("…")
  let bodyRaw: string | null = null
  const bodyRe = /(?:bytes\.NewBufferString|strings\.NewReader)\s*\(\s*("(?:[^"\\]|\\.)*")/g
  let b: RegExpExecArray | null
  while ((b = bodyRe.exec(src)) !== null) {
    bodyRaw = unescapeLiteral(b[1].slice(1, -1))
    break
  }
  return { url, method: detectMethod(src), headers, bodyRaw, bodyFromObject: null }
}

/**
 * Rust (reqwest)：`client.post("URL")` / `.header("K", "V")` /
 * `.body("…")` / `.json(payload)` / `.bearer_auth("…")`（token 记为 Bearer 头）。
 */
function parseRust(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers: KeyValue[] = []
  const headerRe = /\.header\s*\(\s*("(?:[^"\\]|\\.)*")\s*,\s*("(?:[^"\\]|\\.)*")\s*\)/g
  let m: RegExpExecArray | null
  while ((m = headerRe.exec(src)) !== null) {
    headers.push(kv(unescapeLiteral(m[1].slice(1, -1)), unescapeLiteral(m[2].slice(1, -1))))
  }
  // .bearer_auth("tok") → Authorization: Bearer 头（导入后即用，无需二次配置）。
  const bearer = /\.bearer_auth\s*\(\s*("(?:[^"\\]|\\.)*")\s*\)/.exec(src)
  if (bearer) headers.push(kv('Authorization', `Bearer ${unescapeLiteral(bearer[1].slice(1, -1))}`))
  // .basic_auth("user", "pass") → 还原为 user:pass（parseCodeSnippet 后续转 Basic）。
  const basic = /\.basic_auth\s*\(\s*("(?:[^"\\]|\\.)*")\s*,\s*("(?:[^"\\]|\\.)*")?\s*\)/.exec(src)
  let basicAuth: string | null = null
  if (basic) {
    basicAuth = `${unescapeLiteral(basic[1].slice(1, -1))}:${basic[2] ? unescapeLiteral(basic[2].slice(1, -1)) : ''}`
  }
  let bodyRaw: string | null = null
  let bodyFromObject: string | null = null
  // .body("…") 字面量优先；.json(&x)/.json(payload) 取对象字面量。
  const bodyLit = /\.body\s*\(\s*("(?:[^"\\]|\\.)*")\s*\)/.exec(src)
  if (bodyLit) {
    bodyRaw = unescapeLiteral(bodyLit[1].slice(1, -1))
  } else {
    // `.json(&x)` / `.json(payload)` / `.json(&serde_json::json!({...}))`：
    // 跳过 `&` 与 `json!(` 宏包裹，直取内层对象字面量。
    const jsonCall = /\.json\s*\(\s*&?\s*(?:[A-Za-z_][\w:]*!\s*\(\s*)?/.exec(src)
    if (jsonCall) {
      const v = readValue(src, jsonCall.index + jsonCall[0].length)
      if (v && !v.quoted) bodyFromObject = v.text
      else if (v?.quoted) bodyRaw = v.text
    }
  }
  // reqwest::Method::POST / Method::PUT 显式形式。
  const methodEnum = /Method::(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)/.exec(src)
  const out: PartialParsed = {
    url,
    method: methodEnum ? (methodEnum[1] as HttpMethod) : detectMethod(src),
    headers,
    bodyRaw,
    bodyFromObject,
  }
  if (basicAuth) out.basicAuth = basicAuth
  return out
}

/**
 * PHP：原生 `curl_setopt($ch, CURLOPT_*, …)` 与 Guzzle `$client->post("URL", […])`。
 * 数组字面量 `[...]` / `array(...)` 内取字符串对；`CURLOPT_HTTPHEADER` 的
 * `"K: V"` 条目按首个冒号拆分。
 */
function parsePhp(src: string): PartialParsed {
  const url = findUrl(src)
  if (!url) throw new Error(tFallback('codeimport.noUrl'))
  const headers: KeyValue[] = []
  // PHP 数组转 JSON（`=>` → `:` 后取字符串对；嵌套仅最佳努力）。
  function phpArrayToJson(text: string): string | null {
    const pairs = parseObjectPairs(text.replace(/=>/g, ':'))
    if (!pairs.length) return null
    const obj: Record<string, string> = {}
    for (const [k, val] of pairs) obj[k] = val
    return JSON.stringify(obj)
  }
  /**
   * 读取 `array(...)` / `[...]` 的完整跨度（含引号感知，避免逗号误切）。
   * 返回跨度文本；括号不平衡时返回 null。
   */
  function readArraySpan(from: number): string | null {
    let i = from
    while (i < src.length && /\s/.test(src[i])) i += 1
    let open = ''
    let close = ''
    if (src.startsWith('array', i)) {
      const p = src.indexOf('(', i + 5)
      if (p === -1) return null
      i = p
      open = '('
      close = ')'
    } else if (src[i] === '[') {
      open = '['
      close = ']'
    } else {
      return null
    }
    let depth = 0
    let quote: string | null = null
    for (let j = i; j < src.length; j += 1) {
      const ch = src[j]
      if (quote) {
        if (ch === '\\') j += 1
        else if (ch === quote) quote = null
        continue
      }
      if (ch === '"' || ch === "'") quote = ch
      else if (ch === open) depth += 1
      else if (ch === close) {
        depth -= 1
        if (depth === 0) return src.slice(i, j + 1)
      }
    }
    return null
  }
  // CURLOPT_HTTPHEADER, ["K: V", ...] / array("K: V")
  const hhRe = /CURLOPT_HTTPHEADER\s*,\s*/g
  let hm: RegExpExecArray | null
  while ((hm = hhRe.exec(src)) !== null) {
    const span = readArraySpan(hm.index + hm[0].length)
    if (!span) continue
    const litRe = /"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'/g
    let lm: RegExpExecArray | null
    while ((lm = litRe.exec(span)) !== null) {
      const entry = unescapeLiteral(lm[1] ?? lm[2] ?? '')
      const colon = entry.indexOf(':')
      if (colon > 0) headers.push(kv(entry.slice(0, colon).trim(), entry.slice(colon + 1).trim()))
    }
  }
  // Guzzle: ['headers' => [...]]（`=>` 形式，parseHeadersObject 只认 :/= 故单写）。
  const gzRe = /['"]headers['"]\s*=>\s*/g
  let gm: RegExpExecArray | null
  while ((gm = gzRe.exec(src)) !== null) {
    const v = readValue(src, gm.index + gm[0].length)
    if (v && !v.quoted) {
      for (const [k, val] of parseObjectPairs(v.text.replace(/=>/g, ':')))
        headers.push(kv(k, val))
    }
  }
  let bodyRaw: string | null = null
  let bodyFromObject: string | null = null
  // CURLOPT_POSTFIELDS, "…" / '…'。
  const pf = /CURLOPT_POSTFIELDS\s*,\s*/.exec(src)
  if (pf) {
    const v = readValue(src, pf.index + pf[0].length)
    if (v?.quoted) bodyRaw = v.text
    else if (v) bodyFromObject = v.text
  }
  // Guzzle: 'body' => "…" / 'json' => [...]（`=>` 数组先转 JSON）。
  if (bodyRaw === null && bodyFromObject === null) {
    for (const key of ['body', 'json', 'form_params']) {
      const re = new RegExp(`['"]${key}['"]\\s*=>\\s*`)
      const mm = re.exec(src)
      if (!mm) continue
      const v = readValue(src, mm.index + mm[0].length)
      if (!v) continue
      if (v.quoted) {
        bodyRaw = v.text
        break
      }
      bodyFromObject = phpArrayToJson(v.text) ?? v.text
      break
    }
  }
  // CURLOPT_CUSTOMREQUEST, "PUT" 显式方法。
  const custom = /CURLOPT_CUSTOMREQUEST\s*,\s*['"`](GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)['"`]/i.exec(src)
  return {
    url,
    method: custom ? (custom[1].toUpperCase() as HttpMethod) : detectMethod(src),
    headers,
    bodyRaw,
    bodyFromObject,
  }
}

// ---------- 汇总 ----------

const PARSERS: Record<Exclude<SnippetLang, 'curl'>, (src: string) => PartialParsed> = {
  javascript: parseJavaScript,
  python: parsePython,
  java: parseJava,
  go: parseGo,
  rust: parseRust,
  php: parsePhp,
}

function dedupeHeaders(headers: KeyValue[]): KeyValue[] {
  const out: KeyValue[] = []
  for (const h of headers) {
    const existing = out.find((x) => x.key.toLowerCase() === h.key.toLowerCase())
    if (existing) existing.value = h.value
    else out.push(h)
  }
  return out
}

/** 解析非 cURL 代码片段为 CurlParsed（cURL 请走后端 parse_curl_command）。 */
export function parseCodeSnippet(lang: SnippetLang, src: string): CurlParsed {
  if (lang === 'curl') throw new Error(tFallback('codeimport.curlBackend'))
  const partial = PARSERS[lang](src)
  const headers = dedupeHeaders(partial.headers)
  const ct = headers.find((h) => h.key.toLowerCase() === 'content-type')?.value ?? ''
  const raw = partial.bodyFromObject ?? partial.bodyRaw
  // 对象字面量（JS/Python dict）转 JSON 文本：键加引号（JSON5 宽松写法兼容）。
  let body: BodySpec | null = null
  if (raw !== null && raw.trim()) {
    body = inferBody(partial.bodyFromObject !== null ? normalizeObjectLiteral(raw) : raw, ct)
  }
  // Rust .basic_auth("u", "p") 等可还原凭据的写法 → Basic 认证（其余仍为 none）。
  let auth: AuthSpec = { type: 'none' }
  if (partial.basicAuth) {
    const colon = partial.basicAuth.indexOf(':')
    auth = {
      type: 'basic',
      username: colon === -1 ? partial.basicAuth : partial.basicAuth.slice(0, colon),
      password: colon === -1 ? '' : partial.basicAuth.slice(colon + 1),
    }
  }
  return { url: partial.url, method: partial.method, headers, body, auth }
}

/** JS/Python 对象字面量 → 尽力规整为合法 JSON 文本（裸键加引号、单引号换双引号、去尾逗号）。 */
function normalizeObjectLiteral(src: string): string {
  const inner = src.trim()
  if (looksLikeJson(inner)) {
    try {
      return JSON.stringify(JSON.parse(inner))
    } catch {
      return inner
    }
  }
  let out = inner
  // 裸键加引号：{ name: → { "name":
  out = out.replace(/([{,]\s*)([A-Za-z_$][\w$-]*)\s*:/g, '$1"$2":')
  // 单引号字符串 → 双引号（内容中的双引号转义）
  out = out.replace(/'((?:[^'\\]|\\.)*)'/g, (_, body: string) => `"${body.replace(/"/g, '\\"')}"`)
  // 尾逗号
  out = out.replace(/,(\s*[}\]])/g, '$1')
  try {
    return JSON.stringify(JSON.parse(out))
  } catch {
    return inner
  }
}
