/**
 * MockRuleDialog 单测：挂载后必须渲染出 Modal 弹层。
 * 回归背景：曾漏 import Modal，Vue 解析失败导致「打开 Mock 管理」看似无反应。
 */
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import MockRuleDialog from '@/domains/mock/components/MockRuleDialog.vue'
import { useLocaleStore } from '@/stores/locale'

vi.mock('@/stores/workspace.ts', () => ({
  useWorkspaceStore: () => ({
    project: { id: 'p-1', name: 'P' },
  }),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    listMockRules: vi.fn().mockResolvedValue([]),
    saveMockRule: vi.fn(),
    deleteMockRule: vi.fn(),
  }),
}))

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn() }),
}))

describe('MockRuleDialog', () => {
  it('应渲染 Modal 弹层（标题含规则计数）', async () => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
    mount(MockRuleDialog)
    await flushPromises()
    // Modal Teleport 到 body，需查 document 而非 wrapper
    const mask = document.body.querySelector('.m-mask')
    expect(mask, 'Modal 遮罩应渲染到 body').not.toBeNull()
    expect(mask?.textContent).toContain('Mock 规则')
  })
})
