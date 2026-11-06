/**
 * projectStats 单测：仪表板统计聚合契约——
 * 输入必须是 snake_case 载荷（project_id / endpoint_count），
 * camelCase 畸形条目跳过而不是产出 NaN（对应 list_project_stats 曾因
 * Rust camelCase 序列化导致仪表板统计全为 0 的事故）。
 */
import { describe, expect, it } from 'vitest'
import { aggregateProjectStats, sumCounts, totalEndpointCount } from './projectStats'
import type { ProjectStat } from '../types/miyoApi'

const snakeStats: ProjectStat[] = [
  {
    project_id: 'p1',
    endpoint_count: 7,
    scenario_count: 2,
    request_count: 30,
    latest_method: 'GET',
    latest_path: '/pets',
  },
  {
    project_id: 'p2',
    endpoint_count: 6,
    scenario_count: 0,
    request_count: 0,
    latest_method: null,
    latest_path: null,
  },
  {
    project_id: 'p3',
    endpoint_count: 2,
    scenario_count: 1,
    request_count: 5,
    latest_method: 'POST',
    latest_path: '/graphql',
  },
]

describe('aggregateProjectStats', () => {
  it('按 project_id 汇总接口/场景/请求数与最近接口', () => {
    const { counts, scenarioCounts, requestCounts, latest } = aggregateProjectStats(snakeStats)
    expect(counts).toEqual({ p1: 7, p2: 6, p3: 2 })
    expect(scenarioCounts).toEqual({ p1: 2, p2: 0, p3: 1 })
    expect(requestCounts).toEqual({ p1: 30, p2: 0, p3: 5 })
    expect(latest.p1).toEqual({ method: 'GET', path: '/pets' })
    expect(latest.p2).toBeNull()
    expect(latest.p3).toEqual({ method: 'POST', path: '/graphql' })
  })

  it('旧载荷缺 scenario_count / request_count 时按 0 计（后发字段向后兼容）', () => {
    const legacy = [
      { project_id: 'p1', endpoint_count: 3, latest_method: 'GET', latest_path: '/x' },
    ] as unknown as ProjectStat[]
    const { counts, scenarioCounts, requestCounts } = aggregateProjectStats(legacy)
    expect(counts).toEqual({ p1: 3 })
    expect(scenarioCounts).toEqual({ p1: 0 })
    expect(requestCounts).toEqual({ p1: 0 })
  })

  it('camelCase 畸形条目（后端命名断裂回归）整体跳过，不产生 NaN', () => {
    const broken = [
      { projectId: 'p1', endpointCount: 7 },
      { project_id: 'p2', endpoint_count: '6' },
    ] as unknown as ProjectStat[]
    const { counts, latest } = aggregateProjectStats(broken)
    expect(counts).toEqual({})
    expect(latest).toEqual({})
    expect(totalEndpointCount(counts)).toBe(0)
  })

  it('null / undefined 入参安全返回空结果', () => {
    const empty = { counts: {}, scenarioCounts: {}, requestCounts: {}, latest: {} }
    expect(aggregateProjectStats(null)).toEqual(empty)
    expect(aggregateProjectStats(undefined)).toEqual(empty)
  })

  it('接口数为 0 的项目也会保留（区分「无项目」与「零接口」）', () => {
    const { counts } = aggregateProjectStats([
      {
        project_id: 'p1',
        endpoint_count: 0,
        scenario_count: 0,
        request_count: 0,
        latest_method: null,
        latest_path: null,
      },
    ])
    expect(counts).toEqual({ p1: 0 })
  })
})

describe('sumCounts / totalEndpointCount', () => {
  it('求和与空值兜底', () => {
    expect(sumCounts({ a: 7, b: 6, c: 2 })).toBe(15)
    expect(sumCounts({})).toBe(0)
    expect(totalEndpointCount({ a: 7 })).toBe(7)
    expect(totalEndpointCount({})).toBe(0)
  })
})
