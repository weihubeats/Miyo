/**
 * AuthPanel 单测：认证类型切换 / 动态签名高级配置交互稳定性。
 * 锁定回归：config 缺失时渲染不崩溃、展开高级配置不导致页面子树卸载。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick, reactive } from 'vue'
import AuthPanel from '@/domains/endpoint/components/AuthPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import { collectErrors } from '@/testUtils/componentTest'
import type { Endpoint } from '@/types/miyoApi'

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({ oauthAuthorize: vi.fn() }),
}))

function draft(): Endpoint {
  return reactive(makeDraft({ id: 'ep-1' }))
}

/** 动态签名类型但 config 缺失（历史数据 / IPC 异常路径）。 */
function draftWithoutConfig(): Endpoint {
  const d = makeDraft({ id: 'ep-1' })
  d.request.auth = { type: 'dynamic_signature' } as Endpoint['request']['auth']
  return reactive(d)
}

beforeEach(() => {
  vi.clearAllMocks()
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

describe('AuthPanel：动态签名交互稳定性', () => {
  it('下拉选中动态签名 → 点击高级设置展开收起，不抛错、面板不消失', async () => {
    const errors = collectErrors()
    const wrapper = mount(AuthPanel, { props: { draft: draft() } })
    await nextTick()

    await wrapper.find('.auth-type-select button').trigger('click')
    await nextTick()
    const dyn = Array.from(document.querySelectorAll('.cs-opt')).find((o) =>
      o.textContent?.includes('动态签名'),
    )
    expect(dyn).toBeTruthy()
    ;(dyn as HTMLElement).click()
    await nextTick()

    expect(wrapper.find('.sig-form').exists()).toBe(true)
    expect(wrapper.find('.sig-adv-body').exists()).toBe(true)

    const toggle = wrapper.find('.sig-toggle')
    await toggle.trigger('click')
    await nextTick()
    expect(wrapper.find('.sig-adv-body').exists()).toBe(false)

    await toggle.trigger('click')
    await nextTick()
    expect(wrapper.find('.sig-adv-body').exists()).toBe(true)
    expect(wrapper.find('.sig-form').exists()).toBe(true)

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('config 缺失时渲染不崩溃，且自动物化默认配置供 v-model 写入', async () => {
    const errors = collectErrors()
    const wrapper = mount(AuthPanel, { props: { draft: draftWithoutConfig() } })
    await nextTick()

    expect(wrapper.find('.sig-form').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="App Key"]').exists()).toBe(true)

    await wrapper.find('.sig-toggle').trigger('click')
    await nextTick()
    expect(wrapper.find('.sig-adv-body').exists()).toBe(true)

    const keyInput = wrapper.find('input[placeholder="App Key"]')
    await keyInput.setValue('key-123')
    expect((wrapper.vm.$props.draft as Endpoint).request.auth).toMatchObject({
      type: 'dynamic_signature',
      config: expect.objectContaining({ app_key: 'key-123' }),
    })

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })
})
