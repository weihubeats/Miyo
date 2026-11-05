<script setup lang="ts">
/**
 * ScenarioStepDetail：右侧步骤详情面板（从 ScenarioEditor 抽出重构）。
 *
 * 信息架构调整：旧版展开步骤卡后，在卡片内铺「只读请求摘要 + 提取 / 断言
 * 编辑」，中部编排区被撑得很高；新版把每步的完整配置集中到右栏，按 Tab
 * 划分（请求配置 / 变量提取 / 断言规则），中部步骤卡只留紧凑摘要。
 *
 * 职责边界（与旧版一致）：
 * - 请求配置只读：步骤引用式（endpoint_id），请求参数真源在接口编辑器，
 *   顶部明示该契约并提供「在接口编辑器中修改」直达（emit edit-endpoint）；
 * - 提取 / 断言为步骤自有字段，可编辑（沿用旧行内编辑交互，含期望值
 *   字符串编辑态——保存时按字面量智能转型，规则不变）。
 *
 * 编辑实现沿用 ParamsPanel 的 computed 包装模式：props.step 是宿主编辑态
 * 的同一响应式对象，改动即时反映到编排卡片徽章与脏检查。
 */
import { computed, ref } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import CustomSelect from '../ui/CustomSelect.vue'
import EmptyState from '../ui/EmptyState.vue'
import Tabs, { type TabItem } from '../ui/Tabs.vue'
import { methodBadgeTone } from '../../utils/methodTone'
import { formatDurationShort } from '../../utils/format'
import type { EditorStep } from '../ScenarioEditor.vue'
import type { Endpoint } from '../../types/miyoApi'

const props = defineProps<{
  /** 选中步骤（null = 未选中，展示空态）。 */
  step: EditorStep | null
  /** 步骤序号（1-based，仅展示）。 */
  index: number
  /** 步骤引用的接口（null = 未选 / 已删除）。 */
  endpoint: Endpoint | null
  /** 物化 URL 模板（路径变量代入，{{var}} 保留执行期渲染）。 */
  url: string
  /** 只读请求分区（宿主从接口配置物化，保证与运行口径一致）。 */
  pathVars: { key: string; value: string }[]
  params: { key: string; value: string }[]
  headers: { key: string; value: string }[]
  bodyKind: string
  bodyText: string
  /** 执行状态徽章（运行后展示；null = 无状态）。 */
  statusTone: string | null
  statusText: string | null
  /** 执行耗时（ms；运行后展示）。 */
  durationMs: number | null
}>()

const emit = defineEmits<{
  (e: 'edit-endpoint'): void
  (e: 'close'): void
}>()

const t = useLocaleStore().t

// ---------- Tab（切换步骤保持所在 Tab） ----------
const activeTab = ref<string>('request')

const tabs = computed<TabItem[]>(() => [
  { key: 'request', label: t('scenario.detailTabRequest') },
  { key: 'extract', label: t('scenario.detailTabExtract'), count: props.step?.extract.length || undefined },
  { key: 'assert', label: t('scenario.detailTabAssert'), count: props.step?.assertions.length || undefined },
])

// ---------- 提取规则编辑（computed 包装 props.step 内数组，ParamsPanel 同模式：
// 值为同一响应式数组引用，splice / push 即时反映到宿主编辑态与脏检查） ----------
const EXTRACT_FROM_OPTIONS = computed(() => [
  { value: 'body', label: t('scenario.fromBody') },
  { value: 'header', label: t('scenario.fromHeader') },
])

const extractRows = computed(() => props.step?.extract ?? [])

function addExtract(): void {
  extractRows.value.push({ name: '', from: 'body', path: '' })
}

function removeExtract(j: number): void {
  extractRows.value.splice(j, 1)
}

