/**
 * CustomNumberInput 单测：手输提交链路（#134 回归——单向绑定父级曾丢弃手输值，
 * 失焦按过期 prop 提交）与既有双向回显用法的语义保持。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import CustomNumberInput from './CustomNumberInput.vue'
import { useLocaleStore } from '../../stores/locale'

function mountInput(props: Record<string, unknown> = {}) {
  return mount(
    CustomNumberInput,
    { props: { modelValue: 300, min: 1, max: 3600, step: 10, ...props }, attachTo: document.body },
  )
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 组件 setup 读取 locale store；文案断言锁定中文（jsdom 默认英文会漂移）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('CustomNumberInput：单向绑定父级（通用设置超时，#134）', () => {
  it('手输后失焦按输入值提交 change / update:modelValue', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('120')
    await input.trigger('blur')
    expect(wrapper.emitted('change')?.at(-1)).toEqual([120])
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([120])
  })

  it('手输超范围值失焦按 min/max 钳制', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('99999')
    await input.trigger('blur')
    expect(wrapper.emitted('change')?.at(-1)).toEqual([3600])
  })

  it('非法输入失焦恢复父级值（change 携带原值而非垃圾输入）', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('abc')
    await input.trigger('blur')
    await nextTick()
    expect(wrapper.emitted('change')?.at(-1)).toEqual([300])
    expect((input.element as HTMLInputElement).value).toBe('300')
  })

  it('步进按钮以手输值为基准（而非过期 prop）', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('120')
    await wrapper.find('.cni-step').trigger('click')
    expect(wrapper.emitted('change')?.at(-1)).toEqual([130])
  })

  it('回车直接提交（走失焦保存路径）', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('120')
    // 真实浏览器回车 key 为 'Enter'（test-utils 的 .enter 修饰符会小写，须显式传）
    await input.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('change')?.at(-1)).toEqual([120])
  })

  it('未编辑直接失焦维持原语义：按当前值提交 change', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.trigger('blur')
    expect(wrapper.emitted('change')).toEqual([[300]])
  })
})

describe('CustomNumberInput：双向回显父级（LoadTest / MockRule / EndpointEditor 既有用法）', () => {
  it('手输后父级回显，失焦提交同一值（行为不变）', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('120')
    await wrapper.setProps({ modelValue: 120 })
    await input.trigger('blur')
    expect(wrapper.emitted('change')?.at(-1)).toEqual([120])
  })

  it('清空后失焦保持空值，不误恢复', async () => {
    const wrapper = mountInput({ modelValue: '' })
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('')
    await wrapper.setProps({ modelValue: '' })
    await input.trigger('blur')
    await nextTick()
    expect(wrapper.emitted('change')).toBeUndefined()
    expect((input.element as HTMLInputElement).value).toBe('')
  })

  it('父级忽略非法输入（如 EndpointEditor）时失焦恢复原值', async () => {
    const wrapper = mountInput()
    const input = wrapper.find('input')
    input.element.focus()
    await input.setValue('abc')
    await input.trigger('blur')
    await nextTick()
    // prop 未被回写（仍 300）→ 恢复展示 300
    expect((input.element as HTMLInputElement).value).toBe('300')
  })
})
