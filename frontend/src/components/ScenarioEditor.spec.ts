/**
 * ScenarioEditor 单测：挂载即加载场景并正常渲染。
 *
 * 回归护栏：immediate watch 在 setup 期间同步执行，若引用了声明在其后的
 * 状态（曾踩过 runResult 的暂时性死区），WebKit 下场景页签一打开就崩
 *（"Cannot access uninitialized variable"）。本测试保证挂载路径不被改坏。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

import type {
  Endpoint,
  Scenario,
  ScenarioRunDetail,
  ScenarioRunSummary,
} from '../types/miyoApi'
const runScenario = vi.fn()
const listScenarioRuns = vi.fn<(scenarioId: string, limit?: number) => Promise<ScenarioRunSummary[]>>(
  async () => [],
)
const getScenarioRun = vi.fn<(id: string) => Promise<ScenarioRunDetail | null>>(async () => null)

vi.mock('../composables/useMiyoApi', async (importOriginal) => {
  const mod = await importOriginal<typeof import('../composables/useMiyoApi')>()
  return {
    ...mod,
    // 只覆盖运行与历史查询；其余函数（store 内部用的列表 / 保存等）保持原实现。
    useMiyoApi: () => ({ ...mod.useMiyoApi(), runScenario, listScenarioRuns, getScenarioRun }),
  }
})

import ScenarioEditor from './ScenarioEditor.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import SegmentedControl from './ui/SegmentedControl.vue'
import tooltipOverflow from '../directives/tooltipOverflow'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'

// jsdom 未实现的浏览器 API：定位滚动与 reduced-motion 查询。
Element.prototype.scrollIntoView = vi.fn()
window.matchMedia =
  window.matchMedia ??
  ((query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }) as MediaQueryList)

/** 统一挂载：注册 JsonTree 依赖的全局 tooltip-overflow 指令（应用内由 main.ts 注册）。 */
function mountEditor(props: { scenarioId: string }) {
  return mount(ScenarioEditor, {
    props,
    global: { directives: { 'tooltip-overflow': tooltipOverflow } },
  })
}

function makeEndpoint(): Endpoint {
  const now = '2026-01-01T00:00:00.000Z'
  return {
    id: 'ep-1',
    project_id: 'p-1',
    folder_id: null,
    name: '用户文章',
    method: 'GET',
    path: '/users/{{userId}}/posts',
    description: '',
    status: 'developing',
    sort_order: 0,
    request: {
      params: [],
      headers: [],
      path_variables: [],
      auth: { type: 'none' },
      body: { mode: 'none' },
      active_tab: null,
      timeout_ms: null,
      follow_redirects: true,
      tests: null,
      disable_cookies: false,
      body_docs: {},
    },
    created_at: now,
    updated_at: now,
  }
}

function makeScenario(): Scenario {
  return {
    id: 'sc-1',
    project_id: 'p-1',
    name: '登录后查订单',
    description: '拿 token 查列表',
    sort_order: 0,
    steps: [
      {
        endpoint_id: 'ep-1',
        enabled: true,
        extract: [{ name: 'token', from: 'body', path: '$.token' }],
        assertions: [],
      },
    ],
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

describe('ScenarioEditor', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('挂载即加载场景（回归：setup 期不得踩声明在后的状态）', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario()]

    const wrapper = mountEditor({ scenarioId: 'sc-1' })
    await nextTick()

    const nameInput = wrapper.find('.se-name').element as HTMLInputElement
    expect(nameInput.value).toBe('登录后查订单')
    const descInput = wrapper.find('.se-desc').element as HTMLInputElement
    expect(descInput.value).toBe('拿 token 查列表')
    // 链路变量预览汇总启用步骤的提取目标。
    expect(wrapper.text()).toContain('{{token}}')
    // 无保存改动时保存按钮禁用。
    const saveBtn = wrapper.findAll('button').find((b) => b.text().includes('保存'))
    expect(saveBtn?.attributes('disabled')).toBeDefined()
  })

  it('脚本视图渲染生成脚本（CodeMirror 高亮）且可进入编辑', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeScenario()]
    store.endpoints = [makeEndpoint()]

    const wrapper = mountEditor({ scenarioId: 'sc-1' })
    await nextTick()

    // 切到脚本视图：应渲染生成脚本（含 BASE 常量与链路变量）
    const segmented = wrapper.findComponent(SegmentedControl)
    segmented.vm.$emit('update:modelValue', 'script')
    await nextTick()

    const cm = wrapper.find('.se-script .cm-content')
    expect(cm.exists()).toBe(true)
    expect(cm.text()).toContain('const BASE = process.env.BASE_URL')
    expect(cm.text()).toContain('http://localhost/users/${userId}/posts')

    // 编辑脚本 → 进入自定义态（保存按钮因脏检查启用）
    const editor = wrapper.findComponent(JsonCodeMirror)
    expect(editor.exists()).toBe(true)
    editor.vm.$emit('update:modelValue', 'const BASE = "https://x";')
    await nextTick()
    expect(wrapper.text()).toContain('脚本已手动编辑')
  })

  it('场景不存在时渲染占位而非崩溃', async () => {
    const store = useWorkspaceStore()
    store.scenarios = []

    const wrapper = mountEditor({ scenarioId: 'gone' })
    await nextTick()

    expect(wrapper.find('.se-missing').exists()).toBe(true)
  })
})

