/**
 * scenarioScript 生成器单测：链路变量传递（前步提取 → 后步 URL 引用）、
 * 断言映射、URL 模板转换、无 {{}} 残留。
 */
import { describe, expect, it } from 'vitest'
import {
  generateScenarioScript,
  isSensitiveVarName,
  jsonPathToJs,
  urlTemplateToJs,
  type ScriptStep,
} from '@/utils/scenarioScript'

/** 一致性测试的步骤构造器：缺省字段统一物化（生成器同款容错）。 */
function step(partial: Partial<ScriptStep> & Pick<ScriptStep, 'name' | 'method' | 'url'>): ScriptStep {
  return {
    params: [],
    headers: [],
    body: null,
    bodyKind: 'none',
    extract: [],
    assertions: [],
    ...partial,
  }
}

describe('jsonPathToJs', () => {
  it('简单路径转可选链', () => {
    expect(jsonPathToJs('$.userId')).toBe('body?.userId')
    expect(jsonPathToJs('$.data.token')).toBe('body?.data?.token')
    expect(jsonPathToJs('$[0].userId')).toBe('body?.[0]?.userId')
    expect(jsonPathToJs('$')).toBe('body')
  })

  it('复杂路径返回 null（生成 TODO 注释）', () => {
    expect(jsonPathToJs('$..id')).toBeNull()
    expect(jsonPathToJs('$.a[?(@.x)]')).toBeNull()
    expect(jsonPathToJs('nope')).toBeNull()
  })
})

describe('urlTemplateToJs', () => {
  it('base 与链路变量转模板插值', () => {
    expect(urlTemplateToJs('{{base_url}}/users/{{userId}}/posts')).toBe(
      '`${BASE}/users/${userId}/posts`',
    )
  })

  it('模板串内反引号与 ${ 字面量转义', () => {
    const out = urlTemplateToJs('{{base_url}}/x?a=`y`')
    expect(out.startsWith('`')).toBe(true)
    expect(out).toContain('\\`y\\`')
  })
})

describe('generateScenarioScript', () => {
  const steps = [
    {
      name: '文章详情',
      method: 'GET',
      url: '{{base_url}}/posts/1',
      headers: [],
      body: null,
      bodyKind: 'other' as const,
      extract: [{ name: 'userId', from: 'body' as const, path: '$.userId' }],
      assertions: [{ type: 'status' as const, path: null, op: 'eq' as const, expected: 200 }],
    },
    {
      name: '用户文章',
      method: 'GET',
      url: '{{base_url}}/users/{{userId}}/posts',
      headers: [{ key: 'X-Trace', value: '{{userId}}' }],
      body: '{"title":"t"}',
      bodyKind: 'json' as const,
      extract: [],
      assertions: [
        { type: 'jsonpath' as const, path: '$[0].userId', op: 'eq' as const, expected: '{{userId}}' },
        { type: 'response_time_ms' as const, path: null, op: 'lt' as const, expected: 2000 },
      ],
    },
  ]

  it('生成可运行脚本：链路变量贯通 + 断言映射', () => {
    const code = generateScenarioScript('链路演示', steps, {
      base: 'https://jsonplaceholder.typicode.com',
      stopOnFailure: true,
    })
    // 环境兜底 + BASE 覆盖入口。
    expect(code).toContain("const BASE = process.env.BASE_URL ?? \"https://jsonplaceholder.typicode.com\";")
    // 第一步提取。
    expect(code).toContain('var userId = (body1?.userId); // 提取 $.userId')
    // 第二步 URL 与请求头引用前步变量，无 {{}} 残留。
    expect(code).toContain('`${BASE}/users/${userId}/posts`')
    expect(code).toContain('"X-Trace": `${userId}`')
    expect(code).not.toContain('{{')
    // 断言：jsonpath 链 + 耗时 + 失败即停退出。
    expect(code).toContain('check(`jsonpath $[0].userId eq ${userId}`')
    expect(code).toContain('body2?.[0]?.userId === `${userId}`')
    expect(code).toContain('Date.now() - t2')
    expect(code).toContain('process.exit(1)')
    // JSON body 自动补 Content-Type。
    expect(code).toContain('"Content-Type": "application/json"')
  })

  it('关闭失败即停时不含退出分支', () => {
    const code = generateScenarioScript('s', steps, { base: 'http://x', stopOnFailure: false })
    expect(code).not.toContain('失败即停：')
    expect(code).toContain('if (results.some((r) => !r.ok)) process.exit(1);')
  })
})

