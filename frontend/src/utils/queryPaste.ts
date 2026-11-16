/**
 * queryPaste：把「一整条 query string / 表单正文」粘贴成字段行（x-www-form-urlencoded、
 * form-data 两页共用）。
 *
 * 场景：从文档、浏览器地址栏、抓包工具、日志里复制 `?a=1&b=2` 或 `a=1\nb=2`，
 * 粘进字段表——默认会整条落进当前单元格（一行塞满 `a=1&b=2`，还得手工拆）。
 * 这里解析成键值对再合并进表格：同名键就地更新，新键追加。
 *
 * 判定尽量保守：每段都必须带 `=`（纯文本 / 一段 JSON 不碰），解码用
 * decodeURIComponent（`+` 视作空格），解码失败保留原文而不是丢字段。
 */

export interface QueryPair {
  key: string
  value: string
}

/**
 * 解析剪贴板里的 query string / 表单正文；不像键值对时返回 null（放行原生粘贴）。
 *
 * 支持：前导 `?`、`&` 或换行分隔、空段忽略、百分号编码与 `+` 空格。
 */
export function parseQueryPairsPaste(text: string): QueryPair[] | null {
  const src = text.trim().replace(/^\?/, '')
  if (!src) return null
  const pairs: QueryPair[] = []
  for (const seg of src.split(/[&\n\r]+/)) {
    const piece = seg.trim()
    if (!piece) continue
    const eq = piece.indexOf('=')
    if (eq === -1) return null // 有任何一段不是 key=value，就不是 query string
    const key = decodeComponent(piece.slice(0, eq).trim())
    if (!key) continue
    pairs.push({ key, value: decodeComponent(piece.slice(eq + 1).trim()) })
  }
  return pairs.length ? pairs : null
}

/**
 * 把键值对合并进已有字段行：同名键（区分大小写）就地更新值，其余追加。
 * 返回新数组（不改入参），并给出更新 / 新增条数供提示文案使用。
 */
export function mergeQueryPairs<T extends { key?: string; value?: string }>(
  rows: readonly T[],
  pairs: readonly QueryPair[],
  createRow: (pair: QueryPair) => T,
): { rows: T[]; updated: number; added: number } {
  const out = rows.map((r) => ({ ...r }))
  let updated = 0
  let added = 0
  for (const pair of pairs) {
    const hit = out.find((r) => (r.key ?? '') === pair.key)
    if (hit) {
      hit.value = pair.value
      updated += 1
    } else {
      out.push(createRow(pair))
      added += 1
    }
  }
  return { rows: out, updated, added }
}

function decodeComponent(s: string): string {
  if (!s.includes('%') && !s.includes('+')) return s
  try {
    return decodeURIComponent(s.replace(/\+/g, ' '))
  } catch {
    return s
  }
}
