/**
 * scenarioVars：场景变量链路计算（纯函数，供变量链路面板使用）。
 *
 * 从真实场景配置推导（不写死示例数据）：
 * - 定义：每步 extract 规则的变量名（该步响应后产生）；
 * - 引用：扫描各步的请求模板文本（URL / 头值 / 参数值 / Body 原文 / 断言期望）
 *   中的 {{var}} 占位；
 * - 状态：引用先于定义（含同步引用）或从未定义、且不在外部变量池
 *   （环境 / 项目 / 全局 / 内置）中 → 未定义，面板警示。
 *
 * 运行时值不在此计算——面板直接读 ScenarioRunResult.variables（后端最终变量池）。
 */

/** 与 miyo-sync redact.rs `is_sensitive_key` 同口径的敏感键词根：运行时值默认脱敏。 */
const SENSITIVE_TOKENS = [
  'authorization',
  'cookie',
  'token',
  'secret',
  'password',
  'passwd',
  'pwd',
  'api-key',
  'apikey',
  'api_key',
  'access_key',
  'accesskey',
  'private',
  'credential',
  'sessionid',
  'session_id',
  'x-auth',
] as const

/** 变量名是否命中敏感词根（脱敏展示用，键名不脱敏、仅值）。 */
export function isSensitiveVarName(name: string): boolean {
  const k = name.toLowerCase()
  return SENSITIVE_TOKENS.some((t) => k.includes(t))
}

/** {{var}} 占位提取：返回去重后的变量名（保序）。 */
export function extractVarRefs(text: string): string[] {
  const out: string[] = []
  const re = /\{\{\s*([^{}]+?)\s*\}\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(text))) {
    const name = m[1]!
    if (!out.includes(name)) out.push(name)
  }
  return out
}

/** 变量定义处（某步骤的 extract 规则）。 */
export interface VarSource {
  stepIndex: number
  stepName: string
  from: 'body' | 'header'
  path: string
}

/** 变量引用处（某步骤的请求模板里出现了 {{var}}）。 */
export interface VarUsage {
  stepIndex: number
  stepName: string
}

/** 单个变量的链路条目。 */
export interface VarChainEntry {
  name: string
  /** 定义处列表（未定义变量为空数组）。 */
  sources: VarSource[]
  /** 引用处列表。 */
  usages: VarUsage[]
  /** 未定义：被引用但无定义、也不在外部变量池。 */
  missing: boolean
  /** 外部变量（环境 / 项目 / 全局 / 内置），非链路产物。 */
  external: boolean
}

/** 链路计算的单步输入（全部来自真实配置，由组件物化）。 */
export interface VarScanStep {
  index: number
  name: string
  /** 该步 extract 规则（去空行后的有效行）。 */
  extracts: { name: string; from: 'body' | 'header'; path: string }[]
  /** 参与引用扫描的模板文本（URL / 头值 / 参数值 / Body / 断言期望…）。 */
  referenceTexts: string[]
}

/**
 * 计算变量链路。
 *
 * 状态判定规则：步骤 j 引用的变量 v 有效，当且仅当某个更早步骤 i<j 定义了 v，
 * 或 v 在外部变量池中；同步骤内「先引用后提取」在运行时取不到值（请求先于
 * 响应），同样计为未定义。变量名区分大小写（与后端 VariableMap 渲染一致）。
 *
 * @param steps 按执行顺序排列的启用步骤
 * @param externalNames 外部变量池（环境 / 项目 / 全局 / 内置），键名集合
 */
export function computeVarChain(steps: VarScanStep[], externalNames: Iterable<string>): VarChainEntry[] {
  const external = new Set(externalNames)
  interface Entry {
    name: string
    sources: VarSource[]
    usages: VarUsage[]
    /** 引用处里最早出现的步骤序号（判定「先引用后定义」用）。 */
    firstUsedAt: number | null
  }
  const order: string[] = []
  const map = new Map<string, Entry>()
  const entryOf = (name: string): Entry => {
    let e = map.get(name)
    if (!e) {
      e = { name, sources: [], usages: [], firstUsedAt: null }
      map.set(name, e)
      order.push(name)
    }
    return e
  }

  for (const step of steps) {
    for (const r of step.extracts) {
      const e = entryOf(r.name)
      if (!e.sources.some((s) => s.stepIndex === step.index)) {
        e.sources.push({ stepIndex: step.index, stepName: step.name, from: r.from, path: r.path })
      }
    }
    const seenInStep = new Set<string>()
    for (const text of step.referenceTexts) {
      if (!text) continue
      for (const name of extractVarRefs(text)) {
        if (seenInStep.has(name)) continue
        seenInStep.add(name)
        const e = entryOf(name)
        if (e.firstUsedAt == null) e.firstUsedAt = step.index
        if (!e.usages.some((u) => u.stepIndex === step.index)) {
          e.usages.push({ stepIndex: step.index, stepName: step.name })
        }
      }
    }
  }

  return order.map((name) => {
    const e = map.get(name)!
    const externalVar = !e.sources.length && external.has(name)
    // 未定义：无定义处、非外部变量，但被引用；或定义晚于最早引用（运行时同样取不到）。
    const definedBeforeUse = e.sources.some((s) => e.firstUsedAt == null || s.stepIndex < e.firstUsedAt)
    const missing = !externalVar && !definedBeforeUse && e.usages.length > 0
    return { name, sources: e.sources, usages: e.usages, missing, external: externalVar }
  })
}
