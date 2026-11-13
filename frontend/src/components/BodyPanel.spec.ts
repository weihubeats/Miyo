/**
 * BodyPanel 单测：raw JSON 编辑 + 格式化的数据流（回归：格式化使用了修改前的旧内容）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import BodyPanel from './BodyPanel.vue'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import { resetBodyDrafts } from '../utils/bodyDrafts'
import type { Endpoint } from '../types/miyoApi'

// 草稿表按接口 id 隔离且活在模块作用域：夹具 id 固定，用例间必须重置。
beforeEach(resetBodyDrafts)

function jsonDraft(raw: string): Endpoint {
  const d = makeDraft()
  d.request.body = { mode: 'json', raw }
  return reactive(d)
}

/** 读取草稿 json body 的 raw（联合类型窄化辅助）。 */
function rawOf(d: Endpoint): string {
  return (d.request.body as { raw: string }).raw
}

function mountPanel(draft: Endpoint) {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  return mount(BodyPanel, { props: { draft } })
}

/** 美化 / 压缩按钮已是纯图标（title 提示），按 title 定位。 */
function prettyBtnOf(wrapper: ReturnType<typeof mountPanel>) {
  return wrapper.findAll('button').find((b) => b.attributes('title') === '美化（格式化 JSON）')
}
function compactBtnOf(wrapper: ReturnType<typeof mountPanel>) {
  return wrapper.findAll('button').find((b) => b.attributes('title') === '压缩 JSON')
}

