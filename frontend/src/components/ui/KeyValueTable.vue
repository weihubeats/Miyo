<script setup lang="ts">
/**
 * KeyValueTable：键/值/描述 三列（可选启用列）表格，Postman 式自动追加行。
 * - 末尾常驻一条空白「幽灵行」（弱化显示）：在其中任一单元格输入即自动
 *   成为正式行，并立即在下方补齐新的空白行，无需点「+ 添加」；
 * - 行删除按钮仅对正式行显示（悬停显现）；中间空行失焦自动移除，只保留底部一条幽灵行；
 * - 键盘流：Tab 天然从 Key → Value；Value 内按 Enter / Tab 跳到下一行 Key，
 *   处于末尾时自动补行并聚焦。
 */
import { computed, nextTick, ref, watch, type ComponentPublicInstance } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import { useVarAutocomplete } from '../../composables/useVarAutocomplete'
import { rowKey } from '../../utils/rowKey'
import IconButton from './IconButton.vue'
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
  }>(),
  {
    showEnable: true,
    showDescription: true,
    descriptionPlaceholder: '',
    disabled: false,
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
  return { key: '', value: '', enabled: true, description: '' }
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
  const list = [...(v ?? [])]
  const last = list[list.length - 1]
  if (!last || !isEmpty(last)) list.push(blank())
  rows.value = list
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

/** 上抛：过滤掉中间的空行（保留底部幽灵行）；内容未变时跳过。 */
function sync(): void {
  const list = rows.value.filter((r, i) => i === rows.value.length - 1 || r.key || r.value)
  const fp = contentFingerprint(list)
  if (fp === lastEmittedFp) return
  lastEmittedFp = fp
  emit('update:modelValue', list)
}

function isGhost(row: KVRow): boolean {
  return isEmpty(row) && row === rows.value[rows.value.length - 1]
}

function ensureTail(): void {
  const last = rows.value[rows.value.length - 1]
  // 就地 push：ref 数组的 push 本身即触发重渲染，无需整体换新数组引用
  //（换引用会让每次键入都触发整表 v-for 重摊一遍）。
  if (!isEmpty(last)) rows.value.push(blank())
}

function remove(index: number): void {
  rows.value.splice(index, 1)
  sync()
}

function onCellInput(): void {
  ensureTail()
  sync()
}

function onCellBlur(row: KVRow): void {
  const i = rows.value.indexOf(row)
  if (i === -1 || i === rows.value.length - 1) return
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
</script>

<template>
  <div class="kvt" :class="{ disabled }">
    <div class="kvt-head" :style="gridStyle()">
      <span v-if="showEnable" class="kvt-col kvt-enable"></span>
      <span class="kvt-col kvt-key miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colKey') }}</span>
      <span class="kvt-col kvt-value miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colValue') }}</span>
      <span v-if="showDescription" class="kvt-col kvt-desc miyo-mono" :style="gridStyle() ? { width: '100%' } : undefined">{{ t('kv.colDesc') }}</span>
      <span class="kvt-col kvt-actions"></span>
    </div>

    <div
      v-for="(row, i) in rows"
      :key="rowKey(row)"
      class="kvt-row"
      :class="{ off: showEnable && row.enabled === false, ghost: isGhost(row) }"
      :style="gridStyle()"
    >
      <span v-if="showEnable" class="kvt-col kvt-enable">
        <input
          v-model="row.enabled"
          type="checkbox"
          class="kvt-check"
          :disabled="disabled || isGhost(row)"
          @change="sync"
        />
      </span>
      <input
        v-model="row.key"
        class="kvt-input kvt-col kvt-key miyo-mono"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveKeyPh"
        :disabled="disabled"
        spellcheck="false"
        :ref="setKeyRef(i)"
        @input="onCellInput"
        @keydown="onKeydown($event, i)"
        @blur="onCellBlur(row)"
      />
      <input
        v-model="row.value"
        class="kvt-input kvt-col kvt-value miyo-mono"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveValuePh"
        :disabled="disabled"
        spellcheck="false"
        @input="onValueAcInput"
        @keydown="onKeydown($event, i)"
        @blur="onValueBlur(row)"
      />
      <input
        v-if="showDescription"
        v-model="row.description"
        class="kvt-input kvt-col kvt-desc"
        :style="gridStyle() ? { width: '100%' } : undefined"
        :placeholder="effectiveDescPh"
        :disabled="disabled"
        spellcheck="false"
        @input="onCellInput"
        @keydown="onKeydown($event, i)"
        @blur="onCellBlur(row)"
      />
      <span class="kvt-col kvt-actions">
        <IconButton
          v-if="!isGhost(row)"
          name="trash"
          :size="13"
          tone="danger"
          :title="t('common.delete')"
          @click="remove(i)"
        />
      </span>
    </div>

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
  min-height: 32px;
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
.kvt-key {
  width: 34%;
  min-width: 120px;
}
.kvt-value {
  flex: 1;
  min-width: 0;
}
.kvt-desc {
  width: 150px;
}
.kvt-actions {
  width: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
}

.kvt-row {
  display: flex;
  align-items: center;
  min-height: 32px;
  border-bottom: 1px solid var(--border);
  transition: background var(--dur) var(--ease);
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

/* 底部幽灵行：弱化占位，输入即自动成为正式行 */
.kvt-row.ghost .kvt-input {
  opacity: 0.5;
}
.kvt-row.ghost .kvt-input:focus {
  opacity: 1;
}

.kvt-check {
  accent-color: var(--accent);
  cursor: pointer;
}

.kvt-input {
  height: 32px;
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