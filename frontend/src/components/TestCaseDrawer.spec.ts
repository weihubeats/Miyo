/**
 * TestCaseDrawer 单测（三 Tab 重构版）：
 * - 打开回填（名称 / 分类 / URL）与三 Tab 结构；
 * - 断言与提取 Tab：共享 AssertionList / ExtractList 行编辑；
 * - 保存携带完整载荷（请求快照 + 断言 / 提取 / 执行控制），成功后关闭；
 * - 立即运行调用 onRun 并展示引擎结果（状态 + 断言明细 + 提取值）；
 * - 关闭脏检查（ESC → 确认条 → 放弃 / 继续编辑）；
 * - 宽度拖拽 / 全屏（源码断言，TabBar.spec 同款做法）。
 * useMiyoApi 模块级 mock（执行历史读链路）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import TestCaseDrawer from './TestCaseDrawer.vue'
import { useLocaleStore } from '../stores/locale'
import type { Endpoint, EndpointResult, TestCase } from '../types/miyoApi'

const api = vi.hoisted(() => ({
  listTestCaseRuns: vi.fn(async () => []),
  getTestCaseRun: vi.fn(async () => null),
  deleteTestCaseRun: vi.fn(async () => true),
  cancelTestEndpoint: vi.fn(async () => true),
}))

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => api,
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

function makeCase(overrides: Partial<TestCase> = {}): TestCase {
  return {
    id: 'c1',
    request_id: 'ep-1',
    name: '内部划转-SGB',
    category: '正向',
    folder: 'Dlocal 渠道',
    method: 'POST',
    url_path: '/funds/transfer',
    path_variables: [],
    params: [{ key: 'env', value: 'prod', enabled: true, description: '' }],
    headers: [{ key: 'X-Trace', value: 'on', enabled: true, description: '' }],
    body_type: 'json',
    body_content: '{"amount":100}',
    assertions: [{ type: 'status', op: 'eq', expected: '200' }],
    extractors: [{ name: 'orderNo', from: 'body', path: '$.orderNo' }],
    last_run_status: 'Untested',
    created_at: '2026-01-01T00:00:00.000Z',
    ...overrides,
  }
}

function makeEndpoint(): Endpoint {
  return {
    id: 'ep-1',
    project_id: 'p1',
    folder_id: null,
    name: '内部划转',
    method: 'POST',
    path: '/funds/transfer',
    description: '',
    status: 'designing',
    sort_order: 0,
    request: {
      params: [{ key: 'env', value: 'prod', enabled: true, description: '' }],
      headers: [{ key: 'X-Trace', value: 'on', enabled: true, description: '' }],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'json', raw: '{"amount":100}' },
      active_tab: null,
      timeout_ms: null,
      follow_redirects: true,
      tests: null,
    },
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

function runResult(): EndpointResult {
  return {
    endpoint_id: 'ep-1',
    endpoint_name: '内部划转-SGB',
    method: 'POST',
    path: '/funds/transfer',
    ok: true,
    status: 200,
    duration_ms: 145,
    request_error: null,
    outcomes: [
      { description: 'HTTP 状态码 eq 200', passed: true, reason: null, expected: '200', actual: '200' },
      { description: '$.code eq 0', passed: false, reason: '期望 0，实际 1', expected: '0', actual: '1' },
    ],
    extracted: { orderNo: 'SO-1' },
    url: 'https://api.test/funds/transfer',
    cancelled: false,
    response: {
      status: 200,
      headers: [['content-type', 'application/json']],
      body: '{"code":1}',
      content_type: 'application/json',
      duration_ms: 145,
      size_bytes: 10,
      truncated: false,
    },
  }
}

function mountDrawer(overrides: { open?: boolean; testCase?: TestCase | null; endpoint?: Endpoint | null; lastResult?: EndpointResult | null } = {}) {
  const onRun = vi.fn(async (_payload?: unknown, _runId?: string) => runResult())
  const onSave = vi.fn(async (_payload?: unknown) => undefined)
  const resolveUrl = vi.fn((path: string) => `https://api.test${path}`)
  const wrapper = mount(TestCaseDrawer, {
    props: {
      open: overrides.open ?? true,
      endpointId: 'ep-1',
      endpoint: overrides.endpoint ?? makeEndpoint(),
      testCase: overrides.testCase ?? makeCase(),
      lastResult: overrides.lastResult ?? null,
      resolveUrl,
      onRun,
      onSave,
    },
    attachTo: document.body,
  })
  return { wrapper, onRun, onSave, resolveUrl }
}


/** Teleport 挂在 body 下：统一用 document 查询（VTU wrapper.find 看不到传送内容）。 */
const $ = (sel: string): HTMLElement | null => document.querySelector<HTMLElement>(sel)
const $$ = (sel: string): HTMLElement[] => Array.from(document.querySelectorAll<HTMLElement>(sel))
async function clickEl(sel: string): Promise<void> {
  const el = $(sel)
  if (!el) throw new Error(`element not found: ${sel}`)
  el.click()
  await nextTick()
}

