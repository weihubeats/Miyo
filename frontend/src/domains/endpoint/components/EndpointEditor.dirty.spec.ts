/**
 * EndpointEditor 回归测试：打开接口 / 保存接口后不得被误判为「有改动」。
 *
 * 背景一：method watcher 监听 draft.method，但打开/切换接口时 draft 从
 * null → 有值，method 必然「变化」一次——旧实现把它当作用户改方法，
 * 回写 applyMethodDefaults 副作用（body 初始化 / Content-Type / active_tab），
 * 刚打开的接口立刻 isDirty。修复后：同一接口内的 method 变化才应用默认。
 *
 * 背景二（用户实测「导入 curl 后保存了，左边还有未保存的小点」）：method
 * watcher 的源是「每次求值都新建的数组」，Vue 的 hasChanged 对数组按引用比较，
 * 于是**草稿对象被替换**（保存成功时 store 把 draft 换成后端回执、打开接口）
 * 也会触发回调；回调里只挡了「id 变化」，同 id 的替换继续走到
 * applyMethodDefaults——把 active_tab 写回刚保存的草稿（POST 空体还会被塞进
 * JSON body + Content-Type）→ 保存成功即变「有改动」，且用户可见内容被悄悄改写。
 */
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint } from '@/types/miyoApi'

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () =>
    new Proxy(
      {
        // 真后端 save_endpoint 原样回执（crates/miyo-tauri .../endpoint.rs）。
        saveEndpoint: async (ep: Endpoint) => JSON.parse(JSON.stringify(ep)) as Endpoint,
      },
      { get: (t, p) => (p in t ? t[p as 'saveEndpoint'] : () => Promise.resolve(null)) },
    ),
}))

/** 浅挂载 + 显式 BodyPanel 桩（真实 BodyPanel 才有 flushPendingInput，
 *  浅挂载的自动桩没有，保存按钮路径会抛错）。 */
function mountEditorShallow() {
  return mount(EndpointEditor, {
    shallow: true,
    global: {
      stubs: { BodyPanel: { setup: () => ({ flushPendingInput: () => {} }), template: '<div />' } },
    },
  })
}

function endpoint(method: Endpoint['method']): Endpoint {
  return makeDraft({ id: 'ep-1', name: '新建宠物', method, path: '/pets' })
}

async function mountEditor() {
  setActivePinia(createPinia())
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  const ep = endpoint('GET')
  store.endpoints = [ep]
  const wrapper = mount(EndpointEditor, { shallow: true })
  await nextTick()
  return { wrapper, store, ep }
}

