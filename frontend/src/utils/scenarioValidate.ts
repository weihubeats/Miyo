/**
 * scenarioValidate：场景运行前校验（纯函数，无 Vue / 无 IO）。
 *
 * 校验分层：
 * - **阻断（error）**：跑起来必然失败或产生非预期副作用，运行前必须修；
 * - **提示（warning）**：大概率是配置写错，但允许「仍然运行」（轮询类场景常在
 *   首轮前变量还没值）。
 *
 * 八类检查与需求逐条对应：
 * 1. 场景级：名称为空、无启用步骤、环境未选（生产环境由宿主另行提示）；
 * 2. 步骤引用：引用式步骤目标接口缺失（已删除 / 跨项目）；
 * 3. 请求模板：URL 为空、未闭合 `{{`、URL 缺占位且不像绝对地址；
 * 4. 变量：引用了从未定义、也不在外部变量池的变量；定义了却没人用（warning）；
 * 5. 提取规则：变量名为空 / 非法字符、路径为空、同一步重复定义；覆盖还是
 *    warning；
 * 6. 断言：类型缺省、需要路径却没写、需要期望值却没写、算子与类型不匹配；
 * 7. 循环：mode 非法、count 越界、while 缺条件、上限越界、间隔越界；
 * 8. 条件 / 并行：条件源为 var 却缺变量名、算子缺期望值；并行标记不成批
 *    （单个 parallel 步骤退化串行，warning）。
 *
 * 返回项带 `stepIndex`（-1 = 场景级）与 `field`，宿主据此定位到步骤与页签。
 */
import { extractVarRefs } from '@/utils/scenarioVars'

/** 定位锚点：步骤页签（`control` = 条件 / 循环 / 等待）。 */
export type ValidateField = 'request' | 'control' | 'extract' | 'assert' | 'vars' | 'general'

export type ValidateSeverity = 'error' | 'warning'

/** 单条校验结果。`message` 是 i18n 键（`detail` 为插值参数）。 */
export interface ValidateIssue {
  severity: ValidateSeverity
  /** 0 基步骤下标；-1 = 场景级（无步骤可定位）。 */
  stepIndex: number
  field: ValidateField
  /** i18n 键。 */
  message: string
  /** 消息插值参数。 */
  detail?: Record<string, string | number>
  /** 参与定位的子项键（变量名 / 规则名），用于在页签内高亮。 */
  subject?: string
}

/** 供校验器读取的最小步骤视图（宿主从编辑态物化，避免与编辑器类型耦合）。 */
export interface ValidateStep {
  index: number
  enabled: boolean
  /** 展示名（空名由场景级 name 提示覆盖）。 */
  name: string
  /** 引用式步骤的接口 id；内联步骤为 null。 */
  endpointId: string | null
  /** 引用目标是否仍存在（内联步骤恒 true）。 */
  endpointResolved: boolean
  /** 最终 URL 模板（引用式步骤含 overrides.url）。 */
  url: string
  /** 参与 `{{var}}` 引用的模板文本（URL / 参数 / 头 / Body / 断言期望）。 */
  referenceTexts: string[]
  /** 有效提取规则（name 非空即计入定义；path 空单独报错）。 */
  extracts: { name: string; path: string; from?: string }[]
  /** 有效断言规则。 */
  assertions: { type: string; path: string; op: string; needsExpected: boolean; expected: string }[]
  /** 步骤级断言原始值是否可解析（宿主传 false 表示 JSON 结构非法）。 */
  assertionsParsed: boolean
  /** 提取原始值是否可解析。 */
  extractParsed: boolean
  condition: {
    source: 'var' | 'prevStatus'
    name: string
    op: string
    value: unknown
    needsValue: boolean
  } | null
  loop: {
    mode: string
    count: number | null
    maxLoops: number | null
    intervalMs: number | null
    /** while 模式的继续条件。 */
    whileCondition: { name: string; op: string; value: unknown; needsValue: boolean } | null
  } | null
  parallel: boolean
}

export interface ValidateOptions {
  /** 外部变量池（环境 / 项目 / 全局 / 内置）键名。 */
  externalNames: Iterable<string>
  /** 场景名（空 → error）。 */
  scenarioName: string
  /** 是否已选环境（缺省视为已选，宿主按需传 false）。 */
  hasEnvironment?: boolean
  /** 允许的运行上限常量（与后端 StepLoop 同口径）。 */
  maxLoopCount?: number
  maxWhileLoops?: number
  maxIntervalMs?: number
}

