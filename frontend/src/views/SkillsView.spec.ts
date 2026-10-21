/**
 * SkillsView 交互单测：点列表只切换右侧详情，不发生路由跳转。
 *
 * 回归点：此前点击 skill 会打开弹窗（甚至被误认为页面跳转），
 * 现在选中态完全由本地状态驱动，路由保持不变。
 */
import { describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '../stores/locale'
import type { SkillDoc, SkillIndex, SkillLinkState, SkillSummary } from '../types/foxApi'

const api = vi.hoisted(() => ({
  skillsScan: vi.fn(),
  skillSetEnabled: vi.fn(),
  skillDirPath: vi.fn(),
  skillRead: vi.fn(),
  skillCreate: vi.fn(),
  skillSave: vi.fn(),
  skillDelete: vi.fn(),
}))

vi.mock('../composables/useFoxApi', () => ({ useFoxApi: () => api }))
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), info: vi.fn(), warning: vi.fn(), error: vi.fn() }),
}))
vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
  useRoute: () => ({ path: '/skills' }),
}))
// 只关心点击语义，Tauri 侧插件在 jsdom 下会直接抛错。
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: vi.fn() }))
// 顶栏拖窗在 jsdom 下没有 Tauri window 运行时。
vi.mock('../composables/useWindowDrag', () => ({ useWindowDrag: vi.fn() }))

import SkillsView from './SkillsView.vue'

function summary(name: string, state: SkillLinkState): SkillSummary {
  return {
    name,
    description: `${name} 描述`,
    sourceRoot: '/home/u/.agents/skills',
    dir: `/home/u/.agents/skills/${name}`,
    updatedAt: 1,
    extraEntries: [],
    states: [
      {
        toolId: 'claude-code',
        toolName: 'Claude Code',
        root: '/home/u/.claude/skills',
        state,
        detail: null,
      },
    ],
    warnings: [],
  }
}

const INDEX: SkillIndex = {
  sourceRoot: '/home/u/.agents/skills',
  skills: [summary('alpha', 'linked'), summary('beta', 'drifted')],
  foreign: [],
  tools: [
    {
      id: 'claude-code',
      displayName: 'Claude Code',
      kind: 'symlink',
      verified: true,
      root: '/home/u/.claude/skills',
      present: true,
    },
    {
      id: 'cursor',
      displayName: 'Cursor',
      kind: 'symlink',
      verified: true,
      root: '/home/u/.cursor/skills',
      present: true,
    },
    {
      id: 'codex',
      displayName: 'Codex',
      kind: 'symlink',
      verified: false,
      root: '/home/u/.codex/skills',
      present: false,
    },
  ],
  warnings: [],
}

function doc(name: string): SkillDoc {
  return {
    summary: summary(name, 'linked'),
    frontmatter: { name, description: `${name} 描述` },
    body: `${name} 正文`,
    raw: `---\nname: ${name}\n---\n`,
  }
}

async function mountView() {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  api.skillsScan.mockResolvedValue(INDEX)
  api.skillRead.mockImplementation((name: string) => Promise.resolve(doc(name)))
  api.skillCreate.mockResolvedValue(summary('gamma', 'missing'))

  const wrapper = mount(SkillsView, { attachTo: document.body })
  // 等待 onMounted 里的扫描 + 自动选中第一条
  await vi.waitFor(() => expect(api.skillRead).toHaveBeenCalled())
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('SkillsView 列表 / 详情分栏', () => {
  it('渲染列表并在右侧展示首条详情', async () => {
    const wrapper = await mountView()

    const items = wrapper.findAll('.skill-item')
    expect(items).toHaveLength(2)
    expect(items[0].classes()).toContain('selected')
    expect(wrapper.find('.detail-view').exists()).toBe(true)
    expect(wrapper.find('.detail-view h1').text()).toBe('alpha')
    expect(api.skillRead.mock.calls[0][0] as string).toBe('alpha')

    wrapper.unmount()
  })

  it('点击另一条：切选中态并加载详情，不打开编辑表单', async () => {
    const wrapper = await mountView()

    await wrapper.findAll('.skill-item')[1].trigger('click')
    await vi.waitFor(() => {
      expect(api.skillRead.mock.calls.some((call) => (call[0] as string) === 'beta')).toBe(true)
      expect(wrapper.find('.detail-view h1').exists()).toBe(true)
    })

    const items = wrapper.findAll('.skill-item')
    expect(items[1].classes()).toContain('selected')
    expect(items[0].classes()).not.toContain('selected')
    expect(wrapper.find('.detail-view h1').text()).toBe('beta')
    // 编辑表单只在点「编辑」后出现
    expect(wrapper.find('.detail-form').exists()).toBe(false)

    wrapper.unmount()
  })

  it('点「编辑」才切到表单，取消后回到只读详情', async () => {
    const wrapper = await mountView()

    await wrapper.find('.detail-actions .rf-btn-primary').trigger('click')
    expect(wrapper.find('.detail-form').exists()).toBe(true)

    // 表单里第一个按钮是「取消」
    await wrapper.find('.detail-form .rf-btn').trigger('click')
    expect(wrapper.find('.detail-form').exists()).toBe(false)
    expect(wrapper.find('.detail-view').exists()).toBe(true)

    wrapper.unmount()
  })
})
describe('SkillsView 使用方式', () => {
  it('从 description 里提取别名 / 触发条件 / 命令示例', async () => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    api.skillsScan.mockResolvedValue(INDEX)
    api.skillRead.mockImplementation((name: string) => {
      const base = doc(name)
      base.frontmatter.description = [
        '流程级代码审查。',
        '别名：`/flow-review`、`/fcr`',
        '触发条件：',
        '- 直接调用：`/flow-review --entry ClassName.method`',
        '- 需求对齐：`/flow-review --entry OrderService.createOrder --req docs/PRD-order.md`',
        'harness_lifecycle: active',
      ].join('\n')
      return Promise.resolve(base)
    })

    const wrapper = mount(SkillsView, { attachTo: document.body })
    await vi.waitFor(() => expect(wrapper.find('.usage-block').exists()).toBe(true))

    const heads = wrapper.findAll('.usage-head').map((node) => node.text())
    expect(heads).toEqual(['别名', '触发条件'])

    const spans = wrapper.findAll('.usage-span').map((node) => node.text())
    expect(spans).toContain('/flow-review')
    expect(spans).toContain('/fcr')
    expect(spans).toContain('/flow-review --entry OrderService.createOrder --req docs/PRD-order.md')
    // yaml 片段不会被当成用法
    expect(spans.some((span) => span.includes('harness_lifecycle'))).toBe(false)

    wrapper.unmount()
  })

  it('描述里没有用法结构时给出提示', async () => {
    const wrapper = await mountView()
    await vi.waitFor(() => expect(wrapper.find('.detail-view').exists()).toBe(true))

    expect(wrapper.findAll('.usage-block')).toHaveLength(0)
    expect(wrapper.find('.detail-view').text()).toContain('该 skill 没有在描述里写明调用方式')

    wrapper.unmount()
  })
})

