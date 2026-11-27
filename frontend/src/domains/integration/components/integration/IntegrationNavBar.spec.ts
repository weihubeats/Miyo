/**
 * IntegrationNavBar：统一导航（返回目标 + 层级面包屑）。
 * - 返回按钮文案随层级变化，点击 emit back；
 * - 祖先层级可点，当前层级 aria-current=page 且不可点。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import IntegrationNavBar from '@/domains/integration/components/integration/IntegrationNavBar.vue'

describe('IntegrationNavBar', () => {
  it('展示返回目标文案并 emit back', async () => {
    const w = mount(IntegrationNavBar, {
      props: { backLabel: '返回测试场景', crumbs: [{ label: '集成测试' }] },
    })
    const back = w.find('.int-nav-back')
    expect(back.text()).toContain('返回测试场景')
    await back.trigger('click')
    expect(w.emitted('back')).toHaveLength(1)
  })

  it('按层级渲染面包屑，当前层级标记 aria-current 且不可点', () => {
    const w = mount(IntegrationNavBar, {
      props: {
        backLabel: '返回批次详情',
        crumbs: [
          { label: '集成测试' },
          { label: '执行历史' },
          { label: '批次 #12ab' },
        ],
      },
    })
    const crumbs = w.findAll('.int-nav-crumb')
    expect(crumbs.map((c) => c.text())).toEqual(['集成测试', '执行历史', '批次 #12ab'])
    expect(crumbs[2]!.attributes('aria-current')).toBe('page')
    expect(w.findAll('.int-nav-crumb.link')).toHaveLength(0)
  })

  it('suffix=true：末级不标记 current（后面还有可编辑当前对象）', () => {
    const w = mount(IntegrationNavBar, {
      props: {
        backLabel: '返回测试场景',
        suffix: true,
        crumbs: [{ label: '集成测试' }, { label: '电商业务' }],
      },
    })
    expect(w.findAll('.int-nav-crumb.current')).toHaveLength(0)
    expect(w.find('.int-nav-crumb').attributes('aria-current')).toBeUndefined()
  })

  it('可点祖先层级触发回调', async () => {
    const onClick = vi.fn()
    const w = mount(IntegrationNavBar, {
      props: {
        backLabel: '返回测试场景',
        crumbs: [{ label: '集成测试', onClick }, { label: '电商业务' }],
      },
    })
    const link = w.find('.int-nav-crumb.link')
    expect(link.text()).toBe('集成测试')
    await link.trigger('click')
    expect(onClick).toHaveBeenCalledTimes(1)
  })
})