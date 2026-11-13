/**
 * EnvironmentBar 下拉菜单回归（选择器本体为共享组件 EnvironmentSelector，
 * EnvironmentBar 以 group 外观内嵌；本文件同时锁定该共享组件的结构与交互）：
 * - 列表项必须是「左区 [状态点 | 上下双行文字列]」结构——名称与 Base URL 各占一行，
 *   URL 永不与名称横向挤压截断（历史 bug：URL 单行横排被严重截断）；
 * - 面板结构顺序：搜索 → 分隔线 → 列表 → 分隔线 → 底部操作栏；
 * - 选中项最右 ✓、hover 复制按钮阻止冒泡（不触发切换环境）；
 * - 搜索按名称 / Base URL 过滤。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import EnvironmentBar from '@/domains/environment/components/EnvironmentBar.vue'
import { useLocaleStore } from '@/stores/locale'
import { copyText } from '@/utils/clipboard'
import type { Environment } from '@/types/miyoApi'

const setEnvironment = vi.fn()

const ENVS: Environment[] = [
  {
    id: 'e-dev',
    project_id: 'p1',
    name: '开发环境',
    base_url: 'https://jsonplaceholder.typicode.com',
    variables: [],
    created_at: '',
    updated_at: '',
  },
  {
    id: 'e-test',
    project_id: 'p1',
    name: '测试环境',
    base_url: 'https://api.test.example.com',
    variables: [],
    created_at: '',
    updated_at: '',
  },
]

vi.mock('@/stores/workspace.ts', () => ({
  useWorkspaceStore: () => ({
    environments: ENVS,
    activeEnvId: 'e-dev',
    project: { id: 'p1' },
    globalVariables: [],
    globalParams: [],
    setEnvironment,
  }),
}))

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() }),
}))

vi.mock('@/utils/clipboard.ts', () => ({
  copyText: vi.fn(async () => true),
}))

function mountBar() {
  return mount(EnvironmentBar, {
    attachTo: document.body,
    global: {
      stubs: { EnvironmentManager: true, EnvironmentQuickView: true },
    },
  })
}

async function openPop(wrapper: ReturnType<typeof mountBar>) {
  await wrapper.find('.cs-trigger').trigger('click')
  await nextTick()
  const pop = document.querySelector('.cs-pop')
  expect(pop).toBeTruthy()
  // popClass 挂在 .cs-pop 自身（同元素双类）——样式必须用 .cs-pop.env-pop，
  // 写成后代 .env-pop .cs-pop 会静默失效（回归见本文件头注释）
  expect(pop!.classList.contains('env-pop')).toBe(true)
  return pop as HTMLElement
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  setEnvironment.mockClear()
  vi.mocked(copyText).mockClear()
  // jsdom 无布局：scrollIntoView 打桩（键盘高亮滚动会调用）
  HTMLElement.prototype.scrollIntoView = vi.fn()
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('EnvironmentBar 下拉菜单', () => {
  it('面板结构顺序：搜索 → 列表 → 底部操作栏（兄弟直挂，各带分隔线）', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    const kids = Array.from(pop.children).map((el) => el.className)
    expect(kids[0]).toContain('cs-pop-search')
    expect(kids[1]).toContain('cs-pop-list')
    expect(kids[2]).toContain('cs-pop-footer')
    // 搜索框存在且有占位符
    const input = pop.querySelector<HTMLInputElement>('.es-search-input')
    expect(input).toBeTruthy()
    expect(input!.placeholder).toBe('搜索环境名称或地址…')
    wrapper.unmount()
  })

  it('列表项双行结构：状态点 | 文字列(名称行 + URL行)，URL 独占第二行不与名称横排', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    // 「无环境」行：有左区与文字列，无 URL 行
    const rows = Array.from(pop.querySelectorAll<HTMLElement>('.cs-pop-list .cs-opt'))
    expect(rows.length).toBe(3)
    const noEnv = rows[0]
    expect(noEnv.querySelector('.env-opt-left .env-dot')).toBeTruthy()
    expect(noEnv.querySelector('.env-opt-text .env-opt-name')?.textContent?.trim()).toBe('无环境')
    expect(noEnv.querySelector('.env-opt-url')).toBeNull()

    // 开发环境行：左区 = 点 + 文字列；文字列内 名称在前、URL 在后（flex-col 上下两行）
    const dev = rows[1]
    const left = dev.querySelector('.env-opt-left')
    expect(left).toBeTruthy()
    expect(left!.querySelector(':scope > .env-dot')).toBeTruthy()
    const textCol = left!.querySelector<HTMLElement>(':scope > .env-opt-text')
    expect(textCol).toBeTruthy()
    const kids = Array.from(textCol!.children).map((el) => el.className)
    expect(kids[0]).toContain('env-opt-name')
    expect(kids[1]).toContain('env-opt-url')
    // URL 全文渲染 + title 悬浮，而不是被横向截掉内容
    const url = textCol!.querySelector<HTMLElement>('.env-opt-url')!
    expect(url.textContent?.trim()).toBe('https://jsonplaceholder.typicode.com')
    expect(url.getAttribute('title')).toBe('https://jsonplaceholder.typicode.com')
    // 名称不是 URL 的父级/兄弟横排——URL 必须在 .env-opt-text 内（双行容器）
    expect(url.closest('.env-opt-name')).toBeNull()
    wrapper.unmount()
  })

  it('选中项最右侧 ✓（左点不抢位）；状态点颜色按环境归类', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    const sel = pop.querySelector('.cs-pop-list .cs-opt.sel')!
    expect(sel).toBeTruthy()
    expect(sel.querySelector('.env-opt-name')?.textContent?.trim()).toBe('开发环境')
    // ✓ 在行内最右的 check 槽位
    const check = sel.querySelector('.cs-opt-check')
    expect(check).toBeTruthy()
    expect(check!.querySelector('svg')).toBeTruthy()
    // 色点归类：开发绿(ed-dev) / 测试蓝(ed-test)
    const rows = pop.querySelectorAll('.cs-pop-list .cs-opt')
    expect(rows[1].querySelector('.env-dot')!.classList.contains('ed-dev')).toBe(true)
    expect(rows[2].querySelector('.env-dot')!.classList.contains('ed-test')).toBe(true)
    wrapper.unmount()
  })

  it('底部操作栏：新建环境 / 管理环境 两端按钮', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    const btns = Array.from(pop.querySelectorAll('.cs-pop-footer .es-foot-btn'))
    expect(btns).toHaveLength(2)
    expect(btns[0].textContent).toContain('新建环境')
    expect(btns[1].textContent).toContain('管理环境')
    wrapper.unmount()
  })

  it('底栏「新建环境」打开环境管理弹窗（弹窗由选择器自持，管理大 chunk 不常驻）', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    Array.from(pop.querySelectorAll<HTMLButtonElement>('.es-foot-btn'))[0].click()
    await nextTick()

    // 管理弹窗是 stub（异步组件按需渲染），props 以属性形式落到 stub 上
    const mgr = wrapper.findComponent({ name: 'EnvironmentManager' })
    expect(mgr.exists()).toBe(true)
    const attrs = mgr.attributes()
    expect(attrs.open).toBe('true')
    expect(attrs['create-new']).toBe('true')
    // 打开管理弹窗前先收起下拉
    expect(document.querySelector('.cs-pop')).toBeNull()
    wrapper.unmount()
  })

  it('hover 复制按钮：写剪贴板且阻止冒泡（不切换环境）', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    // 测试环境行的第一个操作按钮 = 复制
    const testRow = pop.querySelectorAll('.cs-pop-list .cs-opt')[2]
    const copyBtn = testRow.querySelector<HTMLButtonElement>('.es-act')!
    expect(copyBtn.getAttribute('title')).toBe('复制 Base URL')
    copyBtn.click()
    await nextTick()
    expect(copyText).toHaveBeenCalledWith('https://api.test.example.com')
    expect(setEnvironment).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('搜索过滤：按名称与 Base URL 匹配，无环境项在过滤时剔除', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)
    const input = pop.querySelector<HTMLInputElement>('.es-search-input')!

    input.value = '测试'
    input.dispatchEvent(new Event('input'))
    await nextTick()
    let rows = pop.querySelectorAll('.cs-pop-list .cs-opt')
    expect(rows).toHaveLength(1)
    expect(rows[0].querySelector('.env-opt-name')?.textContent?.trim()).toBe('测试环境')

    input.value = 'jsonplaceholder'
    input.dispatchEvent(new Event('input'))
    await nextTick()
    rows = pop.querySelectorAll('.cs-pop-list .cs-opt')
    expect(rows).toHaveLength(1)
    expect(rows[0].querySelector('.env-opt-name')?.textContent?.trim()).toBe('开发环境')

    input.value = 'zzz-none'
    input.dispatchEvent(new Event('input'))
    await nextTick()
    expect(pop.querySelectorAll('.cs-pop-list .cs-opt')).toHaveLength(0)
    expect(pop.querySelector('.es-search-empty')?.textContent).toContain('无匹配环境')
    wrapper.unmount()
  })

  it('点击列表项切换环境', async () => {
    const wrapper = mountBar()
    const pop = await openPop(wrapper)

    const testRow = pop.querySelectorAll<HTMLElement>('.cs-pop-list .cs-opt')[2]
    testRow.click()
    await nextTick()
    expect(setEnvironment).toHaveBeenCalledWith('e-test')
    wrapper.unmount()
  })
})
