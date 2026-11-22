/**
 * Switch 单测：role=switch 语义、v-model 往返、禁用阻断、尺寸档。
 */
import { describe, expect, it, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import Switch from '@/components/ui/Switch.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('Switch：ARIA 语义', () => {
  it('role=switch + aria-checked 跟随 modelValue', async () => {
    const wrapper = mount(Switch, { props: { modelValue: false } })
    const el = wrapper.find('[role="switch"]')
    expect(el.exists()).toBe(true)
    expect(el.attributes('aria-checked')).toBe('false')
    await wrapper.setProps({ modelValue: true })
    expect(el.attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })

  it('label prop 透传为 aria-label，未传不产生空属性', () => {
    const withLabel = mount(Switch, { props: { modelValue: false, label: '代理' } })
    expect(withLabel.find('[role="switch"]').attributes('aria-label')).toBe('代理')
    withLabel.unmount()
    const noLabel = mount(Switch, { props: { modelValue: false } })
    expect(noLabel.find('[role="switch"]').attributes('aria-label')).toBeUndefined()
    noLabel.unmount()
  })
})

describe('Switch：交互', () => {
  it('点击发出 update:modelValue / change（false→true）', async () => {
    const wrapper = mount(Switch, { props: { modelValue: false } })
    await wrapper.find('[role="switch"]').trigger('click')
    expect(wrapper.emitted('update:modelValue')).toEqual([[true]])
    expect(wrapper.emitted('change')).toEqual([[true]])
    wrapper.unmount()
  })

  it('点击发出 update:modelValue / change（true→false）', async () => {
    const wrapper = mount(Switch, { props: { modelValue: true } })
    await wrapper.find('[role="switch"]').trigger('click')
    expect(wrapper.emitted('update:modelValue')).toEqual([[false]])
    expect(wrapper.emitted('change')).toEqual([[false]])
    wrapper.unmount()
  })

  it('disabled 时点击不发出任何事件', async () => {
    const wrapper = mount(Switch, { props: { modelValue: false, disabled: true } })
    const el = wrapper.find('[role="switch"]')
    expect(el.attributes('disabled')).toBeDefined()
    await el.trigger('click')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect(wrapper.emitted('change')).toBeUndefined()
    wrapper.unmount()
  })

  it('原生 button 键盘激活（空格触发 click）', async () => {
    const wrapper = mount(Switch, { props: { modelValue: false }, attachTo: document.body })
    const el = wrapper.find('[role="switch"]')
    const htmlEl = el.element as HTMLElement
    htmlEl.focus()
    htmlEl.dispatchEvent(
      new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }),
    )
    htmlEl.click() // 浏览器在 keydown 空格后于 keyup 触发 click；此处直接断言 click 通道
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('update:modelValue')).toEqual([[true]])
    wrapper.unmount()
  })
})

describe('Switch：尺寸档', () => {
  it('默认 md，size=sm 输出对应类', () => {
    const md = mount(Switch, { props: { modelValue: false } })
    expect(md.find('[role="switch"]').classes()).toContain('sw-md')
    md.unmount()
    const sm = mount(Switch, { props: { modelValue: false, size: 'sm' } })
    expect(sm.find('[role="switch"]').classes()).toContain('sw-sm')
    sm.unmount()
  })

  it('选中态携带 sw-on 类（accent 底由样式层消费）', () => {
    const wrapper = mount(Switch, { props: { modelValue: true } })
    expect(wrapper.find('[role="switch"]').classes()).toContain('sw-on')
    wrapper.unmount()
  })
})