describe('导出脚本与真实请求的一致性（对齐 miyo-http 发送路径）', () => {
  it('查询参数按 append_query 语义追加（已有 ? 用 & 续）', () => {
    const script = generateScenarioScript(
      '查询参数',
      [
        step({
          name: '列表',
          method: 'GET',
          url: '{{base_url}}/users?page=1',
          params: [
            { key: 'size', value: '20' },
            { key: 'q', value: '{{keyword}}' },
          ],
        }),
      ],
      { base: 'http://127.0.0.1:1', stopOnFailure: false, envDefaults: { keyword: 'abc' } },
    )
    expect(script).toContain('let url1 = `${BASE}/users?page=1`;')
    expect(script).toContain("url1 += (url1.includes('?') ? '&' : '?') + new URLSearchParams([")
    expect(script).toContain('["size", "20"]')
    expect(script).toContain('["q", `${keyword}`]')
  })

  it('Bearer / Basic / ApiKey 认证映射为头或参数', () => {
    const bearer = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'GET', url: '{{base_url}}/a', auth: { kind: 'bearer', token: '{{token}}' } })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(bearer).toContain('"Authorization": `Bearer ${token}`,')

    const basic = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'GET', url: '{{base_url}}/a', auth: { kind: 'basic', username: 'u', password: 'p' } })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(basic).toContain("'Basic ' + btoa(")

    const apiKey = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'GET', url: '{{base_url}}/a', auth: { kind: 'apikey_header', key: 'X-Key', value: 'v' } })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(apiKey).toContain('"X-Key": "v"')

    const apiQuery = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'GET', url: '{{base_url}}/a', auth: { kind: 'apikey_query', key: 'api_key', value: 'v' } })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(apiQuery).toContain('["api_key", "v"]')
  })

  it('不可映射鉴权（OAuth2 等）生成明确注释，不静默丢失', () => {
    const script = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'GET', url: '{{base_url}}/a', authUnsupported: 'OAuth2' })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(script).toContain('// 注意：该步骤使用 OAuth2')
    expect(script).toContain('自行补齐对应凭据')
  })

  it('urlencoded / multipart / graphql body 与 Miyo 发送同构', () => {
    const urlenc = generateScenarioScript(
      'b',
      [step({
        name: 'x', method: 'POST', url: '{{base_url}}/form', bodyKind: 'urlencoded',
        bodyFields: [{ key: 'a', value: '1' }, { key: 'b', value: '{{v}}' }],
      })],
      { base: 'http://x', stopOnFailure: false, envDefaults: { v: '2' } },
    )
    expect(urlenc).toContain("new URLSearchParams([")
    expect(urlenc).toContain('["a", "1"],')
    expect(urlenc).toContain('["b", `${v}`]')

    const mp = generateScenarioScript(
      'b',
      [step({
        name: 'x', method: 'POST', url: '{{base_url}}/up', bodyKind: 'multipart',
        bodyFields: [{ key: 'note', value: 'hi' }], bodyFileFields: ['avatar'],
      })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(mp).toContain('new FormData()')
    expect(mp).toContain('fd.append("note", "hi")')
    expect(mp).toContain('文件字段 "avatar"')

    const gql = generateScenarioScript(
      'b',
      [step({
        name: 'x', method: 'POST', url: '{{base_url}}/graphql', bodyKind: 'graphql',
        body: 'query { users { id } }', bodyVariables: '{"limit":10}',
      })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(gql).toContain('variables: JSON.parse(')
    expect(gql).toContain('limit')
  })

  it('binary / grpc 生成明确的不支持注释（binary 附本地文件读取行）', () => {
    const bin = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'POST', url: '{{base_url}}/up', bodyKind: 'binary', bodyPath: '/tmp/a.bin' })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(bin).toContain("import { readFileSync } from 'node:fs';")
    expect(bin).toContain('// body: readFileSync("/tmp/a.bin"),')

    const grpc = generateScenarioScript(
      'b',
      [step({ name: 'x', method: 'POST', url: '{{base_url}}/g', bodyKind: 'grpc' })],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(grpc).toContain('gRPC 调用无法用 HTTP fetch 还原')
  })

  it('环境变量声明：非敏感名内嵌默认，敏感名只引用 MIYO_VAR_*', () => {
    const script = generateScenarioScript(
      'b',
      [
        step({
          name: 'x', method: 'GET', url: '{{base_url}}/users/{{region}}?token={{api_token}}',
        }),
      ],
      { base: 'http://x', stopOnFailure: false, envDefaults: { region: 'cn-north', api_token: 'SHOULD-NOT-EMBED' } },
    )
    expect(script).toContain('var region = process.env.MIYO_VAR_REGION ?? "cn-north";')
    expect(script).toContain("process.env.MIYO_VAR_API_TOKEN ?? ''")
    expect(script).not.toContain('SHOULD-NOT-EMBED')
    expect(script).toContain('MIYO_VAR_API_TOKEN')
  })

  it('敏感名单启发式：token/secret/password 类不内嵌', () => {
    expect(isSensitiveVarName('api_token')).toBe(true)
    expect(isSensitiveVarName('PASSWORD')).toBe(true)
    expect(isSensitiveVarName('region')).toBe(false)
    expect(isSensitiveVarName('base_url')).toBe(false)
  })


describe('generateScenarioScript 执行控制', () => {
  const base = {
    name: '步',
    method: 'GET',
    url: '{{base_url}}/x',
    headers: [],
    body: null,
    bodyKind: 'none' as const,
    extract: [],
    assertions: [],
  }

  it('waitMs 生成真实 sleep 调用', () => {
    const code = generateScenarioScript('t', [{ ...base, waitMs: 500 }], {
      base: 'http://x',
      stopOnFailure: false,
    })
    expect(code).toContain('const sleep = (ms)')
    expect(code).toContain('await sleep(500); // 执行前等待')
  })

  it('条件 / while 循环 / 并行生成说明注释', () => {
    const code = generateScenarioScript(
      't',
      [
        { ...base, condition: { source: 'var', name: 'paid', op: 'eq', value: 'true' } },
        { ...base, loop: { mode: 'while', max_loops: 5, interval_ms: 300 } },
        { ...base, parallel: true },
      ],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(code).toContain('执行条件（变量 paid eq true）')
    expect(code).toContain('条件轮询（每轮后求值继续，最多 5 轮，间隔 300ms）')
    expect(code).toContain('并行执行')
  })

  it('count 循环提示固定轮数', () => {
    const code = generateScenarioScript(
      't',
      [{ ...base, loop: { mode: 'count', count: 4 } }],
      { base: 'http://x', stopOnFailure: false },
    )
    expect(code).toContain('循环执行 4 轮')
  })
})})
