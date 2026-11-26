/**
 * 场景业务目录树的纯函数：建树、子树收集、路径、防环校验、状态汇总。
 *
 * 与组件解耦（ScenarioLibrary 只消费结果），便于单测覆盖多级目录的边界
 * （孤儿节点、循环父子、删除策略预览）。
 */
import type { ScenarioFolder, ScenarioLibraryItem } from '@/types/miyoApi'

export type FolderStatus = 'ok' | 'fail' | 'running' | 'mixed' | 'none'

export interface FolderTreeNode {
  folder: ScenarioFolder
  children: FolderTreeNode[]
  /** 子树内场景总数（含自身目录 + 后代目录）。 */
  scenarioCount: number
  /** 子树执行状态汇总：任一失败 → fail；任一通过 → ok；混合 → mixed。 */
  status: FolderStatus
}

/** 最近一次执行判定（与列表状态点同口径）。 */
export function lastRunResult(
  item: ScenarioLibraryItem,
): 'ok' | 'fail' | 'cancel' | null {
  const r = item.last_run
  if (!r) return null
  if (r.cancelled) return 'cancel'
  return r.fail_count > 0 ? 'fail' : 'ok'
}

/** 目录 id → 子树 id 集合（含自身）。孤儿 / 环不丢节点（以访问集防死循环）。 */
export function collectSubtreeIds(
  folders: ScenarioFolder[],
  rootId: string,
): Set<string> {
  const childrenByParent = new Map<string | null, ScenarioFolder[]>()
  for (const f of folders) {
    const key = f.parent_id ?? null
    const list = childrenByParent.get(key) ?? []
    list.push(f)
    childrenByParent.set(key, list)
  }
  const seen = new Set<string>()
  const stack = [rootId]
  while (stack.length) {
    const id = stack.pop()!
    if (seen.has(id)) continue
    seen.add(id)
    for (const child of childrenByParent.get(id) ?? []) stack.push(child.id)
  }
  return seen
}

/** 目录路径（根 → 叶，用 `/` 连接）；找不到 / 孤儿时返回空串。 */
export function folderPath(folders: ScenarioFolder[], folderId: string | null): string {
  if (!folderId) return ''
  const byId = new Map(folders.map((f) => [f.id, f]))
  const parts: string[] = []
  const seen = new Set<string>()
  let current = byId.get(folderId)
  while (current && !seen.has(current.id)) {
    seen.add(current.id)
    parts.unshift(current.name)
    current = current.parent_id ? byId.get(current.parent_id) : undefined
  }
  return parts.join(' / ')
}

/** 把目录 id 移动到 parentId 是否合法（不能移到自己 / 自己的后代）。 */
export function canMoveFolder(
  folders: ScenarioFolder[],
  folderId: string,
  parentId: string | null,
): boolean {
  if (parentId === null) return true
  if (parentId === folderId) return false
  return !collectSubtreeIds(folders, folderId).has(parentId)
}

/**
 * 建树：返回根节点数组（同级按 sort_order → 名称序）。孤儿节点（父级缺失）
 * 提升为根，避免数据异常导致目录不可见。
 */
export function buildFolderTree(
  folders: ScenarioFolder[],
  items: ScenarioLibraryItem[],
): FolderTreeNode[] {
  const byParent = new Map<string | null, ScenarioFolder[]>()
  const ids = new Set(folders.map((f) => f.id))
  for (const f of folders) {
    // 父级缺失 → 提升为根（容错孤儿数据）。
    const key = f.parent_id && ids.has(f.parent_id) ? f.parent_id : null
    const list = byParent.get(key) ?? []
    list.push(f)
    byParent.set(key, list)
  }
  const itemsByFolder = new Map<string, ScenarioLibraryItem[]>()
  const uncategorized: ScenarioLibraryItem[] = []
  for (const item of items) {
    const fid = item.folder_id ?? null
    if (!fid || !ids.has(fid)) {
      uncategorized.push(item)
      continue
    }
    const list = itemsByFolder.get(fid) ?? []
    list.push(item)
    itemsByFolder.set(fid, list)
  }

  const build = (folder: ScenarioFolder, guard: Set<string>): FolderTreeNode => {
    guard.add(folder.id)
    const own = itemsByFolder.get(folder.id) ?? []
    let count = own.length
    let hasOk = own.some((i) => lastRunResult(i) === 'ok')
    let hasFail = own.some((i) => lastRunResult(i) === 'fail')
    const children: FolderTreeNode[] = []
    for (const child of byParent.get(folder.id) ?? []) {
      if (guard.has(child.id)) continue // 环保护
      const node = build(child, guard)
      children.push(node)
      count += node.scenarioCount
      if (node.status === 'fail') hasFail = true
      else if (node.status === 'ok' || node.status === 'mixed') hasOk = true
    }
    children.sort(
      (a, b) =>
        a.folder.sort_order - b.folder.sort_order ||
        a.folder.name.localeCompare(b.folder.name),
    )
    return {
      folder,
      children,
      scenarioCount: count,
      status: hasFail ? 'fail' : hasOk ? 'ok' : 'none',
    }
  }

  const roots = (byParent.get(null) ?? [])
    .slice()
    .sort(
      (a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name),
    )
    .map((f) => build(f, new Set()))
  return roots
}

/** 未分类场景数（folder_id 为空 / 指向已删目录）。 */
export function uncategorizedCount(
  folders: ScenarioFolder[],
  items: ScenarioLibraryItem[],
): number {
  const ids = new Set(folders.map((f) => f.id))
  return items.filter((i) => !i.folder_id || !ids.has(i.folder_id)).length
}

/** 扁平化目录树（只含展开的分支），供树列表渲染（避免递归组件）。 */
export interface FlatFolderRow {
  node: FolderTreeNode
  depth: number
}

export function flattenFolderTree(
  nodes: FolderTreeNode[],
  expanded: string[],
): FlatFolderRow[] {
  const open = new Set(expanded)
  const rows: FlatFolderRow[] = []
  const walk = (list: FolderTreeNode[], depth: number): void => {
    for (const node of list) {
      rows.push({ node, depth })
      if (node.children.length && open.has(node.folder.id)) {
        walk(node.children, depth + 1)
      }
    }
  }
  walk(nodes, 0)
  return rows
}