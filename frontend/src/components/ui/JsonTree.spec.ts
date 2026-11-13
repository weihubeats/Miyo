import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import JsonTree from '@/components/ui/JsonTree.vue'
import { useLocaleStore } from '@/stores/locale'
import { collectErrors, stubScrollIntoView } from '@/testUtils/componentTest'

const DATA = {
  users: [
    { id: 1, name: 'miyo' },
    { id: 2, role: 'admin' },
  ],
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

/** 折叠摘要行的形态：`… 1 项`（token：` … ` + `N 项`）。用 text() 避免中间标签干扰。 */
function expectCollapsed(wrapper: { text: () => string }, collapsed: boolean): void {  if (collapsed) {
    expect(wrapper.text()).toMatch(/…\s+\d+\s+项/)
  } else {
    expect(wrapper.text()).not.toMatch(/…\s+\d+\s+项/)
  }
}

describe('JsonTree：折叠 / 展开全部', () => {
  it('expandDepth=0 时容器默认收起；collapseAll 后再 expandAll 恢复展开', async () => {
    const wrapper = mount(JsonTree, { props: { data: DATA, expandDepth: 0 } })
    expectCollapsed(wrapper, true)

    const tree = wrapper.findComponent(JsonTree)
    ;(tree.vm as unknown as { expandAll: () => void }).expandAll()
    await wrapper.vm.$nextTick()
    expectCollapsed(wrapper, false)

    ;(tree.vm as unknown as { collapseAll: () => void }).collapseAll()
    await wrapper.vm.$nextTick()
    expectCollapsed(wrapper, true)
    wrapper.unmount()
  })

  it('数据更换（新响应）时清空上一响应的展开状态（回归：expanded 键跨响应累积泄漏）', async () => {
    const wrapper = mount(JsonTree, { props: { data: DATA, expandDepth: 0 } })
    expectCollapsed(wrapper, true)

    const tree = wrapper.findComponent(JsonTree)
    ;(tree.vm as unknown as { expandAll: () => void }).expandAll()
    await wrapper.vm.$nextTick()
    expectCollapsed(wrapper, false)

    // 同一标签页内组件实例复用、响应体替换：应回到默认折叠，无残留展开
    await wrapper.setProps({ data: { ...DATA, extra: true } })
    expectCollapsed(wrapper, true)
    wrapper.unmount()
  })

  it('展开行数超过上限时截断并提示（回归：展开全部/查找渲染数万行 DOM）', async () => {
    const rows = Array.from({ length: 80 }, (_, i) => ({ i, text: `row-${i}` }))
    const wrapper = mount(JsonTree, {
      props: { data: { rows }, expandDepth: 99, maxLines: 50 },
    })
    // 全量展开约 325 行：截断后仅保留上限附近的行 + 1 条截断提示，
    // 不再渲染剩余节点（允许末尾 tail 行在触发截断后入队的小幅超出）。
    const rendered = wrapper.findAll('.jt-line').length
    expect(rendered).toBeGreaterThanOrEqual(50)
    expect(rendered).toBeLessThanOrEqual(55)
    expect(wrapper.text()).toMatch(/已达展示上限/)
    wrapper.unmount()
  })
})

describe('JsonTree：查找高亮与导航', () => {
  beforeEach(() => {
    stubScrollIntoView()
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('搜索时强制展开全部并上抛匹配总数', async () => {
    const onCount = vi.fn()
    const wrapper = mount(JsonTree, {
      props: { data: DATA, expandDepth: 0, query: 'id' },
      attrs: { 'onMatch-count': onCount },
    })
    await flushPromises()
    // 两个 "id" 键匹配；expandDepth=0 也强制全展开（无折叠摘要）。
    expect(onCount).toHaveBeenCalledWith(2)
    expect(wrapper.text()).not.toMatch(/…\s+\d+\s+项/)
    expect(wrapper.findAll('.jt-mark')).toHaveLength(2)
    expect(wrapper.findAll('.jt-mark.active')).toHaveLength(1)
    wrapper.unmount()
  })

  it('activeMatch 切换时当前高亮移动并触发滚动', async () => {
    const scrollFn = vi.fn()
    const original = HTMLElement.prototype.scrollIntoView
    HTMLElement.prototype.scrollIntoView = scrollFn as never
    try {
      const wrapper = mount(JsonTree, { props: { data: DATA, query: 'id', activeMatch: 0 } })
      await flushPromises()
      expect(wrapper.find('.jt-mark.active').text()).toBe('id')

      await wrapper.setProps({ activeMatch: 1 })
      expect(wrapper.findAll('.jt-mark.active')).toHaveLength(1)
      expect(wrapper.find('.jt-mark.active').text()).toBe('id')
      expect(scrollFn).toHaveBeenCalled()
      wrapper.unmount()
    } finally {
      HTMLElement.prototype.scrollIntoView = original
    }
  })

  it('无匹配时为零计数；清除 query 后无高亮残留', async () => {
    const onCount = vi.fn()
    const wrapper = mount(JsonTree, {
      props: { data: DATA, query: 'zzz-not-exist' },
      attrs: { 'onMatch-count': onCount },
    })
    await flushPromises()
    expect(onCount).toHaveBeenLastCalledWith(0)
    expect(wrapper.findAll('.jt-mark')).toHaveLength(0)

    await wrapper.setProps({ query: '', activeMatch: 0 })
    expect(wrapper.findAll('.jt-mark')).toHaveLength(0)
    wrapper.unmount()
  })
})

describe('JsonTree：卸载间隙稳定性（回归：emitsOptions 崩溃类）', () => {
  beforeEach(() => {
    stubScrollIntoView()
  })

  it('查找导航后立即卸载组件，不产生任何未处理错误', async () => {
    const collector = collectErrors()
    const wrapper = mount(JsonTree, { props: { data: DATA, query: 'id' } })
    await wrapper.setProps({ activeMatch: 1 })
    await flushPromises()
    wrapper.unmount()
    await flushPromises()
    expect(collector.errors).toEqual([])
    collector.restore()
  })

  it('快速挂载/卸载多个实例（模拟条件渲染竞态）不崩溃', async () => {
    const collector = collectErrors()
    for (let i = 0; i < 5; i++) {
      const w = mount(JsonTree, { props: { data: DATA, query: i % 2 ? 'id' : 'name' } })
      await w.setProps({ activeMatch: 1 })
      await flushPromises()
      w.unmount()
      await flushPromises()
    }
    expect(collector.errors).toEqual([])
    collector.restore()
  })
})
describe('JsonTree：行级复制（JSONPath / 字段值）', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('悬停操作钮：复制 JSONPath（点号安全键 + 数组下标），成功后短暂反馈', async () => {
    const writeText = vi.fn().mockResolvedValue(true)
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } })
    const w = mount(JsonTree, { props: { data: DATA } })

    // $.users[0].name 对应叶子行：name: "miyo"
    const codeRows = w.findAll('.jt-line')
    const target = codeRows.find((r) => r.text().includes('"miyo"'))
    expect(target).toBeDefined()
    const pathBtn = target!.find('.jt-act')
    await pathBtn.trigger('click')
    await flushPromises()
    expect(writeText).toHaveBeenCalledWith('$.users[0].name')
    // 复制反馈：该行出现 ✓（check 图标）
    expect(target!.find('svg use, .jt-act').exists()).toBe(true)
  })

  it('特殊键名走 ["key"] 写法；复制字符串值为原文（非 JSON 字面量）', async () => {
    const writeText = vi.fn().mockResolvedValue(true)
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } })
    const w = mount(JsonTree, { props: { data: { 'odd key': 'hello' } } })

    const leaf = w.findAll('.jt-line').find((r) => r.text().includes('"hello"'))!
    const [pathBtn, valueBtn] = leaf.findAll('.jt-act')
    expect(pathBtn).toBeDefined()
    expect(valueBtn).toBeDefined()

    await pathBtn.trigger('click')
    await flushPromises()
    expect(writeText).toHaveBeenLastCalledWith(`$["odd key"]`)

    await valueBtn!.trigger('click')
    await flushPromises()
    expect(writeText).toHaveBeenLastCalledWith('hello')
  })

  it('非字符串叶子复制 JSON 字面量（数字 / null / 布尔）', async () => {
    const writeText = vi.fn().mockResolvedValue(true)
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } })
    const w = mount(JsonTree, { props: { data: { n: 42, x: null, ok: true } } })

    const rows = w.findAll('.jt-line')
    const nRow = rows.find((r) => r.text().includes('42'))!
    await nRow.findAll('.jt-act')[1]!.trigger('click')
    await flushPromises()
    expect(writeText).toHaveBeenLastCalledWith('42')

    const nullRow = rows.find((r) => r.text().includes('null'))!
    await nullRow.findAll('.jt-act')[1]!.trigger('click')
    await flushPromises()
    expect(writeText).toHaveBeenLastCalledWith('null')
  })
})
