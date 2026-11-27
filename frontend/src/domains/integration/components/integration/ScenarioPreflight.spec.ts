/**
 * ScenarioPreflight：运行前校验面板。
 * - 默认折叠，只显示阻断 / 警告计数；
 * - 展开后列出问题、定位、修复建议、忽略非阻断警告；
 * - 有阻断时保留受控强制执行入口；无问题显示校验通过。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'
import ScenarioPreflight from '@/domains/integration/components/integration/ScenarioPreflight.vue'
import type { ValidateIssue } from '@/utils/scenarioValidate'

const issues: ValidateIssue[] = [
  {
    severity: 'error',
    stepIndex: 0,
    field: 'request',
    message: 'integration.validateEndpointMissing',
    subject: 'step-0',
  },
  {
    severity: 'warning',
    stepIndex: 1,
    field: 'vars',
    message: 'integration.validateVarUnused',
    detail: { name: 'orderId' },
    subject: 'orderId',
  },
]

function mountPanel(overrides: Record<string, unknown> = {}) {
  return mount(ScenarioPreflight, {
    props: {
      issues,
      errorCount: 1,
      warnCount: 1,
      ignored: new Set<string>(),
      open: false,
      ...overrides,
    },
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('ScenarioPreflight', () => {
  it('默认折叠：显示阻断 / 警告计数，不展开明细', () => {
    const w = mountPanel()
    expect(w.find('.wf-validate').classes()).toContain('collapsed')
    expect(w.find('.wf-validate-tag.error').text()).toContain('1')
    expect(w.find('.wf-validate-tag.warn').exists()).toBe(true)
    expect(w.find('.wf-validate-list').exists()).toBe(false)
  })

  it('点击展开明细；条目点击 emit locate；含修复建议', async () => {
    const w = mountPanel({ open: true })
    const items = w.findAll('.wf-validate-item')
    expect(items).toHaveLength(2)
    expect(w.text()).toContain('建议')

    await items[0]!.trigger('click')
    expect(w.emitted('locate')?.at(-1)?.[0]).toMatchObject({ severity: 'error', field: 'request' })
  })

  it('忽略非阻断警告：emit 稳定键；忽略后不计入警告计数并显示已忽略数', () => {
    const w = mountPanel({ open: true })
    const warning = w.findAll('.wf-validate-item')[1]!
    const ignoreBtn = warning.findAll('button').at(-1)!
    ignoreBtn.trigger('click')
    const key = w.emitted('ignore')?.at(-1)?.[0] as string
    expect(key).toContain('warning')
    expect(key).toContain('orderId')

    const w2 = mountPanel({ open: true, ignored: new Set([key]) })
    expect(w2.find('.wf-validate-tag.warn').exists()).toBe(false)
    expect(w2.text()).toContain('已忽略 1 条警告')
  })

  it('阻断问题保留受控强制执行入口', async () => {
    const w = mountPanel({ open: true })
    const force = w.findAll('button').find((b) => b.text().includes('仍然运行'))!
    await force.trigger('click')
    expect(w.emitted('force-run')).toHaveLength(1)
  })

  it('无问题时显示校验通过', () => {
    const w = mountPanel({ issues: [], errorCount: 0, warnCount: 0, open: true })
    expect(w.find('.wf-validate-tag.ok').exists()).toBe(true)
    expect(w.text()).toContain('校验通过')
  })
})