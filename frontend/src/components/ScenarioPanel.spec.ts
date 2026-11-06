/**
 * ScenarioPanel 单测：侧栏场景管理区（搜索 / 复制 / 最近执行状态 / 删除）。
 *
 * 组件 setup 已调 useLocaleStore，spec 统一 setActivePinia + setMode('zh')
 * （AGENTS.md 组件 spec 固定语言规范）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

const listScenarioRuns = vi.fn<(id: string, limit?: number) => Promise<ScenarioRunSummary[]>>(
  async () => [],
)
const saveScenario = vi.fn(async (s: Scenario) => s)
const deleteScenario = vi.fn(async (_id: string) => {})

vi.mock('../composables/useMiyoApi', async (importOriginal) => {
  const mod = await importOriginal<typeof import('../composables/useMiyoApi')>()
  return {
    ...mod,
    // 只覆盖运行历史 / 场景保存 / 删除；其余保持原实现。
    useMiyoApi: () => ({
      ...mod.useMiyoApi(),
      listScenarioRuns,
      saveScenario,
      deleteScenario,
    }),
  }
})

import ScenarioPanel from './ScenarioPanel.vue'
import Popconfirm from './ui/Popconfirm.vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import type { Scenario, ScenarioRunSummary } from '../types/miyoApi'

function makeScenario(id: string, name: string, stepCount = 1): Scenario {
  const now = '2026-01-01T00:00:00.000Z'
  return {
    id,
    project_id: 'p-1',
    name,
    description: '',
    sort_order: 0,
    steps: Array.from({ length: stepCount }, () => ({
      endpoint_id: 'ep-1',
      enabled: true,
      extract: [],
      assertions: [],
    })),
    created_at: now,
    updated_at: now,
  }
}

describe('ScenarioPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    listScenarioRuns.mockReset()
    listScenarioRuns.mockImplementation(async () => [])
    saveScenario.mockReset()
    saveScenario.mockImplementation(async (s: Scenario) => s)
    deleteScenario.mockReset()
    deleteScenario.mockImplementation(async () => {})
  })

  it('渲染场景列表：步骤数徽章 + 最近执行状态点（成功 / 失败 / 无运行）', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario('sc-a', '下单链路', 3), makeScenario('sc-b', '登录链路', 2)]
    listScenarioRuns.mockImplementation(async (id: string) => {
      if (id !== 'sc-a') return []
      return [
        {
          id: 'run-1',
          finished_at: '2026-01-02T03:04:05.000Z',
          duration_ms: 12,
          total_steps: 3,
          ok_count: 2,
          fail_count: 1,
          skipped_count: 0,
          cancelled: false,
          stopped_on_failure: true,
        },
      ]
    })

    const wrapper = mount(ScenarioPanel)
    await flushPromises()

    const rows = wrapper.findAll('.sp-row')
    expect(rows.length).toBe(2)
    expect(rows[0]!.text()).toContain('3 步')
    expect(rows[1]!.text()).toContain('2 步')
    // sc-a 最近一次运行存在失败 → 红点；sc-b 无运行 → 灰点。
    expect(rows[0]!.find('.sp-run-dot.fail').exists()).toBe(true)
    expect(rows[1]!.find('.sp-run-dot.ok').exists()).toBe(false)
    expect(rows[1]!.find('.sp-run-dot.fail').exists()).toBe(false)
  })

  it('搜索按名称即时过滤', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario('sc-a', '下单链路'), makeScenario('sc-b', '登录链路')]

    const wrapper = mount(ScenarioPanel)
    await nextTick()

    await wrapper.find('.sp-search-input').setValue('下单')
    await nextTick()
    const rows = wrapper.findAll('.sp-row')
    expect(rows.length).toBe(1)
    expect(rows[0]!.text()).toContain('下单链路')

    await wrapper.find('.sp-search-clear').trigger('click')
    await nextTick()
    expect(wrapper.findAll('.sp-row').length).toBe(2)
  })

  it('复制场景：深拷贝步骤落库为新场景并打开', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario('sc-a', '下单链路', 2)]

    const wrapper = mount(ScenarioPanel)
    await flushPromises()

    const dupBtn = wrapper.findAll('.sp-actions .sp-act')[0]!
    expect(dupBtn.attributes('aria-label')).toBe('复制场景')
    await dupBtn.trigger('click')
    await flushPromises()

    expect(saveScenario).toHaveBeenCalledTimes(1)
    const saved = saveScenario.mock.calls[0]![0] as Scenario
    expect(saved.id).not.toBe('sc-a')
    expect(saved.name).toBe('下单链路 副本')
    expect(saved.steps).toHaveLength(2)
    // 复制后自动打开新场景页签。
    expect(store.activeTabId).toBe(`scenario:${saved.id}`)
  })

  /**
   * 真实点击链路：点垃圾桶 → 气泡展开 → 点气泡里的删除。
   * 旧用例直接 $emit('confirm')，跳过了「气泡能不能弹出来」这一段，
   * 气泡根本没展开时测试照样绿（v0.0.54 用户实测点删除无反应即此情形）。
   */
  it('删除场景：点垃圾桶弹出确认气泡，点确认才真正删除', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario('sc-a', '下单链路')]

    const wrapper = mount(ScenarioPanel, { attachTo: document.body })
    await flushPromises()

    await wrapper.find('.sp-act[aria-label="删除场景"]').trigger('click')
    await flushPromises()
    const pop = document.body.querySelector('.pc-pop')
    expect(pop).not.toBeNull()
    // 点图标不应触发行的 openScenario，也不应直接删除
    expect(deleteScenario).not.toHaveBeenCalled()

    const confirmBtn = Array.from(pop!.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === '删除',
    )
    expect(confirmBtn).toBeTruthy()
    confirmBtn!.click()
    await flushPromises()

    expect(deleteScenario).toHaveBeenCalledWith('sc-a')
    expect(store.scenarios.length).toBe(0)
    wrapper.unmount()
  })

  it('删除场景：Popconfirm 确认后调用删除', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario('sc-a', '下单链路')]

    const wrapper = mount(ScenarioPanel)
    await flushPromises()

    await wrapper.findComponent(Popconfirm).vm.$emit('confirm')
    await flushPromises()

    expect(deleteScenario).toHaveBeenCalledWith('sc-a')
    expect(store.scenarios.length).toBe(0)
  })
})
