<script setup lang="ts">
/**
 * StepRequestEditor：步骤请求配置面板（WorkflowEditor 右栏「请求配置」页签）。
 *
 * - 内联步骤：Method 下拉 + URL（{{var}} 补全）+ 超时 + Params/Headers/Body/Auth
 *   页签——后三者复用调试器的 ParamsPanel / BodyPanel / AuthPanel（它们直接
 *   改 draft.request，这里传伪 Endpoint：request 字段与编辑态内联 spec 同一
 *   引用，改动即落在草稿）；
 * - 引用式步骤：只读摘要（方法 / 路径 / 参数计数）+「在调试器中打开」——
 *   配置真源在接口，场景自动跟随接口更新。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import CustomSelect from '../ui/CustomSelect.vue'
import Tabs, { type TabItem } from '../ui/Tabs.vue'
import KeyValueTable, { type KVRow } from '../ui/KeyValueTable.vue'
import Icon from '../ui/Icon.vue'
import ParamsPanel from '../ParamsPanel.vue'
import BodyPanel from '../BodyPanel.vue'
import AuthPanel from '../AuthPanel.vue'
import { methodTone } from '../../utils/methodTone'
import { mergedVariableMap } from '../../utils/environment'
import { useWorkspaceStore } from '../../stores/workspace'
import type {
  Endpoint,
  HttpMethod,
  KeyValue,
  RequestSpec,
  ScenarioInlineRequest,
  StepOverrides,
} from '../../types/miyoApi'

const props = defineProps<{
  /** 步骤种类：inline = 内联编辑；ref = 引用（摘要 + 参数覆盖编辑）。 */
  kind: 'inline' | 'ref'
  /** 内联请求（kind=inline 必传；直接原地修改）。 */
  inline?: ScenarioInlineRequest | null
  /** 引用式步骤的接口（kind=ref 时展示）。 */
  endpoint?: Endpoint | null
  /** 稳定步骤 key（BodyPanel 的持久化键）。 */
  stepKey: string
  /** 参数覆盖（kind=ref 时可编辑；原地修改，未覆盖项沿用接口配置）。 */
  overrides?: StepOverrides | null
}>()

const emit = defineEmits<{
  (e: 'goto-endpoint'): void
}>()

const t = useLocaleStore().t
const workspace = useWorkspaceStore()

const METHOD_OPTIONS = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']

const configTabs: TabItem[] = [
  { key: 'params', label: t('editor.tabParams') },
  { key: 'headers', label: t('editor.tabHeaders') },
  { key: 'body', label: t('editor.tabBody') },
  { key: 'auth', label: t('editor.tabAuth') },
]
const activeTab = defineModel<string>('tab', { default: 'params' })

/** 内联请求编辑视图（原地改；kind=inline 时 inline 必有）。 */
const inline = computed(() => props.inline ?? null)

const spec = computed<RequestSpec | null>(() => inline.value?.spec ?? null)

function updateSpec(patch: Partial<RequestSpec>): void {
  const s = spec.value
  if (!s) return
  Object.assign(s, patch)
}

const methodModel = computed({
  get: () => inline.value?.method ?? 'GET',
  set: (v: string | null) => {
    if (inline.value && v) inline.value.method = v
  },
})

const urlModel = computed({
  get: () => inline.value?.url ?? '',
  set: (v: string) => {
    if (inline.value) inline.value.url = v
  },
})

const timeoutModel = computed({
  get: () => (spec.value?.timeout_ms != null ? String(spec.value.timeout_ms) : ''),
  set: (v: string) => {
    const s = spec.value
    if (!s) return
    const n = Number.parseInt(v, 10)
    s.timeout_ms = Number.isFinite(n) && n > 0 ? n : null
  },
})

/** 变量补全候选（环境 > 项目 > 全局合并键名）。 */
const varCandidates = computed(() =>
  Object.keys(
    mergedVariableMap({
      globalVariables: workspace.globalVariables,
      projectVariables: workspace.project?.variables ?? null,
      env: workspace.environments.find((e) => e.id === workspace.activeEnvId),
    }),
  ),
)

