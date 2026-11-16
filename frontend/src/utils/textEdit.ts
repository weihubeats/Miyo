/**
 * textEdit：把一段文本整体落到 textarea 上（粘贴即美化的落库动作）。
 *
 * 两条路径：
 * - 优先 `execCommand('insertText')`：浏览器把这次替换记进**原生撤销栈**，
 *   用户粘错了按 Ctrl/⌘+Z 能整块退回（实证：程序化写 `el.value` 会清空撤销栈）；
 * - 内核不支持（jsdom / 老内核 / 未聚焦）时退回直接改 DOM，并 dispatch `input`
 *   事件让宿主的 v-model / 防抖镜像照常收到新值。
 *
 * 与地址栏 EndpointEditor 的 `replaceByExecCommand` 同源思路，但这里只服务代码区
 * （整篇替换 + 落光标），不复用那份带选区插入的实现。
 */
export function replaceTextareaValue(ta: HTMLTextAreaElement, text: string, caret: number): void {
  ta.setSelectionRange(0, ta.value.length)
  let inserted = false
  try {
    inserted = document.execCommand?.('insertText', false, text) === true && ta.value === text
  } catch {
    inserted = false
  }
  if (!inserted) {
    ta.value = text
    ta.dispatchEvent(new Event('input', { bubbles: true }))
  }
  ta.setSelectionRange(caret, caret)
}
