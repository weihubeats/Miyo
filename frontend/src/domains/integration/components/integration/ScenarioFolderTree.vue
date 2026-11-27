<script setup lang="ts">
/**
 * ScenarioFolderTree：集成测试场景库左侧业务目录树。
 *
 * - 扁平渲染（flattenFolderTree，避免递归组件）+ 展开/折叠状态由父级持有；
 * - 每行：折叠箭头 / 目录图标 / 名称 / 场景数 / 执行状态汇总点；
 * - 拖拽：目录行可拖到其他目录（移动目录），场景可拖到目录（移动场景）；
 *   根级「全部场景」行作为移到根目录的落点；
 * - 行内操作：新建子目录 / 重命名 / 删除（悬停显示，避免常驻噪音）。
 */
import { computed, inject, ref } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { flattenFolderTree, type FolderTreeNode } from '@/domains/integration/utils/folderTree'
import type { ScenarioFolder } from '@/types/miyoApi'

/** 拖拽负载（父级 provide：kind 区分目录 / 场景，ids 为场景多选）。 */
export interface ScenarioDragState {
  kind: 'scenario' | 'folder' | null
  ids: string[]
}

const props = defineProps<{
  nodes: FolderTreeNode[]
  /** null = 全部场景；'' = 未分类；其余 = 目录 id。 */
  selectedId: string | null
  expanded: string[]
  totalCount: number
  uncategorized: number
  /** 当前筛选结果数（「全部场景」行右侧展示）。 */
  filteredCount: number
  canMoveFolder: (folderId: string, parentId: string | null) => boolean
}>()

const emit = defineEmits<{
  select: [id: string | null]
  toggle: [id: string]
  create: [parentId: string | null]
  rename: [folder: ScenarioFolder]
  remove: [folder: ScenarioFolder]
  run: [folderId: string]
  moveFolder: [id: string, parentId: string | null]
  dropScenarios: [folderId: string | null]
}>()

const t = useLocaleStore().t
const drag = inject<ScenarioDragState>('scenarioDrag', { kind: null, ids: [] })

const rows = computed(() => flattenFolderTree(props.nodes, props.expanded))
const open = computed(() => new Set(props.expanded))

/** 悬停行（行内操作显示）。 */
const hoverId = ref<string | null>(null)
/** 拖拽落点高亮（row key：'root' | folder id）。 */
const dropTarget = ref<string | null>(null)

function statusClass(status: FolderTreeNode['status']): string {
  return status === 'ok' ? 'ok' : status === 'fail' ? 'fail' : 'none'
}

function onDragOver(e: DragEvent, key: string): void {
  if (!drag.kind) return
  e.preventDefault()
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
  dropTarget.value = key
}

function onDragLeave(key: string): void {
  if (dropTarget.value === key) dropTarget.value = null
}

function onDrop(_key: string, parentId: string | null): void {
  dropTarget.value = null
  if (drag.kind === 'scenario') {
    emit('dropScenarios', parentId)
  } else if (drag.kind === 'folder' && drag.ids[0] && drag.ids[0] !== parentId) {
    emit('moveFolder', drag.ids[0], parentId)
  }
}

/** 目录行拖拽开始（区分目录 / 场景负载由父级 provide 的 drag 状态决定）。 */
function onFolderDragStart(e: DragEvent, folder: ScenarioFolder): void {
  drag.kind = 'folder'
  drag.ids = [folder.id]
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('application/x-miyo-folder', folder.id)
  }
}

function onFolderDragEnd(): void {
  drag.kind = null
  drag.ids = []
  dropTarget.value = null
}

/** 目录行是否可作为当前拖拽落点（防环 + 不允许拖到自己）。 */
function canDropHere(folderId: string): boolean {
  if (drag.kind === 'scenario') return true
  if (drag.kind === 'folder' && drag.ids[0]) {
    return props.canMoveFolder(drag.ids[0], folderId)
  }
  return false
}
</script>

