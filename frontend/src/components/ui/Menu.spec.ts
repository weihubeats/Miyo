/**
 * Menu 单测：open/close 事件（触发器 Tooltip 靠它避让菜单）、选择流程、
 * 键盘导航（↑↓/Home/End 漫游 + Enter 激活 + 确认视图 Enter/Esc）。
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

function openVia(wrapper: ReturnType<typeof mount>, anchor: HTMLElement, items: unknown[]): void {
  const vm = wrapper.vm as unknown as {
    openAt: (el: HTMLElement, items: unknown[], side?: string) => void
  }
  vm.openAt(anchor, items)
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
    openVia(wrapper, anchor, [{ key: 'a', label: '动作A' }])
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
    openVia(wrapper, anchor, [{ key: 'a', label: '动作A', disabled: true }])
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

describe('Menu：键盘导航', () => {
  /** 键盘事件派发到菜单容器（真实使用：容器聚焦后按键）。 */
  async function press(key: string): Promise<void> {
    document.querySelector<HTMLElement>('.miyo-menu')!
      .dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
    await nextTick()
  }

  /** 当前 aria-activedescendant 指向的项 key。 */
  function activeKey(): string | null {
    const menu = document.querySelector<HTMLElement>('.miyo-menu')!
    return menu.getAttribute('aria-activedescendant')?.split('-item-')[1] ?? null
  }

  it('打开即高亮首个可用项，↑↓ 循环漫游跳过 disabled，Home/End 跳边界', async () => {
    const { wrapper, anchor } = mountMenu()
    openVia(wrapper, anchor, [
      { key: 'a', label: 'A' },
      { key: 'b', label: 'B', disabled: true },
      { key: 'c', label: 'C' },
    ])
    await nextTick()
    expect(activeKey()).toBe('a')
    await press('ArrowDown')
    expect(activeKey()).toBe('c') // b disabled 跳过
    await press('ArrowDown')
    expect(activeKey()).toBe('a') // 循环回绕
    await press('ArrowUp')
    expect(activeKey()).toBe('c')
    await press('End')
    expect(activeKey()).toBe('c')
    await press('Home')
    expect(activeKey()).toBe('a')
    wrapper.unmount()
  })

  it('Enter 激活当前项；关闭后焦点归还触发元素', async () => {
    const { wrapper, anchor } = mountMenu()
    anchor.focus()
    openVia(wrapper, anchor, [
      { key: 'a', label: 'A' },
      { key: 'b', label: 'B' },
    ])
    await nextTick()
    await press('ArrowDown')
    await press('Enter')
    await nextTick()
    expect(wrapper.emitted('select')?.[0]?.[0]).toMatchObject({ key: 'b' })
    expect(document.querySelector('.miyo-menu')).toBeNull()
    expect(document.activeElement).toBe(anchor)
    wrapper.unmount()
  })

  it('confirm 项：Enter 进入确认视图，确认视图 Enter 确认 / Esc 返回列表', async () => {
    const { wrapper, anchor } = mountMenu()
    openVia(wrapper, anchor, [{ key: 'del', label: '删除', confirm: '确认删除？' }])
    await nextTick()
    await press('Enter') // 激活 confirm 项 → 进入确认视图
    await nextTick()
    expect(document.querySelector('.miyo-menu-confirm')).toBeTruthy()
    expect(wrapper.emitted('confirm')).toBeUndefined()
    await press('Escape') // 确认视图 Esc → 回列表（菜单不关）
    await nextTick()
    expect(document.querySelector('.miyo-menu-confirm')).toBeNull()
    expect(document.querySelector('.miyo-menu')).toBeTruthy()
    expect(wrapper.emitted('close')).toBeUndefined()
    await press('Enter') // 再次激活 → 确认视图
    await nextTick()
    await press('Enter') // 确认视图 Enter → confirm
    await nextTick()
    expect(wrapper.emitted('confirm')?.[0]?.[0]).toMatchObject({ key: 'del' })
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })
})