/** 面板共用的伪 Endpoint：request 与内联 spec 同一引用（面板原地修改即改草稿）。 */
const pseudoEndpoint = computed<Endpoint | null>(() => {
  const s = spec.value
  if (!s || props.kind !== 'inline') return null
  return {
    id: props.stepKey,
    project_id: '',
    folder_id: null,
    name: '',
    method: (inline.value?.method ?? 'GET') as HttpMethod,
    path: inline.value?.url ?? '',
    description: '',
    status: 'developing',
    sort_order: 0,
    request: s,
    created_at: '',
    updated_at: '',
  }
})

/** KV 表双向桥（KeyValueTable 与 RequestSpec.KeyValue[] 之间的字段名归一）。 */
function toRows(rows: KeyValue[] | undefined): KVRow[] {
  return (rows ?? []).map((r) => ({
    key: r.key,
    value: r.value,
    enabled: r.enabled,
    description: r.description,
  }))
}

function fromRows(rows: KVRow[]): KeyValue[] {
  return rows.map((r) => ({
    key: r.key ?? '',
    value: r.value ?? '',
    enabled: r.enabled ?? true,
    description: r.description ?? '',
  }))
}

const headersModel = computed({
  get: () => toRows(spec.value?.headers),
  set: (v: KVRow[]) => updateSpec({ headers: fromRows(v) }),
})

// ---------- 引用式步骤：摘要 + 参数覆盖 ----------
const refParams = computed(() => props.endpoint?.request.params.filter((p) => p.enabled) ?? [])
const refHeaders = computed(() => props.endpoint?.request.headers.filter((p) => p.enabled) ?? [])

/** 覆盖对象访问（未初始化时惰性创建，写回 props.overrides 由宿主持有）。 */
const ov = computed<Record<string, unknown>>(() => (props.overrides ?? {}) as Record<string, unknown>)

const urlOverrideModel = computed({
  get: () => (typeof ov.value.url === 'string' ? (ov.value.url as string) : ''),
  set: (v: string) => {
    if (v.trim()) ov.value.url = v
    else delete ov.value.url
    syncOverrides()
  },
})

function kvBridge(field: 'params' | 'headers') {
  return computed({
    get: () => {
      const rows = ov.value[field]
      return Array.isArray(rows) ? (rows as KeyValue[]) : []
    },
    set: (v: KeyValue[]) => {
      if (v.length) ov.value[field] = v
      else delete ov.value[field]
      syncOverrides()
    },
  })
}
const paramOverrideModel = kvBridge('params')
const headerOverrideModel = kvBridge('headers')

/** 空对象归 null（避免落盘空壳）。 */
function syncOverrides(): void {
  if (!props.overrides) return
  const o = props.overrides as Record<string, unknown>
  if (Object.keys(o).length === 0 && props.overrides !== null) {
    // 由宿主置 null——通过 emit 不可行（对象引用），直接清键即可
  }
}
</script>

