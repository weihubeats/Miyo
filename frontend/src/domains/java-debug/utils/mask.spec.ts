/**
 * mask.ts 单测：敏感参数名识别、对象键递归脱敏、位置参数按形参名脱敏。
 * 展示层脱敏不改动原始值（返回新结构）。
 */
import { describe, expect, it } from 'vitest'
import { isSensitiveName, maskArgs, maskValue } from '@/domains/java-debug/utils/mask'

describe('isSensitiveName', () => {
  it('命中密码 / token / 凭据类关键词', () => {
    for (const name of ['password', 'userPassword', 'db_passwd', 'accessToken', 'api_key', 'apiKey', 'secret', 'authorization', 'auth', 'credential', 'privateKey']) {
      expect(isSensitiveName(name), name).toBe(true)
    }
  })

  it('普通参数名不误伤', () => {
    for (const name of ['username', 'orderId', 'author', 'timeout', 'request', 'autoRetry']) {
      expect(isSensitiveName(name), name).toBe(false)
    }
  })
})

describe('maskValue', () => {
  it('对象敏感键替换为掩码，普通键保留', () => {
    const out = maskValue({ username: 'alice', password: 'p@ss', nested: { token: 't', age: 3 } })
    expect(out).toEqual({ username: 'alice', password: '******', nested: { token: '******', age: 3 } })
  })

  it('数组逐项递归；原始输入不被修改', () => {
    const input = { token: 'secret', list: [{ password: 'x' }] }
    const out = maskValue(input)
    expect(out).toEqual({ token: '******', list: [{ password: '******' }] })
    expect((input as { token: string }).token).toBe('secret')
    expect((input.list[0] as { password: string }).password).toBe('x')
  })

  it('标量原样返回', () => {
    expect(maskValue('plain')).toBe('plain')
    expect(maskValue(42)).toBe(42)
    expect(maskValue(null)).toBeNull()
  })
})

describe('maskArgs', () => {
  it('位置参数按形参名整值掩码', () => {
    const args = [{ password: 'x' }, 'alice']
    const out = maskArgs(args, ['password', 'username'])
    expect(out).toEqual(['******', 'alice'])
  })

  it('无形参名时退化为对象键掩码', () => {
    expect(maskArgs({ accessToken: 't', size: 1 })).toEqual({ accessToken: '******', size: 1 })
  })

  it('数组且形参名缺失 → 逐项掩码', () => {
    expect(maskArgs([{ secret: 1 }])).toEqual([{ secret: '******' }])
  })
})
