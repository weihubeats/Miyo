/**
 * highlight.ts：轻量语法高亮工具（正则分词，无第三方依赖）。
 *
 * - escapeHtml / highlightJSON / highlightGraphQL 从 GraphQLView.vue 提取为共享实现；
 * - 输出 HTML 片段，配合调用方作用域内的 .hl-* 颜色类（.hl-s/.hl-k/.hl-n/.hl-b/.hl-c/.hl-v/.hl-p）。
 */
import type { CodeLang } from '../types/miyoApi'

/** HTML 转义（所有高亮输出必须先转义再包 span）。 */
export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

const GRAPHQL_KEYWORDS =
  'query|mutation|subscription|fragment|on|schema|scalar|type|interface|union|enum|input|implements|directive|extend|true|false|null'
/** 模块级预构建：原实现每个标识符 token 都 split('|') 一次再 includes。 */
const GRAPHQL_KEYWORD_SET: ReadonlySet<string> = new Set(GRAPHQL_KEYWORDS.split('|'))

/** 轻量 GraphQL 高亮（注释 / 字符串 / 变量 / 关键字 / 数字）。 */
export function highlightGraphQL(code: string): string {
  const re =
    /(#[^\n]*)|("(?:[^"\\]|\\.)*"|"""(?:.|\n)*?""")|(\$[A-Za-z_][A-Za-z0-9_]*)|([A-Za-z_][A-Za-z0-9_]*)|(-?\b\d+(?:\.\d+)?\b)|([{}\[\]():!=\|&,.])/g
  let out = ''
  let last = 0
  for (const m of code.matchAll(re)) {
    out += escapeHtml(code.slice(last, m.index))
    const [full, comment, str, variable, ident, num] = m
    if (comment) out += `<span class="hl-c">${escapeHtml(full)}</span>`
    else if (str) out += `<span class="hl-s">${escapeHtml(full)}</span>`
    else if (variable) out += `<span class="hl-v">${escapeHtml(full)}</span>`
    else if (ident) {
      if (GRAPHQL_KEYWORD_SET.has(ident)) {
        out += `<span class="hl-k">${escapeHtml(full)}</span>`
      } else {
        out += escapeHtml(full)
      }
    } else if (num) out += `<span class="hl-n">${escapeHtml(full)}</span>`
    else out += `<span class="hl-p">${escapeHtml(full)}</span>`
    last = m.index! + full.length
  }
  out += escapeHtml(code.slice(last))
  return out
}

/** JSON 词法片段：文本 + 着色类（'' 表示不着色）。 */
export interface JsonToken {
  text: string
  cls: string
}

const JSON_RE =
  /("(?:[^"\\]|\\.)*")(\s*:)?|(-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|(true|false)|(\bnull\b)|([{}\[\],])/g

/** JSON 分词：键 / 字符串 / 数字 / 布尔 / null / 标点（请求编辑器与响应视图共用）。 */
export function jsonTokens(code: string): JsonToken[] {
  const out: JsonToken[] = []
  let last = 0
  for (const m of code.matchAll(JSON_RE)) {
    const head = code.slice(last, m.index)
    if (head) out.push({ text: head, cls: '' })
    const [full, str, colon, num, bool, nul, punct] = m
    if (str) out.push({ text: full, cls: colon ? 'hl-k' : 'hl-s' })
    else if (num) out.push({ text: full, cls: 'hl-n' })
    else if (bool) out.push({ text: full, cls: 'hl-b' })
    else if (nul) out.push({ text: full, cls: 'hl-null' })
    else if (punct) out.push({ text: full, cls: 'hl-p' })
    last = m.index! + full.length
  }
  const tail = code.slice(last)
  if (tail) out.push({ text: tail, cls: '' })
  return out
}

/** 轻量 JSON 高亮（键 / 字符串 / 数字 / 布尔 / null / 标点）。 */
export function highlightJSON(code: string): string {
  return jsonTokens(code)
    .map((t) => (t.cls ? `<span class="${t.cls}">${escapeHtml(t.text)}</span>` : escapeHtml(t.text)))
    .join('')
}

