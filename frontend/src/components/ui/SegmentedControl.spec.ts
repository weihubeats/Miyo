/**
 * SegmentedControl 单测：键盘导航（ARIA tablist + roving tabindex + ←/→ Home/End）。
 * 与 Tabs 同款自动激活模式；整组 disabled 时不拦截方向键。
 */
import { describe, expect, it, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import type { SegmentOption } from '@/components/ui/SegmentedControl.vue'

const OPTIONS: SegmentOption[] = [
  { value: 'form', label: 'form-data' },
  { value: 'raw', label: 'raw' },
  { value: 'json', label: 'JSON', panelId: 'json-panel' },
]

function mountSeg(modelValue = 'raw', options: SegmentOption[] = OPTIONS, disabled = false) {
  return mount(SegmentedControl, {
    props: { modelValue, options, disabled },
    attachTo: document.body,
  })
}

function key(wrapper: ReturnType<typeof mountSeg>, k: string): KeyboardEvent {
  const active = wrapper.find('[role="tab"][tabindex="0"]')
  expect(active.exists(), '应有可聚焦的激活项').toBe(true)
  ;(active.element as HTMLElement).focus()
  const ev = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  wrapper.element.dispatchEvent(ev)
  return ev
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('SegmentedControl：ARIA 与键盘', () => {
  it('role=tablist/tab、aria-selected、roving tabindex、aria-controls 仅在给了 panelId 时出现', () => {
    const wrapper = mountSeg()
    expect(wrapper.find('[role="tablist"]').exists()).toBe(true)
    const items = wrapper.findAll('[role="tab"]')
    expect(items.map((i) => i.attributes('aria-selected'))).toEqual(['false', 'true', 'false'])
    expect(items.map((i) => i.attributes('tabindex'))).toEqual(['-1', '0', '-1'])
    expect(items[0].attributes('aria-controls')).toBeUndefined()
    expect(items[2].attributes('aria-controls')).toBe('json-panel')
    wrapper.unmount()
  })

  it('ArrowRight 激活下一项，Home 跳回首项', async () => {
    const wrapper = mountSeg()
    key(wrapper, 'ArrowRight')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['json'])
    await wrapper.setProps({ modelValue: 'json' })
    expect(document.activeElement?.getAttribute('data-value')).toBe('json')

    key(wrapper, 'Home')
    expect(wrapper.emitted('update:modelValue')?.[1]).toEqual(['form'])
    wrapper.unmount()
  })

  it('整组 disabled 时不拦截方向键（保持原生）', () => {
    const wrapper = mountSeg('raw', OPTIONS, true)
    const ev = key(wrapper, 'ArrowRight')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect(ev.defaultPrevented).toBe(false)
    wrapper.unmount()
  })
})
