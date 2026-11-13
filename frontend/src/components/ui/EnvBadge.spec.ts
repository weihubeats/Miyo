/**
 * EnvBadge 单测：激活环境展示（名称 + Base URL + 语义色）/ 无激活环境回落「无环境」。
 * 用例运行（抽屉「立即运行」/ 面板「全部运行」）的环境可见性回归。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import EnvBadge from '@/components/ui/EnvBadge.vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import type { Environment } from '@/types/miyoApi'

function makeEnv(id: string, name: string, base_url: string): Environment {
  return {
    id,
    project_id: 'p1',
    name,
    base_url,
    variables: [],
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

describe('EnvBadge', () => {
  it('展示激活环境的名称与 Base URL', () => {
    const store = useWorkspaceStore()
    store.environments = [makeEnv('e1', '开发环境', 'http://127.0.0.1:4010')]
    store.activeEnvId = 'e1'
    const wrapper = mount(EnvBadge)
    expect(wrapper.text()).toContain('开发环境')
    expect(wrapper.text()).toContain('http://127.0.0.1:4010')
    // 语义色点：dev 归类
    expect(wrapper.find('.env-badge-dot').classes()).toContain('dev')
  })

  it('无激活环境时回落「无环境」', () => {
    const wrapper = mount(EnvBadge)
    expect(wrapper.text()).toContain('无环境')
    expect(wrapper.find('.env-badge-url').exists()).toBe(false)
  })

  it('activeEnvId 指向不存在的环境时按无环境处理', () => {
    const store = useWorkspaceStore()
    store.environments = [makeEnv('e1', '开发环境', 'http://127.0.0.1:4010')]
    store.activeEnvId = 'missing'
    const wrapper = mount(EnvBadge)
    expect(wrapper.text()).toContain('无环境')
  })
})
