/**
 * DesignPanel 单测：基本信息卡片 / 请求参数表 / Body 模式切换 /
 * Responses 增改删 / 保存事件与未保存提示。
 * store 与 miyoApi 以模块级 mock 替换；locale 需真实 Pinia（组件 setup 调 useLocaleStore）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import DesignPanel from '@/domains/endpoint/components/DesignPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint, ResponseExample } from '@/types/miyoApi'

const storeMock = vi.hoisted(() => {
  const examples = new Map<string, ResponseExample[]>()
  return {
    examples,
    // addResponse 校验接口已落库（response_examples 外键）；默认 ep-1 已保存。
    endpoints: [{ id: 'ep-1' }] as Array<{ id: string }>,
    setExamples: (entries: [string, ResponseExample[]][]) => {
      examples.clear()
      for (const [k, v] of entries) examples.set(k, v)
    },
    isDirty: vi.fn<() => boolean>(() => false),
    removeExample: vi.fn(async () => {}),
  }
})

const apiMock = vi.hoisted(() => ({
  saveExample: vi.fn(),
}))

vi.mock('@/stores/workspace.ts', () => ({
  useWorkspaceStore: () => storeMock,
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => apiMock,
}))

function draft(): Endpoint {
  const d = makeDraft({ id: 'ep-1', method: 'POST', path: '/api/v1/transfer' })
  d.request.params.push({ key: 'userId', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' })
  return reactive(d)
}

function mountPanel(d: Endpoint = draft()) {
  return mount(DesignPanel, { props: { draft: d } })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  vi.clearAllMocks()
  storeMock.isDirty.mockReturnValue(false)
  storeMock.setExamples([])
  storeMock.endpoints = [{ id: 'ep-1' }]
  apiMock.saveExample.mockImplementation(async (ex: ResponseExample) => ex)
})

describe('DesignPanel：基本信息与顶部操作栏', () => {
  it('渲染名称 / Method 徽标 / 路径输入', () => {
    const wrapper = mountPanel()
    expect((wrapper.find('input[placeholder="例如：获取用户列表"]').element as HTMLInputElement).value).toBe(
      '测试接口',
    )
    expect(wrapper.find('.method-pill').text()).toBe('POST')
    expect((wrapper.find('.path-input').element as HTMLInputElement).value).toBe('/api/v1/transfer')
  })

  it('未保存状态：isDirty=false 隐藏黄点，true 显示「未保存」', async () => {
    const wrapper = mountPanel()
    expect(wrapper.find('.dirty-hint').exists()).toBe(false)

    storeMock.isDirty.mockReturnValue(true)
    const dirty = mountPanel()
    expect(dirty.find('.dirty-hint').text()).toContain('未保存')
  })

  it('点击「保存设计」向父级发出 save 事件（名称确认逻辑由编辑器处理）', async () => {
    const wrapper = mountPanel()
    await wrapper.find('.save-btn').trigger('click')
    expect(wrapper.emitted('save')).toHaveLength(1)
  })

  it('修改名称写入草稿（v-model 直连 store 草稿对象）', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    await wrapper.find('input[placeholder="例如：获取用户列表"]').setValue('新名称')
    expect(d.name).toBe('新名称')
  })
})

describe('DesignPanel：请求参数定义', () => {
  it('Params 表渲染已有参数并支持行内修改写回草稿', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    const keyInput = wrapper.find('.pdt-table .pdt-input')
    expect((keyInput.element as HTMLInputElement).value).toBe('userId')

    await keyInput.setValue('uid')
    expect(d.request.params[0].key).toBe('uid')
  })

  it('「添加参数」追加空行到草稿数组', async () => {
    const d = draft()
    const wrapper = mountPanel(d)
    await wrapper.find('.pdt-add').trigger('click')
    expect(d.request.params).toHaveLength(2)
    expect(d.request.params[1]).toMatchObject({ key: '', field_type: 'string', required: true })
  })

  it('Headers Tab 同样挂接参数表', async () => {
    const d = draft()
    d.request.headers.push({ key: 'X-Trace', value: '', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    const wrapper = mountPanel(d)
    await wrapper.findAll('.tabs .tab').find((t) => t.text().startsWith('Headers'))!.trigger('click')

    // fixture 自带默认 Header，这里定位新追加的 X-Trace 行
    const traceInput = wrapper
      .findAll('.pdt-table .pdt-input')
      .find((i) => (i.element as HTMLInputElement).value === 'X-Trace')
    expect(traceInput).toBeDefined()

    await traceInput!.setValue('X-Trace-Id')
    expect(d.request.headers.at(-1)!.key).toBe('X-Trace-Id')
  })
})

describe('DesignPanel：Body 设计器', () => {
  async function openBodyTab(wrapper: ReturnType<typeof mountPanel>): Promise<void> {
    await wrapper.findAll('.tabs .tab').find((t) => t.text() === 'Body')!.trigger('click')
  }

  /** 切到文本模式（默认结构编辑器）。 */
  async function toTextMode(wrapper: ReturnType<typeof mountPanel>): Promise<void> {
    await wrapper.findAll('.seg button').find((b) => b.text() === '文本')!.trigger('click')
  }

  it('JSON 模式默认结构编辑器；切文本后编辑直接写草稿 raw 并校验非法 JSON', async () => {
    const d = draft()
    d.request.body = { mode: 'json', raw: '{"amount":100}' }
    const wrapper = mountPanel(d)
    await openBodyTab(wrapper)

    expect(wrapper.find('.jse').exists()).toBe(true)
    expect(wrapper.find('.body-json').exists()).toBe(false)

    await toTextMode(wrapper)
    const area = wrapper.find('.body-json')
    expect(wrapper.find('.json-state').text()).toContain('合法')

    await area.setValue('{ bad json')
    expect(d.request.body).toEqual({ mode: 'json', raw: '{ bad json' })
    expect(wrapper.find('.json-state').classes()).toContain('bad')
  })

  it('结构编辑器：改字段说明 / 示例值写回 raw 与 body_docs', async () => {
    const d = draft()
    d.request.body = { mode: 'json', raw: '{"amount":100}' }
    const wrapper = mountPanel(d)
    await openBodyTab(wrapper)

    const desc = wrapper.find('.jse-desc')
    expect(desc.exists()).toBe(true)
    await desc.setValue('转账金额')
    const body = d.request.body as { mode: string; raw?: string }
    expect(body.mode).toBe('json')
    expect(d.request.body_docs).toEqual({ '/amount': { description: '转账金额', required: true } })
    expect(JSON.parse(body.raw ?? '')).toEqual({ amount: 100 })
  })

  it('非法 JSON 切结构模式被拦截并提示', async () => {
    const d = draft()
    d.request.body = { mode: 'json', raw: '{ bad json' }
    const wrapper = mountPanel(d)
    await openBodyTab(wrapper)
    await toTextMode(wrapper)
    await wrapper.findAll('.seg button').find((b) => b.text() === '结构')!.trigger('click')
    // 回不到结构模式，仍为文本
    expect(wrapper.find('.jse').exists()).toBe(false)
    expect(wrapper.find('.body-json').exists()).toBe(true)
  })

  it('切换为 Form Data 时初始化 urlencoded 容器并搬移 JSON 字段', async () => {
    const d = draft()
    d.request.body = { mode: 'json', raw: '{"a":1,"b":"x"}' }
    const wrapper = mountPanel(d)
    await openBodyTab(wrapper)

    await wrapper.findAll('.seg button').find((b) => b.text() === 'Form Data')!.trigger('click')
    // 旧数据不丢：json 对象逐键转为字段行（组件内部已把容器换成 urlencoded）
    expect(d.request.body).toEqual({
      mode: 'urlencoded',
      fields: [
        { key: 'a', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
        { key: 'b', value: 'x', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      ],
    })
  })

  it('Form Data 切回 JSON：字段行回搬为 JSON 文本', async () => {
    const d = draft()
    d.request.body = {
      mode: 'urlencoded',
      fields: [{ key: 'amount', value: '100', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    }
    const wrapper = mountPanel(d)
    await openBodyTab(wrapper)

    await wrapper.findAll('.seg button').find((b) => b.text() === 'JSON')!.trigger('click')
    const body = d.request.body as { mode: string; raw?: string }
    expect(body.mode).toBe('json')
    expect(JSON.parse(body.raw ?? '')).toEqual({ amount: '100' })
  })
})

describe('DesignPanel：返回响应 (Responses)', () => {
  function example(overrides: Partial<ResponseExample> = {}): ResponseExample {
    return {
      id: 'ex-1',
      endpoint_id: 'ep-1',
      name: '成功响应',
      status: 200,
      headers: {},
      body: '{"code":0}',
      content_type: 'application/json',
      created_at: '2026-08-16T14:17:00Z',
      updated_at: '2026-08-16T14:17:00Z',
      ...overrides,
    }
  }

  it('默认自动展开首个成功响应，结构编辑器显示字段树', async () => {
    storeMock.setExamples([['ep-1', [example()]]])
    const wrapper = mountPanel()

    // 挂载即自动展开 200 响应，结构模式显示字段树
    expect(wrapper.find('.resp-editor').exists()).toBe(true)
    expect(wrapper.find('.resp-editor .jse').exists()).toBe(true)
    expect(wrapper.find('.resp-editor textarea').exists()).toBe(false)

    await wrapper.find('.resp-row').trigger('click')
    expect(wrapper.find('.resp-editor').exists()).toBe(false)

    await wrapper.find('.resp-row').trigger('click')
    expect(wrapper.find('.resp-editor').exists()).toBe(true)
  })

  it('切文本模式后：编辑后的 Body 经 saveExample 落库并刷新缓存', async () => {
    const ex = example()
    storeMock.setExamples([['ep-1', [ex]]])
    const wrapper = mountPanel()
    // 200 默认展开 → 切文本
    await wrapper
      .findAll('.resp-editor .seg button')
      .find((b) => b.text() === '文本')!
      .trigger('click')
    await wrapper.find('.resp-editor textarea').setValue('{"code":1}')
    await wrapper.find('.resp-actions .miyo-btn').trigger('click')

    expect(apiMock.saveExample).toHaveBeenCalledTimes(1)
    expect(apiMock.saveExample.mock.calls[0][0]).toMatchObject({ id: 'ex-1', body: '{"code":1}' })
    expect(storeMock.examples.get('ep-1')![0].body).toBe('{"code":1}')
  })

  it('结构模式编辑说明：saveExample 一并提交 docs', async () => {
    storeMock.setExamples([['ep-1', [example()]]])
    const wrapper = mountPanel()
    await wrapper.find('.resp-editor .jse-desc').setValue('业务响应码')
    await wrapper.find('.resp-actions .miyo-btn').trigger('click')

    expect(apiMock.saveExample.mock.calls[0][0]).toMatchObject({
      id: 'ex-1',
      docs: { '/code': { description: '业务响应码', required: true } },
    })
  })

  it('响应结构编辑未保存时实时预览同步（读编辑缓冲而非已存 body）', async () => {
    storeMock.setExamples([['ep-1', [example()]]])
    const wrapper = mountPanel()
    expect(wrapper.find('.preview-code').text()).toContain('code')

    // 改字段名（未点保存）→ 预览立即跟随
    await wrapper.find('.resp-editor .jse-key').setValue('retcode')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.preview-code').text()).toContain('retcode')
    expect(wrapper.find('.preview-code').text()).not.toContain('"code"')
  })

  it('响应文本模式编辑未保存时实时预览同步', async () => {
    storeMock.setExamples([['ep-1', [example({ body: '{"code":0}' })]]])
    const wrapper = mountPanel()
    await wrapper
      .findAll('.resp-editor .seg button')
      .find((b) => b.text() === '文本')!
      .trigger('click')
    await wrapper.find('.resp-editor textarea').setValue('{"code":42}')
    await wrapper.vm.$nextTick()
    // schema 视图叶子只显示类型，切 Mock 视图断言示例值来自编辑缓冲
    await wrapper.findAll('.preview-pills button').find((b) => b.text() === 'Mock 示例')!.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.preview-code').text()).toContain('42')
  })

  it('快速添加：标题行预设按键创建对应响应示例并自动展开', async () => {
    const wrapper = mountPanel()
    await wrapper.findAll('.resp-preset').find((b) => b.text().includes('400'))!.trigger('click')

    expect(apiMock.saveExample).toHaveBeenCalledWith(
      expect.objectContaining({ status: 400, endpoint_id: 'ep-1' }),
    )
  })

  it('删除响应走 store.removeExample', async () => {
    storeMock.setExamples([['ep-1', [example()]]])
    const wrapper = mountPanel()
    // 默认已展开，删除按钮在编辑器操作区

    // Popconfirm 确认后触发 confirm 事件
    await wrapper.findComponent({ name: 'Popconfirm' }).vm.$emit('confirm')
    await flushPromises()
    expect(storeMock.removeExample).toHaveBeenCalledWith('ep-1', 'ex-1')
  })

  it('重复状态码：已有 200 时预设禁用且直调也不再创建', async () => {
    storeMock.setExamples([['ep-1', [example({ id: 'ex-200', status: 200, name: '200 响应' })]]])
    const wrapper = mountPanel()

    const preset200 = wrapper.findAll('.resp-preset').find((b) => b.text().includes('200'))!
    expect((preset200.element as HTMLButtonElement).disabled).toBe(true)
    await preset200.trigger('click')
    expect(apiMock.saveExample).not.toHaveBeenCalled()
  })

  it('接口未落库：添加响应被拦截，不调 saveExample（外键保护）', async () => {
    storeMock.endpoints = []
    const wrapper = mountPanel()
    await wrapper.findAll('.resp-preset').find((b) => b.text().includes('400'))!.trigger('click')
    await flushPromises()
    expect(apiMock.saveExample).not.toHaveBeenCalled()
  })

  it('预设按键不消费自定义名称输入框里已输入的文字', async () => {
    const wrapper = mountPanel()
    await wrapper.find('.resp-name-input').setValue('我的错误')
    await wrapper.findAll('.resp-preset').find((b) => b.text().includes('400'))!.trigger('click')
    await flushPromises()
    expect(apiMock.saveExample).toHaveBeenCalledTimes(1)
    // 预设走默认名（i18n「{v} 响应」），输入框文字保留给下一次自定义添加
    expect(apiMock.saveExample.mock.calls[0][0].name).toBe('400 响应')
    expect((wrapper.find('.resp-name-input').element as HTMLInputElement).value).toBe('我的错误')
  })
})
