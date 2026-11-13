/**
 * errorText：任意抛出值 → 可读错误文案。
 * 回归 AI 测试中心「[object Object]」报错：非 Error 实例的拒绝值
 * （Tauri 早期 reject 的 {code,message} / 插件桥接层异常）经 String() 后
 * 只剩 "[object Object]"，真实原因丢失。
 */
import { describe, expect, it } from 'vitest'
import { errorCode, errorText } from './errorText'

describe('errorText', () => {
  it('Error 实例取 message', () => {
    expect(errorText(new Error('连接失败：目标拒绝连接'))).toBe('连接失败：目标拒绝连接')
  })

  it('字符串原样返回', () => {
    expect(errorText('invalid-json')).toBe('invalid-json')
  })

  it('后端 {code,message} 对象：带 code 前缀，绝不产出 [object Object]', () => {
    const text = errorText({ code: 'AI_PROVIDER', message: 'AI 服务返回 401：invalid api key' })
    expect(text).toBe('AI_PROVIDER: AI 服务返回 401：invalid api key')
    expect(text).not.toContain('[object Object]')
  })

  it('只有 message 的对象', () => {
    expect(errorText({ message: '工具生成测试用例草稿' })).toBe('工具生成测试用例草稿')
  })

  it('只有 error / reason 字段的对象', () => {
    expect(errorText({ error: 'boom' })).toBe('boom')
    expect(errorText({ reason: 'timeout' })).toBe('timeout')
  })

  it('任意对象退化为 JSON，而非 [object Object]', () => {
    const text = errorText({ status: 500, detail: { nested: true } })
    expect(text).toBe('{"status":500,"detail":{"nested":true}}')
  })

  it('数字 / null / undefined 不抛异常', () => {
    expect(errorText(42)).toBe('42')
    expect(errorText(null)).toBe('')
    expect(errorText(undefined)).toBe('')
  })

  it('errorCode 从 Error.code（Object.defineProperty 挂载）与对象 code 读取', () => {
    const err = new Error('x')
    Object.defineProperty(err, 'code', { value: 'VALIDATION', enumerable: true })
    expect(errorCode(err)).toBe('VALIDATION')
    expect(errorCode({ code: 'NOT_FOUND', message: 'y' })).toBe('NOT_FOUND')
    expect(errorCode('plain')).toBe('')
  })
})