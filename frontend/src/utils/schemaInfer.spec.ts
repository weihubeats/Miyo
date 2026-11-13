/** schemaInfer：JSON 样本 → 树状 Schema 行（文档预览 Tree Table 数据源）。 */
import { describe, expect, it } from 'vitest'
import { exampleText, inferSchema, mockJsonFromSchema, typeLabelOf } from '@/utils/schemaInfer'

describe('inferSchema 嵌套对象', () => {
  it(' payer/payee 嵌套对象下钻为 children，叶子带示例值', () => {
    const rows = inferSchema({
      orderNo: 'A123',
      amount: 100,
      payer: { userId: 'u1', name: '张三' },
      payee: { userId: 'u2', name: '李四' },
    })
    expect(rows.map((r) => r.name)).toEqual(['orderNo', 'amount', 'payer', 'payee'])
    expect(rows[0]).toMatchObject({ type: 'string', example: 'A123', depth: 0, required: true })
    expect(rows[1]).toMatchObject({ type: 'number', example: '100' })
    expect(rows[2]).toMatchObject({ type: 'object', example: '' })
    expect(rows[2].children.map((c) => c.name)).toEqual(['userId', 'name'])
    expect(rows[2].children[0]).toMatchObject({ type: 'string', depth: 1 })
  })

  it(' 空对象的 children 为空数组', () => {
    const rows = inferSchema({ extra: {} })
    expect(rows[0].type).toBe('object')
    expect(rows[0].children).toEqual([])
  })
})

describe('inferSchema 数组', () => {
  it(' object 数组推断 itemType 并下钻元素字段', () => {
    const rows = inferSchema({
      items: [{ id: 1, tag: 'a' }],
    })
    expect(rows[0].type).toBe('array')
    expect(rows[0].itemType).toBe('object')
    expect(rows[0].children.map((c) => c.name)).toEqual(['id', 'tag'])
    expect(typeLabelOf(rows[0])).toBe('array<object>')
  })

  it(' 标量数组只推断 itemType，不生成 children', () => {
    const rows = inferSchema({ tags: ['a', 'b'] })
    expect(rows[0].itemType).toBe('string')
    expect(rows[0].children).toEqual([])
    expect(typeLabelOf(rows[0])).toBe('array<string>')
  })

  it(' 空数组 / 全 null 数组 itemType 为 undefined（展示为 array<any>）', () => {
    expect(inferSchema({ a: [] })[0].itemType).toBeUndefined()
    expect(inferSchema({ a: [null, null] })[0].itemType).toBeUndefined()
  })
})

describe('inferSchema 标量与根', () => {
  it(' boolean / null 正确分型', () => {
    const rows = inferSchema({ ok: true, gone: null })
    expect(rows[0].type).toBe('boolean')
    expect(rows[1].type).toBe('null')
  })

  it(' 根为数组时包装为单行', () => {
    const rows = inferSchema([1, 2])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({ type: 'array', itemType: 'number' })
  })

  it(' 根为标量时包装为单行', () => {
    expect(inferSchema('hi')).toHaveLength(1)
    expect(inferSchema('hi')[0].type).toBe('string')
  })
})

describe('exampleText', () => {
  it(' 字符串去引号，对象保持 JSON 序列化', () => {
    expect(exampleText('abc')).toBe('abc')
    expect(exampleText({ a: 1 })).toBe('{"a":1}')
  })

  it(' 超长文本按字符截断（中文安全）', () => {
    const long = '支付'.repeat(40)
    const out = exampleText(long)
    expect(out.length).toBeLessThanOrEqual(42)
    expect(out.endsWith('...')).toBe(true)
    expect(Array.from(out).length).toBeLessThanOrEqual(42 + 0)
  })
})

describe('mockJsonFromSchema：Schema → Mock JSON', () => {
  it('叶子取示例值，嵌套 object / 数组递归映射', () => {
    const mock = mockJsonFromSchema(
      inferSchema({
        title: '测试标题',
        body: '测试内容',
        count: 3,
        ok: true,
        tags: ['a'],
        payer: { userId: 'u1' },
        items: [{ id: 1 }],
      }),
    )
    expect(mock).toEqual({
      title: '测试标题',
      body: '测试内容',
      count: 3,
      ok: true,
      tags: ['string'],
      payer: { userId: 'u1' },
      items: [{ id: 1 }],
    })
  })

  it('同名 key 只保留首次出现（重复键样本唯一性保障）', () => {
    // JSON.parse 会丢弃重复键，这里手工构造重名兄弟行验证映射逻辑。
    const rows = inferSchema({ body: 'first' })
    rows.push({ ...rows[0], example: 'second' })
    expect(mockJsonFromSchema(rows)).toEqual({ body: 'first' })
  })

  it('无示例值时按类型给占位值，空输入返回 null', () => {
    expect(mockJsonFromSchema(inferSchema({ s: '', n: 0, b: false, x: null }))).toEqual({
      s: 'string',
      n: 0,
      b: false,
      x: null,
    })
    expect(mockJsonFromSchema([])).toBeNull()
    expect(mockJsonFromSchema(inferSchema({}))).toBeNull()
  })

  it('空数组 / 全 null 数组元素占位为 null', () => {
    expect(mockJsonFromSchema(inferSchema({ a: [] }))).toEqual({ a: [null] })
  })
})
