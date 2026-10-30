<script setup lang="ts">
/**
 * SchemaTreeTable：API 文档标准的树状参数表格（文档预览用）。
 *
 * - 列：字段名 | 类型 | 必填 | 说明（示例值）；
 * - 嵌套 object / object 数组可展开折叠（chevron + 缩进层级）；
 * - 类型 Badge 按语义着色：string 柔和绿 / object 蓝 / number 黄 / boolean 紫 / array 紫罗兰；
 * - 数据来自 schemaInfer（样本推断），「必填」由调用方语义决定（默认 true）。
 */
import { computed, ref, watch } from 'vue'
import Icon from '../ui/Icon.vue'
import { useLocaleStore } from '../../stores/locale'
import type { SchemaRow, SchemaType } from '../../utils/schemaInfer'
import { typeLabelOf } from '../../utils/schemaInfer'

const locale = useLocaleStore()
const t = locale.t

const props = withDefaults(
  defineProps<{
    rows: SchemaRow[]
    /** 初始展开到第几层（根字段为 0 层，默认展开一层嵌套）。 */
    defaultExpandDepth?: number
  }>(),
  { defaultExpandDepth: 1 },
)

/** 类型 → Badge 语义类（scoped CSS 里定义配色）。 */
const TYPE_CLASS: Record<SchemaType, string> = {
  string: 't-string',
  number: 't-number',
  boolean: 't-boolean',
  object: 't-object',
  array: 't-array',
  null: 't-null',
}

/** 展开状态（key 为扁平化路径，含索引防重键；数据切换时重置为默认展开层级）。 */
const expanded = ref<Set<string>>(new Set())

/** 行的唯一 key（与可见行扁平化保持同一算法）。 */
function keyOf(parentKey: string, index: number, row: SchemaRow): string {
  return `${parentKey}/${index}:${row.name}`
}

/** 按 defaultExpandDepth 初始化展开集合。 */
function resetExpanded(rows: SchemaRow[]): void {
  const out = new Set<string>()
  const walk = (list: SchemaRow[], parentKey: string): void => {
    list.forEach((row, i) => {
      const key = keyOf(parentKey, i, row)
      if (row.children.length > 0 && row.depth < props.defaultExpandDepth) {
        out.add(key)
        walk(row.children, key)
      }
    })
  }
  walk(rows, '')
  expanded.value = out
}

watch(
  () => props.rows,
  (rows) => resetExpanded(rows),
  { immediate: true },
)

/** 可见行（DFS：折叠的分支跳过 children）。 */
const visibleRows = computed(() => {
  const out: Array<{ key: string; row: SchemaRow; expandable: boolean; open: boolean }> = []
  const walk = (rows: SchemaRow[], parentKey: string): void => {
    rows.forEach((row, i) => {
      const key = keyOf(parentKey, i, row)
      const expandable = row.children.length > 0
      const open = expanded.value.has(key)
      out.push({ key, row, expandable, open })
      if (open && expandable) walk(row.children, key)
    })
  }
  walk(props.rows, '')
  return out
})

function toggle(key: string, expandable: boolean): void {
  if (!expandable) return
  const next = new Set(expanded.value)
  if (next.has(key)) {
    next.delete(key)
  } else {
    next.add(key)
  }
  expanded.value = next
}
</script>

