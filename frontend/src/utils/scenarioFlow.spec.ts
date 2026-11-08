import { describe, expect, it } from 'vitest'
import { computeFlowGraph, groupBatches, requiredVarsFrom, toFlowSteps, type FlowStep } from './scenarioFlow'

const fs = (patch: Partial<FlowStep> = {}): FlowStep => ({
  index: 0,
  enabled: true,
  name: '步骤',
  produces: [],
  consumes: [],
  looping: false,
  parallel: false,
  ...patch,
})

describe('computeFlowGraph', () => {
  it('空步骤不产生连线', () => {
    expect(computeFlowGraph([], ['base_url']).edges).toEqual([])
  })

  it('首个启用步骤的连线来自链路入口（from = -1）', () => {
    const { edges } = computeFlowGraph([fs()], [])
    expect(edges).toHaveLength(1)
    expect(edges[0]!.from).toBe(-1)
    expect(edges[0]!.to).toBe(0)
  })

  it('前序定义的变量挂到下一步的连线上', () => {
    const { edges } = computeFlowGraph(
      [
        fs({ index: 0, produces: ['orderId'] }),
        fs({ index: 1, consumes: ['orderId'] }),
      ],
      [],
    )
    expect(edges[1]!.vars).toEqual(['orderId'])
  })

  it('跨多步传递：中间的步骤不消费，也仍从前序定义点流转', () => {
    const { edges, definedBy } = computeFlowGraph(
      [
        fs({ index: 0, produces: ['token'] }),
        fs({ index: 1 }),
        fs({ index: 2, consumes: ['token'] }),
      ],
      [],
    )
    expect(definedBy.get('token')).toBe(0)
    expect(edges[1]!.vars).toEqual([])
    expect(edges[2]!.vars).toEqual(['token'])
  })

  it('外部变量归入 externalVars，不算链路数据流', () => {
    const { edges } = computeFlowGraph([fs({ consumes: ['base_url', 'token'] })], ['base_url', 'token'])
    expect(edges[0]!.vars).toEqual([])
    expect(edges[0]!.externalVars).toEqual(['base_url', 'token'])
  })

  it('后置定义不构成流转（先引用后定义是配置错误）', () => {
    const { edges } = computeFlowGraph(
      [fs({ index: 0, consumes: ['late'] }), fs({ index: 1, produces: ['late'] })],
      [],
    )
    expect(edges[0]!.vars).toEqual([])
    expect(edges[1]!.vars).toEqual([])
  })

  it('禁用步骤不产生变量；连线按启用步骤相邻计算并灰显', () => {
    const { edges, definedBy } = computeFlowGraph(
      [
        fs({ index: 0, enabled: false, produces: ['ghost'] }),
        fs({ index: 1, consumes: ['ghost'] }),
      ],
      [],
    )
    expect(definedBy.has('ghost')).toBe(false)
    expect(edges[0]!.muted).toBe(false) // 首个启用步骤前面没有可灰显的邻居
  })

  it('并行同批的连线标记为虚线（parallel）', () => {
    const { edges } = computeFlowGraph(
      [fs({ index: 0, parallel: true }), fs({ index: 1, parallel: true }), fs({ index: 2 })],
      [],
    )
    expect(edges[1]!.parallel).toBe(true)
    expect(edges[2]!.parallel).toBe(false)
  })

  it('变量去重保序', () => {
    const { edges } = computeFlowGraph(
      [fs({ index: 0, produces: ['a', 'b'] }), fs({ index: 1, consumes: ['b', 'a', 'b'] })],
      [],
    )
    expect(edges[1]!.vars).toEqual(['b', 'a'])
  })
})

