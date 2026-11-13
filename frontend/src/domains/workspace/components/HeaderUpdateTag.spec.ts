/**
 * HeaderUpdateTag 单测：顶部更新入口的悬停预览卡片。
 * 锁定：无待装更新不渲染触发区 / 悬停延迟展示本版日志（标题 + Markdown 渲染）/
 * 移开延迟收起（移入卡片可保持）/ 点击收起预览并请求打开详情弹窗。
 * 注：卡片 Teleport 到 body，须从 document 层查询。
 */
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { Update } from '@tauri-apps/plugin-updater'
import HeaderUpdateTag from '@/domains/workspace/components/HeaderUpdateTag.vue'
import {
  openReleaseSignal,
  restorePendingUpdate,
  takePendingUpdate,
} from '@/domains/workspace/composables/useAutoUpdate'
import { useLocaleStore } from '@/stores/locale'

vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))

/** 与组件内部宽限一致（延迟出现 / 延迟收起）。 */
const SHOW_MS = 180
const HIDE_MS = 120

const NOTES = '## 更新内容\n\n- 修复 **崩溃**\n- 优化启动速度'

function fakeUpdate(): Update {
  return {
    available: true,
    version: '1.2.3',
    date: '2026-10-16T09:51:31Z',
    body: NOTES,
    close: vi.fn(),
    downloadAndInstall: vi.fn(),
  } as unknown as Update
}

function card(): HTMLElement | null {
  return document.body.querySelector('.miyo-update-card')
}

/** 挂载并悬停触发区，推进到卡片出齐。 */
async function hoverOpen(): Promise<ReturnType<typeof mount>> {
  const wrapper = mount(HeaderUpdateTag, { attachTo: document.body })
  await wrapper.find('.miyo-update-tag').trigger('mouseenter')
  vi.advanceTimersByTime(SHOW_MS)
  await nextTick()
  return wrapper
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  localStorage.clear()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  // 清掉上个用例残留的模块级暂存（useAutoUpdate 是文件级共享状态）
  takePendingUpdate()?.close()
})

afterEach(() => {
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('HeaderUpdateTag：悬停展示本版更新内容', () => {
  it('无待装更新时不渲染触发区与卡片', () => {
    const wrapper = mount(HeaderUpdateTag, { attachTo: document.body })

    expect(wrapper.find('.miyo-update-tag').exists()).toBe(false)
    expect(card()).toBeNull()

    wrapper.unmount()
  })

  it('悬停延迟展示卡片：版本标题 + Markdown 日志渲染', async () => {
    restorePendingUpdate(fakeUpdate())
    const wrapper = mount(HeaderUpdateTag, { attachTo: document.body })

    expect(wrapper.find('.miyo-update-tag').text()).toContain('更新 v1.2.3')
    await wrapper.find('.miyo-update-tag').trigger('mouseenter')
    // 延迟未到不出现（划过不误触）
    expect(card()).toBeNull()
    vi.advanceTimersByTime(SHOW_MS)
    await nextTick()

    const el = card()
    expect(el?.textContent).toContain('发现新版本 v1.2.3')
    expect(el?.textContent).toContain('2026')
    const notes = el?.querySelector('.rn-notes')
    expect(notes?.querySelector('h4')?.textContent).toBe('更新内容')
    expect(notes?.innerHTML).toContain('<strong>崩溃</strong>')
    expect(notes?.querySelectorAll('li').length).toBe(2)

    wrapper.unmount()
  })

  it('移开延迟收起；移入卡片则保持展开', async () => {
    restorePendingUpdate(fakeUpdate())
    const wrapper = await hoverOpen()

    await wrapper.find('.miyo-update-tag').trigger('mouseleave')
    expect(card()).toBeTruthy()
    vi.advanceTimersByTime(HIDE_MS)
    await nextTick()
    expect(card()).toBeNull()

    // 再次悬停 → 移入卡片：收起被撤销，日志可继续阅读
    // （真实事件序：按钮先 mouseleave 排队收起，卡片 mouseenter 随后撤销）
    await wrapper.find('.miyo-update-tag').trigger('mouseenter')
    vi.advanceTimersByTime(SHOW_MS)
    await nextTick()
    await wrapper.find('.miyo-update-tag').trigger('mouseleave')
    card()!.dispatchEvent(new MouseEvent('mouseenter'))
    vi.advanceTimersByTime(HIDE_MS)
    await nextTick()
    expect(card()).toBeTruthy()

    // 离开卡片 → 收起
    card()!.dispatchEvent(new MouseEvent('mouseleave'))
    vi.advanceTimersByTime(HIDE_MS)
    await nextTick()
    expect(card()).toBeNull()

    wrapper.unmount()
  })

  it('点击：收起预览并请求打开详情弹窗', async () => {
    restorePendingUpdate(fakeUpdate())
    const wrapper = await hoverOpen()

    const before = openReleaseSignal.value
    await wrapper.find('.miyo-update-tag').trigger('click')

    expect(openReleaseSignal.value).toBe(before + 1)
    expect(card()).toBeNull()

    wrapper.unmount()
  })
})
