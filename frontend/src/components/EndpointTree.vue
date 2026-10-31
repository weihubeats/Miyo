<script lang="ts">
/**
 * 共享拖拽状态（模块级 reactive，递归实例间可见）。
 * 采用指针事件实现（pointerdown/move/up + elementFromPoint 命中测试），
 * 规避 WKWebView 对 HTML5 Drag & Drop（dragover/drop/dataTransfer）支持不稳的问题。
 */
import { reactive, type InjectionKey, type Ref } from 'vue'

export interface DndTarget {
  drop: 'folder' | 'before' | 'after' | 'root'
  id: string | null
  index: number
}

export const dndState = reactive({
  active: false,
  kind: '' as 'folder' | 'endpoint' | '',
  id: '',
  target: null as DndTarget | null,
})

/**
 * 多选状态（模块级 reactive，递归实例间共享）：
 * Ctrl/⌘ 点击多选，Shift 连选同层，批量删除 / 批量移动。
 */
export const treeSelection = reactive({
  endpoints: new Set<string>(),
  folders: new Set<string>(),
})

export function clearTreeSelection(): void {
  treeSelection.endpoints.clear()
  treeSelection.folders.clear()
}

/**
 * 键盘焦点行 id（模块级共享，跨递归实例唯一真源）。
 * roving tabindex：只有焦点行 tabindex=0，其余 -1；↑↓/Home/End 沿可见行移动。
 * 根实例负责初始化与失效兜底（行被删除 / 切换项目时回落到首行）。
 */
export const treeFocus = reactive({ id: null as string | null })

/** 拖拽结束后抑制紧随其后的 click（避免误打开接口/文件夹）。 */
let suppressClick = false

export function consumeSuppressClick(): boolean {
  const s = suppressClick
  suppressClick = false
  return s
}

export function setSuppressClick(v: boolean): void {
  suppressClick = v
}

/**
 * 递归树行内编辑状态的注入键 —— 必须在模块作用域声明。
 * <script setup> 体内的常量会随每个实例重新创建：每层子树各持有一个
 * 不相等的 Symbol，provide/inject 永远匹配不上（此前的缺陷）。
 */
interface TreeEditState {
  kind: 'create-folder' | 'rename-folder' | 'rename-endpoint'
  id?: string
  parentId?: string | null
}

const TREE_EDIT_STATE: InjectionKey<{
  editing: Ref<TreeEditState | null>
  editValue: Ref<string>
}> = Symbol('endpoint-tree-edit')

/**
 * 共享子索引的类型（实例见 <script setup>）。
 * parentId / folderId → 直属子项：每层子树原来各自
 * `store.folders.filter + store.endpoints.filter` 全表扫描，
 * O(层数 × (F+E))；索引后每层 O(1) 取直属子项。
 */
export interface TreeChildIndex {
  foldersByParent: Map<string | null, import('../types/miyoApi').Folder[]>
  endpointsByFolder: Map<string | null, import('../types/miyoApi').Endpoint[]>
}

const TREE_INDEX: InjectionKey<import('vue').ComputedRef<TreeChildIndex>> =
  Symbol('endpoint-tree-index')
</script>

<script setup lang="ts">
/**
 * EndpointTree：项目接口树（递归）。props.folderId 为 null 时渲染根节点。
 *
 * - 文件夹节点：SVG 图标 + chevron 旋转动画、展开/收起；
 * - 接口节点：方法=彩色 mono 文本、脏标记圆点；
 * - hover 显现「⋯」更多按钮 → 弹出动作菜单（新建/导入/重命名/删除，删除带行内确认）；
 * - 行高 28px、hover/选中态、缩进引导线；
 * - 新建/重命名用行内输入（Enter 提交 / Esc 取消）；
 * - 根级新建文件夹由侧栏头部按钮触发（defineExpose(startEdit)）；
 * - 拖拽移动：跨实例共享载荷，dragover 显式 dropEffect=move；
 * - ARIA 树语义（role=tree/treeitem + aria-level/expanded/selected）与键盘：
 *   ↑↓ 移动焦点、Home/End 跳首尾、→ 展开 / ← 折叠、Enter 打开，roving tabindex。
 */
