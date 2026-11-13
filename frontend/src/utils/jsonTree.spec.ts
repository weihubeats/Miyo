/** jsonTree：结构化 Body 编辑器纯函数（解析 / 序列化 / 指针 / sidecar）。 */
import { describe, expect, it } from 'vitest'
import {
  docsFromTree,
  emptyJsonObjectNode,
  parseJsonTree,
  parseJsonTreeWithDocs,
  pointerEscape,
  rebindPointers,
  treeToJsonString,
  treeToPrettyJson,
} from '@/utils/jsonTree'

describe('parseJsonTree', () => {
  it('解析嵌套对象，指针为 RFC6901 转义路径', () => {
    const root = parseJsonTree('{"user":{"name":"张三","a/b":1}}')
    expect(root).not.toBeNull()
    const user = root!.children[0]
    expect(user).toMatchObject({ key: 'user', type: 'object', pointer: '/user' })
    expect(user.children[0]).toMatchObject({ key: 'name', pointer: '/user/name' })
    expect(user.children[1]).toMatchObject({ key: 'a/b', pointer: '/user/a~1b' })
  })

  it('对象数组：子字段指针带 /0 样本路径', () => {
    const root = parseJsonTree('{"items":[{"id":1,"tag":"a"}]}')
    const items = root!.children[0]
    expect(items).toMatchObject({ type: 'array', pointer: '/items' })
    expect(items.children.map((c) => c.pointer)).toEqual(['/items/0/id', '/items/0/tag'])
    expect(items.children[0]).toMatchObject({ type: 'number', value: '1' })
  })

  it('空文本给空对象根；非法 JSON / 非对象根返回 null', () => {
    expect(parseJsonTree('  ')).toMatchObject({ type: 'object', children: [] })
    expect(parseJsonTree('{ bad')).toBeNull()
    expect(parseJsonTree('[1]')).toBeNull()
    expect(parseJsonTree('"str"')).toBeNull()
  })
})

describe('tree 序列化', () => {
  it('roundtrip：对象 + 数组 + 标量', () => {
    const src = { a: 1, b: 'x', c: true, d: null, e: [1, 2], f: [{ k: 'v' }] }
    const root = parseJsonTree(JSON.stringify(src))!
    expect(JSON.parse(treeToJsonString(root))).toEqual(src)
    expect(JSON.parse(treeToPrettyJson(root))).toEqual(src)
  })

  it('对象去重保留首个子字段；空 key 跳过', () => {
    // JSON.parse 本身丢弃重复 key（保留后者），此处手动造重复子字段验证序列化去重
    const root = parseJsonTree('{"dup":1}')!
    root.children.push({ ...root.children[0], key: 'dup', value: '999' })
    root.children.push({ ...root.children[0], key: '', pointer: '' })
    expect(JSON.parse(treeToJsonString(root))).toEqual({ dup: 1 })
  })
})

describe('docs sidecar', () => {
  it('docsFromTree：只导出有描述或非必填的节点', () => {
    const root = parseJsonTree('{"a":1,"b":2}')!
    root.children[0].description = '金额'
    root.children[1].required = false
    expect(docsFromTree(root)).toEqual({
      '/a': { description: '金额', required: true },
      '/b': { description: '', required: false },
    })
  })

  it('parseJsonTreeWithDocs：按指针回填 description/required', () => {
    const root = parseJsonTreeWithDocs('{"a":1}', { '/a': { description: 'x', required: false } })
    expect(root!.children[0]).toMatchObject({ description: 'x', required: false })
    expect(parseJsonTreeWithDocs('{ bad', {})).toBeNull()
  })

  it('pointerEscape：~ 与 /', () => {
    expect(pointerEscape('a~b/c')).toBe('a~0b~1c')
  })
})

describe('rebindPointers / emptyJsonObjectNode', () => {
  it('改名后子树指针重算', () => {
    const root = parseJsonTree('{"old":{"x":1}}')!
    root.children[0].key = 'new'
    const next = rebindPointers(root, '')
    expect(next.children[0].pointer).toBe('/new')
    expect(next.children[0].children[0].pointer).toBe('/new/x')
  })

  it('空对象根可用且序列化为 {}', () => {
    const root = emptyJsonObjectNode()
    expect(treeToJsonString(root)).toBe('{}')
  })
})
