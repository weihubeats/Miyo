/**
 * xmlFormat.ts：XML 美化与内容类型判定（响应区 Pretty 视图用）。
 *
 * - formatXml：扫描器分词（引号内 `>` 不截断标签）+ 缩进重排；
 *   「短开标签 + 单段文本 + 同名闭合」折叠为单行（`<name>x</name>` 不拆三行）；
 *   闭合标签错配按夹取处理（宁可缩进略歪也不抛错中断预览）；
 * - isXmlContentType：以 xml 结尾的 MIME 主类型与 +xml 后缀（atom/svg/soap…）都算。
 *
 * 刻意不做 XML 校验器：调试场景常拿到残缺报文，格式化只负责「更好读」，
 * 解析失败抛 XmlFormatError 由调用方回退原文。
 */
export class XmlFormatError extends Error {}

/** 单个输出行的最大内联文本长度，超出拆行。 */
const INLINE_TEXT_MAX = 120

interface XmlTag {
  kind: 'open' | 'close' | 'self' | 'meta' // meta = <?xml…?> / <!--…--> / <!DOCTYPE…>
  name: string
  text: string
}

/** 引号感知的单标签扫描（属性值含 `>` 不截断）；注释/CDATA/PI 走各自终止符。 */
function nextTag(text: string, start: number): { end: number; raw: string } {
  // CDATA：以 ]]> 结束（内容里的 > 与引号都无意义）
  if (text.startsWith('<![CDATA[', start)) {
    const end = text.indexOf(']]>', start + 9)
    if (end === -1) throw new XmlFormatError('unclosed CDATA')
    return { end: end + 3, raw: text.slice(start, end + 3) }
  }
  // 注释：以 --> 结束
  if (text.startsWith('<!--', start)) {
    const end = text.indexOf('-->', start + 4)
    if (end === -1) throw new XmlFormatError('unclosed comment')
    return { end: end + 3, raw: text.slice(start, end + 3) }
  }
  let quote: string | null = null
  for (let i = start + 1; i < text.length; i++) {
    const c = text[i]
    if (quote) {
      if (c === quote) quote = null
      continue
    }
    if (c === '"' || c === "'") {
      quote = c
      continue
    }
    if (c === '>') return { end: i + 1, raw: text.slice(start, i + 1) }
  }
  throw new XmlFormatError('unclosed tag')
}

function classifyTag(raw: string): XmlTag {
  const close = /^<\//.test(raw)
  const self = /\/>$/.test(raw)
  const meta = /^<[?!]/.test(raw)
  const nameMatch = raw.match(/^<\/?\s*([\w.:-]+)/)
  return {
    kind: close ? 'close' : self ? 'self' : meta ? 'meta' : 'open',
    name: nameMatch?.[1] ?? '',
    text: raw,
  }
}

/** 把原文切成 标签 / 文本 token 序列。 */
function tokenize(text: string): { tag: XmlTag | null; text: string }[] {
  const out: { tag: XmlTag | null; text: string }[] = []
  let buf = ''
  let i = 0
  while (i < text.length) {
    if (text[i] === '<') {
      if (buf.trim()) out.push({ tag: null, text: buf })
      buf = ''
      const { end, raw } = nextTag(text, i)
      out.push({ tag: classifyTag(raw), text: raw })
      i = end
    } else {
      buf += text[i]
      i++
    }
  }
  if (buf.trim()) out.push({ tag: null, text: buf })
  return out
}

export function formatXml(input: string): string {
  const trimmed = input.trim()
  if (!trimmed.startsWith('<')) throw new XmlFormatError('not xml')

  const tokens = tokenize(trimmed)
  const out: string[] = []
  let depth = 0

  const pad = (): string => '  '.repeat(depth)
  const push = (s: string): void => {
    out.push(pad() + s)
  }

  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i]
    if (!tok.tag) {
      push(tok.text.trim())
      continue
    }
    if (tok.tag.kind === 'close') {
      depth = Math.max(0, depth - 1)
      push(tok.tag.text)
      continue
    }
    if (tok.tag.kind === 'meta' || tok.tag.kind === 'self') {
      push(tok.tag.text)
      continue
    }
    // 开标签：短文本 + 同名闭合 → 折叠单行
    const nextText = tokens[i + 1]
    const nextClose = tokens[i + 2]
    if (
      nextText &&
      !nextText.tag &&
      nextText.text.trim().length <= INLINE_TEXT_MAX &&
      nextClose?.tag?.kind === 'close' &&
      nextClose.tag.name === tok.tag.name
    ) {
      push(`${tok.tag.text}${nextText.text.trim()}${nextClose.tag.text}`)
      i += 2
      continue
    }
    push(tok.tag.text)
    depth++
  }
  return out.join('\n')
}

/** Content-Type 是否为 XML 家族（application/xml、text/xml、*+xml）。 */
export function isXmlContentType(contentType: string): boolean {
  const c = contentType.split(';')[0].trim().toLowerCase()
  return /\/xml$/.test(c) || /\+xml$/.test(c)
}
