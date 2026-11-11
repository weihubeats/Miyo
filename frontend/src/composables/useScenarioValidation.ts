/**
 * useScenarioValidation：场景变量链路 + 运行前校验（自 WorkflowEditor 抽出）。
 *
 * 职责（纯派生状态，无 DOM 依赖）：
 * - varScanSteps / varChain：步骤文本扫描 → 变量链路（供连线图与变量面板）；
 * - validateSteps：编辑态 → 校验模型（scenarioValidate 的输入形状）；
 * - issues / issueSummary / errorCount / warnCount：运行前校验结果；
 * - validateOpen：校验面板展开态（出现阻断时自动展开一次，之后用户自控）；
 * - stepIssues：单步问题（步骤卡角标）。
 *
 * 留在宿主的视图层：gotoIssue（切视图 / 选中步骤 / 页签 / 滚动定位）依赖
 * 编辑器局部 UI 状态，不进本组合式。
 */
import { computed, ref, watch, type Ref } from 'vue'
import type { useWorkspaceStore } from '../stores/workspace'
import type { Endpoint } from '../types/miyoApi'
import { computeVarChain, type VarScanStep } from '../utils/scenarioVars'
import {
  validateScenario,
  summarizeIssues,
  issuesOfStep,
  type ValidateStep,
  type ValidateIssue,
} from '../utils/scenarioValidate'
import { mergedVariableMap } from '../utils/environment'
import { applyUrlOverride } from '../utils/stepOverrides'
import type { EditorStep } from '../components/integration/editorTypes'

export interface UseScenarioValidationOptions {
  /** 编辑态步骤数组（WorkflowEditor 的 steps）。 */
  steps: Ref<EditorStep[]>
  workspace: ReturnType<typeof useWorkspaceStore>
  /** 场景名（校验「名称为空」用）。 */
  name: Ref<string>
  /** 引用式步骤的接口解析（宿主持有，含 base_url 模板与路径变量）。 */
  endpointOf: (s: EditorStep) => Endpoint | null
  urlTemplateFor: (ep: Endpoint) => string
  /** 步骤展示名（宿主 i18n 兜底「未命名步骤 N」）。 */
  stepTitle: (s: EditorStep) => string
}

/** 表达式面板里「值型算子」集合：这些算子缺期望值无法求值。 */
const VALUE_OPS = new Set([
  'eq',
  'neq',
  'contains',
  'not_contains',
  'gt',
  'gte',
  'lt',
  'lte',
  'regex',
  'in',
])

export function useScenarioValidation(options: UseScenarioValidationOptions) {
  const { steps, workspace, name, endpointOf, urlTemplateFor, stepTitle } = options

  // ---------- 变量链路（复用 scenarioVars 计算 + ScenarioVarsPanel 展示） ----------
  const varScanSteps = computed<VarScanStep[]>(() =>
    steps.value.map((s, i) => {
      const texts: string[] = []
      if (s.kind === 'inline') {
        texts.push(s.inline.url)
        for (const kv of [...s.inline.spec.params, ...s.inline.spec.headers]) {
          texts.push(kv.value)
        }
        if (s.inline.spec.body.mode === 'json' || s.inline.spec.body.mode === 'text') {
          texts.push(s.inline.spec.body.raw)
        }
      } else {
        const ep = endpointOf(s)
        if (ep) {
          texts.push(urlTemplateFor(ep))
          for (const kv of [...ep.request.params, ...ep.request.headers]) texts.push(kv.value)
          if (ep.request.body.mode === 'json' || ep.request.body.mode === 'text') {
            texts.push(ep.request.body.raw)
          }
        }
      }
      for (const a of s.assertions) texts.push(a.expected)
      return {
        index: i,
        name: stepTitle(s),
        extracts: s.extract.filter((e) => e.name.trim() && e.path.trim()),
        referenceTexts: texts,
      }
    }),
  )

  const externalVarNames = computed(() =>
    Object.keys(
      mergedVariableMap({
        globalVariables: workspace.globalVariables,
        projectVariables: workspace.project?.variables ?? null,
        env: workspace.environments.find((e) => e.id === workspace.activeEnvId),
      }),
    ),
  )
  const varChain = computed(() => computeVarChain(varScanSteps.value, externalVarNames.value))

  // ---------- 运行前校验（纯函数物化，见 utils/scenarioValidate） ----------
  const validateSteps = computed<ValidateStep[]>(() =>
    steps.value.map((s, i) => {
      const ep = s.kind === 'ref' ? endpointOf(s) : null
      const texts = varScanSteps.value[i]?.referenceTexts ?? []
      const cond = s.condition
      const wc = s.loop?.while_condition ?? null
      return {
        index: i,
        enabled: s.enabled,
        name: stepTitle(s),
        endpointId: s.endpointId,
        endpointResolved: s.kind === 'inline' || ep != null,
        url:
          s.kind === 'inline'
            ? s.inline.url
            : ep
              ? applyUrlOverride(urlTemplateFor(ep), s.overrides)
              : '',
        referenceTexts: texts,
        extracts: s.extract.filter((e) => e.name.trim() || e.path.trim()),
        assertions: s.assertions.map((a) => ({
          type: a.type,
          path: a.path,
          op: a.op,
          needsExpected: VALUE_OPS.has(a.op),
          expected: a.expected,
        })),
        assertionsParsed: true,
        extractParsed: true,
        condition: cond
          ? {
              source: (cond.source ?? 'var') as 'var' | 'prevStatus',
              name: cond.name ?? '',
              op: cond.op ?? 'eq',
              value: cond.value ?? null,
              needsValue: VALUE_OPS.has(cond.op ?? 'eq'),
            }
          : null,
        loop: s.loop
          ? {
              mode: s.loop.mode,
              count: s.loop.count ?? null,
              maxLoops: s.loop.max_loops ?? null,
              intervalMs: s.loop.interval_ms ?? null,
              whileCondition: wc
                ? {
                    name: wc.name ?? '',
                    op: wc.op ?? 'eq',
                    value: wc.value ?? null,
                    needsValue: VALUE_OPS.has(wc.op ?? 'eq'),
                  }
                : null,
            }
          : null,
        parallel: s.parallel,
      }
    }),
  )

  const issues = computed(() =>
    validateScenario(validateSteps.value, {
      externalNames: externalVarNames.value,
      scenarioName: name.value,
      hasEnvironment: !!workspace.activeEnvId,
    }),
  )
  const issueSummary = computed(() => summarizeIssues(issues.value))
  const errorCount = computed(() => issueSummary.value.errors)
  const warnCount = computed(() => issueSummary.value.warnings)
  /** 校验面板展开态：有阻断时默认展开，用户可手动收起。 */
  const validateOpen = ref(false)
  watch(
    errorCount,
    (n, prev) => {
      if (n > 0 && !prev) validateOpen.value = true
    },
    { immediate: true },
  )

  /** 单步问题（步骤卡角标）。 */
  function stepIssues(stepIndex: number): ValidateIssue[] {
    return issuesOfStep(issues.value, stepIndex)
  }

  return {
    varScanSteps,
    externalVarNames,
    varChain,
    validateSteps,
    issues,
    issueSummary,
    errorCount,
    warnCount,
    validateOpen,
    stepIssues,
  }
}
