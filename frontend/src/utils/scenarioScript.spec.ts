/**
 * scenarioScript 生成器单测：链路变量传递（前步提取 → 后步 URL 引用）、
 * 断言映射、URL 模板转换、无 {{}} 残留。
 */
import { describe, expect, it } from 'vitest'
import { generateScenarioScript, jsonPathToJs, urlTemplateToJs } from './scenarioScript'

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
