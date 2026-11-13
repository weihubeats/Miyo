<script setup lang="ts">
/**
 * KeyValueTable：键/值/描述 三列（可选启用列）表格。
 * - 无幽灵占位行：空表即空表，新增走底部「+ 添加」显式入口（空行占位曾
 *   被用户误读为禁用数据行）；行内输入的行失焦若仍为空则自动移除；
 * - 键盘流：Tab 天然从 Key → Value；Value 内按 Enter / Tab 跳到下一行 Key，
 *   处于末尾时自动补行并聚焦（快速连续录入）；
 * - `searchable` 时顶部工具栏提供过滤条 + #toolbar-actions 插槽
 *   （调用方放批量编辑 / 抽取等同级操作，避免两层工具栏）。
 */
import { computed, nextTick, ref, watch, type ComponentPublicInstance } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import { useVarAutocomplete } from '../../composables/useVarAutocomplete'
import { rowKey } from '../../utils/rowKey'
import IconButton from './IconButton.vue'
import Icon from './Icon.vue'
import VarSuggest from './VarSuggest.vue'

export interface KVRow {
  key?: string
  value?: string
  enabled?: boolean
  description?: string
}

const props = withDefaults(
  defineProps<{
    modelValue: KVRow[]
    showEnable?: boolean
    showDescription?: boolean
    /** 缺省时在模板里用 t() 兜底（defineProps 默认值不能调 t()）。 */
    keyPlaceholder?: string
    valuePlaceholder?: string
    descriptionPlaceholder?: string
    disabled?: boolean
    /** 自定义列宽（key / value / description 百分比）。缺省时使用 flex 自适应布局。 */
    columnWidths?: [string, string, string]
    /**
     * Value 列 {{变量}} 自动补全候选；不传则不启用（保持既有调用方行为不变）。
     * 由调用方传 computed（如 useVarCandidates），变量增删后候选即时刷新。
     */
    varCandidates?: string[]
    /** 顶部过滤框：按 key / value / 描述模糊过滤行（过滤只影响显示，不改数据）。 */
    searchable?: boolean
    /** 底部「+ 添加」按钮文案（缺省用 kv.addRow 兜底）。 */
    addLabel?: string
  }>(),
  {
    showEnable: true,
    showDescription: true,
    descriptionPlaceholder: '',
    disabled: false,
    searchable: false,
  },
)

const emit = defineEmits<{ 'update:modelValue': [rows: KVRow[]] }>()

const locale = useLocaleStore()
const t = locale.t

/** Value 列 {{变量}} 补全（未传 varCandidates 时候选为空 → onInput 永不展开）。 */
const varAc = useVarAutocomplete(computed(() => props.varCandidates ?? []))
const { open: varAcOpen, items: varAcItems, activeIndex: varAcIndex, anchor: varAcAnchor } = varAc

function onValueAcInput(event: Event): void {
  onCellInput()
  varAc.onInput(event.target as HTMLInputElement)
}

function onValueAcPick(index: number): void {
  const el = varAcAnchor.value
  if (el instanceof HTMLInputElement) varAc.pick(el, index)
}

/** Value 列失焦：清空中间空行 + 收起补全弹层（点弹层候选项时因 mousedown.prevent 不会触发）。 */
function onValueBlur(row: KVRow): void {
  onCellBlur(row)
  varAc.close()
}

/** 未传 key / value / 描述列 placeholder 时按当前语言兜底。 */
const effectiveKeyPh = computed(() => props.keyPlaceholder || t('kv.colKey'))
const effectiveValuePh = computed(() => props.valuePlaceholder || t('kv.colValue'))
/** 未传描述列 placeholder 时按当前语言兜底。 */
const effectiveDescPh = computed(() => props.descriptionPlaceholder || t('kv.descPh'))

const rows = ref<KVRow[]>([])
const keyInputs = new Map<number, HTMLInputElement>()

function blank(): KVRow {
  return { key: '', value: '', enabled: true, description: '', }
}

