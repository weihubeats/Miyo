/**
 * Tabs 单测：键盘导航（ARIA tablist + roving tabindex + ←/→ Home/End）。
 * 自动激活模式：方向键切到哪项就激活哪项（与点击等价）。
 */
import { describe, expect, it, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import Tabs from '@/components/ui/Tabs.vue'
import type { TabItem } from '@/components/ui/Tabs.vue'

const TABS: TabItem[] = [
  { key: 'request', label: '请求' },
  { key: 'params', label: '参数' },
  { key: 'docs', label: '文档', panelId: 'docs-panel' },
]

function mountTabs(modelValue = 'request', tabs: TabItem[] = TABS) {
  return mount(Tabs, { props: { modelValue, tabs }, attachTo: document.body })
}

/** 模拟用户 Tab 进入页签条：先把焦点放在当前激活项上，再发键盘事件。 */
function focusActive(wrapper: ReturnType<typeof mountTabs>): void {
  const active = wrapper.find('[role="tab"][tabindex="0"]')
  expect(active.exists(), '应有可聚焦的激活页签').toBe(true)
  ;(active.element as HTMLElement).focus()
}

function key(wrapper: ReturnType<typeof mountTabs>, k: string): KeyboardEvent {
  focusActive(wrapper)
  const ev = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
  wrapper.element.dispatchEvent(ev)
  return ev
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('Tabs：ARIA 语义', () => {
  it('容器 role=tablist，页签 role=tab + aria-selected', () => {
    const wrapper = mountTabs()
    expect(wrapper.find('[role="tablist"]').exists()).toBe(true)
    const items = wrapper.findAll('[role="tab"]')
    expect(items).toHaveLength(3)
    expect(items[0].attributes('aria-selected')).toBe('true')
    expect(items[1].attributes('aria-selected')).toBe('false')
    wrapper.unmount()
  })

  it('roving tabindex：仅激活项可 Tab 进入', () => {
    const wrapper = mountTabs('params')
    const items = wrapper.findAll('[role="tab"]')
    expect(items.map((i) => i.attributes('tabindex'))).toEqual(['-1', '0', '-1'])
    wrapper.unmount()
  })

  it('传 panelId 才渲染 aria-controls，未传不产生空属性', () => {
    const wrapper = mountTabs()
    const items = wrapper.findAll('[role="tab"]')
    expect(items[0].attributes('aria-controls')).toBeUndefined()
    expect(items[2].attributes('aria-controls')).toBe('docs-panel')
    wrapper.unmount()
  })

  it('disabled 页签跳过方向键导航', async () => {
    const wrapper = mountTabs('request', [
      ...TABS.slice(0, 2),
      { key: 'off', label: '禁用', disabled: true },
    ])
    key(wrapper, 'ArrowRight')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['params'])
    wrapper.unmount()
  })
})

describe('Tabs：键盘导航', () => {
  it('ArrowRight 激活下一项并把焦点移过去', async () => {
    const wrapper = mountTabs()
    key(wrapper, 'ArrowRight')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['params'])
    expect(wrapper.emitted('change')?.[0]).toEqual(['params'])
    await wrapper.setProps({ modelValue: 'params' })
    expect(document.activeElement?.getAttribute('data-key')).toBe('params')
    wrapper.unmount()
  })

  it('ArrowLeft 在首项回绕到末项', () => {
    const wrapper = mountTabs()
    key(wrapper, 'ArrowLeft')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['docs'])
    wrapper.unmount()
  })

  it('Home / End 跳首尾', () => {
    const w1 = mountTabs('params')
    key(w1, 'Home')
    expect(w1.emitted('update:modelValue')?.[0]).toEqual(['request'])
    w1.unmount()

    const w2 = mountTabs('request')
    key(w2, 'End')
    expect(w2.emitted('update:modelValue')?.[0]).toEqual(['docs'])
    w2.unmount()
  })

  it('非导航键不拦截（保持原生行为）', () => {
    const wrapper = mountTabs()
    const ev = key(wrapper, 'Enter')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect(ev.defaultPrevented).toBe(false)
    wrapper.unmount()
  })
})
