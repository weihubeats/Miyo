import { describe, expect, it } from 'vitest'
import type { RequestSpec } from '../types/miyoApi'
import { applyMethodDefaults, envBadgeLabel, envBadgeTooltip, methodNeedsBody, smartTabFor } from './requestBar'

function req(overrides: Partial<RequestSpec> = {}): RequestSpec {
  return {
    params: [],
    headers: [],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    active_tab: null,
    timeout_ms: 30_000,
    follow_redirects: true,
    tests: null,
    disable_cookies: false,
    ...overrides,
  }
}

describe('methodNeedsBody / smartTabFor', () => {
  it('POST / PUT / PATCH → Body；其余 → Params', () => {
    expect(methodNeedsBody('POST')).toBe(true)
    expect(methodNeedsBody('PUT')).toBe(true)
    expect(methodNeedsBody('PATCH')).toBe(true)
    expect(methodNeedsBody('GET')).toBe(false)
    expect(methodNeedsBody('DELETE')).toBe(false)
    expect(methodNeedsBody('HEAD')).toBe(false)
    expect(methodNeedsBody('OPTIONS')).toBe(false)
    expect(smartTabFor('POST')).toBe('body')
    expect(smartTabFor('GET')).toBe('params')
  })
})

describe('applyMethodDefaults', () => {
  it('POST + 空体 → 初始化为 JSON {} + Content-Type application/json，返回 body', () => {
    const r = req()
    const tab = applyMethodDefaults(r, 'POST')
    expect(tab).toBe('body')
    expect(r.body).toEqual({ mode: 'json', raw: '{}' })
    expect(r.headers).toContainEqual({
      key: 'Content-Type',
      value: 'application/json',
      enabled: true,
      description: '',
      field_type: 'string',
      required: true,
      example: '',
    })
  })

  it('POST + 已有 body 内容 → 保留原样，仍返回 body', () => {
    const r = req({ body: { mode: 'json', raw: '{"a":1}' } })
    const tab = applyMethodDefaults(r, 'POST')
    expect(tab).toBe('body')
    expect(r.body).toEqual({ mode: 'json', raw: '{"a":1}' })
  })

  it('POST + 非空 text body → 不强制 JSON，保留原样', () => {
    const r = req({ body: { mode: 'text', raw: 'hello' } })
    applyMethodDefaults(r, 'POST')
    expect(r.body).toEqual({ mode: 'text', raw: 'hello' })
  })

  it('GET / DELETE → 返回 params，绝不触碰 body', () => {
    const r = req()
    expect(applyMethodDefaults(r, 'GET')).toBe('params')
    expect(applyMethodDefaults(r, 'DELETE')).toBe('params')
    expect(r.body).toEqual({ mode: 'none' })
  })
})

describe('envBadgeLabel', () => {
  const base = { urlDomain: 'https://paymentv2test.redotpay.net', resolvedDomain: '', envName: 'Test' }

  it('有解析域名 → 直接展示裸域名（去协议）', () => {
    expect(
      envBadgeLabel({ ...base, resolvedDomain: 'https://paymentv2test.redotpay.net' }),
    ).toBe('paymentv2test.redotpay.net')
  })

  it('无域名可显示 → 退回环境名', () => {
    expect(envBadgeLabel({ urlDomain: '', resolvedDomain: '', envName: 'Test' })).toBe('Test')
  })

  it('变量未解析 → 展示字面量（含协议原文）', () => {
    expect(envBadgeLabel({ urlDomain: '{{base_url}}', resolvedDomain: '', envName: 'Test' })).toBe('{{base_url}}')
  })
})

describe('envBadgeTooltip', () => {
  const t = (key: string, params?: Record<string, string>): string => {
    const dict: Record<string, string> = {
      'editor.badgeEnv': '环境：{env}',
      'editor.badgeUnresolved': '{v} 未定义，请求将按字面量发送',
      'editor.badgeEnvBase': '环境：{env} | 基础路径：{url}',
      'editor.badgeSession': '基础路径：{url}（会话 Base URL）',
    }
    let text = dict[key] ?? key
    for (const [k, v] of Object.entries(params ?? {})) text = text.split(`{${k}}`).join(v)
    return text
  }

  it('环境已解析 → 环境：X | 基础路径：完整 URL', () => {
    expect(
      envBadgeTooltip(
        {
          urlDomain: '{{base_url}}',
          resolvedDomain: 'https://paymentv2test.redotpay.net',
          envName: 'Test',
        },
        t,
      ),
    ).toBe('环境：Test | 基础路径：https://paymentv2test.redotpay.net')
  })

  it('变量未定义 → 字面量警告（DNS 失败的直观提示）', () => {
    expect(
      envBadgeTooltip({ urlDomain: '{{base_url}}', resolvedDomain: '', envName: 'Test' }, t),
    ).toBe('{{base_url}} 未定义，请求将按字面量发送')
  })

  it('无环境（会话 Base URL）→ 标注会话来源', () => {
    expect(
      envBadgeTooltip({ urlDomain: 'https://api.x.com', resolvedDomain: 'https://api.x.com', envName: '' }, t),
    ).toBe('基础路径：https://api.x.com（会话 Base URL）')
  })

  it('无任何域名 → 仅展示环境名', () => {
    expect(envBadgeTooltip({ urlDomain: '', resolvedDomain: '', envName: 'Test' }, t)).toBe('环境：Test')
    expect(envBadgeTooltip({ urlDomain: '', resolvedDomain: '', envName: '' }, t)).toBe('')
  })
})
describe('applyMethodDefaults：GRPC 分支', () => {
  it('GRPC + 空 body → 初始化 grpc 配置（空服务/方法、{} 消息、反射），返回 grpc', () => {
    const r = req()
    const tab = applyMethodDefaults(r, 'GRPC')
    expect(tab).toBe('grpc')
    expect(r.body).toEqual({
      mode: 'grpc',
      spec: { service: '', method: '', message: '{}', use_tls: false, proto_ids: [] },
    })
  })

  it('GRPC + 已有 grpc 配置 → 保留原样（不覆盖 service/method/message）', () => {
    const spec = {
      service: 'grpcbin.GRPCBin',
      method: 'DummyUnary',
      message: '{"f_string":"x"}',
      use_tls: true,
      proto_ids: ['pf-1'],
    }
    const r = req({ body: { mode: 'grpc', spec } })
    const tab = applyMethodDefaults(r, 'GRPC')
    expect(tab).toBe('grpc')
    expect(r.body).toEqual({ mode: 'grpc', spec })
  })

  it('GRPC + 已有 JSON body → 覆盖为 grpc 配置（HTTP body 对 gRPC 无意义）', () => {
    const r = req({ body: { mode: 'json', raw: '{"a":1}' } })
    const tab = applyMethodDefaults(r, 'GRPC')
    expect(tab).toBe('grpc')
    expect(r.body.mode).toBe('grpc')
  })
})