function isEmpty(row: KVRow): boolean {
  return !row.key && !row.value
}

/** 上抛内容指纹：四列内容拼串，内容没变就不重复上抛（省掉父级无谓整表回写）。 */
function contentFingerprint(list: KVRow[]): string {
  let out = ''
  for (const r of list) {
    out += `${r.key ?? ''}\u0000${r.value ?? ''}\u0000${r.enabled ? 1 : 0}\u0000${r.description ?? ''}\u0001`
  }
  return out
}

let lastEmittedFp: string | null = null

function rebuild(v: KVRow[] | undefined): void {
  // 不追加幽灵行：空表保持空表，「+ 添加」是唯一的新行入口
  rows.value = [...(v ?? [])]
  // 外部结构变化后，下一次输入必须重新上抛（本地指纹基线失效）。
  lastEmittedFp = null
}

/**
 * 结构指纹 watch（原 deep watch）：
 *
 * - deep watch 把每次键入（v-model 就地写行字段）都算作变更，触发整表重建 +
 *   父级回写 + 重渲染，键入一路 O(行数×列数) 地跑；
 * - 输入框绑定的本就是行对象属性，字段级的外部改动由渲染副作用自行更新 DOM，
 *   不需要重建行数组；
 * - 这里只跟踪「行对象身份」：换引用 / 增删行 / 整组换对象才重建。
 *   用指纹字符串而非直接 watch 数组，是为避免 `?? []` 这类每次求值返回新数组
 *   的取值器造成「引用永远在变」的重建风暴。
 */
watch(
  () => {
    const list = props.modelValue
    if (!list?.length) return String(list?.length ?? -1)
    let fp = String(list.length)
    for (let i = 0; i < list.length; i++) fp += `|${rowKey(list[i])}`
    return fp
  },
  () => rebuild(props.modelValue),
  { immediate: true },
)

/** 上抛：过滤全部空行（无幽灵行，空表上抛空数组）；内容未变时跳过。 */
function sync(): void {
  const list = rows.value.filter((r) => r.key || r.value)
  const fp = contentFingerprint(list)
  if (fp === lastEmittedFp) return
  lastEmittedFp = fp
  emit('update:modelValue', list)
}

/** 末尾非空则补一行（Value 内 Enter / Tab 跳行的快速录入路径）。 */
function ensureTail(): void {
  const last = rows.value[rows.value.length - 1]
  if (!last || !isEmpty(last)) rows.value.push(blank())
}

/** 显式添加一行并聚焦其 Key 输入框（唯一的新行入口）。 */
async function addRow(): Promise<void> {
  rows.value.push(blank())
  lastEmittedFp = null
  sync()
  await nextTick()
  keyInputs.get(rows.value.length - 1)?.focus()
}

function remove(index: number): void {
  rows.value.splice(index, 1)
  sync()
}

function onCellInput(): void {
  sync()
}

function onCellBlur(row: KVRow): void {
  const i = rows.value.indexOf(row)
  if (i === -1) return
  // 空行（键值皆空）失焦即移除：不留误导性的空白占位行
  if (isEmpty(row)) {
    rows.value.splice(i, 1)
    sync()
  }
}

function setKeyRef(i: number): (ref: Element | ComponentPublicInstance | null) => void {
  return (ref) => {
    const el = ref instanceof HTMLInputElement ? ref : null
    if (el) keyInputs.set(i, el)
    else keyInputs.delete(i)
  }
}

