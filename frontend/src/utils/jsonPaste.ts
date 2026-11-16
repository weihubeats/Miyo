/**
 * jsonPaste：粘贴时自动美化 JSON（请求体 raw(JSON)、GraphQL variables、gRPC 消息共用）。
 *
 * 场景：用户从文档 / 聊天窗口复制一段压缩 JSON（`{"a":1,"b":[1,2]}`）粘进
 * 编辑器，落地的是一整行——既难读，也难在中间改字段。这里在 paste 事件里先
 * 排版再落库，产出与工具栏「美化」完全一致（同一 `prettyJson`，保重复键 / 键序 /
 * 数字原文，见 utils/jsonFormat）。
 *
 * 位置规则（整篇替换 / 只动粘贴块 / 已美化放行）见 utils/pasteFormat 的共用引擎，
 * 本模块只提供 JSON 专属的两件东西：排版器与「像不像 JSON 值」的快速判定。
 */
import { prettyJson } from '@/utils/jsonFormat'
import { PASTE_FORMAT_MAX_CHARS, autoFormatPasted } from '@/utils/pasteFormat'
import type { PasteEdit } from '@/utils/pasteFormat'

export { PASTE_FORMAT_MAX_CHARS }
export type { PasteEdit }

/**
 * 计算「粘贴 JSON 即美化」的编辑结果。
 *
 * @param doc    编辑器当前全文（textarea 实时值，不用可能滞后的 modelValue）
 * @param start  选区起点
 * @param end    选区终点
 * @param pasted 剪贴板文本
 * @returns 需要落到文档上的完整文本与光标位置；`null` = 不拦截，走原生粘贴
 */
export function autoFormatJsonPaste(
  doc: string,
  start: number,
  end: number,
  pasted: string,
): PasteEdit | null {
  return autoFormatPasted(doc, start, end, pasted, {
    format: tryPretty,
    looksLike: looksLikeJsonValue,
  })
}

/** 粘贴块是否以 `{` / `[` 开头（去掉前导空白后），即 JSON 值而非任意文本。 */
export function looksLikeJsonValue(src: string): boolean {
  const head = src.trimStart()[0]
  return head === '{' || head === '['
}

/** 美化失败（语法错误等）时返回 null，交给原生粘贴。 */
export function tryPretty(src: string): string | null {
  try {
    return prettyJson(src)
  } catch {
    return null
  }
}
