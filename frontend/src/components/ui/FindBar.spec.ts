/**
 * FindBar 单测：替换行（replaceable 可选项）/ 焦点管理 / 键盘行为。
 * 回归边界：不传 replaceable 时与旧版纯查找条一致（响应查看器等宿主不受影响）。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import FindBar from '@/components/ui/FindBar.vue'
import { useLocaleStore } from '@/stores/locale'

function mountBar(props: Record<string, unknown> = {}) {
  return mount(
    FindBar,
    { props: { query: '', index: 0, total: 0, ...props }, attachTo: document.body },
  )
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('FindBar：纯查找（向后兼容）', () => {
  it('不传 replaceable：无展开钮、无替换行，计数与导航不受影响', async () => {
    const wrapper = mountBar({ query: 'x', index: 0, total: 2 })
    expect(wrapper.find('.findbar-expander').exists()).toBe(false)
    expect(wrapper.find('.findbar-replace-row').exists()).toBe(false)
    expect(wrapper.find('.findbar-count').text()).toBe('1 / 2')
    await wrapper.findAll('.findbar-btn')[1]!.trigger('click')
    expect(wrapper.emitted('next')).toHaveLength(1)
  })

  it('打开时查找框聚焦', () => {
    const wrapper = mountBar()
    expect(document.activeElement).toBe(wrapper.find('.findbar-input').element)
  })
})

describe('FindBar：替换行（请求 Body）', () => {
  it('replaceable：展开钮点击发 toggle-replace；replaceOpen 渲染替换行并聚焦', async () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: false })
    const expander = wrapper.find('.findbar-expander')
    expect(expander.exists()).toBe(true)
    await expander.trigger('click')
    expect(wrapper.emitted('toggle-replace')).toHaveLength(1)

    await wrapper.setProps({ replaceOpen: true, replace: 'new' })
    await nextTick()
    expect(wrapper.find('.findbar-replace-row').exists()).toBe(true)
    // 展开后焦点跟进替换输入框
    expect(document.activeElement).toBe(wrapper.find('.findbar-replace-row input').element)
  })

  it('替换输入 v-model 回传；「替换」按钮与 Enter 发 replace-one', async () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: true, query: 'old' })
    const replaceInput = wrapper.find('.findbar-replace-row input')
    await replaceInput.setValue('new')
    expect(wrapper.emitted('update:replace')?.at(-1)).toEqual(['new'])
    await wrapper.findAll('.findbar-replace-btn')[0]!.trigger('click')
    expect(wrapper.emitted('replace-one')).toHaveLength(1)
    // 一般编辑器惯例：Enter 替换下一处（单个替换）
    await replaceInput.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('replace-one')).toHaveLength(2)
  })

  it('「全部替换」按钮与 Ctrl/Cmd+Alt+Enter 发 replace-all；查找词为空时按钮禁用', async () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: true, query: '' })
    // 查找词为空：替换按钮禁用
    for (const btn of wrapper.findAll('.findbar-replace-btn')) {
      expect(btn.attributes('disabled')).toBeDefined()
    }
    await wrapper.setProps({ query: 'old' })
    await wrapper.find('.findbar-replace-btn + .findbar-replace-btn').trigger('click')
    expect(wrapper.emitted('replace-all')).toHaveLength(1)
    const replaceInput = wrapper.find('.findbar-replace-row input')
    await replaceInput.trigger('keydown', { key: 'Enter', ctrlKey: true, altKey: true })
    expect(wrapper.emitted('replace-all')).toHaveLength(2)
    await replaceInput.trigger('keydown', { key: 'Enter', metaKey: true, altKey: true })
    expect(wrapper.emitted('replace-all')).toHaveLength(3)
  })

  it('替换输入 Esc 关闭整个查找条', async () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: true })
    await wrapper.find('.findbar-replace-row input').trigger('keydown', { key: 'Escape' })
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('快捷键直开（replaceOpen 首挂即 true）：焦点落在替换输入框', () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: true })
    expect(document.activeElement).toBe(wrapper.find('.findbar-replace-row input').element)
  })

  it('replaceable 但未展开：查找框聚焦', () => {
    const wrapper = mountBar({ replaceable: true, replaceOpen: false })
    expect(document.activeElement).toBe(wrapper.find('.findbar-input').element)
  })
})
