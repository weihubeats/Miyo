/**
 * 版本兼容性（前端预检层）单测：x.y.z 逐段比较与 SDK 协议版本口径。
 * 后端（miyo-plugin::compat）为启用/更新的强制真源，这里锁定前端 UX 预检语义。
 */
import { describe, expect, it } from 'vitest'
import { isCompatible, isSdkSupported, parseVersion } from '@/plugins/compatibility'

describe('版本兼容性', () => {
  it('parseVersion：严格 x.y.z', () => {
    expect(parseVersion('0.0.60')).toEqual([0, 0, 60])
    expect(parseVersion('12.345.678')).toEqual([12, 345, 678])
    expect(parseVersion('1.2')).toBeNull()
    expect(parseVersion('1.2.3.4')).toBeNull()
    expect(parseVersion('0.0.x')).toBeNull()
    expect(parseVersion('0.0.1000')).toBeNull()
    expect(parseVersion('')).toBeNull()
  })

  it('isCompatible：逐段比较（patch 位不做字符串序）', () => {
    expect(isCompatible('0.0.60', '0.0.60')).toBe(true)
    expect(isCompatible('0.0.60', '0.1.0')).toBe(true)
    expect(isCompatible('0.0.9', '0.0.10')).toBe(true)
    expect(isCompatible('0.0.10', '0.0.9')).toBe(false)
    expect(isCompatible('0.1.0', '0.0.99')).toBe(false)
    expect(isCompatible('latest', '0.0.60')).toBe(false)
  })

  it('isSdkSupported：仅支持当前协议版本', () => {
    expect(isSdkSupported(1)).toBe(true)
    expect(isSdkSupported(2)).toBe(false)
    expect(isSdkSupported(0)).toBe(false)
  })
})
