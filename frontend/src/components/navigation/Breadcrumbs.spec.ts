/**
 * Breadcrumbs 单测：面包屑导航（多级页面导航规范）。
 * 锁定回归：末级不可点（aria-current）、上级可点（to / onSelect）、
 * 超限折叠中段为省略号。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createRouter, createMemoryHistory, type Router } from 'vue-router'
import Breadcrumbs, { type BreadcrumbItem } from '@/components/navigation/Breadcrumbs.vue'
import { useLocaleStore } from '@/stores/locale'

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/integration', component: { template: '<div/>' } },
      { path: '/other', component: { template: '<div/>' } },
      { path: '/:pathMatch(.*)*', component: { template: '<div/>' } },
    ],
  })
}

async function mountCrumbs(items: BreadcrumbItem[], max?: number): Promise<{
  wrapper: ReturnType<typeof mount>
  router: Router
}> {
  const router = makeRouter()
  await router.push('/integration')
  await router.isReady()
  const wrapper = mount(Breadcrumbs, {
    props: max ? { items, max } : { items },
    global: { plugins: [router] },
  })
  return { wrapper, router }
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('Breadcrumbs：面包屑导航', () => {
  it('末级渲染为当前位置（aria-current，不可点），上级 to 可点跳转', async () => {
    const { wrapper, router } = await mountCrumbs([
      { label: '集成测试', to: '/integration' },
      { label: '场景 A' },
    ])
    const links = wrapper.findAll('.crumb-link')
    expect(links).toHaveLength(1)
    const current = wrapper.find('.crumb-current')
    expect(current.text()).toBe('场景 A')
    expect(current.attributes('aria-current')).toBe('page')
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    await links[0]!.trigger('click')
    expect(pushSpy).toHaveBeenCalledWith('/integration')
  })

  it('onSelect 层级可点（页面内动作，如编辑器态回列表）', async () => {
    const onSelect = vi.fn()
    const { wrapper } = await mountCrumbs([
      { label: '集成测试', onSelect },
      { label: '场景 A' },
    ])
    await wrapper.find('.crumb-link').trigger('click')
    expect(onSelect).toHaveBeenCalledTimes(1)
  })

  it('超限折叠：保留首层与末两层，中段并入省略号', async () => {
    const { wrapper } = await mountCrumbs(
      [
        { label: 'L0', to: '/other' },
        { label: 'L1' },
        { label: 'L2' },
        { label: 'L3' },
        { label: 'L4' },
      ],
      3,
    )
    expect(wrapper.findAll('.crumb-link')).toHaveLength(1) // 仅 L0 可点
    expect(wrapper.find('.crumb-ellipsis').exists()).toBe(true)
    expect(wrapper.find('.crumb-ellipsis').attributes('title')).toBe('L1 / L2')
    expect(wrapper.find('.crumb-current').text()).toBe('L4')
  })

  it('未超限不渲染省略号', async () => {
    const { wrapper } = await mountCrumbs([
      { label: 'L0', to: '/other' },
      { label: 'L1' },
    ])
    expect(wrapper.find('.crumb-ellipsis').exists()).toBe(false)
  })
})
