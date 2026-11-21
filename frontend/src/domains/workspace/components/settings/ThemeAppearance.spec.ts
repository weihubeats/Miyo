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
    document.documentElement.removeAttribute('data-style')
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('渲染 8 内置主题卡 + 自定义入口（radiogroup）', () => {
    const wrapper = mount(ThemeAppearance)
    // radiogroup[0] = 主题网格（风格网格另见下方专属用例）
    const radios = wrapper.findAll('[role="radiogroup"]')[0].findAll('[role="radio"][aria-checked]')
    expect(radios.length).toBe(9)
    expect(wrapper.text()).toContain('石墨深色')
    expect(wrapper.text()).toContain('海盐蓝')
    expect(wrapper.text()).toContain('海盐浅蓝')
    expect(wrapper.text()).toContain('森林绿')
    expect(wrapper.text()).toContain('自定义')
    expect(wrapper.text()).toContain('跟随系统')
    wrapper.unmount()
  })

  it('渲染 3 张视觉风格卡（radiogroup）并选中当前档', () => {
    const wrapper = mount(ThemeAppearance)
    const groups = wrapper.findAll('[role="radiogroup"]')
    expect(groups.length).toBe(2)
    const styleCards = groups[1].findAll('[role="radio"]')
    expect(styleCards.length).toBe(3)
    expect(wrapper.text()).toContain('开发者工作台')
    expect(wrapper.text()).toContain('极简')
    expect(wrapper.text()).toContain('柔和')
    // 预览子树按风格重新作用域 token（真实界面预览的数据源即 [data-style] 块）
    const preview = groups[1].findAll('.sc-preview')
    expect(preview.map((n) => n.attributes('data-style'))).toEqual(['studio', 'minimal', 'soft'])
    wrapper.unmount()
  })

  it('点击风格卡立即生效（data-style + 持久化），与颜色主题独立', async () => {
    const wrapper = mount(ThemeAppearance)
    const groups = wrapper.findAll('[role="radiogroup"]')
    const soft = groups[1].findAll('[role="radio"]').find((n) => n.text().includes('柔和'))!
    await soft.trigger('click')
    const theme = useThemeStore()
    expect(theme.style).toBe('soft')
    expect(document.documentElement.getAttribute('data-style')).toBe('soft')
    expect(theme.themeId).toBe('dark') // 颜色主题未被风格切换牵动
    expect(JSON.parse(localStorage.getItem('miyo.theme.config')!).style).toBe('soft')
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
