/**
 * EndpointEditor 单测：配置区「Path」页签与请求设置 Popover（单请求超时 / 跟随重定向）。
 * 两者都绑定 draft.request，走既有脏检查与保存链路，故只断言字段落值。
 * 设置面板 Teleport 到 body（config-box 是滚动容器），DOM 断言须从 document 层查询。
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from './EndpointEditor.vue'
import CustomNumberInput from './ui/CustomNumberInput.vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { makeDraft } from '../testUtils/draftFixture'
import type { Endpoint } from '../types/miyoApi'

vi.mock('../composables/useMiyoApi', () => ({
  useMiyoApi: () => new Proxy({}, { get: () => () => Promise.resolve(null) }),
}))

async function mountEditor(prepare?: (ep: Endpoint) => void) {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
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
  const ep = makeDraft({ id: 'ep-1', name: '宠物', method: 'GET', path: '/pets/:id' })
  prepare?.(ep)
  store.endpoints = [ep]
  // 全量挂载：Path 页签由子组件 Tabs 渲染，shallow 会把它 stub 掉；
  // 全局指令（main.ts 注册）在单测里空实现，避免 unresolved 警告
  const wrapper = mount(EndpointEditor, {
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await nextTick()
  store.openEndpoint(ep)
  await nextTick()
  await nextTick()
  return { wrapper, store, ep }
}

describe('EndpointEditor：Path 页签', () => {
  it('页签列表含 Path，角标跟随 path_variables 条数；点击即切换', async () => {
    const { wrapper } = await mountEditor((ep) => {
      ep.request.path_variables = [
        { key: 'id', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
        { key: 'scope', value: '', enabled: false, description: '', field_type: 'string', required: true, example: '' },
      ]
    })

    const pathTab = wrapper
      .findAll('.tabs .tab')
      .find((t) => t.find('.tab-label').text() === 'Path')
    expect(pathTab).toBeTruthy()
    expect(pathTab!.find('.tab-badge').text()).toBe('2')

    await pathTab!.trigger('click')
    await nextTick()
    const active = wrapper
      .findAll('.tabs .tab')
      .find((t) => t.attributes('aria-selected') === 'true')
    expect(active?.find('.tab-label').text()).toBe('Path')
    wrapper.unmount()
  })
})

describe('EndpointEditor：请求设置 Popover', () => {
  /** 点触发钮展开面板，返回 document 层的弹层元素。 */
  async function openSettings(wrapper: ReturnType<typeof mount>) {
    await wrapper.find('.rs-trigger.rs-settings').trigger('click')
    await nextTick()
    return document.body.querySelector('.rs-pop')
  }

  it('默认收起：页签行有触发钮且无常驻设置行；展开后字段齐全、占位带全局值', async () => {
    const { wrapper } = await mountEditor()
    const trigger = wrapper.find('.rs-trigger.rs-settings')
    expect(trigger.exists()).toBe(true)
    expect(trigger.text()).toContain('请求设置')
    // 无常驻设置行：面板默认不在文档中
    expect(document.body.querySelector('.rs-pop')).toBeNull()

    const pop = await openSettings(wrapper)
    expect(pop?.textContent).toContain('超时')
    expect(pop?.textContent).toContain('毫秒')
    expect(pop?.textContent).toContain('跟随重定向')
    // mock 后端返回 null（从未配置）→ store 回退内置默认 → 占位符带秒数
    const input = wrapper.findComponent(CustomNumberInput)
    expect(input.props('placeholder')).toBe('留空用全局（300 秒）')
    wrapper.unmount()
  })

  it('Esc / 外部点击关闭；切换接口自动收起', async () => {
    const { wrapper, store } = await mountEditor()
    await openSettings(wrapper)
    expect(document.body.querySelector('.rs-pop')).toBeTruthy()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(document.body.querySelector('.rs-pop')).toBeNull()

    await openSettings(wrapper)
    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    await nextTick()
    expect(document.body.querySelector('.rs-pop')).toBeNull()

    // 切换接口：面板随 draft.id 变化收起（残留面板会挂在新接口上下文里）
    await openSettings(wrapper)
    const next = makeDraft({ id: 'ep-2', name: '另一接口', method: 'GET', path: '/x' })
    store.endpoints.push(next)
    store.openEndpoint(next)
    await nextTick()
    await nextTick()
    expect(document.body.querySelector('.rs-pop')).toBeNull()
    wrapper.unmount()
  })

  it('超时输入写入 request.timeout_ms：空串→null，非法输入不写回', async () => {
    const { wrapper, store } = await mountEditor()
    await openSettings(wrapper)
    const input = wrapper.findComponent(CustomNumberInput)
    expect(input.exists()).toBe(true)
    expect(input.props('modelValue')).toBe(30000)

    await input.vm.$emit('update:modelValue', '1500')
    expect(store.draftOf('ep-1')!.request.timeout_ms).toBe(1500)

    await input.vm.$emit('update:modelValue', '')
    expect(store.draftOf('ep-1')!.request.timeout_ms).toBeNull()

    await input.vm.$emit('update:modelValue', 2000)
    await input.vm.$emit('update:modelValue', 'abc')
    expect(store.draftOf('ep-1')!.request.timeout_ms).toBe(2000)
    wrapper.unmount()
  })

  it('「跟随重定向」复选直接驱动 request.follow_redirects', async () => {
    const { wrapper, store } = await mountEditor()
    await openSettings(wrapper)
    const checkbox = document.querySelector('.rs-check input') as HTMLInputElement
    expect(checkbox).toBeTruthy()
    expect(store.draftOf('ep-1')!.request.follow_redirects).toBe(true)

    // Teleport 面板不在 wrapper 子树内，直接改值 + 派发 change（与原生行为一致）
    checkbox.checked = false
    checkbox.dispatchEvent(new Event('change'))
    await nextTick()
    expect(store.draftOf('ep-1')!.request.follow_redirects).toBe(false)
    checkbox.checked = true
    checkbox.dispatchEvent(new Event('change'))
    await nextTick()
    expect(store.draftOf('ep-1')!.request.follow_redirects).toBe(true)
    wrapper.unmount()
  })

  it('非默认设置点亮触发钮角标：设了超时或关闭重定向；展开后角标隐藏', async () => {
    const first = await mountEditor((ep) => {
      ep.request.timeout_ms = 1500
    })
    expect(first.wrapper.find('.rs-dot').exists()).toBe(true)
    first.wrapper.unmount()

    const second = await mountEditor((ep) => {
      ep.request.follow_redirects = false
    })
    expect(second.wrapper.find('.rs-dot').exists()).toBe(true)
    await second.wrapper.find('.rs-trigger.rs-settings').trigger('click')
    await nextTick()
    expect(second.wrapper.find('.rs-dot').exists()).toBe(false)

    await second.wrapper.find('.rs-trigger.rs-settings').trigger('click')
    await nextTick()
    expect(document.body.querySelector('.rs-pop')).toBeNull()
    second.wrapper.unmount()
  })
})
