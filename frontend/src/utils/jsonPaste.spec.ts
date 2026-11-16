/**
 * autoFormatJsonPaste 单测：粘贴即美化的三条硬约束——
 * (1) 空文档 / 全选粘贴排版整篇；(2) 文档已有内容时只排版粘贴块；
 * (3) 非 JSON 值、已美化内容一律放行原生粘贴（返回 null）。
 */
import { describe, expect, it } from 'vitest'
import { PASTE_FORMAT_MAX_CHARS, autoFormatJsonPaste } from '@/utils/jsonPaste'

const MINIFIED = '{"name":"miyo","tags":["a","b"],"n":1}'
const PRETTY = '{\n  "name": "miyo",\n  "tags": [\n    "a",\n    "b"\n  ],\n  "n": 1\n}'

describe('autoFormatJsonPaste', () => {
  it('空文档粘贴压缩 JSON：排版整篇，光标落在末尾', () => {
    const out = autoFormatJsonPaste('', 0, 0, MINIFIED)
    expect(out).not.toBeNull()
    expect(out!.text).toBe(PRETTY)
    expect(out!.caret).toBe(PRETTY.length)
  })

  it('全选粘贴（替换旧内容）：排版整篇', () => {
    const doc = '{"old":true}'
    const out = autoFormatJsonPaste(doc, 0, doc.length, MINIFIED)
    expect(out!.text).toBe(PRETTY)
    expect(out!.caret).toBe(PRETTY.length)
  })

  it('文档已有内容时只排版粘贴块，已有内容一个字符不动', () => {
    const doc = '{\n  "a": 1,\n  \n}'
    const caret = doc.indexOf('\n}') // 空行处
    const block = '{\n  "b": [\n    1,\n    2\n  ]\n}'
    const out = autoFormatJsonPaste(doc, caret, caret, '{"b":[1,2]}')
    expect(out).not.toBeNull()
    expect(out!.text).toBe(`{\n  "a": 1,\n  ${block}\n}`)
    expect(out!.caret).toBe(caret + block.length)
  })

  it('粘贴块不是对象/数组：不拦截（纯文本、数字、null）', () => {
    expect(autoFormatJsonPaste('', 0, 0, 'hello')).toBeNull()
    expect(autoFormatJsonPaste('', 0, 0, '123')).toBeNull()
    expect(autoFormatJsonPaste('', 0, 0, 'null')).toBeNull()
  })

  it('粘贴块 JSON 语法错误：不拦截（交给原生粘贴，让它保持原样）', () => {
    expect(autoFormatJsonPaste('', 0, 0, '{"a":}')).toBeNull()
  })

  it('本来就已美化：不拦截，保留原生粘贴的撤销栈', () => {
    expect(autoFormatJsonPaste('', 0, 0, PRETTY)).toBeNull()
  })

  it('数组根也排版', () => {
    const out = autoFormatJsonPaste('', 0, 0, '[1,2]')
    expect(out!.text).toBe('[\n  1,\n  2\n]')
  })

  it('超过阈值不排版（避免大文档粘贴卡顿）', () => {
    const big = `{"a":"${'x'.repeat(PASTE_FORMAT_MAX_CHARS)}"}`
    expect(autoFormatJsonPaste('', 0, 0, big)).toBeNull()
  })

  it('保留重复键与数字原文（与工具栏美化同源）', () => {
    const out = autoFormatJsonPaste('', 0, 0, '{"a":1.50,"A":1,"a":2}')
    expect(out!.text).toBe('{\n  "a": 1.50,\n  "A": 1,\n  "a": 2\n}')
  })

  it('非法选区（越界 / 反向）不崩，按整篇处理', () => {
    const out = autoFormatJsonPaste('', -5, 999, MINIFIED)
    expect(out!.text).toBe(PRETTY)
  })
})
