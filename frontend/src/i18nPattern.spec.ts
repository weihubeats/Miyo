/**
 * i18n 写法回归测试：src 业务代码禁止用户可见的硬编码文案。
 *
 * 语言切换经 stores/locale.ts 的 t()/tFallback() 生效；漏接的硬编码文案
 * 会在另一种语言下原样露出（写死中文 → 英文界面露中文；写死英文 → 中文界面露英文）。
 * 允许的例外：
 * - 字典本体（src/i18n/）；
 * - 注释（代码注释、CSS 块注释、HTML 注释，非用户可见）；
 * - 存库数据值（测试用例分类 '正向/负向/…'、模块默认名 '默认'）；
 * - 语言自称（'简体中文' 在任何语言下都显示原文）；
 * - 正则里用于匹配用户输入的中文（environment.ts 环境名归类）。
 *
 * 英文硬编码只扫模板：仅 .vue 的根 <template> 块（含属性），TS 代码里的日志、
 * 错误码、协议常量、解析器分支值不在约束范围。两条启发式：
 * 1) 静态 placeholder 属性（`placeholder="…"` / `placeholder='…'`，含 key-placeholder
 *    等 `*-placeholder`；`:placeholder` / `v-bind:` 绑定不算硬编码）；
 * 2) 纯文本节点（`>Key<`、`>Body<`、`>soon<` 这类列头 / 页签 / 徽标文案）；
 *    先把属性值清空再匹配，避免 `v-if="a > b < c"` 这类比较表达式误伤。
 *
 * 取舍说明：
 * - 白名单只放行「两种语言都显示原文」的专有名词（品牌、协议属性、格式名、
 *   GraphQL 术语、语言名、HTTP 方法 / 状态短语）与示例内容（路径、URL、
 *   JSON / curl / GraphQL 片段、HTML 实体化的 {{变量}}、Rust 模块路径）；
 *   真正要拦的可译 UI 词（Key / Value / Description / Body / soon …）一律不放行；
 * - 含 {{插值}} 的动态文案无法静态判定，一律跳过（它们本就该走 t()）；
 * - 个别文件的遗留英文硬编码（见 TEMPLATE_ALLOWLIST）不在本次修复范围，
 *   用「文件 + 精确候选文本」放行，不开整文件的洞。
 */
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const SRC = join(process.cwd(), 'src')
const CJK = /[\u4e00-\u9fff]/

/** 存库数据值：任何文件中出现都不算文案（展示层经 caseCategoryLabel 等映射翻译）。 */
const DATA_TOKENS = /'?(全部|正向|负向|边界值|安全性|其他)'?/g

/** 每文件的白名单行（正则），用于无法归入数据值语义的特例。 */
const FILE_ALLOWLIST: Record<string, RegExp[]> = {
  'src/utils/environment.ts': [CJK], // 正则匹配用户输入的环境名，非展示文案
  'src/components/SettingsDialog.vue': [/'简体中文'/], // 语言自称
  'src/utils/clipboard.ts': [/console\.error/], // 开发者日志，非 UI 文案
  // 场景脚本生成器：产物是独立可运行的 JS（无法走 t()），中文为生成物文案
  //（与 Rust 侧 export/markdown 生成器同性质）
  'src/utils/scenarioScript.ts': [CJK],
}

/**
 * 英文规则的文件级白名单：只放行精确候选文本（待后续任务修复的遗留硬编码），
 * 不开整文件的洞。key = 相对 src 的路径。
 * 条目清零说明：原两条遗留项已修复——
 * - `src/components/HeadersPanel.vue`：Header / Value 占位改为
 *   `t('headers.keyPh')` 与 KeyValueTable 默认 `t('kv.colValue')`；
 * - `src/components/MockRuleDialog.vue`：Header 按钮与 key / value 占位改为
 *   `t('mockrule.addHeader')` 与 `t('kv.colKey'/'kv.colValue')`。
 * 故 TEMPLATE_ALLOWLIST 保持为空对象（结构保留，供后续真正无法归类的特例）。
 */
const TEMPLATE_ALLOWLIST: Record<string, RegExp[]> = {}

/**
 * 模板文本节点里保留原文的专有名词：品牌 / 产品特性、帧与 Cookie 协议属性、
 * 数据格式名、GraphQL 术语、代码生成的语言名、HTTP 方法与状态短语。
 * 这些在两种语言下都显示原文，走 t() 只会制造等值键，故直接白名单；
 * 可译的 UI 词（Key / Value / Description / Body / Params / soon …）不在此列。
 */
const EN_TERM_ALLOW =
  /^(Miyo|Mock|Ping|Pong|Secure|HttpOnly|SameSite|Path|Domain|Expires|JSON|YAML|TOML|XML|HTML|CSV|Markdown \(\.md\)|GraphQL|Query|Mutation|Subscription|Variables|operationName|cURL|curl|TypeScript|JavaScript( \([^)]*\))?|Java|Python|Go|Shell|C#|GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|TRACE|SSE|WS|WSS|TLS|OK|Created|Accepted|No Content|Bad Request|Unauthorized|Forbidden|Not Found|Conflict|Gone|Too Many Requests|Internal Server Error|Bad Gateway|Service Unavailable)$/

/** 静态 placeholder 属性（:placeholder / v-bind: 绑定由负向后顾排除）。 */
const PLACEHOLDER_ATTR = /(?<![:\w-])[\w-]*placeholder\s*=\s*(?:"([^"]*)"|'([^']*)')/g