<template>
  <div class="stt">
    <table class="stt-table">
      <thead>
        <tr>
          <th class="col-field">{{ t('schema.colField') }}</th>
          <th class="col-type">{{ t('schema.colType') }}</th>
          <th class="col-required">{{ t('schema.colRequired') }}</th>
          <th class="col-desc">{{ t('schema.colDesc') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in visibleRows" :key="item.key" class="stt-row">
          <td class="col-field">
            <div
              class="field-cell"
              :class="{ clickable: item.expandable }"
              :style="{ paddingLeft: `${8 + item.row.depth * 18}px` }"
              @click="toggle(item.key, item.expandable)"
            >
              <Icon
                v-if="item.expandable"
                :name="item.open ? 'chevron-down' : 'chevron-right'"
                :size="12"
                class="field-caret"
              />
              <span v-else class="field-caret field-dot"></span>
              <code class="field-name" :class="{ 'field-parent': item.expandable }">
                {{ item.row.name || 'items[]' }}
              </code>
            </div>
          </td>
          <td class="col-type">
            <span class="type-badge" :class="TYPE_CLASS[item.row.type]">
              {{ typeLabelOf(item.row) }}
            </span>
          </td>
          <td class="col-required">
            <span v-if="item.row.required" class="req-yes">{{ t('schema.colRequired') }}</span>
            <span v-else class="req-no">—</span>
          </td>
          <td class="col-desc">
            <code v-if="item.row.example" class="desc-example" v-tooltip-overflow>
              {{ item.row.example }}
            </code>
            <span v-else-if="item.expandable" class="desc-hint">
              {{ item.row.type === 'array' ? t('schema.arrayItem') : t('schema.nestedObject') }}
            </span>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.stt {
  overflow-x: auto;
}

.stt-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
}

.stt-table th {
  padding: 6px 8px;
  text-align: left;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
  /* 表头分隔线：比行分隔线更重一档，划分字段区与数据区 */
  border-bottom: 1px solid var(--border-strong);
  white-space: nowrap;
}

.stt-row td {
  padding: 4px 8px;
  border-bottom: 1px solid var(--border);
  color: var(--text-2);
  vertical-align: middle;
}

/* 末行不画分隔线（卡片内自带头部分隔） */
.stt-row:last-child td {
  border-bottom: none;
}

.stt-row:hover td {
  background: var(--bg-hover);
}

.col-field {
  width: 34%;
}
.col-type {
  width: 17%;
}
.col-required {
  width: 9%;
  white-space: nowrap;
}
.col-desc {
  color: var(--text-3);
}

.field-cell {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}
.field-cell.clickable {
  cursor: pointer;
}
.field-cell.clickable:hover .field-name {
  color: var(--text-1);
}

.field-caret {
  flex-shrink: 0;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease), color var(--dur) var(--ease);
}
.field-cell.clickable:hover .field-caret {
  color: var(--text-2);
}

/* 叶子行的占位圆点（与 caret 对齐） */
.field-dot {
  width: 12px;
  height: 12px;
  display: inline-block;
}
.field-dot::before {
  content: '';
  display: block;
  width: 3px;
  height: 3px;
  margin: 4.5px auto;
  border-radius: 50%;
  background: var(--text-3);
}

.field-name {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.field-parent {
  font-weight: 600;
}

/* ---- 类型 Badge：string 柔和绿 / object 蓝 / number 黄 / boolean 紫 / array 紫罗兰 ---- */
.type-badge {
  display: inline-block;
  padding: 2px 6px;
  border-radius: 4px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.6;
  border: 1px solid transparent;
  white-space: nowrap;
}
.t-string {
  color: var(--success);
  background: var(--success-tint);
  border-color: color-mix(in srgb, var(--success) 22%, transparent);
}
.t-object {
  color: var(--info);
  background: var(--info-tint);
  border-color: color-mix(in srgb, var(--info) 22%, transparent);
}
.t-number {
  color: var(--warning);
  background: var(--warning-tint);
  border-color: color-mix(in srgb, var(--warning) 22%, transparent);
}
.t-boolean {
  color: var(--accent);
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 22%, transparent);
}
.t-array {
  color: var(--patch);
  background: var(--patch-tint);
  border-color: color-mix(in srgb, var(--patch) 22%, transparent);
}
.t-null {
  color: var(--text-3);
  background: var(--bg-hover);
}

.req-yes {
  display: inline-block;
  padding: 0 6px;
  border-radius: 4px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  background: var(--warning-tint);
}
.req-no {
  color: var(--text-3);
}

.desc-example {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: block;
  max-width: 320px;
}
.desc-hint {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
</style>
