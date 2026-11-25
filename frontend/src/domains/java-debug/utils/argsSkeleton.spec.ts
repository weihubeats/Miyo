/**
 * 参数骨架 / 记忆单测。
 *
 * 背景（真实反馈，2026-11）：attach 调试里集合 / Map / DTO 参数默认 `null`，
 * 用户每次调用都要点「编辑 JSON」手写完整结构；同一个方法重跑也要从头填。
 * 这里锁定「给出可执行骨架」「沿用上次参数」两条契约。
 */
import { describe, expect, it } from 'vitest'
import { defaultArgFor, dtoSkeleton, mergeArgs, simpleTypeName } from './argsSkeleton'

describe('simpleTypeName', () => {
  it('剥包名，保留数组后缀', () => {
    expect(simpleTypeName('java.lang.String')).toBe('String')
    expect(simpleTypeName('com.x.Foo[]')).toBe('Foo[]')
    expect(simpleTypeName('byte[]')).toBe('byte[]')
  })

  it('保留泛型签名（只剥外层包名）', () => {
    expect(simpleTypeName('java.util.List<com.x.Foo>')).toBe('List<com.x.Foo>')
    expect(simpleTypeName('PageQuery<com.x.Foo>')).toBe('PageQuery<com.x.Foo>')
  })

  it('空值不炸', () => {
    expect(simpleTypeName(null)).toBe('')
    expect(simpleTypeName(undefined)).toBe('')
    expect(simpleTypeName('  ')).toBe('')
  })
})

describe('defaultArgFor 标量', () => {
  it('字符串 → 空串，布尔 → false', () => {
    expect(defaultArgFor('java.lang.String')).toBe('')
    expect(defaultArgFor('boolean')).toBe(false)
    expect(defaultArgFor('Boolean')).toBe(false)
  })

  it('整型 / 浮点 / BigDecimal → 数字 0（JSON 里必须是 number，不能是字符串）', () => {
    for (const t of ['int', 'Integer', 'long', 'Long', 'short', 'byte', 'BigInteger']) {
      expect(defaultArgFor(t)).toBe(0)
    }
    for (const t of ['double', 'Double', 'float', 'BigDecimal']) {
      expect(defaultArgFor(t)).toBe(0)
    }
  })
})

describe('defaultArgFor 集合 / Map / 数组', () => {
  it('集合 → 空数组（可直接改元素，不用点编辑）', () => {
    for (const t of ['java.util.List', 'List', 'ArrayList', 'Set', 'Collection']) {
      expect(defaultArgFor(t)).toEqual([])
    }
  })

  it('Map → 空对象', () => {
    for (const t of ['java.util.Map', 'Map', 'HashMap', 'JSONObject']) {
      expect(defaultArgFor(t)).toEqual({})
    }
  })

  it('数组 → 空数组', () => {
    expect(defaultArgFor('byte[]')).toEqual([])
    expect(defaultArgFor('com.x.Foo[]')).toEqual([])
  })

  it('泛型签名优先于擦除类型', () => {
    // 擦除是 List，泛型说明是 Map 风格包装时不误判 —— 用 Optional<List<X>> 验证递归
    expect(defaultArgFor('java.util.List', 'java.util.List<java.lang.String>')).toEqual([])
    expect(
      defaultArgFor('java.util.Optional', 'java.util.Optional<java.util.Map<java.lang.String,java.lang.Object>>'),
    ).toEqual({})
  })
})

describe('defaultArgFor 未知类型', () => {
  it('不认识的类给 null（宁缺勿猜业务字段）', () => {
    expect(defaultArgFor('com.x.OrderService')).toBeNull()
    expect(defaultArgFor('com.x.SomeWeirdDto')).toBeNull()
  })

  it('常见 DTO 给键齐全的骨架', () => {
    expect(defaultArgFor('com.x.dto.PageQuery')).toEqual({ pageNo: 1, pageSize: 10 })
    expect(defaultArgFor('com.x.dto.PageRequest')).toEqual({ page: 0, size: 20 })
  })

  it('包装响应 → code/message/data', () => {
    expect(defaultArgFor('com.x.Result')).toEqual({ code: 0, message: '', data: null })
  })
})

describe('dtoSkeleton', () => {
  it('剥泛型后仍能命中模板', () => {
    expect(dtoSkeleton('com.x.PageQuery<com.y.Item>')).toEqual({ pageNo: 1, pageSize: 10 })
  })

  it('返回副本（调用方改它不污染模板）', () => {
    const a = dtoSkeleton('Money')!
    a.amount = 999
    expect(dtoSkeleton('Money')!.amount).toBe(0)
  })

  it('未知类型 → null', () => {
    expect(dtoSkeleton('com.x.Nope')).toBeNull()
  })
})

describe('mergeArgs（上次参数复用）', () => {
  it('类型兼容 → 沿用上次值', () => {
    const skeleton = ['', 0, false]
    expect(mergeArgs(['BTC', 42, true], skeleton)).toEqual(['BTC', 42, true])
  })

  it('类型不兼容 → 回落到骨架默认', () => {
    const skeleton = ['', 0]
    // 上次把数字填进了字符串位（改过方法签名）
    expect(mergeArgs([123, 'oops'], skeleton)).toEqual(['', 0])
  })

  it('参数个数不一致 → 骨架（不强行对齐，避免错位）', () => {
    expect(mergeArgs(['a'], ['', 0])).toEqual(['', 0])
  })

  it('没有上次参数 → 骨架原样', () => {
    const skeleton = ['', []]
    expect(mergeArgs(null, skeleton)).toEqual(['', []])
    expect(mergeArgs(undefined, skeleton)).toEqual(['', []])
  })

  it('骨架是 null 而上次有值 → 沿用上次（未知结构不重置用户的输入）', () => {
    expect(mergeArgs([{ a: 1 }], [null])).toEqual([{ a: 1 }])
  })

  it('数组 ↔ 对象不互串', () => {
    expect(mergeArgs([[1, 2]], [{}])).toEqual([{}])
    expect(mergeArgs([{ k: 1 }], [[]])).toEqual([[]])
  })
})