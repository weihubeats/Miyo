/**
 * EndpointTree 单测：递归树的「新建子文件夹」链路回归。
 *
 * 背景：行内编辑状态若不跨递归实例共享（或共享键被声明为实例级 Symbol），
 * 点「新建子文件夹」后输入框永远不渲染（状态留在触发菜单的父实例，
 * 输入行渲染条件在 folderId === parentId 的【子】实例上）。
 */
import { describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import EndpointTree from './EndpointTree.vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { stubScrollIntoView } from '../testUtils/componentTest'
import { makeDraft } from '../testUtils/draftFixture'
import type { Folder } from '../types/miyoApi'

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => ({
    listExamples: vi.fn().mockResolvedValue([]),
    listRequestExamples: vi.fn().mockResolvedValue([]),
    listTestCases: vi.fn().mockResolvedValue([]),
  }),
}))

function folder(id: string, name: string, parentId: string | null): Folder {
  const now = '2026-01-01T00:00:00.000Z'
  return {
    id,
    project_id: 'proj-test-1',
    parent_id: parentId,
    name,
    sort_order: 0,
    created_at: now,
    updated_at: now,
  }
}

async function mountTree() {
  // 展开态为模块级共享（跨用例存活）：每例复位避免串现场
  ;(globalThis as { __resetTreeExpanded?: () => void }).__resetTreeExpanded?.()
  localStorage.removeItem('miyo.tree-expanded.proj-test-1')
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.folders = [folder('f-root', '宠物管理', null), folder('f-sub', '子目录', 'f-root')]
  store.endpoints = []
  const wrapper = mount(EndpointTree, {
    props: { folderId: null, search: '' },
    global: { plugins: [] },
    attachTo: document.body,
  })
  return { wrapper, store }
}

describe('EndpointTree：新建子文件夹', () => {
  it('点击「新建子文件夹」后，目标文件夹子树内出现行内输入框', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountTree()

      // 根级文件夹「宠物管理」行上的 ⋯ 按钮
      const row = wrapper.find('[data-dnd-kind="folder"][data-dnd-id="f-root"]')
      expect(row.exists()).toBe(true)
      await row.find('button').trigger('click')

      // Menu teleport 到 body：从 document 中找到「新建子文件夹」菜单项并点击
      const item = [...document.querySelectorAll('button')].find((b) =>
        b.textContent?.includes('新建子文件夹'),
      )
      expect(item, '菜单项应已渲染').toBeDefined()
      item!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      await new Promise((r) => setTimeout(r, 0))

      // 输入行渲染在【子实例】（folderId === f-root）的子树里：
      // 共享状态生效的直接证据——父实例本地状态永远到不了这里
      const childrenOfFolder = wrapper.find(
        '[data-dnd-tree-root="f-root"], .tree-children [data-dnd-tree-root="f-root"]',
      )
      const input = childrenOfFolder.find('input.tree-input')
      expect(input.exists(), '子文件夹行内输入框应出现').toBe(true)
      expect(input.attributes('placeholder')).toBe('文件夹名称')
    } finally {
      restore()
    }
  })
})

describe('EndpointTree：行间无点击死区', () => {
  it('.tree 容器间隙为 0（gap 会在行与行之间留下点不中的死区）', () => {
    // jsdom 无布局，定位能力用源码断言（TabBar sticky 同款做法）
    const src = readFileSync(join(process.cwd(), 'src/components/EndpointTree.vue'), 'utf8')
    const style = src.slice(src.indexOf('<style'))
    expect(style).toMatch(/\.tree\s*\{[^}]*gap:\s*0/)
  })
})

