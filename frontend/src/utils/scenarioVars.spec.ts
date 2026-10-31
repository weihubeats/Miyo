import { describe, expect, it } from 'vitest'
import { computeVarChain, extractVarRefs, isSensitiveVarName } from './scenarioVars'

describe('extractVarRefs', () => {
  it('提取 {{var}} 占位并去重保序', () => {
    expect(extractVarRefs('/users/{{userId}}/posts?token={{token}}&x={{userId}}')).toEqual([
      'userId',
      'token',
    ])
  })

  it('容忍占位内空白，忽略嵌套花括号残缺', () => {
    expect(extractVarRefs('{{ userId }}')).toEqual(['userId'])
    expect(extractVarRefs('{{ {{a}}')).toEqual(['a'])
    expect(extractVarRefs('no vars')).toEqual([])
  })
})

describe('computeVarChain', () => {
  const step = (index: number, name: string, texts: string[], extracts: { name: string; path: string }[] = []) => ({
    index,
    name,
    extracts: extracts.map((e) => ({ ...e, from: 'body' as const })),
    referenceTexts: texts,
  })

  it('来源与引用位置从真实配置推导', () => {
    const chain = computeVarChain(
      [
        step(0, '登录', [], [{ name: 'token', path: '$.token' }]),
        step(1, '用户文章', ['http://x/api/users/{{userId}}/posts', '{{token}}'], [
          { name: 'userId', path: '$.data.id' },
        ]),
        step(2, '用户详情', ['/users/{{userId}}?t={{token}}']),
      ],
      [],
    )
    const token = chain.find((v) => v.name === 'token')!
    expect(token.sources).toHaveLength(1)
    expect(token.sources[0]).toMatchObject({ stepIndex: 0, stepName: '登录', path: '$.token' })
    expect(token.usages.map((u) => u.stepIndex)).toEqual([1, 2])
    expect(token.missing).toBe(false)

    const userId = chain.find((v) => v.name === 'userId')!
    // userId 在步骤 1 提取、同步骤 URL 引用 → 先引用后定义，运行时取不到。
    expect(userId.missing).toBe(true)
  })

  it('外部变量池命中不计未定义', () => {
    const chain = computeVarChain([step(0, '查询', ['/x?k={{base_url}}'])], ['base_url'])
    expect(chain[0]).toMatchObject({ name: 'base_url', external: true, missing: false })
  })

  it('从未定义且未引用的变量不产生条目；未引用的定义也不产生警示', () => {
    const chain = computeVarChain([step(0, '登录', ['plain url'], [{ name: 'unused', path: '$.a' }])], [])
    const entry = chain.find((v) => v.name === 'unused')
    expect(entry).toBeDefined()
    expect(entry!.missing).toBe(false)
    expect(entry!.usages).toHaveLength(0)
  })

  it('未定义变量：被引用但全链路无定义', () => {
    const chain = computeVarChain([step(0, '查询', ['/orders/{{orderId}}'])], [])
    expect(chain[0]).toMatchObject({ name: 'orderId', missing: true, sources: [] })
  })
})

describe('isSensitiveVarName', () => {
  it('与后端脱敏词根同口径', () => {
    expect(isSensitiveVarName('access_token')).toBe(true)
    expect(isSensitiveVarName('Authorization')).toBe(true)
    expect(isSensitiveVarName('api_key')).toBe(true)
    expect(isSensitiveVarName('userId')).toBe(false)
    expect(isSensitiveVarName('base_url')).toBe(false)
  })
})
