/**
 * base64 工具单测：UTF-8 安全编解码往返 + 非法输入安全兜底。
 * ws_send 的 binary/ping 帧契约（后端要求 base64）依赖这两个函数。
 */
import { describe, expect, it } from 'vitest'
import { fromBase64Utf8, fromBase64Utf8Safe, toBase64Utf8 } from '@/utils/base64'

describe('base64Utf8', () => {
  it('ASCII 往返', () => {
    expect(toBase64Utf8('hello')).toBe(btoa('hello'))
    expect(fromBase64Utf8(toBase64Utf8('hello'))).toBe('hello')
  })

  it('中文 / emoji 往返（btoa 直接编码会抛错）', () => {
    const raw = '中文帧 ✅ {"amount": 100}'
    expect(() => btoa(raw)).toThrow()
    expect(fromBase64Utf8(toBase64Utf8(raw))).toBe(raw)
  })

  it('空串', () => {
    expect(toBase64Utf8('')).toBe('')
    expect(fromBase64Utf8('')).toBe('')
  })

  it('fromBase64Utf8Safe：非法输入回退原文不抛异常', () => {
    expect(fromBase64Utf8Safe('not-base64!!!')).toBe('not-base64!!!')
    expect(fromBase64Utf8Safe(toBase64Utf8('ok'))).toBe('ok')
  })
})
