import { describe, expect, beforeEach, it } from 'vitest'
import { migrateLegacyStorageKeys } from '@/utils/legacyStorage'

describe('migrateLegacyStorageKeys（品牌更名 localStorage 迁移）', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('旧键存在且新键缺失时搬移并删除旧键', () => {
    localStorage.setItem('rustfox.theme.mode', 'dark')
    localStorage.setItem('rustfox.locale.mode', 'zh')
    localStorage.setItem('rustfox:shortcut-bindings', '{"a":"b"}')
    localStorage.setItem('rustfox_graphql_history', '[1,2]')
    localStorage.setItem('rustfox.base-url.proj-1', 'https://api.example.com')

    migrateLegacyStorageKeys()

    expect(localStorage.getItem('miyo.theme.mode')).toBe('dark')
    expect(localStorage.getItem('miyo.locale.mode')).toBe('zh')
    expect(localStorage.getItem('miyo:shortcut-bindings')).toBe('{"a":"b"}')
    expect(localStorage.getItem('miyo_graphql_history')).toBe('[1,2]')
    expect(localStorage.getItem('miyo.base-url.proj-1')).toBe('https://api.example.com')
    expect(localStorage.getItem('rustfox.theme.mode')).toBeNull()
    expect(localStorage.getItem('rustfox.base-url.proj-1')).toBeNull()
  })

  it('新键已存在时不覆盖，仅删除旧键', () => {
    localStorage.setItem('rustfox.theme.mode', 'light')
    localStorage.setItem('miyo.theme.mode', 'dark')
    localStorage.setItem('rustfox.base-url.p', 'old')
    localStorage.setItem('miyo.base-url.p', 'new')

    migrateLegacyStorageKeys()

    expect(localStorage.getItem('miyo.theme.mode')).toBe('dark')
    expect(localStorage.getItem('miyo.base-url.p')).toBe('new')
    expect(localStorage.getItem('rustfox.theme.mode')).toBeNull()
    expect(localStorage.getItem('rustfox.base-url.p')).toBeNull()
  })

  it('幂等：重复调用无副作用', () => {
    localStorage.setItem('rustfox.open-projects', '["p1"]')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('miyo.open-projects')).toBe('["p1"]')
    migrateLegacyStorageKeys()
    expect(localStorage.getItem('miyo.open-projects')).toBe('["p1"]')
  })

  it('前缀迁移保留 projectId 后缀', () => {
    localStorage.setItem('rustfox.base-url.aaa-bbb', 'u1')
    localStorage.setItem('rustfox.base-url.ccc', 'u2')

    migrateLegacyStorageKeys()

    expect(localStorage.getItem('miyo.base-url.aaa-bbb')).toBe('u1')
    expect(localStorage.getItem('miyo.base-url.ccc')).toBe('u2')
  })
})
