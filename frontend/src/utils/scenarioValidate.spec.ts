import { describe, expect, it } from 'vitest'
import { validateScenario, summarizeIssues, issuesOfStep, type ValidateStep } from './scenarioValidate'

/** 造一个「干净」的步骤，各用例只改关心的字段。 */
function step(patch: Partial<ValidateStep> = {}): ValidateStep {
  return {
    index: 0,
    enabled: true,
    name: '步骤',
    endpointId: null,
    endpointResolved: true,
    url: '{{base_url}}/orders',
    referenceTexts: [],
    extracts: [],
    assertions: [],
    assertionsParsed: true,
    extractParsed: true,
    condition: null,
    loop: null,
    parallel: false,
    ...patch,
  }
}

const opts = (external: string[] = ['base_url']) => ({ externalNames: external, scenarioName: '收单链路' })

const messages = (issues: ReturnType<typeof validateScenario>) => issues.map((i) => i.message)

describe('validateScenario：场景级', () => {
  it('名称为空、无启用步骤都是阻断', () => {
    const issues = validateScenario([step({ enabled: false })], { ...opts(), scenarioName: '  ' })
    expect(messages(issues)).toContain('integration.vNameEmpty')
    expect(messages(issues)).toContain('integration.vNoEnabledStep')
    expect(summarizeIssues(issues).errors).toBe(2)
  })

  it('未选环境只是提示', () => {
    const issues = validateScenario([step()], { ...opts(), hasEnvironment: false })
    expect(issues).toHaveLength(1)
    expect(issues[0]!.severity).toBe('warning')
  })

  it('干净步骤不产生任何问题', () => {
    expect(validateScenario([step()], opts())).toEqual([])
  })
})

describe('validateScenario：步骤引用与请求模板', () => {
  it('引用式步骤目标接口缺失 → 阻断', () => {
    const issues = validateScenario([step({ endpointId: 'ep-1', endpointResolved: false })], opts())
    expect(messages(issues)).toContain('integration.vEndpointMissing')
    expect(issues[0]!.field).toBe('request')
  })

  it('URL 为空 → 阻断；未闭合占位 → 阻断', () => {
    expect(messages(validateScenario([step({ url: '   ' })], opts()))).toContain('integration.vUrlEmpty')
    expect(messages(validateScenario([step({ url: '{{base_url}}/a{{b' })], opts()))).toContain(
      'integration.vPlaceholderUnclosed',
    )
  })

  it('相对路径无 base_url → 提示不阻断', () => {
    const issues = validateScenario([step({ url: '/orders' })], opts())
    expect(issues).toHaveLength(1)
    expect(issues[0]!.message).toBe('integration.vUrlRelative')
    expect(issues[0]!.severity).toBe('warning')
  })

  it('绝对地址不需要 base_url', () => {
    expect(validateScenario([step({ url: 'https://api.test/orders' })], opts())).toEqual([])
  })
})

describe('validateScenario：变量链路', () => {
  it('引用未定义变量 → 阻断并带定位锚点', () => {
    const issues = validateScenario([step({ referenceTexts: ['{{orderId}}'] })], opts())
    const hit = issues.find((i) => i.message === 'integration.vVarUndefined')
    expect(hit?.stepIndex).toBe(0)
    expect(hit?.subject).toBe('orderId')
    expect(hit?.field).toBe('vars')
  })

  it('同步骤「先引用后提取」运行时取不到值 → 阻断', () => {
    const issues = validateScenario(
      [step({ referenceTexts: ['{{token}}'], extracts: [{ name: 'token', path: '$.token' }] })],
      opts(),
    )
    expect(messages(issues)).toContain('integration.vVarUsedBeforeDefined')
  })

  it('前序步骤定义后引用 → 无问题', () => {
    const issues = validateScenario(
      [
        step({ index: 0, extracts: [{ name: 'orderId', path: '$.id' }] }),
        step({ index: 1, referenceTexts: ['/orders/{{orderId}}'] }),
      ],
      opts(),
    )
    expect(issues).toEqual([])
  })

  it('外部变量不报未定义', () => {
    expect(validateScenario([step({ referenceTexts: ['{{base_url}}'] })], opts())).toEqual([])
  })

  it('禁用步骤既不产生也不消费变量', () => {
    const issues = validateScenario(
      [step({ index: 0, enabled: false, extracts: [{ name: 'orderId', path: '$.id' }] }), step({ index: 1, referenceTexts: ['{{orderId}}'] })],
      opts(),
    )
    expect(messages(issues)).toContain('integration.vVarUndefined')
  })
})

