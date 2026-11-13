/**
 * textareaIndent 单测：Tab 缩进 / Shift+Tab 反缩进 / input 事件派发。
 */
import { describe, expect, it, vi } from 'vitest'
import { handleTextareaTab } from '@/utils/textareaIndent'

function ta(value: string, start: number, end: number): HTMLTextAreaElement {
  const el = document.createElement('textarea')
  el.value = value
  el.selectionStart = start
  el.selectionEnd = end
  document.body.appendChild(el)
  return el
}

function tabKey(shift = false): KeyboardEvent {
  return new KeyboardEvent('keydown', { key: 'Tab', shiftKey: shift, bubbles: true, cancelable: true })
}

describe('handleTextareaTab', () => {
  it('非 Tab 键直接返回（不 preventDefault、不改值）', () => {
    const el = ta('ab', 1, 1)
    const e = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    handleTextareaTab(el, e)
    expect(e.defaultPrevented).toBe(false)
    expect(el.value).toBe('ab')
    el.remove()
  })

  it('单点 Tab：在光标处插入 2 空格并派发 input', () => {
    const el = ta('{\n}', 2, 2)
    const onInput = vi.fn()
    el.addEventListener('input', onInput)
    handleTextareaTab(el, tabKey())
    expect(el.value).toBe('{\n  }')
    expect(el.selectionStart).toBe(4)
    expect(onInput).toHaveBeenCalledTimes(1)
    el.remove()
  })

  it('多行选区 Tab：整块缩进，选区同步后移', () => {
    const el = ta('a\nb\nc', 0, 5)
    handleTextareaTab(el, tabKey())
    expect(el.value).toBe('  a\n  b\n  c')
    expect(el.selectionStart).toBe(2)
    expect(el.selectionEnd).toBe(5 + 2 * 3)
    el.remove()
  })

  it('Shift+Tab：整块反缩进（不足 2 空格去空为止）', () => {
    const el = ta('  a\n b\nc', 0, 8)
    handleTextareaTab(el, tabKey(true))
    expect(el.value).toBe('a\nb\nc')
    expect(el.selectionStart).toBe(0)
    expect(el.selectionEnd).toBe(5)
    el.remove()
  })

  it('Shift+Tab 单行：反缩进当前行', () => {
    const el = ta('{\n  x\n}', 4, 4)
    handleTextareaTab(el, tabKey(true))
    expect(el.value).toBe('{\nx\n}')
    expect(el.selectionStart).toBe(2)
    el.remove()
  })
})