/** JSON 高亮 + 查找标记（响应行视图与请求编辑器用；空 query 时退化为纯高亮；activeMatch >= 0 时当前项附加 active 类）。 */
export function highlightJSONText(code: string, query: string, activeMatch?: number): string {
  if (!query) return highlightJSON(code)
  const ql = query.toLowerCase()
  const lower = code.toLowerCase()

  // 1. 提取全局所有匹配区间
  const matches: Array<{ start: number; end: number; index: number }> = []
  let from = 0
  let matchIdx = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) break
    matches.push({ start: idx, end: idx + query.length, index: matchIdx })
    matchIdx += 1
    from = idx + query.length
  }
  if (!matches.length) return highlightJSON(code)

  // 2. 词法分词（完整 JSON）
  const tokens = jsonTokens(code)

  // 3. 逐 token 渲染高亮与查找 mark
  let currentOffset = 0
  let out = ''

  for (const t of tokens) {
    const tokStart = currentOffset
    const tokEnd = tokStart + t.text.length
    currentOffset = tokEnd

    // 检查是否有匹配与该 token 重叠
    const overlapping = matches.filter((m) => m.start < tokEnd && m.end > tokStart)

    let tokenHtml = ''
    if (!overlapping.length) {
      tokenHtml = escapeHtml(t.text)
    } else {
      let relFrom = 0
      for (const m of overlapping) {
        const matchRelStart = Math.max(0, m.start - tokStart)
        const matchRelEnd = Math.min(t.text.length, m.end - tokStart)

        if (matchRelStart > relFrom) {
          tokenHtml += escapeHtml(t.text.slice(relFrom, matchRelStart))
        }

        const isActive = activeMatch !== undefined && m.index === activeMatch
        const cls = isActive ? 'rp-find-mark active' : 'rp-find-mark'
        tokenHtml += `<mark class="${cls}">${escapeHtml(t.text.slice(matchRelStart, matchRelEnd))}</mark>`
        relFrom = matchRelEnd
      }
      if (relFrom < t.text.length) {
        tokenHtml += escapeHtml(t.text.slice(relFrom))
      }
    }

    if (t.cls) {
      out += `<span class="${t.cls}">${tokenHtml}</span>`
    } else {
      out += tokenHtml
    }
  }

  return out
}

/* ==========================================================================
 * XML 高亮（响应区 XML Pretty 行视图）：注释 / CDATA / 声明 / 标签名 /
 * 属性值着色，正文默认色。与 JSON 同思路——单遍正则分词 + 先转义再包 span。
 * ========================================================================== */