describe('validateScenario：提取规则', () => {
  it('空名 / 非法名 / 重复 / 空路径', () => {
    const issues = validateScenario(
      [
        step({
          extracts: [
            { name: '', path: '$.a' },
            { name: 'bad name', path: '$.b' },
            { name: 'dup', path: '$.c' },
            { name: 'dup', path: '$.d' },
            { name: 'noPath', path: ' ' },
          ],
          referenceTexts: ['{{dup}} {{noPath}}'],
        }),
      ],
      opts(),
    )
    const m = messages(issues)
    expect(m).toContain('integration.vExtractNameEmpty')
    expect(m).toContain('integration.vExtractNameInvalid')
    expect(m).toContain('integration.vExtractDuplicate')
    expect(m).toContain('integration.vExtractPathEmpty')
  })

  it('定义的变量没人用 → 提示', () => {
    const issues = validateScenario([step({ extracts: [{ name: 'unused', path: '$.x' }] })], opts())
    expect(issues).toHaveLength(1)
    expect(issues[0]!.message).toBe('integration.vExtractUnused')
    expect(issues[0]!.severity).toBe('warning')
  })

  it('与环境变量同名 → 提示遮蔽', () => {
    const issues = validateScenario(
      [step({ extracts: [{ name: 'base_url', path: '$.u' }], referenceTexts: ['{{base_url}}'] })],
      opts(),
    )
    expect(messages(issues)).toContain('integration.vExtractShadowsExternal')
  })
})

describe('validateScenario：断言', () => {
  it('缺类型 / 需要路径却没写 / 需要期望值却没写', () => {
    const issues = validateScenario(
      [
        step({
          assertions: [
            { type: '', path: '', op: 'eq', needsExpected: false, expected: '' },
            { type: 'json', path: '', op: 'eq', needsExpected: true, expected: 'ok' },
            { type: 'json', path: '$.status', op: 'eq', needsExpected: true, expected: '' },
          ],
        }),
      ],
      opts(),
    )
    const m = messages(issues)
    expect(m).toContain('integration.vAssertTypeEmpty')
    expect(m).toContain('integration.vAssertPathEmpty')
    expect(m).toContain('integration.vAssertExpectedEmpty')
  })

  it('exists 不需要期望值，写了反而提示', () => {
    const issues = validateScenario(
      [step({ assertions: [{ type: 'json', path: '$.ok', op: 'exists', needsExpected: false, expected: '1' }] })],
      opts(),
    )
    expect(issues).toHaveLength(1)
    expect(issues[0]!.severity).toBe('warning')
  })

  it('断言 JSON 结构非法 → 阻断', () => {
    const issues = validateScenario([step({ assertionsParsed: false })], opts())
    expect(messages(issues)).toContain('integration.vAssertUnparsable')
  })
})

