/**
 * Spinner 单测：默认纯装饰（aria-hidden），传 label 升级为 role=status。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import Spinner from '@/components/ui/Spinner.vue'

describe('Spinner', () => {
  it('默认装饰态：aria-hidden，无 role', () => {
    const w = mount(Spinner)
    const el = w.find('.miyo-spinner')
    expect(el.exists()).toBe(true)
    expect(el.attributes('aria-hidden')).toBe('true')
    expect(el.attributes('role')).toBeUndefined()
    w.unmount()
  })

  it('传 label 后渲染 role=status + aria-label', () => {
    const w = mount(Spinner, { props: { label: '运行中' } })
    const el = w.find('.miyo-spinner')
    expect(el.attributes('role')).toBe('status')
    expect(el.attributes('aria-label')).toBe('运行中')
    expect(el.attributes('aria-hidden')).toBeUndefined()
    w.unmount()
  })
})
