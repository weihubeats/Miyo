/**
 * ReleaseNotesModal 单测：独立更新详情弹窗。
 * 锁定：打开接管暂存（标题 + Markdown 日志渲染）/ 稍后再说放回暂存 /
 * 下载进度与下载中禁关（Esc 被守卫拦截）/ 成功自动关闭 + 延迟重启 /
 * 浅色主题适配（主题化包裹框 + miyo-btn 按钮族，无恒深底覆写）。
 * 注：本地 flush 走纯微任务（VTU flushPromises 走 setTimeout，与假时钟组合会挂起）。
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { Update } from '@tauri-apps/plugin-updater'
import ReleaseNotesModal from '@/domains/workspace/components/ReleaseNotesModal.vue'
import {
  hasPendingUpdate,
  pendingUpdateVersion,
  restorePendingUpdate,
  takePendingUpdate,
} from '@/domains/workspace/composables/useAutoUpdate'
import { useLocaleStore } from '@/stores/locale'

const mocks = vi.hoisted(() => ({
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

vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: mocks.relaunch }))
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))
vi.mock('@/composables/useToast.ts', () => ({ useToast: () => mocks.toast }))

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
  await nextTick()
}

const NOTES = '## 更新内容\n\n- 修复 **崩溃**\n- 优化启动速度'

function fakeUpdate(downloadAndInstall: ReturnType<typeof vi.fn>, body = NOTES): Update {
  return {
    available: true,
    version: '1.2.3',
    body,
    close: vi.fn(),
    downloadAndInstall,
  } as unknown as Update
}

/** Modal 走 Teleport 到 body，须从 document 层查询。 */
function dialog(): Element | null {
  return document.body.querySelector('.m-dialog')
}

function actionButtons(): HTMLElement[] {
  return Array.from(dialog()?.querySelectorAll('.rn-actions button') ?? []) as HTMLElement[]
}

