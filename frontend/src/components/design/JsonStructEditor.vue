<script setup lang="ts">
/**
 * JsonStructEditor：设计页结构化 Body 编辑器（YApi 式字段树）。
 *
 * - 本地树（根 JsonTreeNode）持有结构 + 每字段 description / required；
 * - 扁平化递归渲染（depth 缩进），支持任意嵌套；
 * - 类型切换 / 改名 / 增删 → rebindPointers 后发出 update:raw + update:docs；
 * - update:docs 与 update:raw 成对发出，父级写回 body_docs / example.docs；
 * - 外部 raw 变化（文本模式回切）经 watch 重新 parse，防止回环覆写。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import IconButton from '../ui/IconButton.vue'
import EmptyState from '../ui/EmptyState.vue'
import {
  docsFromTree,
  emptyJsonObjectNode,
  freshNid,
  parseJsonTreeWithDocs,
  rebindPointers,
  treeToPrettyJson,
  pointerEscape,
  type JsonTreeNode,
  type JsonTreeType,
} from '../../utils/jsonTree'
import type { FieldDoc } from '../../types/miyoApi'

const props = defineProps<{
  raw: string
  docs?: Record<string, FieldDoc>
  disabled?: boolean
}>()

const emit = defineEmits<{
  'update:raw': [raw: string]
  'update:docs': [docs: Record<string, FieldDoc>]
}>()

const locale = useLocaleStore()
const t = locale.t

const root = ref<JsonTreeNode>(emptyJsonObjectNode())

/** 首次 hydrate 前不走回声守卫（初始 root 为空对象，序列化会误等）。 */
let hydrated = false

/**
 * 文本模式回切 / 外部 raw 更新 → 重解析 + 注入 docs。
 * 自己 emit 写回的回声（text === 当前树序列化）跳过，防换树丢焦点。
 */
watch(
  () => props.raw,
  (text) => {
    if (hydrated && text === treeToPrettyJson(root.value)) return
    const next = parseJsonTreeWithDocs(text, props.docs)
    if (next) root.value = next
    hydrated = true
  },
  { immediate: true },
)

/** 每次变更后发出整树序列化 + sidecar。 */
function emitAll(): void {
  root.value = rebindPointers(root.value, root.value.pointer)
  emit('update:raw', treeToPrettyJson(root.value))
  emit('update:docs', docsFromTree(root.value))
}

const TYPE_OPTIONS: { value: JsonTreeType; label: string }[] = [
  { value: 'string', label: 'String' },
  { value: 'number', label: 'Number' },
  { value: 'boolean', label: 'Boolean' },
  { value: 'object', label: 'Object' },
  { value: 'array', label: 'Array' },
  { value: 'null', label: 'Null' },
]

/** 扁平化行：node + 嵌套深度（渲染缩进）。 */
interface FlatRow {
  node: JsonTreeNode
  depth: number
  /** 父容器（删除 / 加子用）。 */
  parent: JsonTreeNode
  index: number
}

function walk(node: JsonTreeNode, depth: number, out: FlatRow[]): void {
  node.children.forEach((child, i) => {
    out.push({ node: child, depth, parent: node, index: i })
    if (child.type === 'object' || child.type === 'array') {
      walk(child, depth + 1, out)
    }
  })
}

const rows = computed<FlatRow[]>(() => {
  const out: FlatRow[] = []
  walk(root.value, 0, out)
  return out
})

function uniqueKey(parent: JsonTreeNode, base: string): string {
  const taken = new Set(parent.children.map((c) => c.key))
  let n = 1
  let key = base
  while (taken.has(key)) {
    n += 1
    key = `${base}${n}`
  }
  return key
}

function emptyLeaf(key: string, parent: JsonTreeNode): JsonTreeNode {
  const isArray = parent.type === 'array'
  const pointer = isArray
    ? `${parent.pointer}/0/${pointerEscape(key)}`
    : `${parent.pointer}/${pointerEscape(key)}`
  return {
    nid: freshNid(),
    key,
    type: 'string',
    value: '',
    children: [],
    pointer,
    description: '',
    required: true,
  }
}

function addChild(parent: JsonTreeNode): void {
  const base = parent.type === 'array' ? 'item' : 'field'
  parent.children.push(emptyLeaf(uniqueKey(parent, base), parent))
  emitAll()
}

function removeAt(parent: JsonTreeNode, index: number): void {
  parent.children.splice(index, 1)
  emitAll()
}

function changeType(node: JsonTreeNode, next: JsonTreeType): void {
  node.type = next
  if (next === 'object' || next === 'array') {
    node.value = ''
    node.children = []
  } else {
    node.children = []
    node.value =
      next === 'string' ? '' : next === 'number' ? '0' : next === 'boolean' ? 'false' : 'null'
  }
  emitAll()
}

function rename(node: JsonTreeNode, key: string): void {
  node.key = key
  emitAll()
}

function setDesc(node: JsonTreeNode, desc: string): void {
  node.description = desc
  emitAll()
}

function setRequired(node: JsonTreeNode, required: boolean): void {
  node.required = required
  emitAll()
}

