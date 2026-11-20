/**
 * ThemeAppearance 冒烟测试：主题网格渲染 / 选卡即生效 / 自定义编辑器展开 / 键盘漫游。
 * 组件 setup 消费 locale store：按 AGENTS.md 规范固定 zh 语言 + 独立 pinia，
 * 避免断言文案随环境语言漂移。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ThemeAppearance from '@/domains/workspace/components/settings/ThemeAppearance.vue'
import { useLocaleStore } from '@/stores/locale'
import { useThemeStore } from '@/stores/theme'

describe('ThemeAppearance', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    try {
      localStorage.clear()
    } catch {
      // 存储不可用时跳过（用例内状态互不影响依赖重建 pinia）
    }
    document.documentElement.removeAttribute('data-theme')
    document.documentElement.removeAttribute('data-theme-id')
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('渲染 8 内置主题卡 + 自定义入口（radiogroup）', () => {
    const wrapper = mount(ThemeAppearance)
    const radios = wrapper.findAll('[role="radio"][aria-checked]')
    expect(radios.length).toBe(9)
    expect(wrapper.text()).toContain('石墨深色')
    expect(wrapper.text()).toContain('海盐蓝')
    expect(wrapper.text()).toContain('海盐浅蓝')
    expect(wrapper.text()).toContain('森林绿')
    expect(wrapper.text()).toContain('自定义')
    expect(wrapper.text()).toContain('跟随系统')
    wrapper.unmount()
  })

  it('点击主题卡立即生效（DOM data-theme-id + 选卡状态）', async () => {
    const wrapper = mount(ThemeAppearance)
    const ocean = wrapper.findAll('[role="radio"]').find((n) => n.text().includes('海盐蓝'))!
    await ocean.trigger('click')
    const theme = useThemeStore()
    expect(theme.themeId).toBe('ocean')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('ocean')
    expect(ocean.attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })

  it('选中自定义后展开编辑器（5 个颜色行 + 恢复默认主题）', async () => {
    const wrapper = mount(ThemeAppearance)
    const custom = wrapper.findAll('[role="radio"]').find((n) => n.text().includes('自定义'))!
    await custom.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('input[type="color"]').length).toBe(5)
    expect(wrapper.text()).toContain('恢复默认主题')
    const theme = useThemeStore()
    expect(theme.custom).not.toBeNull()
    // 恢复默认主题：清空自定义槽位并回到石墨深色
    theme.resetTheme()
    expect(theme.themeId).toBe('dark')
    wrapper.unmount()
  })

  it('网格方向键漫游：焦点在主题卡间移动', async () => {
    const wrapper = mount(ThemeAppearance, { attachTo: document.body })
    const cards = wrapper.findAll('button[role="radio"]')
    await cards[0].trigger('keydown', { key: 'ArrowRight' })
    expect(document.activeElement).toBe(cards[1].element)
    await cards[1].trigger('keydown', { key: 'ArrowLeft' })
    expect(document.activeElement).toBe(cards[0].element)
    wrapper.unmount()
  })
})
