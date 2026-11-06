/**
 * projectStats：仪表板项目统计聚合（list_project_stats 载荷 → 计数 / 最近接口）。
 *
 * 输入为后端 snake_case 载荷（miyoApi.d.ts ProjectStat）。曾因 Rust 侧
 * camelCase 序列化导致前端读取 undefined、统计显示异常——本模块对每条
 * 记录做显式字段校验：畸形条目（含 camelCase 键）直接跳过，宁显示 0
 * 不产出 NaN；Rust 侧由序列化契约测试锁死键名。
 *
 * scenario_count / request_count 为后发字段：缺省（旧载荷）按 0 计，
 * 不作为条目有效性判据（有效判据仍以 endpoint_count 为准）。
 */
import type { HttpMethod, ProjectStat } from '../types/miyoApi'

export interface LatestEndpoint {
  method: HttpMethod
  path: string
}

export interface AggregatedProjectStats {
  counts: Record<string, number>
  scenarioCounts: Record<string, number>
  requestCounts: Record<string, number>
  latest: Record<string, LatestEndpoint | null>
}

/** 数值字段兜底：缺失/畸形一律按 0，不产出 NaN。 */
function safeCount(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : 0
}

/** 统计聚合：按 project_id 汇总接口/场景/请求数与最近更新的接口；畸形条目跳过。 */
export function aggregateProjectStats(stats: ProjectStat[] | null | undefined): AggregatedProjectStats {
  const counts: Record<string, number> = {}
  const scenarioCounts: Record<string, number> = {}
  const requestCounts: Record<string, number> = {}
  const latest: Record<string, LatestEndpoint | null> = {}
  for (const s of stats ?? []) {
    if (typeof s?.project_id !== 'string' || typeof s.endpoint_count !== 'number') continue
    counts[s.project_id] = s.endpoint_count
    scenarioCounts[s.project_id] = safeCount(s.scenario_count)
    requestCounts[s.project_id] = safeCount(s.request_count)
    latest[s.project_id] =
      typeof s.latest_method === 'string' && typeof s.latest_path === 'string'
        ? { method: s.latest_method as HttpMethod, path: s.latest_path }
        : null
  }
  return { counts, scenarioCounts, requestCounts, latest }
}

/** 计数字典求和（无有效条目时为 0，不会出现 NaN）。 */
export function sumCounts(counts: Record<string, number>): number {
  return Object.values(counts).reduce((a, b) => a + b, 0)
}

/** 全部项目接口总数。 */
export function totalEndpointCount(counts: Record<string, number>): number {
  return sumCounts(counts)
}