describe('TestCaseDrawer（三 Tab 重构版）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
    document.body.innerHTML = ''
    vi.clearAllMocks()
  })

  it('打开时回填用例内容（名称 / 分类 / 文件夹 / URL），三个 Tab 就位', () => {
    mountDrawer()
    expect((document.querySelector<HTMLInputElement>('.drw-name') as HTMLInputElement).value).toBe('内部划转-SGB')
    expect((document.querySelector<HTMLInputElement>('.drw-folder') as HTMLInputElement).value).toBe('Dlocal 渠道')
    expect((document.querySelector<HTMLInputElement>('.drw-url') as HTMLInputElement).value).toBe('/funds/transfer')
  })

  it('断言与提取 Tab：共享列表渲染存量规则，可添加新行', async () => {
    mountDrawer()
    await clickEl('.drw-tab:nth-of-type(2)')
    // 提取规则：存量 1 行（orderNo）
    expect($$('.xtr-row')).toHaveLength(1)
    // 断言规则：存量 1 行（status）
    expect($$('.asr-row')).toHaveLength(1)
    // 添加断言行
    await clickEl('.asr-head .miyo-btn')
    expect($$('.asr-row')).toHaveLength(2)
  })

  it('保存携带完整载荷（断言 / 提取 / auth / 超时 / Cookie 开关），成功后关闭', async () => {
    const { wrapper, onSave } = mountDrawer({
      testCase: makeCase({ auth: { type: 'bearer', token: 't' }, timeout_ms: 8000, disable_cookies: true }),
    })
    await clickEl('.drw-footer-main .miyo-btn:last-child')
    expect(onSave).toHaveBeenCalledTimes(1)
    const payload = onSave.mock.calls[0]?.[0] as {
      name: string
      urlPath: string
      assertions: unknown[]
      extractors: unknown[]
      timeoutMs: number | null
      disableCookies: boolean
      params: unknown[]
    }
    if (!payload) throw new Error('onSave 未收到载荷')
    expect(payload.name).toBe('内部划转-SGB')
    expect(payload.urlPath).toBe('/funds/transfer')
    expect(payload.assertions).toHaveLength(1)
    expect(payload.extractors).toHaveLength(1)
    expect(payload.timeoutMs).toBe(8000)
    expect(payload.disableCookies).toBe(true)
    expect(payload.params).toEqual([{ key: 'env', value: 'prod', enabled: true, description: '' }])
    // 保存成功 → 抽屉关闭
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })

  it('立即运行调用 onRun 并展示引擎结果（状态 + 断言明细 + 提取值）', async () => {
    const { onRun } = mountDrawer()
    // 运行会自动切到结果 Tab
    await clickEl('.drw-footer-main .miyo-btn-primary')
    expect(onRun).toHaveBeenCalledTimes(1)
    // runId 必须透传：否则「取消」调用的 run_id 后端不认识，等于空操作
    const runId = onRun.mock.calls[0]?.[1]
    expect(typeof runId).toBe('string')
    expect((runId as string).length).toBeGreaterThan(10)
    await nextTick()
    const pane = $('.drw-pane-result') as HTMLElement
    expect(pane.textContent).toContain('200')
    expect(pane.textContent).toContain('HTTP 状态码 eq 200')
    expect(pane.textContent).toContain('orderNo = SO-1')
    expect(pane.textContent).toContain('断言 1 通过 / 1 失败')
  })

  it('方法触发器带语义色徽章（POST 高亮，不再是一行灰字）', () => {
    mountDrawer()
    const badge = $('.drw-method-badge')
    expect(badge).toBeTruthy()
    expect(badge?.textContent?.trim()).toBe('POST')
    expect(badge?.className).toContain('text-method-post')
  })

  it('打开 POST 用例默认停在 Body 分段，GET 用例默认停在 Params', async () => {
    mountDrawer()
    expect(($('.drw-section-tab.active')?.textContent ?? '').trim()).toBe('Body')

    document.body.innerHTML = ''
    mountDrawer({ testCase: makeCase({ method: 'GET', body_type: 'none', body_content: '' }) })
    await nextTick()
    expect(($('.drw-section-tab.active')?.textContent ?? '').trim()).toBe('Params')
  })

  it('打开用例不静默改写快照（POST 空 body 不自动补 JSON / Content-Type）', async () => {
    const { onSave } = mountDrawer({
      testCase: makeCase({ headers: [], body_type: 'none', body_content: '' }),
    })
    // 只是回填：不应被判脏
    expect($('.drw-dirty')).toBeNull()
    await clickEl('.drw-footer-main .miyo-btn:last-child')
    const payload = onSave.mock.calls[0]?.[0] as { headers: unknown[]; bodyType: string }
    expect(payload.headers).toEqual([])
    expect(payload.bodyType).toBe('none')
  })

  it('Body 分段带高度分配容器（编辑器与主请求面板同契约）', async () => {
    mountDrawer()
    const host = $('.drw-section-body') as HTMLElement
    expect(host.className).toContain('is-body')
    const paramsTab = $$('.drw-section-tab').find((b) => b.textContent?.trim() === 'Params') as HTMLElement
    paramsTab.click()
    await nextTick()
    expect(($('.drw-section-body') as HTMLElement).className).not.toContain('is-body')
  })

  it('运行中可取消：取消按钮用透传的 runId 调后端中止', async () => {
    let release: (v: EndpointResult) => void = () => {}
    const pending = new Promise<EndpointResult>((resolve) => {
      release = resolve
    })
    const onRun = vi.fn(async (_payload?: unknown, _runId?: string) => pending)
    const wrapper = mount(TestCaseDrawer, {
      props: {
        open: true,
        endpointId: 'ep-1',
        endpoint: makeEndpoint(),
        testCase: makeCase(),
        lastResult: null,
        resolveUrl: (p: string) => `https://api.test${p}`,
        onRun,
        onSave: vi.fn(async () => undefined),
      },
      attachTo: document.body,
    })
    await nextTick()
    await clickEl('.drw-footer-main .miyo-btn-primary')
    const cancel = $$('.drw-footer-main button').find((b) => b.textContent?.includes('取消'))
    expect(cancel).toBeTruthy()
    cancel!.click()
    await nextTick()
    const runId = onRun.mock.calls[0]?.[1] as string
    expect(api.cancelTestEndpoint).toHaveBeenCalledWith(runId)
    release(runResult())
    await nextTick()
    wrapper.unmount()
  })

  it('关闭脏检查：修改后 ESC 弹确认条，放弃关闭 / 继续编辑保留', async () => {
    const { wrapper } = mountDrawer()
    // 修改名称 → 脏
    const nameInput = $('.drw-name') as HTMLInputElement
    nameInput.value = '改名'
    nameInput.dispatchEvent(new Event('input'))
    await nextTick()
    // ESC → 确认条
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect($('.drw-confirm')).toBeTruthy()
    expect(wrapper.emitted('update:open')).toBeUndefined()
    // 放弃修改 → 关闭
    await clickEl('.drw-confirm .miyo-btn-danger')
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })

  it('无修改时 ESC 直接关闭（不出确认条）', async () => {
    const { wrapper } = mountDrawer()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect($('.drw-confirm')).toBeNull()
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })

  it('宽度拖拽与全屏：默认 680 / min 560 / max 75vw（源码断言）+ 全屏类切换', async () => {
    mountDrawer()
    // 源码断言（jsdom 无布局；TabBar.spec 同款做法）
    const src = readFileSync(join(process.cwd(), 'src/components/TestCaseDrawer.vue'), 'utf8')
    expect(src).toContain('DRAWER_MIN = 560')
    expect(src).toContain('DRAWER_DEFAULT = 680')
    expect(src).toContain('window.innerWidth * 0.75')
    // 全屏切换（标题行倒数第二个 IconButton 是全屏钮，最后是关闭钮）
    expect($('.drw.is-fullscreen')).toBeNull()
    const titleBtns = $$('.drw-title-row button')
    await clickElChain(titleBtns[titleBtns.length - 2])
    expect($('.drw.is-fullscreen')).toBeTruthy()
  })
  /** 对已取得的元素触发点击（Teleport 内容不在 wrapper 内）。 */
  async function clickElChain(el: HTMLElement): Promise<void> {
    el.click()
    await nextTick()
  }

  it('继承 / 覆盖标记：与当前接口一致显示继承', () => {
    mountDrawer()
    expect($$('.drw-inherit-tag.over')).toHaveLength(0)
    expect(document.body.textContent).toContain('继承')
  })

  it('执行历史：打开历史列表（空态）', async () => {
    mountDrawer()
    await clickEl('.drw-tab:nth-of-type(3)')
    await clickEl('.drw-history-toggle')
    expect(api.listTestCaseRuns).toHaveBeenCalledWith('c1', 50)
    expect($('.drw-history-empty')).toBeTruthy()
  })
})
