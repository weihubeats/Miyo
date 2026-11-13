/**
 * scenarioFlow：工作流的「连线」与「变量流向」计算（纯函数）。
 *
 * 编辑器把步骤纵向堆叠，步骤之间画一条连接线；线上挂的是**从前一步流向后一步
 * 的变量**。要画出这条线，需要知道每个变量在哪一步产生（extract）以及在哪一步
 * 被消费（`{{var}}` 引用 / 条件名 / while 条件名）。
 *
 * 与 `scenarioVars.computeVarChain` 的分工：
 * - computeVarChain 面向「变量面板」：按变量聚合，跨全场景找第一个定义处；
 * - scenarioFlow 面向「相邻连线」：按**边**聚合，回答「这条线上流过什么」，并且
 *   区分配置产物（extract）与外部变量（环境等），后者不是链路本身的数据流，
 *   但显示出来有助于理解「这一步吃了什么」。
 */

import { extractVarRefs } from '@/utils/scenarioVars'

/** 单步输入（宿主从编辑态物化）。 */
export interface FlowStep {
  index: number
  enabled: boolean
  name: string
  /** 该步产生的变量（extract 规则名）。 */
  produces: string[]
  /** 该步消费的变量名（去重、保序）。 */
  consumes: string[]
  /** 循环标记：步骤卡上画个回转箭头。 */
  looping: boolean
  /** 并行标记：相邻多步成批，连线画虚线。 */
  parallel: boolean
}

/** 相邻两步之间的连线。 */
export interface FlowEdge {
  /** 起点步骤下标（-1 = 链路入口，画在第一步上方）。 */
  from: number
  /** 终点步骤下标。 */
  to: number
  /** 在这条线上传递的变量名：前序某步定义、且被终点步骤消费。 */
  vars: string[]
  /** 终点步骤消费的外部变量（环境 / 项目 / 全局 / 内置）。 */
  externalVars: string[]
  /** 起点步骤被禁用（连线灰显）。 */
  muted: boolean
  /** 批内并行（虚线）。 */
  parallel: boolean
}

export interface FlowGraph {
  /** 参与绘制的边（含入口边 from = -1）。 */
  edges: FlowEdge[]
  /** 变量名 → 定义它的步骤下标（仅启用步骤；同名取最早）。 */
  definedBy: Map<string, number>
}

/**
 * 计算工作流连线。
 *
 * @param steps 全部步骤（含禁用，禁用步骤也要画出来，只是连线灰显）
 * @param externalNames 外部变量池键名
 */
export function computeFlowGraph(steps: FlowStep[], externalNames: Iterable<string>): FlowGraph {
  const external = new Set(externalNames)
  const definedBy = new Map<string, number>()

  // 先扫一遍产出（只看启用步骤：禁用的步骤不会真的产生值）。
  for (const s of steps) {
    if (!s.enabled) continue
    for (const raw of s.produces) {
      const name = raw.trim()
      if (!name) continue
      if (!definedBy.has(name)) definedBy.set(name, s.index)
    }
  }

  const edges: FlowEdge[] = []
  const enabled = steps.filter((s) => s.enabled)
  if (!enabled.length) return { edges, definedBy }

  for (let i = 0; i < enabled.length; i += 1) {
    const cur = enabled[i]!
    const prev = i > 0 ? enabled[i - 1]! : null
    const vars: string[] = []
    const externalVars: string[] = []
    for (const raw of cur.consumes) {
      const name = raw.trim()
      if (!name) continue
      const at = definedBy.get(name)
      // 前序步骤（严格更早）定义 → 属于链路数据流；否则是外部变量。
      if (at != null && at < cur.index) {
        if (!vars.includes(name)) vars.push(name)
      } else if (external.has(name)) {
        if (!externalVars.includes(name)) externalVars.push(name)
      }
    }
    edges.push({
      from: prev ? prev.index : -1,
      to: cur.index,
      vars,
      externalVars,
      muted: prev ? !prev.enabled : false,
      parallel: !!prev?.parallel && !!cur.parallel,
    })
  }
  return { edges, definedBy }
}

/** 从 edit 态物化成流图输入：produces / consumes 来自真实配置，不写死示例。 */
export function toFlowSteps(
  steps: {
    index: number
    enabled: boolean
    name: string
    extracts: { name: string }[]
    referenceTexts: string[]
    conditionName?: string | null
    loopWhileName?: string | null
    looping: boolean
    parallel: boolean
  }[],
): FlowStep[] {
  return steps.map((s) => {
    const consumes: string[] = []
    const add = (n: string | null | undefined) => {
      const name = (n ?? '').trim()
      if (name && !consumes.includes(name)) consumes.push(name)
    }
    for (const text of s.referenceTexts) {
      if (!text) continue
      for (const name of extractVarRefs(text)) add(name)
    }
    add(s.conditionName)
    add(s.loopWhileName)
    return {
      index: s.index,
      enabled: s.enabled,
      name: s.name,
      produces: s.extracts.map((e) => e.name.trim()).filter(Boolean),
      consumes,
      looping: s.looping,
      parallel: s.parallel,
    }
  })
}

/**
 * 计算某步起真正需要的外部输入：该步（含）之后消费、但不是更早启用步骤提取的变量。
 *
 * 这是「无运行时快照的单步 / 从步骤执行」的守卫：漏掉一个变量，后端就会拿到
 * `{{var}}` 未替换的字面量或空值，甚至对写操作产生非预期请求。因此这里返回的
 * 名单必须在运行前全部由用户填齐。
 *
 * @param flowSteps 按执行顺序的流图输入
 * @param definedBy 变量 → 定义它的步骤下标（见 computeFlowGraph）
 * @param externalNames 外部变量池（环境 / 项目 / 全局 / 内置），不需要手填
 */
export function requiredVarsFrom(
  flowSteps: FlowStep[],
  definedBy: Map<string, number>,
  externalNames: Iterable<string>,
  fromIdx: number,
): string[] {
  const external = new Set(externalNames)
  const out: string[] = []
  for (const s of flowSteps) {
    if (s.index < fromIdx || !s.enabled) continue
    for (const name of s.consumes) {
      if (out.includes(name)) continue
      if (external.has(name)) continue
      const producer = definedBy.get(name)
      // 生产者在起点之前（或根本不存在）→ 快照缺失时取不到值，必须手填；
      // 生产者在起点之后 → 链路自己会补齐，无需手填。
      if (producer == null || producer < fromIdx) out.push(name)
    }
  }
  return out
}

/**
 * 流水线视角的批次划分：连续 `parallel` 步骤合成一批（单步自成一档）。
 * 编辑器据此在步骤卡之间画「批内虚线、批间实线」，也让并行语义在视觉上成立。
 */
export function groupBatches(steps: FlowStep[]): FlowStep[][] {
  const batches: FlowStep[][] = []
  let run: FlowStep[] = []
  for (const s of steps) {
    if (s.parallel) {
      run.push(s)
      continue
    }
    if (run.length) {
      batches.push(run)
      run = []
    }
    batches.push([s])
  }
  if (run.length) batches.push(run)
  return batches
}