/**
 * ScenarioStepCard：步骤卡片信息与状态。
 * - 序号 / 方法 / 名称 / 路径 / 状态 / 断言数 / 提取数 / 耗时；
 * - 失败态只做左缘强调；折叠后隐藏路径与徽章；
 * - 勾选、启用、复制、菜单、删除、折叠、查看结果事件。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'
import ScenarioStepCard from '@/domains/integration/components/integration/ScenarioStepCard.vue'

function card(overrides: Record<string, unknown> = {}) {
  return mount(ScenarioStepCard, {
    props: {
      index: 1,
      method: 'POST',
      title: '创建订单',
      path: '{{base_url}}/orders',
      tone: null,
      toneText: '',
      duration: null,
      httpStatus: null,
      write: false,
      enabled: true,
      selected: false,
      checked: false,
      hasError: false,
      issueCount: 0,
      collapsed: false,
      elapsed: '',
      loopLabel: '',
      conditionOn: false,
      parallel: false,
      waitMs: null,
      extractCount: 0,
      assertCount: 0,
      canViewResult: false,
      ...overrides,
    },
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('ScenarioStepCard', () => {
  it('展示序号、方法、名称、路径、断言与提取数量', () => {
    const w = card({ extractCount: 2, assertCount: 3 })
    const text = w.text()
    expect(text).toContain('2')
    expect(text).toContain('POST')
    expect(text).toContain('创建订单')
    expect(text).toContain('{{base_url}}/orders')
    expect(text).toContain('提取 2')
    expect(text).toContain('断言 3')
    expect(text).not.toContain('写操作')
  })

  it('执行状态、HTTP 码与耗时可见；失败态加 failed 类', () => {
    const w = card({ tone: 'fail', toneText: '失败', httpStatus: 500, duration: 1500 })
    expect(w.find('.wf-step-tone').text()).toBe('失败')
    expect(w.find('.wf-step-http').text()).toBe('500')
    expect(w.find('.wf-step-duration').exists()).toBe(true)
    expect(w.classes()).toContain('failed')
  })

  it('折叠后隐藏路径与徽章，仍保留名称与状态', async () => {
    const w = card({ collapsed: true, tone: 'ok', toneText: '成功', assertCount: 1 })
    expect(w.text()).not.toContain('{{base_url}}/orders')
    expect(w.find('.wf-step-name').text()).toBe('创建订单')
    expect(w.find('.wf-step-tone').text()).toBe('成功')

    await w.find('.wf-step-actions button').trigger('click')
    expect(w.emitted('toggle-collapse')).toHaveLength(1)
  })

  it('勾选 / 启用 / 复制 / 菜单 / 查看结果事件转发', async () => {
    const w = card({ canViewResult: true })
    const checkbox = w.find('.wf-step-check')
    await checkbox.setValue(true)
    expect(w.emitted('toggle-check')?.at(-1)).toEqual([true])

    await w.trigger('click')
    expect(w.emitted('select')).toHaveLength(1)

    const acts = w.findAll('.wf-step-actions button')
    // 折叠 / 查看结果 / 启停 / 复制 / 更多 / 删除
    await acts[1]!.trigger('click')
    expect(w.emitted('view-result')).toHaveLength(1)
    await acts[2]!.trigger('click')
    expect(w.emitted('toggle-enabled')).toHaveLength(1)
    await acts[3]!.trigger('click')
    expect(w.emitted('duplicate')).toHaveLength(1)
    await acts[4]!.trigger('click')
    expect(w.emitted('menu')).toHaveLength(1)
  })

  it('禁用步骤加 disabled 类；校验问题角标区分阻断', () => {
    const w = card({ enabled: false, hasError: true, issueCount: 2 })
    expect(w.classes()).toContain('disabled')
    const issue = w.find('.wf-step-issue')
    expect(issue.text()).toBe('2')
    expect(issue.classes()).toContain('err')
  })
})