/** 挂载（默认关闭）→ 打开，等接管暂存落定。 */
async function openModal(update: Update): Promise<ReturnType<typeof mount>> {
  restorePendingUpdate(update)
  const wrapper = mount(ReleaseNotesModal, { props: { open: false }, attachTo: document.body })
  await wrapper.setProps({ open: true })
  await flush()
  return wrapper
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  vi.clearAllMocks()
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

describe('ReleaseNotesModal：更新详情弹窗', () => {
  it('打开接管暂存：品牌紫标题 + Markdown 日志渲染，暂存被取走', async () => {
    const wrapper = await openModal(fakeUpdate(vi.fn(() => Promise.resolve())))

    const title = dialog()?.querySelector('.m-title')?.textContent ?? ''
    expect(title).toContain('🚀')
    expect(title).toContain('发现新版本 v1.2.3')
    expect(hasPendingUpdate.value).toBe(false)
    expect(pendingUpdateVersion()).toBeNull()

    const notes = dialog()?.querySelector('.rn-notes')
    expect(notes?.querySelector('h4')?.textContent).toBe('更新内容')
    expect(notes?.innerHTML).toContain('<strong>崩溃</strong>')
    expect(notes?.querySelectorAll('li').length).toBe(2)

    wrapper.unmount()
  })

  it('更新说明为空时展示兜底文案', async () => {
    const wrapper = await openModal(fakeUpdate(vi.fn(), ''))

    expect(dialog()?.textContent).toContain('此版本暂无更新说明')

    wrapper.unmount()
  })

  it('稍后再说：emit 关闭且放回暂存（提醒入口复亮）', async () => {
    const wrapper = await openModal(fakeUpdate(vi.fn()))

    const later = actionButtons()[0]
    expect(later.textContent).toContain('稍后再说')
    later.click()
    await flush()
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])

    // 真实链路由 v-model 把 open 置 false → watch 放回暂存
    await wrapper.setProps({ open: false })
    await flush()
    expect(hasPendingUpdate.value).toBe(true)
    expect(pendingUpdateVersion()).toBe('1.2.3')

    wrapper.unmount()
  })

  it('立即下载并重启：进度条落定 → 成功自动关闭并延迟重启；下载中禁关', async () => {
    let resolveDownload!: () => void
    const dai = vi.fn((cb: (e: unknown) => void) => {
      cb({ event: 'Started', data: { contentLength: 10_000_000 } })
      vi.advanceTimersByTime(1000)
      cb({ event: 'Progress', data: { chunkLength: 5_000_000 } })
      return new Promise<void>((r) => {
        resolveDownload = r
      })
    })
    const wrapper = await openModal(fakeUpdate(dai))

    actionButtons()[1].click()
    await flush()

    const bar = dialog()!.querySelector('.rn-progress')
    expect(bar?.getAttribute('role')).toBe('progressbar')
    expect(bar?.getAttribute('aria-valuenow')).toBe('50')
    expect(dialog()?.textContent).toContain('50%')

    // 下载中：Esc 被 guardClose 拦截，不产生关闭事件
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await flush()
    expect(wrapper.emitted('update:open')).toBeUndefined()

    resolveDownload()
    await flush()
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
    expect(mocks.toast.success).toHaveBeenCalledWith('更新已安装，即将重启')
    expect(hasPendingUpdate.value).toBe(false)

    vi.advanceTimersByTime(800)
    await flush()
    expect(mocks.relaunch).toHaveBeenCalledTimes(1)

    wrapper.unmount()
  })

  it('下载失败：就地展示错误，主按钮恢复可点即重试', async () => {
    const dai = vi
      .fn()
      .mockRejectedValueOnce(new Error('network down'))
      .mockResolvedValue(undefined)
    const wrapper = await openModal(fakeUpdate(dai))

    actionButtons()[1].click()
    await flush()

    expect(dialog()?.querySelector('.rn-error')?.textContent).toContain(
      '下载失败：network down',
    )
    expect(actionButtons()[1].hasAttribute('disabled')).toBe(false)

    // 再点主按钮 → 重试成功（无下载事件也走完成功链）
    actionButtons()[1].click()
    await flush()
    expect(dai).toHaveBeenCalledTimes(2)
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])

    wrapper.unmount()
  })

  it('浅色主题适配：日志框主题化，按钮走 miyo-btn 族，源码无恒深底覆写与写死深色类', async () => {
    const wrapper = await openModal(fakeUpdate(vi.fn()))

    // 日志包裹框走主题化类（--bg-code 井底），不带写死的深色字面量类
    const box = dialog()?.querySelector('.rn-notes-box')
    expect(box).toBeTruthy()
    expect(box!.className).not.toContain('bg-zinc-950/80')
    expect(box!.className).not.toContain('border-white/5')
    expect(box!.className).not.toContain('text-zinc-300')
    // 两个按钮走仓规按钮族：次要 miyo-btn + 主 miyo-btn-primary（统一高度/配色/禁用态）
    expect(actionButtons()[0].classList.contains('miyo-btn')).toBe(true)
    expect(actionButtons()[0].classList.contains('miyo-btn-primary')).toBe(false)
    expect(actionButtons()[1].classList.contains('miyo-btn-primary')).toBe(true)

    wrapper.unmount()

    // 组件源：非 scoped 段不再强制恒深底（文本/底色 token 覆写已除，随主题）；
    // 主按钮不再硬编码紫与深紫阴影（浅色下糊成脏影）
    const src = readFileSync(join(process.cwd(), 'src/domains/workspace/components/ReleaseNotesModal.vue'), 'utf8')
    expect(src).not.toContain('--text-1: #fafafa')
    expect(src).not.toContain('--bg-elevated: rgba(24, 24, 27')
    expect(src).not.toContain('bg-zinc-950/80')
    expect(src).not.toContain('hover:text-white')
    expect(src).not.toContain('bg-purple-600')
    expect(src).not.toContain('shadow-purple-950')
  })
})