describe('SkillsView 筛选', () => {
  it('按名称过滤，并在无结果时给出空态', async () => {
    const wrapper = await mountView()
    expect(wrapper.findAll('.skill-item')).toHaveLength(2)

    const search = wrapper.find('.filter-search')
    await search.setValue('beta')
    expect(wrapper.findAll('.skill-item')).toHaveLength(1)
    expect(wrapper.find('.item-name').text()).toBe('beta')

    await search.setValue('no-such-skill')
    expect(wrapper.findAll('.skill-item')).toHaveLength(0)
    expect(wrapper.text()).toContain('没有匹配的 skill')

    // 清除筛选后列表恢复
    await wrapper.find('.filter-clear').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('.skill-item')).toHaveLength(2)

    wrapper.unmount()
  })

  it('「只看问题」过滤掉全部已启用的 skill', async () => {
    const wrapper = await mountView()

    // 「只看问题」收纳在筛选面板里：先展开，再勾选。
    await wrapper.find('.filter-toggle').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.find('.filter-tool-issues').trigger('click')
    await wrapper.vm.$nextTick()

    // INDEX 里 alpha 全绿、beta 分叉 → 只看问题只剩 beta
    const items = wrapper.findAll('.skill-item')
    expect(items).toHaveLength(1)
    expect(items[0].find('.item-name').text()).toBe('beta')

    // 视图里以可关闭的 pill 呈现
    expect(wrapper.find('.filter-pill').exists()).toBe(true)

    wrapper.unmount()
  })

  it('按工具筛选：勾选一个未启用该工具的工具后列表清空', async () => {
    const wrapper = await mountView()

    await wrapper.find('.filter-toggle').trigger('click')
    await wrapper.vm.$nextTick()

    const names = wrapper.findAll('.filter-tool').map((node) => node.text())
    expect(names).toContain('Claude Code')
    expect(names).toContain('Cursor')
    expect(names).toContain('Codex')

    // alpha 在 claude-code 上已启用；再勾选一个它没启用的工具，列表应变空
    const cursor = wrapper
      .findAll('.filter-tool')
      .find((node) => node.text() === 'Cursor')!
    await cursor.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('.skill-item')).toHaveLength(0)
    expect(wrapper.text()).toContain('没有匹配的 skill')

    wrapper.unmount()
  })

  it('路径框提供一键复制', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.assign(navigator, { clipboard: { writeText } })

    const wrapper = await mountView()
    await vi.waitFor(() => expect(wrapper.find('.path-box').exists()).toBe(true))

    expect(wrapper.find('.path-value').text()).toBe(doc('alpha').summary.dir)

    await wrapper.find('.path-copy').trigger('click')
    expect(writeText).toHaveBeenCalledWith(doc('alpha').summary.dir)

    wrapper.unmount()
  })
})

describe('SkillsView 首次扫描骨架', () => {
  it('扫描未完成时显示骨架，完成后换成真实列表', async () => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')

    // 挂起 scan，模拟首帧：此时应看到骨架而不是空白。
    let release: (value: SkillIndex) => void = () => {}
    api.skillsScan.mockReturnValue(
      new Promise<SkillIndex>((resolve) => {
        release = resolve
      }),
    )
    api.skillRead.mockImplementation((name: string) => Promise.resolve(doc(name)))

    const wrapper = mount(SkillsView, { attachTo: document.body })
    await nextTick()

    // 骨架应已渲染，且不带真实 skill 条目
    expect(wrapper.find('ul.skill-list[aria-busy="true"]').exists()).toBe(true)
    expect(wrapper.findAll('.skeleton-item')).toHaveLength(6)
    expect(wrapper.findAll('.skill-item:not(.skeleton-item)')).toHaveLength(0)
    // 骨架阶段不应出现空态提示（那是「真的没有 skill」）
    expect(wrapper.text()).not.toContain('还没有 skill')

    release(INDEX)
    await vi.waitFor(() => expect(wrapper.findAll('.skeleton-item')).toHaveLength(0))
    expect(wrapper.findAll('.skill-item')).toHaveLength(2)
    expect(wrapper.find('ul.skill-list[aria-busy="true"]').exists()).toBe(false)

    wrapper.unmount()
  })
})
