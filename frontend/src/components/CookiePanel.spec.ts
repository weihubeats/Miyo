/**
 * CookiePanel 单测：列表渲染 + 按域清理。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CookiePanel from './CookiePanel.vue'
import { useLocaleStore } from '../stores/locale'
import type { CookieEntry } from '../types/miyoApi'

const apiMocks = {
  cookieList: vi.fn(),
  cookieClear: vi.fn().mockResolvedValue(1),
}

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => apiMocks,
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() }),
}))

function cookie(partial: Partial<CookieEntry> & { name: string }): CookieEntry {
  return {
    value: 'v',
    domain: 'api.example.com',
    path: '/',
    expires_at: null,
    secure: false,
    http_only: false,
    ...partial,
  }
}

describe('CookiePanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
  })

  it('按域名分组渲染 + 值默认掩码 + 揭示后截断', async () => {
    apiMocks.cookieList.mockResolvedValue([
      cookie({ name: 'sid', value: 'abcdef1234', domain: 'api.example.com' }),
      cookie({ name: 't', value: 'x'.repeat(60), domain: 'other.com', http_only: true }),
    ])
    const wrapper = mount(CookiePanel, { attachTo: document.body })
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.text()).toContain('api.example.com')
    expect(wrapper.text()).toContain('other.com')
    expect(wrapper.text()).toContain('sid')
    // 默认掩码：原文与截断省略号都不出现。
    expect(wrapper.text()).not.toContain('abcdef1234')
    expect(wrapper.text()).not.toContain('…')
    expect(wrapper.text()).toContain('••••')
    // 点眼睛揭示：值可见且仍按长度截断（悬浮 title 不再携带全文）。
    await wrapper.find('.cp-reload.active, [aria-pressed]').trigger('click')
    expect(wrapper.text()).toContain('abcdef1234')
    expect(wrapper.text()).toContain('…')
    expect(wrapper.find('.cp-row').attributes('title')).toBeUndefined()
    expect(apiMocks.cookieList).toHaveBeenCalled()
    wrapper.unmount()
  })

  /** 过滤词防抖后连发请求：旧响应后到不得覆盖新结果（seq 守卫）。 */
  it('过滤竞态：过期响应被丢弃', async () => {
    let resolveFirst!: (v: CookieEntry[]) => void
    apiMocks.cookieList.mockImplementationOnce(
      () => new Promise<CookieEntry[]>((r) => (resolveFirst = r)),
    )
    apiMocks.cookieList.mockResolvedValueOnce([cookie({ name: 'new', value: 'v2' })])
    const wrapper = mount(CookiePanel, { attachTo: document.body })
    await new Promise((r) => setTimeout(r, 0))
    // 第二次请求（新过滤词）先返回；随后旧请求才 resolve。
    await wrapper.find('.cp-reload:not(.active):not([aria-pressed])').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.text()).toContain('new')
    resolveFirst([cookie({ name: 'stale', value: 'old' })])
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.text()).not.toContain('stale')
    wrapper.unmount()
  })

  it('空列表展示空状态', async () => {
    apiMocks.cookieList.mockResolvedValue([])
    const wrapper = mount(CookiePanel, { attachTo: document.body })
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.text()).toContain('暂无 Cookie')
    wrapper.unmount()
  })
})
