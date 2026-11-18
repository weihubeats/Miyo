/**
 * useGoBack 单测：统一返回策略（第四阶段）分支覆盖。
 * 锁定回归：有会话历史 → router.back()；深链直开 → fallback push；
 * 不传 fallback 时兜底 /projects（不会回退到应用外）。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createRouter, createMemoryHistory, type Router } from 'vue-router'
import { defineComponent, h } from 'vue'
import { useGoBack, hasInAppHistory, NAV_HOME } from '@/composables/useGoBack'

/** 挂载一个调用 useGoBack 的测试宿主组件，返回宿主内的 goBack 引用。 */
function mountHost(router: Router): (fallback?: string) => void {
  let captured: ((fallback?: string) => void) | null = null
  const Host = defineComponent({
    setup() {
      captured = useGoBack()
      return () => h('div')
    },
  })
  mount(Host, { global: { plugins: [router] } })
  return captured ?? (() => {})
}

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/projects', component: { template: '<div/>' } },
      { path: '/plugins', component: { template: '<div/>' } },
      { path: '/deep-page', component: { template: '<div/>' } },
    ],
  })
}

describe('useGoBack：统一返回策略', () => {
  it('会话内有可回退历史（state.back 存在）→ router.back() 原路返回', async () => {
    const router = makeRouter()
    await router.push('/deep-page')
    await router.isReady()
    // 模拟 vue-router 写入的来源标记（仅应用内导航会写）
    window.history.replaceState({ ...window.history.state, back: '/projects' }, '')

    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)

    const goBack = mountHost(router)
    goBack()

    expect(backSpy).toHaveBeenCalledTimes(1)
    expect(pushSpy).not.toHaveBeenCalled()
  })

  it('深链直开（state.back 为 null）→ push 兜底目标（默认 /projects）', async () => {
    const router = makeRouter()
    await router.push('/deep-page')
    await router.isReady()
    window.history.replaceState({ ...window.history.state, back: null }, '')

    const backSpy = vi.spyOn(router, 'back').mockImplementation(() => {})
    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)

    const goBack = mountHost(router)
    goBack()

    expect(pushSpy).toHaveBeenCalledWith(NAV_HOME)
    expect(backSpy).not.toHaveBeenCalled()
  })

  it('指定 fallback（如插件宿主回插件中心）→ 深链时 push 该目标', async () => {
    const router = makeRouter()
    await router.push('/deep-page')
    await router.isReady()
    window.history.replaceState({ ...window.history.state, back: null }, '')

    const pushSpy = vi.spyOn(router, 'push').mockResolvedValue(undefined)
    vi.spyOn(router, 'back').mockImplementation(() => {})

    const goBack = mountHost(router)
    goBack('/plugins')

    expect(pushSpy).toHaveBeenCalledWith('/plugins')
  })

  it('hasInAppHistory：跟随 window.history.state.back 判定', () => {
    window.history.replaceState({ back: '/x' }, '')
    expect(hasInAppHistory()).toBe(true)
    window.history.replaceState({ back: null }, '')
    expect(hasInAppHistory()).toBe(false)
    window.history.replaceState({}, '')
    expect(hasInAppHistory()).toBe(false)
  })
})
