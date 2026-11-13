/**
 * CreatePanel 单测：分组渲染 / 选择 / Esc / 键盘导航 / 定位。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import CreatePanel from '@/domains/workspace/components/CreatePanel.vue'
import { useLocaleStore } from '@/stores/locale'

function mountPanel() {
  const wrapper = mount(CreatePanel, { attachTo: document.body })
  const anchor = document.createElement('button')
  document.body.appendChild(anchor)
  return { wrapper, anchor }
}

function openPanel(wrapper: ReturnType<typeof mountPanel>['wrapper'], anchor: HTMLElement) {
  ;(
    wrapper.vm as unknown as {
      openAt: (el: HTMLElement) => void
    }
  ).openAt(anchor)
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('CreatePanel：新建面板', () => {
  it('打开后渲染分组标题与 4 个真实动作', async () => {
    const { wrapper, anchor } = mountPanel()
    openPanel(wrapper, anchor)
    await nextTick()
    expect(document.querySelector('.cp-pop')).toBeTruthy()
    expect(document.querySelector('.cp-group-title')?.textContent).toBe('新建')
    const labels = Array.from(document.querySelectorAll('.cp-card-label, .cp-row-label')).map(
      (el) => el.textContent?.trim(),
    )
    expect(labels).toEqual(['新建 HTTP 请求', '新建文件夹', '导入 cURL', '导入接口 (Postman / Swagger)'])
    expect(wrapper.emitted('open')).toHaveLength(1)
    wrapper.unmount()
  })

  it('点击动作触发 select 并关闭', async () => {
    const { wrapper, anchor } = mountPanel()
    openPanel(wrapper, anchor)
    await nextTick()
    document
      .querySelectorAll<HTMLElement>('.cp-card')[1]!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('select')?.[0]).toEqual(['new-folder'])
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(document.querySelector('.cp-pop')).toBeNull()
    wrapper.unmount()
  })

  it('Esc 关闭；键盘下键 + 回车选中第二项', async () => {
    const { wrapper, anchor } = mountPanel()
    openPanel(wrapper, anchor)
    await nextTick()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    expect(document.querySelector('.cp-pop')).toBeNull()

    openPanel(wrapper, anchor)
    await nextTick()
    // 面板内按键冒泡到面板处理器（document 级只处理 Esc）
    const firstCard = document.querySelector<HTMLElement>('.cp-card')!
    firstCard.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    await nextTick()
    firstCard.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('select')?.[0]).toEqual(['new-folder'])
    wrapper.unmount()
  })

  it('面板在触发器下方左对齐展开（有空间时不向上翻）', async () => {
    const { wrapper, anchor } = mountPanel()
    openPanel(wrapper, anchor)
    await nextTick()
    await nextTick()
    const pop = document.querySelector('.cp-pop') as HTMLElement
    // jsdom 无布局（rect 全 0）：left 钳制到 8，top 落到触发器下方 +8
    expect(pop.style.left).toBe('8px')
    expect(pop.style.top).toBe('8px')
    wrapper.unmount()
  })
})
