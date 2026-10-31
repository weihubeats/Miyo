/**
 * QuickSearch 单测：空查询展示最近与操作、模糊过滤、Enter 打开、Esc 关闭。
 * 面板 Teleport 到 body——断言与事件都走 document（CodeExportDialog.spec 同款）。
 * store 直接置数据（无 IPC 依赖；store 创建期的后台项目恢复失败已被吞掉）。
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import QuickSearch from './QuickSearch.vue'
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
    name: '登录链路',
    description: '',
    sort_order: 0,
    steps: [],
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

function bodyText(): string {
  return document.body.textContent ?? ''
}

function inputEl(): HTMLInputElement {
  const el = document.body.querySelector<HTMLInputElement>('.qs-input')
  expect(el).toBeTruthy()
  return el!
}

async function setQuery(value: string): Promise<void> {
  const el = inputEl()
  el.value = value
  el.dispatchEvent(new Event('input'))
  await nextTick()
}

async function pressKey(key: string): Promise<void> {
  inputEl().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await nextTick()
}

describe('QuickSearch 命令面板', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    const store = useWorkspaceStore()
    store.endpoints = [makeEndpoint()]
    store.scenarios = [makeScenario()]
    store.environments = []
    store.histories = []
    store.recentEndpointIds = ['ep-1']
  })

  it('空查询展示最近打开与全局操作', async () => {
    const wrapper = mount(QuickSearch, { props: { open: true }, attachTo: document.body })
    await nextTick()
    expect(bodyText()).toContain('最近打开')
    expect(bodyText()).toContain('用户文章')
    expect(bodyText()).toContain('新建请求')
    expect(bodyText()).toContain('打开设置')
    wrapper.unmount()
  })

  it('Enter 打开最近接口：激活页签切换并请求关闭', async () => {
    const store = useWorkspaceStore()
    const wrapper = mount(QuickSearch, { props: { open: true }, attachTo: document.body })
    await nextTick()
    await pressKey('Enter')
    expect(store.activeTabId).toBe('ep-1')
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
    wrapper.unmount()
  })

  it('模糊查询命中场景（子序列），Enter 打开场景页签', async () => {
    const store = useWorkspaceStore()
    const wrapper = mount(QuickSearch, { props: { open: true }, attachTo: document.body })
    await nextTick()
    await setQuery('登链')
    expect(bodyText()).toContain('登录链路')
    // 接口名不含该子序列（「用户文章」），不应混入结果
    expect(bodyText()).not.toContain('用户文章')
    await pressKey('Enter')
    expect(store.activeTabId).toBe('scenario:sc-1')
    wrapper.unmount()
  })

  it('Esc 关闭面板', async () => {
    const wrapper = mount(QuickSearch, { props: { open: true }, attachTo: document.body })
    await nextTick()
    await pressKey('Escape')
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false])
    wrapper.unmount()
  })

  it('无匹配展示空状态', async () => {
    const wrapper = mount(QuickSearch, { props: { open: true }, attachTo: document.body })
    await nextTick()
    await setQuery('完全不存在的关键词xyz')
    expect(bodyText()).toContain('无匹配结果')
    wrapper.unmount()
  })

  it('关闭后不渲染（v-if 卸载）', async () => {
    const wrapper = mount(QuickSearch, { props: { open: false } })
    expect(document.body.querySelector('.qs-backdrop')).toBeNull()
    wrapper.unmount()
  })
})
