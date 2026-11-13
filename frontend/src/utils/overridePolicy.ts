/**
 * overridePolicy：引用式步骤的「覆盖策略」三态（纯函数）。
 *
 * 后端 `apply_step_overrides` 只有一种语义：**按 key 合并**（params / headers /
 * path_variables 逐行覆盖，body 整体替换），未提供字段沿用接口配置。UI 若只暴露
 * 「有没有 overrides」，用户无法表达「我要把接口带的这个 header 删掉」——
 * 这正是策略三态要补的：
 *
 * - `inherit`（继承）：完全不覆盖，全部沿用接口当前配置；
 * - `merge`（合并）：只覆盖出现过的项，其余沿用（= 后端原生语义）；
 * - `replace`（替换）：整体接管 params / headers / path_variables，接口里多出来的
 *   行不再生效（实现方式是把接口的其余行显式写入 overrides，禁用态）。
 *
 * 三态在数据上只是 `overrides` 的不同内容，**没有新增持久化字段**，因此
 * ScenarioStep 序列化保持兼容，MCP 契约不变。
 */
import type { KeyValue, RequestSpec, StepOverrides } from '@/types/miyoApi'

export type OverridePolicy = 'inherit' | 'merge' | 'replace'

/** 三态选项（宿主渲染下拉用，文案由调用方翻译）。 */
export const OVERRIDE_POLICIES: OverridePolicy[] = ['inherit', 'merge', 'replace']

/**
 * 推断当前 overrides 属于哪种策略。
 *
 * 判定不了「替换」时一律回落到「合并」：行为更保守，且是后端原生语义。
 * 判据：接口里每一行 params / headers / path_variables 都在 overrides 中出现
 * （header 大小写不敏感），且 body 被整体替换过 → 视为替换。
 */
export function detectPolicy(spec: RequestSpec | null, overrides: StepOverrides | null | undefined): OverridePolicy {
  if (!overrides) return 'inherit'
  const hasAny =
    overrides.url != null ||
    overrides.body != null ||
    (overrides.params?.length ?? 0) > 0 ||
    (overrides.headers?.length ?? 0) > 0 ||
    (overrides.path_variables?.length ?? 0) > 0
  if (!hasAny) return 'inherit'
  if (!spec) return 'merge'

  const covers = (field: 'params' | 'headers' | 'path_variables'): boolean => {
    const base = spec[field] ?? []
    if (!base.length) return true
    const rows = overrides[field] ?? []
    return base.every((row) =>
      rows.some((r) =>
        field === 'headers'
          ? (r.key ?? '').toLowerCase() === row.key.toLowerCase()
          : (r.key ?? '') === row.key,
      ),
    )
  }
  const coversBody = spec.body?.mode === 'none' || overrides.body != null
  return covers('params') && covers('headers') && covers('path_variables') && coversBody ? 'replace' : 'merge'
}

/**
 * 把一条「禁用态」的继承行物化进 overrides（replace 用）。
 *
 * 只是把接口已有行**原样**写进 overrides（enabled 跟随接口），让后端把它们
 * 当作步骤配置，从而与「接口里多出来的行」解耦——但注意：这并不删除接口行，
 * 只是让 UI 有完整可编辑的行集合。真正的删除靠 enabled=false。
 */
function inheritRow(row: KeyValue): Partial<KeyValue> {
  // 保留设计元数据（field_type/required/example）：replace 重建行时不丢用户标注
  return {
    key: row.key,
    value: row.value,
    enabled: row.enabled,
    description: row.description,
    field_type: row.field_type ?? 'string',
    required: row.required ?? true,
    example: row.example ?? '',
  }
}

/**
 * 切换策略，返回新的 overrides（不修改入参）。
 *
 * @param spec 接口当前配置（inherit → replace 时用来预填完整行集合）
 * @param prev 现有 overrides
 */
export function applyPolicy(policy: OverridePolicy, spec: RequestSpec | null, prev: StepOverrides | null | undefined): StepOverrides | null {
  if (policy === 'inherit') return null
  if (policy === 'merge') {
    // 从替换退回合并：保留用户手写的行，去掉仅用于「吃住接口行」的继承副本。
    if (!prev || !spec) return prev ?? null
    const strip = (field: 'params' | 'headers' | 'path_variables'): Partial<KeyValue>[] | undefined => {
      const rows = prev[field]
      if (!rows?.length) return rows
      const base = spec[field] ?? []
      const kept = rows.filter(
        (r) => !base.some((b) => (field === 'headers' ? b.key.toLowerCase() === (r.key ?? '').toLowerCase() : b.key === r.key) && b.value === r.value && b.enabled === r.enabled),
      )
      return kept.length ? kept : undefined
    }
    return {
      url: prev.url,
      body: prev.body,
      params: strip('params'),
      headers: strip('headers'),
      path_variables: strip('path_variables'),
    }
  }
  // replace：以接口全量行作为起点，已有覆盖行优先。
  if (!spec) return prev ?? { params: [], headers: [] }
  const take = (field: 'params' | 'headers' | 'path_variables'): Partial<KeyValue>[] => {
    const base = (spec[field] ?? []).map(inheritRow)
    const over = prev?.[field] ?? []
    for (const o of over) {
      const hit = base.find((b) =>
        field === 'headers' ? (b.key ?? '').toLowerCase() === (o.key ?? '').toLowerCase() : b.key === o.key,
      )
      if (hit) {
        hit.value = o.value ?? hit.value
        hit.enabled = o.enabled ?? hit.enabled
        hit.description = o.description ?? hit.description
      } else {
        base.push(o)
      }
    }
    return base
  }
  return {
    url: prev?.url,
    body: prev?.body ?? (spec.body?.mode !== 'none' ? spec.body : undefined),
    params: take('params'),
    headers: take('headers'),
    path_variables: take('path_variables'),
  }
}

/** 策略的 i18n 键（宿主渲染下拉时用）。 */
export function policyLabelKey(policy: OverridePolicy): string {
  return `integration.policy.${policy}`
}