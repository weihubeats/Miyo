/**
 * ScenarioVarsPanel：变量依赖可视化（本次新增「提取后未使用」标记与过滤）。
 * - 总览行显示未使用数量；
 * - 未使用变量打「未使用」徽章；
 * - 「未使用」过滤器只保留提取后无引用的变量；
 * - 外部定位（highlight）自动展开并搜索。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'
import ScenarioVarsPanel from '@/domains/integration/components/scenario/scenario/ScenarioVarsPanel.vue'
import type { VarChainEntry } from '@/utils/scenarioVars'

const chain: VarChainEntry[] = [
  {
    name: 'orderId',
    sources: [{ stepIndex: 0, stepName: '创建订单', from: 'body', path: '$.id' }],
    usages: [{ stepIndex: 1, stepName: '支付预览' }],
    missing: false,
    external: false,
  },
  {
    name: 'unusedId',
    sources: [{ stepIndex: 0, stepName: '创建订单', from: 'body', path: '$.unused' }],
    usages: [],
    missing: false,
    external: false,
  },
  {
    name: 'ghost',
    sources: [],
    usages: [{ stepIndex: 1, stepName: '支付预览' }],
    missing: true,
    external: false,
  },
  {
    name: 'base_url',
    sources: [],
    usages: [{ stepIndex: 1, stepName: '支付预览' }],
    missing: false,
    external: true,
  },
]

function mountPanel(props: Record<string, unknown> = {}) {
  return mount(ScenarioVarsPanel, {
    props: { chain, runtimeVars: null, ...props },
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('ScenarioVarsPanel 变量依赖', () => {
  it('总览显示未使用数量', () => {
    const w = mountPanel()
    expect(w.text()).toContain('提取后未使用 1')
  })

  it('展开后未使用变量带「未使用」徽章，「未使用」过滤器只保留它', async () => {
    const w = mountPanel()
    await w.find('.se-vars-head').trigger('click')

    const unusedEntry = w.findAll('.se-var-entry').find((e) => e.text().includes('unusedId'))!
    expect(unusedEntry.text()).toContain('未使用')

    const unusedFilter = w.findAll('.se-vars-filter').find((b) => b.text().includes('未使用'))!
    await unusedFilter.trigger('click')
    const entries = w.findAll('.se-var-entry')
    expect(entries).toHaveLength(1)
    expect(entries[0]!.text()).toContain('unusedId')
    // 未定义 / 外部变量不混入「未使用」
    expect(entries[0]!.text()).not.toContain('ghost')
    expect(entries[0]!.text()).not.toContain('base_url')
  })

  it('highlight 定位：自动展开并过滤到目标变量', async () => {
    const w = mountPanel({ highlight: 'orderId' })
    expect(w.text()).toContain('创建订单')
    const entries = w.findAll('.se-var-entry')
    expect(entries).toHaveLength(1)
    expect(entries[0]!.text()).toContain('orderId')
  })

  it('来源 / 引用步骤点击 emit locate（下标定位）', async () => {
    const w = mountPanel()
    await w.find('.se-vars-head').trigger('click')
    const entry = w.findAll('.se-var-entry').find((e) => e.text().includes('orderId'))!
    await entry.find('.se-var-entry-head').trigger('click')
    const locs = entry.findAll('.se-var-loc')
    expect(locs.length).toBeGreaterThan(0)
    await locs[0]!.trigger('click')
    expect(w.emitted('locate')?.at(-1)).toEqual([0])
  })
})