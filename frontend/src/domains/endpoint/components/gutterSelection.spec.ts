/**
 * 行号选区契约：复制响应时应与请求编辑器一致——行号不参与选区视觉，也不进剪贴板。
 *
 * 背景：响应侧的 `.rp-line-gutter` / `.jt-gutter` 与正文同处一个选择流
 *（span 兄弟节点），不同于 CodeMirror 把 gutter 放在独立 DOM 层
 *（`.cm-gutters`）。因此仅靠 `user-select: none` 只能挡住「从行号起选」，
 * 从正文往左拖 / 三击 / 全选时浏览器仍会把跨过的行号一起涂蓝
 *（用户实测：复制响应时行号 1..6 被选中）。
 *
 * jsdom 不做样式计算，故按源码断言（与 themePattern.spec.ts 同法）。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const read = (p: string) => readFileSync(join(process.cwd(), p), 'utf8')

/** 取某个 CSS 选择器的规则体（花括号配平），必须在行首匹配。 */
function ruleBody(src: string, selector: string): string {
  const re = new RegExp(`^${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')} \\{`, 'm')
  const m = re.exec(src)
  expect(m, `未找到规则 ${selector}`).not.toBeNull()
  const open = src.indexOf('{', m!.index)
  let depth = 0
  for (let i = open; i < src.length; i += 1) {
    if (src[i] === '{') depth += 1
    else if (src[i] === '}') {
      depth -= 1
      if (depth === 0) return src.slice(open + 1, i)
    }
  }
  throw new Error(`规则 ${selector} 花括号未配平`)
}

const CASES = [
  { file: 'src/domains/endpoint/components/ResponsePanel.vue', cls: 'rp-line-gutter' },
  { file: 'src/components/ui/JsonTree.vue', cls: 'jt-gutter' },
] as const

describe('行号 gutter：选区与复制都不卷入行号', () => {
  for (const { file, cls } of CASES) {
    it(`${cls} 保留 user-select:none（挡住从行号起选）`, () => {
      const body = ruleBody(read(file), `.${cls}`)
      expect(body, '行号必须禁止选中').toContain('user-select: none')
    })

    it(`${cls} 的 ::selection 背景透明（被扫过也不涂蓝）`, () => {
      const src = read(file)
      // 必须存在该伪元素规则
      const re = new RegExp(`^\\.${cls}::selection \\{`, 'm')
      expect(re.test(src), `${cls}::selection 规则缺失 → 从正文往左拖会连同行号一起涂蓝`).toBe(true)

      const body = ruleBody(src, `.${cls}::selection`)
      expect(
        body,
        '::selection 背景必须透明，否则行号被跨选时会显示高亮',
      ).toMatch(/background(-color)?:\s*transparent/)
      // 高亮消失后仍要保住行号原色，避免被主题的 selection-fg 改成正文色
      expect(body, '行号颜色须保持自身 token').toMatch(/color:\s*var\(--tok-gutter\)/)
    })
  }
})
