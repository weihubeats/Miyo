/**
 * 地址栏文本编辑工具（自 EndpointEditor 抽出的纯函数，直接单测）。
 *
 * 这些函数只依赖 DOM 输入参数，不持有组件状态；粘贴分流的**编排**
 * （cURL 拦截 / 分段取词 / 原生插入）见 EndpointEditor 的 onUrlPaste。
 */

/**
 * 从一段含空白碎片的文本里取「像请求路径」的那一段。
 *
 * 取词规则：
 * 1. 首段是「只剩 origin 的完整地址」（前缀里带了不可见空白被截断，`http://h:1 /routes`）→
 *    与后面的路径段拼回一条完整 URL，别把主机丢掉；
 * 2. 否则取含 `/` 的那一段（路径必含 `/`，标签不会）；
 * 3. 都没有 `/` 时取首段（`routes routes` → `routes`）。
 */
export function pickPathToken(text: string): string {
  const tokens = text
    .replace(/[\r\n\t]+/g, ' ')
    .split(/\s+/)
    .filter(Boolean)
  const head = tokens[0] ?? ''
  if (/^(?:https?|wss?):\/\/[^/]+$/i.test(head)) {
    const path = tokens.slice(1).find((token) => token.startsWith('/'))
    if (path) return `${head}${path}`
  }
  return tokens.find((token) => token.includes('/')) ?? head
}

/**
 * 光标处替换当前选区插入：与敲键同一类原生编辑事务，进撤销栈（Cmd+Z 可回退）。
 * 关键：WebKit 的 smart paste 空格只在**原生粘贴**路径生成，insertText 不受影响。
 */
export function insertAtByExecCommand(
  el: HTMLInputElement,
  start: number,
  end: number,
  text: string,
): boolean {
  try {
    el.setSelectionRange(start, end)
    return document.execCommand('insertText', false, text)
  } catch {
    return false
  }
}

/**
 * 整条替换：选中全字段后走原生插入 —— 与直接敲键同一类编辑事务，
 * 替换本身也进撤销栈（Cmd+Z 能回到替换前的内容）。
 */
export function replaceByExecCommand(el: HTMLInputElement, text: string): boolean {
  try {
    el.setSelectionRange(0, el.value.length)
    return document.execCommand('insertText', false, text)
  } catch {
    return false
  }
}

/** 兜底程序化写值（execCommand 不可用的环境）；程序化写值不进撤销栈，仅保证结果正确。 */
export function setFieldValueAt(el: HTMLInputElement, text: string, caret: number): void {
  el.value = text
  try {
    el.setSelectionRange(caret, caret)
  } catch {
    /* 非文本输入（类型限制）：跳过光标定位 */
  }
  el.dispatchEvent(new Event('input', { bubbles: true }))
}

/** 兜底整条替换 = 写值 + 光标到末尾。 */
export function setFieldValue(el: HTMLInputElement, text: string): void {
  setFieldValueAt(el, text, text.length)
}

/** 归一化 shell 续行 + 去终端提示符，与 CurlImportDialog 保持一致。 */
export function normalizeCurlCommand(raw: string): string {
  return raw
    .replace(/^\s*[$#>]+[ \t]+/, '')
    .replace(/\\[ \t]*\r?\n/g, ' ')
    .trim()
}