/** Key/Value/描述共用：仅 Value 内 Enter/Tab 跳下一行 Key；Key 内 Tab 走默认顺序（→Value）。 */
function onKeydown(event: KeyboardEvent, i: number): void {
  // 补全弹层打开时接管 ↑↓/Enter/Tab/Esc（Enter 插入变量而非跳行）
  if (varAc.handleKeydown(event, event.target as HTMLInputElement)) return
  if (!(event.key === 'Enter' || event.key === 'Tab')) return
  if (!(event.target as HTMLElement).classList.contains('kvt-value')) return
  event.preventDefault()
  ensureTail()
  void nextTick(() => {
    const el = keyInputs.get(i + 1)
    if (!el) return
    el.focus()
    if (!el.value) el.select()
  })
}
/** grid 模式：启用复选框 + 操作列固定宽，key/value/desc 按百分比精确分配。 */
function gridTemplate(): string | undefined {
  if (!props.columnWidths) return undefined
  const [k, v, d] = props.columnWidths
  const enable = props.showEnable ? '36px' : '0px'
  const desc = props.showDescription ? d : '0px'
  const actions = '32px'
  return `${enable} ${k} ${v} ${desc} ${actions}`
}

function gridStyle(): Record<string, string> | undefined {
  const cols = gridTemplate()
  return cols ? { gridTemplateColumns: cols, display: 'grid' } : undefined
}

// ---------- 行过滤（searchable）：只影响渲染，不改 rows 数据 ----------
const filter = ref('')

/** 命中的行（携带原索引，编辑 / 删除 / 跳格仍按原始行号工作）。 */
const visibleItems = computed<{ i: number; row: KVRow }[]>(() => {
  const q = filter.value.trim().toLowerCase()
  const out: { i: number; row: KVRow }[] = []
  if (!q) {
    for (let i = 0; i < rows.value.length; i++) out.push({ i, row: rows.value[i] })
    return out
  }
  for (let i = 0; i < rows.value.length; i++) {
    const row = rows.value[i]
    const hay = `${row.key ?? ''}\n${row.value ?? ''}\n${row.description ?? ''}`.toLowerCase()
    if (hay.includes(q)) out.push({ i, row })
  }
  return out
})

function clearFilter(): void {
  filter.value = ''
}
</script>

<template>
  <div class="kvt" :class="{ disabled }">
    <div v-if="searchable" class="kvt-search">
      <Icon name="search" :size="12" class="kvt-search-icon" />
      <input
        v-model="filter"
        class="kvt-search-input"
        :placeholder="t('kv.searchPh')"
        spellcheck="false"
      />
      <button v-if="filter" type="button" class="kvt-search-clear" :title="t('common.clearSearch')" @click="clearFilter">
        <Icon name="x" :size="11" />
      </button>
      <!-- 同级操作入口（批量编辑 / 抽取等），与过滤条同一工具栏 -->
      <slot name="toolbar-actions"></slot>
    </div>

    <div class="kvt-head" :style="gridStyle()">
      <span v-if="showEnable" class="kvt-col kvt-enable"></span>
      <span class="kvt-col kvt-key miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colKey') }}</span>
      <span class="kvt-col kvt-value miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colValue') }}</span>
      <span v-if="showDescription" class="kvt-col kvt-desc miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colDesc') }}</span>
      <span class="kvt-col kvt-actions"></span>
    </div>

    <div
      v-for="entry in visibleItems"
      :key="rowKey(entry.row)"
      class="kvt-row"
      :class="{ off: showEnable && entry.row.enabled === false }"
      :style="gridStyle()"
    >
      <span v-if="showEnable" class="kvt-col kvt-enable">
        <input
          v-model="entry.row.enabled"
          type="checkbox"
          class="kvt-check"
          :disabled="disabled"
          @change="sync"
        />
      </span>
      <input
        v-model="entry.row.key"
        class="kvt-input kvt-col kvt-key miyo-mono"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveKeyPh"
        :disabled="disabled"
        spellcheck="false"
        :ref="setKeyRef(entry.i)"
        @input="onCellInput"
        @keydown="onKeydown($event, entry.i)"
        @blur="onCellBlur(entry.row)"
      />
      <input
        v-model="entry.row.value"
        class="kvt-input kvt-col kvt-value miyo-mono"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveValuePh"
        :disabled="disabled"
        spellcheck="false"
        @input="onValueAcInput"
        @keydown="onKeydown($event, entry.i)"
        @blur="onValueBlur(entry.row)"
      />
      <input
        v-if="showDescription"
        v-model="entry.row.description"
        class="kvt-input kvt-col kvt-desc"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveDescPh"
        :disabled="disabled"
        spellcheck="false"
        @input="onCellInput"
        @keydown="onKeydown($event, entry.i)"
        @blur="onCellBlur(entry.row)"
      />
      <span class="kvt-col kvt-actions">
        <IconButton
          name="trash"
          :size="13"
          tone="danger"
          :title="t('common.delete')"
          @click="remove(entry.i)"
        />
      </span>
    </div>

    <div v-if="filter && !visibleItems.length" class="kvt-empty">{{ t('kv.noMatch') }}</div>

    <!-- 显式添加入口：空表 / 过滤中亦可添加 -->
    <button v-if="!disabled" class="kvt-add" type="button" @click="addRow">
      <Icon name="plus" :size="12" />
      {{ addLabel || t('kv.addRow') }}
    </button>

    <VarSuggest
      v-if="varAcOpen && varAcAnchor"
      :anchor="varAcAnchor"
      :items="varAcItems"
      :active-index="varAcIndex"
      @pick="onValueAcPick"
    />
  </div>