const XML_RE =
  /(<!--[\s\S]*?-->)|(<!\[CDATA\[[\s\S]*?\]\]>)|(<\?[\s\S]*?\?>)|(<\/?)([A-Za-z_][\w.:_-]*)((?:\s+[A-Za-z_][\w.:_-]*\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*(\/?>)/g

/** 轻量 XML 高亮：标签名 hl-k、属性值 hl-s、注释/CDATA/声明 hl-c、括号 hl-p。 */
export function highlightXML(code: string): string {
  let out = ''
  let last = 0
  for (const m of code.matchAll(XML_RE)) {
    out += escapeHtml(code.slice(last, m.index))
    const [full, comment, cdata, decl, open, name, attrs, close] = m
    if (comment || cdata || decl) {
      out += `<span class="hl-c">${escapeHtml(full)}</span>`
    } else {
      out += `<span class="hl-p">${escapeHtml(open ?? '')}</span>`
      out += `<span class="hl-k">${escapeHtml(name ?? '')}</span>`
      if (attrs) {
        // 属性串逐段拆：名默认色、= hl-p、值 hl-s
        const attrRe = /([A-Za-z_][\w.:_-]*)(\s*=\s*)("[^"]*"|'[^']*')/g
        let pos = 0
        for (const am of attrs.matchAll(attrRe)) {
          out += escapeHtml(attrs.slice(pos, am.index))
          out += escapeHtml(am[1] ?? '')
          out += `<span class="hl-p">${escapeHtml(am[2] ?? '')}</span>`
          out += `<span class="hl-s">${escapeHtml(am[3] ?? '')}</span>`
          pos = am.index! + am[0].length
        }
        out += escapeHtml(attrs.slice(pos))
      }
      out += `<span class="hl-p">${escapeHtml(close ?? '')}</span>`
    }
    last = m.index! + full.length
  }
  out += escapeHtml(code.slice(last))
  return out
}

/** XML 高亮 + 查找标记（结构与 highlightJSONText 相同：先全局匹配区间，再逐 token 渲染）。 */
export function highlightXMLText(code: string, query: string, activeMatch?: number): string {
  if (!query) return highlightXML(code)
  const ql = query.toLowerCase()
  const lower = code.toLowerCase()

  const matches: Array<{ start: number; end: number; index: number }> = []
  let from = 0
  let matchIdx = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) break
    matches.push({ start: idx, end: idx + query.length, index: matchIdx })
    matchIdx += 1
    from = idx + query.length
  }
  if (!matches.length) return highlightXML(code)

  // 逐段推进：用 XML_RE 分词并按偏移叠加查找 mark
  let currentOffset = 0
  let out = ''
  type Seg = { text: string; cls: string | null }
  const segments: Seg[] = []
  let last = 0
  for (const m of code.matchAll(XML_RE)) {
    if (m.index! > last) segments.push({ text: code.slice(last, m.index), cls: null })
    const [full, comment, cdata, decl, open, name, attrs, close] = m
    if (comment || cdata || decl) {
      segments.push({ text: full, cls: 'hl-c' })
    } else {
      segments.push({ text: open ?? '', cls: 'hl-p' })
      segments.push({ text: name ?? '', cls: 'hl-k' })
      if (attrs) {
        const attrRe = /([A-Za-z_][\w.:_-]*)(\s*=\s*)("[^"]*"|'[^']*')/g
        let pos = 0
        for (const am of attrs.matchAll(attrRe)) {
          segments.push({ text: attrs.slice(pos, am.index), cls: null })
          segments.push({ text: am[1] ?? '', cls: null })
          segments.push({ text: am[2] ?? '', cls: 'hl-p' })
          segments.push({ text: am[3] ?? '', cls: 'hl-s' })
          pos = am.index! + am[0].length
        }
        segments.push({ text: attrs.slice(pos), cls: null })
      }
      segments.push({ text: close ?? '', cls: 'hl-p' })
    }
    last = m.index! + full.length
  }
  if (last < code.length) segments.push({ text: code.slice(last), cls: null })

  for (const seg of segments) {
    const tokStart = currentOffset
    const tokEnd = tokStart + seg.text.length
    currentOffset = tokEnd
    const overlapping = matches.filter((m) => m.start < tokEnd && m.end > tokStart)
    let tokenHtml = ''
    if (!overlapping.length) {
      tokenHtml = escapeHtml(seg.text)
    } else {
      let relFrom = 0
      for (const m of overlapping) {
        const s = Math.max(0, m.start - tokStart)
        const e = Math.min(seg.text.length, m.end - tokStart)
        if (s > relFrom) tokenHtml += escapeHtml(seg.text.slice(relFrom, s))
        const cls = activeMatch !== undefined && m.index === activeMatch ? 'rp-find-mark active' : 'rp-find-mark'
        tokenHtml += `<mark class="${cls}">${escapeHtml(seg.text.slice(s, e))}</mark>`
        relFrom = e
      }
      if (relFrom < seg.text.length) tokenHtml += escapeHtml(seg.text.slice(relFrom))
    }
    out += seg.cls ? `<span class="${seg.cls}">${tokenHtml}</span>` : tokenHtml
  }
  return out
}

/* ==========================================================================
 * 代码生成高亮（cURL / JavaScript / Java / Go / Rust …）
 * 与 JSON/GraphQL 同思路：单遍正则分词 + 先转义再包 span；
 * 分类依据匹配文本本身（首字符 / 关键字表），因此只需一个捕获组。
 * ========================================================================== */

