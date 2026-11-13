import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import ResponsePanel from '@/domains/endpoint/components/ResponsePanel.vue'
import { useLocaleStore } from '@/stores/locale'
import type { ExecuteResponse } from '@/types/miyoApi'
import { collectErrors, stubScrollIntoView } from '@/testUtils/componentTest'
import { copyText } from '@/utils/clipboard'

vi.mock('@/utils/clipboard.ts', () => ({
  copyText: vi.fn(async () => true),
}))

function makeResponse(body: string): ExecuteResponse {
  return {
    status: 201,
    headers: [['content-type', 'application/json']],
    body,
    content_type: 'application/json',
    duration_ms: 769,
    size_bytes: 83,
    truncated: false,
  }
}

/** 按语义定位复制按钮（第二层常驻操作）。 */
function findCopyBtn(wrapper: ReturnType<typeof mount>) {
  return wrapper
    .findAll('.rp-icon-btn')
    .find((b) => b.attributes('aria-label')?.includes('复制响应正文'))
}

/** 按语义定位「更多操作」按钮（低频操作收进 ⋯ 菜单）。 */
function findMoreBtn(wrapper: ReturnType<typeof mount>) {
  return wrapper
    .findAll('.rp-icon-btn')
    .find((b) => b.attributes('aria-label')?.includes('更多操作'))
}

/** 打开「更多」菜单并点选指定标签的操作项（菜单 teleport 到 body，用后清理）。 */
async function pickMoreItem(wrapper: ReturnType<typeof mount>, label: string): Promise<void> {
  const moreBtn = findMoreBtn(wrapper)
  expect(moreBtn).toBeTruthy()
  await moreBtn!.trigger('click')
  await wrapper.vm.$nextTick()
  const item = [...document.querySelectorAll<HTMLButtonElement>('.miyo-menu .miyo-menu-item')].find(
    (b) => b.textContent?.includes(label),
  )
  expect(item).toBeTruthy()
  item!.click()
  await flushPromises()
  document.querySelectorAll('.miyo-menu').forEach((el) => el.remove())
}

/** 切换视图模式（SegmentedControl：pretty=0 / raw=1 / tree=2 / preview=3…）。 */
async function switchMode(wrapper: ReturnType<typeof mount>, index: number): Promise<void> {
  await wrapper.findAll('.seg-item')[index]!.trigger('click')
  await wrapper.vm.$nextTick()
}

/** 通过 ⌘F 打开查找条并输入关键词。 */
async function openFind(wrapper: ReturnType<typeof mount>): Promise<void> {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', metaKey: true }))
  await wrapper.vm.$nextTick()
  const input = wrapper.find('.findbar-input')
  expect(input.exists()).toBe(true)
  await input.setValue('id')
  await flushPromises()
  // 查找词经 160ms 防抖后才驱动高亮/计数
  await new Promise((r) => setTimeout(r, 200))
  await flushPromises()
}

describe('ResponsePanel：状态栏元数据', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('渲染 状态码 / 耗时 / 大小，且带 tone-ok 强调', () => {
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse('{}') } })
    expect(wrapper.find('.rp').classes()).toContain('tone-ok')
    expect(wrapper.text()).toContain('201 Created')
    expect(wrapper.text()).toContain('769.00 ms')
    expect(wrapper.text()).toContain('83 B')
    expect(wrapper.text()).toContain('application/json')
    wrapper.unmount()
  })

  it('4xx/5xx 状态呈现 err 强调', () => {
    const resp = makeResponse('{}')
    resp.status = 500
    const wrapper = mount(ResponsePanel, { props: { response: resp } })
    expect(wrapper.find('.rp').classes()).toContain('tone-err')
    expect(wrapper.text()).toContain('Internal Server Error')
    wrapper.unmount()
  })
})

