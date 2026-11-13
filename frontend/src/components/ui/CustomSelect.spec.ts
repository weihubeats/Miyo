/**
 * CustomSelect 单测：弹窗最小宽度 / open-close 事件 / 选项 title（长标签悬停可读）。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import { useLocaleStore } from '@/stores/locale'

const OPTIONS = [
  { value: '', label: '默认模块' },
  { value: 'm1', label: '小奏技术·用户服务（很长的模块名会被截断）' },
]

function mountSelect(props: Record<string, unknown> = {}) {
  return mount(CustomSelect, {
    props: { options: OPTIONS, ...props },
    attachTo: document.body,
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('CustomSelect：下拉行为', () => {
  it('打开菜单触发 open，选择后触发 change/update 与 close', async () => {
    const wrapper = mountSelect({ modelValue: '' })
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    expect(wrapper.emitted('open')).toHaveLength(1)
    expect(document.querySelector('.cs-pop')).toBeTruthy()

    const opts = Array.from(document.querySelectorAll<HTMLElement>('.cs-opt'))
    expect(opts).toHaveLength(2)
    opts[1].click()
    await nextTick()
    expect(wrapper.emitted('change')?.[0]).toEqual(['m1'])
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['m1'])
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(document.querySelector('.cs-pop')).toBeNull()
    wrapper.unmount()
  })

  it('默认弹窗与触发器等宽；popMinWidth 撑宽窄触发器', async () => {
    const narrow = mountSelect({ modelValue: '' })
    await narrow.find('.cs-trigger').trigger('click')
    await nextTick()
    // jsdom 无布局，触发器宽为 0：默认等宽即 0
    expect((document.querySelector('.cs-pop') as HTMLElement).style.width).toBe('0px')
    narrow.unmount()
    document.body.innerHTML = ''

    const wide = mountSelect({ modelValue: '', popMinWidth: 220 })
    await wide.find('.cs-trigger').trigger('click')
    await nextTick()
    expect((document.querySelector('.cs-pop') as HTMLElement).style.width).toBe('220px')
    wide.unmount()
  })

  it('选项带 title，截断的长标签悬停可查看全文', async () => {
    const wrapper = mountSelect({ modelValue: '' })
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    const labels = Array.from(document.querySelectorAll('.cs-opt-label'))
    expect(labels[1].getAttribute('title')).toBe(OPTIONS[1].label)
    wrapper.unmount()
  })
})

describe('CustomSelect：header 分隔项', () => {
  const GROUPED = [
    { value: 'GET', label: 'GET' },
    { value: 'POST', label: 'POST' },
    { value: '__h__', label: '其他协议', header: true },
    { value: 'GRPC', label: 'GRPC' },
  ]

  function mountGrouped(modelValue = 'GET') {
    return mount(CustomSelect, {
      props: { options: GROUPED, modelValue },
      attachTo: document.body,
    })
  }

  it('header 渲染为分隔标题，不是可选项（无 role=option、点击不触发 change）', async () => {
    const wrapper = mountGrouped()
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    const header = document.querySelector('.cs-opt-header')
    expect(header?.textContent).toContain('其他协议')
    // 可选 .cs-opt 只有 3 个（header 不在内）
    expect(document.querySelectorAll('.cs-opt')).toHaveLength(3)
    header!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(wrapper.emitted('change')).toBeUndefined()
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('键盘 ArrowDown 从选中项循环到下一可选项，跳过 header', async () => {
    const wrapper = mountGrouped('POST') // POST 的下一个可选是 GRPC（跳过 header）
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    await wrapper.find('.cs-trigger').trigger('keydown', { key: 'ArrowDown' })
    await nextTick()
    const hl = document.querySelector('.cs-opt.hl .cs-opt-label')
    expect(hl?.textContent).toBe('GRPC')
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('选中项为最后一个时 ArrowDown 循环回首个可选项', async () => {
    const wrapper = mountGrouped('GRPC')
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    await wrapper.find('.cs-trigger').trigger('keydown', { key: 'ArrowDown' })
    await nextTick()
    expect(document.querySelector('.cs-opt.hl .cs-opt-label')?.textContent).toBe('GET')
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('Enter 在高亮项上确认选中（header 永不成为高亮）', async () => {
    const wrapper = mountGrouped('GET')
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    await wrapper.find('.cs-trigger').trigger('keydown', { key: 'ArrowDown' }) // POST
    await wrapper.find('.cs-trigger').trigger('keydown', { key: 'Enter' })
    await nextTick()
    expect(wrapper.emitted('change')?.[0]).toEqual(['POST'])
    wrapper.unmount()
    document.body.innerHTML = ''
  })
})

describe('CustomSelect：tone 语义色与勾选位置', () => {
  const TONED = [
    { value: 'GET', label: 'GET', tone: 'text-method-get' },
    { value: 'POST', label: 'POST' },
  ]

  it('tone 类透传到选项行；选中 tone 行不再用 accent 着色（class 组合正确）', async () => {
    const wrapper = mount(CustomSelect, {
      props: { options: TONED, modelValue: 'GET' },
      attachTo: document.body,
    })
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.cs-opt'))
    expect(rows[0].classList.contains('text-method-get')).toBe(true)
    // tone + sel 并存：CSS 由 .cs-opt.tone.sel 接管（语义色浅底），不落回 accent
    expect(rows[0].classList.contains('sel')).toBe(true)
    expect(rows[0].classList.contains('tone')).toBe(true)
    // 无 tone 的行不携带 tone 类
    expect(rows[1].classList.contains('tone')).toBe(false)
    wrapper.unmount()
    document.body.innerHTML = ''
  })

  it('勾选符在行尾（label 之后），未选中行不渲染勾选图标', async () => {
    const wrapper = mount(CustomSelect, {
      props: { options: TONED, modelValue: 'GET' },
      attachTo: document.body,
    })
    await wrapper.find('.cs-trigger').trigger('click')
    await nextTick()
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.cs-opt'))
    // 行内顺序：label 在前、check 在后（勾选统一行尾，文本严格左对齐）
    const label = rows[0].querySelector('.cs-opt-label')!
    const check = rows[0].querySelector('.cs-opt-check')!
    expect(label.nextElementSibling?.contains(check) || label.nextElementSibling === check).toBe(true)
    // 未选中行没有勾选图标
    expect(rows[1].querySelector('.cs-opt-check svg')).toBeNull()
    wrapper.unmount()
    document.body.innerHTML = ''
  })
})