/** 属性值（清空后再扫文本节点，避免属性里的比较表达式误伤）。 */
const ATTR_VALUES = /=\s*"[^"]*"/g

/** 纯文本节点：不含标签、花括号（插值 / 绑定）与 HTML 注释。 */
const TEXT_NODE = />\s*([^<>{}]+?)\s*</g

/** placeholder 中的示例内容：路径 / JSON / URL / curl / GraphQL 查询片段。 */
const PLACEHOLDER_EXAMPLE = /^(\/|\{|https?:\/\/|wss?:\/\/|socks5?:\/\/|curl\s|query\s|mutation\s)/

/** 文本节点中的示例内容：路径、各类 URL 与文件名（如 SKILL.md）。 */
const TEXT_EXAMPLE = /^(\/|https?:\/\/|wss?:\/\/|socks5?:\/\/|[A-Za-z0-9_-]+\.[A-Za-z0-9]+$)/

/** 含 2 个以上拉丁字母才算“英文短语”（跳过纯数字 / 符号 / 单字母）。 */
const HAS_ENGLISH = /[A-Za-z]{2,}/

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) {
      if (name === 'node_modules' || name === 'dist' || name === 'i18n' || name === 'testUtils') continue
      sourceFiles(p, out)
    } else if ((name.endsWith('.vue') || name.endsWith('.ts')) && !name.endsWith('.spec.ts')) {
      out.push(p)
    }
  }
  return out
}

/** 取根 <template> 块（顶格，嵌套 <template #x> 有缩进），并去 HTML 注释。 */
function templateBlock(text: string): string | null {
  const start = text.startsWith('<template') ? 0 : text.indexOf('\n<template')
  if (start < 0) return null
  const end = text.lastIndexOf('\n</template>')
  if (end < start) return null
  return text.slice(start, end).replace(/<!--[\s\S]*?-->/g, '')
}

/** 去注释后按行检查残留 CJK（注释里的中文是给开发者看的，不在约束范围）。 */
function visibleCjkLines(text: string): string[] {
  let stripped = text.replace(/\/\*[\s\S]*?\*\//g, '') // 块注释（含 CSS）
  stripped = stripped.replace(/<!--[\s\S]*?-->/g, '') // HTML 注释
  const offenders: string[] = []
  for (const raw of stripped.split('\n')) {
    const line = raw.replace(/(?<!:)\/\/.*$/, '') // 行注释（https:// 不受影响）
    const withoutData = line.replace(DATA_TOKENS, '')
    if (CJK.test(withoutData)) offenders.push(raw.trim())
  }
  return offenders
}

/** 模板内英文硬编码：静态 placeholder 属性 + 纯文本节点（allow 按候选文本过滤）。 */
function visibleEnLines(text: string, allow: RegExp[]): string[] {
  const tpl = templateBlock(text)
  if (!tpl) return []
  const offenders: string[] = []
  const allowed = (value: string) => allow.some((re) => re.test(value))

  for (const m of tpl.matchAll(PLACEHOLDER_ATTR)) {
    const value = (m[1] ?? m[2] ?? '').trim()
    if (!HAS_ENGLISH.test(value)) continue
    if (allowed(value)) continue
    if (value.includes('{{') || value.includes('&#123;')) continue // 模板变量 / 实体化 {{
    if (PLACEHOLDER_EXAMPLE.test(value)) continue // 路径 / URL / 代码示例
    offenders.push(`placeholder=${JSON.stringify(value)}`)
  }

  const bare = tpl.replace(ATTR_VALUES, '=""')
  for (const m of bare.matchAll(TEXT_NODE)) {
    const value = m[1].trim()
    if (!HAS_ENGLISH.test(value)) continue
    if (allowed(value)) continue
    if (EN_TERM_ALLOW.test(value)) continue
    if (TEXT_EXAMPLE.test(value)) continue // 路径 / URL 示例
    if (value.includes('&#123;')) continue // 实体化的 {{变量}}
    if (value.includes('::')) continue // Rust 模块路径（如 miyo-core::curl_parser）
    offenders.push(`text=${JSON.stringify(value)}`)
  }
  return offenders
}

describe('i18n 写法', () => {
  it('用户可见文案不硬编码中文（一律走 t()/tFallback()）', () => {
    const offenders: string[] = []
    for (const file of sourceFiles(SRC)) {
      const rel = file.slice(SRC.length + 1)
      const allow = FILE_ALLOWLIST[`src/${rel}`] ?? []
      const lines = visibleCjkLines(readFileSync(file, 'utf8')).filter(
        (line) => !allow.some((re) => re.test(line)),
      )
      for (const line of lines) offenders.push(`${rel}: ${line}`)
    }
    expect(offenders).toEqual([])
  })

  it('模板内不硬编码英文（placeholder / 文本节点一律走 t()）', () => {
    const offenders: string[] = []
    for (const file of sourceFiles(SRC)) {
      if (!file.endsWith('.vue')) continue
      const rel = file.slice(SRC.length + 1)
      const allow = TEMPLATE_ALLOWLIST[`src/${rel}`] ?? []
      const lines = visibleEnLines(readFileSync(file, 'utf8'), allow)
      for (const line of lines) offenders.push(`${rel}: ${line}`)
    }
    expect(offenders).toEqual([])
  })
})
