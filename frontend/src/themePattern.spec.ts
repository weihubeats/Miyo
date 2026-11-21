/**
 * 主题写法回归测试：组件内禁止任何 `html[data-theme]` 选择器与写死的深底/亮紫字面量。
 *
 * - `:global(html[data-theme=...])` 形式经 Vite 构建后丢失（产物中无对应规则），
 *   导致浅色主题静默失效（侧边栏 / 右键菜单曾全黑）；
 * - 裸 `html[data-theme=...]` 形式虽不丢，但属组件级主题覆写，割裂
 *   「主题差异一律走 style.css CSS 变量（深/浅各一套）」的单源约束。
 * 两种形式一律收敛为 style.css 中的成对 token，组件只消费 var()。
 *
 * 另拦截三类实证过的浅色缺陷（ReleaseNotesModal / DeleteConfirmModal /
 * ExportDocsDialog / CodeSnippetsPanel 踩过）：
 * - `--text-N: #hex`：组件级恒深底 token 直写（浅色下黑底弹窗）；
 * - `background: #0a0a0a`：写死深代码井（浅色下黑洞块）；
 * - `bg-purple-600` / `hover:bg-purple-500` / `shadow-purple-950`：
 *   硬编码亮紫填充与深紫脏影，一律走 --primary / --grad-primary 令牌
 *   （浅色降刺眼）。新特例先改本 spec 白名单再写代码。
 */
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

function vueFiles(dir: string): string[] {
  const out: string[] = []
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) {
      if (name === 'node_modules' || name === 'dist') continue
      out.push(...vueFiles(p))
    } else if (name.endsWith('.vue')) {
      out.push(p)
    }
  }
  return out
}

/** 深底/亮紫字面量黑名单（注释不算——只查非注释行）。 */
const BANNED = [
  /--text-[123]: ?#/, // 组件级恒深底 token 直写
  /--bg-elevated: ?(rgba|#)/, // 恒深底容器色直写（映射成对 token 不拦）
  /background: ?#0a0a0a/, // 写死深代码井
  /bg-purple-600(?![\d/])/, // 硬编码亮紫填充（走 --primary；淡色 tint（/20 等）放行）
  /hover:bg-purple-500(?![\d/])/, // 硬编码亮紫 hover（同上，tint 放行）
  /shadow-purple-950/, // 深紫脏影（走 --primary-glow）
  /rgba\(168, ?85, ?247/, // 硬编码旧 accent 紫（走 --accent / --accent-tint / --focus-ring）
  /rgba\(9, ?12, ?22/, // 硬编码阴影色（走 --shadow-* 主题自适应阴影）
  /color: ?#fff\b/, // 白色文字字面量（彩色底走 --text-on-accent；WindowControls 平台例外在 spec 底部白名单）
  /background: ?#fff\b/, // 白色填充（开关旋钮等彩色底元素走 --text-on-accent）
]

describe('主题写法', () => {
  it('不存在组件级 html[data-theme] 覆写（:global 构建丢弃；裸选择器亦禁）', () => {
    const offenders: string[] = []
    for (const file of vueFiles(join(process.cwd(), 'src'))) {
      const text = readFileSync(file, 'utf8')
      if (text.includes(':global(html[') || text.includes('html[data-theme')) offenders.push(file)
    }
    expect(offenders).toEqual([])
  })

  it('不存在写死的恒深底 token / 深代码井 / 亮紫填充字面量（浅色突兀实证）', () => {
    const offenders: { file: string; pattern: string }[] = []
    /** 平台语义例外：Windows 原生关闭钮红色 + 白字（非主题范畴）。 */
    const WHITELIST_FILES = ['WindowControls.vue']
    for (const file of vueFiles(join(process.cwd(), 'src'))) {
      if (WHITELIST_FILES.some((w) => file.endsWith(w))) continue
      // 先全文剥离注释（块注释可跨行、HTML 注释同理），再逐行扫黑名单
      const text = readFileSync(file, 'utf8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/<!--[\s\S]*?-->/g, '')
      for (const line of text.split('\n')) {
        const code = line.replace(/\/\/.*$/, '') // 行注释（URL 内 // 误剥不影响判定）
        for (const p of BANNED) {
          if (p.test(code)) offenders.push({ file, pattern: String(p) })
        }
      }
    }
    expect(offenders).toEqual([])
  })
})
