/**
 * watch 源写法回归测试：禁止「单个 getter 返回数组」作为 watch 源。
 *
 * 反例：`watch(() => [a, b], ([x, y]) => …)`
 *   getter 每次求值都新建一个数组，而 Vue 的 `hasChanged` 对数组按**引用**比较 →
 *   效果重跑时（getter 里读到的任何依赖变化，比如 computed/对象身份被替换）
 *   回调**无条件重跑**，哪怕 a、b 的值一个没变。
 * 正例：`watch([() => a, () => b], ([x, y]) => …)`
 *   多源写法逐元素比较，只有 a / b 真变化才回调。
 *
 * 实证（v0.0.62，用户报「导入 curl 后保存了，左边还有未保存的小点」）：
 * EndpointEditor 的 Method watcher 用了反例写法——保存成功时 store 把草稿换成
 * 后端回执（对象身份变化、内容一字未变），getter 里的 `draft.value` 变了 →
 * 回调重跑 → 只挡了「id 变化」→ 同 id 落到 applyMethodDefaults，把 active_tab
 * 写回刚保存的草稿（POST 空体还会被塞进 JSON body + Content-Type）→ 保存后
 * 立刻又判「有改动」。同类写法当时还有 15 处，均已收口。
 *
 * 新增例外先在本 spec 白名单登记并写清理由（例如确实需要「同一表达式重跑」）。
 */
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = join(process.cwd(), 'src') + '/'

function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) {
      if (name === 'node_modules' || name === 'dist') continue
      out.push(...sourceFiles(p))
    } else if ((name.endsWith('.vue') || name.endsWith('.ts')) && !name.endsWith('.spec.ts')) {
      out.push(p)
    }
  }
  return out
}

/** `watch(() => [ … ])`：单个 getter 直接返回数组字面量（跨行写法同样命中）。 */
const BAD_SOURCE = /watch\s*(?:<[^>()]*>)?\s*\(\s*\n?\s*(?:async\s*)?\(\s*\)\s*=>\s*\[/g

/** 白名单：`<相对路径>: <理由>`（当前为空——全仓已收口）。 */
const ALLOWED = new Set<string>([])

describe('watch 源写法', () => {
  it('不得用「单 getter 返回数组」作 watch 源（对象身份变化会误触发回调）', () => {
    const offenders: string[] = []
    for (const file of sourceFiles(ROOT)) {
      const rel = file.slice(ROOT.length)
      const text = readFileSync(file, 'utf8')
      for (const m of text.matchAll(BAD_SOURCE)) {
        if (ALLOWED.has(rel)) continue
        const line = text.slice(0, m.index).split('\n').length
        offenders.push(`${rel}:${line}`)
      }
    }
    expect(
      offenders,
      `改用多源写法：watch([() => a, () => b], ([x, y]) => …)\n${offenders.join('\n')}`,
    ).toEqual([])
  })
})
