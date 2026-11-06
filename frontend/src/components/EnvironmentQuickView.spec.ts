/**
 * EnvironmentQuickView 快速编辑保存回归：
 * - 保存必须原样保留既有变量的 enabled / local_override / sensitive
 *   （历史 bug：速览保存把停用变量全部复活为启用、丢覆盖开关与敏感标注）；
 * - 仅编辑过的行把值写入本地覆盖（快速编辑即本地覆盖语义）；
 * - 未编辑行显示生效值（本地覆盖优先），保存后字段不变。
 *
 * 组件 Teleport 到 body：断言走 document.querySelector（attachTo + 清场）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import EnvironmentQuickView from './EnvironmentQuickView.vue'
import { useLocaleStore } from '../stores/locale'
import type { Environment, EnvironmentVariable } from '../types/miyoApi'

const VARS: EnvironmentVariable[] = [
  { key: 'token', remote_value: 'r1', local_value: '', enabled: false, description: null },
  { key: 'host', remote_value: 'h1', local_value: 'h2', enabled: true, description: null },
  { key: 'api_key', remote_value: 'k1', local_value: '', enabled: true, sensitive: true, description: null },
]

const ENV: Environment = {
  id: 'e1',
  project_id: 'p1',
  name: '开发',
  base_url: '',
  variables: VARS,
  created_at: '',
  updated_at: '',
}

const updateEnvironment = vi.fn(async (env: Environment) => ({ ...env }))

vi.mock('../stores/workspace', () => ({
  useWorkspaceStore: () => ({
    environments: [ENV],
    activeEnvId: 'e1',
    updateEnvironment,
  }),
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn() }),
}))

function qValueInputs(): HTMLInputElement[] {
  return Array.from(document.querySelectorAll<HTMLInputElement>('.qv-row input.qv-value'))
}

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  updateEnvironment.mockClear()
  document.body.innerHTML = ''
})

function payloadVars(): EnvironmentVariable[] {
  return updateEnvironment.mock.calls[0][0].variables as EnvironmentVariable[]
}

describe('EnvironmentQuickView 保存字段保留', () => {
  it('未编辑直接保存：停用态 / 覆盖开关 / 敏感标注原样保留', async () => {
    const wrapper = mount(EnvironmentQuickView, { props: { anchor: null }, attachTo: document.body })
    await flushPromises()

    // blur 任一行触发保存（未做任何编辑）
    document.querySelectorAll<HTMLInputElement>('.qv-row input.qv-key')[0]!.dispatchEvent(new Event('blur'))
    await flushPromises()
    wrapper.unmount()

    expect(updateEnvironment).toHaveBeenCalledTimes(1)
    const vars = payloadVars()
    expect(vars).toHaveLength(3)
    expect(vars[0]).toMatchObject({ key: 'token', enabled: false, local_value: '' })
    expect(vars[0]!.local_override ?? null).toBeNull()
    expect(vars[1]).toMatchObject({ key: 'host', enabled: true, local_value: 'h2' })
    expect(vars[1]!.local_override ?? null).toBeNull()
    expect(vars[2]).toMatchObject({ key: 'api_key', sensitive: true })
  })

  it('编辑停用变量的值：写入本地覆盖但不复活启用态', async () => {
    const wrapper = mount(EnvironmentQuickView, { props: { anchor: null }, attachTo: document.body })
    await flushPromises()

    // 第一行 = token（停用变量）：编辑值列 → blur 保存
    const valueInput = qValueInputs()[0]!
    valueInput.value = 'edited-token'
    valueInput.dispatchEvent(new Event('input'))
    valueInput.dispatchEvent(new Event('blur'))
    await flushPromises()
    wrapper.unmount()

    const token = payloadVars().find((v) => v.key === 'token')!
    expect(token.enabled).toBe(false) // 不复活
    expect(token.local_value).toBe('edited-token')
    expect(token.local_override).toBe(true) // 快速编辑即本地覆盖
  })

  it('值列显示生效值（本地覆盖优先于远程）', () => {
    mount(EnvironmentQuickView, { props: { anchor: null }, attachTo: document.body })
    // 第二行 = host：本地覆盖 h2 优先于远程 h1
    expect(qValueInputs()[1]!.value).toBe('h2')
  })
})
