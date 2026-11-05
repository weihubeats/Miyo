/**
 * HistoryPanel 单测：关键字搜索 + 状态筛选 + 卡片结构与激活态。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import HistoryPanel from './HistoryPanel.vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import type { RequestHistory } from '../types/miyoApi'

function history(partial: Partial<RequestHistory> & { id: string }): RequestHistory {
  return {
    project_id: 'p-1',
    endpoint_id: null,
    method: 'GET',
    url: 'https://api.example.com/users',
    status: 200,
    duration_ms: 12,
    request_summary_json: '{}',
    response_summary_json: '{}',
    created_at: '2026-01-01T00:00:00.000Z',
    ...partial,
  }
}

function mountPanel() {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.histories = [
    history({ id: 'h-1', url: 'https://api.example.com/users', status: 200 }),
    history({ id: 'h-2', url: 'https://api.example.com/orders', status: 500, method: 'POST' }),
  ]
  return mount(HistoryPanel, { attachTo: document.body })
}

describe('HistoryPanel', () => {
  it('关键字过滤 URL/方法/状态码', async () => {
    const wrapper = mountPanel()
    expect(wrapper.findAll('.hp-row')).toHaveLength(2)
    await wrapper.find('.hp-search').setValue('orders')
    expect(wrapper.findAll('.hp-row')).toHaveLength(1)
    expect(wrapper.text()).toContain('/orders')
    await wrapper.find('.hp-search').setValue('500')
    expect(wrapper.findAll('.hp-row')).toHaveLength(1)
    await wrapper.find('.hp-search').setValue('no-such-thing')
    expect(wrapper.findAll('.hp-row')).toHaveLength(0)
    expect(wrapper.text()).toContain('无匹配记录')
    wrapper.unmount()
  })

  it('状态筛选在 全部/2xx/4xx5xx 间循环', async () => {
    const wrapper = mountPanel()
    const btn = wrapper.find('.hp-status-filter')
    expect(btn.text()).toBe('全部')
    await btn.trigger('click')
    expect(btn.text()).toBe('2xx')
    expect(wrapper.findAll('.hp-row')).toHaveLength(1)
    await btn.trigger('click')
    expect(btn.text()).toBe('4xx5xx')
    expect(wrapper.findAll('.hp-row')).toHaveLength(1)
    await btn.trigger('click')
    expect(btn.text()).toBe('全部')
    expect(wrapper.findAll('.hp-row')).toHaveLength(2)
    wrapper.unmount()
  })

  it('两行卡片：方法 Tag 走 methodTone、状态胶囊分级、耗时紧凑格式（无红左条）', async () => {
    const wrapper = mountPanel()
    const rows = wrapper.findAll('.hp-row')
    // 取消行级红竖线（激活/hover 承担交互反馈）
    expect(rows[0]!.classes()).not.toContain('err')
    expect(rows[1]!.classes()).not.toContain('err')

    expect(rows[0]!.find('.hp-line1').exists()).toBe(true)
    expect(rows[0]!.find('.hp-line2').exists()).toBe(true)

    // 方法微型 Tag：methodTone() 单源徽章类（GET fixture）
    expect(rows[0]!.find('.hp-method').classes().join(' ')).toContain('text-method-get')
    // 状态胶囊：2xx 翠绿 / 5xx 绯红
    expect(rows[0]!.find('.hp-status.ok').exists()).toBe(true)
    expect(rows[1]!.find('.hp-status.err').exists()).toBe(true)
    expect(rows[1]!.text()).toContain('500')

    // duration_ms=12 → 紧凑格式「12 ms」（非精度版「12.00 ms」）
    expect(rows[0]!.text()).toContain('12 ms')
    expect(rows[0]!.text()).not.toContain('12.00 ms')
    wrapper.unmount()
  })

  it('点击记录进入激活态，且仅当前点击行高亮', async () => {
    const wrapper = mountPanel()
    const store = useWorkspaceStore()
    const restoreSpy = vi
      .spyOn(store, 'restoreFromHistory')
      .mockImplementation(() => undefined)
    const rows = wrapper.findAll('.hp-row')
    await rows[0]!.trigger('click')
    expect(restoreSpy).toHaveBeenCalledTimes(1)
    expect(rows[0]!.classes()).toContain('active')
    expect(rows[1]!.classes()).not.toContain('active')
    await rows[1]!.trigger('click')
    expect(rows[0]!.classes()).not.toContain('active')
    expect(rows[1]!.classes()).toContain('active')
    restoreSpy.mockRestore()
    wrapper.unmount()
  })

  it('存量 {{变量}} 前缀短地址兼容：{{base_url}}/api → /api', async () => {
    const store = useWorkspaceStore()
    store.histories = [
      history({ id: 'h-var', url: '{{base_url}}/api/v1/orders', status: 200 }),
    ]
    const wrapper = mount(HistoryPanel, { attachTo: document.body })
    await nextTick()
    expect(wrapper.find('.hp-url').text()).toBe('/api/v1/orders')
    wrapper.unmount()
  })

  it('一键重放：hover 钮触发 restoreFromHistory(…, { send: true })，gRPC 记录不渲染', async () => {
    const store = useWorkspaceStore()
    store.histories = [
      history({ id: 'h-http', url: 'https://api.example.com/users', method: 'GET' }),
      history({ id: 'h-grpc', url: 'helloworld.Greeter/SayHello', method: 'GRPC' }),
    ]
    const wrapper = mount(HistoryPanel, { attachTo: document.body })
    await nextTick()
    const store2 = useWorkspaceStore()
    const restoreSpy = vi.spyOn(store2, 'restoreFromHistory').mockImplementation(() => undefined)

    // 重放钮 hover 显现、每条 HTTP 记录一个；GRPC 记录没有
    const replayBtns = wrapper.findAll('.hp-replay')
    expect(replayBtns).toHaveLength(1)
    await replayBtns[0]!.trigger('click')
    expect(restoreSpy).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'h-http' }),
      { send: true },
    )
    // 点击行点亮激活态
    expect(wrapper.find('.hp-row.active').exists()).toBe(true)

    restoreSpy.mockRestore()
    wrapper.unmount()
  })
})