describe('ScenarioEditor：编排体验升级', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    runScenario.mockReset()
    // mockReset 会清掉工厂默认实现，逐个恢复（避免上一用例的夹具泄漏）。
    listScenarioRuns.mockReset()
    listScenarioRuns.mockImplementation(async () => [])
    getScenarioRun.mockReset()
    getScenarioRun.mockImplementation(async () => null)
  })

  /** 两步场景：第二步禁用（覆盖折叠态渲染 / 禁用态 / 变量链路警示）。 */
  function makeTwoStepScenario(): Scenario {
    const base = makeScenario()
    return {
      ...base,
      id: 'sc-cards',
      steps: [
        {
          endpoint_id: 'ep-1',
          enabled: true,
          extract: [{ name: 'token', from: 'body', path: '$.token' }],
          assertions: [],
        },
        { endpoint_id: 'ep-1', enabled: false, extract: [], assertions: [] },
      ],
    }
  }

  it('步骤卡片折叠态：方法徽章 / URL / 出参 chips / 禁用态', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeTwoStepScenario()]
    store.endpoints = [makeEndpoint()]

    const wrapper = mountEditor({ scenarioId: 'sc-cards' })
    await nextTick()

    expect(wrapper.findAll('.se-card').length).toBe(2)
    expect(wrapper.find('.se-method').text()).toBe('GET')
    expect(wrapper.find('.se-step-url').text()).toContain('/users/{{userId}}/posts')
    // 出参摘要：提取目标 token。
    expect(wrapper.find('.se-var-chip.out').text()).toBe('{{token}}')
    // 入参 {{userId}} 无定义来源 → 警示态。
    expect(wrapper.find('.se-var-chip.mini').classes()).toContain('warn')
    // 禁用步骤卡有 off 标记。
    expect(wrapper.findAll('.se-step.off').length).toBe(1)
  })

  it('变量链路面板：未定义变量警示，已定义变量显示来源与引用', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [makeTwoStepScenario()]
    store.endpoints = [makeEndpoint()]

    const wrapper = mountEditor({ scenarioId: 'sc-cards' })
    await nextTick()

    const panel = wrapper.find('.se-vars-panel')
    expect(panel.exists()).toBe(true)
    // {{userId}} 被引用但无定义 → 未定义徽章；{{token}} 有定义 → 无警示。
    expect(panel.text()).toContain('1 个未定义')
    expect(panel.text()).toContain('未定义')
    const warnChip = panel.find('.se-var-chip.warn')
    expect(warnChip.text()).toBe('{{userId}}')
    // 点击变量名展开详情：来源 / 引用位置行。
    await panel.find('.se-var-entry-head').trigger('click')
    expect(panel.text()).toContain('来源')
  })

  it('执行策略展示为可读选项（默认失败即停）', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [{ ...makeScenario(), id: 'sc-strategy' }]
    store.endpoints = [makeEndpoint()]

    const wrapper = mountEditor({ scenarioId: 'sc-strategy' })
    await nextTick()

    expect(wrapper.find('.se-strategy').exists()).toBe(true)
    expect(wrapper.find('.se-strategy').text()).toContain('失败即停')
  })

  it('运行后渲染结果面板：摘要统计 + 步骤行 + 运行时值；点击行定位步骤', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [{ ...makeScenario(), id: 'sc-run' }]
    store.endpoints = [makeEndpoint()]
    runScenario.mockResolvedValue({
      results: [
        {
          endpoint_id: 'ep-1',
          endpoint_name: '用户文章',
          method: 'GET',
          path: '/users/1/posts',
          ok: true,
          status: 200,
          duration_ms: 12,
          request_error: null,
          url: 'http://localhost/users/1/posts',
          step_index: 0,
          cancelled: false,
          outcomes: [
            {
              description: '状态码等于 200',
              passed: true,
              reason: null,
              expected: '200',
              actual: '200',
            },
          ],
          extracted: { token: 'abc' },
          response: {
            status: 200,
            headers: [['content-type', 'application/json']],
            body: '{"ok":true}',
            content_type: 'application/json',
            duration_ms: 12,
            size_bytes: 11,
            truncated: false,
            body_omitted: false,
            request: null,
          },
        },
      ],
      cancelled: false,
      stopped_on_failure: false,
      variables: { token: 'abc' },
    })

    const wrapper = mountEditor({ scenarioId: 'sc-run' })
    await nextTick()

    await wrapper.find('.se-run').trigger('click')
    await flushPromises()

    const report = wrapper.find('.se-report')
    expect(report.exists()).toBe(true)
    expect(report.text()).toContain('全部通过')
    expect(report.text()).toContain('成功 1')
    expect(report.text()).toContain('200')
    // 结果自描述：渲染后的实际 URL 随行。
    expect(report.text()).toContain('http://localhost/users/1/posts')
    // 结构化断言对比：期望 / 实际。
    expect(report.text()).toContain('期望 200')
    expect(report.text()).toContain('实际 200')
    // 运行结果变量池进入变量链路（运行时值）。
    expect(wrapper.text()).toContain('abc')

    // 展开响应详情：复用 ResponsePanel（.rp 根节点；其内 JsonTree 依赖全局
    // tooltip-overflow 指令，测试环境需手动注册）。
    expect(report.find('.rp').exists()).toBe(false)
    await report.find('.se-rrow .ib').trigger('click')
    await nextTick()
    expect(wrapper.find('.rp').exists()).toBe(true)

    // 点击结果行 → 对应步骤卡展开定位。
    expect(wrapper.find('.se-step-body').exists()).toBe(false)
    await report.find('.se-rrow').trigger('click')
    await nextTick()
    expect(wrapper.find('.se-step-body').exists()).toBe(true)
  })

  it('页签切换（卸载重挂）不丢未保存编辑（模块级缓存恢复）', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [{ ...makeScenario(), id: 'sc-cache' }]
    store.endpoints = [makeEndpoint()]

    const wrapper = mountEditor({ scenarioId: 'sc-cache' })
    await nextTick()
    await wrapper.find('.se-name').setValue('改名了')
    wrapper.unmount()

    const restored = mountEditor({ scenarioId: 'sc-cache' })
    await nextTick()
    expect((restored.find('.se-name').element as HTMLInputElement).value).toBe('改名了')
    expect(restored.find('.se-unsaved').exists()).toBe(true)
    restored.unmount()
  })

  it('重开场景自动恢复最近一次运行（历史标记 + 结果面板渲染）', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [{ ...makeScenario(), id: 'sc-hist' }]
    store.endpoints = [makeEndpoint()]
    listScenarioRuns.mockResolvedValue([
      {
        id: 'run-1',
        finished_at: '2026-01-02T03:04:05.000Z',
        duration_ms: 12,
        total_steps: 1,
        ok_count: 1,
        fail_count: 0,
        skipped_count: 0,
        cancelled: false,
        stopped_on_failure: false,
      },
    ])
    getScenarioRun.mockResolvedValue({
      id: 'run-1',
      finished_at: '2026-01-02T03:04:05.000Z',
      duration_ms: 12,
      total_steps: 1,
      ok_count: 1,
      fail_count: 0,
      skipped_count: 0,
      cancelled: false,
      stopped_on_failure: false,
      results: [
        {
          endpoint_id: 'ep-1',
          endpoint_name: '用户文章',
          method: 'GET',
          path: '/users/1/posts',
          ok: true,
          status: 200,
          duration_ms: 12,
          request_error: null,
          url: 'http://localhost/users/1/posts',
          step_index: 0,
          outcomes: [],
          extracted: { token: 'abc' },
          response: null,
        },
      ],
      variables: { token: 'abc' },
    })

    const wrapper = mountEditor({ scenarioId: 'sc-hist' })
    await flushPromises()

    const report = wrapper.find('.se-report')
    expect(report.exists()).toBe(true)
    // 历史标记 + 摘要与结果渲染。
    expect(report.text()).toContain('历史')
    expect(report.text()).toContain('全部通过')
    expect(report.text()).toContain('成功 1')
    expect(report.text()).toContain('200')
    // 运行时值随历史恢复进变量链路。
    expect(wrapper.text()).toContain('abc')
  })

  it('重跑失败步骤：以原链路变量上下文运行子集并回填原位次', async () => {
    const store = useWorkspaceStore()
    store.scenarios = [
      {
        ...makeScenario(),
        id: 'sc-rerun',
        steps: [
          { endpoint_id: 'ep-1', enabled: true, extract: [], assertions: [] },
          { endpoint_id: 'ep-1', enabled: true, extract: [], assertions: [] },
        ],
      },
    ]
    store.endpoints = [makeEndpoint()]

    const okResult = {
      endpoint_id: 'ep-1',
      endpoint_name: '用户文章',
      method: 'GET',
      path: '/users/1/posts',
      ok: true,
      status: 200,
      duration_ms: 10,
      request_error: null,
      url: 'http://localhost/users/1/posts',
      step_index: 0,
      outcomes: [],
      extracted: {},
      response: null,
    }
    // 首跑：步 1 成功（提取 token），步 2 失败；快照含链路上下文。
    runScenario
      .mockResolvedValueOnce({
        results: [
          { ...okResult, step_index: 0, extracted: { token: 'abc' } },
          {
            ...okResult,
            ok: false,
            status: 500,
            step_index: 1,
            outcomes: [
              { description: '状态码等于 200', passed: false, reason: null, expected: '200', actual: '500' },
            ],
          },
        ],
        cancelled: false,
        stopped_on_failure: false,
        variables: { token: 'abc' },
        vars_snapshots: [{ static_v: 's1' }, { static_v: 's1', token: 'abc' }],
      })
      // 重跑：仅失败步；应收到初始变量快照（含首步提取值）。
      .mockResolvedValueOnce({
        results: [{ ...okResult, status: 200, step_index: 0 }],
        cancelled: false,
        stopped_on_failure: false,
        variables: { token: 'abc', extra: '1' },
        vars_snapshots: [{ static_v: 's1', token: 'abc' }],
      })

    const wrapper = mountEditor({ scenarioId: 'sc-rerun' })
    await nextTick()
    await wrapper.find('.se-run').trigger('click')
    await flushPromises()

    // 首跑后存在失败 → 重跑按钮可见。
    const headrow = wrapper.find('.se-report-headrow')
    const rerunBtn = headrow.findAll('.ib')[0]!
    await rerunBtn.trigger('click')
    await flushPromises()

    // 第二次调用：子集步骤 + 初始变量（首跑快照，含首步提取值），不失败即停。
    expect(runScenario).toHaveBeenCalledTimes(2)
    const second = runScenario.mock.calls[1]![0]!
    expect(second.steps).toHaveLength(1)
    expect(second.stopOnFailure).toBe(false)
    expect(second.initialVars).toEqual({ static_v: 's1', token: 'abc' })

    // 回填原位次：合并后 2 步全部通过，变量池重跑值覆盖。
    const report = wrapper.find('.se-report')
    expect(report.text()).toContain('全部通过')
    expect(report.text()).toContain('成功 2')
  })
})