<template>
  <aside class="sft-root">
    <div class="sft-head">
      <span class="sft-title">{{ t('integration.folderTitle') }}</span>
      <IconButton
        name="folder-plus"
        :size="13"
        :title="t('integration.folderNew')"
        @click="emit('create', null)"
      />
    </div>

    <!-- 根级：全部场景 / 所有目录（也是拖到根目录的落点） -->
    <button
      type="button"
      class="sft-row sft-row-root"
      :class="{ active: selectedId === null, 'drop-ok': dropTarget === 'root' }"
      :title="t('integration.folderRootDrop')"
      @click="emit('select', null)"
      @dragover="onDragOver($event, 'root')"
      @dragleave="onDragLeave('root')"
      @drop.prevent="onDrop('root', null)"
    >
      <span class="sft-chevron-space" aria-hidden="true"></span>
      <Icon name="beaker" :size="13" class="sft-icon" />
      <span class="sft-name">{{ t('integration.folderAll') }}</span>
      <span class="sft-count">
        {{ filteredCount }}<template v-if="filteredCount !== totalCount">/{{ totalCount }}</template>
      </span>
    </button>

    <div class="sft-list">
      <div
        v-for="row in rows"
        :key="row.node.folder.id"
        class="sft-row"
        :class="{
          active: selectedId === row.node.folder.id,
          'drop-ok': dropTarget === row.node.folder.id,
        }"
        :style="{ paddingLeft: `${8 + row.depth * 14}px` }"
        :draggable="true"
        role="button"
        tabindex="0"
        @click="emit('select', row.node.folder.id)"
        @keydown.enter="emit('select', row.node.folder.id)"
        @mouseenter="hoverId = row.node.folder.id"
        @mouseleave="hoverId = null"
        @dragstart="onFolderDragStart($event, row.node.folder)"
        @dragend="onFolderDragEnd"
        @dragover="canDropHere(row.node.folder.id) && onDragOver($event, row.node.folder.id)"
        @dragleave="onDragLeave(row.node.folder.id)"
        @drop.prevent="canDropHere(row.node.folder.id) && onDrop(row.node.folder.id, row.node.folder.id)"
      >
        <button
          v-if="row.node.children.length"
          type="button"
          class="sft-chevron"
          :title="open.has(row.node.folder.id) ? t('integration.folderTitle') : t('integration.folderTitle')"
          @click.stop="emit('toggle', row.node.folder.id)"
        >
          <Icon :name="open.has(row.node.folder.id) ? 'chevron-down' : 'chevron-right'" :size="12" />
        </button>
        <span v-else class="sft-chevron-space" aria-hidden="true"></span>
        <Icon
          :name="open.has(row.node.folder.id) ? 'folder-open' : 'folder'"
          :size="13"
          class="sft-icon"
        />
        <span class="sft-name" :title="row.node.folder.name">{{ row.node.folder.name }}</span>
        <span
          class="sft-dot"
          :class="statusClass(row.node.status)"
          :title="t('integration.libStatusOk')"
          aria-hidden="true"
        ></span>
        <span class="sft-count">{{ row.node.scenarioCount }}</span>
        <span v-if="hoverId === row.node.folder.id" class="sft-actions">
          <IconButton
            name="play"
            :size="12"
            :title="t('testSuite.run')"
            @click.stop="emit('run', row.node.folder.id)"
          />
          <IconButton
            name="folder-plus"
            :size="12"
            :title="t('integration.folderNewChild')"
            @click.stop="emit('create', row.node.folder.id)"
          />
          <IconButton
            name="pencil"
            :size="12"
            :title="t('integration.folderRename')"
            @click.stop="emit('rename', row.node.folder)"
          />
          <IconButton
            name="trash"
            :size="12"
            :title="t('integration.folderDelete')"
            @click.stop="emit('remove', row.node.folder)"
          />
        </span>
      </div>

      <p v-if="!rows.length" class="sft-empty">{{ t('integration.folderEmpty') }}</p>
    </div>

    <!-- 未分类 -->
    <button
      type="button"
      class="sft-row sft-row-leaf"
      :class="{ active: selectedId === '', 'drop-ok': dropTarget === 'uncategorized' }"
      @click="emit('select', '')"
      @dragover="onDragOver($event, 'uncategorized')"
      @dragleave="onDragLeave('uncategorized')"
      @drop.prevent="onDrop('uncategorized', null)"
    >
      <span class="sft-chevron-space" aria-hidden="true"></span>
      <Icon name="file" :size="13" class="sft-icon" />
      <span class="sft-name">{{ t('integration.libNoFolder') }}</span>
      <span class="sft-count">{{ uncategorized }}</span>
    </button>
  </aside>
</template>

<style scoped>
.sft-root {
  flex: none;
  width: 232px;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-right: 1px solid var(--border);
  padding: 8px 6px;
  overflow-y: auto;
}

.sft-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 6px 6px;
}

.sft-title {
  font-size: var(--fs-2xxs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.sft-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.sft-row {
  display: flex;
  align-items: center;
  gap: 5px;
  /* button 子元素 flex（WKWebView 内 block 子元素不伸展，AGENTS.md 实证）。 */
  width: 100%;
  min-height: 26px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-family: inherit;
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
}

.sft-row:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.sft-row.active {
  background: var(--accent-tint);
  border-color: var(--accent);
  color: var(--accent);
}

.sft-row.drop-ok {
  border-style: dashed;
  border-color: var(--accent);
  background: var(--accent-tint);
}

.sft-row-root,
.sft-row-leaf {
  margin-bottom: 2px;
}

.sft-chevron,
.sft-chevron-space {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
}

.sft-chevron {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

.sft-chevron:hover {
  color: var(--text-1);
}

.sft-icon {
  flex: none;
  color: currentColor;
  opacity: 0.85;
}

.sft-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sft-count {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.sft-dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.sft-dot.ok {
  background: var(--success);
}

.sft-dot.fail {
  background: var(--danger);
}

.sft-dot.none {
  background: var(--text-3);
  opacity: 0.4;
}

.sft-actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 1px;
}

.sft-empty {
  margin: 4px 8px;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}
</style>