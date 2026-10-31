/**
 * fuzzyMatch：轻量子序列模糊匹配（无第三方依赖）。
 *
 * 评分口径（越大越相关，null = 不匹配）：
 * - 子序列命中为基础分；连续命中段每字符额外加分（连续前缀最优先）；
 * - 命中词首（分隔符 . / - / _ / 空格 / 驼峰边界之后）加分；
 * - 大小写不敏感；查询含空格时按词切分，全部词命中才算匹配（多词过滤）。
 *
 * 供命令面板（QuickSearch）做接口 / 场景 / 环境检索；目录树内搜索维持
 * 现有子串语义不动（检索面不同：树内是「过滤列表」，面板是「全局找东西」）。
 */

/** 词首判定：首字符，或前面是分隔符 / 小写→大写驼峰边界。 */
function isWordStart(text: string, index: number): boolean {
  if (index === 0) return true
  const prev = text[index - 1]!
  const cur = text[index]!
  if (/[.\-_\s/]/.test(prev)) return true
  return /[a-z]/.test(prev) && /[A-Z]/.test(cur)
}

/** 单词在文本中的模糊得分；null = 该词未命中。 */
function scoreWord(textLower: string, wordLower: string): number | null {
  if (!wordLower) return 0
  if (textLower.includes(wordLower)) {
    // 完整子串：前缀命中最高，其余按位置浅评。
    if (textLower.startsWith(wordLower)) return 100 + wordLower.length
    return 60 + wordLower.length
  }
  let score = 0
  let ti = 0
  let streak = 0
  for (let wi = 0; wi < wordLower.length; wi++) {
    const ch = wordLower[wi]!
    const found = textLower.indexOf(ch, ti)
    if (found === -1) return null
    if (found === ti && wi > 0) {
      streak += 1
      score += 6 + streak
    } else {
      streak = 0
      score += 2
    }
    if (isWordStart(textLower, found)) score += 5
    ti = found + 1
  }
  return score
}

/**
 * 模糊匹配。`query` 按空格切词（全部词须命中，得分取各词和）；
 * 空查询返回 0（调用方自行处理「无查询 = 列全部/最近」）。
 */
export function fuzzyScore(text: string | null | undefined, query: string): number | null {
  if (!text) return null
  const q = query.trim().toLowerCase()
  if (!q) return 0
  const t = text.toLowerCase()
  let total = 0
  for (const word of q.split(/\s+/)) {
    const s = scoreWord(t, word)
    if (s == null) return null
    total += s
  }
  return total
}
