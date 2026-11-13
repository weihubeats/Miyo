/**
 * KeyValueTable 单测：Value 列 {{变量}} 自动补全（可选 varCandidates，不传不启用）。
 * 弹层 VarSuggest teleport 到 body，断言走 document 查询。
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import KeyValueTable, { type KVRow } from '@/components/ui/KeyValueTable.vue'
import { useLocaleStore } from '@/stores/locale'

let wrapper: VueWrapper | null = null

beforeEach(() => {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  document.querySelectorAll('.vs-pop').forEach((el) => el.remove())
})

function mountTable(modelValue: KVRow[] = [], varCandidates?: string[], searchable?: boolean) {
  const props: { modelValue: KVRow[]; varCandidates?: string[]; searchable?: boolean } = { modelValue }
  if (varCandidates) props.varCandidates = varCandidates
  if (searchable !== undefined) props.searchable = searchable
  wrapper = mount(KeyValueTable, { props, attachTo: document.body })
  return wrapper
}

function valueInput(w: VueWrapper): HTMLInputElement {
  return w.findAll('input.kvt-value')[0]!.element as HTMLInputElement
}

/** jsdom 下 setValue 不保证光标，显式设值 + 光标 + 派发 input（v-model 与补全都读 el.value）。 */
function typeInto(input: HTMLInputElement, text: string): void {
  input.value = text
  input.setSelectionRange(text.length, text.length)
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

function press(input: HTMLInputElement, key: string): void {
  input.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
}

function suggestions(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('.vs-pop .vs-item')]
}

describe('KeyValueTable 变量补全', () => {
  const emptyRow: KVRow = { key: '', value: '', enabled: true, description: '', }

  it('输入 {{ 展开候选，方向键切换高亮，Enter 插入 {{name}} 并回写 v-model', async () => {
    const w = mountTable([emptyRow], ['base_url', 'token', 'uuid_x'])
    const input = valueInput(w)

    typeInto(input, '{{')
    await nextTick()
    expect(suggestions().map((el) => el.textContent)).toEqual([
      '{{base_url}}',
      '{{token}}',
      '{{uuid_x}}',
    ])
    expect(suggestions()[0]!.classList.contains('vs-item-hl')).toBe(true)

    press(input, 'ArrowDown')
    await nextTick()
    expect(suggestions()[1]!.classList.contains('vs-item-hl')).toBe(true)

    press(input, 'Enter')
    await nextTick()
    expect(input.value).toBe('{{token}}')
    // emitted() 返回「每次 emit 的参数数组」，故多一层 [0] 取到 rows
    const emitted = w.emitted('update:modelValue') as unknown as KVRow[][][] | undefined
    // 行对象与调用方数组共享引用：v-model 回写后即为新值（空行有值后参与上抛）
    expect(emitted?.at(-1)?.[0]?.[0]).toMatchObject({ key: '', value: '{{token}}' })
    expect(document.querySelector('.vs-pop')).toBeNull()
  })

  it('继续输入按前缀过滤，Esc 关闭弹层', async () => {
    const w = mountTable([emptyRow], ['base_url', 'token'])
    const input = valueInput(w)

    typeInto(input, '{{to')
    await nextTick()
    expect(suggestions().map((el) => el.textContent)).toEqual(['{{token}}'])

    press(input, 'Escape')
    await nextTick()
    expect(document.querySelector('.vs-pop')).toBeNull()
    // Esc 不改值（留给上层「清空地址栏」等行为处理）
    expect(input.value).toBe('{{to')
  })

  it('未传 varCandidates：输入 {{ 不弹候选（保持既有调用方行为）', async () => {
    const w = mountTable([{ key: 'a', value: '', enabled: true, description: '', }])
    const input = w.findAll('input.kvt-value')[0]!.element as HTMLInputElement

    typeInto(input, '{{')
    await nextTick()
    expect(document.querySelector('.vs-pop')).toBeNull()
  })
})

describe('KeyValueTable 行管理（无幽灵行）', () => {
  it('空表只渲染「添加」入口；点击新增行并聚焦 Key', async () => {
    const w = mountTable([])
    expect(w.findAll('.kvt-row').length).toBe(0)
    expect(w.find('.kvt-add').exists()).toBe(true)

    await w.find('.kvt-add').trigger('click')
    await nextTick()
    expect(w.findAll('.kvt-row').length).toBe(1)
    // 新行 Key 输入框获得焦点
    expect(document.activeElement?.classList.contains('kvt-key')).toBe(true)
    const emitted = w.emitted('update:modelValue') as unknown as KVRow[][][] | undefined
    // 空行不上抛
    expect(emitted?.at(-1)?.[0]).toEqual([])
  })

  it('空行失焦自动移除（不留误导性空白占位行）', async () => {
    const w = mountTable([])
    await w.find('.kvt-add').trigger('click')
    await nextTick()
    expect(w.findAll('.kvt-row').length).toBe(1)

    await w.find('input.kvt-key').trigger('blur')
    await nextTick()
    expect(w.findAll('.kvt-row').length).toBe(0)
  })
})

describe('KeyValueTable 行过滤（searchable）', () => {
  const rows: KVRow[] = [
    { key: 'token', value: 'abc123', enabled: true, description: '鉴权', },
    { key: 'page', value: '2', enabled: true, description: '', },
    { key: 'size', value: '20', enabled: false, description: '页大小', },
  ]

  it('默认不渲染过滤条（既有调用方零感知）', () => {
    const w = mountTable(rows)
    expect(w.find('.kvt-search').exists()).toBe(false)
  })

  it('输入关键字只显示匹配行，过滤中的编辑仍回写原行', async () => {
    const w = mountTable(rows, undefined, true)
    expect(w.findAll('.kvt-row').length).toBe(3)

    const search = w.find('input.kvt-search-input')
    await search.setValue('页')
    await nextTick()
    // 「页」只命中 size 的描述「页大小」（token / page 均不含）
    expect(w.findAll('.kvt-row').length).toBe(1)

    // 过滤视图中编辑命中行：值写进原行对象并上抛
    typeInto(w.findAll('input.kvt-value')[0]!.element as HTMLInputElement, '20x')
    await nextTick()
    const emitted = w.emitted('update:modelValue') as unknown as KVRow[][][] | undefined
    const last = emitted?.at(-1)?.[0] ?? []
    expect(last.find((r) => r.key === 'size')?.value).toBe('20x')
    expect(last.find((r) => r.key === 'page')?.value).toBe('2')
  })

  it('无匹配显示占位，点清除恢复全部行', async () => {
    const w = mountTable(rows, undefined, true)
    await w.find('input.kvt-search-input').setValue('zzz-not-exist')
    await nextTick()
    expect(w.findAll('.kvt-row').length).toBe(0)
    expect(w.find('.kvt-empty').text()).toBe('没有匹配的行')

    await w.find('.kvt-search-clear').trigger('click')
    await nextTick()
    expect(w.findAll('.kvt-row').length).toBe(3)
    expect(w.find('.kvt-empty').exists()).toBe(false)
  })
})
