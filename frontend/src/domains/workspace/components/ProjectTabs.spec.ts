/**
 * ProjectTabs 横拖平移单测。
 *
 * 回归「重启还是拖不动」（v0.0.57 用户实测，箭头指向顶栏项目标签条）：
 * 顶栏右侧（Docs / Mock / 环境 / 工具组）按 min-width:max-content 不压缩，
 * 项目一多 `.proj-tabs` 就被压窄、后面的项目被挡住。此前只有滚轮与触控板
 * 能滚，鼠标用户没有任何可见入口——看起来就是「拖不动」。
 *
 * 修复与 TabBar 同款：默认拖 = 平移；指到关闭钮不接管；竖向滚轮转横向；
 * `-webkit-user-drag: none` 挡住 WebKit 原生 drag（否则 pointercancel 掐断平移）。
 */
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import ProjectTabs from '@/domains/workspace/components/ProjectTabs.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import type { Project } from '@/types/miyoApi'

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({ deleteProject: vi.fn(async () => {}), listProjects: vi.fn(async () => []) }),
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ path: '/workspace' }),
  useRouter: () => ({ push: vi.fn(async () => {}) }),
}))

function makeProject(id: string, name: string): Project {
  return {
    id,
    name,
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

/** jsdom 无布局：手动伪造「标签条溢出」。 */
function fakeOverflow(el: HTMLElement): void {
  Object.defineProperty(el, 'scrollWidth', { value: 2000, configurable: true })
  Object.defineProperty(el, 'clientWidth', { value: 400, configurable: true })
}

function pointer(el: HTMLElement, type: string, clientX: number, extra: PointerEventInit = {}) {
  el.dispatchEvent(
    new PointerEvent(type, { clientX, pointerId: 1, button: 0, bubbles: true, cancelable: true, ...extra }),
  )
}

function mountTabs() {
  const store = useWorkspaceStore()
  useLocaleStore().setMode('zh')
  store.project = makeProject('p1', '项目一')
  store.openProjects = [
    { id: 'p1', name: '项目一' },
    { id: 'p2', name: '项目二' },
  ]
  return mount(ProjectTabs, { attachTo: document.body })
}

function tabsEl(): HTMLElement {
  return document.querySelector('.proj-tabs') as HTMLElement
}

describe('ProjectTabs：横拖平移滚动', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('向左拖动时 scrollLeft 增大（标签跟手左移）', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    fakeOverflow(el)
    el.setPointerCapture = vi.fn()
    el.hasPointerCapture = vi.fn(() => true)
    el.releasePointerCapture = vi.fn()
    el.scrollLeft = 100

    pointer(el, 'pointerdown', 500)
    pointer(el, 'pointermove', 400) // 左移 100px

    expect(el.scrollLeft).toBe(200)
    wrapper.unmount()
  })

  it('位移小于阈值视为点击，不平移（避免误触切项目）', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    fakeOverflow(el)
    el.setPointerCapture = vi.fn()
    el.hasPointerCapture = vi.fn(() => true)
    el.releasePointerCapture = vi.fn()
    el.scrollLeft = 100

    pointer(el, 'pointerdown', 500)
    pointer(el, 'pointermove', 498) // 仅 2px

    expect(el.scrollLeft).toBe(100)
    wrapper.unmount()
  })

  it('按在关闭钮上不进入拖拽', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    fakeOverflow(el)
    el.setPointerCapture = vi.fn()
    el.scrollLeft = 100

    const closeBtn = el.querySelector('.pt-close') as HTMLElement
    closeBtn.dispatchEvent(
      new PointerEvent('pointerdown', { clientX: 500, pointerId: 1, button: 0, bubbles: true }),
    )
    pointer(el, 'pointermove', 300)

    expect(el.scrollLeft).toBe(100)
    expect(el.setPointerCapture).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('标签条未溢出时不接管拖动（保持原生行为）', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    Object.defineProperty(el, 'scrollWidth', { value: 300, configurable: true })
    Object.defineProperty(el, 'clientWidth', { value: 400, configurable: true })
    el.setPointerCapture = vi.fn()

    pointer(el, 'pointerdown', 500)
    pointer(el, 'pointermove', 300)

    expect(el.setPointerCapture).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('竖向滚轮转横向平移', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    fakeOverflow(el)
    el.scrollLeft = 100

    const wheel = new WheelEvent('wheel', { deltaY: 60, deltaX: 0, bubbles: true, cancelable: true })
    el.dispatchEvent(wheel)

    expect(el.scrollLeft).toBe(160)
    expect(wheel.defaultPrevented).toBe(true)
    wrapper.unmount()
  })

  it('触控板横向分量占优时仍走原生滚动，不劫持', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    fakeOverflow(el)
    el.scrollLeft = 100

    const wheel = new WheelEvent('wheel', { deltaY: 10, deltaX: 60, bubbles: true, cancelable: true })
    el.dispatchEvent(wheel)

    expect(el.scrollLeft).toBe(100)
    expect(wheel.defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it('拖拽后紧随的 click 被吞掉，不切换项目', () => {
    const wrapper = mountTabs()
    const el = tabsEl()
    const store = useWorkspaceStore()
    fakeOverflow(el)
    el.setPointerCapture = vi.fn()
    el.hasPointerCapture = vi.fn(() => true)
    el.releasePointerCapture = vi.fn()
    el.scrollLeft = 100

    pointer(el, 'pointerdown', 500)
    pointer(el, 'pointermove', 300)
    pointer(el, 'pointerup', 300)

    // 拖拽结束那一下 click：不应落到第二个项目上
    const second = el.querySelectorAll('.proj-tab')[1] as HTMLElement
    second.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    expect(store.project?.id).toBe('p1')
    wrapper.unmount()
  })

  it('样式禁掉 WebKit 原生拖拽（否则平移会被 pointercancel 掐断）', () => {
    const src = readFileSync(join(process.cwd(), 'src/domains/workspace/components/ProjectTabs.vue'), 'utf8')
    const style = src.slice(src.indexOf('<style'))
    expect(style).toMatch(/\.proj-tab\s*\{[\s\S]*?-webkit-user-drag:\s*none/)
  })

  it('不再靠 hover 露出细滚动条暗示可滚（改为横拖手势）', () => {
    const src = readFileSync(join(process.cwd(), 'src/domains/workspace/components/ProjectTabs.vue'), 'utf8')
    const style = src.slice(src.indexOf('<style'))
    expect(style).not.toMatch(/\.proj-tabs:hover::-webkit-scrollbar/)
  })
})