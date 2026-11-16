/**
 * pasteFormat：粘贴内容自动美化的共用引擎（JSON / XML / HTML 三种排版器共用一套规则）。
 *
 * 规则（宁可不动手，也不做让用户困惑的改写）：
 * 1. 只在「整篇都是这次粘贴的内容」（空编辑器 / 全选粘贴）时排版整篇；
 *    文档已有内容时只排版**粘贴块自己**，绝不重排用户已有的内容；
 * 2. 粘贴块必须自己就能独立解析（不像该格式的一律放行原生粘贴）；
 * 3. 排版结果与原文相同（本来就是美化过的）时不拦截，保留浏览器原生粘贴的
 *    撤销栈与光标行为；
 * 4. 超过阈值不排版：排版是同步全量解析，大文档粘贴会卡住输入。
 *
 * 纯函数：不碰 DOM，编辑器负责把 `PasteEdit` 落到 textarea（utils/textEdit）。
 */

/** 超过此长度的文档不做粘贴排版：与 JsonEditor 的大文档降级阈值同量级。 */
export const PASTE_FORMAT_MAX_CHARS = 200_000

/** 一次「粘贴即美化」的落库结果。 */
export interface PasteEdit {
  /** 排版后的整篇文档文本。 */
  text: string
  /** 落库后光标应停留的位置（排版后文本内的偏移）。 */
  caret: number
}

export interface PasteFormatOptions {
  /** 排版器：抛错或返回 null 都视为「这段内容不该被排版」。 */
  format: (src: string) => string | null
  /** 快速豁免：不像该格式的文本直接放行（避免无谓解析）。 */
  looksLike: (src: string) => boolean
}

/**
 * 计算「粘贴即美化」的编辑结果。
 *
 * @param doc    编辑器当前全文（textarea 实时值，不用可能滞后的 modelValue）
 * @param start  选区起点
 * @param end    选区终点
 * @param pasted 剪贴板文本
 * @returns 需要落到文档上的完整文本与光标位置；`null` = 不拦截，走原生粘贴
 */
export function autoFormatPasted(
  doc: string,
  start: number,
  end: number,
  pasted: string,
  opts: PasteFormatOptions,
): PasteEdit | null {
  if (!pasted.trim()) return null
  if (doc.length + pasted.length > PASTE_FORMAT_MAX_CHARS) return null
  if (!opts.looksLike(pasted)) return null

  const { from, before, after } = splitDoc(doc, start, end)

  // 全选 / 空文档：整篇就是这次粘贴的内容，直接排版整篇
  if (!before.trim() && !after.trim()) {
    const candidate = before + pasted + after
    const formatted = safeFormat(opts, candidate)
    if (!formatted || formatted === candidate) return null
    return { text: formatted, caret: formatted.length }
  }

  // 文档已有内容：只美化粘贴块本身，已有的内容一个字符都不动（光标可精确落在粘贴块末尾）
  const formattedPaste = safeFormat(opts, pasted)
  if (!formattedPaste || formattedPaste === pasted) return null
  return { text: before + formattedPaste + after, caret: from + formattedPaste.length }
}

/**
 * 把「已经排版好的外部文本」（如从 cURL 命令里抽出的 JSON body）按同一套位置规则落地：
 * 空文档 / 全选 → 替换整篇；否则插到光标处（尊重选区）。
 *
 * 与 autoFormatPasted 的差别只有一点：文本已由调用方排版，不需要再判「是否变短 / 变样」，
 * 所以没有返回 null 的分支。
 */
export function placeFormattedText(doc: string, start: number, end: number, text: string): PasteEdit {
  const { from, before, after } = splitDoc(doc, start, end)
  if (!before.trim() && !after.trim()) return { text, caret: text.length }
  return { text: before + text + after, caret: from + text.length }
}

/** 选区归一化：越界 / 反向选区都夹进 [0, doc.length]，from ≤ to 恒成立。 */
function splitDoc(
  doc: string,
  start: number,
  end: number,
): { from: number; before: string; after: string } {
  const from = Math.max(0, Math.min(start, end, doc.length))
  const to = Math.max(0, Math.min(Math.max(start, end), doc.length))
  return { from, before: doc.slice(0, from), after: doc.slice(to) }
}

function safeFormat(opts: PasteFormatOptions, src: string): string | null {
  try {
    return opts.format(src)
  } catch {
    return null
  }
}
