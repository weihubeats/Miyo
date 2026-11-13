/**
 * JsonStructEditor 组件单测：解析渲染 / 字段编辑回传 / 类型切换 / 增删 / 空态。
 * locale 需真实 Pinia（组件 setup 调 useLocaleStore）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JsonStructEditor from './JsonStructEditor.vue'
import { useLocaleStore } from '../../stores/locale'
import type { FieldDoc } from '../../types/miyoApi'

function mountEditor(props: { raw: string; docs?: Record<string, FieldDoc> }) {
  return mount(JsonStructEditor, { props })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('JsonStructEditor', () => {
  it('解析对象根并渲染字段行', () => {
    const w = mountEditor({ raw: '{"amount":100,"name":"x"}' })
    const keys = w.findAll('.jse-key').map((i) => (i.element as HTMLInputElement).value)
    expect(keys).toEqual(['amount', 'name'])
    expect(w.findAll('.jse-value')).toHaveLength(2)
  })

  it('注入 docs：说明 / 必填回填对应行', () => {
    const w = mountEditor({
      raw: '{"amount":1}',
      docs: { '/amount': { description: '金额', required: false } },
    })
    expect((w.find('.jse-desc').element as HTMLInputElement).value).toBe('金额')
    expect((w.find('.jse-req input').element as HTMLInputElement).checked).toBe(false)
  })

  it('改说明同时发出 update:raw 与 update:docs', async () => {
    const w = mountEditor({ raw: '{"a":1}' })
    await w.find('.jse-desc').setValue('业务字段')
    const raw = w.emitted('update:raw')!.at(-1)![0] as string
    const docs = w.emitted('update:docs')!.at(-1)![0] as Record<string, unknown>
    expect(JSON.parse(raw)).toEqual({ a: 1 })
    expect(docs).toEqual({ '/a': { description: '业务字段', required: true } })
  })

  it('改名后序列化键更新（指针重绑）', async () => {
    const w = mountEditor({ raw: '{"old":1}' })
    await w.find('.jse-key').setValue('new')
    const raw = w.emitted('update:raw')!.at(-1)![0] as string
    expect(JSON.parse(raw)).toEqual({ new: 1 })
    const docs = w.emitted('update:docs')!.at(-1)![0] as Record<string, unknown>
    // 无说明无非必填 → 不落 sidecar
    expect(docs).toEqual({})
  })

  it('类型切 string → value 清空并序列化为字符串', async () => {
    const w = mountEditor({ raw: '{"n":42}' })
    await w.find('.jse-type').setValue('string')
    const raw = w.emitted('update:raw')!.at(-1)![0] as string
    expect(JSON.parse(raw)).toEqual({ n: '' })
  })

  it('添加字段追加 field；删除字段移除', async () => {
    const w = mountEditor({ raw: '{"a":1}' })
    await w.find('.jse-add').trigger('click')
    let raw = w.emitted('update:raw')!.at(-1)![0] as string
    expect(JSON.parse(raw)).toEqual({ a: 1, field: '' })
    expect(w.findAll('.jse-key')).toHaveLength(2)

    await w.findAll('.jse-row, .jse-line').at(-1)!.find('.ib.tone-danger').trigger('click')
    raw = w.emitted('update:raw')!.at(-1)![0] as string
    expect(JSON.parse(raw)).toEqual({ a: 1 })
  })

  it('空对象根显示空态 + 添加按钮', () => {
    const w = mountEditor({ raw: '{}' })
    expect(w.findAll('.jse-key')).toHaveLength(0)
    expect(w.find('.es-title').exists()).toBe(true)
    expect(w.find('.jse-add').exists()).toBe(true)
  })

  it('object 子字段展开缩进（depth > 0）', () => {
    const w = mountEditor({ raw: '{"user":{"name":"x"}}' })
    const lines = w.findAll('.jse-line')
    expect(lines).toHaveLength(2)
    const nestedPad = lines[1].attributes('style') ?? ''
    expect(nestedPad).toMatch(/padding-left:\s*26px/)
  })

  it('改名 + 父级回写 raw 不丢焦点（稳定 nid key + 回声守卫）', async () => {
    const w = mount(JsonStructEditor, {
      props: { raw: '{"old":1}' },
      attachTo: document.body,
    })
    try {
      const input = w.find('.jse-key')
      ;(input.element as HTMLInputElement).focus()
      await input.setValue('o')
      const echoed = w.emitted('update:raw')!.at(-1)![0] as string
      await w.setProps({ raw: echoed })
      await nextTick()
      expect(document.activeElement).toBe(input.element)
      expect((input.element as HTMLInputElement).value).toBe('o')
    } finally {
      w.unmount()
    }
  })
})