describe('ResponsePanel：查找（Find in Response）', () => {
  beforeEach(() => {
    stubScrollIntoView()
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('⌘F 打开查找条 → 高亮全部 + n/N 计数 → 上一个/下一个切换 → Esc 关闭并清空', async () => {
    const wrapper = mount(ResponsePanel, {
      props: { response: makeResponse(JSON.stringify({ users: [{ id: 1 }, { id: 2 }] })) },
    })
    // 查找在树视图口径（treeTotal 上报）：切到 Tree 模式
    await switchMode(wrapper, 2)

    await openFind(wrapper)
    expect(wrapper.text()).toContain('1 / 2')
    expect(wrapper.findAll('.jt-mark')).toHaveLength(2)
    expect(wrapper.findAll('.jt-mark.active')).toHaveLength(1)

    // 下一个（第 2 个按钮） → 2 / 2
    await wrapper.findAll('.findbar-btn')[1].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('2 / 2')
    expect(wrapper.findAll('.jt-mark.active')).toHaveLength(1)

    // 上一个（第 1 个按钮） → 回到 1 / 2
    await wrapper.findAll('.findbar-btn')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('1 / 2')

    // Esc 关闭并清空搜索词
    await wrapper.find('.findbar-input').trigger('keydown', { key: 'Escape' })
    await fallbackTick()
    expect(wrapper.find('.findbar').exists()).toBe(false)
    expect(wrapper.findAll('.jt-mark')).toHaveLength(0)
    wrapper.unmount()
  })

  it('无匹配时显示「无匹配」且高亮为零', async () => {
    const wrapper = mount(ResponsePanel, {
      props: { response: makeResponse(JSON.stringify({ users: [{ id: 1 }] })) },
    })
    await openFind(wrapper)
    await wrapper.find('.findbar-input').setValue('zzz')
    await flushPromises()
    await new Promise((r) => setTimeout(r, 200))
    await flushPromises()
    expect(wrapper.text()).toContain('无匹配')
    expect(wrapper.findAll('.jt-mark')).toHaveLength(0)
    wrapper.unmount()
  })

  it('输入其他组件（如文本框）时 ⌘F 不拦截', async () => {
    const wrapper = mount(ResponsePanel, {
      props: { response: makeResponse(JSON.stringify({ id: 1 })) },
    })
    document.body.appendChild(createInput())
    const input = document.body.querySelector('input.fake') as HTMLInputElement
    input.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'f', metaKey: true, bubbles: true }),
    )
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.findbar').exists()).toBe(false)
    input.remove()
    wrapper.unmount()
  })
})

describe('ResponsePanel：复制响应正文', () => {
  it('Tree 视图复制源为原始 body 而非格式化文本（回归）', async () => {
    const body = JSON.stringify({ users: [{ id: 1 }, { id: 2 }] })
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse(body) } })
    await switchMode(wrapper, 2)

    expect(wrapper.find('.jt-line').exists()).toBe(true)

    await findCopyBtn(wrapper)!.trigger('click')

    expect(vi.mocked(copyText)).toHaveBeenCalledTimes(1)
    expect(vi.mocked(copyText)).toHaveBeenCalledWith(body)
    wrapper.unmount()
  })

  it('raw 视图复制源为原始 body', async () => {
    const body = JSON.stringify({ a: 1 })
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse(body) } })
    await switchMode(wrapper, 1)

    await findCopyBtn(wrapper)!.trigger('click')
    expect(vi.mocked(copyText)).toHaveBeenCalledWith(body)
    wrapper.unmount()
  })
})

describe('ResponsePanel：大响应保护', () => {
  it('超过 10 万行时提示截断，且行数组不再全量驻留（回归：rawLines 全量 split）', async () => {
    const body = Array.from({ length: 100_002 }, (_, i) => `line-${i}`).join('\n')
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse(body) } })
    // 非 JSON 文本 → pretty 视图退化为行视图；提示出现且初始仅渲染首块 1000 行。
    expect(wrapper.text()).toMatch(/超出部分未展示/)
    expect(wrapper.findAll('.rp-line').length).toBeLessThanOrEqual(1000)
    wrapper.unmount()
  })

  it('压缩 JSON 单行超长：截断到 10 万字符 + 提示，且不做语法高亮（回归：单行全量分词冻结）', () => {
    // 压缩 JSON 常见「整页一行」且超 1MB（跳过 parse/树）→ 原实现对该行全量 jsonTokens 分词 → 主线程假死。
    const line = `{"data":"${'x'.repeat(1_100_000)}"}`
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse(line) } })

    expect(wrapper.text()).toMatch(/超长单行/)
    const first = wrapper.find('.rp-line-text')
    expect(first.exists()).toBe(true)
    expect(first.text().length).toBeLessThanOrEqual(100_000)
    // 超长行走纯转义：无语法高亮 span。
    expect(first.html()).not.toContain('<span class="hl-')
    wrapper.unmount()
  })

  it('HTML 预览超 1MB 不渲染 iframe（防渲染进程卡死），保留过大提示', async () => {
    const resp = makeResponse(`<html>${'a'.repeat(1_100_000)}</html>`)
    resp.content_type = 'text/html'
    const wrapper = mount(ResponsePanel, { props: { response: resp } })

    // 切到「预览」段（pretty / raw / preview）
    await wrapper.findAll('.seg-item')[2].trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('iframe').exists()).toBe(false)
    expect(wrapper.text()).toMatch(/超过 1 MB/)
    wrapper.unmount()
  })
})

