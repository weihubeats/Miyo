/**
 * ScenarioEditor 单测：挂载即加载场景并正常渲染。
 *
 * 回归护栏：immediate watch 在 setup 期间同步执行，若引用了声明在其后的
 * 状态（曾踩过 runResult 的暂时性死区），WebKit 下场景页签一打开就崩
 *（"Cannot access uninitialized variable"）。本测试保证挂载路径不被改坏。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

import ScenarioEditor from './ScenarioEditor.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import SegmentedControl from './ui/SegmentedControl.vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import type { Endpoint, Scenario } from '../types/miyoApi'

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

    const wrapper = mount(ScenarioEditor, { props: { scenarioId: 'sc-1' } })
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

    const wrapper = mount(ScenarioEditor, { props: { scenarioId: 'sc-1' } })
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

    const wrapper = mount(ScenarioEditor, { props: { scenarioId: 'gone' } })
    await nextTick()

    expect(wrapper.find('.se-missing').exists()).toBe(true)
  })
})