describe('toFlowSteps', () => {
  it('从真实配置物化 produces / consumes', () => {
    const out = toFlowSteps([
      {
        index: 0,
        enabled: true,
        name: '登录',
        extracts: [{ name: 'token' }, { name: '  ' }],
        referenceTexts: ['/login?u={{user}}', ''],
        conditionName: null,
        loopWhileName: null,
        looping: false,
        parallel: false,
      },
    ])
    expect(out[0]!.produces).toEqual(['token'])
    expect(out[0]!.consumes).toEqual(['user'])
  })

  it('条件名与 while 条件名也算消费', () => {
    const [s] = toFlowSteps([
      {
        index: 0,
        enabled: true,
        name: '轮询',
        extracts: [],
        referenceTexts: [],
        conditionName: 'ready',
        loopWhileName: 'status',
        looping: true,
        parallel: false,
      },
    ])
    expect(s!.consumes).toEqual(['ready', 'status'])
  })

  it('同一步内重复引用去重', () => {
    const [s] = toFlowSteps([
      {
        index: 0,
        enabled: true,
        name: 'x',
        extracts: [],
        referenceTexts: ['{{a}}', '{{a}}'],
        conditionName: 'a',
        loopWhileName: null,
        looping: false,
        parallel: false,
      },
    ])
    expect(s!.consumes).toEqual(['a'])
  })
})

describe('requiredVarsFrom（手填前置变量的判定）', () => {
  it('起点之前定义的变量需要手填（快照缺失时取不到）', () => {
    const steps = [
      fs({ index: 0, produces: ['token'] }),
      fs({ index: 1, consumes: ['token'] }),
    ]
    const { definedBy } = computeFlowGraph(steps, [])
    expect(requiredVarsFrom(steps, definedBy, [], 1)).toEqual(['token'])
  })

  it('起点之后才定义的变量不需要手填（链路自行补齐）', () => {
    const steps = [
      fs({ index: 0, consumes: ['later'] }),
      fs({ index: 1, produces: ['later'] }),
    ]
    const { definedBy } = computeFlowGraph(steps, [])
    expect(requiredVarsFrom(steps, definedBy, [], 0)).toEqual([])
    expect(requiredVarsFrom(steps, definedBy, [], 1)).toEqual([])
  })

  it('外部变量（环境 / 项目）不需要手填', () => {
    const steps = [fs({ index: 0, consumes: ['base_url'] })]
    const { definedBy } = computeFlowGraph(steps, ['base_url'])
    expect(requiredVarsFrom(steps, definedBy, ['base_url'], 0)).toEqual([])
  })

  it('从未定义的变量需要手填（典型：手工构造的兜底值）', () => {
    const steps = [fs({ index: 0, consumes: ['ghost'] })]
    const { definedBy } = computeFlowGraph(steps, [])
    expect(requiredVarsFrom(steps, definedBy, [], 0)).toEqual(['ghost'])
  })

  it('禁用步骤的依赖不计入', () => {
    const steps = [fs({ index: 0, enabled: false, consumes: ['ghost'] })]
    const { definedBy } = computeFlowGraph(steps, [])
    expect(requiredVarsFrom(steps, definedBy, [], 0)).toEqual([])
  })

  it('去重保序', () => {
    const steps = [
      fs({ index: 0, consumes: ['b', 'a'] }),
      fs({ index: 1, consumes: ['a', 'b'] }),
    ]
    const { definedBy } = computeFlowGraph(steps, [])
    expect(requiredVarsFrom(steps, definedBy, [], 0)).toEqual(['b', 'a'])
  })
})

describe('groupBatches', () => {
  it('连续 parallel 合批，串行步骤各成一档', () => {
    const batches = groupBatches([
      fs({ index: 0 }),
      fs({ index: 1, parallel: true }),
      fs({ index: 2, parallel: true }),
      fs({ index: 3 }),
    ])
    expect(batches.map((b) => b.map((s) => s.index))).toEqual([[0], [1, 2], [3]])
  })

  it('末尾残留的 parallel 也成批', () => {
    const batches = groupBatches([fs({ index: 0 }), fs({ index: 1, parallel: true })])
    expect(batches.map((b) => b.map((s) => s.index))).toEqual([[0], [1]])
  })

  it('空输入返回空数组', () => {
    expect(groupBatches([])).toEqual([])
  })
})