/**
 * jsonFormat：无损 JSON 格式化（美化 / 紧凑）。
 *
 * `JSON.parse` + `JSON.stringify` 往返会丢失信息：重复键只保留最后一个
 * （RFC 8259 解析器通用行为）、数字字面量被规整（1.50 → 1.5、1e3 → 1000）。
 * 请求体编辑器的「格式化」不应静默改变实际发送的内容，因此这里用
 * 保序、保重键、保数字原文的语法树做纯文本排版：输出风格与
 * `JSON.stringify(x, null, 2)` 完全一致，但完整保留上述信息。
 */
import { tFallback } from '@/stores/locale'

/** 解析 / 排版失败（错误消息含位置信息，随当前语言展示）。 */
export class JsonFormatError extends Error {}

type Node =
  | { kind: 'raw'; text: string }
  | { kind: 'string'; value: string }
  | { kind: 'array'; items: Node[] }
  | { kind: 'object'; entries: Array<{ key: string; value: Node }> }

/** 美化（2 空格缩进）。 */
export function prettyJson(src: string): string {
  return serialize(parseJson(src), 0, true)
}

/** 紧凑（无空白）。 */
export function compactJson(src: string): string {
  return serialize(parseJson(src), 0, false)
}

/**
 * 折叠对象重复键（代码生成用）：同名 key 只保留首次出现，嵌套递归处理，
 * 键顺序与其余内容保持原样。解析失败返回 null（由调用方回退原文）。
 *
 * 背景：Body 原文可能因历史数据 / 导入产生 `"body": …` 重复键，
 * 直接嵌入生成的 cURL / JS / Java 代码会出现同一字段多次出现，
 * 这里在「Schema → Mock JSON」映射出口统一保证每个字段唯一。
 */
export function dedupeJsonKeys(src: string): string | null {
  let root: Node
  try {
    root = parseJson(src)
  } catch {
    return null
  }

  const clean = (node: Node): Node => {
    switch (node.kind) {
      case 'object': {
        const seen = new Set<string>()
        const entries: Array<{ key: string; value: Node }> = []
        for (const e of node.entries) {
          if (seen.has(e.key)) continue
          seen.add(e.key)
          entries.push({ key: e.key, value: clean(e.value) })
        }
        return { kind: 'object', entries }
      }
      case 'array':
        return { kind: 'array', items: node.items.map(clean) }
      default:
        return node
    }
  }

  return serialize(clean(root), 0, false)
}

// ---------- 解析（递归下降，保序保重键） ----------

function parseJson(src: string): Node {
  let i = 0
  const n = src.length

  function skipWs(): void {
    while (i < n && (src[i] === ' ' || src[i] === '\t' || src[i] === '\n' || src[i] === '\r')) i += 1
  }

  function fail(msg: string): never {
    throw new JsonFormatError(msg + tFallback('jsonfmt.position', { v: i }))
  }

  /** 数字 / true / false / null：保留原文（避免 1.50 → 1.5 之类的规整）。 */
  const LITERAL_RE = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?|^true|^false|^null/

  function parseString(): string {
    i += 1 // 跳过开引号
    let out = ''
    while (i < n) {
      const c = src[i]
      if (c === '"') {
        i += 1
        return out
      }
      if (c === '\\') {
        const e = src[i + 1]
        if (e === undefined) fail(tFallback('jsonfmt.unterminated'))
        if (e === 'u') {
          const hex = src.slice(i + 2, i + 6)
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) fail(tFallback('jsonfmt.badUnicode'))
          out += String.fromCharCode(parseInt(hex, 16))
          i += 6
          continue
        }
        const simple: Record<string, string> = {
          '"': '"',
          '\\': '\\',
          '/': '/',
          b: '\b',
          f: '\f',
          n: '\n',
          r: '\r',
          t: '\t',
        }
        if (!(e in simple)) fail(tFallback('jsonfmt.badEscape', { v: e }))
        out += simple[e]
        i += 2
        continue
      }
      out += c
      i += 1
    }
    fail(tFallback('jsonfmt.unterminated'))
  }

  function parseValue(): Node {
    skipWs()
    if (i >= n) fail(tFallback('jsonfmt.incomplete'))
    const c = src[i]
    if (c === '{') return parseObject()
    if (c === '[') return parseArray()
    if (c === '"') return { kind: 'string', value: parseString() }
    // 只截取窗口做匹配，避免每个字面量复制整个剩余文本
    const m = LITERAL_RE.exec(src.slice(Math.min(i, n), i + 64))
    if (m) {
      i += m[0].length
      return { kind: 'raw', text: m[0] }
    }
    fail(tFallback('jsonfmt.badChar', { v: c }))
  }

  function parseObject(): Node {
    i += 1 // {
    const entries: Array<{ key: string; value: Node }> = []
    skipWs()
    if (src[i] === '}') {
      i += 1
      return { kind: 'object', entries }
    }
    for (;;) {
      skipWs()
      if (src[i] !== '"') fail(tFallback('jsonfmt.expectKey'))
      const key = parseString()
      skipWs()
      if (src[i] !== ':') fail(tFallback('jsonfmt.expectColon'))
      i += 1
      const value = parseValue()
      entries.push({ key, value })
      skipWs()
      if (src[i] === ',') {
        i += 1
        continue
      }
      if (src[i] === '}') {
        i += 1
        return { kind: 'object', entries }
      }
      fail(tFallback('jsonfmt.expectObjEnd'))
    }
  }

  function parseArray(): Node {
    i += 1 // [
    const items: Node[] = []
    skipWs()
    if (src[i] === ']') {
      i += 1
      return { kind: 'array', items }
    }
    for (;;) {
      items.push(parseValue())
      skipWs()
      if (src[i] === ',') {
        i += 1
        continue
      }
      if (src[i] === ']') {
        i += 1
        return { kind: 'array', items }
      }
      fail(tFallback('jsonfmt.expectArrEnd'))
    }
  }

  const node = parseValue()
  skipWs()
  if (i < n) fail(tFallback('jsonfmt.trailing'))
  return node
}

// ---------- 排版（风格对齐 JSON.stringify） ----------

function serialize(node: Node, depth: number, pretty: boolean): string {
  switch (node.kind) {
    case 'raw':
      return node.text
    case 'string':
      return JSON.stringify(node.value)
    case 'array': {
      if (!node.items.length) return '[]'
      if (!pretty) return `[${node.items.map((it) => serialize(it, depth, false)).join(',')}]`
      const pad = '  '.repeat(depth + 1)
      const inner = node.items.map((it) => pad + serialize(it, depth + 1, true)).join(',\n')
      return `[\n${inner}\n${'  '.repeat(depth)}]`
    }
    case 'object': {
      if (!node.entries.length) return '{}'
      if (!pretty) {
        return `{${node.entries
          .map((e) => `${JSON.stringify(e.key)}:${serialize(e.value, depth, false)}`)
          .join(',')}}`
      }
      const pad = '  '.repeat(depth + 1)
      const inner = node.entries
        .map((e) => `${pad}${JSON.stringify(e.key)}: ${serialize(e.value, depth + 1, true)}`)
        .join(',\n')
      return `{\n${inner}\n${'  '.repeat(depth)}}`
    }
  }
}
