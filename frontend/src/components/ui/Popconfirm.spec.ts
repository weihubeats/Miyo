/**
 * Popconfirm 单测：触发区点击展开、插槽内 stopPropagation 仍能展开、确认/取消语义。
 *
 * 组件 setup 已调 useLocaleStore，spec 统一 setActivePinia + setMode('zh')
 * （AGENTS.md 组件 spec 固定语言规范）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h } from 'vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import { useLocaleStore } from '@/stores/locale'

/** 挂一个带默认插槽的 Popconfirm，返回 wrapper；气泡被 Teleport 到 body。 */
function mountWithSlot(slot: () => ReturnType<typeof h>) {
  const Host = defineComponent({
    setup() {
      return () => h('div', { class: 'host-row' }, [h(Popconfirm, { title: '确认删除？' }, { default: slot })])
    },
  })
  return mount(Host, { attachTo: document.body })
}

function popEl(): HTMLElement | null {
  return document.body.querySelector('.pc-pop')
}

describe('Popconfirm', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('点击插槽内容展开气泡，再次点击收起', async () => {
    const wrapper = mountWithSlot(() => h('button', { class: 'del' }, '删除'))
    expect(popEl()).toBeNull()

    await wrapper.find('.del').trigger('click')
    await flushPromises()
    expect(popEl()).not.toBeNull()

    await wrapper.find('.del').trigger('click')
    await flushPromises()
    expect(popEl()).toBeNull()
    // 先 unmount 卸载 Teleport（锚点注释挂在 body 上），再清 body
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  /**
   * 回归：行内操作按钮普遍带 `@click.stop`（阻止触发行选中）。
   * 冒泡阶段被截断时，外层 .pc-trigger 收不到 click，气泡永远弹不出来——
   * v0.0.54 用户实测「场景点删除没反应」即此因。
   */
  it('插槽内容 stopPropagation 时仍能展开（点击走捕获阶段）', async () => {
    const wrapper = mountWithSlot(() =>
      h('button', { class: 'del', onClick: (e: Event) => e.stopPropagation() }, '删除'),
    )

    await wrapper.find('.del').trigger('click')
    await flushPromises()
    expect(popEl()).not.toBeNull()
    // 事件被 stop 了，不应该连带触发行点击
    expect(wrapper.find('.host-row').exists()).toBe(true)
    wrapper.unmount()
  })

  it('点取消只收起，不 emit confirm', async () => {
    const wrapper = mountWithSlot(() => h('button', { class: 'del' }, '删除'))

    await wrapper.find('.del').trigger('click')
    await flushPromises()
    Array.from(popEl()!.querySelectorAll('button'))
      .find((b) => b.textContent?.trim() === '取消')!
      .click()
    await flushPromises()
    const pc = wrapper.findComponent(Popconfirm)
    expect(pc.emitted('confirm')).toBeUndefined()
    expect(popEl()).toBeNull()
    // 先 unmount 卸载 Teleport（锚点注释挂在 body 上），再清 body
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('点确认键 emit confirm 后收起', async () => {
    const wrapper = mountWithSlot(() => h('button', { class: 'del' }, '删除'))
    await wrapper.find('.del').trigger('click')
    await flushPromises()
    // 确认键是实心危险按钮（.miyo-btn-danger-solid），文案走 i18n 默认值
    popEl()!.querySelector<HTMLButtonElement>('.miyo-btn-danger-solid')!.click()
    await flushPromises()
    const pc = wrapper.findComponent(Popconfirm)
    expect(pc.emitted('confirm')).toHaveLength(1)
    expect(popEl()).toBeNull()
    // 先 unmount 卸载 Teleport（锚点注释挂在 body 上），再清 body
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('外部点击关闭气泡', async () => {
    const wrapper = mountWithSlot(() => h('button', { class: 'del' }, '删除'))
    await wrapper.find('.del').trigger('click')
    await flushPromises()
    expect(popEl()).not.toBeNull()

    // 必须先 unmount 再清 body：Teleport 的注释锚点在 body 里，
    // 直接清空会让后续 patch 找不到节点（insertBefore of null）。
    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    await flushPromises()
    expect(popEl()).toBeNull()
    // 先 unmount 卸载 Teleport（锚点注释挂在 body 上），再清 body
    wrapper.unmount()
    document.body.innerHTML = ''
  })
})