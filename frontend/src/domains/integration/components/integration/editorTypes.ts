/**
 * 编辑态类型（WorkflowEditor 与其拆出的 composables 共用）。
 *
 * 编辑态 = 共享规则类型 + 必填化：行编辑器共享组件按 Scenario*Rule 消费，
 * 编辑器内部把可选字段必填化以省去层层判空。
 */
import type {
  StepCondition,
  StepLoop,
  RequestSpec,
  ScenarioExtractRule,
  ScenarioAssertionRule,
  StepOverrides,
} from '@/types/miyoApi.d'

export type EditorExtract = ScenarioExtractRule

export type EditorAssertion = ScenarioAssertionRule & {
  name: string
  type: ScenarioAssertionRule['type']
  path: string
  op: NonNullable<ScenarioAssertionRule['op']>
  expected: string
}

export interface EditorStep {
  key: string
  enabled: boolean
  /** 'inline' = 内联请求；'ref' = 接口引用。 */
  kind: 'inline' | 'ref'
  endpointId: string | null
  name: string
  /** 内联请求（kind=inline 时有值；spec 与面板共享引用原地改）。 */
  inline: { method: string; url: string; spec: RequestSpec }
  extract: EditorExtract[]
  assertions: EditorAssertion[]
  /** 执行条件（None = 无条件）。 */
  condition: StepCondition | null
  /** 循环（None = 单次）。 */
  loop: StepLoop | null
  /** 执行前等待毫秒。 */
  waitMs: number | null
  /** 与相邻 parallel 步骤成批并发。 */
  parallel: boolean
  /** 引用式步骤的请求参数覆盖（None = 沿用接口配置）。 */
  overrides: StepOverrides | null
  /** 步骤重试策略（显式配置才启用；null = 不重试）。 */
  retry: import('@/types/miyoApi.d').StepRetry | null
}
