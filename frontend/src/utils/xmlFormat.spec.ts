/**
 * xmlFormat 单测：缩进重排、短文本内联、引号内 `>` 不截断标签、
 * 残缺报文回退、Content-Type 判定。
 */
import { describe, expect, it } from 'vitest'
import { formatXml, isXmlContentType, XmlFormatError } from '@/utils/xmlFormat'

describe('formatXml', () => {
  it('压缩单行展开为缩进结构；短文本键值对内联', () => {
    const out = formatXml('<root><user><name>张三</name><age>18</age></user><bio>very long text</bio></root>')
    expect(out).toBe(
      [
        '<root>',
        '  <user>',
        '    <name>张三</name>',
        '    <age>18</age>',
        '  </user>',
        '  <bio>very long text</bio>',
        '</root>',
      ].join('\n'),
    )
  })

  it('属性值中的 > 不截断标签；自闭合与声明不增减深度', () => {
    const out = formatXml('<?xml version="1.0"?><a x="1>0"><b/><c/></a>')
    expect(out).toBe(
      ['<?xml version="1.0"?>', '<a x="1>0">', '  <b/>', '  <c/>', '</a>'].join('\n'),
    )
  })

  it('多行注释 / CDATA 保持整段', () => {
    const out = formatXml('<a><!-- 多行\n注释 --><t><![CDATA[x > y]]></t></a>')
    expect(out).toContain('<!-- 多行\n注释 -->')
    expect(out).toContain('<![CDATA[x > y]]>')
  })

  it('超长文本不内联（拆行）', () => {
    const long = 'x'.repeat(200)
    const out = formatXml(`<a><t>${long}</t></a>`)
    expect(out).toBe(['<a>', '  <t>', `    ${long}`, '  </t>', '</a>'].join('\n'))
  })

  it('非 XML（不以 < 开头）抛 XmlFormatError', () => {
    expect(() => formatXml('hello world')).toThrow(XmlFormatError)
  })
})

describe('isXmlContentType', () => {
  it('识别 application/xml、text/xml（带 charset）、*+xml 家族；JSON 不算', () => {
    expect(isXmlContentType('application/xml')).toBe(true)
    expect(isXmlContentType('text/xml; charset=utf-8')).toBe(true)
    expect(isXmlContentType('application/atom+xml')).toBe(true)
    expect(isXmlContentType('image/svg+xml')).toBe(true)
    expect(isXmlContentType('application/json')).toBe(false)
    expect(isXmlContentType('text/html')).toBe(false)
    expect(isXmlContentType('')).toBe(false)
  })
})
