/**
 * jsonCompletion 单测：JSON 补全位置判定（键/值/不弹）与同文键收集。
 * 覆盖：对象键位、冒号后值位、数组元素值位、字符串内编辑（键 vs 值消歧）、
 * 嵌套/转义/字符串收尾后（无候选）等边界。
 */
import { describe, expect, it } from 'vitest'
import { collectJsonKeys, scanJsonCompletionSlot } from './jsonCompletion'

/** 构造「光标前文本」：竖线标记光标位，光标之后的模板文本不参与扫描。 */
function scan(tpl: string): ReturnType<typeof scanJsonCompletionSlot> {
  const before = tpl.slice(0, tpl.indexOf('|'))
  return scanJsonCompletionSlot(before)
}

describe('scanJsonCompletionSlot：键位', () => {
  it('{ 后、对象逗号后为键位（引号外）', () => {
    expect(scan('{|').slot).toBe('key')
    expect(scan('{"a": 1,|').slot).toBe('key')
    expect(scan('{ "a": 1, |').slot).toBe('key')
  })

  it('键名字符串内为键位（含编辑已有键中段）', () => {
    const r = scan('{"na|')
    expect(r.slot).toBe('key')
    expect(r.stringStart).toBe(1)

    const r2 = scan('{"a": 1, "na|me"')
    expect(r2.slot).toBe('key')
    expect(r2.stringStart).toBe(9)
  })
})

describe('scanJsonCompletionSlot：值位', () => {
  it('冒号后为值位；数组内（元素/逗号后）为值位', () => {
    expect(scan('{"a": |').slot).toBe('value')
    expect(scan('[|').slot).toBe('value')
    expect(scan('[1, |').slot).toBe('value')
  })

  it('数组内的字符串元素永远是值位（无冒号消歧）', () => {
    const r = scan('["a", "b|')
    expect(r.slot).toBe('value')
  })

  it('值字符串内编辑为值位，但不弹字面量（source 层处理为 null）', () => {
    const r = scan('{"a": "he|')
    expect(r.slot).toBe('value')
    expect(r.stringStart).toBe(6)
  })
})

describe('scanJsonCompletionSlot：不弹', () => {
  it('完整值之后（逗号前）、闭合后顶层不弹', () => {
    expect(scan('{"a": 1|').slot).toBe('none')
    expect(scan('{"a": 1} |').slot).toBe('none')
    expect(scan('').slot).toBe('none')
  })

  it('嵌套括号深度正确（闭合并重新进入键位）', () => {
    expect(scan('{"a": {"b": 2}, |').slot).toBe('key')
    expect(scan('{"a": [1, 2], |').slot).toBe('key')
  })

  it('字符串内的转义引号不破坏状态判定', () => {
    const r = scan('{"a": "say \\"hi|"')
    expect(r.slot).toBe('value')
  })
})

describe('collectJsonKeys', () => {
  it('按出现顺序去重收集键；转义键不炸', () => {
    const doc = '{"b": 1, "a": {"b": 2, "c": "x"}, "d": "not a key: nope"}'
    expect(collectJsonKeys(doc)).toEqual(['b', 'a', 'c', 'd'])
  })

  it('值字符串里的 "x": 形态会被误收（可接受：候选集宁多勿漏）', () => {
    expect(collectJsonKeys('{"a": "p: q"}')).toEqual(['a'])
  })
})
