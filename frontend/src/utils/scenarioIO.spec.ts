/**
 * scenarioIO：导入 / 导出（JSON）——导出物化内联、导入形状校验。
 */
import { describe, expect, it } from 'vitest'
import { buildScenarioExport, inlineStepFromEndpoint, parseScenarioImport } from './scenarioIO'
import type { Endpoint, Scenario } from '../types/miyoApi'

function makeEndpoint(): Endpoint {
  return {
    id: 'ep-1',
    project_id: 'p-1',
    folder_id: null,
    name: '创建订单',
    method: 'POST',
    path: '/shop/orders',
    description: '',
    status: 'developing',
    sort_order: 0,
    request: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'json', raw: '{"sku":"A"}' },
      timeout_ms: null,
      follow_redirects: true,
      tests: null,
      disable_cookies: false,
    },
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

function makeScenario(): Scenario {
  return {
    id: 'sc-1',
    project_id: 'p-1',
    name: '电商主流程',
    description: '下单 → 支付',
    category: '电商',
    sort_order: 0,
    steps: [
      {
        endpoint_id: 'ep-1',
        enabled: true,
        extract: [{ name: 'orderNo', from: 'body', path: '$.orderNo' }],
        assertions: [{ type: 'status', op: 'eq', expected: 201 }],
      },
      {
        endpoint_id: null,
        enabled: false,
        extract: [],
        assertions: [],
        name: '手工步骤',
        request: {
          method: 'GET',
          url: '{{base_url}}/ping',
          spec: {
            params: [],
            headers: [],
            path_variables: [],
            auth: { type: 'none' },
            body: { mode: 'none' },
            timeout_ms: null,
            follow_redirects: true,
            tests: null,
            disable_cookies: false,
          },
        },
      },
    ],
    script: null,
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

describe('scenarioIO 导出', () => {
  it('引用式步骤物化为内联拷贝（脱离项目仍可执行），内联步骤原样保留', () => {
    const ep = makeEndpoint()
    const { json, redactedFields } = buildScenarioExport(makeScenario(), (id) => (id === 'ep-1' ? ep : undefined))
    const parsed = JSON.parse(json) as { magic: string; scenario: { steps: { request?: { url: string; method: string }; enabled: boolean; name?: string | null }[] } }
    expect(parsed.magic).toBe('miyo.scenario')
    const [refStep, inlineStep] = parsed.scenario.steps
    // 引用式 → 内联：URL 模板 = {{base_url}} + 接口路径，方法一致，名字取接口名
    expect(refStep.request?.url).toBe('{{base_url}}/shop/orders')
    expect(refStep.request?.method).toBe('POST')
    expect(refStep.name).toBe('创建订单')
    // 已是内联的步骤不变形
    expect(inlineStep.request?.url).toBe('{{base_url}}/ping')
    expect(inlineStep.enabled).toBe(false)
    // 无敏感值的导出：脱敏清单为空
    expect(redactedFields).toEqual([])
  })

  it('导出脱敏：内联步骤的 auth / 敏感头 / JSON body 凭证替换占位符并登记清单', () => {
    const ep = makeEndpoint()
    ep.request.headers.push({ key: 'Authorization', value: 'Bearer real-tok', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    ep.request.auth = { type: 'bearer', token: 'real-tok-2' }
    ep.request.body = { mode: 'json', raw: '{"password":"pw-1","sku":"A"}' }
    const scenario = makeScenario()
    scenario.steps[0] = { endpoint_id: 'ep-1', enabled: true, extract: [], assertions: [] }
    const { json, redactedFields } = buildScenarioExport(scenario, (id) => (id === 'ep-1' ? ep : undefined))
    expect(json).not.toContain('real-tok')
    expect(json).not.toContain('pw-1')
    expect(json).toContain('__MIYO_REDACTED__')
    expect(json).toContain('sku')
    expect(redactedFields.length).toBeGreaterThan(0)
  })

  it('找不到接口的引用步骤保留为禁用占位（不静默丢步骤）', () => {
    const out = inlineStepFromEndpoint(
      { endpoint_id: 'missing', enabled: true, extract: [], assertions: [] },
      undefined,
    )
    expect(out.enabled).toBe(false)
    expect(out.request).toBeUndefined()
  })
})

describe('scenarioIO 导入', () => {
  it('roundtrip：导出 → 导入得到等价场景体', () => {
    const ep = makeEndpoint()
    const { json } = buildScenarioExport(makeScenario(), (id) => (id === 'ep-1' ? ep : undefined))
    const parsed = parseScenarioImport(json)
    expect(parsed.name).toBe('电商主流程')
    expect(parsed.category).toBe('电商')
    expect(parsed.steps).toHaveLength(2)
    expect(parsed.steps[0].request?.url).toBe('{{base_url}}/shop/orders')
    expect(parsed.steps[0].extract).toEqual([{ name: 'orderNo', from: 'body', path: '$.orderNo' }])
    expect(parsed.steps[1].enabled).toBe(false)
  })

  it('非法 JSON / 缺字段拒绝并给可定位错误', () => {
    expect(() => parseScenarioImport('not json')).toThrow('invalid-json')
    expect(() => parseScenarioImport('{"foo":1}')).toThrow('invalid-shape')
  })

  it('两无步骤（无引用无内联）归一为禁用占位内联', () => {
    const parsed = parseScenarioImport(
      JSON.stringify({ magic: 'miyo.scenario', version: 1, scenario: { name: 'x', steps: [{}] } }),
    )
    expect(parsed.steps[0].enabled).toBe(false)
    expect(parsed.steps[0].request).toBeTruthy()
  })
})
