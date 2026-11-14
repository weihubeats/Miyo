/**
 * FrameLog 单测：关键字/方向过滤、行点击选中、空态与无匹配、行渲染形状。
 * jsdom 无布局：虚拟滚动 enabled 分支以 80 条阈值划分，聚焦过滤与交互。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import FrameLog, { type FrameItem } from '@/domains/endpoint/components/realtime/realtime/FrameLog.vue'
import { setActivePinia, createPinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'

function frame(seq: number, dir: FrameItem['dir'], kind: string, text: string): FrameItem {
  return { seq, t: '10:00:00', dir, kind, text, size: text.length }
}

const ITEMS: FrameItem[] = [
  frame(1, 'sys', 'sys', '已连接 ws://x'),
  frame(2, 'in', 'text', '{"ok":true,"orderNo":"A1"}'),
  frame(3, 'out', 'text', 'ping-1'),
  frame(4, 'in', 'binary', 'eyJhIjoxfQ=='),
]

function mountLog() {
  return mount(FrameLog, {
    props: { items: ITEMS, selectedSeq: null, emptyTitle: '连接后在此查看帧' },
    attachTo: document.body,
  })
}

describe('FrameLog', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('渲染全部行（低于阈值全量渲染）：方向/时间/类型/字符数', () => {
    const w = mountLog()
    const rows = w.findAll('.fl-row')
    expect(rows).toHaveLength(4)
    expect(rows[1]!.classes()).toContain('dir-in')
    expect(rows[2]!.classes()).toContain('dir-out')
    expect(rows[1]!.text()).toContain('10:00:00')
    expect(rows[1]!.text()).toContain('text')
    expect(rows[1]!.find('.fl-size').text()).toBe(String('{"ok":true,"orderNo":"A1"}'.length))
    w.unmount()
  })

  it('关键字过滤（内容命中）+ 计数显示 n/m，无命中显示无匹配', async () => {
    const w = mountLog()
    await w.find('.fl-search-input').setValue('orderNo')
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(1)
    expect(w.find('.fl-count').text()).toBe('1/4')

    await w.find('.fl-search-input').setValue('不存在的帧')
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(0)
    expect(w.find('.fl-nomatch').exists()).toBe(true)
    w.unmount()
  })

  it('方向 chips 过滤：收 / 发 / 系统', async () => {
    const w = mountLog()
    const chips = w.findAll('.fl-dir-chip')
    expect(chips).toHaveLength(4)
    await chips[1]!.trigger('click') // 收
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(2)
    await chips[2]!.trigger('click') // 发
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(1)
    await chips[3]!.trigger('click') // 系统
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(1)
    await chips[0]!.trigger('click') // 全部
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(4)
    w.unmount()
  })

  it('行点击 emit select，传入完整 FrameItem', async () => {
    const w = mountLog()
    await w.findAll('.fl-row')[2]!.trigger('click')
    expect(w.emitted('select')?.[0]).toEqual([ITEMS[2]])
    w.unmount()
  })

  it('空 items 显示空态（不渲染行与过滤计数异常）', () => {
    const w = mount(FrameLog, {
      props: { items: [], selectedSeq: null, emptyTitle: '连接后在此查看帧' },
      attachTo: document.body,
    })
    expect(w.find('.fl-row').exists()).toBe(false)
    expect(w.text()).toContain('连接后在此查看帧')
    expect(w.find('.fl-count').text()).toBe('0/0')
    w.unmount()
  })
})