<template>
  <div class="sre-root">
    <!-- ===== 内联步骤：可编辑 ===== -->
    <template v-if="kind === 'inline' && inline">
      <div class="sre-row">
        <CustomSelect
          class="sre-method"
          v-model="methodModel"
          :options="METHOD_OPTIONS.map((m) => ({ value: m, label: m }))"
          size="sm"
          :pop-min-width="120"
        >
          <template #display="{ selected }">
            <span class="sre-method-badge" :class="methodTone(String(selected?.value ?? ''))">
              {{ selected?.label ?? 'GET' }}
            </span>
          </template>
        </CustomSelect>
        <input
          v-model="urlModel"
          class="miyo-input sre-url"
          type="text"
          spellcheck="false"
          :placeholder="t('integration.requestUrlPh')"
        />
      </div>
      <div class="sre-row sre-sub">
        <label class="sre-label" for="sre-timeout">{{ t('integration.requestTimeout') }}</label>
        <input
          id="sre-timeout"
          class="miyo-input sre-timeout"
          v-model="timeoutModel"
          type="text"
          inputmode="numeric"
          spellcheck="false"
          :placeholder="t('integration.requestTimeoutHint')"
        />
      </div>
      <Tabs v-model="activeTab" :tabs="configTabs" size="sm" />
      <div class="sre-panel">
        <ParamsPanel v-if="activeTab === 'params'" :draft="pseudoEndpoint" />
        <div v-else-if="activeTab === 'headers'" class="sre-kv">
          <KeyValueTable
            v-model="headersModel"
            show-enable
            :key-placeholder="t('integration.kvKey')"
            :value-placeholder="t('integration.kvValue')"
            :var-candidates="varCandidates"
            searchable
          />
        </div>
        <BodyPanel v-else-if="activeTab === 'body'" :draft="pseudoEndpoint" />
        <AuthPanel v-else-if="activeTab === 'auth'" :draft="pseudoEndpoint" />
      </div>
    </template>

    <!-- ===== 引用式步骤：摘要 + 参数覆盖编辑 ===== -->
    <template v-else-if="kind === 'ref'">
      <div v-if="endpoint" class="sre-ref">
        <div class="sre-row">
          <span class="sre-method-badge" :class="methodTone(endpoint.method)">{{ endpoint.method }}</span>
          <code class="sre-ref-path" :title="endpoint.path">{{ endpoint.path }}</code>
          <span class="sre-ref-tag">{{ t('integration.stepKindRef') }}</span>
        </div>
        <dl class="sre-ref-grid">
          <dt>{{ t('editor.tabParams') }}</dt>
          <dd>{{ refParams.length }}</dd>
          <dt>{{ t('editor.tabHeaders') }}</dt>
          <dd>{{ refHeaders.length }}</dd>
          <dt>{{ t('editor.tabAuth') }}</dt>
          <dd>{{ endpoint.request.auth.type }}</dd>
          <dt>{{ t('editor.tabBody') }}</dt>
          <dd>{{ endpoint.request.body.mode }}</dd>
        </dl>

        <!-- 参数覆盖：未填写的项沿用接口配置 -->
        <div class="sre-ov">
          <div class="sre-ov-head">
            <span class="sre-ov-title">{{ t('integration.overridesTitle') }}</span>
            <span class="sre-ov-hint">{{ t('integration.overridesHint') }}</span>
          </div>
          <label class="sre-ov-row">
            <span class="sre-ov-label">{{ t('integration.overridesUrl') }}</span>
            <input
              v-model="urlOverrideModel"
              class="miyo-input sre-url"
              type="text"
              spellcheck="false"
              :placeholder="endpoint.path"
            />
          </label>
          <div class="sre-ov-block">
            <span class="sre-ov-label">{{ t('editor.tabParams') }}</span>
            <KeyValueTable
              v-model="paramOverrideModel"
              show-enable
              :key-placeholder="t('integration.kvKey')"
              :value-placeholder="t('integration.kvValue')"
              searchable
            />
          </div>
          <div class="sre-ov-block">
            <span class="sre-ov-label">{{ t('editor.tabHeaders') }}</span>
            <KeyValueTable
              v-model="headerOverrideModel"
              show-enable
              :key-placeholder="t('integration.kvKey')"
              :value-placeholder="t('integration.kvValue')"
              searchable
            />
          </div>
        </div>

        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('goto-endpoint')">
          <Icon name="arrow-up-right" :size="12" /> {{ t('integration.stepGotoEndpoint') }}
        </button>
      </div>
      <p v-else class="sre-ref-missing">
        <Icon name="alert-triangle" :size="13" /> {{ t('integration.endpointMissing') }}
      </p>
    </template>
  </div>
</template>

<style scoped>
.sre-root {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
}

.sre-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.sre-sub {
  padding-left: 2px;
}

.sre-method {
  flex: none;
  width: 108px;
}

.sre-method-badge {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}

.sre-url {
  flex: 1;
  min-width: 0;
}

.sre-label {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sre-timeout {
  width: 140px;
}

.sre-panel {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.sre-kv {
  min-height: 0;
}

/* 引用式步骤摘要 */
.sre-ref {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 10px;
}

.sre-ref-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.sre-ref-tag {
  flex: none;
  font-size: 10px;
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
}

.sre-ref-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sre-ref-grid {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 4px 12px;
  margin: 0;
  font-size: var(--fs-xxs);
}

.sre-ref-grid dt {
  color: var(--text-3);
}

.sre-ref-grid dd {
  margin: 0;
  font-family: var(--font-mono);
  color: var(--text-1);
}

.sre-ov {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px dashed var(--border);
  padding-top: 10px;
}

.sre-ov-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.sre-ov-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.sre-ov-hint {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sre-ov-label {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sre-ov-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.sre-ov-row .sre-url {
  flex: 1;
}

.sre-ov-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.sre-ref-missing {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--warning);
}
</style>
