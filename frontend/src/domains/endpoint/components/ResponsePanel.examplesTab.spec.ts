/**
 * 响应示例页签契约：示例不再作为独立卡片压在响应区下方。
 *
 * 背景：`.examples` 原先是 `.req-resp-stack` 的兄弟节点，而后者 `flex:1` 吃满
 * 工作区 → 示例卡片被挤到最底部、离开主视区（用户实测反馈）。
 *
 * 修法：示例移入响应面板页签，数据仍由 EndpointEditor 持有（经 #examples 插槽注入）。
 * jsdom 不做样式计算且本例是「模板归属」契约，故按源码断言。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const PANEL = readFileSync(join(process.cwd(), 'src/domains/endpoint/components/ResponsePanel.vue'), 'utf8')
const EDITOR = readFileSync(join(process.cwd(), 'src/domains/endpoint/components/EndpointEditor.vue'), 'utf8')

describe('响应示例：归属响应面板页签', () => {
  it('响应面板注册「示例」页签，且仅在 examplesCount > 0 时出现', () => {
    expect(PANEL, '需声明 examplesCount prop').toContain('examplesCount')
    expect(PANEL, '需有 examples 页签键').toMatch(/key:\s*'examples'/)
    // 空页签占位无意义：必须用数量做条件
    expect(PANEL, '页签须按数量条件注册').toMatch(/examplesCount[\s\S]{0,80}?>\s*0/)
    expect(PANEL, '页签标签走 i18n').toContain("t('response.tabExamples')")
  })

  it('响应面板提供 #examples 插槽与对应内容容器（数据不进本组件）', () => {
    expect(PANEL, '需渲染 examples 插槽').toMatch(/<slot\s+name="examples"\s*\/>/)
    expect(PANEL, '插槽须挂在 examples 页签分支上').toMatch(
      /activeTab\s*===\s*'examples'[\s\S]{0,120}?<slot\s+name="examples"/,
    )
  })

  it('响应面板的 cookies 分支已收窄为 v-else-if（否则 examples 落进兜底分支）', () => {
    expect(PANEL, 'cookies 必须显式匹配，不能再用兜底 v-else').toMatch(
      /v-else-if="activeTab\s*===\s*'cookies'"/,
    )
  })

  it('EndpointEditor 不再把示例作为独立卡片挂在工作区底部', () => {
    // 旧形态：模板里存在 v-if="activeExamples.length" 的顶层 .examples
    expect(EDITOR, '不得再有顶层 examples 卡片').not.toMatch(
      /<div\s+v-if="activeExamples\.length"\s+class="examples"/,
    )
    // 新形态：经插槽注入响应面板
    expect(EDITOR, '须经 #examples 插槽注入').toMatch(/<template\s+#examples>/)
    expect(EDITOR, '须把数量传给响应面板').toMatch(/:examples-count="activeExamples\.length"/)
  })

  it('数据仍归 EndpointEditor 持有（移的是位置，不是数据源）', () => {
    expect(EDITOR, 'activeExamples 仍由本组件提供').toContain('const activeExamples = computed')
    expect(EDITOR, '示例操作函数仍在').toMatch(/function removeExample|const removeExample/)
  })
})
