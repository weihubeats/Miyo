<script setup lang="ts">
/**
 * ParamsPanel：查询参数面板（Postman 式 kv 表格 + 批量编辑）。
 * - 表格由 KeyValueTable 驱动：末尾常驻幽灵行，输入自动补行、空行失焦自动清理；
 * - 右上「批量编辑」切换文本模式：支持 `foo=bar&baz=qux` 与多行 `Key: Value`，
 *   切回表格（⌘Enter / 失焦）时解析入库，Esc 取消。
 */
import { computed, ref } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useVarCandidates } from '../composables/useVarCandidates'
import { handleTextareaTab } from '../utils/textareaIndent'
import KeyValueTable, { type KVRow } from './ui/KeyValueTable.vue'
import type { Endpoint, KeyValue } from '../types/miyoApi'

const props = defineProps<{ draft: Endpoint | null }>()

const locale = useLocaleStore()
const t = locale.t

/** value 列 {{变量}} 自动补全候选（内置 + 环境/项目/全局变量）。 */
const varCandidates = useVarCandidates()

const params = computed(() => props.draft?.request.params ?? [])

function applyParams(rows: KVRow[]): void {
  params.value.splice(0, params.value.length, ...(rows as KeyValue[]))
}

// ---------- 批量编辑 ----------
const bulkMode = ref(false)
const bulkText = ref('')

function serialize(rows: KVRow[]): string {
  return rows
    .filter((r) => r.key || r.value)
    .map((r) => `${r.key}=${r.value}`)
    .join('\n')
}

function parseBulk(text: string): KVRow[] {
  const out: KVRow[] = []
  for (const line of text.split('\n')) {
    for (const chunk of line.split('&')) {
      const t = chunk.trim()
      if (!t) continue
      const eq = t.indexOf('=')
      const colon = t.indexOf(':')
      const idx = eq === -1 ? colon : colon === -1 ? eq : Math.min(eq, colon)
      if (idx <= 0) {
        out.push({ key: t, value: '', enabled: true, description: '' })
      } else {
        out.push({
          key: t.slice(0, idx).trim(),
          value: t.slice(idx + 1).trim(),
          enabled: true,
          description: '',
        })
      }
    }
  }
  return out
}

function enterBulk(): void {
  bulkText.value = serialize(params.value)
  bulkMode.value = true
}

function commitBulk(): void {
  applyParams(parseBulk(bulkText.value))
  bulkMode.value = false
}

function cancelBulk(): void {
  bulkMode.value = false
}

/** 批量编辑 Tab 缩进而非跳出焦点（Esc 取消 / ⌘Enter 提交保持不变）。 */
function onBulkKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab') handleTextareaTab(e.target as HTMLTextAreaElement, e)
}
</script>

<template>
  <div class="panel">
    <div class="params-head">
      <span class="params-count">{{ t('params.count', { n: (params as KeyValue[]).length }) }}</span>
      <button class="bulk-btn" type="button" @click="bulkMode ? commitBulk() : enterBulk()">
        {{ bulkMode ? t('params.tableEdit') : t('params.bulkEdit') }}
      </button>
    </div>

    <div v-if="bulkMode" class="bulk-wrap">
      <textarea
        ref="bulkArea"
        v-model="bulkText"
        class="bulk-area miyo-mono"
        autofocus
        spellcheck="false"
        :placeholder="t('params.bulkPh')"
        @blur="commitBulk"
        @keydown.tab="onBulkKeydown"
        @keydown.esc.prevent="cancelBulk"
        @keydown.meta.enter.prevent="commitBulk"
      ></textarea>
      <p class="bulk-hint">{{ t('params.bulkHint') }}</p>
    </div>

    <KeyValueTable
      v-else
      :model-value="params"
      :var-candidates="varCandidates"
      @update:model-value="applyParams"
    />
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.params-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.params-count {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.bulk-btn {
  height: 22px;
  padding: 0 8px;
  border: none;
  background: none;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xs);
  font-family: inherit;
  color: var(--text-2);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.bulk-btn:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.bulk-wrap {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.bulk-area {
  width: 100%;
  min-height: 140px;
  resize: vertical;
  font-size: 12px;
  line-height: 1.6;
}

.bulk-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>