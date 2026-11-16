/**
 * markupFormat：XML / HTML 的**保守**排版器（粘贴自动美化用）。
 *
 * 为什么保守：标记语言里空白是有语义的。`<p>hello <b>x</b></p>` 中间那格空格删掉
 * 就变成 `hello x` 粘成一坨，`<pre>` / `<textarea>` / `<script>` / `<style>` 里的
 * 内容更是必须逐字节保留。所以这里只在「元素内部全是元素/注释」的纯结构层加换行
 * 缩进，其余情况一律按原文保留：
 *
 * - 子节点全是元素 / 注释 / CDATA / 纯空白 → 换行 + 2 空格缩进；
 * - 有实际文本（混合内容、单行 `<name>John</name>`）→ 该元素整段按原文输出；
 * - `pre` / `textarea` / `script` / `style` → 整段按原文输出；
 * - 属性、注释、CDATA、`<?xml … ?>` / `<!DOCTYPE …>` 原样保留；
 * - 标签不配对（HTML 常见省略闭合标签的写法、残缺 XML）→ 返回 null，不排版。
 *
 * 只做「重排空白」，不改一个字符的内容：做的就是用户手工做的事，不会破坏请求体语义。
 */

/** HTML 空元素：没有闭合标签也不算解析错误。 */
const VOID_TAGS = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input',
  'link', 'meta', 'param', 'source', 'track', 'wbr',
])

/** 内容逐字节必须保留的元素。 */
const RAW_TEXT_TAGS = new Set(['pre', 'textarea', 'script', 'style'])

type TokKind = 'open' | 'close' | 'text' | 'comment' | 'cdata' | 'pi' | 'doctype'

interface Tok {
  kind: TokKind
  /** 原文片段（text 为其内容，其余含定界符）。 */
  raw: string
  /** 标签名（open / close 有值，其余为空）。 */
  name: string
  selfClosing: boolean
  start: number
  end: number
}

type Node =
  | { kind: 'text'; raw: string }
  | { kind: 'leaf'; raw: string }
  | {
      kind: 'el'
      name: string
      openRaw: string
      closeRaw: string
      selfClosing: boolean
      children: Node[]
      start: number
      end: number
    }

/** 粘贴块是否像标记语言（`<` 开头的对象级内容），不像就放行原生粘贴。 */
export function looksLikeMarkup(src: string): boolean {
  return /^\s*</.test(src)
}

/** 排版 XML / HTML；无法可靠解析（标签不配对等）时返回 null。 */
export function prettyMarkup(src: string): string | null {
  const text = src.trim()
  if (!text) return null
  const toks = tokenize(text)
  if (!toks || !toks.some((t) => t.kind === 'open')) return null
  const roots = buildTree(toks)
  if (!roots) return null
  return roots
    .map((n) => serializeNode(n, 0, text))
    .join('\n')
    .replace(/^\n+|\n+$/g, '')
}

// ---------- 词法 ----------

function tokenize(src: string): Tok[] | null {
  const toks: Tok[] = []
  const n = src.length
  let i = 0
  while (i < n) {
    if (src[i] !== '<') {
      const next = src.indexOf('<', i)
      const stop = next === -1 ? n : next
      toks.push({ kind: 'text', raw: src.slice(i, stop), name: '', selfClosing: false, start: i, end: stop })
      i = stop
      continue
    }
    // 注释 / CDATA / PI / DOCTYPE：各自有独立定界符
    const ended = readDelimited(src, i)
    if (ended) {
      toks.push(ended)
      i = ended.end
      continue
    }
    if (src.startsWith('</', i)) {
      const end = findTagEnd(src, i + 2)
      if (end === -1) return null
      const raw = src.slice(i, end + 1)
      const name = /^<\/\s*([^\s>]+)/.exec(raw)?.[1]
      if (!name) return null
      toks.push({ kind: 'close', raw, name, selfClosing: false, start: i, end: end + 1 })
      i = end + 1
      continue
    }
    const end = findTagEnd(src, i + 1)
    if (end === -1) return null
    const raw = src.slice(i, end + 1)
    const name = /^<\s*([^\s/>]+)/.exec(raw)?.[1]
    if (!name) return null
    toks.push({
      kind: 'open',
      raw,
      name,
      selfClosing: /\/\s*>$/.test(raw),
      start: i,
      end: end + 1,
    })
    i = end + 1
  }
  return toks
}

