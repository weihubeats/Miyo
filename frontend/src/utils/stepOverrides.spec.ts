/**
 * applyStepOverrides 的步骤级超时覆盖（与 miyo-core::apply_step_overrides 同语义回归）：
 * - 数字写入 spec.timeout_ms；
 * - null 显式清除（回退全局 / 默认超时）；
 * - 非法值抛错（保存期 / 物化期拦截兜底）；
 * - 未传键不改动原值。
 */
import { describe, expect, it } from 'vitest'
import { applyStepOverrides } from '@/utils/stepOverrides'
import type { RequestSpec } from '@/types/miyoApi.d'

function spec(): RequestSpec {
  return {
    params: [],
    headers: [],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    timeout_ms: null,
    follow_redirects: true,
    tests: null,
    disable_cookies: false,
  }
}

describe('applyStepOverrides · timeout_ms', () => {
  it('数字毫秒写入 spec.timeout_ms', () => {
    const s = spec()
    applyStepOverrides(s, { timeout_ms: 5000 })
    expect(s.timeout_ms).toBe(5000)
  })

  it('null 不改动原值（同后端语义；清除靠删除 overrides 键）', () => {
    const s = spec()
    s.timeout_ms = 8000
    applyStepOverrides(s, { timeout_ms: null })
    expect(s.timeout_ms).toBe(8000)
  })

  it('未传键不改动原值', () => {
    const s = spec()
    s.timeout_ms = 8000
    applyStepOverrides(s, { headers: [{ key: 'X-A', value: 'b' }] })
    expect(s.timeout_ms).toBe(8000)
  })

  it('非法值抛错（负数 / 非整数 / 非数字）', () => {
    const s = spec()
    expect(() => applyStepOverrides(s, { timeout_ms: -1 })).toThrow()
    expect(() => applyStepOverrides(s, { timeout_ms: 1.5 })).toThrow()
    // 运行时非法形状（绕过类型系统模拟脏数据 / 手改导入文件）
    expect(() => applyStepOverrides(s, { timeout_ms: '5s' as unknown as number })).toThrow()
  })

  it('0 合法（引擎侧语义为尽快超时，由后端收钳）', () => {
    const s = spec()
    applyStepOverrides(s, { timeout_ms: 0 })
    expect(s.timeout_ms).toBe(0)
  })
})
