/**
 * StatusBadge 单测：tone → .miyo-status.tone-* 类映射、状态点渲染。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import StatusBadge from '@/components/ui/StatusBadge.vue'

describe('StatusBadge', () => {
  it('渲染 tone 类与文案', () => {
    const w = mount(StatusBadge, { props: { tone: 'ok', label: '通过' } })
    const el = w.find('.miyo-status')
    expect(el.classes()).toContain('tone-ok')
    expect(el.text()).toBe('通过')
    w.unmount()
  })

  it('默认无状态点，dot=true 渲染且 aria-hidden', () => {
    const w0 = mount(StatusBadge, { props: { tone: 'fail', label: '失败' } })
    expect(w0.find('.st-dot').exists()).toBe(false)
    w0.unmount()

    const w1 = mount(StatusBadge, { props: { tone: 'run', label: '运行中', dot: true } })
    expect(w1.classes().join(' ')).toContain('tone-run')
    const dot = w1.find('.st-dot')
    expect(dot.exists()).toBe(true)
    expect(dot.attributes('aria-hidden')).toBe('true')
    w1.unmount()
  })

  it('七态运行 tone 均有对应类', () => {
    for (const tone of ['ok', 'fail', 'warn', 'info', 'run', 'wait', 'skip', 'blocked', 'cancel'] as const) {
      const w = mount(StatusBadge, { props: { tone, label: 'x' } })
      expect(w.find('.miyo-status').classes()).toContain(`tone-${tone}`)
      w.unmount()
    }
  })
})