function setValue(node: JsonTreeNode, value: string): void {
  node.value = value
  emitAll()
}

function isContainer(node: JsonTreeNode): boolean {
  return node.type === 'object' || node.type === 'array'
}

function isScalar(node: JsonTreeNode): boolean {
  return node.type === 'string' || node.type === 'number' || node.type === 'boolean'
}

const canAdd = computed(() => isContainer(root.value) || root.value.type === 'object')
</script>

<template>
  <div class="jse" :class="{ disabled }">
    <div class="jse-toolbar">
      <button type="button" class="jse-add" :disabled="disabled || !canAdd" @click="addChild(root)">
        {{ t('design.addField') }}
      </button>
    </div>

    <div class="jse-tree">
      <div v-if="!rows.length" class="jse-empty">
        <EmptyState icon="list" :title="t('design.addField')" compact />
      </div>
      <div
        v-for="row in rows"
        :key="row.node.nid"
        class="jse-line"
        :style="{ paddingLeft: `${8 + row.depth * 18}px` }"
      >
        <input
          class="jse-key mono"
          :value="row.node.key"
          :placeholder="t('design.fieldKeyPh')"
          :disabled="disabled"
          spellcheck="false"
          @input="rename(row.node, ($event.target as HTMLInputElement).value)"
        />
        <select
          class="jse-type"
          :value="row.node.type"
          :disabled="disabled"
          @change="changeType(row.node, ($event.target as HTMLSelectElement).value as JsonTreeType)"
        >
          <option v-for="opt in TYPE_OPTIONS" :key="opt.value" :value="opt.value">
            {{ opt.label }}
          </option>
        </select>
        <label class="jse-req" :title="t('paramtable.required')">
          <input
            type="checkbox"
            :checked="row.node.required"
            :disabled="disabled"
            :aria-label="t('paramtable.required')"
            @change="setRequired(row.node, ($event.target as HTMLInputElement).checked)"
          />
        </label>
        <input
          class="jse-desc"
          :value="row.node.description"
          :placeholder="t('design.fieldDescPh')"
          :disabled="disabled"
          spellcheck="false"
          @input="setDesc(row.node, ($event.target as HTMLInputElement).value)"
        />
        <input
          v-if="isScalar(row.node)"
          class="jse-value mono"
          :value="row.node.value"
          :placeholder="t('design.fieldExamplePh')"
          :disabled="disabled"
          spellcheck="false"
          @input="setValue(row.node, ($event.target as HTMLInputElement).value)"
        />
        <IconButton
          v-if="isContainer(row.node)"
          name="plus"
          :size="13"
          :disabled="disabled"
          :title="t('design.addField')"
          @click="addChild(row.node)"
        />
        <IconButton
          name="trash"
          :size="13"
          tone="danger"
          :disabled="disabled"
          :title="t('paramtable.deleteParam')"
          @click="removeAt(row.parent, row.index)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.jse {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.jse.disabled {
  opacity: 0.6;
}

.jse-toolbar {
  display: flex;
  justify-content: flex-start;
}

.jse-add {
  padding: 3px 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--accent);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.jse-add:hover:not(:disabled) {
  background: var(--accent-tint);
}
.jse-add:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.jse-tree {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.jse-empty {
  padding: 8px 0;
}

.jse-line {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-top: 3px;
  padding-bottom: 3px;
  padding-right: 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.jse-line:hover {
  border-color: var(--border);
  background: var(--bg-hover);
}

.jse-key {
  width: 130px;
  flex-shrink: 0;
  height: 26px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-1);
  font-size: 12px;
}
.jse-key:hover:not(:disabled) {
  border-color: var(--border);
}
.jse-key:focus {
  outline: none;
  border-color: var(--accent);
  background: var(--bg-code);
}
.jse-key::placeholder {
  color: var(--text-3);
}

.jse-type {
  width: 78px;
  flex-shrink: 0;
  height: 26px;
  padding: 0 4px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  cursor: pointer;
}
.jse-type:hover:not(:disabled) {
  border-color: var(--border);
}
.jse-type:focus {
  outline: none;
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.jse-req {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  cursor: pointer;
}
.jse-req input {
  width: 14px;
  height: 14px;
  accent-color: var(--accent);
  cursor: pointer;
}

.jse-desc {
  flex: 1;
  min-width: 80px;
  height: 26px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-1);
  font-family: inherit;
  font-size: 12px;
}
.jse-desc:hover:not(:disabled) {
  border-color: var(--border);
}
.jse-desc:focus {
  outline: none;
  border-color: var(--accent);
  background: var(--bg-code);
}
.jse-desc::placeholder {
  color: var(--text-3);
}

.jse-value {
  width: 110px;
  flex-shrink: 0;
  height: 26px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-size: 12px;
}
.jse-value:hover:not(:disabled) {
  border-color: var(--border);
}
.jse-value:focus {
  outline: none;
  border-color: var(--accent);
  background: var(--bg-code);
}
.jse-value::placeholder {
  color: var(--text-3);
}
</style>
