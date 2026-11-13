/**
 * Menu 单测：open/close 事件（触发器 Tooltip 靠它避让菜单）与选择流程。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import Menu from '@/components/ui/Menu.vue'
import { useLocaleStore } from '@/stores/locale'

function mountMenu() {
  const wrapper = mount(Menu, { attachTo: document.body })
  const anchor = document.createElement('button')
  document.body.appendChild(anchor)
  return { wrapper, anchor }
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('Menu：打开关闭事件与选择', () => {
  it('openAt 触发 open；选择后触发 select + close', async () => {
    const { wrapper, anchor } = mountMenu()
    const vm = wrapper.vm as unknown as {
      openAt: (el: HTMLElement, items: unknown[], side?: string) => void
    }
    vm.openAt(anchor, [{ key: 'a', label: '动作A' }])
    await nextTick()
    expect(wrapper.emitted('open')).toHaveLength(1)
    expect(document.querySelector('.miyo-menu')).toBeTruthy()

    document
      .querySelector<HTMLElement>('.miyo-menu-item')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('select')?.[0]?.[0]).toMatchObject({ key: 'a' })
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(document.querySelector('.miyo-menu')).toBeNull()
    wrapper.unmount()
  })

  it('Esc 关闭触发 close；已关闭时重复 close 不重复触发', async () => {
    const { wrapper, anchor } = mountMenu()
    const vm = wrapper.vm as unknown as {
      openAt: (el: HTMLElement, items: unknown[], side?: string) => void
      close: () => void
    }
    vm.openAt(anchor, [{ key: 'a', label: '动作A' }])
    await nextTick()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    // 已关闭：再次 close 不应重复触发
    vm.close()
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('disabled 项点击无反应', async () => {
    const { wrapper, anchor } = mountMenu()
    const vm = wrapper.vm as unknown as {
      openAt: (el: HTMLElement, items: unknown[], side?: string) => void
    }
    vm.openAt(anchor, [{ key: 'a', label: '动作A', disabled: true }])
    await nextTick()
    document
      .querySelector<HTMLElement>('.miyo-menu-item')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('select')).toBeUndefined()
    expect(document.querySelector('.miyo-menu')).toBeTruthy()
    wrapper.unmount()
  })
})