describe('validateScenario：循环', () => {
  const loop = (patch: Partial<NonNullable<ValidateStep['loop']>> = {}) => ({
    mode: 'count',
    count: 3,
    maxLoops: null,
    intervalMs: null,
    whileCondition: null,
    ...patch,
  })

  it('count 合法不报错', () => {
    expect(validateScenario([step({ loop: loop() })], opts())).toEqual([])
  })

  it('count 为 0 / 超上限 → 阻断', () => {
    expect(messages(validateScenario([step({ loop: loop({ count: 0 }) })], opts()))).toContain(
      'integration.vLoopCountInvalid',
    )
    expect(messages(validateScenario([step({ loop: loop({ count: 1001 }) })], opts()))).toContain(
      'integration.vLoopCountInvalid',
    )
  })

  it('while 缺条件 → 阻断；max_loops 越界 → 阻断', () => {
    const issues = validateScenario(
      [step({ loop: loop({ mode: 'while', count: null, maxLoops: 500 }) })],
      opts(),
    )
    const m = messages(issues)
    expect(m).toContain('integration.vLoopWhileEmpty')
    expect(m).toContain('integration.vLoopMaxInvalid')
  })

  it('while 条件带变量名与值 → 无问题', () => {
    const issues = validateScenario(
      [
        step({
          loop: loop({
            mode: 'while',
            count: null,
            maxLoops: 10,
            whileCondition: { name: 'status', op: 'eq', value: 'pending', needsValue: true },
          }),
        }),
      ],
      opts(['base_url', 'status']),
    )
    expect(issues).toEqual([])
  })

  it('间隔超上限 → 阻断', () => {
    expect(messages(validateScenario([step({ loop: loop({ intervalMs: 60_001 }) })], opts()))).toContain(
      'integration.vLoopIntervalInvalid',
    )
  })

  it('mode 非法 → 阻断', () => {
    expect(messages(validateScenario([step({ loop: loop({ mode: 'weird' }) })], opts()))).toContain(
      'integration.vLoopModeInvalid',
    )
  })
})

describe('validateScenario：条件与并行', () => {
  it('条件缺变量名 / 缺期望值', () => {
    const issues = validateScenario(
      [
        step({ condition: { source: 'var', name: ' ', op: 'eq', value: null, needsValue: true } }),
      ],
      opts(),
    )
    const m = messages(issues)
    expect(m).toContain('integration.vCondNoVar')
    expect(m).toContain('integration.vCondNoValue')
  })

  it('prevStatus 条件不需要变量名', () => {
    const issues = validateScenario(
      [step({ condition: { source: 'prevStatus', name: '', op: 'eq', value: 200, needsValue: true } })],
      opts(),
    )
    expect(issues).toEqual([])
  })

  it('条件引用了未定义的运行时变量 → 阻断', () => {
    const issues = validateScenario(
      [step({ condition: { source: 'var', name: 'ghost', op: 'exists', value: null, needsValue: false } })],
      opts(),
    )
    expect(messages(issues)).toContain('integration.vCondVarUndefined')
  })

  it('单个 parallel 步骤退化串行 → 提示', () => {
    const issues = validateScenario([step({ parallel: true })], opts())
    expect(issues).toHaveLength(1)
    expect(issues[0]!.message).toBe('integration.vParallelAlone')
    expect(issues[0]!.severity).toBe('warning')
  })

  it('两个以上连续 parallel 成批 → 无提示', () => {
    const issues = validateScenario([step({ index: 0, parallel: true }), step({ index: 1, parallel: true })], opts())
    expect(issues).toEqual([])
  })
})

describe('validateScenario：输出顺序与汇总', () => {
  it('阻断在前，同级按步骤下标稳定', () => {
    const issues = validateScenario(
      [
        step({ index: 0, extracts: [{ name: 'unused', path: '$.x' }] }),
        step({ index: 1, referenceTexts: ['{{ghost}}'] }),
        step({ index: 2, extracts: [{ name: 'unused2', path: '$.y' }] }),
      ],
      opts(),
    )
    expect(issues[0]!.severity).toBe('error')
    expect(issues[0]!.stepIndex).toBe(1)
    expect(issues.slice(1).every((i) => i.severity === 'warning')).toBe(true)
  })

  it('issuesOfStep 只取该步骤', () => {
    const issues = validateScenario(
      [step({ index: 0, referenceTexts: ['{{ghost}}'] }), step({ index: 1, referenceTexts: ['{{ghost2}}'] })],
      opts(),
    )
    expect(issuesOfStep(issues, 1).map((i) => i.subject)).toEqual(['ghost2'])
  })
})