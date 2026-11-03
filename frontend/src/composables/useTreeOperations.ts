/**
 * useTreeOperations：目录树行内编辑 + CRUD 包装（自 EndpointTree 抽出）。
 *
 * 职责（原 EndpointTree startEdit/cancelEdit/commitEdit/removeFolder/
 * removeEndpoint/duplicate 六个函数，行为不变）：
 * - 行内编辑三态（新建子文件夹 / 重命名文件夹 / 重命名接口）；
 * - 删除（文件夹 / 接口，删除后清多选集）与接口复制；
 * - 全部走 workspace store，失败 toast（console 带来源标记）。
 *
 * 两份递归共享状态由宿主传入（EndpointTree 是递归组件）：
 * - 行内编辑态 editing/editValue：经模块级 TREE_EDIT_STATE 注入共享
 *   （每层子树读写同一份）——注入键一并移到本模块导出（<script setup>
 *   体内的 Symbol 会随实例重建，provide/inject 永远匹配不上）；
 * - 多选集 treeSelection：模块级 reactive，删除后清对应项。
 */
import type { InjectionKey, Ref } from 'vue'
import type { useWorkspaceStore } from '../stores/workspace'
import type { useToast } from '../composables/useToast'
import type { Endpoint, Folder } from '../types/miyoApi'

/** 递归树行内编辑状态（注入共享，见文件头注释）。 */
export interface TreeEditState {
  kind: 'create-folder' | 'rename-folder' | 'rename-endpoint'
  id?: string
  parentId?: string | null
}

/** 模块级注入键：全部递归实例共享同一份编辑态。 */
export const TREE_EDIT_STATE: InjectionKey<{
  editing: Ref<TreeEditState | null>
  editValue: Ref<string>
}> = Symbol('endpoint-tree-edit')

/** 多选集的最小形状（与 EndpointTree 模块级 treeSelection 同构）。 */
export interface TreeSelectionSets {
  endpoints: Set<string>
  folders: Set<string>
}

export interface UseTreeOperationsOptions {
  store: ReturnType<typeof useWorkspaceStore>
  toast: ReturnType<typeof useToast>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
  /** 行内编辑态（递归共享，宿主注入/提供）。 */
  editing: Ref<TreeEditState | null>
  editValue: Ref<string>
  /** 展开集（Ref 包裹，递归实例共享）：新建子文件夹后展开目标。 */
  expanded: Ref<Set<string>>
  /** 多选集（模块级共享；删除后清对应项）。 */
  treeSelection: TreeSelectionSets
}

export function useTreeOperations(options: UseTreeOperationsOptions) {
  const { store, toast, t, editing, editValue, expanded, treeSelection } = options

  function startEdit(
    kind: 'create-folder' | 'rename-folder' | 'rename-endpoint',
    opts?: { id?: string; parentId?: string | null },
  ): void {
    editing.value = { kind, ...opts }
    editValue.value = ''
    if (kind === 'rename-folder' && opts?.id) {
      editValue.value = store.folders.find((f) => f.id === opts.id)?.name ?? ''
    }
    if (kind === 'rename-endpoint' && opts?.id) {
      editValue.value = store.endpoints.find((e) => e.id === opts.id)?.name ?? ''
    }
  }

  function cancelEdit(): void {
    editing.value = null
    editValue.value = ''
  }

  async function commitEdit(): Promise<void> {
    const ed = editing.value
    if (!ed) return
    const name = editValue.value.trim()
    if (!name) {
      cancelEdit()
      return
    }
    const now = new Date().toISOString()
    try {
      if (ed.kind === 'create-folder') {
        const folder: Folder = {
          id: crypto.randomUUID(),
          project_id: store.project!.id,
          parent_id: ed.parentId ?? null,
          name,
          sort_order: 0,
          created_at: now,
          updated_at: now,
        }
        await store.saveFolder(folder)
        if (ed.parentId) expanded.value.add(ed.parentId)
      } else if (ed.kind === 'rename-folder' && ed.id) {
        const f = store.folders.find((x) => x.id === ed.id)
        if (f) await store.saveFolder({ ...f, name, updated_at: now })
      } else if (ed.kind === 'rename-endpoint' && ed.id) {
        await store.renameEndpoint(ed.id, name)
      }
    } catch (err) {
      console.error('[EndpointTree.commitEdit]', err)
      toast.error(
        ed.kind === 'create-folder' ? t('tree.createFolderFail') : t('tree.renameFail'),
        { message: err instanceof Error ? err.message : String(err) },
      )
    } finally {
      cancelEdit()
    }
  }

  async function removeFolder(id: string): Promise<void> {
    try {
      await store.deleteFolder(id)
    } catch (err) {
      console.error('[EndpointTree.removeFolder]', err)
      toast.error(t('tree.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
    } finally {
      treeSelection.folders.delete(id)
    }
  }

  async function removeEndpoint(e: Endpoint): Promise<void> {
    try {
      await store.deleteEndpoint(e.id)
    } catch (err) {
      console.error('[EndpointTree.removeEndpoint]', err)
      toast.error(t('tree.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
    } finally {
      treeSelection.endpoints.delete(e.id)
    }
  }

  async function duplicate(e: Endpoint): Promise<void> {
    try {
      await store.duplicateEndpoint(e.id)
    } catch (err) {
      console.error('[EndpointTree.duplicate]', err)
      toast.error(t('tree.duplicateFail'), { message: err instanceof Error ? err.message : String(err) })
    }
  }

  return { startEdit, cancelEdit, commitEdit, removeFolder, removeEndpoint, duplicate }
}
