/**
 * markupFormat 单测：保守排版的边界——纯结构层换行缩进，混合内容 / pre 等
 * 逐字节保留，标签不配对直接拒绝（返回 null，调用方放行原生粘贴）。
 */
import { describe, expect, it } from 'vitest'
import { looksLikeMarkup, prettyMarkup } from '@/utils/markupFormat'

describe('prettyMarkup', () => {
  it('纯结构层：元素逐行缩进，属性与自闭合原样保留', () => {
    expect(prettyMarkup('<root><a href="x">1</a><b/></root>')).toBe(
      '<root>\n  <a href="x">1</a>\n  <b/>\n</root>',
    )
  })

  it('HTML void 元素（br / img）不需要闭合标签', () => {
    expect(prettyMarkup('<div><br><img src="a.png"></div>')).toBe(
      '<div>\n  <br>\n  <img src="a.png">\n</div>',
    )
  })

  it('混合内容（实际文本 + 内联标签）：整段按原文保留，不动空白', () => {
    const src = '<p>hello <b>x</b></p>'
    expect(prettyMarkup(src)).toBe(src)
  })

  it('pre / script / textarea 内容逐字节保留', () => {
    expect(prettyMarkup('<div><pre>a\n   b</pre></div>')).toBe('<div>\n  <pre>a\n   b</pre>\n</div>')
    expect(prettyMarkup('<div><script>if (a) { b() }</script></div>')).toBe(
      '<div>\n  <script>if (a) { b() }</script>\n</div>',
    )
  })

  it('注释 / CDATA / 声明原样保留并独立成行', () => {
    expect(prettyMarkup('<a><!-- c --><b><![CDATA[x<y]]></b></a>')).toBe(
      '<a>\n  <!-- c -->\n  <b>\n    <![CDATA[x<y]]>\n  </b>\n</a>',
    )
    expect(prettyMarkup('<?xml version="1.0"?><a><b>1</b></a>')).toBe(
      '<?xml version="1.0"?>\n<a>\n  <b>1</b>\n</a>',
    )
  })

  it('纯空白子节点被丢弃，空元素挤成一行', () => {
    expect(prettyMarkup('<a>  \n </a>')).toBe('<a></a>')
  })

  it('标签不配对：拒绝排版（返回 null，交给原生粘贴）', () => {
    expect(prettyMarkup('<a><b></a>')).toBeNull()
    expect(prettyMarkup('<a><lib>1</a>')).toBeNull()
  })

  it('没有标签的纯文本：不排版', () => {
    expect(prettyMarkup('hello world')).toBeNull()
    expect(looksLikeMarkup('hello')).toBe(false)
  })

  it('looksLikeMarkup 只认 `<` 开头', () => {
    expect(looksLikeMarkup('  <a/>')).toBe(true)
    expect(looksLikeMarkup('{"a":1}')).toBe(false)
  })
})