// ---------- 断言规则编辑 ----------
const ASSERT_TYPE_OPTIONS = computed(() => [
  { value: 'status', label: t('scenario.assertStatus') },
  { value: 'jsonpath', label: t('scenario.assertJsonpath') },
  { value: 'header', label: t('scenario.assertHeader') },
  { value: 'response_time_ms', label: t('scenario.assertTime') },
])
const ASSERT_OP_OPTIONS = computed(() =>
  (
    ['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'exists', 'not_exists', 'empty', 'not_empty'] as const
  ).map((op) => ({ value: op, label: t(`scenario.op_${op}`) })),
)
const NO_EXPECTED_OPS = new Set(['exists', 'not_exists', 'empty', 'not_empty'])

const assertRows = computed(() => props.step?.assertions ?? [])

function addAssert(): void {
  assertRows.value.push({ name: null, type: 'status', path: '', op: 'eq', expected: '' })
}

function removeAssert(j: number): void {
  assertRows.value.splice(j, 1)
}

/** 提取占位符随来源切换（响应体 JSONPath / 响应头名）。 */
function extractPathPh(from: string): string {
  return from === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')
}

/** 断言路径占位符随类型切换。 */
function assertPathPh(type: string): string {
  return type === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')
}

// ---------- 请求分区可见性（无内容不占位；body=none 明示「无请求体」） ----------
const bodyModeLabel = computed(() => {
  switch (props.bodyKind) {
    case 'json':
      return t('scenario.bodyJson')
    case 'text':
      return t('scenario.bodyText')
    case 'urlencoded':
      return t('scenario.bodyUrlencoded')
    case 'multipart':
      return t('scenario.bodyMultipart')
    case 'graphql':
      return t('scenario.bodyGraphql')
    case 'binary':
      return t('scenario.bodyBinary')
    case 'none':
      return t('scenario.reqNone')
    default:
      return ''
  }
})

const hasBody = computed(() => props.bodyKind !== 'none' && !!props.bodyText)

/** 请求体（multipart 等表单）字段计数替代全文。 */
const bodyFieldCount = computed(() => {
  if (props.bodyKind !== 'urlencoded' && props.bodyKind !== 'multipart') return null
  return props.bodyText ? props.bodyText.split('&').filter(Boolean).length : 0
})
</script>

<template>
  <!-- 未选中：空态引导 -->
  <div v-if="!step" class="se-detail-empty">
    <EmptyState icon="arrow-up-right" :title="t('scenario.detailEmptyTitle')" :description="t('scenario.detailEmptyDesc')" compact />
  </div>

  <!-- 选中：Tab 详情 -->
  <template v-else>
    <div class="se-detail-head">
      <span class="se-detail-title">
        <span class="se-detail-idx">{{ index }}</span>
        {{ t('scenario.detailTitle') }}
      </span>
      <span v-if="statusTone" class="se-detail-status" :class="`tone-${statusTone}`">{{ statusText }}</span>
      <span v-if="durationMs != null" class="se-detail-time" :title="t('scenario.stepDuration')">
        <Icon name="clock" :size="11" />{{ formatDurationShort(durationMs) }}
      </span>
      <IconButton name="x" :size="13" :label="t('scenario.detailClose')" :title="t('scenario.detailClose')" @click="emit('close')" />
    </div>

    <Tabs v-model="activeTab" :tabs="tabs" size="sm" class="se-detail-tabs" />

    <div class="se-detail-body">
      <!-- ===== 请求配置（只读；真源在接口编辑器） ===== -->
      <div v-if="activeTab === 'request'" class="se-detail-tab">
        <div v-if="endpoint" class="se-detail-ref">
          <p class="se-detail-ref-hint">{{ t('scenario.detailRequestRef') }}</p>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('edit-endpoint')">
            <Icon name="pencil" :size="12" /> {{ t('scenario.detailEditInEndpoint') }}
          </button>
        </div>

        <div v-else class="se-detail-missing">
          <Icon name="alert-triangle" :size="13" />
          {{ t('scenario.endpointMissing') }}
        </div>

        <div v-if="endpoint" class="se-section">
          <div class="se-section-title">{{ t('scenario.sectionUrl') }}</div>
          <div class="se-url-row">
            <span class="se-method" :class="methodBadgeTone(endpoint.method)">{{ endpoint.method }}</span>
            <code class="se-url" :title="url">{{ url }}</code>
          </div>
        </div>

        <div v-if="pathVars.length" class="se-section">
          <div class="se-section-title">{{ t('scenario.reqPathVars') }} · {{ pathVars.length }}</div>
          <div v-for="p in pathVars" :key="`pv-${p.key}`" class="se-kv">
            <code class="se-kv-key">{{ p.key }}</code>
            <code class="se-kv-val">{{ p.value }}</code>
          </div>
        </div>

        <div v-if="params.length" class="se-section">
          <div class="se-section-title">{{ t('scenario.reqQuery') }} · {{ params.length }}</div>
          <div v-for="p in params" :key="`q-${p.key}`" class="se-kv">
            <code class="se-kv-key">{{ p.key }}</code>
            <code class="se-kv-val">{{ p.value }}</code>
          </div>
        </div>

        <div v-if="headers.length" class="se-section">
          <div class="se-section-title">{{ t('scenario.reqHeaders') }} · {{ headers.length }}</div>
          <div v-for="h in headers" :key="`h-${h.key}`" class="se-kv">
            <code class="se-kv-key">{{ h.key }}</code>
            <code class="se-kv-val">{{ h.value }}</code>
          </div>
        </div>

        <div v-if="endpoint" class="se-section">
          <div class="se-section-title">
            {{ t('scenario.reqBody') }}
            <span v-if="bodyModeLabel" class="se-section-tag">{{ bodyModeLabel }}</span>
            <span v-if="bodyFieldCount != null && bodyFieldCount > 0" class="se-section-tag">
              {{ t('scenario.fieldCount', { n: bodyFieldCount }) }}
            </span>
          </div>
          <pre v-if="hasBody" class="se-body-pre">{{ bodyText }}</pre>
          <p v-else class="se-section-none">{{ t('scenario.reqNone') }}</p>
        </div>
      </div>

      <!-- ===== 变量提取（可编辑） ===== -->
      <div v-else-if="activeTab === 'extract'" class="se-detail-tab">
        <div class="se-rule-title">
          <span>{{ t('scenario.extractSection') }}</span>
          <button class="se-rule-add" type="button" @click="addExtract">
            <Icon name="plus" :size="12" /> {{ t('scenario.addExtract') }}
          </button>
        </div>
        <p v-if="!extractRows.length" class="se-rule-empty">{{ t('scenario.ruleEmpty') }}</p>
        <div v-for="(r, j) in extractRows" :key="j" class="se-rule-row">
          <input
            v-model="r.name"
            class="miyo-input miyo-input-sm se-r-name"
            :placeholder="t('scenario.extractNamePh')"
            spellcheck="false"
          />
          <CustomSelect v-model="r.from" :options="EXTRACT_FROM_OPTIONS" size="sm" class="se-r-from" />
          <input
            v-model="r.path"
            class="miyo-input miyo-input-sm se-r-path"
            :placeholder="extractPathPh(r.from)"
            spellcheck="false"
          />
          <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="removeExtract(j)" />
        </div>
      </div>

      <!-- ===== 断言规则（可编辑） ===== -->
      <div v-else class="se-detail-tab">
        <div class="se-rule-title">
          <span>{{ t('scenario.assertSection') }}</span>
          <button class="se-rule-add" type="button" @click="addAssert">
            <Icon name="plus" :size="12" /> {{ t('scenario.addAssert') }}
          </button>
        </div>
        <p v-if="!assertRows.length" class="se-rule-empty">{{ t('scenario.ruleEmpty') }}</p>
        <div v-for="(a, j) in assertRows" :key="j" class="se-rule-col">
          <div class="se-rule-row">
            <CustomSelect v-model="a.type" :options="ASSERT_TYPE_OPTIONS" size="sm" class="se-a-type" />
            <input
              v-if="a.type === 'jsonpath' || a.type === 'header'"
              v-model="a.path"
              class="miyo-input miyo-input-sm se-r-path"
              :placeholder="assertPathPh(a.type)"
              spellcheck="false"
            />
            <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="removeAssert(j)" />
          </div>
          <div class="se-rule-row">
            <CustomSelect v-model="a.op" :options="ASSERT_OP_OPTIONS" size="sm" class="se-a-op" />
            <input
              v-if="!NO_EXPECTED_OPS.has(a.op ?? 'eq')"
              v-model="a.expected"
              class="miyo-input miyo-input-sm se-r-path"
              :placeholder="t('scenario.expectedPh')"
              spellcheck="false"
            />
          </div>
        </div>
      </div>
    </div>
  </template>
</template>

<style scoped>
.se-detail-empty {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
}

/* ---- 头部 ---- */
.se-detail-head {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px 8px;
}

.se-detail-title {
  flex: 1;
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}

.se-detail-idx {
  flex: none;
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-tint);
  color: var(--accent);
  font-size: var(--fs-xxs);
  font-weight: 700;
}