describe('EndpointEditor：打开接口不误报脏', () => {
  it('打开 GET 接口后 isDirty 为 false（草稿不被回写）', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()
    await nextTick()

    expect(store.isDirty(ep.id)).toBe(false)
    const draft = store.draftOf(ep.id)
    expect(draft?.request.body).toEqual({ mode: 'none' })
    // 夹具无 active_tab 字段（undefined）：只要没被 watcher 写入即可
    expect(draft?.request.active_tab ?? null).toBeNull()
    wrapper.unmount()
  })

  it('同一接口内手动改 Method 仍应用智能默认（body 初始化 + active_tab）', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()

    // 模拟用户在方法选择器 GET → POST（id 不变 → 视为用户编辑）
    const draft = store.draftOf(ep.id)!
    expect(draft.method).toBe('GET')
    draft.method = 'POST'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(true))

    expect(draft.request.body).toMatchObject({ mode: 'json' })
    expect(draft.request.active_tab).toBe('body')
    wrapper.unmount()
  })

  it('回归：GET→POST→GET 往返后不再显示「有改动」', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()
    const draft = store.draftOf(ep.id)!

    draft.method = 'POST'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(true))

    draft.method = 'GET'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(false))

    // request 完全还原为保存态（body / active_tab / POST 附加的 Content-Type 头被撤销）
    expect(draft.request.body).toEqual({ mode: 'none' })
    expect(draft.request.active_tab ?? null).toBeNull()
    expect(draft.request.headers.some((h) => h.key === 'Content-Type')).toBe(false)
    wrapper.unmount()
  })

  /**
   * 回归（用户实测：「接口已经保存了，为什么左边还有一个小点」）：
   * active_tab 既是持久化字段又是 isDirty 的比较项，页签 setter 写回草稿时，
   * 「只是切到 Headers 看一眼」就把接口标记成有改动——保存成功后再点个页签，
   * 左侧树的小圆点又亮。页签属于查看偏好，不得进入脏检查。
   */
  it('点击配置页签不写回草稿、不产生「有改动」（保存后切页签仍是干净的）', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()
    await nextTick()
    expect(store.isDirty(ep.id)).toBe(false)

    const draft = store.draftOf(ep.id)!
    const before = JSON.stringify(draft.request)

    // 用户在配置页签之间来回切（Params → Headers → Body → Examples → Params）
    const tabs = wrapper.findComponent({ name: 'Tabs' })
    for (const key of ['headers', 'body', 'examples', 'params']) {
      tabs.vm.$emit('update:modelValue', key)
      await nextTick()
      await nextTick()
      expect(store.isDirty(ep.id)).toBe(false)
    }

    // 整个 request 一字未动（页签选择没有混进请求体）
    expect(JSON.stringify(draft.request)).toBe(before)
    expect(draft.request.active_tab ?? null).toBeNull()
    wrapper.unmount()
  })

  it('已保存 active_tab 的接口：打开后切页签同样不判脏', async () => {
    setActivePinia(createPinia())
    const store = useWorkspaceStore()
    store.project = {
      id: 'proj-test-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    const ep = makeDraft({ id: 'ep-2', name: 'ss', method: 'GET', path: '/' })
    ep.request.active_tab = 'params'
    store.endpoints = [ep]
    const wrapper = mount(EndpointEditor, { shallow: true })
    await nextTick()
    store.openEndpoint(ep)
    await nextTick()
    await nextTick()

    const tabs = wrapper.findComponent({ name: 'Tabs' })
    tabs.vm.$emit('update:modelValue', 'headers')
    await nextTick()
    await nextTick()
    expect(store.isDirty(ep.id)).toBe(false)
    // 保存态里的页签记忆不回写、不被抹掉
    expect(store.draftOf(ep.id)?.request.active_tab).toBe('params')
    wrapper.unmount()
  })

  it('往返还原即使中途改过 body 内容也整体回到快照（用户语义：改回=没改）', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()
    const draft = store.draftOf(ep.id)!

    draft.method = 'POST'
    await vi.waitFor(() => expect(draft.request.body).toMatchObject({ mode: 'json' }))
    draft.request.body = { mode: 'json', raw: '{"typed":1}' }

    draft.method = 'GET'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(false))
    expect(draft.request.body).toEqual({ mode: 'none' })
    wrapper.unmount()
  })

  it('保存后再改回原方法：与新的保存态不同，仍正确显示「有改动」', async () => {
    const { wrapper, store, ep } = await mountEditor()

    store.openEndpoint(ep)
    await nextTick()
    const draft = store.draftOf(ep.id)!

    draft.method = 'POST'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(true))

    // 模拟保存：保存态同步为当前草稿（POST + json body）
    const savedNow = JSON.parse(JSON.stringify(draft)) as Endpoint
    store.endpoints = [savedNow]

    draft.method = 'GET'
    await vi.waitFor(() => expect(store.isDirty(ep.id)).toBe(true))
    expect(draft.request.body).toEqual({ mode: 'none' })
    wrapper.unmount()
  })

  /**
   * 回归（用户实测：「导入 curl 后保存了，左边还有未保存的小点」）。
   * 保存成功会把草稿整体换成后端回执（store.saveActiveDraft）；旧实现下
   * method watcher 因「数组源按引用比较」被这次替换触发，同 id 直接落到
   * applyMethodDefaults：GET 被写回 active_tab='params' → 立刻又判脏。
   */
  it('保存已存接口后不再显示「有改动」（草稿被保存态替换不算编辑）', async () => {
    setActivePinia(createPinia())
    const store = useWorkspaceStore()
    store.project = {
      id: 'proj-test-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    const ep = makeDraft({ id: 'ep-save-1', name: '查询渠道订单', method: 'GET', path: '/orders' })
    store.endpoints = [ep]
    const wrapper = mountEditorShallow()
    await nextTick()

    store.openEndpoint(ep)
    await nextTick()
    await nextTick()
    expect(store.isDirty(ep.id)).toBe(false)

    store.draftOf(ep.id)!.name = '查询渠道订单 v2'
    await wrapper.find('.save-main').trigger('click')
    await flushPromises()
    await nextTick()
    await nextTick()

    const draft = store.draftOf(ep.id)!
    // 保存后草稿与保存态一字不差（旧实现会多出 active_tab）
    expect(JSON.stringify(draft)).toBe(JSON.stringify(store.endpoints[0]))
    expect(store.isDirty(ep.id)).toBe(false)
    wrapper.unmount()
  })

  /**
   * 同源反例（同一次污染的另一半症状）：POST 空体接口保存后，草稿被
   * applyMethodDefaults 塞进 JSON body + Content-Type——不仅亮小点，
   * 编辑器里还会凭空出现一个用户没写过的请求体。
   */
  it('保存 POST 空体接口后不注入 body / Content-Type，且不判脏', async () => {
    setActivePinia(createPinia())
    const store = useWorkspaceStore()
    store.project = {
      id: 'proj-test-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    const ep = makeDraft({
      id: 'ep-save-2',
      name: '查询渠道订单',
      method: 'POST',
      path: '/orders',
      request: { ...makeDraft().request, body: { mode: 'none' }, auth: { type: 'none' } },
    })
    store.endpoints = [ep]
    const wrapper = mountEditorShallow()
    await nextTick()

    store.openEndpoint(ep)
    await nextTick()
    await nextTick()
    expect(store.isDirty(ep.id)).toBe(false)

    store.draftOf(ep.id)!.name = '查询渠道订单 v2'
    await wrapper.find('.save-main').trigger('click')
    await flushPromises()
    await nextTick()
    await nextTick()

    const draft = store.draftOf(ep.id)!
    expect(draft.request.body).toEqual({ mode: 'none' })
    expect(draft.request.headers.some((h) => h.key === 'Content-Type')).toBe(false)
    expect(store.isDirty(ep.id)).toBe(false)
    wrapper.unmount()
  })
})
