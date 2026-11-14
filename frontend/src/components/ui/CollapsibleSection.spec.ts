/**
 * CollapsibleSection 单测：非受控开合 / v-model:open 受控 / 计数徽章 / a11y。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import CollapsibleSection from '@/components/ui/CollapsibleSection.vue'

describe('CollapsibleSection', () => {
  it('defaultOpen 默认展开，点击头部收起/展开', async () => {
    const w = mount(CollapsibleSection, {
      props: { title: '断言' },
      slots: { default: '<div class="inner">内容</div>' },
    })
    const head = w.find('.sec-head')
    expect(head.attributes('aria-expanded')).toBe('true')
    expect(w.find('.inner').exists()).toBe(true)

    await head.trigger('click')
    expect(head.attributes('aria-expanded')).toBe('false')
    expect(w.emitted('update:open')?.[0]).toEqual([false])
    // 收起仅视觉隐藏（grid 0fr），内容仍在 DOM
    expect(w.find('.inner').exists()).toBe(true)

    await head.trigger('click')
    expect(w.emitted('update:open')?.[1]).toEqual([true])
    w.unmount()
  })

  it('defaultOpen=false 初始收起', () => {
    const w = mount(CollapsibleSection, { props: { title: '请求', defaultOpen: false } })
    expect(w.find('.sec-head').attributes('aria-expanded')).toBe('false')
    w.unmount()
  })

  it('受控模式：open prop 驱动，点击发出 update:open', async () => {
    const w = mount(CollapsibleSection, { props: { title: '响应', open: false } })
    expect(w.find('.sec-head').attributes('aria-expanded')).toBe('false')
    await w.find('.sec-head').trigger('click')
    expect(w.emitted('update:open')?.[0]).toEqual([true])
    // 父级未响应 prop，仍保持收起
    expect(w.find('.sec-head').attributes('aria-expanded')).toBe('false')
    w.unmount()
  })

  it('count 徽章与 aria-controls 关联', () => {
    const w = mount(CollapsibleSection, { props: { title: '提取', count: 3 } })
    expect(w.find('.sec-count').text()).toBe('3')
    const controls = w.find('.sec-head').attributes('aria-controls')
    expect(w.find(`#${controls}`).exists()).toBe(true)
    w.unmount()
  })
})
