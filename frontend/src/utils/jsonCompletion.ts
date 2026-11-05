/**
 * jsonCompletion：CodeMirror JSON 上下文补全源（utils 层纯函数 + CM 适配）。
 *
 * 设计取舍：
 * - 键候选来自**当前文档**里出现过的键（按出现顺序去重），不凭空臆造字段——
 *   调试工具的 JSON 多是「照着已有结构改」，同文键是最有用的候选集；
 * - 值候选固定 true / false / null；
 * - 位置判定用光标前文本的轻量扫描（引号态 + 括号栈 + 开引号前结构字符），
 *   不依赖 Lezer 语法树——半成品 JSON（键后没冒号等）在语法树上不可靠。
 */
import type { CompletionContext, CompletionResult } from '@codemirror/autocomplete'

/** 补全位置语义。 */
export type JsonCompletionSlot = 'key' | 'value' | 'none'

interface ScanResult {
  slot: JsonCompletionSlot
  /** 在字符串内时：开启引号在行文中的偏移（补全替换起点 = 引号后一位）。 */
  stringStart: number
}

/**
 * 扫描光标前文本，判定补全位置。
 * - 字符串内：看开启引号前最近的非空白字符——`{`/`,` → 键位；`:` → 值位；
 *   数组内（栈顶 `[`）一律值位；
 * - 字符串外：最近结构字符 `:` 或栈顶 `[`/数组逗号 → 值位；栈顶 `{` 且最近是
 *   `{`/`,` → 键位；其余（冒号后已输入完整值等）不弹。
 */
export function scanJsonCompletionSlot(before: string): ScanResult {
  const stack: string[] = []
  let inString = false
  let escaped = false
  let stringStart = -1
  let lastMeaningful = ''
  for (let i = 0; i < before.length; i++) {
    const ch = before[i]
    if (inString) {
      if (escaped) {
        escaped = false
        continue
      }
      if (ch === '\\') {
        escaped = true
        continue
      }
      if (ch === '"') {
        inString = false
        lastMeaningful = '"'
      }
      continue
    }
    if (ch === '"') {
      inString = true
      stringStart = i
      continue
    }
    if (ch === '{' || ch === '[') {
      stack.push(ch)
      lastMeaningful = ch
      continue
    }
    if (ch === '}' || ch === ']') {
      stack.pop()
      lastMeaningful = ch
      continue
    }
    if (!/\s/.test(ch)) lastMeaningful = ch
  }

  const top = stack[stack.length - 1]
  if (inString) {
    // 数组里的字符串元素永远是值；对象内看开引号前的结构字符
    if (top === '[') return { slot: 'value', stringStart }
    if (lastMeaningful === ':') return { slot: 'value', stringStart }
    if (lastMeaningful === '{' || lastMeaningful === ',' || lastMeaningful === '"') {
      return { slot: 'key', stringStart }
    }
    return { slot: 'none', stringStart }
  }
  if (top === '[') return { slot: 'value', stringStart: -1 }
  if (lastMeaningful === ':') return { slot: 'value', stringStart: -1 }
  if (top === '{' && (lastMeaningful === '{' || lastMeaningful === ',')) {
    return { slot: 'key', stringStart: -1 }
  }
  return { slot: 'none', stringStart: -1 }
}

/** 收集文档中出现过对象键（`"key":` 形式），按出现顺序去重。 */
export function collectJsonKeys(doc: string): string[] {
  const out: string[] = []
  const seen = new Set<string>()
  const re = /"((?:[^"\\]|\\.)*)"\s*:/g
  let m: RegExpExecArray | null
  while ((m = re.exec(doc)) !== null) {
    const key = m[1] ?? ''
    if (!seen.has(key)) {
      seen.add(key)
      out.push(key)
    }
  }
  return out
}

/** 值位固定候选。 */
const VALUE_OPTIONS = ['true', 'false', 'null']

/** CodeMirror 补全源：键位建议同文键、值位建议字面量，其余不弹。 */
export function jsonCompletionSource(ctx: CompletionContext): CompletionResult | null {
  const scan = scanJsonCompletionSlot(ctx.state.sliceDoc(0, ctx.pos))

  if (scan.slot === 'key') {
    const keys = collectJsonKeys(ctx.state.doc.toString())
    if (!keys.length) return null
    if (scan.stringStart >= 0) {
      // 引号内补全：替换引号后的已输入前缀（收尾引号由 closeBrackets 已备好）
      return {
        from: scan.stringStart + 1,
        options: keys.map((k) => ({ label: k, type: 'property' })),
      }
    }
    // 引号外（`{` / `,` 后空白）：整键插入 `"key": `（Postman/Apifox 习惯）
    return {
      from: ctx.pos,
      options: keys.map((k) => ({
        label: k,
        type: 'property',
        apply: `"${k}": `,
      })),
    }
  }

  if (scan.slot === 'value') {
    // 引号内是字符串值位：不弹字面量（避免污染字符串编辑）
    if (scan.stringStart >= 0) return null
    return {
      from: ctx.pos,
      options: VALUE_OPTIONS.map((v) => ({ label: v, type: 'keyword' })),
    }
  }
  return null
}
