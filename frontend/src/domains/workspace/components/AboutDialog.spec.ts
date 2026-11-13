/**
 * AboutDialog 单测：更新下载流程 UI。
 * 锁定：Release Notes 链接 / 进度信息（百分比·体量·速度）/ 安装中状态 /
 * 失败卡片内错误与按钮恢复重试 / 成功后延迟重启。
 * 注：本地 flush 走纯微任务（VTU flushPromises 走 setTimeout，与假时钟组合会挂起）。
 */
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { Update } from '@tauri-apps/plugin-updater'
import AboutDialog from '@/domains/workspace/components/AboutDialog.vue'
import { useLocaleStore } from '@/stores/locale'
import { collectErrors } from '@/testUtils/componentTest'

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  openUrl: vi.fn(async () => undefined),
  relaunch: vi.fn(async () => undefined),
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
    toast: vi.fn(),
    dismiss: vi.fn(),
    clearAll: vi.fn(),
  },
}))

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mocks.openUrl }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: mocks.relaunch }))
vi.mock('@/composables/useToast.ts', () => ({ useToast: () => mocks.toast }))

/** 排空微任务链 + Vue 渲染队列（不依赖被假时钟接管的 setTimeout/setImmediate）。 */
async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
  await nextTick()
}

function fakeUpdate(downloadAndInstall: ReturnType<typeof vi.fn>): Update {
  return {
    available: true,
    version: '1.2.3',
    body: '修复若干问题',
    close: vi.fn(),
    downloadAndInstall,
  } as unknown as Update
}

/** Modal 走 Teleport 到 body，须从 document 层查询。 */
function dialog(): Element | null {
  return document.body.querySelector('.m-dialog')
}

function updateCard(): Element | null {
  return dialog()?.querySelector('.a-update') ?? null
}

function cardText(): string {
  return updateCard()?.textContent ?? ''
}

/** 点「检查更新」并落定（check 已在各用例预置返回值）。 */
async function clickCheck(): Promise<void> {
  const links = dialog()?.querySelectorAll('.a-links button')
  expect(links?.length).toBe(2)
  ;(links?.[1] as HTMLElement).click()
  await flush()
}

