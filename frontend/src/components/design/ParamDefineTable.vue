<script setup lang="ts">
/**
 * ParamDefineTable：接口设计页的参数定义表。
 *
 * - 列：参数名 | 类型 | 必填 | 启用 | 说明 | 示例值 | 操作（删除）；
 * - 行内直接编辑，修改以整组新数组回调（update:modelValue），由父级写回草稿；
 * - 「+ 添加参数」在末尾追加空行（enabled 默认开启）；
 * - 类型下拉为原生 select 紧凑样式（行内控件不引 CustomSelect，避免每行浮层开销）。
 */
import { useLocaleStore } from '../../stores/locale'
import { rowKey } from '../../utils/rowKey'
import IconButton from '../ui/IconButton.vue'
import EmptyState from '../ui/EmptyState.vue'
import type { FieldType, KeyValue } from '../../types/miyoApi'

const locale = useLocaleStore()
const t = locale.t

const props = withDefaults(
  defineProps<{
    rows: KeyValue[]
    keyPlaceholder?: string
    /** 是否展示「示例值」列（Body 表单等场景可关）。 */
    showExample?: boolean
  }>(),
  // defineProps 默认值会被提升到 setup 之外，不能引用 t()；占位符在模板兜底取词。
  { showExample: true },
)

const emit = defineEmits<{ 'update:modelValue': [rows: KeyValue[]] }>()

const FIELD_TYPES: { value: FieldType; label: string }[] = [
  { value: 'string', label: 'String' },
  { value: 'number', label: 'Number' },
  { value: 'boolean', label: 'Boolean' },
  { value: 'object', label: 'Object' },
]

function typeOf(row: KeyValue): FieldType {
  return row.field_type ?? 'string'
}
function requiredOf(row: KeyValue): boolean {
  return row.required ?? true
}
function enabledOf(row: KeyValue): boolean {
  return row.enabled ?? true
}

/**
 * 单元格修改：就地更新该行对象 + 整组回传新数组。
 * 就地写保住了行对象身份——`:key="rowKey(row)"` 才能稳定，键入时行 DOM 不被
 * 重建（原实现每次键入 `{...row}` 浅拷贝回传，换稳定 key 会让输入框丢焦点）；
 * 回传新数组仍走父级 writeRows，下游拿到的更新链路不变。
 */
function patch(index: number, part: Partial<KeyValue>): void {
  const row = props.rows[index]
  if (!row) return
  Object.assign(row, part)
  emit('update:modelValue', [...props.rows])
}

function addRow(): void {
  emit('update:modelValue', [
    ...props.rows,
    {
      key: '',
      value: '',
      enabled: true,
      description: '',
      field_type: 'string',
      required: true,
      example: '',
    },
  ])
}

function removeRow(index: number): void {
  emit(
    'update:modelValue',
    props.rows.filter((_, i) => i !== index),
  )
}
</script>

