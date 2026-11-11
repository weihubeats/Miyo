/**
 * 运行结果视图回归：运行结束自动切「执行结果」不得崩溃。
 *
 * 背景（2026-11 实测）：从编辑器点「运行」，跑到结束后 `viewMode` 切到
 * `'result'` 挂载 RunResultView，弹出「异常：Cannot destructure property 'type'
 * of 'vnode' as it is null」。真实根因是 RunResultView 的 setup 期 TDZ——
 * `watch(() => props.result, …, { immediate: true })` 里写 `respView.value`，
 * 而 `respView` 声明在该 watch 之后；setup 同步段就抛 ReferenceError，
 * 组件树 patch 中断，后续渲染才连锁炸成 vnode null。
 *
 * 第二个坑：时间线模板读 `r.outcomes.length` / `r.method` 无兜底。真实后端
 * 必带这两个字段，但旧运行历史（落库 JSON）与历史回读可能缺，
 * 缺一个就把整个结果视图打空。
 *
 * 本 spec 覆盖两条：
 * 1. 点运行 → 跑完 → 自动进入结果视图，且无 unhandled rejection；
 * 2. 旧形态步骤结果（缺 outcomes / method）仍能渲染。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h } from 'vue'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/path', () => ({ join: vi.fn(async (...p: string[]) => p.join('/')) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('sortablejs', () => ({ default: { create: vi.fn(() => ({ destroy: vi.fn(), option: vi.fn() })) } }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }))

function emptySpec() {
  return {
    params: [],
    headers: [],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    timeout_ms: null,
    follow_redirects: true,
    tests: null,
  }
}

/** 三步链式 GET：首步提取 userId，后两步引用（覆盖跨步变量高亮路径）。 */
const scenario = {
  id: 'sc-1',
  project_id: 'p-1',
  name: '文章链路',
  description: '',
  category: '',
  sort_order: 0,
  script: null,
  steps: [
    {
      endpoint_id: null,
      enabled: true,
      name: '文章详情',
      extract: [{ name: 'userId', from: 'body', path: '$.userId' }],
      assertions: [],
      request: { method: 'GET', url: '{{base_url}}/posts/1', spec: emptySpec() },
    },
    {
      endpoint_id: null,
      enabled: true,
      name: '用户文章',
      extract: [],
      assertions: [],
      request: { method: 'GET', url: '{{base_url}}/users/{{userId}}/posts', spec: emptySpec() },
    },
    {
      endpoint_id: null,
      enabled: true,
      name: '用户详情',
      extract: [],
      assertions: [],
      request: { method: 'GET', url: '{{base_url}}/users/1', spec: emptySpec() },
    },
  ],
}

const stepResult = (i: number) => ({
  endpoint_id: `ep-${i}`,
  endpoint_name: scenario.steps[i]!.name,
  method: 'GET',
  path: scenario.steps[i]!.request.url,
  step_index: i,
  ok: true,
  status: 200,
  duration_ms: 12,
  outcomes: [],
  extracted: i === 0 ? { userId: '1' } : {},
  response: null,
})

/** 旧形态：缺 method / outcomes（历史回读的旧 JSON 就是这个形状）。 */
const legacyStepResult = {
  endpoint_id: 'ep-0',
  endpoint_name: '旧结果',
  path: '/legacy',
  ok: true,
  status: 200,
  duration_ms: 5,
  response: null,
}

const api = {
  listScenarios: vi.fn(async () => [scenario]),
  saveScenario: vi.fn(async () => scenario),
  runScenario: vi.fn(async () => ({
    results: [stepResult(0), stepResult(1), stepResult(2)],
    cancelled: false,
    stopped_on_failure: false,
    variables: {},
    vars_snapshots: [],
  })),
  listScenarioRuns: vi.fn(async () => []),
  getScenarioRun: vi.fn(async () => null),
  cancelScenarioRun: vi.fn(async () => {}),
  stopScenarioRun: vi.fn(async () => {}),
}
vi.mock('../../composables/useMiyoApi', () => ({ useMiyoApi: () => api }))
vi.mock('../../composables/useToast', () => ({
  useToast: () => ({ info: vi.fn(), warning: vi.fn(), error: vi.fn(), success: vi.fn() }),
}))

import WorkflowEditor from './WorkflowEditor.vue'
import RunResultView from './RunResultView.vue'
import { useLocaleStore } from '../../stores/locale'
import { useWorkspaceStore } from '../../stores/workspace'
import { useIntegrationStore } from '../../stores/integration'
import type { ScenarioRunResult } from '../../types/miyoApi'

async function mountEditor() {
  const workspace = useWorkspaceStore()
  workspace.project = {
    id: 'p-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '',
    updated_at: '',
  } as never
  useIntegrationStore().projectId = 'p-1'
  const Host = defineComponent({ setup: () => () => h(WorkflowEditor, { scenarioId: 'sc-1' }) })
  const wrapper = mount(Host, { attachTo: document.body })
  await flushPromises()
  await flushPromises()
  return wrapper
}

describe('运行结果视图：运行结束自动切换', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    vi.clearAllMocks()
  })

  it('点运行 → 跑完 → 自动进入执行结果视图（respView TDZ 回归）', async () => {
    const wrapper = await mountEditor()
    const runBtn = wrapper.findAll('.wf-topbar button').find((b) => b.text().includes('运行'))
    expect(runBtn).toBeTruthy()

    await runBtn!.trigger('click')
    await flushPromises()
    await flushPromises()
    await flushPromises()

    expect(api.runScenario).toHaveBeenCalled()
    // 结果视图挂上：说明 RunResultView setup 完整跑完（TDZ 会让它整棵树挂掉）
    expect(wrapper.find('.rv-timeline').exists()).toBe(true)
    expect(wrapper.findAll('.rv-step')).toHaveLength(3)
    wrapper.unmount()
  })

  it('旧形态结果（缺 outcomes / method）仍能渲染，不把视图打空', async () => {
    const result = {
      results: [legacyStepResult],
      cancelled: false,
      stopped_on_failure: false,
      variables: {},
    } as unknown as ScenarioRunResult
    const wrapper = mount(RunResultView, {
      props: {
        scenarioName: '旧结果',
        runId: 'r-1',
        running: false,
        startedAt: null,
        finishedAt: null,
        result,
        varChain: [],
      },
      attachTo: document.body,
    })
    await flushPromises()

    expect(wrapper.find('.rv-timeline').exists()).toBe(true)
    expect(wrapper.findAll('.rv-step')).toHaveLength(1)
    wrapper.unmount()
  })
})