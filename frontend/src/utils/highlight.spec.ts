/**
 * highlight 单测：代码生成面板的多语言轻量高亮——
 * 关键字 / 字符串 / 注释 / flag / 宏着色，且所有文本先 HTML 转义。
 */
import { describe, expect, it } from 'vitest'
import { escapeHtml, highlightCode, highlightJSON, highlightJSONText, jsonTokens } from '@/utils/highlight'

describe('highlightCode：JavaScript', () => {
  it('关键字、字符串、数字分别着色', () => {
    const html = highlightCode('js', `const url = 'https://x';\nawait fetch(url, 1);`)
    expect(html).toContain('<span class="hl-k">const</span>')
    expect(html).toContain('<span class="hl-k">await</span>')
    expect(html).toContain(`<span class="hl-s">'https://x'</span>`)
    expect(html).toContain('<span class="hl-n">1</span>')
  })

  it('行注释整段着色，HTML 特殊字符先转义', () => {
    const html = highlightCode('js', '// a<b & c\nlet x = 1;')
    expect(html).toContain('<span class="hl-c">// a&lt;b &amp; c</span>')
  })
})

describe('highlightCode：cURL / Rust', () => {
  it('cURL：# 注释、-X/--data flag、$ 变量着色', () => {
    const html = highlightCode('curl', `# upload\ncurl -X POST --data '{"a":1}' $URL`)
    expect(html).toContain('<span class="hl-c"># upload</span>')
    expect(html).toContain('<span class="hl-v">-X</span>')
    expect(html).toContain('<span class="hl-v">--data</span>')
    expect(html).toContain('<span class="hl-v">$URL</span>')
    expect(html).toContain(`<span class="hl-s">'{&quot;a&quot;:1}'</span>`)
  })

  it('Rust：fn 关键字与 println! 宏；!= 不吞感叹号', () => {
    const html = highlightCode('rust', `fn main() {\nprintln!("hi");\nassert!(1 != 2);\n}`)
    expect(html).toContain('<span class="hl-k">fn</span>')
    expect(html).toContain('<span class="hl-v">println!</span>')
    expect(html).toContain('<span class="hl-v">assert!</span>')
    expect(html).not.toContain('!=</span>')
  })

  it('未知语言兜底为纯转义', () => {
    expect(highlightCode('unknown' as never, '<p>&')).toBe('&lt;p&gt;&amp;')
  })
})

describe('JSON 语法高亮与搜索标记', () => {
  const sample = JSON.stringify({
    name: 'miyo',
    count: 42,
    active: true,
    extra: null,
  }, null, 2)

  it('jsonTokens 正确分词并打标类名', () => {
    const tokens = jsonTokens(sample)
    const keys = tokens.filter((t) => t.cls === 'hl-k')
    const strs = tokens.filter((t) => t.cls === 'hl-s')
    const nums = tokens.filter((t) => t.cls === 'hl-n')
    const bools = tokens.filter((t) => t.cls === 'hl-b')
    const nulls = tokens.filter((t) => t.cls === 'hl-null')

    expect(keys.length).toBe(4)
    expect(strs.some((t) => t.text.includes('"miyo"'))).toBe(true)
    expect(nums.some((t) => t.text === '42')).toBe(true)
    expect(bools.some((t) => t.text === 'true')).toBe(true)
    expect(nulls.some((t) => t.text === 'null')).toBe(true)
  })

  it('highlightJSON 生成正确的 class 标签并转义 HTML', () => {
    const html = highlightJSON('{"<tag>": "value & more"}')
    expect(html).toContain('<span class="hl-k">&quot;&lt;tag&gt;&quot;:</span>')
    expect(html).toContain('<span class="hl-s">&quot;value &amp; more&quot;</span>')
  })

  it('highlightJSONText 在空 query 时与 highlightJSON 完全一致', () => {
    expect(highlightJSONText(sample, '')).toBe(highlightJSON(sample))
  })

  it('highlightJSONText 搜索时在保留语法高亮 span 的同时插入 mark 标记', () => {
    const html = highlightJSONText(sample, 'name', 0)
    // 依然保留 hl-k 语法着色，且内部包含 active mark
    expect(html).toContain('<span class="hl-k">&quot;<mark class="rp-find-mark active">name</mark>&quot;:</span>')
    // 其它没有匹配的键依然正常语法高亮
    expect(html).toContain('<span class="hl-k">&quot;count&quot;:</span>')
    expect(html).toContain('<span class="hl-n">42</span>')
    expect(html).toContain('<span class="hl-b">true</span>')
  })

  it('highlightJSONText 支持多匹配与 activeMatch 索引高亮', () => {
    const code = '{"miyo_key": "miyo_val", "other": "miyo_again"}'
    const html = highlightJSONText(code, 'miyo', 1)
    // 共有 3 处匹配，第 2 处（index 1）应为 active
    const marks = html.match(/<mark class="[^"]+">miyo<\/mark>/g)
    expect(marks).toHaveLength(3)
    expect(html).toContain('<span class="hl-k">&quot;<mark class="rp-find-mark">miyo</mark>_key&quot;:</span>')
    expect(html).toContain('<span class="hl-s">&quot;<mark class="rp-find-mark active">miyo</mark>_val&quot;</span>')
    expect(html).toContain('<span class="hl-s">&quot;<mark class="rp-find-mark">miyo</mark>_again&quot;</span>')
  })
})

describe('既有工具回归', () => {
  it('escapeHtml / highlightJSON 行为不变', () => {
    expect(escapeHtml('<a href="x">')).toBe('&lt;a href=&quot;x&quot;&gt;')
    expect(highlightJSON('{"a":1}')).toContain('hl-k')
  })
})
