import { describe, expect, it } from 'vitest'
import { applyPolicy, detectPolicy, OVERRIDE_POLICIES, policyLabelKey } from '@/utils/overridePolicy'
import type { RequestSpec, StepOverrides } from '@/types/miyoApi'

function spec(patch: Partial<RequestSpec> = {}): RequestSpec {
  return {
    params: [],
    headers: [{ key: 'X-Trace', value: 'a', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    timeout_ms: null,
    follow_redirects: true,
    tests: null,
    ...patch,
    disable_cookies: false,
  }
}

describe('detectPolicy', () => {
  it('无 overrides → 继承', () => {
    expect(detectPolicy(spec(), null)).toBe('inherit')
    expect(detectPolicy(spec(), {})).toBe('inherit')
  })

  it('只覆盖了一部分行 → 合并', () => {
    expect(detectPolicy(spec(), { headers: [{ key: 'X-Else', value: 'b' }] })).toBe('merge')
  })

  it('接口所有行都被吃住且 body 也覆盖 → 替换', () => {
    const overrides: StepOverrides = {
      headers: [{ key: 'x-trace', value: 'b', enabled: true }],
      body: { mode: 'json', raw: '{}' },
    }
    expect(detectPolicy(spec(), overrides)).toBe('replace')
  })

  it('header 键大小写不敏感', () => {
    expect(detectPolicy(spec(), { headers: [{ key: 'x-TRACE', value: 'z' }], body: { mode: 'none' } })).toBe('replace')
  })

  it('接口无行时，只要给了 overrides 即视为替换', () => {
    expect(detectPolicy(spec({ headers: [] }), { headers: [{ key: 'A', value: '1' }] })).toBe('replace')
  })
})

describe('applyPolicy', () => {
  it('inherit → null', () => {
    expect(applyPolicy('inherit', spec(), { url: '/x' })).toBeNull()
  })

  it('replace 预填接口全量行', () => {
    const out = applyPolicy('replace', spec(), { headers: [{ key: 'X-New', value: 'n' }] })
    expect(out?.headers).toEqual([
      { key: 'X-Trace', value: 'a', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'X-New', value: 'n' },
    ])
  })

  it('replace 时已有覆盖行优先，不重复插入', () => {
    const out = applyPolicy('replace', spec(), { headers: [{ key: 'x-trace', value: 'changed', enabled: false }] })
    expect(out?.headers).toEqual([{ key: 'X-Trace', value: 'changed', enabled: false, description: '', field_type: 'string', required: true, example: '' }])
  })

  it('replace 预填 body（接口有 body 时）', () => {
    const out = applyPolicy('replace', spec({ body: { mode: 'json', raw: '{"a":1}' } }), null)
    expect(out?.body).toEqual({ mode: 'json', raw: '{"a":1}' })
  })

  it('merge 退回时剥掉纯继承副本', () => {
    const full = applyPolicy('replace', spec(), null)
    const merged = applyPolicy('merge', spec(), full)
    expect(merged?.headers).toBeUndefined()
  })

  it('merge 退回时保留用户手写行', () => {
    const full = applyPolicy('replace', spec(), { headers: [{ key: 'X-New', value: 'n' }] })
    const merged = applyPolicy('merge', spec(), full)
    expect(merged?.headers).toEqual([{ key: 'X-New', value: 'n' }])
  })

  it('replace 不修改入参', () => {
    const prev: StepOverrides = { headers: [{ key: 'X-New', value: 'n' }] }
    applyPolicy('replace', spec(), prev)
    expect(prev.headers).toEqual([{ key: 'X-New', value: 'n' }])
  })
})

describe('常量', () => {
  it('三态顺序固定，标签键可用', () => {
    expect(OVERRIDE_POLICIES).toEqual(['inherit', 'merge', 'replace'])
    expect(OVERRIDE_POLICIES.map(policyLabelKey)).toEqual([
      'integration.policy.inherit',
      'integration.policy.merge',
      'integration.policy.replace',
    ])
  })
})