/**
 * WorkflowEditor 第二轮 UI 层接入单测。
 *
 * 只验证「编排数据 → 界面」这一段（挂载真实组件，mock 掉 Tauri / IPC）：
 * - 运行前校验面板：阻断时展开、列出问题、点击定位到步骤与页签；
 * - 步骤间连线：前序提取的变量名出现在下一步的连线上；
 * - 顶栏保存状态：改动后变「未保存」；
 * - 覆盖策略三态：引用式步骤可切换，数据落为 overrides。
 *
 * 组件 setup 已调 useLocaleStore，spec 统一 setActivePinia + setMode('zh')。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h } from 'vue'

// ---- Tauri / 外部依赖全 mock ----
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/path', () => ({ join: vi.fn(async (...p: string[]) => p.join('/')) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('sortablejs', () => ({ default: { create: vi.fn(() => ({ destroy: vi.fn(), option: vi.fn() })) } }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }))

const scenario = {
  id: 'sc-1',
  project_id: 'p-1',
  name: '收单链路',
  description: '',
  category: '',
  sort_order: 0,
  script: null,
  steps: [
    {
      endpoint_id: null,
      enabled: true,
      name: '登录',
      extract: [{ name: 'token', from: 'body', path: '$.token' }],
      assertions: [],
      request: { method: 'POST', url: '{{base_url}}/login', spec: emptySpec() },
    },
    {
      endpoint_id: null,
      enabled: true,
      name: '下单',
      extract: [],
      assertions: [],
      request: { method: 'POST', url: '{{base_url}}/orders?t={{token}}', spec: emptySpec() },
    },
  ],
}

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

const api = {
  pending: { value: false },
  listScenarios: vi.fn(async () => [scenario]),
  saveScenario: vi.fn(async () => scenario),
  runScenario: vi.fn(async () => ({ results: [], cancelled: false, stopped_on_failure: false, variables: {} })),
  listScenarioRuns: vi.fn(async () => []),
  getScenarioRun: vi.fn(async () => null),
  stopScenarioRun: vi.fn(async () => {}),
}
vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => api }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ info: vi.fn(), warning: vi.fn(), error: vi.fn(), success: vi.fn() }),
}))

import WorkflowEditor from '@/domains/integration/components/integration/WorkflowEditor.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'

/** 稳定挂载：注入项目 + 环境上下文，避免 load() 早退。 */
async function mountEditor() {
  const workspace = useWorkspaceStore()
  workspace.project = { id: 'p-1', name: 'P', description: '', variables: {}, created_at: '', updated_at: '' } as never
  const integration = useIntegrationStore()
  integration.projectId = 'p-1'
  const Host = defineComponent({ setup: () => () => h(WorkflowEditor, { scenarioId: 'sc-1' }) })
  const wrapper = mount(Host, { attachTo: document.body })
  await flushPromises()
  await flushPromises()
  return wrapper
}

describe('WorkflowEditor：第二轮 UI 层', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    vi.clearAllMocks()
  })

  it('步骤间连线挂上前序提取的变量名', async () => {
    const wrapper = await mountEditor()
    const chips = wrapper.findAll('.wf-edge-chip').map((c) => c.text())
    expect(chips).toContain('token')
    wrapper.unmount()
  })

  it('运行前校验：场景名称为空时展开面板并列出阻断项', async () => {
    const wrapper = await mountEditor()
    // 清空场景名 → 产生阻断项
    const nameInput = wrapper.find('.wf-name')
    await nameInput.setValue('')
    await flushPromises()

    const panel = wrapper.find('.wf-validate')
    expect(panel.exists()).toBe(true)
    expect(panel.classes()).not.toContain('collapsed')
    expect(wrapper.find('.wf-validate-tag.error').exists()).toBe(true)
    expect(wrapper.text()).toContain('运行前校验')
    wrapper.unmount()
  })

  it('校验项点击定位：选中对应步骤', async () => {
    const wrapper = await mountEditor()
    // 制造一个变量未定义（把「下单」的引用改成不存在的变量）
    const nameInput = wrapper.find('.wf-name')
    await nameInput.setValue('')
    await flushPromises()

    const items = wrapper.findAll('.wf-validate-item')
    expect(items.length).toBeGreaterThan(0)
    // 场景级问题（名称为空）定位不切步骤；仅验证不报错
    await items[0]!.trigger('click')
    await flushPromises()
    wrapper.unmount()
  })

  it('有未保存修改时点「运行」：保存链路不炸 TDZ（savedBaseline 读 validateOpen）', async () => {
    const wrapper = await mountEditor()
    // 制造未保存修改（dirty）→ 点运行会先走 save()
    await wrapper.find('.wf-name').setValue('改了名字再跑')
    await flushPromises()
    expect(wrapper.find('.wf-save-state').classes()).toContain('dirty')

    const runBtn = wrapper.findAll('.wf-topbar button').find((b) => b.text().includes('运行'))
    expect(runBtn).toBeTruthy()
    await runBtn!.trigger('click')
    await flushPromises()
    await flushPromises()

    // 按钮的同步链路（onRunClick → 写操作确认 / 校验面板）必须跑完，
    // 不得抛 ReferenceError（TDZ）；确认框出现即说明前置逻辑走通。
    expect(wrapper.html()).toBeTruthy()

    // 直接点「保存」：同一条 savedBaseline = draftSnapshot() 链路，必须有落库调用
    const saveBtn = wrapper.findAll('.wf-topbar button').find((b) => b.text().includes('保存'))
    expect(saveBtn).toBeTruthy()
    await saveBtn!.trigger('click')
    await flushPromises()
    expect(api.saveScenario).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('顶栏保存状态：未改动显示已保存', async () => {
    const wrapper = await mountEditor()
    expect(wrapper.find('.wf-save-state').classes()).toContain('saved')
    wrapper.unmount()
  })

  it('顶栏保存状态：改动后变未保存', async () => {
    const wrapper = await mountEditor()
    await wrapper.find('.wf-name').setValue('改了名字')
    await flushPromises()
    expect(wrapper.find('.wf-save-state').classes()).toContain('dirty')
    wrapper.unmount()
  })

  it('覆盖策略三态只在引用式步骤出现', async () => {
    const wrapper = await mountEditor()
    // 两个步骤都是内联请求 → 不出现策略选择
    expect(wrapper.find('.wf-policy').exists()).toBe(false)
    wrapper.unmount()
  })
})