describe('ResponsePanel：全文缓存回退（body_omitted）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('IPC 只传预览时先经 loadFullBody 取全文再复制', async () => {
    const resp = makeResponse('"preview"')
    resp.body_omitted = true
    const loadFullBody = vi.fn(async () => '"full-body"')
    const wrapper = mount(ResponsePanel, { props: { response: resp, loadFullBody } })

    await findCopyBtn(wrapper)!.trigger('click')
    expect(loadFullBody).toHaveBeenCalledTimes(1)
    expect(vi.mocked(copyText)).toHaveBeenCalledWith('"full-body"')
    wrapper.unmount()
  })

  it('全文缓存过期（loadFullBody 返回 null）：回退复制截断预览', async () => {
    const resp = makeResponse('"preview"')
    resp.body_omitted = true
    const loadFullBody = vi.fn(async () => null)
    const wrapper = mount(ResponsePanel, { props: { response: resp, loadFullBody } })

    await findCopyBtn(wrapper)!.trigger('click')
    expect(vi.mocked(copyText)).toHaveBeenCalledWith('"preview"')
    wrapper.unmount()
  })
})

describe('ResponsePanel：展开全部 / 收起全部', () => {
  it('切换 JSON 树全部节点的展开状态', async () => {
    const wrapper = mount(ResponsePanel, {
      props: { response: makeResponse(JSON.stringify({ a: { b: { c: 1 } } })) },
    })
    await switchMode(wrapper, 2)
    expect(wrapper.find('.jt-line').exists()).toBe(true)

    // 展开/收起已收进「更多」菜单（O(1) 标记，动作随当前树状态切换）
    await pickMoreItem(wrapper, '展开全部')
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).not.toMatch(/…\s+\d+\s+项/)

    await pickMoreItem(wrapper, '收起全部')
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toMatch(/…\s+\d+\s+项/)
    wrapper.unmount()
  })
})

describe('ResponsePanel：卸载间隙稳定性（回归：emitsOptions 崩溃类）', () => {
  beforeEach(() => {
    stubScrollIntoView()
  })

  it('查找导航后立即卸载 / 重新挂载，不产生未处理错误', async () => {
    const collector = collectErrors()
    for (let i = 0; i < 3; i++) {
      const wrapper = mount(ResponsePanel, {
        props: { response: makeResponse(JSON.stringify({ users: [{ id: 1 }, { id: 2 }] })) },
      })
      await openFind(wrapper)
      await wrapper.findAll('.findbar-btn')[1].trigger('click')
      wrapper.unmount()
      await flushPromises()
    }
    expect(collector.errors).toEqual([])
    collector.restore()
  })
})

/** 请求快照样本（Rust `RequestSnapshot` 镜像：变量渲染 / 认证注入后的终值）。 */
function makeSnapshot(): NonNullable<ExecuteResponse['request']> {
  return {
    method: 'POST',
    url: 'https://api.example.com/v1/users?page=2',
    url_template: 'https://{{host}}/v1/users?page={{page}}',
    headers: [
      ['content-type', 'application/json'],
      ['authorization', 'Bearer token'],
    ],
    cookies: [['sid', 'abc123']],
    body: '{"name":"miyo"}',
    body_omitted: false,
  }
}

