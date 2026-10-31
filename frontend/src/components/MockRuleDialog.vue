<script setup lang="ts">
/**
 * MockRuleDialog：Mock 规则管理（列表 / 新建 / 编辑 / 删除）。
 * 规则在 mock_start 时按 priority 降序匹配，可覆盖接口默认示例。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import { rowKey } from '../utils/rowKey'
import CustomSelect from './ui/CustomSelect.vue'
import CustomNumberInput from './ui/CustomNumberInput.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Modal from './ui/Modal.vue'
import Popconfirm from './ui/Popconfirm.vue'
import type { MockRule } from '../types/miyoApi'

const emit = defineEmits<{ close: [] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const rules = ref<MockRule[]>([])
const busy = ref(false)
const editing = ref<MockRule | null>(null)

// ---------- 响应头编辑（模型为 HashMap → 行数组桥接，保存时剔除空键回写） ----------
const respHeaderRows = ref<{ key: string; value: string }[]>([])
watch(
  editing,
  (r) => {
    respHeaderRows.value = r
      ? Object.entries(r.response_headers ?? {}).map(([key, value]) => ({ key, value }))
      : []
  },
  { immediate: true },
)
function addRespHeader(): void {
  respHeaderRows.value.push({ key: '', value: '' })
}
function removeRespHeader(index: number): void {
  respHeaderRows.value.splice(index, 1)
}
function applyRespHeaders(): void {
  if (!editing.value) return
  const out: Record<string, string> = {}
  for (const { key, value } of respHeaderRows.value) {
    const k = key.trim()
    if (k) out[k] = value
  }
  editing.value.response_headers = out
}

const METHODS = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']
const METHOD_OPTIONS = METHODS.map((m) => ({ value: m, label: m }))
const PRIORITY_OPTIONS = computed(() => [
  { value: 0, label: t('mockrule.priority', { n: 0, v: t('mockrule.priorityDefault') }) },
  { value: 1, label: t('mockrule.priority', { n: 1, v: t('mockrule.priorityHigh') }) },
  { value: 2, label: t('mockrule.priority', { n: 2, v: t('mockrule.priorityTop') }) },
])

async function load(): Promise<void> {
  if (!store.project) return
  busy.value = true
  try {
    rules.value = (await api.listMockRules(store.project.id)) ?? []
  } catch (err) {
    toast.error(t('mockrule.loadFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

onMounted(load)

function blankRule(): MockRule {
  const now = new Date().toISOString()
  return {
    id: crypto.randomUUID(),
    project_id: store.project?.id ?? '',
    endpoint_id: null,
    name: '',
    method: 'GET',
    path: '/',
    match_query: [],
    match_headers: [],
    response_status: 200,
    response_headers: {},
    response_body_template: '',
    delay_ms: 0,
    fault_rate_pct: 0,
    fault_status: 500,
    enabled: true,
    priority: 0,
    created_at: now,
    updated_at: now,
  }
}

function addMatch(row: MockRule, which: 'match_query' | 'match_headers'): void {
  row[which].push({ key: '', value: '' })
}

function removeMatch(row: MockRule, which: 'match_query' | 'match_headers', index: number): void {
  row[which].splice(index, 1)
}

async function save(): Promise<void> {
  if (!editing.value) return
  if (!editing.value.name.trim()) {
    toast.error(t('mockrule.nameRequired'))
    return
  }
  busy.value = true
  try {
    applyRespHeaders()
    const saved = await api.saveMockRule({ ...editing.value, updated_at: new Date().toISOString() })
    const idx = rules.value.findIndex((r) => r.id === saved.id)
    if (idx === -1) rules.value.push(saved)
    else rules.value[idx] = saved
    editing.value = null
    toast.success(t('mockrule.saved'))
    // 规则变更即时生效：服务运行中自动热重载（原先需手动点「热重载」，
    // 出现「明明关了规则还命中」的困惑）。
    try {
      const port = await api.mockStatus()
      if (port) await api.mockReload()
    } catch {
      // 未运行或重载失败不打断保存流程
    }
  } catch (err) {
    toast.error(t('mockrule.saveFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

async function remove(rule: MockRule): Promise<void> {
  try {
    await api.deleteMockRule(rule.id)
    rules.value = rules.value.filter((r) => r.id !== rule.id)
    if (editing.value?.id === rule.id) editing.value = null
  } catch (err) {
    toast.error(t('mockrule.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

const listTitle = computed(() => t('mockrule.listTitle', { n: rules.value.length }))
</script>

<template>
  <Modal :open="true" :title="editing ? t('mockrule.editTitle') : listTitle" width="620px" @close="emit('close')">
    <div v-if="editing" class="rule-form">
      <div class="kv-row">
        <input v-model="editing.name" class="miyo-input miyo-input-sm kv-key" :placeholder="t('mockrule.namePh')" />
        <CustomSelect
          :model-value="editing.method"
          :options="METHOD_OPTIONS"
          size="sm"
          @update:model-value="editing.method = String($event) as MockRule['method']"
        >
          <template #display="{ label }">
            <span :class="`miyo-method miyo-method-${editing.method.toLowerCase()}`">{{ label }}</span>
          </template>
        </CustomSelect>
        <input v-model="editing.path" class="miyo-input miyo-input-sm kv-value" placeholder="/users/{id}" />
      </div>
      <div class="kv-row">
        <CustomNumberInput
          :model-value="editing.response_status"
          size="sm"
          :min="100"
          :max="599"
          :placeholder="t('mockrule.statusPh')"
          @update:model-value="editing.response_status = $event === '' ? 100 : Number($event)"
        />
        <CustomNumberInput
          :model-value="editing.delay_ms"
          size="sm"
          :min="0"
          :placeholder="t('mockrule.delayPh')"
          @update:model-value="editing.delay_ms = $event === '' ? 0 : Number($event)"
        />
        <CustomNumberInput
          :model-value="editing.fault_rate_pct ?? 0"
          size="sm"
          :min="0"
          :max="100"
          :placeholder="t('mockrule.faultPh')"
          :title="t('mockrule.faultHint')"
          @update:model-value="editing.fault_rate_pct = $event === '' ? 0 : Math.max(0, Math.min(100, Number($event)))"
        />
        <CustomNumberInput
          :model-value="editing.fault_status ?? 500"
          size="sm"
          :min="100"
          :max="599"
          :placeholder="t('mockrule.faultCodePh')"
          :title="t('mockrule.faultCodeHint')"
          @update:model-value="editing.fault_status = $event === '' ? 500 : Number($event)"
        />
        <CustomSelect
          :model-value="editing.priority"
          :options="PRIORITY_OPTIONS"
          size="sm"
          @update:model-value="editing.priority = Number($event)"
        />
        <label class="rule-enabled">
          <input v-model="editing.enabled" type="checkbox" /> {{ t('mockrule.enabled') }}
        </label>
      </div>
      <div class="rule-sub">{{ t('mockrule.queryMatch') }}</div>
      <div v-for="(m, i) in editing.match_query" :key="rowKey(m)" class="kv-row">
        <input v-model="m.key" class="miyo-input miyo-input-sm kv-key" :placeholder="t('kv.colKey')" />
        <input v-model="m.value" class="miyo-input miyo-input-sm kv-value" :placeholder="t('kv.colValue')" />
        <IconButton name="x" :size="13" :title="t('common.delete')" @click="removeMatch(editing, 'match_query', i)" />
      </div>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="addMatch(editing, 'match_query')">
        <Icon name="plus" :size="13" /> Query
      </button>
      <div class="rule-sub">{{ t('mockrule.headerMatch') }}</div>
      <div v-for="(m, i) in editing.match_headers" :key="rowKey(m)" class="kv-row">
        <input v-model="m.key" class="miyo-input miyo-input-sm kv-key" :placeholder="t('kv.colKey')" />
        <input v-model="m.value" class="miyo-input miyo-input-sm kv-value" :placeholder="t('kv.colValue')" />
        <IconButton name="x" :size="13" :title="t('common.delete')" @click="removeMatch(editing, 'match_headers', i)" />
      </div>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="addMatch(editing, 'match_headers')">
        <Icon name="plus" :size="13" /> {{ t('mockrule.addHeader') }}
      </button>
      <div class="rule-sub">{{ t('mockrule.responseHeaders') }}</div>
      <div v-for="(h, i) in respHeaderRows" :key="i" class="kv-row">
        <input v-model="h.key" class="miyo-input miyo-input-sm kv-key" :placeholder="t('kv.colKey')" />
        <input v-model="h.value" class="miyo-input miyo-input-sm kv-value" :placeholder="t('kv.colValue')" />
        <IconButton name="x" :size="13" :title="t('common.delete')" @click="removeRespHeader(i)" />
      </div>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="addRespHeader">
        <Icon name="plus" :size="13" /> {{ t('mockrule.addResponseHeader') }}
      </button>
      <textarea
        v-model="editing.response_body_template"
        class="miyo-input rule-body"
        spellcheck="false"
        :placeholder="t('mockrule.bodyPh')"
      ></textarea>
    </div>

    <ul v-else-if="rules.length" class="rule-list">
      <li v-for="r in rules" :key="r.id" class="rule-row">
        <span class="rule-method">{{ r.method }}</span>
        <span class="rule-path">{{ r.path }}</span>
        <span class="rule-status">{{ r.response_status }}</span>
        <span class="rule-meta">
          {{ r.enabled ? t('mockrule.enabled') : t('mockrule.disabled') }} · {{ t('mockrule.priorityShort', { n: r.priority }) }}<span v-if="(r.fault_rate_pct ?? 0) > 0"> · {{ t('mockrule.faultMeta', { rate: r.fault_rate_pct ?? 0, code: r.fault_status ?? 500 }) }}</span><span v-if="r.delay_ms"> · {{ t('mockrule.delayMeta', { ms: r.delay_ms }) }}</span>
        </span>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="editing = { ...r }">{{ t('common.edit') }}</button>
        <Popconfirm :title="t('mockrule.deleteConfirm', { name: r.name })" @confirm="remove(r)">
          <IconButton name="trash" :size="13" tone="danger" :title="t('common.delete')" />
        </Popconfirm>
      </li>
    </ul>
    <p v-else class="rule-hint">{{ t('mockrule.emptyHint') }}</p>

    <template #footer>
      <template v-if="editing">
        <button class="miyo-btn miyo-btn-sm" type="button" @click="editing = null">{{ t('common.cancel') }}</button>
        <button class="miyo-btn miyo-btn-primary miyo-btn-sm" type="button" :disabled="busy" @click="save">
          {{ t('common.save') }}
        </button>
      </template>
      <template v-else>
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          @click="editing = blankRule()"
        >
          <Icon name="plus" :size="13" /> {{ t('mockrule.newRule') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('close')">{{ t('common.close') }}</button>
      </template>
    </template>
  </Modal>
</template>

<style scoped>
.rule-form {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.rule-sub {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
  margin-top: 4px;
}

.rule-body {
  min-height: 90px;
  font-family: var(--font-mono);
  font-size: 12px;
  resize: vertical;
}

.rule-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.rule-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
}

.rule-method {
  width: 52px;
  flex-shrink: 0;
  font-weight: 700;
  color: var(--text-2);
}

.rule-path {
  flex: 1;
  font-family: var(--font-mono);
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rule-status {
  width: 40px;
  font-weight: 600;
  color: var(--success);
}

.rule-meta {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.rule-hint {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}

.rule-enabled {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--text-1);
}
.rule-enabled input {
  width: 14px;
  height: 14px;
  accent-color: var(--accent);
  cursor: pointer;
}
</style>