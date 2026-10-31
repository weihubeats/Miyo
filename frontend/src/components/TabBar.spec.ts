/**
 * TabBar 单测：
 * 回归——tab 开多时「+」新建按钮被滚出可视区（+ 在滚动区外固定时，
 * 离末尾 tab 太远；之前在滚动区内末尾时又会被滚丢）。
 * 现状：＋与 ⋯ 紧跟末尾 tab，溢出时各自 sticky 吸在右边缘（Chrome 式），
 * 两种情况都可见。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick, reactive } from 'vue'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import TabBar from './TabBar.vue'
import { useLocaleStore } from '../stores/locale'

// 响应式假 store（watcher 与模板需跟踪 activeTabId 变化）
let mocked: ReturnType<typeof makeStore>
function makeStore() {
  return reactive({
    openTabs: ['t1', 't2', 't3', 't4', 't5', 't6', 't7', 't8'],
    project: { id: 'p1' },
    activeTabId: 't8' as string | null,
    draftOf: (id: string) => ({ method: 'GET', id }),
    titleOf: (id: string) => (id.startsWith('create:') ? '新建…' : `未命名接口 ${id}`),
    isCreateTab: (id: string) => id.startsWith('create:'),
    isScenarioTab: (id: string) => id.startsWith('scenario:'),
    isDirty: vi.fn((_id: string) => false),
    closeTab: vi.fn(),
    closeOtherTabs: vi.fn(),
    closeAllTabs: vi.fn(),
    openNewEndpoint: vi.fn(),
    saveFolder: vi.fn(),
  })
}

vi.mock('../stores/workspace', () => ({
  useWorkspaceStore: () => mocked,
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  mocked = makeStore()
  // jsdom 未实现 scrollIntoView：桩住并记录调用目标
  Element.prototype.scrollIntoView = vi.fn() as unknown as typeof Element.prototype.scrollIntoView
})

afterEach(() => {
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

function mountBar() {
  return mount(TabBar, { attachTo: document.body })
}

describe('TabBar：新建按钮紧跟末尾且不被滚丢', () => {
  it('「+」与「⋯」是滚动区内最后两个孩子：tab 少时紧贴末尾', () => {
    const wrapper = mountBar()
    const scroll = document.querySelector('.tab-scroll')
    expect(scroll).toBeTruthy()
    const kids = Array.from(scroll!.children)
    // 8 个 tab + 「+」触发包裹层 + 「⋯」管理按钮
    expect(kids).toHaveLength(10)
    expect(kids.slice(0, 8).every((el) => el.classList.contains('tab'))).toBe(true)
    // 顺序：+ 在 ⋯ 左边
    const addWrap = kids[8] as HTMLElement
    expect(addWrap.querySelector('.tab-add')).toBeTruthy()
    expect((kids[9] as HTMLElement).classList.contains('tab-more')).toBe(true)
    wrapper.unmount()
  })

  it('「+」与「⋯」分别 sticky 右吸：溢出时钉在右边缘', () => {
    const wrapper = mountBar()
    // jsdom 无布局且本工程 vitest 不注入 SFC 样式，定位能力用源码断言
    //（themePattern.spec.ts 同款做法）；DOM 顺序由上一用例覆盖
    const src = readFileSync(join(process.cwd(), 'src/components/TabBar.vue'), 'utf8')
    const style = src.slice(src.indexOf('<style'))
    expect(style).toContain('.tab-scroll > .tt-trigger')
    // 「⋯」是 trigger 的兄弟，需自带 sticky 才不会滚丢
    expect(style).toMatch(/\.tab-more\s*\{[\s\S]*?position:\s*sticky/)
    expect(style).toMatch(/\.tab-more\s*\{[\s\S]*?right:\s*0/)
    wrapper.unmount()
  })

  it('点击「+」上抛 create 事件（由父级在内容区打开新建选择页）', async () => {
    const wrapper = mountBar()
    document
      .querySelector<HTMLButtonElement>('.tab-add:not(.tab-more)')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('create')).toHaveLength(1)
    // 不再直接建快捷请求：动作由选择页发起
    expect(mocked.openNewEndpoint).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('「新建」伪页签：闪电图标 + 新建… 标题，无方法徽章', () => {
    mocked.openTabs.push('create:c1')
    const wrapper = mountBar()

    const rows = document.querySelectorAll('.tab-scroll .tab')
    expect(rows).toHaveLength(9)
    const last = rows[8] as HTMLElement
    expect(last.querySelector('.tab-spark')).toBeTruthy()
    expect(last.querySelector('.method-tag')).toBeNull()
    expect(last.querySelector('.tab-title')?.textContent).toBe('新建…')

    wrapper.unmount()
  })

  it('切换激活 tab 时滚动的是 tab 本体（+ 按钮不受影响）', async () => {
    const wrapper = mountBar()
    const spy = Element.prototype.scrollIntoView as unknown as ReturnType<typeof vi.fn>
    spy.mockClear()
    mocked.activeTabId = 't1'
    await nextTick()
    await nextTick()
    expect(spy).toHaveBeenCalledTimes(1)
    // 调用目标即当前激活的 tab 行，而非容器或 + 按钮
    expect(spy.mock.instances[0]).toBe(document.querySelector('.tab.active'))
    wrapper.unmount()
  })

  it('关闭按钮调 closeTab（行为保持）', async () => {
    const wrapper = mountBar()
    const closeBtn = document.querySelectorAll('.tab-close')[0] as HTMLButtonElement
    closeBtn.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(mocked.closeTab).toHaveBeenCalled()
    wrapper.unmount()
  })

  /** 打开「⋯」下拉菜单，返回菜单项文案列表。 */
  async function openMenuLabels(): Promise<string[]> {
    document
      .querySelector<HTMLButtonElement>('.tab-more')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    return Array.from(document.querySelectorAll('.miyo-menu-item')).map(
      (m) => m.textContent?.trim() ?? '',
    )
  }

  function clickMenuItem(label: string): void {
    const item = Array.from(document.querySelectorAll<HTMLElement>('.miyo-menu-item')).find(
      (m) => m.textContent?.trim() === label,
    )!
    item.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  }

  it('下拉为标签管理三项：无新建/导入重复项', async () => {
    const wrapper = mountBar()
    expect(await openMenuLabels()).toEqual(['关闭当前标签页', '关闭其他标签页', '关闭全部标签页'])
    wrapper.unmount()
  })

  it('下拉行视觉统一：纯文本行 + 危险项标红分隔', async () => {
    const wrapper = mountBar()
    await openMenuLabels()
    // 三行均无图标（标签不错位），关闭全部标红且组前分隔
    expect(document.querySelectorAll('.miyo-menu-item .miyo-menu-icon')).toHaveLength(0)
    const danger = document.querySelector('.miyo-menu-item.danger')
    expect(danger?.textContent?.trim()).toBe('关闭全部标签页')
    expect(danger?.previousElementSibling?.classList.contains('miyo-menu-divider')).toBe(true)
    wrapper.unmount()
  })

  it('关闭其他：仅保留当前（干净时无二次确认）', async () => {
    const wrapper = mountBar()
    await openMenuLabels()
    clickMenuItem('关闭其他标签页')
    await nextTick()
    expect(mocked.closeOtherTabs).toHaveBeenCalledWith('t8')
    expect(mocked.closeTab).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('关闭全部：干净时直接执行', async () => {
    const wrapper = mountBar()
    await openMenuLabels()
    clickMenuItem('关闭全部标签页')
    await nextTick()
    expect(mocked.closeAllTabs).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('含未保存时行内二次确认，确认后才执行', async () => {
    mocked.isDirty.mockImplementation((id: string) => id !== 't8')
    const wrapper = mountBar()
    await openMenuLabels()
    clickMenuItem('关闭其他标签页')
    await nextTick()
    // 进入确认视图：展示未保存数，尚未执行
    expect(document.querySelector('.miyo-menu-confirm-title')?.textContent).toContain('7')
    expect(mocked.closeOtherTabs).not.toHaveBeenCalled()
    const ok = document.querySelector<HTMLButtonElement>('.miyo-menu-confirm-actions .miyo-btn-danger-solid')!
    ok.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(mocked.closeOtherTabs).toHaveBeenCalledWith('t8')
    wrapper.unmount()
  })

  it('下拉在触发器右侧展开（左对齐，不盖住左侧标签）', async () => {
    // 宽视口 + 靠右的触发器：左对齐展开应落在触发器左缘
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1600 })
    const wrapper = mountBar()
    try {
      const arrow = document.querySelector<HTMLElement>('.tab-more')!
      vi.spyOn(arrow, 'getBoundingClientRect').mockReturnValue({
        left: 1200,
        right: 1240,
        top: 10,
        bottom: 38,
        width: 40,
        height: 28,
        x: 1200,
        y: 10,
        toJSON: () => '',
      })
      arrow.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await nextTick()
      const menu = document.querySelector('.miyo-menu') as HTMLElement
      expect(menu).toBeTruthy()
      // left 对齐：菜单左缘 == 触发器左缘（右对齐会是 1240-176=1064）
      expect(menu.style.left).toBe('1200px')
      wrapper.unmount()
    } finally {
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1024 })
    }
  })

  it('仅剩一个标签时关闭其他禁用', async () => {
    mocked.openTabs.splice(0, mocked.openTabs.length, 't8')
    const wrapper = mountBar()
    await openMenuLabels()
    const disabled = Array.from(
      document.querySelectorAll<HTMLButtonElement>('.miyo-menu-item[disabled]'),
    ).map((m) => m.textContent?.trim())
    expect(disabled).toEqual(['关闭其他标签页'])
    wrapper.unmount()
  })
})