describe('ResponsePanel：「请求」页签（本次实际发出的报文）', () => {
  beforeEach(() => {
    stubScrollIntoView()
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('无请求快照（旧后端 / gRPC）时不占位：页签列表无「请求」', () => {
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse('{}') } })
    expect(wrapper.findAll('.tab-label').map((n) => n.text())).not.toContain('请求')
    wrapper.unmount()
  })

  it('展示 URL / 模板 / 请求头 / Cookie / 请求体', async () => {
    const wrapper = mount(ResponsePanel, {
      props: { response: { ...makeResponse('{}'), request: makeSnapshot() } },
    })
    expect(wrapper.findAll('.tab-label').at(-1)!.text()).toBe('请求')
    await wrapper.findAll('.tab').at(-1)!.trigger('click')
    expect(wrapper.find('.rp-req-url').text()).toBe('https://api.example.com/v1/users?page=2')
    expect(wrapper.find('.rp-req-template').text()).toContain('https://{{host}}/v1/users')
    // 2 个请求头 + 1 个 Cookie 共用同一 key/value 行样式
    expect(wrapper.findAll('.rp-header-row')).toHaveLength(3)
    expect(wrapper.text()).toContain('Bearer token')
    expect(wrapper.find('.rp-req-body').text()).toBe('{"name":"miyo"}')
    wrapper.unmount()
  })

  it('请求体被截断时给出提示', async () => {
    const snap = makeSnapshot()
    snap.body_omitted = true
    const wrapper = mount(ResponsePanel, {
      props: { response: { ...makeResponse('{}'), request: snap } },
    })
    await wrapper.findAll('.tab').at(-1)!.trigger('click')
    expect(wrapper.find('.rp-req-body').exists()).toBe(true)
    expect(wrapper.find('.rp-note').exists()).toBe(true)
    wrapper.unmount()
  })

  it('URL 模板与实际一致时不展示模板行', async () => {
    const snap = makeSnapshot()
    snap.url_template = snap.url
    const wrapper = mount(ResponsePanel, {
      props: { response: { ...makeResponse('{}'), request: snap } },
    })
    await wrapper.findAll('.tab').at(-1)!.trigger('click')
    expect(wrapper.find('.rp-req-template').exists()).toBe(false)
    wrapper.unmount()
  })
})

function createInput(): HTMLInputElement {
  const el = document.createElement('input')
  el.className = 'fake'
  return el
}

/** jsdom 下额外推一帧，确保 Teleport/Transition 缓冲期完成。 */
async function fallbackTick(): Promise<void> {
  await flushPromises()
  await new Promise((r) => setTimeout(r, 0))
}
describe('ResponsePanel：Pretty 语法高亮按 Content-Type 分派', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('JSON 响应：行视图带 hl-k/hl-s/hl-n/hl-b/hl-null token（回归：响应区漏配色全白）', () => {
    const body = JSON.stringify({ name: 'miyo', n: 1, ok: true, x: null })
    const wrapper = mount(ResponsePanel, { props: { response: makeResponse(body) } })
    const html = wrapper.find('.rp-lines').html()
    expect(html).toContain('hl-k') // 键
    expect(html).toContain('hl-s') // 字符串
    expect(html).toContain('hl-n') // 数字
    expect(html).toContain('hl-b') // 布尔
    expect(html).toContain('hl-null') // null
    wrapper.unmount()
  })

  it('application/problem+json 同样按 JSON 高亮（MIME 后缀不漏判）', () => {
    const resp = makeResponse(JSON.stringify({ title: 'boom' }))
    resp.content_type = 'application/problem+json'
    const wrapper = mount(ResponsePanel, { props: { response: resp } })
    expect(wrapper.find('.rp-lines').html()).toContain('hl-k')
    wrapper.unmount()
  })

  it('text/plain 纯文本：不套 JSON token span', () => {
    const resp = makeResponse('plain text {"not":"json"}')
    resp.content_type = 'text/plain'
    const wrapper = mount(ResponsePanel, { props: { response: resp } })
    expect(wrapper.find('.rp-line-text').html()).not.toContain('class="hl-')
    wrapper.unmount()
  })

  it('XML 响应走标签高亮（hl-k 标签名），HTML 同样不吃 JSON 键色', () => {
    const xml = makeResponse('<root><name>miyo</name></root>')
    xml.content_type = 'application/xml'
    const w1 = mount(ResponsePanel, { props: { response: xml } })
    expect(w1.find('.rp-line-text').html()).toContain('hl-k')
    w1.unmount()

    const html = makeResponse('<p class="x">hi</p>')
    html.content_type = 'text/html'
    const w2 = mount(ResponsePanel, { props: { response: html } })
    const h2 = w2.find('.rp-line-text').html()
    expect(h2).toContain('hl-k') // 标签名 p
    expect(h2).toContain('hl-s') // 属性值 "x"
    w2.unmount()
  })
})