interface CodeLangSpec {
  /** 关键字（区分大小写）。 */
  keywords: ReadonlySet<string>
  /** 字面量 true/false/null/nil/None…（hl-b）。 */
  literals: ReadonlySet<string>
  /** 行注释起始（`//` 或 `#`）。 */
  lineComment: ReadonlySet<string>
  /** 支持 /* 块注释。 */
  blockComment: boolean
  /** 字符串字面量模式（按序尝试，命中即整段 hl-s）。 */
  strings: readonly string[]
  /** 支持 @Annotation（Java / PHP）。 */
  annotation: boolean
  /** 支持 #[…] 属性（Rust / PHP8，hl-v，优先于 # 行注释）。 */
  hashAttr: boolean
  /** 支持 $var（shell / PHP，hl-v）。 */
  shellVar: boolean
  /** 支持 -x / --flag（shell，hl-v）。 */
  flags: boolean
  /** 支持 ident! 宏调用（Rust，hl-v）。 */
  macro: boolean
  /** 标识符首字符集扩展（如 $）。 */
  identHead: string
}

const w = (s: string): ReadonlySet<string> => new Set(s.split(' '))

const CODE_LANG_SPECS: Record<CodeLang, CodeLangSpec> = {
  curl: {
    keywords: w('curl echo if then elif else fi while for do done in exit'),
    literals: w('true false'),
    lineComment: w('#'),
    blockComment: false,
    strings: ['"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: false,
    hashAttr: false,
    shellVar: true,
    flags: true,
    macro: false,
    identHead: '',
  },
  js: {
    keywords: w(
      'const let var function return async await if else for while do switch case break continue new class extends super import from export default try catch finally throw typeof instanceof of in delete yield static get set this void',
    ),
    literals: w('true false null undefined NaN'),
    lineComment: w('//'),
    blockComment: true,
    strings: ['`(?:[^`\\\\]|\\\\.)*`', '"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: false,
    hashAttr: false,
    shellVar: false,
    flags: false,
    macro: false,
    identHead: '',
  },
  java: {
    keywords: w(
      'public private protected static final void int long double float boolean char byte short class interface enum record extends implements import package new return if else for while do switch case break continue try catch finally throws throw this super abstract synchronized volatile transient instanceof default assert var sealed permits yield native strictfp',
    ),
    literals: w('true false null'),
    lineComment: w('//'),
    blockComment: true,
    strings: ['"""[\\s\\S]*?"""', '"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: true,
    hashAttr: false,
    shellVar: false,
    flags: false,
    macro: false,
    identHead: '',
  },
  go: {
    keywords: w(
      'func package import var const type struct interface map chan go defer select switch case default if else for range return break continue fallthrough goto',
    ),
    literals: w('true false nil iota'),
    lineComment: w('//'),
    blockComment: true,
    strings: ['`(?:[^`])*`', '"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: false,
    hashAttr: false,
    shellVar: false,
    flags: false,
    macro: false,
    identHead: '',
  },
  rust: {
    keywords: w(
      'fn let mut pub struct enum impl trait match if else for while loop return use mod crate self super as dyn move ref where type in break continue extern unsafe async await const static',
    ),
    literals: w('true false None Some Ok Err'),
    lineComment: w('//'),
    blockComment: true,
    strings: ['"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: false,
    hashAttr: true,
    shellVar: false,
    flags: false,
    macro: true,
    identHead: '',
  },
  python: {
    keywords: w(
      'import from def return class if elif else for while in not and or with as try except finally raise pass lambda global nonlocal assert del yield match case',
    ),
    literals: w('True False None'),
    lineComment: w('#'),
    blockComment: false,
    strings: ['"""[\\s\\S]*?"""', "'''[\\s\\S]*?'''", '"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: false,
    hashAttr: false,
    shellVar: false,
    flags: false,
    macro: false,
    identHead: '',
  },
  php: {
    keywords: w(
      'function return public private protected static class extends implements interface namespace use new echo print if else elseif foreach for while switch case break continue try catch finally throw instanceof array list isset unset require include as',
    ),
    literals: w('true false null TRUE FALSE NULL'),
    lineComment: w('// #'),
    blockComment: true,
    strings: ['"(?:[^"\\\\\\n]|\\\\.)*"', "'(?:[^'\\\\\\n]|\\\\.)*'"],
    annotation: true,
    hashAttr: true,
    shellVar: true,
    flags: false,
    macro: false,
    identHead: '',
  },
}

/** 每语言编译一次分词正则（单捕获组，分类在 wrap 时按文本判定）。 */
const CODE_RE_CACHE = new Map<CodeLang, RegExp>()

function codeLangRe(lang: CodeLang): RegExp {
  const cached = CODE_RE_CACHE.get(lang)
  if (cached) return cached
  const spec = CODE_LANG_SPECS[lang]
  const parts: string[] = []
  if (spec.blockComment) parts.push('/\\*[\\s\\S]*?\\*/')
  const lineStarts = [...spec.lineComment].join('|')
  if (lineStarts) parts.push(`(?:${lineStarts})[^\\n]*`)
  if (spec.strings.length) parts.push(`(?:${spec.strings.join('|')})`)
  if (spec.annotation) parts.push('@[A-Za-z_][A-Za-z0-9_]*')
  if (spec.hashAttr) parts.push('#!?\\[[^\\n]*')
  if (spec.shellVar) parts.push('\\$\\{?[A-Za-z_][A-Za-z0-9_]*\\}?')
  if (spec.flags) parts.push('-{1,2}[A-Za-z][A-Za-z0-9_-]*')
  const bang = spec.macro ? '!?(?!=)' : ''
  parts.push(`[A-Za-z_${spec.identHead}][A-Za-z0-9_${spec.identHead}]*${bang}`)
  parts.push('\\b\\d[A-Za-z0-9_]*(?:\\.\\d+)?(?:[eE][+-]?\\d+)?')
  parts.push(`[{}()\\[\\];,.:<>=+\\-*/%!?&|^~@$#]`)
  const re = new RegExp(parts.join('|'), 'g')
  CODE_RE_CACHE.set(lang, re)
  return re
}

const spanOf = (cls: string, text: string): string => `<span class="${cls}">${escapeHtml(text)}</span>`

/** 按匹配文本归类着色（见 codeLangRe 的分组约定）。 */
function classifyCodeToken(text: string, spec: CodeLangSpec): string {
  const c = text[0]
  if (text.startsWith('/*') || text.startsWith('//')) return spanOf('hl-c', text)
  if (c === '#') {
    if (spec.hashAttr && (text.startsWith('#[') || text.startsWith('#!['))) return spanOf('hl-v', text)
    return spanOf('hl-c', text)
  }
  if (c === '"' || c === "'" || c === '`') return spanOf('hl-s', text)
  if (c === '@' || c === '$') return spanOf('hl-v', text)
  if (/^-{1,2}[A-Za-z]/.test(text)) return spanOf('hl-v', text)
  if (c >= '0' && c <= '9') return spanOf('hl-n', text)
  if (/[A-Za-z_]/.test(c)) {
    if (spec.macro && text.endsWith('!')) return spanOf('hl-v', text)
    const word = text.endsWith('!') ? text.slice(0, -1) : text
    if (spec.literals.has(word)) return spanOf('hl-b', text)
    if (spec.keywords.has(word)) return spanOf('hl-k', text)
    return escapeHtml(text)
  }
  return spanOf('hl-p', text)
}

/** 代码生成面板高亮：lang 为后端 CodeLang，未知语言原样转义返回。 */
export function highlightCode(lang: CodeLang, code: string): string {
  const spec = CODE_LANG_SPECS[lang]
  if (!spec) return escapeHtml(code)
  const re = codeLangRe(lang)
  re.lastIndex = 0
  let out = ''
  let last = 0
  for (const m of code.matchAll(re)) {
    out += escapeHtml(code.slice(last, m.index))
    out += classifyCodeToken(m[0], spec)
    last = m.index! + m[0].length
  }
  out += escapeHtml(code.slice(last))
  return out
}
