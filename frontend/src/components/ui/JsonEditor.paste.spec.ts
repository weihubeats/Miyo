/**
 * JsonEditor 粘贴即美化单测：paste 事件带压缩 JSON → 编辑器内文本被美化并外报
 * update:modelValue；非 JSON / 已美化内容不被拦截（原生粘贴照旧）。
 * jsdom 无 execCommand，走「直接改值 + 主动外报」兜底分支（真实内核优先走
 * execCommand 以保原生撤销栈）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JsonEditor from '@/components/ui/JsonEditor.vue'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

const PRETTY = '{\n  "a": 1\n}'

function mountEditor(modelValue = '') {
  return mount(JsonEditor, {
    props: { modelValue, showStatus: false, formatOnPaste: true },
    attachTo: document.body,
  })
}

/** jsdom 没有 ClipboardEvent 的 clipboardData，用普通事件补一个数据源。 */
function paste(w: ReturnType<typeof mountEditor>, text: string) {
  const ta = w.find('textarea')
  const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
  Object.defineProperty(ev, 'clipboardData', { value: { getData: () => text } })
  ta.element.dispatchEvent(ev)
  return { ev, ta }
}

describe('JsonEditor 粘贴即美化', () => {
  it('空编辑器粘贴压缩 JSON：文本被美化并外报', () => {
    const w = mountEditor()
    const { ev } = paste(w, '{"a":1}')
    expect(ev.defaultPrevented).toBe(true)
    expect(w.find('textarea').element.value).toBe(PRETTY)
    expect(w.emitted('update:modelValue')?.at(-1)).toEqual([PRETTY])
    w.unmount()
  })

  it('粘贴非 JSON 文本：不拦截', () => {
    const w = mountEditor()
    const { ev } = paste(w, 'hello world')
    expect(ev.defaultPrevented).toBe(false)
    expect(w.emitted('update:modelValue')).toBeUndefined()
    w.unmount()
  })

  it('粘贴已美化 JSON：不拦截（保留原生粘贴行为）', () => {
    const w = mountEditor()
    const { ev } = paste(w, PRETTY)
    expect(ev.defaultPrevented).toBe(false)
    w.unmount()
  })

  it('formatOnPaste=false：完全不插手', () => {
    const w = mount(JsonEditor, {
      props: { modelValue: '', showStatus: false, formatOnPaste: false },
      attachTo: document.body,
    })
    const { ev } = paste(w, '{"a":1}')
    expect(ev.defaultPrevented).toBe(false)
    w.unmount()
  })
})
