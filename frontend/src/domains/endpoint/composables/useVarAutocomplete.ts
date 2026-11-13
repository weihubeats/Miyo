/**
 * useVarAutocomplete：`{{变量}}` 输入自动补全（地址栏 / KV 表 value 共用）。
 *
 * 触发：光标紧贴 `{{` 之后（含刚输入 `{{`）→ 弹候选；输入 `}` 或光标移开即收起。
 * 键盘：↑↓ 循环选择、Enter / Tab 插入（并阻止默认的发送 / 跳格）、Esc 关闭
 * （stopPropagation，避免上层「Esc 清空地址栏」等行为误触发）、其余键继续过滤。
 * 插入写回 `{{name}}` 并把光标定位到 `}}` 之后，再派发 input 让 v-model 同步。
 *
 * 弹层由 ui/VarSuggest.vue 渲染（Teleport 到 body + fixed 定位，
 * 不受 config-box / kvt 的 overflow:hidden 裁剪）。
 */
import { onBeforeUnmount, ref, watch, type Ref } from 'vue'

/** 光标前紧邻的 `{{` + 部分变量名（不越过 `}`，保证只提示未成对的引用）。 */
const TRIGGER = /\{\{([A-Za-z0-9_$-]*)$/

/** 单屏最多展示的候选项（避免长变量表撑爆弹层）。 */
const MAX_ITEMS = 50

export interface VarAutocomplete {
  /** 弹层是否展开。 */
  open: Ref<boolean>
  /** 过滤后的候选（已截断）。 */
  items: Ref<string[]>
  /** 当前高亮下标。 */
  activeIndex: Ref<number>
  /** 定位锚点（触发的 input 元素）。 */
  anchor: Ref<HTMLElement | null>
  /** 输入 / 点击后重新判定是否展开。 */
  onInput: (el: HTMLInputElement) => void
  /** 键盘处理：消费该事件时返回 true（调用方需直接 return）。 */
  handleKeydown: (event: KeyboardEvent, el: HTMLInputElement) => boolean
  /** 插入选中项（index 缺省取当前高亮）。 */
  pick: (el: HTMLInputElement, index?: number) => void
  close: () => void
}

export function useVarAutocomplete(candidates: Ref<string[]>): VarAutocomplete {
  const open = ref(false)
  const items = ref<string[]>([])
  const activeIndex = ref(0)
  const anchor = ref<HTMLElement | null>(null)

  function close(): void {
    open.value = false
  }

  function onInput(el: HTMLInputElement): void {
    const caret = el.selectionStart ?? el.value.length
    const m = el.value.slice(0, caret).match(TRIGGER)
    if (!m) {
      close()
      return
    }
    const query = m[1].toLowerCase()
    const matched = query
      ? candidates.value.filter((c) => c.toLowerCase().includes(query))
      : candidates.value
    items.value = matched.slice(0, MAX_ITEMS)
    activeIndex.value = 0
    anchor.value = el
    open.value = items.value.length > 0
  }

  function pick(el: HTMLInputElement, index: number = activeIndex.value): void {
    const item = items.value[index]
    const caret = el.selectionStart ?? el.value.length
    const before = el.value.slice(0, caret)
    const m = before.match(TRIGGER)
    if (item == null || !m) {
      close()
      return
    }
    const start = m.index ?? before.length
    el.value = `${before.slice(0, start)}{{${item}}}${el.value.slice(caret)}`
    try {
      const pos = start + item.length + 4
      el.setSelectionRange(pos, pos)
    } catch {
      // 非文本输入（类型限制）：跳过光标定位
    }
    close()
    // 让 v-model 同步（原生 input 事件冒泡，组件 v-model 同样可捕获）
    el.dispatchEvent(new Event('input', { bubbles: true }))
    el.focus()
  }

  function handleKeydown(event: KeyboardEvent, el: HTMLInputElement): boolean {
    if (!open.value) return false
    const total = items.value.length
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      activeIndex.value = (activeIndex.value + 1) % total
      return true
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      activeIndex.value = (activeIndex.value - 1 + total) % total
      return true
    }
    if (event.key === 'Enter' || event.key === 'Tab') {
      event.preventDefault()
      event.stopPropagation()
      pick(el)
      return true
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      close()
      return true
    }
    return false
  }

  // 弹层为 fixed 定位：滚动 / 缩放时锚点失效，直接收起（与 Menu 一致）。
  function onDocScroll(): void {
    close()
  }
  function onWindowResize(): void {
    close()
  }
  watch(open, (isOpen) => {
    if (isOpen) {
      document.addEventListener('scroll', onDocScroll, true)
      window.addEventListener('resize', onWindowResize)
    } else {
      document.removeEventListener('scroll', onDocScroll, true)
      window.removeEventListener('resize', onWindowResize)
    }
  })
  onBeforeUnmount(() => {
    document.removeEventListener('scroll', onDocScroll, true)
    window.removeEventListener('resize', onWindowResize)
  })

  // 关闭即丢弃候选，避免下次展开闪现旧列表。
  watch(open, (isOpen) => {
    if (!isOpen) items.value = []
  })

  return { open, items, activeIndex, anchor, onInput, handleKeydown, pick, close }
}
