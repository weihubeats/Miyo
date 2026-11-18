/**
 * PageHeader 单测：统一页面顶栏的骨架与行为。
 * 锁定回归：Brand 标题 / 返回钮 / 默认与 actions slot / hideBack / @back 接管。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createRouter, createMemoryHistory, type Router } from 'vue-router'
import PageHeader from '@/components/navigation/PageHeader.vue'
import { useLocaleStore } from '@/stores/locale'

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/projects', component: { template: '<div/>' } },
      { path: '/:pathMatch(.*)*', component: { template: '<div/>' } },
    ],
  })
}

/** PageHeader props 形状（供 mountHeader 精确类型）。 */
type HeaderProps = {
  title: string
  subtitle?: string
  hideBack?: boolean
  hideBrand?: boolean
  backLabel?: string
  backTitle?: string
  backFallback?: string
  onBack?: () => void
}

async function mountHeader(
  props: HeaderProps = { title: 'T' },
  slots: { default?: string; actions?: string } = {},
): Promise<{
  wrapper: ReturnType<typeof mount>
  router: Router
}> {
  const router = makeRouter()
  await router.push('/some-page')
  await router.isReady()
  const wrapper = mount(PageHeader, { props, slots, global: { plugins: [router] } })
  return { wrapper, router }
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('PageHeader：统一页面顶栏', () => {
  it('渲染 Brand 标题 + 统一返回钮（返回 / 返回上一页提示）', async () => {
    const { wrapper } = await mountHeader({ title: '插件中心' })
    expect(wrapper.find('.brand-title').text()).toBe('插件中心')
    const back = wrapper.find('.back-btn')
    expect(back.exists()).toBe(true)
    expect(back.find('.back-btn-label').text()).toBe('返回')
    expect(back.attributes('title')).toBe('返回上一页')
  })

  it('默认与 actions slot 渲染，spacer 存在（顶栏骨架完整）', async () => {
    const { wrapper } = await mountHeader(
      { title: 'T' },
      {
        default: '<div class="ctx">上下文</div>',
        actions: '<button class="act">操作</button>',
      },
    )
    expect(wrapper.find('.ctx').exists()).toBe(true)
    expect(wrapper.find('.act').exists()).toBe(true)
    expect(wrapper.find('.miyo-topbar-spacer').exists()).toBe(true)
    expect(wrapper.find('.miyo-topbar').exists()).toBe(true)
  })

  it('hideBack → 不渲染返回钮（根页面形态）', async () => {
    const { wrapper } = await mountHeader({ title: 'T', hideBack: true })
    expect(wrapper.find('.back-btn').exists()).toBe(false)
  })

  it('hideBrand → 不渲染 Brand（弹出子窗形态）', async () => {
    const { wrapper } = await mountHeader({ title: 'T', hideBrand: true })
    expect(wrapper.find('.brand').exists()).toBe(false)
  })

  it('监听 @back → 完全接管返回（默认导航不再触发）', async () => {
    const onBack = vi.fn()
    const { wrapper, router } = await mountHeader(
      { title: 'T', onBack },
    )
    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    await wrapper.find('.back-btn').trigger('click')
    expect(wrapper.emitted('back')).toHaveLength(1)
    expect(onBack).toHaveBeenCalledTimes(1)
    expect(backSpy).not.toHaveBeenCalled()
    expect(pushSpy).not.toHaveBeenCalled()
  })

  it('未监听 @back → BackButton 走统一默认策略（深链 → push 兜底）', async () => {
    window.history.replaceState({ ...window.history.state, back: null }, '')
    const { wrapper, router } = await mountHeader({ title: 'T' })
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    await wrapper.find('.back-btn').trigger('click')
    expect(pushSpy).toHaveBeenCalledWith('/projects')
  })

  it('backLabel 传空串 → 纯图标返回钮', async () => {
    const { wrapper } = await mountHeader({ title: 'T', backLabel: '' })
    expect(wrapper.find('.back-btn-label').exists()).toBe(false)
  })
})
