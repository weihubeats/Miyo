/**
 * EndpointEditor 回归测试：配置区 Tab 计数与表格可见行一致。
 *
 * 用户实测 bug：Headers 里只填了一条请求头，Tab 上的徽章显示 2。
 * 根因是 KeyValueTable 会把底部空白「幽灵行」一并回写给父级（sync() 保留末行），
 * 而 Tab 计数直接取 `request.headers.length` —— 幽灵行被算进去了。
 * 契约：计数只统计「有键或有值」的行，三类 kv（Params / Headers / Path）同口径。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import { stubScrollIntoView } from '@/testUtils/componentTest'
import type { Endpoint } from '@/types/miyoApi'

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => new Proxy({}, { get: () => () => Promise.resolve(null) }),
}))

/** 请求里带真实行 + 幽灵行（表格实际回写的形状）。 */
const endpoint: Endpoint = makeDraft({
  id: 'ep-cnt',
  name: '计数接口',
  method: 'POST',
  path: '/orders/{orderNo}',
})

endpoint.request.headers = [
  { key: 'Cookie', value: 'sid=1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
  { key: '', value: '', enabled: true, description: '', field_type: 'string', required: true, example: '' },
]
endpoint.request.params = [
  { key: 'page', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
  { key: '', value: '', enabled: true, description: '', field_type: 'string', required: true, example: '' },
]
endpoint.request.path_variables = [
  { key: 'orderNo', value: 'SO1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
  { key: '', value: '', enabled: true, description: '', field_type: 'string', required: true, example: '' },
]

async function mountEditor() {
  setActivePinia(createPinia())
  stubScrollIntoView()
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.endpoints = [endpoint]
  const wrapper = mount(EndpointEditor, {
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await nextTick()
  store.openEndpoint(endpoint)
  await nextTick()
  return { wrapper, store }
}

/** 取某个配置 Tab 的徽章数字（无徽章返回 null）。 */
function badgeOf(
  wrapper: Awaited<ReturnType<typeof mountEditor>>['wrapper'],
  labelStart: string,
): string | null {
  const tab = wrapper.findAll('.tab').find((b) => b.text().startsWith(labelStart))
  if (!tab) return null
  const badge = tab.find('.tab-badge')
  return badge.exists() ? badge.text() : null
}

describe('EndpointEditor：配置 Tab 计数不含空白幽灵行', () => {
  it('1 条请求头 + 1 条幽灵行 → Headers 徽章显示 1', async () => {
    const { wrapper } = await mountEditor()
    expect(badgeOf(wrapper, 'Headers')).toBe('1')
  })

  it('Params / Path 同口径：只算有内容的行', async () => {
    const { wrapper } = await mountEditor()
    expect(badgeOf(wrapper, 'Params')).toBe('1')
    expect(badgeOf(wrapper, 'Path')).toBe('1')
  })

  it('纯空白（空格）行也不计数', async () => {
    const { wrapper, store } = await mountEditor()
    const draft = store.activeEndpoint!
    draft.request.headers.push({ key: '   ', value: '  ', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    await nextTick()
    expect(badgeOf(wrapper, 'Headers')).toBe('1')
  })
})