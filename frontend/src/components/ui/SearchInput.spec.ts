/**
 * SearchInput：统一搜索框组件行为。
 * - v-model 双向；Enter 触发 search；Esc 清空并通知；
 * - 有值渲染清除钮，点击清空；清除钮走 i18n（zh 固定语言断言）。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import SearchInput from './SearchInput.vue'
import { useLocaleStore } from '@/stores/locale'

function mountInput(modelValue = '') {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  return mount(SearchInput, {
    props: { modelValue, placeholder: '搜索…' },
  })
}

describe('SearchInput', () => {
  it('渲染输入框并回显关键字', () => {
    const w = mountInput('abc')
    const input = w.find('input')
    expect((input.element as HTMLInputElement).value).toBe('abc')
    expect(input.attributes('placeholder')).toBe('搜索…')
  })

  it('输入 → update:modelValue；Enter → search', async () => {
    const w = mountInput()
    await w.find('input').setValue('hello')
    expect(w.emitted('update:modelValue')?.[0]).toEqual(['hello'])
    await w.find('input').trigger('keydown', { key: 'Enter' })
    expect(w.emitted('search')).toHaveLength(1)
  })

  it('有值显示清除钮，点击清空并通知 clear；无值不渲染', async () => {
    const w = mountInput()
    expect(w.find('.si-clear').exists()).toBe(false)
    await w.setProps({ modelValue: 'kw' })
    const clear = w.find('.si-clear')
    expect(clear.exists()).toBe(true)
    await clear.trigger('click')
    expect(w.emitted('update:modelValue')?.at(-1)).toEqual([''])
    expect(w.emitted('clear')).toHaveLength(1)
  })

  it('Esc 清空并阻断冒泡（避免连带关闭外层弹层）', async () => {
    const w = mountInput('kw')
    // 外层监听 stopPropagation 后不应收到事件
    let leaked = false
    w.element.addEventListener('keydown', () => {
      leaked = true
    })
    await w.find('input').trigger('keydown', { key: 'Escape' })
    expect(leaked).toBe(false)
    expect(w.emitted('update:modelValue')?.at(-1)).toEqual([''])
  })
})