function click(selector: string): void {
  const el = updateCard()?.querySelector(selector) as HTMLButtonElement | null
  expect(el).toBeTruthy()
  el!.click()
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  vi.clearAllMocks()
  localStorage.clear()
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('AboutDialog：更新下载流程', () => {
  it('发现新版展示卡片：Release Notes 链接指向 GitHub tag 页，底部有自动重启提示', async () => {
    mocks.check.mockResolvedValue(fakeUpdate(vi.fn(() => Promise.resolve())))
    const wrapper = mount(AboutDialog, { props: { open: true }, attachTo: document.body })
    await flush()
    await clickCheck()

    expect(updateCard()).toBeTruthy()
    expect(cardText()).toContain('发现新版本')
    expect(cardText()).toContain('v1.2.3')
    expect(cardText()).toContain('修复若干问题')
    expect(cardText()).toContain('安装完成后自动重启')

    click('.a-notes-link')
    await flush()
    expect(mocks.openUrl).toHaveBeenCalledWith(
      'https://github.com/weihubeats/Miyo/releases/tag/v1.2.3',
    )

    wrapper.unmount()
  })

  it('下载中展示百分比·体量·速度与后台下载提示；Finished 后切安装中并延迟重启', async () => {
    const collector = collectErrors()
    let emit!: (e: unknown) => void
    let resolveDownload!: () => void
    const dai = vi.fn((cb: (e: unknown) => void) => {
      emit = cb
      cb({ event: 'Started', data: { contentLength: 10_000_000 } })
      vi.advanceTimersByTime(1000)
      cb({ event: 'Progress', data: { chunkLength: 3_000_000 } })
      vi.advanceTimersByTime(1000)
      cb({ event: 'Progress', data: { chunkLength: 2_000_000 } })
      return new Promise<void>((r) => {
        resolveDownload = r
      })
    })
    mocks.check.mockResolvedValue(fakeUpdate(dai))
    const wrapper = mount(AboutDialog, { props: { open: true }, attachTo: document.body })
    await flush()
    await clickCheck()
    click('.a-update-btn')
    await flush()

    // 10MB 总量下 1s 内共下 5MB → 50%；速度样本 2MB/1s = 2.0 MB/s（EMA 首样本直取）
    expect(cardText()).toContain('50%')
    expect(cardText()).toContain('5.0 / 10.0 MB')
    expect(cardText()).toContain('2.0 MB/s')
    expect(cardText()).toContain('关闭弹窗将在后台继续下载')

    const bar = updateCard()!.querySelector('.a-update-progress')
    expect(bar?.getAttribute('role')).toBe('progressbar')
    expect(bar?.getAttribute('aria-valuemin')).toBe('0')
    expect(bar?.getAttribute('aria-valuemax')).toBe('100')
    expect(bar?.getAttribute('aria-valuenow')).toBe('50')

    // Finished → 安装中（后台下载提示随安装阶段隐藏，自动重启提示仍在）
    emit({ event: 'Finished', data: {} })
    await flush()
    expect(cardText()).toContain('安装中…')
    expect(cardText()).not.toContain('关闭弹窗将在后台继续下载')
    expect(cardText()).toContain('安装完成后自动重启')
    expect(bar?.getAttribute('aria-valuetext')).toContain('安装中')

    resolveDownload()
    await flush()
    expect(updateCard()).toBeNull()
    expect(mocks.toast.success).toHaveBeenCalledWith('更新已安装，即将重启')

    vi.advanceTimersByTime(800)
    await flush()
    expect(mocks.relaunch).toHaveBeenCalledTimes(1)

    wrapper.unmount()
    collector.restore()
    expect(collector.errors).toEqual([])
  })

  it('发现新版后关闭重开，点下载仍生效（展示态随关闭清空，重开从共享暂存接管）', async () => {
    const dai = vi.fn(() => Promise.resolve())
    mocks.check.mockResolvedValue(fakeUpdate(dai))
    const wrapper = mount(AboutDialog, { props: { open: true }, attachTo: document.body })
    await flush()
    await clickCheck()
    expect(updateCard()).toBeTruthy()

    // 关闭：Update 放回共享暂存，本地展示态（pendingVersion）必须一并清空
    await wrapper.setProps({ open: false })
    await flush()
    expect(updateCard()).toBeNull()

    // 重开：takePendingUpdate 重新接管 → 卡片与 Update 对象同时在
    await wrapper.setProps({ open: true })
    await flush()
    expect(updateCard()).toBeTruthy()
    expect(cardText()).toContain('v1.2.3')

    // 点下载必须真正发起（回归：修复前 pending 为 null，此处静默 return）
    click('.a-update-btn')
    await flush()
    expect(dai).toHaveBeenCalledTimes(1)

    wrapper.unmount()
  })

  it('下载失败：卡片内就地展示错误，按钮恢复可点即重试', async () => {
    const dai = vi
      .fn()
      .mockRejectedValueOnce(new Error('network down'))
      .mockResolvedValue(undefined)
    mocks.check.mockResolvedValue(fakeUpdate(dai))
    const wrapper = mount(AboutDialog, { props: { open: true }, attachTo: document.body })
    await flush()
    await clickCheck()

    click('.a-update-btn')
    await flush()

    // 就地错误 + 安装按钮恢复（= 卡片内一键重试）
    const err = updateCard()!.querySelector('.a-update-error')
    expect(err?.textContent).toContain('下载失败：network down')
    expect(updateCard()!.querySelector('.a-update-btn')).toBeTruthy()
    expect(mocks.toast.error).toHaveBeenCalledWith(
      '下载更新失败',
      expect.objectContaining({
        message: 'network down',
        action: expect.objectContaining({ label: '重试' }),
      }),
    )

    // 再点安装 → 重试成功（无下载事件也走完成功链）
    click('.a-update-btn')
    await flush()
    expect(dai).toHaveBeenCalledTimes(2)
    expect(updateCard()).toBeNull()
    expect(mocks.toast.success).toHaveBeenCalledTimes(1)

    vi.advanceTimersByTime(800)
    await flush()
    expect(mocks.relaunch).toHaveBeenCalledTimes(1)

    wrapper.unmount()
  })
})