<template>
  <div class="pdt">
    <div class="pdt-scroll">
      <table class="pdt-table">
        <thead>
          <tr>
            <th class="col-key">{{ t('paramtable.colKey') }}</th>
            <th class="col-type">{{ t('body.colType') }}</th>
            <th class="col-req">{{ t('paramtable.required') }}</th>
            <th class="col-en">{{ t('paramtable.enabled') }}</th>
            <th class="col-desc">{{ t('paramtable.colDesc') }}</th>
            <th v-if="showExample" class="col-example">{{ t('paramtable.colExample') }}</th>
            <th class="col-op"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, i) in rows" :key="rowKey(row)">
            <td class="col-key">
              <input
                class="pdt-input mono"
                :value="row.key"
                :placeholder="keyPlaceholder ?? t('paramtable.colKey')"
                spellcheck="false"
                @input="patch(i, { key: ($event.target as HTMLInputElement).value })"
              />
            </td>
            <td class="col-type">
              <select
                class="pdt-select"
                :value="typeOf(row)"
                @change="
                  patch(i, {
                    field_type: ($event.target as HTMLSelectElement).value as FieldType,
                  })
                "
              >
                <option v-for="t in FIELD_TYPES" :key="t.value" :value="t.value">{{ t.label }}</option>
              </select>
            </td>
            <td class="col-req">
              <input
                class="pdt-check"
                type="checkbox"
                :checked="requiredOf(row)"
                :aria-label="t('paramtable.required')"
                @change="
                  patch(i, { required: ($event.target as HTMLInputElement).checked })
                "
              />
            </td>
            <td class="col-en">
              <input
                class="pdt-check"
                type="checkbox"
                :checked="enabledOf(row)"
                :aria-label="t('paramtable.enabled')"
                @change="
                  patch(i, { enabled: ($event.target as HTMLInputElement).checked })
                "
              />
            </td>
            <td class="col-desc">
              <input
                class="pdt-input"
                :value="row.description"
                :placeholder="t('paramtable.descPh')"
                spellcheck="false"
                @input="patch(i, { description: ($event.target as HTMLInputElement).value })"
              />
            </td>
            <td v-if="showExample" class="col-example">
              <input
                class="pdt-input mono"
                :value="row.example ?? ''"
                :placeholder="t('paramtable.colExample')"
                spellcheck="false"
                @input="patch(i, { example: ($event.target as HTMLInputElement).value })"
              />
            </td>
            <td class="col-op">
              <IconButton name="trash" :size="13" tone="danger" :title="t('paramtable.deleteParam')" @click="removeRow(i)" />
            </td>
          </tr>
          <tr v-if="!rows.length">
            <td :colspan="showExample ? 7 : 6" class="pdt-empty">
              <EmptyState icon="list" :title="t('paramtable.empty')" compact />
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <button type="button" class="pdt-add" @click="addRow">{{ t('paramtable.add') }}</button>
  </div>
</template>

<style scoped>
.pdt {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.pdt-scroll {
  overflow-x: auto;
}

.pdt-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
}

.pdt-table th {
  padding: 6px 8px;
  text-align: left;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
  border-bottom: 1px solid var(--border-strong);
  white-space: nowrap;
}

.pdt-table td {
  padding: 4px 6px;
  border-bottom: 1px solid var(--border);
  vertical-align: middle;
}

.pdt-table tbody tr:hover td {
  background: var(--bg-hover);
}

.col-key {
  width: 22%;
}
.col-type {
  width: 12%;
}
.col-req {
  width: 6%;
  text-align: center;
}
th.col-req {
  text-align: center;
}
.col-en {
  width: 6%;
  text-align: center;
}
th.col-en {
  text-align: center;
}
.col-desc {
  width: 24%;
}
.col-example {
  width: 20%;
}
.col-op {
  width: 34px;
  text-align: center;
}

/* 行内输入：无边框融入表格，聚焦浮现描边 */
.pdt-input {
  width: 100%;
  height: 26px;
  padding: 0 7px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-1);
  font-family: inherit;
  font-size: 12px;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.pdt-input.mono {
  font-family: var(--font-mono);
}
.pdt-input::placeholder {
  color: var(--text-3);
}
.pdt-input:hover {
  border-color: var(--border);
}
.pdt-input:focus {
  outline: none;
  border-color: var(--accent);
  background: var(--bg-code);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.pdt-select {
  width: 100%;
  height: 26px;
  padding: 0 4px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: border-color var(--dur) var(--ease);
}
.pdt-select:hover {
  border-color: var(--border);
}
.pdt-select:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.pdt-check {
  width: 14px;
  height: 14px;
  accent-color: var(--accent);
  cursor: pointer;
}

.pdt-empty {
  padding: 14px 8px;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
}

.pdt-add {
  align-self: flex-start;
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
.pdt-add:hover {
  background: var(--accent-tint);
}
.pdt-add:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
</style>
