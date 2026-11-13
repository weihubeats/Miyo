/**
 * jsonFormat 单测：无损格式化——保留重复键、键顺序与数字原文；
 * 对无重复键的常规 JSON，输出与 JSON.stringify 风格完全一致。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { JsonFormatError, compactJson, dedupeJsonKeys, prettyJson } from '@/utils/jsonFormat'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 错误文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

describe('prettyJson：无损性', () => {
  it('保留重复键（用户场景：多个 "body" 键格式化后不丢失）', () => {
    const src =
      '{"title":"测试标题","body":"测试内容","body":"测试内容","body":"测试内容","body":"测试内容","userId":1}'
    expect(prettyJson(src)).toBe(`{
  "title": "测试标题",
  "body": "测试内容",
  "body": "测试内容",
  "body": "测试内容",
  "body": "测试内容",
  "userId": 1
}`)
  })

  it('保留键顺序', () => {
    expect(prettyJson('{"z":1,"a":2,"m":3}')).toBe(`{
  "z": 1,
  "a": 2,
  "m": 3
}`)
  })

  it('保留数字原文（1.50 / 1e3 不被规整）', () => {
    expect(prettyJson('{"price":1.50,"exp":1e3}')).toBe(`{
  "price": 1.50,
  "exp": 1e3
}`)
  })

  it('嵌套结构与空容器排版正确', () => {
    expect(prettyJson('{"a":[1,{"b":[]}],"c":{},"d":null,"e":true}')).toBe(`{
  "a": [
    1,
    {
      "b": []
    }
  ],
  "c": {},
  "d": null,
  "e": true
}`)
  })

  it('字符串转义往返（\\u 转义、中文、特殊字符）', () => {
    expect(prettyJson('{"s":"a\\u0041ä行\\n\\t\\"q\\""}')).toBe(
      '{\n  "s": "aAä行\\n\\t\\"q\\""\n}',
    )
  })
})

describe('compactJson', () => {
  it('紧凑输出且保留重复键', () => {
    expect(compactJson('{ "a" : 1, "a": 2, "b": [1, 2] }')).toBe('{"a":1,"a":2,"b":[1,2]}')
  })
})

describe('dedupeJsonKeys：代码生成的重复键折叠', () => {
  it('同名 key 只保留首次出现（回归：4 个 "body" 折叠为 1 个）', () => {
    const src =
      '{"title":"测试标题","body":"测试内容","body":"测试内容","body":"测试内容","body":"测试内容","userId":1}'
    expect(dedupeJsonKeys(src)).toBe('{"title":"测试标题","body":"测试内容","userId":1}')
  })

  it('嵌套对象与数组内递归去重，键顺序不变', () => {
    expect(dedupeJsonKeys('{"a":{"k":1,"k":2},"arr":[{"x":1,"x":9}],"a":{"z":3}}')).toBe(
      '{"a":{"k":1},"arr":[{"x":1}]}',
    )
  })

  it('无重复键时输出紧凑等价 JSON，数字保留原文', () => {
    expect(dedupeJsonKeys('{ "price": 1.50, "tags": ["a"] }')).toBe('{"price":1.50,"tags":["a"]}')
  })

  it('非 JSON 输入返回 null（调用方回退原文）', () => {
    expect(dedupeJsonKeys('not json')).toBeNull()
    expect(dedupeJsonKeys('')).toBeNull()
  })
})

describe('与 JSON.stringify 的等价性（无重复键、常规数字时）', () => {
  it.each([
    '{"a":1,"b":[1,2,3],"c":{"d":"x","e":false}}',
    '[{"id":1,"tags":["a","b"]},{"id":2}]',
    '"top-level string"',
    '42',
    'true',
    'null',
    '{}',
    '[]',
  ])('%s → 输出一致', (src) => {
    expect(prettyJson(src)).toBe(JSON.stringify(JSON.parse(src), null, 2))
    expect(compactJson(src)).toBe(JSON.stringify(JSON.parse(src)))
  })
})

describe('非法输入', () => {
  it('缺少值 / 尾逗号 / 多余内容 / 未闭合字符串 均报中文位置', () => {
    expect(() => prettyJson('{"a":}')).toThrow(JsonFormatError)
    expect(() => prettyJson('{"a":1,}')).toThrow(JsonFormatError)
    expect(() => prettyJson('{} x')).toThrow(/多余内容/)
    expect(() => prettyJson('{"a":"未闭合')).toThrow(/未闭合/)
    expect(() => prettyJson('[1,]')).toThrow(JsonFormatError)
  })
})