import { computed, inject, nextTick, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { escapeHtml } from '../utils/highlight'
import { useTreeExpanded } from './endpointTreeExpanded'
import { fuzzyScore } from '../utils/fuzzyMatch'
import { methodTone } from '../utils/methodTone'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import EmptyState from './ui/EmptyState.vue'
import Menu from './ui/Menu.vue'
import Popconfirm from './ui/Popconfirm.vue'
import DeleteConfirmModal from './DeleteConfirmModal.vue'
import type { MenuItem } from './ui/Menu.vue'
import type { Endpoint, Folder } from '../types/miyoApi'

const props = withDefaults(
  defineProps<{
    folderId: string | null
    search?: string
    /** 自增信号：全部展开 / 全部折叠（侧栏工具栏触发，递归层逐层透传）。 */
    expandTick?: number
    collapseTick?: number
    /** ARIA 树层级（根 1，递归子层 +1，随实例透传）。 */
    level?: number
  }>(),
  { search: '', expandTick: 0, collapseTick: 0, level: 1 },
)
const emit = defineEmits<{ importCurl: [folderId: string | null] }>()

const store = useWorkspaceStore()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const { expanded, loadExpanded } = useTreeExpanded()
// 项目切换（含首挂）加载持久化的展开集；递归实例重复调用幂等。
watch(
  () => store.project?.id,
  (pid) => loadExpanded(pid ?? null),
  { immediate: true },
)

/**
 * 行内编辑（新建文件夹 / 重命名）状态在整棵递归树中共享：
 * EndpointTree 是递归组件，每层子树是独立实例；「新建子文件夹」的输入行
 * 渲染在目标文件夹对应的【子】实例里（folderId === parentId 的那层），
 * 状态若只在触发菜单的父实例本地，子实例永远看不到 → 输入框不出现。
 * 根实例 provide 模块级 TREE_EDIT_STATE，全部实例注入同一份读写。
 */
const inheritedEdit = inject(TREE_EDIT_STATE, null)
const editing = inheritedEdit?.editing ?? ref<TreeEditState | null>(null)
const editValue = inheritedEdit?.editValue ?? ref('')
if (!inheritedEdit) {
  provide(TREE_EDIT_STATE, { editing, editValue })
}

// ---------- 共享子索引（根实例构建，递归子树注入复用） ----------
function buildChildIndex(): TreeChildIndex {
  const foldersByParent = new Map<string | null, Folder[]>()
  for (const f of store.folders) {
    const arr = foldersByParent.get(f.parent_id)
    if (arr) arr.push(f)
    else foldersByParent.set(f.parent_id, [f])
  }
  const endpointsByFolder = new Map<string | null, Endpoint[]>()
  for (const e of store.endpoints) {
    const arr = endpointsByFolder.get(e.folder_id)
    if (arr) arr.push(e)
    else endpointsByFolder.set(e.folder_id, [e])
  }
  return { foldersByParent, endpointsByFolder }
}
const inheritedIndex = inject(TREE_INDEX, null)
/** computed 迭代数组即建立深依赖：原地改名/移动/增删都会触发重建。 */
const childIndex = inheritedIndex ?? computed(buildChildIndex)
if (!inheritedIndex) {
  provide(TREE_INDEX, childIndex)
}

// ---------- 子树惰性挂载 ----------
// 首次展开前不创建子实例（大目录导入时全量递归挂载会拖垮首屏）；
// 展开过即保持挂载（v-show 隐藏），保住嵌套展开态与行内编辑态。
// 搜索命中时临时挂载但不记录：清词后自动卸载，DOM 回到最小态。
// 非响应式 Set：写入时机恒为「expanded 变更触发的当次渲染」，无需再触发。
const mountedEver = new Set<string>()
function ensureMounted(id: string): boolean {
  if (expanded.value.has(id)) {
    mountedEver.add(id)
    return true
  }
  if (searchActive.value) return true
  return mountedEver.has(id)
}

// ---------- 接口搜索（实时过滤） ----------
const query = computed(() => props.search.trim().toLowerCase())
const searchActive = computed(() => query.value.length > 0)

function endpointMatches(e: Endpoint): boolean {
  if (!searchActive.value) return true
  const q = props.search.trim()
  const name = e.name || e.path
  // 模糊匹配（子序列 + 词首加分 + 多词过滤，utils/fuzzyMatch）；query 为连续
  // 子串时 highlightName 仍按原逻辑加 <mark>，散点命中不高亮但不漏结果。
  return fuzzyScore(name, q) != null || fuzzyScore(e.path, q) != null
}

/** 文件夹（或其子孙）是否包含匹配的接口（经索引取直属子项，逐层 O(子项数)）。 */
function folderHasMatch(folderId: string): boolean {
  const idx = childIndex.value
  if ((idx.endpointsByFolder.get(folderId) ?? []).some(endpointMatches)) return true
  return (idx.foldersByParent.get(folderId) ?? []).some((f) => folderHasMatch(f.id))
}

/** 命中子串包 <mark> 高亮（已转义，安全注入 v-html）。 */
function highlightName(text: string): string {
  const q = query.value
  if (!q) return escapeHtml(text)
  const lower = text.toLowerCase()
  let out = ''
  let i = 0
  for (;;) {
    const idx = lower.indexOf(q, i)
    if (idx === -1) {
      out += escapeHtml(text.slice(i))
      break
    }
    out += `${escapeHtml(text.slice(i, idx))}<mark class="tree-hit">${escapeHtml(text.slice(idx, idx + q.length))}</mark>`
    i = idx + q.length
  }
  return out
}

const childFolders = computed(() =>
  (childIndex.value.foldersByParent.get(props.folderId) ?? []).filter(
    (f) => !searchActive.value || folderHasMatch(f.id),
  ),
)
const childEndpoints = computed(() =>
  (childIndex.value.endpointsByFolder.get(props.folderId) ?? []).filter(endpointMatches),
)

function toggleFolder(id: string): void {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

// ---------- 全部展开 / 折叠（tick 信号驱动，递归子树各自响应） ----------
function descendantFolderIds(): string[] {
  const out: string[] = []
  const idx = childIndex.value
  const walk = (parentId: string | null): void => {
    for (const f of idx.foldersByParent.get(parentId) ?? []) {
      out.push(f.id)
      walk(f.id)
    }
  }
  walk(props.folderId)
  return out
}

watch(
  () => props.expandTick,
  (t) => {
    if (t) expanded.value = new Set(descendantFolderIds())
  },
)

watch(
  () => props.collapseTick,
  (t) => {
    if (t) expanded.value = new Set<string>()
  },
)

// ---------- 拖拽排序 / 移动（指针事件实现，规避 WKWebView HTML5 DnD 缺陷） ----------
let dragStart = { x: 0, y: 0 }
let folderExpandTimer: number | null = null

const ghostPos = ref({ x: 0, y: 0 })
const ghostInfo = computed(() => {
  if (!dndState.active) return null
  if (dndState.kind === 'endpoint') {
    const e = store.endpoints.find((x) => x.id === dndState.id)
    if (!e) return null
    return { method: e.method, title: (e.name || e.path).slice(0, 5) }
  }
  const f = store.folders.find((x) => x.id === dndState.id)
  if (!f) return null
  return { method: null, title: f.name.slice(0, 5) }
})

function onRowPointerDown(event: PointerEvent, kind: 'folder' | 'endpoint', id: string): void {
  if (event.button !== 0 || dndState.active || editing.value) return
  dragStart = { x: event.clientX, y: event.clientY }
  dndState.kind = kind
  dndState.id = id
  dndState.target = null
  window.addEventListener('pointermove', onPointerMove)
  window.addEventListener('pointerup', onPointerUp)
  window.addEventListener('pointercancel', onPointerCancel)
}

function endDrag(): void {
  window.removeEventListener('pointermove', onPointerMove)
  window.removeEventListener('pointerup', onPointerUp)
  window.removeEventListener('pointercancel', onPointerCancel)
  document.body.classList.remove('dragging-dnd')
  clearFolderExpand()
  dndState.active = false
  dndState.kind = ''
  dndState.id = ''
  dndState.target = null
}

function onPointerCancel(): void {
  endDrag()
}

/** 命中测试：最近的行（folder / before/after）或树根（append 到该层末尾）。 */
function hitTest(x: number, y: number): DndTarget | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null
  if (!el) return null
  const row = el.closest<HTMLElement>('[data-dnd-kind]')
  if (row) {
    const kind = row.dataset.dndKind
    const id = row.dataset.dndId ?? null
    if (kind === 'folder') return { drop: 'folder', id, index: Number.MAX_SAFE_INTEGER }
    if (kind === 'endpoint') {
      const rect = row.getBoundingClientRect()
      const before = y < rect.top + rect.height / 2
      return {
        drop: before ? 'before' : 'after',
        id,
        index: Number(row.dataset.dndIndex ?? '0'),
      }
    }
    return null
  }
  const root = el.closest<HTMLElement>('[data-dnd-tree-root]')
  if (root)
    return { drop: 'root', id: root.dataset.dndTreeRoot || null, index: Number.MAX_SAFE_INTEGER }
  return null
}

function clearFolderExpand(): void {
  if (folderExpandTimer !== null) {
    window.clearTimeout(folderExpandTimer)
    folderExpandTimer = null
  }
}

/** 悬停在关闭的文件夹上 >500ms 时自动展开（方便拖入其中）。 */
function scheduleFolderExpand(id: string | null): void {
  clearFolderExpand()
  if (!id || searchActive.value || expanded.value.has(id)) return
  folderExpandTimer = window.setTimeout(() => {
    expanded.value.add(id)
    folderExpandTimer = null
  }, 500)
}

function onPointerMove(event: PointerEvent): void {
  if (!dndState.active) {
    if (Math.hypot(event.clientX - dragStart.x, event.clientY - dragStart.y) < 5) return
    dndState.active = true
    document.body.classList.add('dragging-dnd')
  }
  ghostPos.value = { x: event.clientX, y: event.clientY }
  let target = hitTest(event.clientX, event.clientY)
  if (target && target.id === dndState.id && (target.drop === 'before' || target.drop === 'after' || target.drop === 'folder'))
    target = null
  if (target?.drop === 'folder') scheduleFolderExpand(target.id)
  else clearFolderExpand()
  dndState.target = target
}

async function onPointerUp(): Promise<void> {
  const wasActive = dndState.active
  const d = { kind: dndState.kind, id: dndState.id, target: dndState.target }
  endDrag()
  setSuppressClick(wasActive)
  if (!wasActive || !d.target || !d.id) return
  const target = d.target
  try {
    let targetFolder: string | null
    let index: number
    if (target.drop === 'folder') {
      if (target.id === d.id) return
      targetFolder = target.id
      index = Number.MAX_SAFE_INTEGER
    } else if (target.drop === 'before' || target.drop === 'after') {
      const targetEp = store.endpoints.find((x) => x.id === target.id)
      if (!targetEp || targetEp.id === d.id) return
      targetFolder = targetEp.folder_id
      index = targetEp.sort_order + (target.drop === 'after' ? 1 : 0)
    } else {
      targetFolder = target.id
      index = Number.MAX_SAFE_INTEGER
    }
    if (d.kind === 'folder') {
      if (targetFolder !== d.id) {
        await store.moveFolder(d.id, targetFolder, index)
        toast.success(t('tree.folderMoved'))
      }
    } else {
      await store.moveEndpoint(d.id, targetFolder, index)
      toast.success(t('tree.endpointMoved'))
    }
  } catch (err) {
    console.error('[EndpointTree.dnd]', err)
    toast.error(t('tree.moveFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function isOverFolder(id: string): boolean {
  return dndState.active && dndState.target?.drop === 'folder' && dndState.target.id === id
}

function isInsertBefore(id: string): boolean {
  return dndState.active && dndState.target?.drop === 'before' && dndState.target.id === id
}

function isInsertAfter(id: string): boolean {
  return dndState.active && dndState.target?.drop === 'after' && dndState.target.id === id
}

function isDraggingSrc(id: string): boolean {
  return dndState.active && dndState.id === id
}

/** 拖拽结束后的残余 click 在树根捕获层消费掉。 */
function onTreeClickCapture(event: Event): void {
  if (consumeSuppressClick()) event.stopPropagation()
}

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

defineExpose({ startEdit })

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
      await store.saveFolder({
        id: crypto.randomUUID(),
        project_id: store.project!.id,
        parent_id: ed.parentId ?? null,
        name,
        sort_order: 0,
        created_at: now,
        updated_at: now,
      })
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

// ---------- 行内动作菜单 ----------
const menu = ref<InstanceType<typeof Menu> | null>(null)
/** 菜单打开时暂停树键盘导航：Menu 自己用 document keydown 处理方向键 / Enter / Esc。 */
const menuOpen = ref(false)
const menuTarget = ref<{ kind: 'folder' | 'endpoint'; id: string } | null>(null)

// ---------- 删除确认弹窗（居中弹窗取代菜单行内确认，接口/文件夹共用） ----------
const showDeleteConfirm = ref(false)
const deleteTarget = ref<{ kind: 'folder' | 'endpoint'; id: string } | null>(null)

/** 基础标题（「删除接口」/「删除文件夹」）。 */
const deleteTitle = computed(() =>
  deleteTarget.value?.kind === 'folder' ? t('tree.deleteFolder') : t('tree.deleteEndpoint'),
)

/** 句式提示：含被删对象名称与不可撤销说明。 */
const deleteMessage = computed(() => {
  const target = deleteTarget.value
  if (!target) return ''
  if (target.kind === 'folder') {
    const f = store.folders.find((x) => x.id === target.id)
    return t('tree.deleteFolderMessage', { name: f?.name ?? '' })
  }
  const ep = store.endpoints.find((x) => x.id === target.id)
  return t('tree.deleteEndpointMessage', { name: ep ? ep.name || ep.path : '' })
})

/** 接口删除时展示 method/path 代码卡片，文件夹无。 */
const deleteMethod = computed(() => {
  const target = deleteTarget.value
  if (!target || target.kind !== 'endpoint') return ''
  return store.endpoints.find((x) => x.id === target.id)?.method ?? ''
})
const deletePath = computed(() => {
  const target = deleteTarget.value
  if (!target || target.kind !== 'endpoint') return ''
  return store.endpoints.find((x) => x.id === target.id)?.path ?? ''
})

/** 确认删除：关弹窗后按类型分发（找最新实体，右键后列表可能已变化）。 */
async function doDelete(): Promise<void> {
  const target = deleteTarget.value
  showDeleteConfirm.value = false
  deleteTarget.value = null
  if (!target) return
  if (target.kind === 'folder') await removeFolder(target.id)
  else {
    const ep = store.endpoints.find((x) => x.id === target.id)
    if (ep) await removeEndpoint(ep)
  }
}

function openFolderMenu(event: MouseEvent, f: Folder): void {
  menuTarget.value = { kind: 'folder', id: f.id }
  menu.value?.openAt(event.currentTarget as HTMLElement, [
    { key: 'endpoint', label: t('tree.newEndpoint'), icon: 'file-plus' },
    { key: 'import', label: t('tree.importCurl'), icon: 'terminal' },
    { key: 'subfolder', label: t('tree.newSubfolder'), icon: 'folder-plus' },
    { key: 'rename', label: t('common.rename'), icon: 'pencil', dividerBefore: true },
    {
      key: 'delete',
      label: t('tree.deleteFolder'),
      icon: 'trash',
      danger: true,
    },
  ], 'left')
}

function openEndpointMenu(event: MouseEvent, e: Endpoint): void {
  menuTarget.value = { kind: 'endpoint', id: e.id }
  menu.value?.openAt(event.currentTarget as HTMLElement, [
    { key: 'copy', label: t('common.copyAction'), icon: 'copy' },
    { key: 'rename', label: t('common.rename'), icon: 'pencil' },
    {
      key: 'delete',
      label: t('tree.deleteEndpoint'),
      icon: 'trash',
      danger: true,
      dividerBefore: true,
    },
  ], 'left')
}

function onMenuSelect(item: MenuItem): void {
  const target = menuTarget.value
  if (!target) return
  if (item.key === 'delete') {
    deleteTarget.value = target
    showDeleteConfirm.value = true
    return
  }
  if (target.kind === 'folder') {
    if (item.key === 'subfolder') {
      // 先展开目标文件夹：输入行渲染在其子树内，折叠状态下不可见
      expanded.value.add(target.id)
      startEdit('create-folder', { parentId: target.id })
    }
    else if (item.key === 'endpoint') store.openNewEndpoint(target.id)
    else if (item.key === 'import') emit('importCurl', target.id)
    else if (item.key === 'rename') startEdit('rename-folder', { id: target.id })
  } else {
    if (item.key === 'copy') duplicate(store.endpoints.find((x) => x.id === target.id)!)
    else if (item.key === 'rename') startEdit('rename-endpoint', { id: target.id })
  }
}

// ---------- 多选（Ctrl/⌘ 点选、Shift 同层连选） ----------
/** 接口行点击：修饰键走多选，否则清空选择并打开。 */
function onEndpointClick(e: Endpoint, index: number, event: MouseEvent): void {
  if (event.metaKey || event.ctrlKey) {
    if (treeSelection.endpoints.has(e.id)) treeSelection.endpoints.delete(e.id)
    else treeSelection.endpoints.add(e.id)
    return
  }
  if (event.shiftKey) {
    const ids = childEndpoints.value.map((x) => x.id)
    const anchor = [...treeSelection.endpoints].map((id) => ids.indexOf(id)).find((i) => i !== -1)
    const from = anchor ?? index
    const [lo, hi] = from < index ? [from, index] : [index, from]
    for (let i = lo; i <= hi; i++) treeSelection.endpoints.add(ids[i])
    return
  }
  clearTreeSelection()
  store.openEndpoint(e)
}

/** 文件夹名点击：修饰键走多选，否则展开/收起（原行为）。 */
function onFolderClick(f: Folder, event: MouseEvent): void {
  if (event.metaKey || event.ctrlKey) {
    if (treeSelection.folders.has(f.id)) treeSelection.folders.delete(f.id)
    else treeSelection.folders.add(f.id)
    return
  }
  toggleFolder(f.id)
}

/**
 * 行级点击：整行（文本/徽章/空白 padding）都响应，行内控件除外。
 * 背景：点击原来只绑在 .tree-name 文本 span 上，行 padding 与
 * 文本右侧空白点不中任何东西（点击死区）。
 */
function interactiveTarget(event: MouseEvent): boolean {
  return !!(event.target as HTMLElement).closest?.('input,button,textarea,select,a')
}

function onEndpointRowClick(e: Endpoint, index: number, event: MouseEvent): void {
  if (interactiveTarget(event)) return
  onEndpointClick(e, index, event)
}

function onFolderRowClick(f: Folder, event: MouseEvent): void {
  if (interactiveTarget(event)) return
  onFolderClick(f, event)
}

// ---------- 键盘导航（ARIA tree：↑↓ 移动、Home/End 首尾、→ 展开、← 折叠、Enter 打开） ----------
const rootEl = ref<HTMLElement | null>(null)

/** roving tabindex：焦点行 0，其余 -1（焦点行由模块级 treeFocus 跨递归实例共享）。 */
function rowTabbable(id: string): number {
  return treeFocus.id === id ? 0 : -1
}

/** v-show 收起的子树按行内 display:none 判定（jsdom 无布局，不能依赖 getClientRects）。 */
function isRowVisible(el: HTMLElement): boolean {
  for (let n: HTMLElement | null = el; n; n = n.parentElement) {
    if (n.style.display === 'none') return false
  }
  return true
}

/** 整棵树当前可见的行（DOM 顺序 = 阅读顺序，跨递归实例）。 */
function visibleRows(): HTMLElement[] {
  const root = rootEl.value
  if (!root) return []
  return [...root.querySelectorAll<HTMLElement>('.tree-row[data-dnd-id]')].filter(isRowVisible)
}

function focusRow(el: HTMLElement | null): void {
  if (!el) return
  const id = el.dataset.dndId
  if (id) treeFocus.id = id
  el.focus()
}

/**
 * 焦点行失效（行被删除 / 数据重载 / 切换项目）时回落到首行，
 * 保证树始终有一行 tabindex=0，Tab 能进入整棵树。
 */
function syncTreeFocus(): void {
  if (props.folderId !== null) return
  const rows = visibleRows()
  if (!rows.length) {
    treeFocus.id = null
    return
  }
  if (!rows.some((r) => r.dataset.dndId === treeFocus.id)) {
    treeFocus.id = rows[0].dataset.dndId ?? null
  }
}

/** 行获得焦点（鼠标点选 / 程序 focus）即同步 roving tabindex 真源。 */
function onTreeFocusIn(event: FocusEvent): void {
  if (props.folderId !== null) return
  const target = event.target as HTMLElement | null
  const row = (target?.closest('.tree-row[data-dnd-id]') ?? null) as HTMLElement | null
  if (row?.dataset.dndId) treeFocus.id = row.dataset.dndId
}

/** 文件夹行的展开指示（chevron 旋转态）：open = 子树当前可见。 */
function chevronOf(row: HTMLElement): HTMLElement | null {
  if (row.dataset.dndKind !== 'folder') return null
  return row.querySelector<HTMLElement>('.tree-chevron:not(.spacer)')
}

/** 通过 chevron 的点击处理器切换展开：由行所属实例自己执行，跨层状态不串。 */
function toggleViaChevron(row: HTMLElement): void {
  chevronOf(row)?.click()
}

const TREE_NAV_KEYS = ['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'Enter']

function onTreeKeydown(event: KeyboardEvent): void {
  // 处理只挂在根实例上（嵌套实例的按键冒泡到根统一处理）
  if (props.folderId !== null) return
  if (menuOpen.value) return // 菜单打开时方向键归菜单
  if (!TREE_NAV_KEYS.includes(event.key)) return
  const target = event.target as HTMLElement
  // 行内编辑输入框保留原生键盘行为（方向键在文本光标上，不抢焦点）
  if (target.closest('input, textarea, select, [contenteditable="true"]')) return
  // 行内按钮（⋯ 菜单）的 Enter 归原生 click；方向键仍可离开它继续在树内移动
  if (event.key === 'Enter' && target.closest('button, a')) return
  const rows = visibleRows()
  if (!rows.length) return
  const current = target.closest<HTMLElement>('.tree-row[data-dnd-id]')
  if (!current) return
  const index = rows.indexOf(current)

  if (event.key === 'Enter') {
    event.preventDefault()
    const id = current.dataset.dndId ?? ''
    if (current.dataset.dndKind === 'folder') toggleViaChevron(current)
    else {
      const e = store.endpoints.find((x) => x.id === id)
      if (e) {
        clearTreeSelection()
        store.openEndpoint(e)
      }
    }
    return
  }
  if (index === -1) return

  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    const next = event.key === 'ArrowDown' ? index + 1 : index - 1
    focusRow(rows[Math.min(Math.max(next, 0), rows.length - 1)])
    return
  }
  if (event.key === 'Home' || event.key === 'End') {
    event.preventDefault()
    focusRow(event.key === 'Home' ? rows[0] : rows[rows.length - 1])
    return
  }

  const chevron = chevronOf(current)
  if (event.key === 'ArrowRight') {
    event.preventDefault()
    if (!chevron) return // 接口行无展开语义
    if (!chevron.classList.contains('open')) {
      toggleViaChevron(current) // 折叠 → 展开
      return
    }
    // 已展开 → 焦点下移进第一个子行（.tree-children 紧跟在文件夹行之后）
    const kids = current.nextElementSibling?.querySelectorAll<HTMLElement>('.tree-row[data-dnd-id]')
    focusRow([...(kids ?? [])].find(isRowVisible) ?? null)
    return
  }

  // ArrowLeft：展开的文件夹先折叠，否则上移到父文件夹行
  event.preventDefault()
  if (chevron?.classList.contains('open') && !searchActive.value) {
    toggleViaChevron(current)
    return
  }
  const parentRow = current.closest('.tree-children')?.previousElementSibling
  if (parentRow?.matches?.('.tree-row[data-dnd-id]')) focusRow(parentRow as HTMLElement)
}

// 数据 / 搜索变化重算焦点行；根实例挂载后兜底初始化。
// length 一并 watch：嵌套层增删行时根实例自身的 child* 不变，但行消失会让焦点行失效。
if (props.folderId === null) {
  watch(
    [childFolders, childEndpoints, () => store.folders.length, () => store.endpoints.length],
    () => {
      void nextTick(syncTreeFocus)
    },
  )
  onMounted(() => {
    void nextTick(syncTreeFocus)
  })
  onBeforeUnmount(() => {
    treeFocus.id = null
  })
}

const selectionCount = computed(
  () => treeSelection.endpoints.size + treeSelection.folders.size,
)

/** 批量删除选中（含文件夹子树归一化）：快照整体可撤销（store.undoDelete）。 */
async function batchDelete(): Promise<void> {
  const epIds = [...treeSelection.endpoints]
  const folderIds = [...treeSelection.folders]
  clearTreeSelection()
  try {
    await store.batchDelete(epIds, folderIds)
  } catch (err) {
    console.error('[EndpointTree.batchDelete]', err)
    toast.error(t('tree.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

/** 批量移动选中接口到目标文件夹（文件夹选中项不动，仅移动接口）。 */
async function batchMove(targetFolderId: string | null): Promise<void> {
  const epIds = [...treeSelection.endpoints]
  if (!epIds.length) return
  clearTreeSelection()
  try {
    await store.batchMoveEndpoints(epIds, targetFolderId)
  } catch (err) {
    console.error('[EndpointTree.batchMove]', err)
    toast.error(t('tree.batchMoveFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

/** 移动目标菜单：根目录 + 全部文件夹（按名称）。 */
function openMoveMenu(event: MouseEvent): void {
  const items: MenuItem[] = [
    { key: '__root__', label: t('tree.rootFolder'), icon: 'folder' },
    ...[...store.folders]
      .sort((a, b) => a.name.localeCompare(b.name, 'zh'))
      .map((f) => ({ key: f.id, label: f.name, icon: 'folder' as const })),
  ]
  batchMenu.value?.openAt(event.currentTarget as HTMLElement, items, 'left')
}

const batchMenu = ref<InstanceType<typeof Menu> | null>(null)

function onBatchMenuSelect(item: MenuItem): void {
  void batchMove(item.key === '__root__' ? null : item.key)
}
</script>

<template>
  <div
    class="tree"
    ref="rootEl"
    :role="folderId === null ? 'tree' : undefined"
    :aria-multiselectable="folderId === null ? true : undefined"
    :data-dnd-tree-root="folderId ?? ''"
    @click.capture="onTreeClickCapture"
    @keydown="onTreeKeydown"
    @focusin="onTreeFocusIn"
  >
    <template v-for="f in childFolders" :key="f.id">
      <div
        class="tree-row"
        :class="{ 'dnd-over': isOverFolder(f.id), selected: treeSelection.folders.has(f.id) }"
        role="treeitem"
        :aria-level="level"
        :aria-expanded="expanded.has(f.id) || searchActive"
        :aria-selected="treeSelection.folders.has(f.id)"
        :tabindex="rowTabbable(f.id)"
        data-dnd-kind="folder"
        :data-dnd-id="f.id"
        @pointerdown="onRowPointerDown($event, 'folder', f.id)"
        @click="onFolderRowClick(f, $event)"
        @contextmenu.prevent="openFolderMenu($event, f)"
      >
        <span
          class="tree-chevron"
          :class="{ open: expanded.has(f.id) || searchActive }"
          aria-hidden="true"
          @click.stop="toggleFolder(f.id)"
        >
          <Icon name="chevron-right" :size="12" :stroke-width="1.25" />
        </span>
        <template v-if="editing?.kind === 'rename-folder' && editing.id === f.id">
          <input
            v-model="editValue"
            class="miyo-input miyo-input-sm tree-input"
            v-focus-end
            autofocus
            @keyup.enter="commitEdit"
            @keyup.esc="cancelEdit"
            @blur="commitEdit"
          />
        </template>
        <template v-else>
          <span class="tree-folder-icon" @click.stop="toggleFolder(f.id)">
            <Icon :name="expanded.has(f.id) || searchActive ? 'folder-open' : 'folder'" :size="15" />
          </span>
          <span class="tree-name folder" v-tooltip-overflow>{{ f.name }}</span>
          <span class="tree-actions">
            <IconButton name="more-horizontal" :size="13" :title="t('common.moreActions')" @click="openFolderMenu($event, f)" />
          </span>
        </template>
      </div>
      <div v-show="expanded.has(f.id) || searchActive" class="tree-children" role="group">
        <EndpointTree
          v-if="ensureMounted(f.id)"
          :folder-id="f.id"
          :search="props.search"
          :expand-tick="props.expandTick"
          :collapse-tick="props.collapseTick"
          :level="level + 1"
          @import-curl="$emit('importCurl', $event)"
        />
      </div>
    </template>

    <div v-if="editing?.kind === 'create-folder' && editing.parentId === folderId" class="tree-row">
      <input
        v-model="editValue"
        class="miyo-input miyo-input-sm tree-input"
          :placeholder="t('tree.folderNamePh')"
        autofocus
        @keyup.enter="commitEdit"
        @keyup.esc="cancelEdit"
        @blur="commitEdit"
      />
    </div>

    <template v-for="(e, i) in childEndpoints" :key="e.id">
      <div
        class="tree-row"
        :class="{
          active: store.activeTabId === e.id,
          selected: treeSelection.endpoints.has(e.id),
          'insert-before': isInsertBefore(e.id),
          'insert-after': isInsertAfter(e.id),
          'dragging-src': isDraggingSrc(e.id),
        }"
        role="treeitem"
        :aria-level="level"
        :aria-selected="treeSelection.endpoints.has(e.id) || store.activeTabId === e.id"
        :tabindex="rowTabbable(e.id)"
        data-dnd-kind="endpoint"
        :data-dnd-id="e.id"
        :data-dnd-index="i"
        @pointerdown="onRowPointerDown($event, 'endpoint', e.id)"
        @click="onEndpointRowClick(e, i, $event)"
        @contextmenu.prevent="openEndpointMenu($event, e)"
      >
        <template v-if="editing?.kind === 'rename-endpoint' && editing.id === e.id">
          <input
            v-model="editValue"
            class="miyo-input miyo-input-sm tree-input"
            v-focus-end
            autofocus
            @keyup.enter="commitEdit"
            @keyup.esc="cancelEdit"
            @blur="commitEdit"
          />
        </template>
        <template v-else>
          <span class="tree-chevron spacer"></span>
          <span class="tree-method" :class="methodTone(e.method)">{{ e.method }}</span>
          <span class="tree-name" :class="{ dirty: store.isDirty(e.id) }" v-tooltip-overflow>
            <span class="tree-name-text" v-html="highlightName(e.name || e.path)"></span>
            <Icon v-if="store.isDirty(e.id)" class="tree-dirty" name="dot" :size="8" />
          </span>
          <span class="tree-actions">
            <IconButton name="more-horizontal" :size="13" :title="t('common.moreActions')" @click="openEndpointMenu($event, e)" />
          </span>
        </template>
      </div>
    </template>

    <EmptyState
      v-if="folderId === null && searchActive && !childFolders.length && !childEndpoints.length"
      icon="search"
      :title="t('tree.noMatch')"
      compact
    />
  </div>

    <!-- 批量操作条（仅根实例渲染）：多选后出现 -->
    <div v-if="folderId === null && selectionCount > 0" class="tree-batch">
      <span class="tree-batch-count">{{ t('tree.selected', { n: selectionCount }) }}</span>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="openMoveMenu($event)">{{ t('tree.moveTo') }}</button>
      <Popconfirm
        :title="t('tree.batchDeleteConfirm', { n: selectionCount })"
        :confirm-text="t('common.delete')"
        danger
        @confirm="batchDelete"
      >
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button">{{ t('common.delete') }}</button>
      </Popconfirm>
      <button class="miyo-btn miyo-btn-sm miyo-btn-ghost" type="button" @click="clearTreeSelection">
        {{ t('tree.clearSelection') }}
      </button>
    </div>

    <Menu ref="menu" @select="onMenuSelect" @open="menuOpen = true" @close="menuOpen = false" />
    <Menu ref="batchMenu" @select="onBatchMenuSelect" @open="menuOpen = true" @close="menuOpen = false" />

    <DeleteConfirmModal
      v-model:open="showDeleteConfirm"
      :title="deleteTitle"
      :message="deleteMessage"
      :method="deleteMethod || undefined"
      :path="deletePath || undefined"
      @confirm="doDelete"
    />

  <Teleport to="body">
    <div
      v-if="folderId === null && dndState.active && ghostInfo"
      class="dnd-ghost"
      :style="{ left: ghostPos.x + 'px', top: ghostPos.y + 'px' }"
    >
      <span v-if="ghostInfo.method" class="miyo-method" :class="`miyo-method-${ghostInfo.method.toLowerCase()}`">
        {{ ghostInfo.method }}
      </span>
      <Icon v-else name="folder" :size="13" />
      <span class="ghost-title">{{ ghostInfo.title }}</span>
    </div>
  </Teleport>
</template>

<style scoped>
.tree {
  display: flex;
  flex-direction: column;
  /* 行间零间隙：gap 会在行与行之间留下点不中的死区（圆角行边角除外）；
     视觉呼吸感由行内 padding 承担，对标 VS Code / Obsidian 目录树 */
  gap: 0;
}

.tree-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 30px;
  padding: 3px 8px;
  border-radius: var(--radius-md);
  cursor: default;
  transition: background var(--dur) var(--ease);
}
.tree-row:hover {
  background: var(--bg-hover);
}
/* 选中态：Obsidian 全宽紫 pill（渐变 + 光晕 + 白字），渐变走 --grad-primary 系令牌（浅色降刺眼） */
.tree-row.active {
  background: var(--grad-primary);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.22),
    0 4px 14px var(--primary-glow);
}
.tree-row.active::before {
  content: none;
}
.tree-row.active .tree-name,
.tree-row.active .tree-name-text {
  color: #fff;
}
.tree-row.active .tree-chevron,
.tree-row.active .tree-folder-icon {
  color: rgba(255, 255, 255, 0.8);
}
.tree-row.active .tree-actions :deep(.ib) {
  color: rgba(255, 255, 255, 0.75);
}
.tree-row.active .tree-actions :deep(.ib:hover) {
  background: rgba(255, 255, 255, 0.18);
  color: #fff;
}
.tree-row.active .tree-method {
  /* 徽章在紫底上提亮底色保证可读 */
  background: rgba(255, 255, 255, 0.16);
  border-color: rgba(255, 255, 255, 0.25);
  color: #fff;
}
/* 多选态：淡蓝底（与激活态可叠加，激活优先显示） */
.tree-row.selected {
  background: rgba(56, 139, 248, 0.16);
}
/* 批量操作条（根实例，多选时出现） */
.tree-batch {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  margin-top: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  font-size: 12px;
}
.tree-batch-count {
  flex: 1;
  color: var(--text-2);
  white-space: nowrap;
}
/* 拖入文件夹：细虚线描边 + 浅色高亮 */
.tree-row.dnd-over {
  background: var(--accent-tint);
  outline: 1px dashed var(--accent);
  outline-offset: -1px;
}
/* 拖拽中的原行淡出（配合浮空小卡片，去掉厚重描边） */
.tree-row.dragging-src {
  opacity: 0.35;
}
/* 接口间精确插入线：悬停上半 → 上方线；悬停下半 → 下方线 */
.tree-row.insert-before {
  border-top: 2px solid var(--accent);
}
.tree-row.insert-after {
  border-bottom: 2px solid var(--accent);
}
.tree-row.active .tree-name {
  color: var(--text-1);
  font-weight: 500;
}

.tree-chevron {
  width: 16px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  opacity: 0.75;
  cursor: pointer;
  user-select: none;
  transition:
    color var(--dur) var(--ease),
    opacity var(--dur) var(--ease);
}
.tree-chevron:hover {
  color: var(--text-1);
  opacity: 1;
}
.tree-chevron svg {
  transition: transform var(--dur) var(--ease);
}
.tree-chevron.open svg {
  transform: rotate(90deg);
}
.tree-chevron.spacer {
  cursor: default;
}

.tree-folder-icon {
  display: inline-flex;
  align-items: center;
  color: var(--post);
  cursor: pointer;
  flex-shrink: 0;
}

.tree-name {
  flex: 1;
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  color: var(--text-1);
  cursor: pointer;
}
.tree-name.folder {
  font-weight: 600;
}
.tree-name-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 搜索命中高亮（v-html 注入，需 :deep） */
:deep(.tree-hit) {
  padding: 0 1px;
  border-radius: var(--radius-sm);
  background: var(--warning-tint);
  color: var(--warning);
}
.tree-dirty {
  color: var(--warning);
  flex-shrink: 0;
}

/* Method 轻量徽章：布局尺寸，颜色走共享 methodTone（utils/methodTone.ts） */
.tree-method {
  width: 42px;
  height: 18px;
  flex-shrink: 0;
  margin-right: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  border-width: 1px;
  border-style: solid;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.06em;
  line-height: 1.2;
  white-space: nowrap;
}

.tree-actions {
  display: inline-flex;
  align-items: center;
  gap: 1px;
  flex-shrink: 0;
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}
.tree-row:hover .tree-actions,
.tree-row:focus-within .tree-actions {
  opacity: 1;
}
/* 更多按钮：紧凑 20px 触点，默认无背景，hover 淡灰 */
.tree-actions :deep(.ib) {
  width: 20px;
  height: 20px;
  border-radius: 6px;
  background: transparent;
  color: var(--text-3);
}
.tree-actions :deep(.ib:hover) {
  background: var(--bg-hover);
  color: var(--text-1);
}

.tree-input {
  flex: 1;
  min-width: 0;
}

/* 缩进引导线 */
.tree-children {
  padding-left: 16px;
  position: relative;
}
.tree-children::before {
  content: '';
  position: absolute;
  left: 7px;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border);
}

/* 拖拽浮空小卡片：方法标签 + 标题前 5 字符，跟随光标 */
.dnd-ghost {
  position: fixed;
  transform: translate(14px, 18px);
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.4);
  opacity: 0.8;
  pointer-events: none;
  z-index: 9999;
  white-space: nowrap;
}
.dnd-ghost .ghost-title {
  font-size: 12px;
  color: var(--text-1);
}
</style>