describe('EndpointTree：整行可点击（文本外空白也响应）', () => {
  async function mountWithEndpoint() {
    document.body.innerHTML = ''
    ;(globalThis as { __resetTreeExpanded?: () => void }).__resetTreeExpanded?.()
    localStorage.removeItem('miyo.tree-expanded.proj-test-1')
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    const store = useWorkspaceStore()
    store.project = {
      id: 'proj-test-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    store.folders = [folder('f-root', '宠物管理', null)]
    store.endpoints = [
      makeDraft({ id: 'ep-1', name: '用户列表', method: 'GET', path: '/users' }),
      makeDraft({ id: 'ep-2', name: '子接口', method: 'POST', path: '/sub', folder_id: 'f-root' }),
    ]
    const wrapper = mount(EndpointTree, {
      props: { folderId: null, search: '' },
      attachTo: document.body,
    })
    return { wrapper, store }
  }

  it('点击方法徽章 / 行空白处都能打开接口', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper, store } = await mountWithEndpoint()
      // 方法徽章（非文本、非按钮）→ 打开
      await wrapper.find('.tree-row[data-dnd-id="ep-1"] .tree-method').trigger('click')
      expect(store.activeTabId).toBe('ep-1')
      store.activeTabId = null
      await wrapper.find('.tree-row[data-dnd-id="ep-1"]').trigger('click')
      expect(store.activeTabId).toBe('ep-1')
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('点击 ⋯ 按钮只开菜单不打开接口', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper, store } = await mountWithEndpoint()
      await wrapper.find('.tree-row[data-dnd-id="ep-1"] .tree-actions button').trigger('click')
      expect(store.activeTabId).toBeNull()
      expect(document.querySelector('.miyo-menu')).toBeTruthy()
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('点击文件夹行空白处展开/收起（箭头仍可单独点）', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountWithEndpoint()
      // 子树用 v-show 显隐（DOM 常驻），断言可见性而非文本有无
      const children = () => wrapper.find('.tree-row[data-dnd-id="f-root"] + .tree-children')
      expect(children().isVisible()).toBe(false)
      await wrapper.find('.tree-row[data-dnd-id="f-root"]').trigger('click')
      expect(children().isVisible()).toBe(true)
      await wrapper.find('.tree-row[data-dnd-id="f-root"]').trigger('click')
      expect(children().isVisible()).toBe(false)
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })
})

