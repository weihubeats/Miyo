/**
 * queryPaste 单测：query string / 表单正文拆行 + 与已有行的合并规则。
 */
import { describe, expect, it } from 'vitest'
import { mergeQueryPairs, parseQueryPairsPaste } from '@/utils/queryPaste'

describe('parseQueryPairsPaste', () => {
  it('解析 & 分隔的 query string，忽略前导 ?', () => {
    expect(parseQueryPairsPaste('?a=1&b=2')).toEqual([
      { key: 'a', value: '1' },
      { key: 'b', value: '2' },
    ])
  })

  it('支持换行分隔（从表格 / 文档复制的一列）', () => {
    expect(parseQueryPairsPaste('a=1\nb=2\r\nc=3')).toEqual([
      { key: 'a', value: '1' },
      { key: 'b', value: '2' },
      { key: 'c', value: '3' },
    ])
  })

  it('百分号编码与 + 空格解码；解码失败保留原文', () => {
    expect(parseQueryPairsPaste('name=John+Doe&tag=a%2Fb')).toEqual([
      { key: 'name', value: 'John Doe' },
      { key: 'tag', value: 'a/b' },
    ])
    expect(parseQueryPairsPaste('bad=%E4%') /**/).toEqual([{ key: 'bad', value: '%E4%' }])
  })

  it('值里的 = 不被切开；空值保留', () => {
    expect(parseQueryPairsPaste('q=a=b&empty=')).toEqual([
      { key: 'q', value: 'a=b' },
      { key: 'empty', value: '' },
    ])
  })

  it('有空段忽略；有一段不带 = 就整体不认（纯文本 / JSON 不碰）', () => {
    expect(parseQueryPairsPaste('a=1&&b=2')).toEqual([
      { key: 'a', value: '1' },
      { key: 'b', value: '2' },
    ])
    expect(parseQueryPairsPaste('a=1&hello')).toBeNull()
    expect(parseQueryPairsPaste('{"a":1}')).toBeNull()
    expect(parseQueryPairsPaste('   ')).toBeNull()
  })
})

describe('mergeQueryPairs', () => {
  const create = (p: { key: string; value: string }) => ({ key: p.key, value: p.value })

  it('同名键就地更新，新键追加，并回报条数', () => {
    const out = mergeQueryPairs([{ key: 'a', value: 'old' }, { key: 'b', value: '' }], [
      { key: 'a', value: 'new' },
      { key: 'c', value: '3' },
    ], create)
    expect(out.rows).toEqual([
      { key: 'a', value: 'new' },
      { key: 'b', value: '' },
      { key: 'c', value: '3' },
    ])
    expect(out.updated).toBe(1)
    expect(out.added).toBe(1)
  })

  it('不改入参数组', () => {
    const rows = [{ key: 'a', value: 'old' }]
    mergeQueryPairs(rows, [{ key: 'a', value: 'new' }], create)
    expect(rows[0].value).toBe('old')
  })
})