const DEFAULT_MAX_COUNT = 1000
const DEFAULT_MAX_LOOPS = 100
const DEFAULT_MAX_INTERVAL = 60_000

/** 合法变量名：字母 / 数字 / 下划线 / 短横 / 点。后端按字符串键取值，宽松即可。 */
const VAR_NAME_RE = /^[\w.-]+$/

/** 值型算子（与断言面板同集，缺值即无法求值）。 */
const VALUE_OPS = new Set(['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'regex', 'in'])
/** 无值算子。 */
const NO_VALUE_OPS = new Set(['exists', 'empty', 'not_empty'])

/** 断言类型：需要目标路径的类型（其余如 status / duration / json_path 另论）。 */
const PATH_TYPES = new Set(['json', 'json_path', 'header', 'body', 'text'])

function condNeedsValue(op: string): boolean {
  return VALUE_OPS.has(op)
}

/** 模板里是否存在未闭合的 `{{`（`{{a{{b}}` 这类真实打错常被渲染器原样输出）。 */
function hasUnclosedPlaceholder(text: string): boolean {
  // 去掉成对占位后仍残留 `{{` 即为未闭合。
  const stripped = text.replace(/\{\{[^{}]*\}\}/g, '')
  return stripped.includes('{{')
}

/**
 * 执行全套校验。返回顺序：阻断在前，随后按步骤下标稳定排列，
 * 便于面板「先看致命问题」。
 */
export function validateScenario(steps: ValidateStep[], options: ValidateOptions): ValidateIssue[] {
  const issues: ValidateIssue[] = []
  const external = new Set(options.externalNames)
  const maxCount = options.maxLoopCount ?? DEFAULT_MAX_COUNT
  const maxLoops = options.maxWhileLoops ?? DEFAULT_MAX_LOOPS
  const maxInterval = options.maxIntervalMs ?? DEFAULT_MAX_INTERVAL

  const push = (i: Omit<ValidateIssue, 'severity'> & { severity?: ValidateSeverity }) => {
    issues.push({ severity: i.severity ?? 'error', ...i })
  }

  // ---------- 1. 场景级 ----------
  if (!options.scenarioName.trim()) {
    push({ stepIndex: -1, field: 'general', message: 'integration.vNameEmpty' })
  }
  const active = steps.filter((s) => s.enabled)
  if (!active.length) {
    push({ stepIndex: -1, field: 'general', message: 'integration.vNoEnabledStep' })
  }
  if (options.hasEnvironment === false) {
    push({ stepIndex: -1, field: 'general', message: 'integration.vNoEnvironment', severity: 'warning' })
  }

  // ---------- 变量定义表（仅启用步骤；禁用步骤不产生也不消费运行时值） ----------
  const definedAt = new Map<string, number>()
  for (const s of active) {
    for (const r of s.extracts) {
      const name = r.name.trim()
      if (!name) continue
      if (!definedAt.has(name)) definedAt.set(name, s.index)
    }
  }

  for (const s of active) {
    const label = s.name.trim() || `#${s.index + 1}`

    // ---------- 2. 引用式步骤 ----------
    if (s.endpointId && !s.endpointResolved) {
      push({
        stepIndex: s.index,
        field: 'request',
        message: 'integration.vEndpointMissing',
        detail: { step: label },
      })
    }

    // ---------- 3. 请求模板 ----------
    const url = s.url.trim()
    if (!url) {
      push({ stepIndex: s.index, field: 'request', message: 'integration.vUrlEmpty', detail: { step: label } })
    } else {
      if (hasUnclosedPlaceholder(url)) {
        push({ stepIndex: s.index, field: 'request', message: 'integration.vPlaceholderUnclosed', detail: { step: label } })
      }
      // 既无 {{base_url}} 也无 http(s):// 前缀 → 运行时 URL 不合法。
      const absolute = /^[a-zA-Z][\w+.-]*:\/\//.test(url)
      const refs = extractVarRefs(url)
      if (!absolute && !refs.includes('base_url')) {
        push({
          stepIndex: s.index,
          field: 'request',
          message: 'integration.vUrlRelative',
          detail: { step: label },
          severity: 'warning',
        })
      }
      // {{var}} 无法渲染成合法 URL 的一部分（如空格）：提示而非阻断。
      if (/\s/.test(url.replace(/\{\{[^}]*\}\}/g, ''))) {
        push({ stepIndex: s.index, field: 'request', message: 'integration.vUrlSpace', detail: { step: label }, severity: 'warning' })
      }
    }

    // ---------- 4. 变量引用 ----------
    for (const text of s.referenceTexts) {
      if (!text) continue
      for (const name of extractVarRefs(text)) {
        if (external.has(name)) continue
        const at = definedAt.get(name)
        // 同步骤内「先引用后提取」运行时取不到值（请求先于响应）。
        if (at == null || at >= s.index) {
          push({
            stepIndex: s.index,
            field: 'vars',
            message: at == null ? 'integration.vVarUndefined' : 'integration.vVarUsedBeforeDefined',
            detail: { step: label, name },
            subject: name,
          })
        }
      }
    }

    // ---------- 5. 提取规则 ----------
    if (!s.extractParsed) {
      push({ stepIndex: s.index, field: 'extract', message: 'integration.vExtractUnparsable', detail: { step: label } })
    }
    if (!s.assertionsParsed) {
      push({ stepIndex: s.index, field: 'assert', message: 'integration.vAssertUnparsable', detail: { step: label } })
    }
    const seenExtract = new Set<string>()
    for (const r of s.extracts) {
      const name = r.name.trim()
      if (!name) {
        push({ stepIndex: s.index, field: 'extract', message: 'integration.vExtractNameEmpty', detail: { step: label } })
        continue
      }
      if (!VAR_NAME_RE.test(name)) {
        push({
          stepIndex: s.index,
          field: 'extract',
          message: 'integration.vExtractNameInvalid',
          detail: { step: label, name },
          subject: name,
        })
      }
      if (seenExtract.has(name)) {
        push({
          stepIndex: s.index,
          field: 'extract',
          message: 'integration.vExtractDuplicate',
          detail: { step: label, name },
          subject: name,
        })
      }
      seenExtract.add(name)
      if (!r.path.trim()) {
        push({
          stepIndex: s.index,
          field: 'extract',
          message: 'integration.vExtractPathEmpty',
          detail: { step: label, name },
          subject: name,
        })
      }
      if (external.has(name)) {
        push({
          stepIndex: s.index,
          field: 'extract',
          message: 'integration.vExtractShadowsExternal',
          detail: { step: label, name },
          subject: name,
          severity: 'warning',
        })
      }
    }

    // ---------- 6. 断言 ----------
    for (const a of s.assertions) {
      if (!a.type) {
        push({ stepIndex: s.index, field: 'assert', message: 'integration.vAssertTypeEmpty', detail: { step: label } })
        continue
      }
      if (PATH_TYPES.has(a.type) && !a.path.trim()) {
        push({
          stepIndex: s.index,
          field: 'assert',
          message: 'integration.vAssertPathEmpty',
          detail: { step: label, type: a.type },
        })
      }
      if (!VALUE_OPS.has(a.op) && !NO_VALUE_OPS.has(a.op)) {
        push({
          stepIndex: s.index,
          field: 'assert',
          message: 'integration.vAssertOpUnknown',
          detail: { step: label, op: a.op },
        })
      } else if (NO_VALUE_OPS.has(a.op) && a.expected.trim()) {
        push({
          stepIndex: s.index,
          field: 'assert',
          message: 'integration.vAssertExpectedUnexpected',
          detail: { step: label, op: a.op },
          severity: 'warning',
        })
      } else if (a.needsExpected && !a.expected.trim()) {
        push({
          stepIndex: s.index,
          field: 'assert',
          message: 'integration.vAssertExpectedEmpty',
          detail: { step: label, op: a.op },
        })
      }
    }

    // ---------- 7. 循环 ----------
    const loop = s.loop
    if (loop) {
      if (loop.mode === 'count') {
        const n = loop.count
        if (n == null || !Number.isFinite(n) || n < 1 || n > maxCount) {
          push({
            stepIndex: s.index,
            field: 'control',
            message: 'integration.vLoopCountInvalid',
            detail: { step: label, max: maxCount },
          })
        }
      } else if (loop.mode === 'while') {
        const wc = loop.whileCondition
        if (!wc || (!wc.name.trim() && wc.op !== 'exists' && wc.op !== 'empty')) {
          push({ stepIndex: s.index, field: 'control', message: 'integration.vLoopWhileEmpty', detail: { step: label } })
        } else if (wc.needsValue && wc.value === null) {
          push({ stepIndex: s.index, field: 'control', message: 'integration.vLoopWhileNoValue', detail: { step: label } })
        }
        if (loop.maxLoops != null && (loop.maxLoops < 1 || loop.maxLoops > maxLoops)) {
          push({
            stepIndex: s.index,
            field: 'control',
            message: 'integration.vLoopMaxInvalid',
            detail: { step: label, max: maxLoops },
          })
        }
        // 轮询类断言必然首轮前变量还没值——这里只提示，不阻断。
        if (wc && !wc.name.trim()) {
          push({ stepIndex: s.index, field: 'control', message: 'integration.vLoopWhileNoVar', detail: { step: label } })
        }
      } else {
        push({ stepIndex: s.index, field: 'control', message: 'integration.vLoopModeInvalid', detail: { step: label } })
      }
      if (loop.intervalMs != null && (loop.intervalMs < 0 || loop.intervalMs > maxInterval)) {
        push({
          stepIndex: s.index,
          field: 'control',
          message: 'integration.vLoopIntervalInvalid',
          detail: { step: label, max: maxInterval },
        })
      }
    }

    // ---------- 8. 条件 ----------
    const c = s.condition
    if (c) {
      if (c.source === 'var' && !c.name.trim()) {
        push({ stepIndex: s.index, field: 'control', message: 'integration.vCondNoVar', detail: { step: label } })
      }
      if (c.needsValue && c.value === null) {
        push({ stepIndex: s.index, field: 'control', message: 'integration.vCondNoValue', detail: { step: label } })
      }
      if (c.source === 'var' && c.name.trim() && !external.has(c.name.trim())) {
        const at = definedAt.get(c.name.trim())
        if (at == null || at >= s.index) {
          push({
            stepIndex: s.index,
            field: 'control',
            message: 'integration.vCondVarUndefined',
            detail: { step: label, name: c.name.trim() },
            subject: c.name.trim(),
          })
        }
      }
    }
  }

  // ---------- 并行批次（warning：单个 parallel 退化串行，多半是意图没达成） ----------
  let run: ValidateStep[] = []
  const flushParallel = () => {
    if (run.length === 1) {
      const only = run[0]!
      push({
        stepIndex: only.index,
        field: 'control',
        message: 'integration.vParallelAlone',
        detail: { step: only.name.trim() || `#${only.index + 1}` },
        severity: 'warning',
      })
    }
    run = []
  }
  for (const s of active) {
    if (s.parallel) {
      run.push(s)
    } else if (run.length) {
      flushParallel()
    }
  }
  if (run.length) flushParallel()

  // ---------- 未被引用的提取变量（warning：常见于改名后遗留） ----------
  const usedNames = new Set<string>()
  for (const s of active) {
    for (const text of s.referenceTexts) {
      if (!text) continue
      for (const n of extractVarRefs(text)) usedNames.add(n)
    }
    if (s.condition?.name) usedNames.add(s.condition.name.trim())
    if (s.loop?.whileCondition?.name) usedNames.add(s.loop.whileCondition.name.trim())
  }
  for (const [name, at] of definedAt) {
    if (!usedNames.has(name) && !external.has(name)) {
      const step = active.find((s) => s.index === at)!
      push({
        stepIndex: at,
        field: 'extract',
        message: 'integration.vExtractUnused',
        detail: { step: step.name.trim() || `#${at + 1}`, name },
        subject: name,
        severity: 'warning',
      })
    }
  }

  // 排序：error 在前，同级别按步骤下标稳定排序。
  return issues
    .map((issue, seq) => ({ issue, seq }))
    .sort((a, b) => {
      const sev = (a.issue.severity === 'error' ? 0 : 1) - (b.issue.severity === 'error' ? 0 : 1)
      if (sev !== 0) return sev
      if (a.issue.stepIndex !== b.issue.stepIndex) return a.issue.stepIndex - b.issue.stepIndex
      return a.seq - b.seq
    })
    .map((x) => x.issue)
}

/** 汇总：阻断数 / 提示数（顶栏与运行按钮用）。 */
export function summarizeIssues(issues: ValidateIssue[]): { errors: number; warnings: number } {
  let errors = 0
  let warnings = 0
  for (const i of issues) {
    if (i.severity === 'error') errors += 1
    else warnings += 1
  }
  return { errors, warnings }
}

/** 单步骤的问题（步骤卡角标用）。 */
export function issuesOfStep(issues: ValidateIssue[], stepIndex: number): ValidateIssue[] {
  return issues.filter((i) => i.stepIndex === stepIndex)
}

export { condNeedsValue }