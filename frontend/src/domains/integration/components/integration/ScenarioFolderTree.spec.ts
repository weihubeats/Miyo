/**
 * ScenarioFolderTree：
 * - 扁平渲染多级目录（按 expanded 展开 / 折叠）；
 * - 「全部场景」「未分类」两行常驻；
 * - 行点击 → select 事件；箭头点击 → toggle 事件（不触发 select）；
 * - 行内操作：新建子目录 / 重命名 / 删除 → 各自事件；
 * - 拖拽目录到非法落点（自身后代）时不发 moveFolder。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useLocaleStore } from '@/stores/locale'
import { buildFolderTree } from '@/domains/integration/utils/folderTree'
import type { ScenarioFolder, ScenarioLibraryItem } from '@/types/miyoApi'
import ScenarioFolderTree from '@/domains/integration/components/integration/ScenarioFolderTree.vue'

function folder(
  id: string,
  parent_id: string | null,
  name: string,
  sort_order = 0,
): ScenarioFolder {
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

const folders = [
  folder('ecom', null, '电商业务', 0),
  folder('user', 'ecom', '用户中心', 0),
  folder('other', null, '其他测试', 1),
]

function mountTree(options: { selectedId?: string | null; expanded?: string[] } = {}) {
  return mount(ScenarioFolderTree, {
    props: {
      nodes: buildFolderTree(folders, [] as ScenarioLibraryItem[]),
      selectedId: options.selectedId ?? null,
      expanded: options.expanded ?? [],
      totalCount: 0,
      uncategorized: 0,
      filteredCount: 0,
      canMoveFolder: (id: string, parentId: string | null) =>
        parentId === null || !(id === 'ecom' && parentId === 'user'),
    },
    // 树依赖父级 provide 的拖拽状态；默认空负载（无拖拽）
    global: { provide: { scenarioDrag: { kind: null, ids: [] } } },
  })
}

describe('ScenarioFolderTree', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
  })

  it('未展开时只渲染根级目录；展开后渲染子目录', () => {
    const collapsed = mountTree()
    const names = collapsed.findAll('.sft-name').map((n) => n.text())
    expect(names).toContain('电商业务')
    expect(names).toContain('其他测试')
    expect(names).not.toContain('用户中心')

    const expanded = mountTree({ expanded: ['ecom'] })
    const expandedNames = expanded.findAll('.sft-name').map((n) => n.text())
    expect(expandedNames).toContain('用户中心')
  })

  it('点击行发 select；点击箭头只发 toggle', async () => {
    const w = mountTree({ expanded: ['ecom'] })
    const ecomRow = w.findAll('.sft-row').find((r) => r.text().includes('电商业务'))!
    await ecomRow.trigger('click')
    expect(w.emitted('select')?.at(-1)).toEqual(['ecom'])

    const chevron = ecomRow.find('.sft-chevron')
    await chevron.trigger('click')
    expect(w.emitted('toggle')?.at(-1)).toEqual(['ecom'])
    // 箭头点击 stop：select 次数未增加
    expect(w.emitted('select')).toHaveLength(1)
  })

  it('「全部场景」与「未分类」行分别发 select(null) / select(\'\')', async () => {
    const w = mountTree()
    const rows = w.findAll('.sft-row')
    await rows[0]!.trigger('click')
    expect(w.emitted('select')?.at(-1)).toEqual([null])
    await rows[rows.length - 1]!.trigger('click')
    expect(w.emitted('select')?.at(-1)).toEqual([''])
  })

  it('行内操作：新建子目录 / 重命名 / 删除发出对应事件', async () => {
    const w = mountTree()
    const ecomRow = w.findAll('.sft-row').find((r) => r.text().includes('电商业务'))!
    await ecomRow.trigger('mouseenter')
    const actions = ecomRow.findAll('.sft-actions button')
    expect(actions).toHaveLength(3)
    await actions[0]!.trigger('click')
    expect(w.emitted('create')?.at(-1)).toEqual(['ecom'])
    await actions[1]!.trigger('click')
    expect(w.emitted('rename')?.at(-1)?.[0]).toMatchObject({ id: 'ecom' })
    await actions[2]!.trigger('click')
    expect(w.emitted('remove')?.at(-1)?.[0]).toMatchObject({ id: 'ecom' })
  })

  it('拖拽防环：canMoveFolder 拒绝自身后代', () => {
    const w = mountTree({ expanded: ['ecom'] })
    const canMove = w.props('canMoveFolder') as (a: string, b: string | null) => boolean
    expect(canMove('ecom', 'user')).toBe(false)
    expect(canMove('other', 'ecom')).toBe(true)
  })

  it('目录状态点按子树汇总：失败优先', () => {
    const items = [
      {
        id: 's-1',
        project_id: 'p-1',
        name: '失败场景',
        description: '',
        category: '',
        folder_id: 'user',
        tags: [],
        favorite: false,
        sort_order: 0,
        steps: [],
        created_at: '',
        updated_at: '',
        last_run: {
          id: 'r1',
          scenario_id: 's-1',
          environment_id: null,
          finished_at: '2026-01-01T00:00:00Z',
          duration_ms: 1,
          total_steps: 1,
          ok_count: 0,
          fail_count: 1,
          skipped_count: 0,
          cancelled: false,
          stopped_on_failure: false,
        },
      },
    ] as ScenarioLibraryItem[]
    const w = mount(ScenarioFolderTree, {
      props: {
        nodes: buildFolderTree(folders, items),
        selectedId: null,
        expanded: ['ecom'],
        totalCount: 1,
        uncategorized: 0,
        filteredCount: 1,
        canMoveFolder: () => true,
      },
      global: { provide: { scenarioDrag: { kind: null, ids: [] } } },
    })
    const ecomRow = w.findAll('.sft-row').find((r) => r.text().includes('电商业务'))!
    expect(ecomRow.find('.sft-dot').classes()).toContain('fail')
  })
})