/**
 * scenarioScriptParse 单测：生成 → 解析 round-trip（同步到编排的正确性前提），
 * 以及降级形状（复杂 JSONPath / 自造 JS）的明确报错。
 */
import { describe, expect, it } from 'vitest'
import { generateScenarioScript, type ScriptStep } from '@/utils/scenarioScript'
import { parseScenarioScript, ScriptParseError } from '@/utils/scenarioScriptParse'

/** 断言期望值统一成字符串比对（编辑态本来就是字符串形状）。 */
function normExpected(v: unknown): string {
  return v == null ? '' : String(v)
}

function roundTrip(steps: ScriptStep[], stopOnFailure = true) {
  const script = generateScenarioScript('链路演示', steps, {
    base: 'https://api.example.com',
    stopOnFailure,
    envDefaults: {},
  })
  const parsed = parseScenarioScript(script)
  expect(parsed.steps).toHaveLength(steps.length)
  return { script, parsed }
}

describe('scenarioScriptParse：生成 → 解析 round-trip', () => {
  it('链式提取 + 各断言算子 + 查询参数 + 头模板', () => {
    const steps: ScriptStep[] = [
      {
        name: '文章详情',
        method: 'GET',
        url: '{{base_url}}/posts/1',
        params: [{ key: 'full', value: '1' }],
        headers: [{ key: 'X-Trace', value: 'trace-{{run_id}}' }],
        auth: null,
        authUnsupported: null,
        body: null,
        bodyKind: 'none',
        extract: [{ name: 'userId', from: 'body', path: '$.userId' }],
        assertions: [
          { type: 'status', op: 'eq', expected: 200 },
          { type: 'header', path: 'x-miyo', op: 'contains', expected: 'ok' },
          { type: 'response_time_ms', op: 'lt', expected: 3000 },
          { type: 'jsonpath', path: '$.user.name', op: 'eq', expected: 'miyo' },
          { type: 'jsonpath', path: '$.tags[0]', op: 'exists' },
          { type: 'jsonpath', path: '$.count', op: 'gte', expected: 1 },
        ],
      },
      {
        name: '用户文章',
        method: 'GET',
        url: '{{base_url}}/users/{{userId}}/posts?page={{page}}',
        params: [],
        headers: [],
        auth: null,
        authUnsupported: null,
        body: null,
        bodyKind: 'none',
        extract: [{ name: 'firstPost', from: 'body', path: '$[0].id' }],
        assertions: [
          { type: 'jsonpath', path: '$.data.list', op: 'not_empty' },
          { type: 'jsonpath', path: '$.secret', op: 'not_exists' },
          { type: 'status', op: 'neq', expected: 404 },
          { type: 'jsonpath', path: '$.mode', op: 'eq', expected: '{{expected_mode}}' },
        ],
      },
      {
        name: '作者详情',
        method: 'GET',
        url: '{{base_url}}/authors/{{userId}}',
        params: [],
        headers: [],
        auth: { kind: 'bearer', token: '{{token}}' },
        authUnsupported: null,
        body: null,
        bodyKind: 'none',
        extract: [],
        assertions: [
          { name: '响应不能为空', type: 'jsonpath', path: '$.body', op: 'exists' },
        ],
      },
    ]
    const { parsed } = roundTrip(steps)

    // 步骤 1：URL / 参数 / 头模板 / 提取
    expect(parsed.steps[0].name).toBe('文章详情')
    expect(parsed.steps[0].method).toBe('GET')
    expect(parsed.steps[0].url).toBe('{{base_url}}/posts/1')
    expect(parsed.steps[0].params).toEqual([{ key: 'full', value: '1' }])
    expect(parsed.steps[0].headers).toEqual([{ key: 'X-Trace', value: 'trace-{{run_id}}' }])
    expect(parsed.steps[0].extract).toEqual([{ name: 'userId', from: 'body', path: '$.userId' }])

    // 断言逐条还原（期望值经字符串归一；具名 / 匿名都覆盖）
    const a0 = parsed.steps[0].assertions
    expect(a0).toHaveLength(6)
    expect(a0[0]).toMatchObject({ type: 'status', op: 'eq' })
    expect(normExpected(a0[0].expected)).toBe('200')
    expect(a0[1]).toMatchObject({ type: 'header', path: 'x-miyo', op: 'contains', expected: 'ok' })
    expect(a0[2]).toMatchObject({ type: 'response_time_ms', op: 'lt' })
    expect(normExpected(a0[2].expected)).toBe('3000')
    expect(a0[3]).toMatchObject({ type: 'jsonpath', path: '$.user.name', op: 'eq', expected: 'miyo' })
    expect(a0[4]).toMatchObject({ type: 'jsonpath', path: '$.tags[0]', op: 'exists' })
    expect(a0[5]).toMatchObject({ type: 'jsonpath', path: '$.count', op: 'gte' })
    // 数字期望按编辑态约定还原为字符串（保存时 coerceExpected 再转数字）
    expect(normExpected(a0[5].expected)).toBe('1')

    // 步骤 2：链路变量引用 + 剩余算子（neq / not_empty / not_exists / 期望含 {{var}}）
    expect(parsed.steps[1].url).toBe('{{base_url}}/users/{{userId}}/posts?page={{page}}')
    const a1 = parsed.steps[1].assertions
    expect(a1[0]).toMatchObject({ type: 'jsonpath', path: '$.data.list', op: 'not_empty' })
    expect(a1[1]).toMatchObject({ type: 'jsonpath', path: '$.secret', op: 'not_exists' })
    expect(a1[2]).toMatchObject({ type: 'status', op: 'neq' })
    expect(normExpected(a1[2].expected)).toBe('404')
    expect(a1[3]).toMatchObject({ type: 'jsonpath', path: '$.mode', op: 'eq' })
    expect(a1[3].expected).toBe('{{expected_mode}}')
    expect(parsed.steps[1].extract).toEqual([{ name: 'firstPost', from: 'body', path: '$[0].id' }])

    // 步骤 3：鉴权头并入（Bearer → Authorization 头；编排便不回写它，仅供比对）
    expect(parsed.steps[2].headers).toEqual([{ key: 'Authorization', value: 'Bearer {{token}}' }])
    expect(parsed.steps[2].assertions[0].name).toBe('响应不能为空')
    expect(parsed.steps[2].assertions[0].path).toBe('$.body')
  })

  it('JSON / 文本 Body 按原文还原；graphql query 与 variables 还原', () => {
    const steps: ScriptStep[] = [
      {
        name: '下单',
        method: 'POST',
        url: '{{base_url}}/orders',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: '{"sku":"A1","qty":2}',
        bodyKind: 'json',
        extract: [{ name: 'orderId', from: 'body', path: '$.orderId' }],
        assertions: [{ type: 'status', op: 'eq', expected: 201 }],
      },
      {
        name: '支付预览',
        method: 'POST',
        url: '{{base_url}}/pay/preview',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: 'raw-text-body',
        bodyKind: 'text',
        extract: [],
        assertions: [],
      },
      {
        name: 'GraphQL 查询',
        method: 'POST',
        url: '{{base_url}}/graphql',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: 'query Hero($id: ID!) { hero(id: $id) { name } }',
        bodyVariables: '{"id":"42"}',
        bodyKind: 'graphql',
        extract: [],
        assertions: [],
      },
    ]
    const { parsed } = roundTrip(steps)

    expect(parsed.steps[0].body).toBe('{"sku":"A1","qty":2}')
    expect(parsed.steps[0].bodyKind).toBe('json')
    // text：无 Content-Type 头 → 判为 text
    expect(parsed.steps[1].body).toBe('raw-text-body')
    expect(parsed.steps[1].bodyKind).toBe('text')
    // graphql：query 与 variables 文本都还原
    expect(parsed.steps[2].body).toBe('query Hero($id: ID!) { hero(id: $id) { name } }')
    expect(parsed.steps[2].bodyKind).toBe('graphql')
    expect(parsed.steps[2].bodyVariables).toBe('{"id":"42"}')
  })

  it('urlencoded / multipart 文本字段还原', () => {
    const steps: ScriptStep[] = [
      {
        name: '登录',
        method: 'POST',
        url: '{{base_url}}/login',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: null,
        bodyKind: 'urlencoded',
        bodyFields: [
          { key: 'user', value: 'miyo' },
          { key: 'next', value: '{{redirect}}' },
        ],
        extract: [{ name: 'sid', from: 'header', path: 'X-Session' }],
        assertions: [],
      },
      {
        name: '上传',
        method: 'POST',
        url: '{{base_url}}/upload',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: null,
        bodyKind: 'multipart',
        bodyFields: [{ key: 'note', value: '说明' }],
        bodyFileFields: ['avatar'],
        extract: [],
        assertions: [],
      },
    ]
    const { parsed } = roundTrip(steps)

    expect(parsed.steps[0].bodyKind).toBe('urlencoded')
    expect(parsed.steps[0].bodyFields).toEqual([
      { key: 'user', value: 'miyo' },
      { key: 'next', value: '{{redirect}}' },
    ])
    // 头提取：path 保留原始大小写
    expect(parsed.steps[0].extract).toEqual([{ name: 'sid', from: 'header', path: 'X-Session' }])

    expect(parsed.steps[1].bodyKind).toBe('multipart')
    expect(parsed.steps[1].bodyFields).toEqual([{ key: 'note', value: '说明' }])
  })

  it('「注意」降级行收进 warnings 不阻断解析', () => {
    const steps: ScriptStep[] = [
      {
        name: '签名接口',
        method: 'GET',
        url: '{{base_url}}/signed',
        headers: [],
        auth: null,
        authUnsupported: 'DynamicSignature',
        body: null,
        bodyKind: 'none',
        extract: [],
        assertions: [],
      },
    ]
    const { script, parsed } = roundTrip(steps)
    expect(script).toContain('// 注意：该步骤使用 DynamicSignature')
    expect(parsed.steps[0].name).toBe('签名接口')
    expect(parsed.warnings.length).toBeGreaterThan(0)
    expect(parsed.warnings.join('\n')).toContain('DynamicSignature')
  })
})