describe('EndpointTree：ARIA 树语义 + 键盘导航', () => {
  async function mountKeyboardTree() {
    document.body.innerHTML = ''
    ;(globalThis as { __resetTreeExpanded?: () => void }).__resetTreeExpanded?.()
    localStorage.removeItem('miyo.tree-expanded.proj-test-1')
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
    const store = useWorkspaceStore()
    store.project = {
      id: 'proj-test-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    store.folders = [folder('f-root', '宠物管理', null), folder('f-sub', '子目录', 'f-root')]
    store.endpoints = [
      makeDraft({ id: 'ep-1', name: '用户列表', method: 'GET', path: '/users' }),
      makeDraft({ id: 'ep-2', name: '子接口', method: 'POST', path: '/sub', folder_id: 'f-root' }),
    ]
    const wrapper = mount(EndpointTree, {
      props: { folderId: null, search: '' },
      attachTo: document.body,
    })
    // roving tabindex 在 onMounted 的 nextTick 里落到首行
    await nextTick()
    return { wrapper, store }
  }

  function row(id: string): HTMLElement {
    const el = document.querySelector<HTMLElement>(`.tree-row[data-dnd-id="${id}"]`)
    expect(el, `行 ${id} 应存在`).toBeTruthy()
    return el!
  }

  function press(target: HTMLElement, k: string): void {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  }

  const childrenOf = (id: string) =>
    document.querySelector<HTMLElement>(`[data-dnd-id="${id}"] + .tree-children`)!

  it('根容器 role=tree，行 role=treeitem + aria-level，焦点行 roving tabindex', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountKeyboardTree()
      const root = document.querySelector('[data-dnd-tree-root=""]')!
      expect(root.getAttribute('role')).toBe('tree')
      expect(root.getAttribute('aria-multiselectable')).toBe('true')
      expect(row('f-root').getAttribute('role')).toBe('treeitem')
      expect(row('f-root').getAttribute('aria-level')).toBe('1')
      expect(row('f-root').getAttribute('aria-expanded')).toBe('false')
      // 焦点行 0，其余 -1（Tab 进树只落一个入口）
      expect(row('f-root').getAttribute('tabindex')).toBe('0')
      expect(row('ep-1').getAttribute('tabindex')).toBe('-1')
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('↑↓ 在可见行间移动焦点（收起的子树不进焦点序列）', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountKeyboardTree()
      row('f-root').focus()
      press(row('f-root'), 'ArrowDown')
      expect(document.activeElement).toBe(row('ep-1'))
      press(row('ep-1'), 'ArrowUp')
      expect(document.activeElement).toBe(row('f-root'))
      // 收起时子行不可见：↓ 直接跳过它到下一层可见行
      press(row('f-root'), 'ArrowDown')
      expect(document.activeElement).toBe(row('ep-1'))
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('→ 展开折叠的文件夹，再 → 落到首个子行；子行 aria-level=2', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountKeyboardTree()
      expect(childrenOf('f-root').style.display).toBe('none')
      row('f-root').focus()
      press(row('f-root'), 'ArrowRight')
      await nextTick()
      expect(childrenOf('f-root').style.display).not.toBe('none')
      expect(row('f-root').getAttribute('aria-expanded')).toBe('true')
      // 已展开 → 焦点下移进第一个子行
      press(row('f-root'), 'ArrowRight')
      expect(document.activeElement).toBe(row('f-sub'))
      expect(row('f-sub').getAttribute('aria-level')).toBe('2')
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('← 收起展开的文件夹；收起态再 ← 上移到父行', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountKeyboardTree()
      row('f-root').focus()
      press(row('f-root'), 'ArrowRight')
      await nextTick()
      press(row('f-root'), 'ArrowLeft')
      await nextTick()
      expect(childrenOf('f-root').style.display).toBe('none')
      expect(row('f-root').getAttribute('aria-expanded')).toBe('false')
      // 再按 ←：已收起 → 上移（根行已无父行，焦点保持在根行）
      row('f-root').focus()
      press(row('f-root'), 'ArrowLeft')
      expect(document.activeElement).toBe(row('f-root'))
      // 子层行的 ← 才体现「上移到父行」
      press(row('f-root'), 'ArrowRight')
      await nextTick()
      press(row('f-sub'), 'ArrowLeft')
      expect(document.activeElement).toBe(row('f-root'))
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('Enter 打开接口行（走既有 openEndpoint 链路），Home/End 跳首尾', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper, store } = await mountKeyboardTree()
      row('ep-1').focus()
      press(row('ep-1'), 'Enter')
      expect(store.activeTabId).toBe('ep-1')
      store.activeTabId = null

      row('f-root').focus()
      press(row('f-root'), 'End')
      expect(document.activeElement).toBe(row('ep-1'))
      press(row('ep-1'), 'Home')
      expect(document.activeElement).toBe(row('f-root'))
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })

  it('行内编辑输入框 / 按钮不被树导航抢键（原生行为保持）', async () => {
    const restore = stubScrollIntoView()
    try {
      const { wrapper } = await mountKeyboardTree()
      // 焦点落在行内 ⋯ 按钮：Enter 归按钮原生 click，方向键仍可在树内移动
      const btn = row('f-root').querySelector<HTMLElement>('.tree-actions button')!
      btn.focus()
      const ev = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
      btn.dispatchEvent(ev)
      expect(ev.defaultPrevented).toBe(false)
      press(btn, 'ArrowDown')
      expect(document.activeElement).toBe(row('ep-1'))
      wrapper.unmount()
    } finally {
      restore()
      document.body.innerHTML = ''
    }
  })
})
