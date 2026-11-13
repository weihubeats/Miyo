/**
 * runStepBuild：已保存场景（Scenario.steps）→ 运行入参步骤（BuiltRunStep）。
 *
 * 与 WorkflowEditor 的 buildOneRunStep 同语义（引用式克隆接口配置 + 应用
 * overrides；内联步骤自持规格），供 AI 测试工作区复用运行链路
 * （useScenarioRun + run_scenario）而不必经过编辑态。
 */
import type {
  Endpoint,
  Scenario,
  ScenarioStep,
} from '../types/miyoApi'
import { tFallback } from '../stores/locale'
import { applyStepOverrides, applyUrlOverride } from './stepOverrides'
import type { BuiltRunStep } from '../composables/useScenarioRun'

/** 编辑态断言 → 引擎断言（expected 保留原值：字符串数字由引擎宽松比较）。 */
function normalizeAssertions(raw: unknown): BuiltRunStep['assertions'] {
  if (!Array.isArray(raw)) return []
  return raw
    .filter((r) => r && typeof r === 'object' && (r as { type?: string }).type)
    .map((r) => {
      const row = r as {
        type: string
        path?: string | null
        op?: string
        expected?: unknown
      }
      return {
        type: row.type,
        path: row.path?.trim() || null,
        op: row.op || 'eq',
        expected: row.expected,
      }
    })
}

/** 提取规则过滤（name + path 都非空才有效）。 */
function normalizeExtract(raw: unknown): BuiltRunStep['extract'] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((r) => r as { name?: string; path?: string })
    .filter((r) => r.name?.trim() && r.path?.trim())
    .map((r) => ({ name: r.name!.trim(), path: r.path!.trim() }))
}

/** 单步物化；disabled / 接口缺失返回 null（跳过该步）。 */
function buildOne(step: ScenarioStep, endpointById: Map<string, Endpoint>, index: number): BuiltRunStep | null {
  if (!step.enabled) return null
  const extract = normalizeExtract(step.extract)
  const assertions = normalizeAssertions(step.assertions)

  if (step.request) {
    // 内联步骤：请求定义自持。
    const spec = JSON.parse(JSON.stringify(step.request.spec ?? {}))
    return {
      retry: step.retry ?? null,
      index,
      endpoint: {
        id: `inline-${index}`,
        project_id: '',
        folder_id: null,
        name: step.name?.trim() || tFallback('integration.stepUntitled', { n: index + 1 }),
        method: step.request.method as Endpoint['method'],
        path: step.request.url,
        description: '',
        status: 'developing',
        sort_order: 0,
        request: spec,
        created_at: '',
        updated_at: '',
      },
      url: step.request.url,
      enabled: true,
      extract,
      assertions,
      condition: step.condition ?? null,
      loop: step.loop ?? null,
      wait_ms: step.wait_ms ?? null,
      parallel: step.parallel ?? false,
    }
  }

  // 引用式：接口当前配置 + 步骤覆盖。
  if (!step.endpoint_id) return null
  const ep = endpointById.get(step.endpoint_id)
  if (!ep) return null
  const spec: import('../types/miyoApi').RequestSpec = JSON.parse(JSON.stringify(ep.request))
  try {
    applyStepOverrides(spec, step.overrides)
  } catch {
    return null
  }
  return {
    retry: step.retry ?? null,
    index,
    endpoint: { ...ep, request: spec },
    url: applyUrlOverride(ep.path, step.overrides),
    enabled: true,
    extract,
    assertions,
    condition: step.condition ?? null,
    loop: step.loop ?? null,
    wait_ms: step.wait_ms ?? null,
    parallel: step.parallel ?? false,
  }
}

/** 场景步骤数组 → 运行入参（保持位次映射：disabled 步留位次但不执行）。 */
export function buildRunStepsFromSteps(
  steps: ScenarioStep[],
  endpoints: Endpoint[],
): BuiltRunStep[] {
  const byId = new Map(endpoints.map((e) => [e.id, e]))
  const out: BuiltRunStep[] = []
  steps.forEach((s, i) => {
    const built = buildOne(s, byId, i)
    if (built) out.push(built)
  })
  return out
}

/** 场景主步骤物化。 */
export function buildRunStepsFromScenario(
  scenario: Scenario,
  endpoints: Endpoint[],
): BuiltRunStep[] {
  return buildRunStepsFromSteps(scenario.steps ?? [], endpoints)
}
