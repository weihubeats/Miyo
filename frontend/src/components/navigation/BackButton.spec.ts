/**
 * BackButton 单测：统一返回按钮的渲染与行为分支。
 * 锁定回归：默认策略（back / fallback push）、onBack 守卫拦截、label 渲染。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createRouter, createMemoryHistory, type Router } from 'vue-router'
import BackButton from '@/components/navigation/BackButton.vue'
import { useLocaleStore } from '@/stores/locale'
import { NAV_HOME } from '@/composables/useGoBack'

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/projects', component: { template: '<div/>' } },
      { path: '/plugins', component: { template: '<div/>' } },
      { path: '/:pathMatch(.*)*', component: { template: '<div/>' } },
    ],
  })
}

async function mountButton(props: Record<string, unknown> = {}): Promise<{
  wrapper: ReturnType<typeof mount>
  router: Router
}> {
  const router = makeRouter()
  await router.push('/deep-page')
  await router.isReady()
  const wrapper = mount(BackButton, {
    props,
    global: { plugins: [router] },
  })
  return { wrapper, router }
}

beforeEach(() => {
  // BackButton 经 useLocaleStore 取 t：spec 显式建 Pinia 并锁定中文，
  // 避免文案断言随环境语言漂移（AuthPanel.spec.ts 同款约定）。
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('BackButton：统一返回按钮', () => {
  it('默认渲染纯图标箭头 + title「返回」', async () => {
    const { wrapper } = await mountButton()
    expect(wrapper.find('svg').exists()).toBe(true)
    expect(wrapper.find('.back-btn-label').exists()).toBe(false)
    expect(wrapper.attributes('title')).toBe('返回')
  })

  it('传 label 渲染文字（如「返回」），title 缺省为「返回上一页」', async () => {
    const { wrapper } = await mountButton({ label: '返回' })
    expect(wrapper.find('.back-btn-label').text()).toBe('返回')
    expect(wrapper.attributes('title')).toBe('返回上一页')
  })

  it('点击：有会话历史 → router.back()', async () => {
    window.history.replaceState({ ...window.history.state, back: '/projects' }, '')
    const { wrapper, router } = await mountButton()
    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    await wrapper.trigger('click')
    expect(backSpy).toHaveBeenCalledTimes(1)
  })

  it('点击：深链直开 → push fallback（默认 /projects）', async () => {
    window.history.replaceState({ ...window.history.state, back: null }, '')
    const { wrapper, router } = await mountButton()
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    await wrapper.trigger('click')
    expect(pushSpy).toHaveBeenCalledWith(NAV_HOME)
  })

  it('点击：深链直开 + 自定义 fallback → push 该目标', async () => {
    window.history.replaceState({ ...window.history.state, back: null }, '')
    const { wrapper, router } = await mountButton({ fallback: '/plugins' })
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    await wrapper.trigger('click')
    expect(pushSpy).toHaveBeenCalledWith('/plugins')
  })

  it('onBack 守卫返回 false → 拦截默认返回', async () => {
    window.history.replaceState({ ...window.history.state, back: '/projects' }, '')
    const onBack = vi.fn(() => false)
    const { wrapper, router } = await mountButton({ onBack })
    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    await wrapper.trigger('click')
    expect(onBack).toHaveBeenCalledTimes(1)
    expect(backSpy).not.toHaveBeenCalled()
  })

  it('onBack 放行（返回 undefined）→ 执行默认返回', async () => {
    window.history.replaceState({ ...window.history.state, back: '/projects' }, '')
    const onBack = vi.fn(() => undefined)
    const { wrapper, router } = await mountButton({ onBack })
    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    await wrapper.trigger('click')
    expect(backSpy).toHaveBeenCalledTimes(1)
  })

  it('emit click 事件（页面可感知点击）', async () => {
    const { wrapper } = await mountButton()
    await wrapper.trigger('click')
    expect(wrapper.emitted('click')).toHaveLength(1)
  })
})
