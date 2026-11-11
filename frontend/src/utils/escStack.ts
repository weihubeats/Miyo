/**
 * ESC 层级仲裁栈：弹层（Modal / Drawer / Menu / Popconfirm…）打开时压入
 * 关闭处理器，关闭时弹出；一次 ESC 只关闭最顶层。
 *
 * 解决的问题：此前 Modal / Drawer 在 document 上无条件响应 ESC，Menu /
 * Popconfirm / CustomSelect 又各自处理 ESC——嵌套弹层（Modal 里开下拉 /
 * 菜单 / 确认气泡）按一次 ESC 会把子弹层和 Modal 一起关掉。同节点
 * document 监听器无法用 stopPropagation 切断（注册顺序不可控），改为
 * 显式的「后开先关」栈仲裁。
 *
 * 用法：open 状态变化时调用 pushEscHandler / 返回的卸载函数：
 *   watch(open, (v) => { off?.(); off = v ? pushEscHandler(close) : null })
 * 处理器返回 false 表示「本轮不响应」（如 findbar 打开时抽屉放行）。
 *
 * 监听同时挂 window 与 document：既有测试 / 部分组件在 window 上派发
 * KeyboardEvent（该事件不经过 document），真实按键与 document 派发则两者
 * 都会触发——用 WeakSet 按事件对象去重。
 */

export type EscHandler = () => boolean | void

interface EscEntry {
  id: number
  handler: EscHandler
}

const stack: EscEntry[] = []
/** 已处理过的事件（window / document 双监听去重）。 */
const seen = new WeakSet<KeyboardEvent>()
let seq = 0
let bound = false

function dispatch(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || seen.has(event)) return
  seen.add(event)
  // 顶层（后压入）先响应；返回 false 表示放行给更低层。
  for (let i = stack.length - 1; i >= 0; i--) {
    event.preventDefault()
    const consumed = stack[i].handler()
    if (consumed !== false) break
  }
}

function bind(): void {
  if (bound) return
  bound = true
  document.addEventListener('keydown', dispatch)
  window.addEventListener('keydown', dispatch)
}

function unbind(): void {
  if (!bound || stack.length) return
  bound = false
  document.removeEventListener('keydown', dispatch)
  window.removeEventListener('keydown', dispatch)
}

/**
 * 压入一个 ESC 处理器（后开先关）。返回卸载函数；组件卸载时必须调用
 * （onBeforeUnmount），否则处理器泄漏会让更低层收不到 ESC。
 */
export function pushEscHandler(handler: EscHandler): () => void {
  const id = ++seq
  stack.push({ id, handler })
  bind()
  return () => {
    const idx = stack.findIndex((x) => x.id === id)
    if (idx !== -1) stack.splice(idx, 1)
    unbind()
  }
}