describe('TabBar：键盘导航（role=tablist + roving tabindex + ←→ / Delete）', () => {
  function rows(): HTMLElement[] {
    return [...document.querySelectorAll<HTMLElement>('.tab-scroll .tab')]
  }

  function press(target: HTMLElement, k: string): void {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  }

  it('容器 role=tablist，页签 role=tab + aria-selected + roving tabindex', () => {
    const wrapper = mountBar()
    expect(document.querySelector('.tab-scroll')?.getAttribute('role')).toBe('tablist')
    const list = rows()
    expect(list).toHaveLength(8)
    expect(list.every((el) => el.getAttribute('role') === 'tab')).toBe(true)
    // 激活项 t8 可 Tab 进入，其余 -1
    expect(list.map((el) => el.getAttribute('tabindex'))).toEqual([
      '-1', '-1', '-1', '-1', '-1', '-1', '-1', '0',
    ])
    expect(list[7].getAttribute('aria-selected')).toBe('true')
    expect(list[0].getAttribute('aria-selected')).toBe('false')
    wrapper.unmount()
  })

  it('← / → 移动焦点并激活相邻页签', () => {
    const wrapper = mountBar()
    const active = rows()[7]
    active.focus()
    press(active, 'ArrowLeft')
    expect(mocked.activeTabId).toBe('t7')
    expect(document.activeElement).toBe(rows()[6])
    press(rows()[6], 'ArrowRight')
    expect(mocked.activeTabId).toBe('t8')
    wrapper.unmount()
  })

  it('Home / End 跳首尾，末尾回绕到首', () => {
    const wrapper = mountBar()
    rows()[7].focus()
    press(rows()[7], 'Home')
    expect(mocked.activeTabId).toBe('t1')
    press(rows()[0], 'ArrowLeft')
    expect(mocked.activeTabId).toBe('t8')
    wrapper.unmount()
  })

  it('Delete 走既有关闭链路：干净页签直接 closeTab', async () => {
    const wrapper = mountBar()
    rows()[7].focus()
    press(rows()[7], 'Delete')
    await nextTick()
    expect(mocked.closeTab).toHaveBeenCalledWith('t8')
    wrapper.unmount()
  })

  it('Delete 脏页签弹既有 Popconfirm，不直接关闭', async () => {
    mocked.isDirty.mockImplementation((id: string) => id === 't8')
    const wrapper = mountBar()
    rows()[7].focus()
    press(rows()[7], 'Delete')
    await nextTick()
    expect(mocked.closeTab).not.toHaveBeenCalled()
    expect(document.querySelector('.pc-pop')).toBeTruthy()
    wrapper.unmount()
  })
})
