/**
 * EndpointEditor 接口名输入框单测：
 *
 * 回归「下面的这个滚动条还是没办法拖动」（v0.0.57 用户实测）。
 * 箭头指向的并不是滚动条，而是面包屑里的接口名 input——它不写 size 时按浏览器
 * 默认（≈20 字符 ≈ 154px）撑开，短名字（如「SS」）聚焦全选后整条是一根蓝色长条，
 * 看起来就是一根拖不动的滚动条。
 *
 * 修复：`field-sizing: content`（Safari 17+）+ 按内容长度给 size 兜底老 WebView。
 * 这里锁住后端不依赖布局的那一半（size 随名字长度变化），
 * 样式侧靠源码断言（jsdom 无布局，与 TabBar.spec 同款做法）。
 */
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint } from '@/types/miyoApi'

const apiMock = vi.hoisted(() => ({
  executeRequest: vi.fn(async () => {
    throw new Error('not used')
  }),
  listExamples: vi.fn(async () => []),
  listRequestExamples: vi.fn(async () => []),
  listTestCases: vi.fn(async () => []),
  listHistories: vi.fn(async () => []),
  listEnvironments: vi.fn(async () => []),
  getActiveEnvironment: vi.fn(async () => null),
  getGlobalVariables: vi.fn(async () => []),
  getGlobalParams: vi.fn(async () => []),
  saveRequestExample: vi.fn(),
  saveExample: vi.fn(),
  parseCurlCommand: vi.fn(async () => null),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))

async function mountWithName(name: string) {
  const store = useWorkspaceStore()
  useLocaleStore().setMode('zh')
  store.project = {
    id: 'proj-name-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  const endpoint: Endpoint = makeDraft({ id: `ep-${name}`, name, path: '/' })
  store.endpoints = [endpoint]
  const wrapper = mount(EndpointEditor, { shallow: true })
  await nextTick()
  store.openEndpoint(endpoint)
  await nextTick()
  return wrapper
}

describe('EndpointEditor：接口名输入框不撑成「假滚动条」', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('短名字的 size 收缩到内容长度（不再占满浏览器默认 20 字符）', async () => {
    const wrapper = await mountWithName('SS')
    const input = wrapper.find('.crumb-name')
    expect(input.exists()).toBe(true)
    // 2 字符 + 1 位光标余量
    expect(input.attributes('size')).toBe('3')
    wrapper.unmount()
  })

  it('长名字的 size 随内容增长（受上限约束）', async () => {
    const wrapper = await mountWithName('发布订单到支付中心并回调通知')
    const input = wrapper.find('.crumb-name')
    // 14 个字符 + 1 位光标余量
    expect(Number(input.attributes('size'))).toBe(15)
    wrapper.unmount()
  })

  it('空名字仍保留最小可点击宽度', async () => {
    const wrapper = await mountWithName('')
    expect(wrapper.find('.crumb-name').attributes('size')).toBe('2')
    wrapper.unmount()
  })

  it('改名字后 size 跟着更新（不只在挂载时求值）', async () => {
    const wrapper = await mountWithName('SS')
    const input = wrapper.find('.crumb-name')
    await input.setValue('新名字更长了')
    await nextTick()
    expect(Number(wrapper.find('.crumb-name').attributes('size'))).toBe(7)
    wrapper.unmount()
  })

  it('样式侧声明了 field-sizing: content（撑满容器宽度改成跟内容走）', () => {
    const src = readFileSync(join(process.cwd(), 'src/domains/endpoint/components/EndpointEditor.vue'), 'utf8')
    const style = src.slice(src.indexOf('<style'))
    expect(style).toMatch(/\.crumb-name\s*\{[\s\S]*?field-sizing:\s*content/)
  })
})