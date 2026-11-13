/**
 * markdownLite 单测：更新日志渲染的结构支持与 XSS 防护。
 * 锁定：标题/列表/行内样式/链接白名单、HTML 转义、代码块原文输出、段内换行。
 */
import { describe, expect, it } from 'vitest'
import { renderMarkdownLite } from '@/utils/markdownLite'

describe('renderMarkdownLite', () => {
  it('空文返回空串', () => {
    expect(renderMarkdownLite('')).toBe('')
    expect(renderMarkdownLite('   \n  ')).toBe('')
  })

  it('标题 / 无序与有序列表 / 粗体 / 行内代码 / 链接', () => {
    const html = renderMarkdownLite(
      [
        '## 亮点',
        '',
        '- 修复 **崩溃** 问题',
        '- 支持 `HTTP/3`',
        '',
        '1. 第一步',
        '2. 第二步',
        '',
        '详见 [发布页](https://example.com/v1?a=1&b=2)',
      ].join('\n'),
    )
    expect(html).toContain('<h4>亮点</h4>')
    expect(html).toContain('<ul>')
    expect(html).toContain('<strong>崩溃</strong>')
    expect(html).toContain('<code>HTTP/3</code>')
    expect(html).toContain('<ol>')
    expect(html.match(/<li>/g)?.length).toBe(4)
    expect(html).toContain('<a href="https://example.com/v1?a=1&amp;b=2" rel="noreferrer">发布页</a>')
  })

  it('段内单换行渲染为 <br>，空行分段', () => {
    const html = renderMarkdownLite('第一行\n第二行\n\n第三段')
    expect(html).toBe('<p>第一行<br>第二行</p><p>第三段</p>')
  })

  it('代码块原文输出，内部标记不解析', () => {
    const html = renderMarkdownLite('```\n**not bold**\n<script>\n```')
    expect(html).toBe('<pre><code>**not bold**\n&lt;script&gt;</code></pre>')
  })

  it('XSS：HTML 先转义，链接协议白名单拦截 javascript:', () => {
    const html = renderMarkdownLite('<img src=x onerror=alert(1)> [点我](javascript:alert(1))')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;img')
    expect(html).not.toContain('<a href="javascript')
    // 协议不匹配白名单 → 保持纯文本（可转义展示，不产生可点链接）
    expect(html).toContain('[点我](javascript:alert(1))')
  })

  it('引用 / 分割线 / 斜体', () => {
    const html = renderMarkdownLite('> 注意事项\n\n---\n\n*斜体*')
    expect(html).toContain('<blockquote>注意事项</blockquote>')
    expect(html).toContain('<hr>')
    expect(html).toContain('<em>斜体</em>')
  })
})
