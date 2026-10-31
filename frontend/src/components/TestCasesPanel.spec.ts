/**
 * TestCasesPanel 单测：分类筛选计数 / 行点击回填 + 切调试页 / 菜单动作接线。
 * store 与 useToast 模块级 mock（无需 Pinia）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import TestCasesPanel from './TestCasesPanel.vue'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import type { Endpoint, TestCase } from '../types/miyoApi'

const store = vi.hoisted(() => ({
  map: new Map<string, TestCase[]>(),
  meta: new Map<string, { status: number; durationMs: number }>(),
  get testCases() {
    return store.map
  },
  get caseRunMeta() {
    return store.meta
  },
  set: (entries: [string, TestCase[]][]) => {
    store.map.clear()
    for (const [k, v] of entries) store.map.set(k, v)
  },
  activeEnvId: null as string | null,
  /** EnvBadge（运行环境徽章）读取；面板 spec mock 的 store 需自带。 */
  environments: [] as Array<{ id: string; name: string; base_url: string }>,
  runTestCase: vi.fn(),
  runAllTestCases: vi.fn(),
  applyTestCaseToDraft: vi.fn(),
  openTestCaseInDebug: vi.fn(),
  updateTestCaseContent: vi.fn(),
  resolveTestCaseUrl: vi.fn((path: string) => path),
  saveTestCase: vi.fn(),
  renameTestCase: vi.fn(),
  cloneTestCase: vi.fn(),
  removeTestCase: vi.fn(),
}))

