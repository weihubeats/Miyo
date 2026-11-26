/**
 * folderTree 纯函数单测：多级建树 / 子树 / 路径 / 防环 / 状态汇总 / 孤儿容错。
 */
import { describe, expect, it } from 'vitest'
import {
  buildFolderTree,
  canMoveFolder,
  collectSubtreeIds,
  folderPath,
  lastRunResult,
  uncategorizedCount,
} from '@/domains/integration/utils/folderTree'
import type { ScenarioFolder, ScenarioLibraryItem } from '@/types/miyoApi'

function folder(id: string, parent_id: string | null, name: string, sort_order = 0): ScenarioFolder {
  return {
    id,
    project_id: 'p-1',
    parent_id,
    name,
    sort_order,
    created_at: '',
    updated_at: '',
  }
}

function item(id: string, folder_id: string | null, fail = false, ran = true): ScenarioLibraryItem {
  return {
    id,
    project_id: 'p-1',
    name: id,
    description: '',
    category: '',
    folder_id,
    tags: [],
    favorite: false,
    sort_order: 0,
    steps: [],
    created_at: '',
    updated_at: '',
    last_run: ran
      ? {
          id: `run-${id}`,
          scenario_id: id,
          environment_id: null,
          finished_at: '2026-01-01T00:00:00Z',
          duration_ms: 10,
          total_steps: 1,
          ok_count: fail ? 0 : 1,
          fail_count: fail ? 1 : 0,
          skipped_count: 0,
          cancelled: false,
          stopped_on_failure: false,
        }
      : null,
  }
}

describe('folderTree', () => {
  const folders = [
    folder('ecom', null, '电商业务', 0),
    folder('user', 'ecom', '用户中心', 0),
    folder('order', 'ecom', '订单中心', 1),
    folder('other', null, '其他测试', 1),
  ]

  it('collectSubtreeIds 含自身与全部后代', () => {
    expect([...collectSubtreeIds(folders, 'ecom')].sort()).toEqual(['ecom', 'order', 'user'])
    expect([...collectSubtreeIds(folders, 'user')]).toEqual(['user'])
  })

  it('folderPath 根 → 叶', () => {
    expect(folderPath(folders, 'user')).toBe('电商业务 / 用户中心')
    expect(folderPath(folders, null)).toBe('')
    expect(folderPath(folders, 'missing')).toBe('')
  })

  it('canMoveFolder 阻止移到自身 / 后代', () => {
    expect(canMoveFolder(folders, 'ecom', null)).toBe(true)
    expect(canMoveFolder(folders, 'ecom', 'ecom')).toBe(false)
    expect(canMoveFolder(folders, 'ecom', 'user')).toBe(false)
    expect(canMoveFolder(folders, 'user', 'other')).toBe(true)
  })

  it('buildFolderTree 汇总场景数与执行状态', () => {
    const items = [
      item('a', 'user'),
      item('b', 'order', true),
      item('c', 'ecom'),
      item('d', null),
    ]
    const tree = buildFolderTree(folders, items)
    expect(tree.map((n) => n.folder.id)).toEqual(['ecom', 'other'])
    const ecom = tree[0]!
    expect(ecom.scenarioCount).toBe(3)
    expect(ecom.status).toBe('fail') // 后代有失败
    const user = ecom.children.find((c) => c.folder.id === 'user')!
    expect(user.scenarioCount).toBe(1)
    expect(user.status).toBe('ok')
    expect(tree[1]!.status).toBe('none')
    expect(uncategorizedCount(folders, items)).toBe(1)
  })

  it('孤儿目录提升为根，避免不可见', () => {
    const orphanTree = buildFolderTree([folder('x', 'missing-parent', '孤儿')], [])
    expect(orphanTree).toHaveLength(1)
    expect(orphanTree[0]!.folder.id).toBe('x')
  })

  it('lastRunResult 区分取消 / 失败 / 通过', () => {
    expect(lastRunResult(item('a', null, true))).toBe('fail')
    expect(lastRunResult(item('b', null, false))).toBe('ok')
    expect(lastRunResult(item('c', null, false, false))).toBe(null)
  })
})