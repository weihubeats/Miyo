/**
 * 请求卡高度契约回归测试（源码级，jsdom 无布局引擎）。
 *
 * 锁住三处经真实链路 headless 实测才暴露的布局常量，防止再次被「顺手清理」：
 * 1. `.config-box.grow` 必须带 min-height 保底：无响应期请求卡 `flex:1` 吃剩余空间，
 *    窗口偏矮时会被一路压扁到 Body 编辑区只剩一条线（实测 vp 400~700 时
 *    编辑区 0~62px，用户实测「打开 POST 接口 Body 被压缩」）；
 * 2. `.editor` 必须纵向可滚：保底高度超出可视区时，靠它把内容滚出来而不是裁掉；
 * 3. BodyPanel 不再带 `fill` 开关（编辑器恒填满面板）：`fill=false` 会回到
 *    「贴内容生长」，与请求卡 grow 叠加后卡片中部留出大片空白
 *    （实测分割条下拉 +120px → 空白 118px，用户实测「中间很大的空白」）。
 *    行为层断言见 BodyPanel.spec.ts 的 hl-wrap 不带 auto-height。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const root = process.cwd()

function read(rel: string): string {
  return readFileSync(join(root, rel), 'utf8')
}

/** 抽出一条 CSS 规则块（含选择器），先剥注释避免注释里的同名片段误命中。 */
function cssBlock(css: string, selector: string): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '')
  const start = stripped.indexOf(`${selector} {`)
  if (start === -1) return ''
  const end = stripped.indexOf('}', start)
  return end === -1 ? stripped.slice(start) : stripped.slice(start, end + 1)
}

/** 抽取 `<BodyPanel … />` 之外的源码（模板断言用）。 */
const endpointEditor = read('src/domains/endpoint/components/EndpointEditor.vue')
const bodyPanel = read('src/domains/endpoint/components/BodyPanel.vue')

describe('请求卡高度契约', () => {
  it('.config-box.grow 带 min-height 保底（矮窗口不压扁 Body 编辑区）', () => {
    const block = cssBlock(endpointEditor, '.config-box.grow')
    expect(block).not.toBe('')
    const m = block.match(/min-height:\s*(\d+)px/)
    expect(m, '.config-box.grow 缺少 min-height 保底').not.toBeNull()
    // 整卡需容下页签行 + 模式条 + 编辑器（160 地板）+ 设置条，低于 240 无意义
    expect(Number(m![1])).toBeGreaterThanOrEqual(240)
  })

  it('.editor 纵向可滚（保底高度溢出可视区时滚出来而非裁掉）', () => {
    const block = cssBlock(endpointEditor, '.editor')
    expect(block).not.toBe('')
    expect(block).toMatch(/overflow-y:\s*auto/)
  })

  it('BodyPanel 无 fill 开关（编辑器恒填满面板，不回到贴内容生长）', () => {
    expect(bodyPanel).not.toMatch(/\bfill\??:/)
    expect(bodyPanel).not.toContain(':auto-height="!fill"')
    expect(endpointEditor).not.toContain(':fill=')
  })
})