describe('scenarioScriptParse：降级 / 超出结构 → 明确报错', () => {
  it('复杂 JSONPath 的 TODO 注释 → 带行号的 ScriptParseError', () => {
    const steps: ScriptStep[] = [
      {
        name: '递归路径',
        method: 'GET',
        url: '{{base_url}}/x',
        headers: [],
        auth: null,
        authUnsupported: null,
        body: null,
        bodyKind: 'none',
        extract: [],
        assertions: [{ type: 'jsonpath', path: '$..name', op: 'eq', expected: 'x' }],
      },
    ]
    const script = generateScenarioScript('链路演示', steps, {
      base: 'https://api.example.com',
      stopOnFailure: true,
      envDefaults: {},
    })
    try {
      parseScenarioScript(script)
      expect.unreachable('应当抛错')
    } catch (e) {
      expect(e).toBeInstanceOf(ScriptParseError)
      expect((e as ScriptParseError).line).toBeGreaterThan(0)
      expect((e as Error).message).toContain('降级注释')
    }
  })

  it('步骤里混入自造 JS → 报无法识别的行', () => {
    const base = generateScenarioScript(
      '链路演示',
      [
        {
          name: '文章详情',
          method: 'GET',
          url: '{{base_url}}/posts/1',
          headers: [],
          auth: null,
          authUnsupported: null,
          body: null,
          bodyKind: 'none',
          extract: [],
          assertions: [],
        },
      ],
      { base: 'https://api.example.com', stopOnFailure: false, envDefaults: {} },
    )
    // 在步骤内插入一段自定义 JS
    const polluted = base.replace(
      'let res1 = await fetch(url1, {',
      'if (Math.random() > 0.5) { console.log("lucky"); }\nlet res1 = await fetch(url1, {',
    )
    expect(() => parseScenarioScript(polluted)).toThrowError(/无法识别的脚本行/)
  })

  it('空脚本 / 无步骤 → 明确报错', () => {
    expect(() => parseScenarioScript('const BASE = "x";')).toThrowError(/没有识别到任何步骤/)
  })
})