/** 注释 / CDATA / `<?…?>` / `<!DOCTYPE …>` 的统一处理；不是这四类时返回 null。 */
function readDelimited(src: string, i: number): Tok | null {
  const mk = (kind: TokKind, stop: number): Tok => ({
    kind,
    raw: src.slice(i, stop),
    name: '',
    selfClosing: true,
    start: i,
    end: stop,
  })
  if (src.startsWith('<!--', i)) {
    const end = src.indexOf('-->', i + 4)
    return end === -1 ? null : mk('comment', end + 3)
  }
  if (src.startsWith('<![CDATA[', i)) {
    const end = src.indexOf(']]>', i + 9)
    return end === -1 ? null : mk('cdata', end + 3)
  }
  if (src.startsWith('<?', i)) {
    const end = src.indexOf('?>', i + 2)
    return end === -1 ? null : mk('pi', end + 2)
  }
  if (src.startsWith('<!', i)) {
    const end = findTagEnd(src, i + 2)
    return end === -1 ? null : mk('doctype', end + 1)
  }
  return null
}

/** 找到标签结尾的 `>`（跳过属性值引号内的 `>`）。 */
function findTagEnd(src: string, from: number): number {
  let quote: string | null = null
  for (let i = from; i < src.length; i += 1) {
    const c = src[i]
    if (quote) {
      if (c === quote) quote = null
      continue
    }
    if (c === '"' || c === "'") {
      quote = c
      continue
    }
    if (c === '>') return i
  }
  return -1
}

// ---------- 语法树 ----------

function buildTree(toks: Tok[]): Node[] | null {
  const roots: Node[] = []
  const stack: Array<Extract<Node, { kind: 'el' }>> = []
  const push = (node: Node): void => {
    const top = stack.at(-1)
    if (top) top.children.push(node)
    else roots.push(node)
  }
  for (const tok of toks) {
    if (tok.kind === 'text') {
      push({ kind: 'text', raw: tok.raw })
      continue
    }
    if (tok.kind === 'open') {
      const selfClosing = tok.selfClosing || VOID_TAGS.has(tok.name.toLowerCase())
      const el: Extract<Node, { kind: 'el' }> = {
        kind: 'el',
        name: tok.name,
        openRaw: tok.raw,
        closeRaw: '',
        selfClosing,
        children: [],
        start: tok.start,
        end: tok.end,
      }
      push(el)
      if (!selfClosing) stack.push(el)
      continue
    }
    if (tok.kind === 'close') {
      const top = stack.pop()
      // 不配对（HTML 省略闭合标签、残缺文档）：整体拒绝排版，不猜结构
      if (!top || top.name.toLowerCase() !== tok.name.toLowerCase()) return null
      top.closeRaw = tok.raw
      top.end = tok.end
      continue
    }
    push({ kind: 'leaf', raw: tok.raw })
  }
  if (stack.length) return null
  return roots
}

// ---------- 输出 ----------

function serializeNode(node: Node, depth: number, src: string): string {
  const pad = '  '.repeat(depth)
  if (node.kind === 'text') return pad + node.raw.trim()
  if (node.kind === 'leaf') return pad + node.raw
  return serializeElement(node, depth, src, pad)
}

function serializeElement(
  el: Extract<Node, { kind: 'el' }>,
  depth: number,
  src: string,
  pad: string,
): string {
  // 内容逐字节保留的元素：整段原文输出（内部缩进不动）
  if (RAW_TEXT_TAGS.has(el.name.toLowerCase())) return pad + src.slice(el.start, el.end)
  if (el.selfClosing) return pad + el.openRaw

  const structural = el.children.every((c) => c.kind !== 'text' || c.raw.trim() === '')
  // 非文本子节点（元素 / 注释 / CDATA / PI）：逐行缩进
  const markupKids = el.children.filter((c) => c.kind !== 'text')
  if (structural && markupKids.length) {
    const inner = markupKids.map((c) => serializeNode(c, depth + 1, src)).join('\n')
    return `${pad}${el.openRaw}\n${inner}\n${pad}${el.closeRaw}`
  }
  if (structural) return pad + el.openRaw + el.closeRaw
  // 混合内容 / 实际文本：空白有语义，该元素整段按原文保留
  return pad + src.slice(el.start, el.end)
}
