/**
 * EnvVarTable 长变量表滚动回归（issue #172）：
 * 「管理环境」页面变量太多时无法上下滑动——根因是变量区的 flex 链断了：
 * `.evt`（EnvVarTable 根）没有 `flex: 1; min-height: 0`，整块按内容高度撑开，
 * 把 `.em-table` 的滚动区顶出可视区域，外层又是 overflow: hidden，于是滚不动。
 *
 * 滚动是纯 CSS 行为（jsdom 不做布局），因此本 spec 直接断言关键规则存在，
 * 并在挂载后核对 DOM 结构（变量表放在 .em-table 里、行数 = 数据条数）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import EnvVarTable from '@/domains/environment/components/env/EnvVarTable.vue'
import { useLocaleStore } from '@/stores/locale'
import type { EnvironmentVariable } from '@/types/miyoApi.d'

const SRC = readFileSync(join(process.cwd(), 'src/domains/environment/components/env/EnvVarTable.vue'), 'utf8')
const WORKSPACE_SRC = readFileSync(join(process.cwd(), 'src/domains/environment/components/env/EnvWorkspace.vue'), 'utf8')
const GLOBAL_CSS = readFileSync(join(process.cwd(), 'src/style.css'), 'utf8')

/** 取某选择器的规则体（scoped 类名不变，直接按字面量匹配）。 */
function rule(css: string, selector: string): string {
  const m = css.match(new RegExp(`\\${selector}\\s*\\{([^}]*)\\}`))
  expect(m, `缺少规则 ${selector}`).toBeTruthy()
  return m![1]
}

function rows(n: number): EnvironmentVariable[] {
  return Array.from({ length: n }, (_, i) => ({
    key: `api_key_${i + 1}`,
    remote_value: `value-${i + 1}`,
    local_value: '',
    enabled: true,
    local_override: null,
    sensitive: null,
    description: null,
  }))
}

describe('EnvVarTable 长变量表滚动（issue #172）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('变量区根节点可压缩并占满剩余高度：flex:1 + min-height:0', () => {
    const body = rule(SRC, '.evt')
    expect(body).toMatch(/flex:\s*1/)
    expect(body).toMatch(/min-height:\s*0/)
    expect(body).toMatch(/flex-direction:\s*column/)
  })

  it('变量表容器自身滚动，不能撑破父级', () => {
    const table = rule(GLOBAL_CSS, '.em-table')
    expect(table).toMatch(/overflow-y:\s*auto/)
    expect(table).toMatch(/min-height:\s*0/)
    expect(table).toMatch(/flex:\s*1/)
  })

  it('变量区父级与右侧编辑栏都能压缩（整条 flex 链完整）', () => {
    const section = rule(WORKSPACE_SRC, '.em-section-vars')
    expect(section).toMatch(/flex:\s*1/)
    expect(section).toMatch(/min-height:\s*0/)
    const editor = rule(WORKSPACE_SRC, '.em-editor')
    expect(editor).toMatch(/min-height:\s*0/)
  })

  it('渲染 23 条变量：所有行都在内部滚动容器里，不在外层直接铺开', () => {
    const wrapper = mount(EnvVarTable, {
      props: { rows: rows(23), hideSensitive: true },
      attachTo: document.body,
    })
    const root = wrapper.find('.evt')
    expect(root.exists()).toBe(true)
    // 滚动容器是 .em-table，行数据都在它内部
    const table = root.find('.em-table')
    expect(table.exists()).toBe(true)
    expect(table.findAll('.evt-row')).toHaveLength(23)
    // 工具行（搜索 / 批量导入 / 遮蔽开关）在滚动区之外，滚动时始终可见
    expect(root.find('.evt-toolbar').exists()).toBe(true)
    expect(root.find('.em-table .evt-toolbar').exists()).toBe(false)
    wrapper.unmount()
  })
})