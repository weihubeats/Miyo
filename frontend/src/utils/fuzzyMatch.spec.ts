import { describe, expect, it } from 'vitest'
import { fuzzyScore } from './fuzzyMatch'

describe('fuzzyScore', () => {
  it('空查询得 0 分（调用方自行处理列全量）', () => {
    expect(fuzzyScore('UserService', '')).toBe(0)
    expect(fuzzyScore('UserService', '   ')).toBe(0)
    expect(fuzzyScore(null, 'x')).toBeNull()
  })

  it('前缀命中强于中部子串，子串强于散点', () => {
    const prefix = fuzzyScore('UserService', 'user')!
    const middle = fuzzyScore('AdminUserService', 'user')!
    const scattered = fuzzyScore('u-s-e-r-list', 'user')!
    expect(prefix).toBeGreaterThan(middle)
    expect(middle).toBeGreaterThan(scattered)
    expect(scattered).toBeGreaterThan(0)
  })

  it('大小写不敏感，词首加分', () => {
    expect(fuzzyScore('getUserPosts', 'gup')).not.toBeNull()
    // 同为散点命中：词首（分隔符后）比词中加分更多。
    const atWordStart = fuzzyScore('foo-bar', 'fb')!
    const midWord = fuzzyScore('foob', 'fb')!
    expect(atWordStart).toBeGreaterThan(midWord)
  })

  it('多词查询：全部命中才算匹配', () => {
    expect(fuzzyScore('user service list', 'user list')).not.toBeNull()
    expect(fuzzyScore('user service list', 'user order')).toBeNull()
  })

  it('不匹配返回 null', () => {
    expect(fuzzyScore('login', 'logout')).toBeNull()
  })
})
