/**
 * stepOverrides：引用式步骤的请求参数覆盖（前端物化用，语义与
 * miyo-core::apply_step_overrides 一致——params/headers/path_variables
 * 按 key 合并（header 键大小写不敏感），body 整体替换）。
 */
import type { KeyValue, RequestSpec, StepOverrides } from '../types/miyoApi'

/** 应用覆盖到请求规格（原地修改）。非法 overrides 抛错（保存期拦截兜底）。 */
export function applyStepOverrides(spec: RequestSpec, overrides: StepOverrides | null | undefined): void {
  if (!overrides) return
  const obj = overrides as Record<string, unknown>
  if (typeof obj !== 'object' || Array.isArray(obj)) {
    throw new Error('integration.overridesInvalid')
  }

  const mergeKv = (field: 'params' | 'headers' | 'path_variables') => {
    const rows = obj[field]
    if (!Array.isArray(rows)) return
    const target = spec[field]
    for (const r of rows) {
      const row = r as Partial<KeyValue>
      if (typeof row.key !== 'string' || !row.key) {
        throw new Error(`integration.overridesInvalid`)
      }
      const value = typeof row.value === 'string' ? row.value : ''
      const enabled = typeof row.enabled === 'boolean' ? row.enabled : true
      const hit =
        field === 'headers'
          ? target.find((e) => e.key.toLowerCase() === row.key!.toLowerCase())
          : target.find((e) => e.key === row.key)
      if (hit) {
        hit.value = value
        hit.enabled = enabled
      } else {
        target.push({
          key: row.key,
          value,
          enabled,
          description: typeof row.description === 'string' ? row.description : '',
        })
      }
    }
  }
  mergeKv('params')
  mergeKv('headers')
  mergeKv('path_variables')

  if (obj.body != null) {
    spec.body = obj.body as RequestSpec['body']
  }
}

/** 应用 url 覆盖（返回最终 URL 模板）。 */
export function applyUrlOverride(urlTemplate: string, overrides: StepOverrides | null | undefined): string {
  return overrides?.url ?? urlTemplate
}