describe('BodyPanel：raw JSON 编辑与格式化', () => {
  it('编辑后点击「格式化」应格式化最新内容', async () => {
    const draft = jsonDraft('{"a":1}')
    const wrapper = mountPanel(draft)

    // raw + JSON 子类型 → 渲染 JsonEditor（工具条含「格式化」按钮）
    const formatBtn = prettyBtnOf(wrapper)
    expect(formatBtn).toBeDefined()
    expect(wrapper.findComponent({ name: 'JsonEditor' }).exists()).toBe(true)

    // 模拟用户编辑 textarea（input → v-model 写回草稿）
    const ta = wrapper.find('textarea')
    await ta.setValue('{"name":"alice","age":18}')
    expect(rawOf(draft)).toBe('{"name":"alice","age":18}')

    await formatBtn!.trigger('click')
    expect(rawOf(draft)).toBe(
      JSON.stringify({ name: 'alice', age: 18 }, null, 2),
    )
  })

  it('「美化」使用编辑后的最新内容', async () => {
    const draft = jsonDraft('{ "a": 1 }')
    const wrapper = mountPanel(draft)
    const ta = wrapper.find('textarea')
    await ta.setValue('{ "b": 2, "c": 3 }')
    await prettyBtnOf(wrapper)!.trigger('click')
    expect(rawOf(draft)).toBe('{\n  "b": 2,\n  "c": 3\n}')
  })

  it('「压缩」按钮输出紧凑 JSON；顶部工具栏显示校验状态', async () => {
    const draft = jsonDraft('{ "a": 1 }')
    const wrapper = mountPanel(draft)
    const ta = wrapper.find('textarea')
    await ta.setValue('{ "b": 2, "c": 3 }')
    await compactBtnOf(wrapper)!.trigger('click')
    expect(rawOf(draft)).toBe('{"b":2,"c":3}')

    // 校验状态徽标：有效（已外移到模式栏右侧，非悬浮层、不在编辑器工具栏内）
    expect(wrapper.find('.hl-float').exists()).toBe(false)
    expect(wrapper.find('.je-status').exists()).toBe(false)
    expect(wrapper.find('.bp-json-status').text()).toContain('JSON 有效')
  })

  it('回归：格式化保留重复键（多个 "body" 键不丢失）', async () => {
    const draft = jsonDraft(
      '{"title":"测试标题","body":"测试内容","body":"测试内容","body":"测试内容","body":"测试内容","userId":1}',
    )
    const wrapper = mountPanel(draft)
    await prettyBtnOf(wrapper)!.trigger('click')
    expect(rawOf(draft)).toBe(`{
  "title": "测试标题",
  "body": "测试内容",
  "body": "测试内容",
  "body": "测试内容",
  "body": "测试内容",
  "userId": 1
}`)
  })

  it('回归：DOM 已更新但 input 事件丢失时，格式化仍以 textarea 实际内容为准', async () => {
    const draft = jsonDraft('{"a":1}')
    const wrapper = mountPanel(draft)
    const ta = wrapper.find('textarea')

    // 直接改 DOM 值、不派发 input 事件（模拟真实浏览器中事件丢失/延迟的失同步），
    // 此时 props.modelValue 仍是旧值 {"a":1}
    ta.element.value = '{"name":"alice","age":18}'
    await prettyBtnOf(wrapper)!.trigger('click')

    expect(rawOf(draft)).toBe(
      JSON.stringify({ name: 'alice', age: 18 }, null, 2),
    )
  })

  it('在请求体中查找：打开 FindBar 并高亮与计数', async () => {
    const draft = jsonDraft('{"user":"miyo","name":"miyo"}')
    const wrapper = mountPanel(draft)

    // 点击搜索图标按钮打开 FindBar
    const searchBtn = wrapper.find('.bp-icon-btn')
    expect(searchBtn.exists()).toBe(true)
    await searchBtn.trigger('click')

    expect(wrapper.findComponent({ name: 'FindBar' }).exists()).toBe(true)

    // 输入查找词
    const findInput = wrapper.find('.findbar-input')
    await findInput.setValue('miyo')

    // 等待防抖
    await new Promise((r) => setTimeout(r, 200))
    await wrapper.vm.$nextTick()

    // JsonEditor 渲染 mark 高亮
    expect(wrapper.findAll('.rp-find-mark').length).toBe(2)
    expect(wrapper.findAll('.rp-find-mark.active').length).toBe(1)

    // 切换下一个匹配（Body 的 FindBar 带 replaceable，首枚是替换行展开钮，按 title 定位）
    const nextBtn = wrapper
      .findAll('.findbar-btn')
      .find((b) => b.attributes('title') === '下一个 (Enter)')!
    await nextBtn.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.findbar-count').text()).toContain('2 / 2')

    // Esc 关闭
    await findInput.trigger('keydown', { key: 'Escape' })
    expect(wrapper.findComponent({ name: 'FindBar' }).exists()).toBe(false)
  })

  it('在请求体中全部替换：JSON 草稿即时回写，计数同步刷新', async () => {
    const draft = jsonDraft('{"user":"miyo","name":"miyo"}')
    const wrapper = mountPanel(draft)

    await wrapper.find('.bp-icon-btn').trigger('click')
    await wrapper.find('.findbar-expander').trigger('click')
    const inputs = wrapper.findAll('.findbar-input')
    await inputs[0]!.setValue('miyo')
    await inputs[1]!.setValue('kano')
    // 「全部替换」= 替换行第二个按钮
    await wrapper.findAll('.findbar-replace-btn')[1]!.trigger('click')

    // 替换按即时 query（非防抖值）执行，草稿即刻回写
    expect(rawOf(draft)).toBe('{"user":"kano","name":"kano"}')
    // 替换后无残留匹配
    expect(wrapper.find('.findbar-count').text()).toContain('无匹配')
  })

  it('在 raw 文本（非 JSON 子类型）中全部替换：经防抖镜像落库', async () => {
    const d = makeDraft()
    d.request.body = { mode: 'text', raw: 'a=1&b=2&c=1' }
    const draft = reactive(d)
    const wrapper = mountPanel(draft)

    await wrapper.find('.bp-icon-btn').trigger('click')
    await wrapper.find('.findbar-expander').trigger('click')
    const inputs = wrapper.findAll('.findbar-input')
    await inputs[0]!.setValue('1')
    await inputs[1]!.setValue('9')
    await wrapper.findAll('.findbar-replace-btn')[1]!.trigger('click')

    expect(rawOf(draft)).toBe('a=9&b=2&c=9')
  })

  it('替换内容为空 = 删除全部匹配；查找词为空时不动作', async () => {
    const draft = jsonDraft('{"a":"x","b":"x"}')
    const wrapper = mountPanel(draft)
    await wrapper.find('.bp-icon-btn').trigger('click')
    await wrapper.find('.findbar-expander').trigger('click')
    const inputs = wrapper.findAll('.findbar-input')
    await inputs[0]!.setValue('x')
    await inputs[1]!.setValue('')
    await wrapper.findAll('.findbar-replace-btn')[1]!.trigger('click')
    expect(rawOf(draft)).toBe('{"a":"","b":""}')

    // 查找词为空：替换按钮无效果
    await inputs[0]!.setValue('')
    await inputs[1]!.setValue('y')
    await wrapper.findAll('.findbar-replace-btn')[1]!.trigger('click')
    expect(rawOf(draft)).toBe('{"a":"","b":""}')
  })

  it('在请求体中逐个替换：「替换」替换当前匹配并推进到下一个', async () => {
    const draft = jsonDraft('{"user":"miyo","name":"miyo"}')
    const wrapper = mountPanel(draft)

    await wrapper.find('.bp-icon-btn').trigger('click')
    await wrapper.find('.findbar-expander').trigger('click')
    const inputs = wrapper.findAll('.findbar-input')
    await inputs[0]!.setValue('miyo')
    await inputs[1]!.setValue('kano')

    // 第一次「替换」：替换第 1 处，激活序号停在原位即指向下一处
    await wrapper.findAll('.findbar-replace-btn')[0]!.trigger('click')
    expect(rawOf(draft)).toBe('{"user":"kano","name":"miyo"}')
    expect(wrapper.find('.findbar-count').text()).toContain('1 / 1')

    // 再替换一次：清零
    await wrapper.findAll('.findbar-replace-btn')[0]!.trigger('click')
    expect(rawOf(draft)).toBe('{"user":"kano","name":"kano"}')
    expect(wrapper.find('.findbar-count').text()).toContain('无匹配')
  })

  it('在 raw 文本（非 JSON 子类型）中逐个替换：镜像落库且选区推进', async () => {
    const d = makeDraft()
    d.request.body = { mode: 'text', raw: 'a=1&b=2&c=1' }
    const draft = reactive(d)
    const wrapper = mountPanel(draft)

    await wrapper.find('.bp-icon-btn').trigger('click')
    await wrapper.find('.findbar-expander').trigger('click')
    const inputs = wrapper.findAll('.findbar-input')
    await inputs[0]!.setValue('1')
    await inputs[1]!.setValue('9')
    await wrapper.findAll('.findbar-replace-btn')[0]!.trigger('click')

    expect(rawOf(draft)).toBe('a=9&b=2&c=1')
  })

  it('快捷键 Ctrl/Cmd+R：打开查找并直接展开替换行', async () => {
    const draft = jsonDraft('{"a":1}')
    // window 级监听：事件必须经真实文档树冒泡（attachTo），游离挂载收不到
    const wrapper = mount(BodyPanel, { props: { draft }, attachTo: document.body })
    await wrapper.find('textarea').trigger('keydown', { key: 'r', ctrlKey: true })
    expect(wrapper.findComponent({ name: 'FindBar' }).exists()).toBe(true)
    expect(wrapper.find('.findbar-replace-row').exists()).toBe(true)
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('form-data→无→form-data：字段记忆还原', async () => {
    const draft = makeDraft({ id: 'ep-form-mem' })
    draft.request.body = {
      mode: 'multipart',
      fields: [{ key: 'a', value_type: 'text', value: '1', enabled: true }],
    }
    const wrapper = mountPanel(reactive(draft))
    const tabBtn = (label: string) =>
      wrapper.findAll('.seg-item').find((b) => b.text() === label)!

    await tabBtn('无').trigger('click')
    expect(draft.request.body.mode).toBe('none')

    await tabBtn('form-data').trigger('click')
    expect(draft.request.body).toEqual({
      mode: 'multipart',
      fields: [{ key: 'a', value_type: 'text', value: '1', enabled: true }],
    })
  })

  it('graphql→无→GraphQL：query 记忆还原', async () => {
    const draft = makeDraft({ id: 'ep-gql-mem' })
    draft.request.body = {
      mode: 'graphql',
      spec: { query: 'query { hero }', variables: '{}', operation_name: '' },
    }
    const wrapper = mountPanel(reactive(draft))
    const tabBtn = (label: string) =>
      wrapper.findAll('.seg-item').find((b) => b.text() === label)!

    await tabBtn('无').trigger('click')
    expect(draft.request.body.mode).toBe('none')

    await tabBtn('GraphQL').trigger('click')
    expect(draft.request.body).toEqual({
      mode: 'graphql',
      spec: { query: 'query { hero }', variables: '{}', operation_name: '' },
    })
  })

  it('binary→无→binary：文件路径记忆还原', async () => {
    const draft = makeDraft({ id: 'ep-bin-mem' })
    draft.request.body = { mode: 'binary', path: '/tmp/a.bin' }
    const wrapper = mountPanel(reactive(draft))
    const tabBtn = (label: string) =>
      wrapper.findAll('.seg-item').find((b) => b.text() === label)!

    await tabBtn('无').trigger('click')
    await tabBtn('binary').trigger('click')
    expect(draft.request.body).toEqual({ mode: 'binary', path: '/tmp/a.bin' })
  })

  it('urlencoded↔form-data 直接切换：保持实时转换，不读旧记忆', async () => {
    const draft = makeDraft({ id: 'ep-conv-mem' })
    draft.request.body = {
      mode: 'urlencoded',
      fields: [{ key: 'a', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    }
    const wrapper = mountPanel(reactive(draft))
    const tabBtn = (label: string) =>
      wrapper.findAll('.seg-item').find((b) => b.text() === label)!

    await tabBtn('form-data').trigger('click')
    expect(draft.request.body).toEqual({
      mode: 'multipart',
      fields: [{ key: 'a', value_type: 'text', value: '1', enabled: true }],
    })
  })

  it('raw(JSON)→无→raw：还原 JSON 子类型与文本，而非默认 text', async () => {
    const draft = jsonDraft('{"a":1}')
    draft.request.headers.push({ key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    const wrapper = mountPanel(draft)
    const tabBtn = (label: string) =>
      wrapper.findAll('.seg-item').find((b) => b.text() === label)!

    await tabBtn('无').trigger('click')
    expect(draft.request.body.mode).toBe('none')

    await tabBtn('raw').trigger('click')
    // 子类型 + 文本 + Content-Type 全部还原
    expect(draft.request.body).toEqual({ mode: 'json', raw: '{"a":1}' })
    expect(
      draft.request.headers.find((h) => h.key.toLowerCase() === 'content-type')?.value,
    ).toBe('application/json')
    // 编辑器回到 JSON 视图
    expect(wrapper.findComponent({ name: 'JsonEditor' }).exists()).toBe(true)
  })

  it('Tab 缩进：JsonEditor 内按 Tab 插入 2 空格（焦点不跳出）', async () => {
    const draft = jsonDraft('{\n}')
    const wrapper = mountPanel(draft)
    const ta = wrapper.find('textarea')
    const el = ta.element as HTMLTextAreaElement
    el.selectionStart = el.selectionEnd = 2
    await ta.trigger('keydown', { key: 'Tab' })
    expect(rawOf(draft)).toBe('{\n  }')
    expect(el.selectionStart).toBe(4)
  })

  it('Tab 缩进：raw 文本 textarea 同样缩进并回写草稿', async () => {
    const d = makeDraft()
    d.request.body = { mode: 'text', raw: 'hi' }
    const draft = reactive(d)
    const wrapper = mountPanel(draft)
    const ta = wrapper.find('textarea.body-input')
    expect(ta.exists()).toBe(true)
    const el = ta.element as HTMLTextAreaElement
    el.selectionStart = el.selectionEnd = 2
    await ta.trigger('keydown', { key: 'Tab' })
    // raw 文本走防抖缓冲：blur 落库
    await ta.trigger('blur')
    expect(rawOf(draft)).toBe('hi  ')
  })

  it('JSON 编辑器填满分配高度（不贴内容生长）：hl-wrap 不带 auto-height', async () => {
    const draft = jsonDraft('{\n  "a": 1\n}')
    const wrapper = mountPanel(draft)
    const wrap = wrapper.find('.hl-wrap')
    expect(wrap.exists()).toBe(true)
    // 回归：编辑器若回到 auto-height 贴内容，请求卡 grow/下拉出的多余高度
    // 会在卡片中部留出大片空白（用户实测）
    expect(wrap.classes()).not.toContain('auto-height')
    expect(wrapper.findComponent({ name: 'JsonEditor' }).props('autoHeight')).toBe(false)
  })

  it('GraphQL：variables 次级编辑器保留贴内容（auto-height）', async () => {
    const d = makeDraft()
    d.request.body = {
      mode: 'graphql',
      spec: { query: 'query { hero }', variables: '{"id":"42"}', operation_name: '' },
    }
    const wrapper = mountPanel(reactive(d))
    const jsonEditors = wrapper.findAllComponents({ name: 'JsonEditor' })
    expect(jsonEditors).toHaveLength(1)
    // 主 query textarea（flex:1）吸收剩余高度，variables 只贴内容生长
    expect(jsonEditors[0]!.props('autoHeight')).toBe(true)
  })
})

/**
 * 回归：发送内容恒等于「当前选中类型下方的参数」。
 *
 * 页签是数据的独立分区：raw 文本、form-data 字段、urlencoded 字段、GraphQL query
 * 各自保存，互不覆盖；切换类型只换 request.body 的引用，不靠推导回填（推导会给出
 * 空 json 文本 / 空字段表，正是「输入框里有参数、带出去的却不对」的来源）。
 * 草稿按接口隔离且跨组件卸载存活（BodyPanel 切到 Params/Headers 页签即重建）。
 */
describe('BodyPanel：发送内容 = 当前类型自己的参数', () => {
  function bodyTabOf(wrapper: ReturnType<typeof mountPanel>, label: string) {
    return wrapper.findAll('.seg-item').find((b) => b.text() === label)!
  }
  function ctOf(d: Endpoint) {
    return d.request.headers.filter((h) => h.key.toLowerCase() === 'content-type').map((h) => h.value)
  }

  it('raw(JSON) 参数 → form-data → 切回 raw：文本与 Content-Type 都在', async () => {
    const draft = jsonDraft('{"user":"alice"}')
    const wrapper = mountPanel(draft)

    await bodyTabOf(wrapper, 'form-data').trigger('click')
    expect((draft.request.body as { mode: string }).mode).toBe('multipart')
    await bodyTabOf(wrapper, 'raw').trigger('click')

    expect(rawOf(draft)).toBe('{"user":"alice"}')
    expect(ctOf(draft)).toEqual(['application/json'])
  })

  it('form-data 字段 → raw → 切回 form-data：字段仍在（两种类型互不覆盖）', async () => {
    const draft = reactive(makeDraft())
    draft.request.body = {
      mode: 'multipart',
      fields: [{ key: 'avatar', value_type: 'text', value: 'a.png', enabled: true }],
    }
    const wrapper = mountPanel(draft)

    await bodyTabOf(wrapper, 'raw').trigger('click')
    expect(draft.request.body).toEqual({ mode: 'text', raw: '' })
    await wrapper.find('textarea.body-input').setValue('id=7')
    await bodyTabOf(wrapper, 'form-data').trigger('click')

    expect(draft.request.body).toMatchObject({
      mode: 'multipart',
      fields: [{ key: 'avatar', value: 'a.png' }],
    })
    expect((wrapper.find('input.mp-key').element as HTMLInputElement).value).toBe('avatar')

    // 再切回 raw：刚填的文本也还在（互不覆盖）
    await bodyTabOf(wrapper, 'raw').trigger('click')
    expect(rawOf(draft)).toBe('id=7')
  })

  it('Content-Type 跟随当前类型，切回后恢复', async () => {
    const draft = jsonDraft('{"a":1}')
    const wrapper = mountPanel(draft)
    await bodyTabOf(wrapper, 'form-data').trigger('click')
    // form-data 的 boundary 由执行器生成，显式头必须移除
    expect(ctOf(draft)).toEqual([])
    await bodyTabOf(wrapper, 'x-www-form-urlencoded').trigger('click')
    expect(ctOf(draft)).toEqual(['application/x-www-form-urlencoded'])
    await bodyTabOf(wrapper, 'raw').trigger('click')
    expect(ctOf(draft)).toEqual(['application/json'])
  })

  it('cURL 导入的 urlencoded 文本（raw + Text 子类型）：切走再切回不被换成 text/plain', async () => {
    const draft = reactive(makeDraft())
    // 复现用户导入：-d 'a=1&b=2' + Content-Type: application/x-www-form-urlencoded
    draft.request.body = { mode: 'text', raw: 'a=1&b=2' }
    draft.request.headers.push({
      key: 'Content-Type',
      value: 'application/x-www-form-urlencoded',
      enabled: true,
      description: '', field_type: 'string', required: true, example: '' })
    const wrapper = mountPanel(draft)

    await bodyTabOf(wrapper, 'form-data').trigger('click')
    await bodyTabOf(wrapper, 'raw').trigger('click')

    expect(rawOf(draft)).toBe('a=1&b=2')
    expect((draft.request.body as { mode: string }).mode).toBe('text')
    expect(ctOf(draft)).toEqual(['application/x-www-form-urlencoded'])
  })

  it('form-data ↔ x-www-form-urlencoded 仍实时互转字段', async () => {
    const draft = reactive(makeDraft())
    draft.request.body = {
      mode: 'multipart',
      fields: [{ key: 'k', value_type: 'text', value: 'v', enabled: true }],
    }
    const wrapper = mountPanel(draft)
    await bodyTabOf(wrapper, 'x-www-form-urlencoded').trigger('click')
    expect(draft.request.body).toMatchObject({ mode: 'urlencoded', fields: [{ key: 'k', value: 'v' }] })
    await bodyTabOf(wrapper, 'form-data').trigger('click')
    expect(draft.request.body).toMatchObject({ mode: 'multipart', fields: [{ key: 'k', value: 'v' }] })
  })

  it('键入未落盘（150ms 内）就切类型：内容仍保留', async () => {
    const draft = reactive(makeDraft())
    draft.request.body = { mode: 'text', raw: '' }
    const wrapper = mountPanel(draft)
    await wrapper.find('textarea.body-input').setValue('id=7')
    await bodyTabOf(wrapper, 'form-data').trigger('click')
    await bodyTabOf(wrapper, 'raw').trigger('click')
    expect(rawOf(draft)).toBe('id=7')
  })
})
