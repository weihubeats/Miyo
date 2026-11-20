/**
 * FrameLog 单测：关键字/方向过滤、行点击选中、空态与无匹配、行渲染形状。
 * jsdom 无布局：虚拟滚动 enabled 分支以 80 条阈值划分，聚焦过滤与交互。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import FrameLog, { type FrameItem } from '@/domains/endpoint/components/realtime/realtime/FrameLog.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
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
    await w.find('.fl-search .si-input').setValue('orderNo')
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(1)
    expect(w.find('.fl-count').text()).toBe('1/4')

    await w.find('.fl-search .si-input').setValue('不存在的帧')
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

  it('帧类型下拉过滤：切到 binary 只显示 binary 帧', async () => {
    const w = mountLog()
    const kindSel = w.findComponent(CustomSelect)
    expect(kindSel.exists()).toBe(true)
    kindSel.vm.$emit('update:modelValue', 'binary')
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(1)
    expect(w.findAll('.fl-row')[0]!.classes()).toContain('dir-in')
    w.unmount()
  })

  it('暂停展示：冻结视图继续缓冲（+n 徽标），恢复后补齐；清空 emit clear', async () => {
    const w = mountLog()
    // 点暂停按钮（title 含「暂停消息展示」）
    const pauseBtn = w.findAll('button').find((b) => b.attributes('title')?.includes('暂停消息展示'))
    expect(pauseBtn).toBeTruthy()
    await pauseBtn!.trigger('click')
    // 新帧到达（宿主仍在推）：视图冻结在 4 行，缓冲 +1
    await w.setProps({ items: [...ITEMS, frame(5, 'in', 'text', 'late')] })
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(4)
    expect(w.find('.fl-pending').text()).toBe('+1')
    // 恢复：补齐 5 行，徽标消失
    const resumeBtn = w.findAll('button').find((b) => b.attributes('title')?.includes('恢复消息展示'))
    await resumeBtn!.trigger('click')
    await nextTick()
    expect(w.findAll('.fl-row')).toHaveLength(5)
    expect(w.find('.fl-pending').exists()).toBe(false)
    // 清空按钮 emit clear（宿主负责落空）
    const clearBtn = w.findAll('button').find((b) => b.attributes('title') === '清空日志')
    await clearBtn!.trigger('click')
    expect(w.emitted('clear')).toHaveLength(1)
    w.unmount()
  })

  it('导出菜单：选 JSON / TXT 后 emit export', async () => {
    const w = mountLog()
    const exportBtn = w.findAll('button').find((b) => b.attributes('title') === '导出日志')
    await exportBtn!.trigger('click')
    // Menu Teleport 到 body
    const menu = document.querySelector('.miyo-menu')!
    expect(menu).toBeTruthy()
    const items = [...menu.querySelectorAll('.miyo-menu-item')]
    expect(items.map((i) => i.textContent?.trim())).toEqual(['导出 JSON', '导出 TXT'])
    ;(items[0] as HTMLButtonElement).click()
    await nextTick()
    expect(w.emitted('export')?.[0]).toEqual(['json'])
    w.unmount()
  })
})