</template>

<style scoped>
.kvt {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
.kvt.disabled {
  opacity: 0.5;
  pointer-events: none;
}

/* 顶部过滤条（searchable）：只过滤显示，不改数据 */
.kvt-search {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-code);
  color: var(--text-3);
}
.kvt-search-icon {
  flex-shrink: 0;
  opacity: 0.7;
}
.kvt-search-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-xs);
  font-family: inherit;
  outline: none;
  padding: 0;
}
.kvt-search-input::placeholder {
  color: var(--text-3);
}
/* 工具栏插槽内容（批量编辑等按钮）不参与压缩 */
.kvt-search :slotted(*) {
  flex-shrink: 0;
}
.kvt-search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.kvt-search-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.kvt-empty {
  padding: 14px 8px;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* 显式添加入口：整宽弱按钮（与历史「加载更多」同语言），补齐幽灵行移除后的新行路径 */
.kvt-add {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  width: 100%;
  height: 26px;
  border: none;
  border-top: 1px dashed var(--border);
  background: none;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-3);
  cursor: pointer;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.kvt-add:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.kvt-head {
  display: flex;
  align-items: center;
  height: 28px;
  border-bottom: 1px solid var(--border);
  /* 表头带走 --bg-code（slate-50/深 #121318）：--bg-panel 改白后与数据行
     同色仅剩 border-bottom，恢复「白卡内浅灰表头带」的层次 */
  background: var(--bg-code);
  font-size: var(--fs-xxs);
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-3);
}

.kvt-row {
  display: flex;
  align-items: center;
  min-height: 28px;
  border-bottom: 1px solid var(--border);
  transition: background var(--dur) var(--ease);
}

.kvt-col {
  flex-shrink: 0;
}
.kvt-enable {
  width: 36px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
/* 三列统一节奏：key 30% / value 弹性 / 描述 24%（value 是主编辑列占最大） */
.kvt-key {
  width: 30%;
  min-width: 110px;
}
.kvt-value {
  flex: 1;
  min-width: 0;
}
.kvt-desc {
  width: 24%;
  min-width: 110px;
}
.kvt-actions {
  width: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
}

.kvt-row:last-child {
  border-bottom: none;
}
.kvt-row:hover {
  background: var(--bg-hover);
}
.kvt-row.off .kvt-input {
  opacity: 0.45;
}

.kvt-check {
  accent-color: var(--accent);
  cursor: pointer;
}

.kvt-input {
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-1);
  font-size: 12px;
  outline: none;
  padding: 0 8px;
  min-width: 0;
}
.kvt-input::placeholder {
  color: var(--text-3);
}
.kvt-input:focus {
  background: var(--bg-elevated);
  box-shadow: inset 0 0 0 1px var(--accent);
}

.kvt-actions {
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}
.kvt-row:hover .kvt-actions,
.kvt-row:focus-within .kvt-actions {
  opacity: 1;
}
</style>