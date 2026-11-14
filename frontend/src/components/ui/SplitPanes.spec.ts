/**
 * SplitPanes 单测：拖拽改宽（clamp）+ 键盘调宽 + 双击 reset + 窄容器堆叠降级。
 * jsdom 无布局：stackBelow 模式下 clientWidth 恒 0，正好覆盖「低于阈值」分支。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import SplitPanes from '@/components/ui/SplitPanes.vue'

function fire(type: string, x: number): void {
  window.dispatchEvent(new MouseEvent(type, { clientX: x, bubbles: true }))
}

describe('SplitPanes', () => {
  it('拖拽分割线按位移更新宽度（v-model:width）', async () => {
    const w = mount(SplitPanes, {
      props: { width: 420, min: 220, max: 800 },
      slots: { first: '<div>请求</div>', second: '<div>响应</div>' },
    })
    await w.find('.sp-divider').trigger('mousedown', { clientX: 500 })
    fire('mousemove', 560) // +60
    fire('mouseup', 560)
    expect(w.emitted('update:width')?.[0]).toEqual([480])
    w.unmount()
  })

  it('宽度夹紧到 [min, max]', async () => {
    const w = mount(SplitPanes, {
      props: { width: 420, min: 220, max: 800 },
    })
    await w.find('.sp-divider').trigger('mousedown', { clientX: 0 })
    fire('mousemove', 2000) // 远超 max
    fire('mouseup', 2000)
    expect(w.emitted('update:width')?.[0]).toEqual([800])

    await w.find('.sp-divider').trigger('mousedown', { clientX: 0 })
    fire('mousemove', -2000) // 远低于 min
    fire('mouseup', -2000)
    expect(w.emitted('update:width')?.[1]).toEqual([220])
    w.unmount()
  })

  it('键盘 ←/→ 调宽，Shift 步进 48，Enter 发 reset', async () => {
    const w = mount(SplitPanes, { props: { width: 400, min: 200 } })
    const d = w.find('.sp-divider')
    await d.trigger('keydown', { key: 'ArrowLeft' })
    expect(w.emitted('update:width')?.[0]).toEqual([384])
    await d.trigger('keydown', { key: 'ArrowRight', shiftKey: true })
    expect(w.emitted('update:width')?.[1]).toEqual([448])
    await d.trigger('keydown', { key: 'Enter' })
    expect(w.emitted('reset')).toHaveLength(1)
    w.unmount()
  })

  it('双击分割线发 reset', async () => {
    const w = mount(SplitPanes, { props: { width: 400 } })
    await w.find('.sp-divider').trigger('dblclick')
    expect(w.emitted('reset')).toHaveLength(1)
    w.unmount()
  })

  it('容器低于 stackBelow 时转堆叠：隐藏分割线、首栏不再定宽', async () => {
    const w = mount(SplitPanes, {
      props: { width: 420, stackBelow: 900 },
    })
    await w.vm.$nextTick() // measure 在 onMounted 置位，等 DOM 刷新
    expect(w.find('.sp.stacked').exists()).toBe(true)
    expect(w.find('.sp-divider').exists()).toBe(false)
    expect(w.find('.sp-first').attributes('style')).toBeUndefined()
    w.unmount()
  })

  it('渲染宽度夹紧（旧记忆超 max 不撑破布局）', () => {
    const w = mount(SplitPanes, { props: { width: 5000, max: 800, min: 220 } })
    expect(w.find('.sp-first').attributes('style')).toContain('800px')
    w.unmount()
  })

  it('widthOn=second：宽度落在次栏，拖拽方向反转（左拖=加宽）', async () => {
    const w = mount(SplitPanes, {
      props: { width: 420, min: 220, widthOn: 'second' },
      slots: { first: '<div>列表</div>', second: '<div>详情</div>' },
    })
    expect(w.find('.sp-second').attributes('style')).toContain('420px')
    expect(w.find('.sp-first').attributes('style')).toBeUndefined()

    await w.find('.sp-divider').trigger('mousedown', { clientX: 500 })
    fire('mousemove', 440) // 左移 60 → 次栏加宽 60
    fire('mouseup', 440)
    expect(w.emitted('update:width')?.[0]).toEqual([480])
    w.unmount()
  })
})