.se-detail-status {
  flex: none;
  font-size: var(--fs-xxs);
  padding: 1px 8px;
  border-radius: 999px;
}

.se-detail-status.tone-ok {
  background: var(--success-tint);
  color: var(--success);
}

.se-detail-status.tone-fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.se-detail-status.tone-skip,
.se-detail-status.tone-wait {
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-detail-status.tone-run {
  background: var(--accent-tint);
  color: var(--accent);
}

.se-detail-status.tone-cancel {
  background: var(--warning-tint);
  color: var(--warning);
}

.se-detail-time {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- Tab ---- */
.se-detail-tabs {
  flex: none;
  margin: 0 12px;
}

.se-detail-tabs :deep(.tabs) {
  border-bottom: 1px solid var(--border);
}

/* ---- Tab 体 ---- */
.se-detail-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px;
}

.se-detail-tab {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 请求配置只读提示 + 直达编辑 */
.se-detail-ref {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px 10px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
}

.se-detail-ref-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}

.se-detail-missing {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--danger);
}

/* ---- 只读分区 ---- */
.se-section {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.se-section-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
}

.se-section-tag {
  font-weight: 400;
  padding: 0 6px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-3);
}

.se-section-none {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-url-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.se-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-1);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 4px 8px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.se-kv {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: var(--fs-xxs);
  min-width: 0;
}

.se-kv-key {
  flex: none;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono);
  color: var(--tok-key);
}

.se-kv-val {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  color: var(--text-2);
  word-break: break-all;
  overflow-wrap: anywhere;
}

.se-body-pre {
  margin: 0;
  max-height: 220px;
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-2);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 6px 8px;
  white-space: pre-wrap;
  word-break: break-all;
}

/* ---- 提取 / 断言编辑（沿用旧行样式） ---- */
.se-rule-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-2);
}

.se-rule-add {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
}

.se-rule-add:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.se-rule-empty {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-rule-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.se-rule-col {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding-bottom: 6px;
  border-bottom: 1px dashed var(--border);
}

.se-r-name {
  flex: 0 0 110px;
}

.se-r-from {
  flex: 0 0 88px;
}

.se-r-path {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

.se-a-type {
  flex: 0 0 108px;
}

.se-a-op {
  flex: 0 0 92px;
}

/* ---- Method 徽章（与编排卡同语言） ---- */
.se-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.4px;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}
</style>
