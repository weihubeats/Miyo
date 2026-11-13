/**
 * CreateChoicePage 单测：新建选择页（接口标签条「+」打开的独立页面）。
 * 锁定：两张竖排大卡片文案与顺序 / 点击上抛 pick(key) / Esc 上抛 close。
 */
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import CreateChoicePage from '@/domains/workspace/components/CreateChoicePage.vue'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('CreateChoicePage：新建选择页', () => {
  it('渲染两张竖排大卡片：新建请求 / 导入 cURL', () => {
    const wrapper = mount(CreateChoicePage)

    expect(wrapper.findAll('.cc-card')).toHaveLength(2)
    const labels = wrapper.findAll('.cc-label').map((el) => el.text())
    expect(labels).toEqual(['新建 HTTP 请求', '导入 cURL'])
    // 大徽章走主题色调（蓝 zap / 绿 terminal），页面本身不弹 toast 不发请求
    expect(wrapper.find('.cc-icon.tone-blue').exists()).toBe(true)
    expect(wrapper.find('.cc-icon.tone-green').exists()).toBe(true)

    wrapper.unmount()
  })

  it('点击卡片上抛 pick(key)，动作由父级执行', async () => {
    const wrapper = mount(CreateChoicePage)
    const cards = wrapper.findAll('.cc-card')

    await cards[0].trigger('click')
    expect(wrapper.emitted('pick')?.[0]).toEqual(['new-request'])

    await cards[1].trigger('click')
    expect(wrapper.emitted('pick')?.[1]).toEqual(['import-curl'])

    wrapper.unmount()
  })

  it('Esc 上抛 close（回到编辑器）', async () => {
    const wrapper = mount(CreateChoicePage, { attachTo: document.body })

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    // 其余按键不关页
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)

    wrapper.unmount()
  })
})
