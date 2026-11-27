/**
 * ScenarioWorkbenchHeader：全局导航栏。
 * - 返回按钮始终可见并 emit back；
 * - 场景名内联编辑 emit update:name；
 * - 中部三个视图页签 emit update:tab；
 * - 右侧保存状态四态 + 保存 / 运行 / 更多动作。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'
import ScenarioWorkbenchHeader from '@/domains/integration/components/integration/ScenarioWorkbenchHeader.vue'

const tabs = [
  { key: 'manual', label: '编辑' },
  { key: 'script', label: '脚本' },
  { key: 'result', label: '执行结果' },
]

function mountHeader(overrides: Record<string, unknown> = {}) {
  return mount(ScenarioWorkbenchHeader, {
    props: {
      name: '收单链路',
      backLabel: '返回测试场景',
      crumbs: [
        { label: '集成测试' },
        { label: '交易' },
        { label: '支付' },
      ],
      tab: 'manual',
      tabs,
      dirty: false,
      saving: false,
      saveError: false,
      running: false,
      ...overrides,
    },
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('ScenarioWorkbenchHeader', () => {
  it('返回按钮始终可见并触发 back；展示名称与目录', async () => {
    const w = mountHeader()
    expect(w.find('.wf-back').exists()).toBe(true)
    const back = w.find('.int-nav-back')
    await back.trigger('click')
    expect(w.emitted('back')).toHaveLength(1)
    expect(w.find('.wf-name').element).toHaveProperty('value', '收单链路')
    expect(w.text()).toContain('返回测试场景')
    expect(w.text()).toContain('集成测试')
    expect(w.text()).toContain('交易')
    expect(w.text()).toContain('支付')
  })

  it('名称编辑 emit update:name；页签切换 emit update:tab', async () => {
    const w = mountHeader()
    await w.find('.wf-name').setValue('新名字')
    expect(w.emitted('update:name')?.at(-1)).toEqual(['新名字'])

    const tabButtons = w.findAll('.wf-view-tabs button')
    expect(tabButtons).toHaveLength(3)
    await tabButtons[2]!.trigger('click')
    expect(w.emitted('update:tab')?.at(-1)).toEqual(['result'])
  })

  it('保存 / 运行 / 更多动作 emit 对应事件', async () => {
    const w = mountHeader()
    const buttons = w.findAll('.wf-topbar-right button')
    const save = buttons.find((b) => b.text().includes('保存'))!
    const run = buttons.find((b) => b.text().includes('运行'))!
    await save.trigger('click')
    expect(w.emitted('save')).toHaveLength(1)
    await run.trigger('click')
    expect(w.emitted('run')).toHaveLength(1)

    const more = w.find('.wf-topbar-right button[title="更多操作"]')
    await more.trigger('click')
    expect(w.emitted('more')).toHaveLength(1)
  })

  it('保存状态四态：已保存 / 未保存 / 保存中 / 保存失败', () => {
    expect(mountHeader().find('.wf-save-state').classes()).toContain('saved')
    expect(mountHeader({ dirty: true }).find('.wf-save-state').classes()).toContain('dirty')
    expect(mountHeader({ saving: true }).find('.wf-save-state').classes()).toContain('saving')
    const failed = mountHeader({ saveError: true })
    expect(failed.find('.wf-save-state').classes()).toContain('failed')
    expect(failed.find('.wf-save-state').text()).toContain('保存失败')
  })

  it('运行中：运行按钮切换为停止', () => {
    const w = mountHeader({ running: true })
    const run = w.findAll('.wf-topbar-right button').find((b) => b.text().includes('停止'))!
    expect(run).toBeTruthy()
    expect(run.classes()).toContain('miyo-btn-danger-solid')
  })
})