vi.mock('../stores/workspace', () => ({
  useWorkspaceStore: () => store,
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

function draft(): Endpoint {
  return reactive(makeDraft({ id: 'ep-1', method: 'POST', path: '/funds/transfer' }))
}

function makeCase(id: string, name: string, category: TestCase['category'], bodyContent = '{}'): TestCase {
  return {
    id,
    request_id: 'ep-1',
    name,
    category,
    method: 'POST',
    url_path: '/funds/transfer',
    path_variables: [],
    params: [],
    headers: [],
    body_type: 'json',
    body_content: bodyContent,
    last_run_status: 'Untested',
    created_at: '2026-01-01T00:00:00.000Z',
  }
}

function mountPanel(d: Endpoint) {
  return mount(TestCasesPanel, { props: { draft: d } })
}

describe('TestCasesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
    store.set([['ep-1', [makeCase('c1', '内部划转-SGB', '正向'), makeCase('c2', '金额超限', '边界值')]]])
    store.meta.clear()
    store.environments = []
    store.activeEnvId = null
    vi.clearAllMocks()
  })

  it('运行结果列：未运行显示 —；有元信息时显示 状态码 + 耗时', () => {
    const wrapper = mountPanel(draft())
    expect(wrapper.text()).toContain('—')
    store.meta.set('c1', { status: 200, durationMs: 145 })
    const w2 = mountPanel(draft())
    expect(w2.text()).toContain('200 OK')
    expect(w2.text()).toContain('(145ms)')
    store.meta.set('c2', { status: 500, durationMs: 320 })
    const w3 = mountPanel(draft())
    expect(w3.text()).toContain('500 Internal Error')
    expect(w3.text()).toContain('(320ms)')
  })

  it('Method 展示为 Badge（带方法语义色）', () => {
    const wrapper = mountPanel(draft())
    const badges = wrapper.findAll('.tcp-method')
    expect(badges).toHaveLength(2)
    expect(badges[0].text()).toBe('POST')
  })

  it('渲染表格行 + 分类计数', () => {
    const wrapper = mountPanel(draft())
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(2)
    const filterTexts = wrapper.findAll('.tcp-filter').map((n) => n.text())
    expect(filterTexts).toContain('全部 2')
    expect(filterTexts).toContain('正向 1')
    expect(filterTexts).toContain('边界值 1')
  })

  it('点击行体 → 该行进入选中态（浅色下可辨的 is-selected）', async () => {
    const wrapper = mountPanel(draft())
    const rows = wrapper.findAll('.tcp-body-row')
    expect(rows.every((r) => !r.classes().includes('is-selected'))).toBe(true)

    await rows[1].trigger('click')
    expect(rows[1].classes()).toContain('is-selected')
    // 单选语义：选中第 2 行后第 1 行不再选中
    expect(rows[0].classes()).not.toContain('is-selected')

    await rows[0].trigger('click')
    expect(rows[0].classes()).toContain('is-selected')
    expect(rows[1].classes()).not.toContain('is-selected')
  })

  it('工具栏展示当前激活环境（全部运行的环境上下文徽章）', () => {
    store.environments = [{ id: 'e1', name: '开发环境', base_url: 'http://127.0.0.1:4010' }]
    store.activeEnvId = 'e1'
    const wrapper = mountPanel(draft())
    const badge = wrapper.find('.env-badge')
    expect(badge.exists()).toBe(true)
    expect(badge.text()).toContain('开发环境')
    expect(badge.text()).toContain('http://127.0.0.1:4010')
  })

  it('点击分类 Tab 过滤列表', async () => {
    const wrapper = mountPanel(draft())
    const boundary = wrapper.findAll('.tcp-filter').find((n) => n.text().includes('边界值'))
    await boundary!.trigger('click')
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(1)
    expect(wrapper.text()).toContain('金额超限')
    expect(wrapper.text()).not.toContain('内部划转-SGB')
  })

  it('关键字搜索命中请求 Body 文本', async () => {
    store.set([
      [
        'ep-1',
        [
          makeCase('c1', '内部划转-SGB', '正向', '{"amount":100,"currency":"CNY"}'),
          makeCase('c2', '金额超限', '边界值', '{"amount":999999}'),
        ],
      ],
    ])
    const wrapper = mountPanel(draft())
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(2)
    await wrapper.find('.tcp-search-input').setValue('cny')
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(1)
    expect(wrapper.text()).toContain('内部划转-SGB')
    expect(wrapper.text()).not.toContain('金额超限')
  })

  it('搜索无命中时显示无匹配空态，可一键清空恢复', async () => {
    const wrapper = mountPanel(draft())
    await wrapper.find('.tcp-search-input').setValue('no-such-body-zzz')
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(0)
    expect(wrapper.text()).toContain('无匹配用例')
    await wrapper.find('.tcp-search-clear').trigger('click')
    expect(wrapper.findAll('.tcp-body-row')).toHaveLength(2)
  })

  it('点击用例名称 → 打开抽屉（不切调试页）', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    await wrapper.findAll('.tcp-name-btn')[0].trigger('click')
    expect(document.querySelectorAll('.drw').length).toBeGreaterThan(0)
    expect(store.openTestCaseInDebug).not.toHaveBeenCalled()
    expect(store.applyTestCaseToDraft).not.toHaveBeenCalled()
  })

  it('...菜单：直接运行 / 编辑用例 / 在调试页打开 / 克隆 / 删除', async () => {
    const wrapper = mountPanel(draft())
    await wrapper.findAll('.tcp-col-ops button')[0].trigger('click')
    const menuItems = Array.from(document.querySelectorAll('.miyo-menu-item'))
    expect(menuItems.map((m) => m.textContent)).toEqual(
      expect.arrayContaining(['直接运行', '编辑用例', '在调试页打开', '克隆', '删除']),
    )
    // 直接运行：原地执行，不切 Tab
    ;(menuItems.find((m) => m.textContent === '直接运行') as HTMLButtonElement).click()
    expect(store.runTestCase).toHaveBeenCalled()
    expect(store.openTestCaseInDebug).not.toHaveBeenCalled()
    // 在调试页打开：显式回填 + 切 Tab
    await wrapper.findAll('.tcp-col-ops button')[0].trigger('click')
    const items2 = Array.from(document.querySelectorAll('.miyo-menu-item'))
    ;(items2.find((m) => m.textContent === '在调试页打开') as HTMLButtonElement).click()
    expect(store.openTestCaseInDebug).toHaveBeenCalledWith('ep-1', expect.objectContaining({ name: '内部划转-SGB' }))
    // 编辑用例：打开抽屉
    await wrapper.findAll('.tcp-col-ops button')[0].trigger('click')
    const items3 = Array.from(document.querySelectorAll('.miyo-menu-item'))
    ;(items3.find((m) => m.textContent === '编辑用例') as HTMLButtonElement).click()
    expect(document.querySelectorAll('.drw').length).toBeGreaterThan(0)
    // 克隆
    await wrapper.findAll('.tcp-col-ops button')[0].trigger('click')
    const items4 = Array.from(document.querySelectorAll('.miyo-menu-item'))
    ;(items4.find((m) => m.textContent === '克隆') as HTMLButtonElement).click()
    expect(store.cloneTestCase).toHaveBeenCalledWith('ep-1', expect.objectContaining({ name: '内部划转-SGB' }))
  })

  it('直接运行不切换 Tab，状态列显示加载态', async () => {
    let release: (v: null) => void = () => {}
    store.runTestCase.mockReturnValueOnce(new Promise((res) => (release = res)))
    const wrapper = mountPanel(draft())
    await wrapper.findAll('.tcp-col-ops button')[0].trigger('click')
    const runItem = Array.from(document.querySelectorAll('.miyo-menu-item')).find(
      (m) => m.textContent === '直接运行',
    )!
    ;(runItem as HTMLButtonElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(store.openTestCaseInDebug).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('运行中…')
    release(null)
  })

  it('「添加用例」打开 Modal，确认后按当前草稿快照保存', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    await wrapper.findAll('button').find((b) => b.text().includes('添加用例'))!.trigger('click')
    const modal = Array.from(document.querySelectorAll('.m-dialog')).length
    expect(modal).toBeGreaterThan(0)
    // 填写名称后点击确认按钮（Modal Teleport）
    const input = document.querySelector<HTMLInputElement>('.tcm-input')!
    input.value = 'POST /funds/transfer'
    input.dispatchEvent(new Event('input'))
    await new Promise((r) => setTimeout(r, 0))
    const confirm = Array.from(document.querySelectorAll('.tcm-actions button')).find(
      (b) => b.textContent?.trim() === '确认',
    )
    expect(confirm).toBeDefined()
    expect((confirm as HTMLButtonElement).disabled).toBe(false)
    ;(confirm as HTMLButtonElement).click()
    await vi.waitFor(() => {
      expect(store.saveTestCase).toHaveBeenCalledWith(
        'ep-1',
        'POST /funds/transfer',
        '正向',
        d.request,
        '/funds/transfer',
        'POST',
        '',
      )
    })
  })
})