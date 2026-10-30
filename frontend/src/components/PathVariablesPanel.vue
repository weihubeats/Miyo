<script setup lang="ts">
/**
 * PathVariablesPanel：路径变量编辑（配置区 Path 页签）。
 *
 * - 抽取只在用户点「从路径抽取」时发生（自动回写会让刚打开的接口
 *   被 isDirty 误判为「有改动」，见 EndpointEditor.dirty.spec）；
 * - 写入 draft.request.path_variables：文档预览页与 OpenAPI 导出都读该字段，
 *   发送链路的 {id} / {{id}} 代入由 EndpointEditor.buildUrl 完成
 *   （镜像 miyo-core util::replace_path_variables 的两种写法）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useToast } from '../composables/useToast'
import KeyValueTable, { type KVRow } from './ui/KeyValueTable.vue'
import type { Endpoint, KeyValue } from '../types/miyoApi'
import { extractPathVarNames } from '../utils/pathVariables'

const props = defineProps<{ draft: Endpoint | null }>()

const locale = useLocaleStore()
const t = locale.t
const toast = useToast()

const rows = computed(() => props.draft?.request.path_variables ?? [])
const count = computed(() => rows.value.filter((r) => (r.key ?? '').trim()).length)

/** 上抛：直接赋值（path_variables 可能缺省，splice 到临时数组会静默丢失）。 */
function apply(next: KVRow[]): void {
  const request = props.draft?.request
  if (!request) return
  request.path_variables = next as KeyValue[]
}

/**
 * 从路径抽取占位名：委托 `utils/pathVariables.extractPathVarNames`
 * （与后端 `miyo_core::util::extract_path_variables` 同源的单一实现）。
 */
const extractNames = extractPathVarNames

/** 抽取：已有同名行保留取值，缺失的补空行（不删除用户手写的行）。 */
function extract(): void {
  const d = props.draft
  if (!d) return
  const names = extractNames(d.path)
  if (!names.length) {
    toast.warning(t('editor.pathNoVars'))
    return
  }
  const current = d.request.path_variables ?? []
  const known = new Set(current.map((r) => r.key))
  const merged: KeyValue[] = [...current]
  for (const name of names) {
    if (known.has(name)) continue
    merged.push({ key: name, value: '', enabled: true, description: '' })
  }
  d.request.path_variables = merged
  toast.success(t('editor.pathExtracted', { n: names.length }))
}
</script>

<template>
  <div class="panel">
    <div class="pv-head">
      <span class="pv-count">{{ t('editor.pathCount', { n: count }) }}</span>
      <button class="pv-extract" type="button" @click="extract">
        {{ t('editor.pathExtract') }}
      </button>
    </div>
    <KeyValueTable :model-value="rows" @update:model-value="apply" />
    <p class="pv-hint">{{ t('editor.pathHint') }}</p>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.pv-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.pv-count {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.pv-extract {
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
.pv-extract:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.pv-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
