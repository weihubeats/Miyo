<script setup lang="ts">
/**
 * BodyPanel：请求体面板。
 * Tab Bar（Postman 风格）：none / form-data / x-www-form-urlencoded / raw / binary / graphql。
 * - raw 为 json+text 聚合视图：右侧子类型下拉（JSON/Text/JS/HTML/XML）切换
 *   编辑器并联动 Content-Type 请求头（映射逻辑见 utils/bodyMode.ts）；
 * - binary 为本地文件路径，发送时后端读取原始字节作为请求体；
 * - urlencoded/multipart 为字段行编辑，graphql 为 query/variables 编辑器。
 * 高度契约：编辑器恒填满本面板分配的高度（面板 flex:1，卡片高度由请求卡
 * grow / 分割条 / 响应期分派），高度入口只有请求卡底边分割条——编辑器**不**再
 * 贴内容生长：卡片吃满剩余空间而编辑器只到内容高时，卡片中部会留出一片空白
 * （用户实测「下拉后中间很大空白」）。例外：graphql 的 variables 是次级编辑器，
 * 保持 auto-height 贴内容，由主 query textarea（flex:1）吸收剩余高度。
 * bodyAny 用 any 放宽联合类型访问（模板 v-model 直写 raw / spec.*）。
 */
import { computed, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { countOccurrences, useFindInText } from '../composables/useFindInText'

import CustomSelect from './ui/CustomSelect.vue'
import FindBar from './ui/FindBar.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import JsonEditor from './ui/JsonEditor.vue'
import KeyValueTable, { type KVRow } from './ui/KeyValueTable.vue'
import SegmentedControl, { type SegmentOption } from './ui/SegmentedControl.vue'
import { RAW_SUBTYPES, applyBodyTab, applyRawSubtype, tabOf } from '../utils/bodyMode'
import { draftsOf, rememberTabContentType, restoreTabContentType, setSlot, slotOf } from '../utils/bodyDrafts'
import { handleTextareaTab } from '../utils/textareaIndent'
import type { BodyTab, RawSubtype } from '../utils/bodyMode'
import type { BodySpec, Endpoint, GraphQLSpec, KeyValue, MultipartField } from '../types/miyoApi'

const props = defineProps<{
  draft: Endpoint | null
}>()

const locale = useLocaleStore()
const t = locale.t

/** 编辑视图：BodySpec 联合各分支字段统一为必填（运行时按 activeTab 分支存在）。 */
type EditableBody = {
  mode: string
  raw: string
  path: string
  spec: GraphQLSpec
  fields: KeyValue[] | MultipartField[]
}

const bodyAny = computed(() => props.draft?.request.body as EditableBody)
const headersAny = computed(() => props.draft?.request.headers as KeyValue[] | undefined)
const graphql = computed(() => bodyAny.value.spec)
const urlencodedFields = computed(() => bodyAny.value.fields as KeyValue[])
const multipartFields = computed(() => bodyAny.value.fields as MultipartField[])

const BODY_TABS = computed<SegmentOption[]>(() => [
  { value: 'none', label: t('body.none') },
  { value: 'form-data', label: 'form-data' },
  { value: 'x-www-form-urlencoded', label: 'x-www-form-urlencoded' },
  { value: 'raw', label: 'raw' },
  { value: 'binary', label: 'binary' },
  { value: 'graphql', label: 'GraphQL' },
])

const RAW_SUBTYPE_OPTIONS = RAW_SUBTYPES.map((s) => ({ value: s.value, label: s.label }))

/** 请求体 JSON 校验状态（JsonEditor 外报，标签渲染在模式栏右侧）。 */
type JsonStatus = 'empty' | 'ok' | 'invalid' | 'large'
const jsonStatus = ref<JsonStatus>('empty')
const jsonStatusText = computed(
  () =>
    ({ ok: t('jsonEditor.ok'), invalid: t('jsonEditor.invalid'), large: t('jsonEditor.large') })[
      jsonStatus.value as 'ok' | 'invalid' | 'large'
    ] ?? '',
)
const jsonStatusHint = computed(
  () =>
    (
      {
        ok: t('jsonEditor.okHint'),
        invalid: t('jsonEditor.invalidHint'),
        large: t('jsonEditor.largeHint'),
      }
    )[jsonStatus.value as 'ok' | 'invalid' | 'large'] ?? '',
)

/** form-data ↔ urlencoded 可互转：直接切换时保留字段实时转换（用户要求保留）。 */
function isConvertiblePair(from: BodyTab, to: BodyTab): boolean {
  return (
    (from === 'form-data' && to === 'x-www-form-urlencoded') ||
    (from === 'x-www-form-urlencoded' && to === 'form-data')
  )
}

/** 把防抖镜像里未落盘的键入刷进当前 body（= 当前页签自己的草稿）。 */
function flushPendingInputs(): void {
  rawText.flush()
  gqlQuery.flush()
  gqlOp.flush()
  binPath.flush()
}

const activeTab = computed({
  get: () => tabOf((bodyAny.value ?? { mode: 'none' }) as BodySpec, headersAny.value ?? []),
  set: (tab: string) => {
    const d = props.draft
    if (!d) return
    const next = tab as BodyTab
    const prev = activeTab.value
    if (next === prev) return
    // 先刷出未落盘的键入（写进的正是当前页签的草稿对象），再切引用
    flushPendingInputs()
    const headers = d.request.headers ?? []
    setSlot(d, prev, d.request.body as BodySpec)
    // Content-Type 是页签自己的数据（cURL 导入的 urlencoded 等不能被模式默认值抹掉）
    rememberTabContentType(d, prev, headers)
    if (isConvertiblePair(prev, next)) {
      // 表单两种类型互转：沿用 applyBodyTab 的字段转换与表单 MIME
      applyBodyTab(d.request, next)
      setSlot(d, next, d.request.body as BodySpec)
      rememberTabContentType(d, next, headers)
    } else {
      // 其余只切到当前页签自己的那份草稿：发送内容恒 = 当前选中类型的参数
      d.request.body = slotOf(d, next)
      restoreTabContentType(d, next, headers)
    }
  },
})

// 外部整体替换 request.body（用例回填 / 撤销 / 导入）后，同步当前页签草稿。
watch(
  () => props.draft?.request.body,
  (body) => {
    const d = props.draft
    if (d && body) setSlot(d, activeTab.value, body as BodySpec)
  },
)

const rawSubtype = computed({
  get: () => (props.draft ? draftsOf(props.draft).rawSubtype : 'json'),
  set: (subtype: string) => {
    const d = props.draft
    if (!d) return
    const drafts = draftsOf(d)
    drafts.rawSubtype = subtype as RawSubtype
    // raw 文本跨子类型保留（json 模式 / text 模式）+ 同步 Content-Type
    applyRawSubtype(d.request, drafts.rawSubtype)
    setSlot(d, 'raw', d.request.body as BodySpec)
    // 子类型变了，raw 页签的 Content-Type 也跟着归档，切走再回来能原样恢复
    rememberTabContentType(d, 'raw', d.request.headers ?? [])
  },
})

const RAW_PLACEHOLDER = computed<Record<RawSubtype, string>>(() => ({
  json: '{ "key": "value" }',
  text: t('body.textPh'),
  javascript: '// JavaScript',
  html: '<!DOCTYPE html>\n<html>…</html>',
  xml: '<?xml version="1.0" encoding="UTF-8"?>\n<root>…</root>',
}))

const MULTIPART_TYPE_OPTIONS = computed(() => [
  { value: 'text', label: t('body.textType') },
  { value: 'file_path', label: t('body.filePathType') },
])

// ---------- 查找（Find in Request Body） ----------
const panelRef = ref<HTMLElement | null>(null)
const rawTextareaRef = ref<HTMLTextAreaElement | null>(null)
const gqlTextareaRef = ref<HTMLTextAreaElement | null>(null)

const jsonTotal = ref(0)

const total = computed(() => {
  if (activeTab.value === 'raw' && rawSubtype.value === 'json') {
    return jsonTotal.value
  }
  if (activeTab.value === 'raw') {
    return countOccurrences(bodyAny.value?.raw ?? '', searchQuery.value)
  }
  if (activeTab.value === 'graphql') {
    return countOccurrences(graphql.value?.query ?? '', searchQuery.value)
  }
  return 0
})

// 查找状态机（开关 / 防抖 / 导航）走 composables/useFindInText；JSON 模式
// 计数由 CodeMirror 高亮管线（JsonEditor）经 watch 回写 jsonTotal。
const {
  findOpen,
  query,
  searchQuery,
  activeMatch,
  nextMatch,
  prevMatch,
  closeFind,
  toggleFind,
} = useFindInText(total, {
  onClose: () => {
    jsonTotal.value = 0
  },
})

function getMatchOffsets(text: string, q: string, targetIdx: number): [number, number] | null {
  if (!q) return null
  const ql = q.toLowerCase()
  const lower = text.toLowerCase()
  let from = 0
  let cur = 0
  for (;;) {
    const idx = lower.indexOf(ql, from)
    if (idx === -1) return null
    if (cur === targetIdx) return [idx, idx + q.length]
    cur += 1
    from = idx + q.length
  }
}

watch(
  () => [searchQuery.value, activeMatch.value],
  ([q, matchIdx]) => {
    if (!q || matchIdx === undefined) return
    if (activeTab.value === 'raw' && rawSubtype.value === 'json') return
    const ta = activeTab.value === 'graphql' ? gqlTextareaRef.value : rawTextareaRef.value
    if (!ta) return
    const text = ta.value
    const offsets = getMatchOffsets(text, String(q), Number(matchIdx))
    if (!offsets) return
    const [start, end] = offsets
    ta.focus()
    ta.setSelectionRange(start, end)
  },
)

/** 代码区 Tab / Shift+Tab 缩进而非跳出焦点（与 JsonEditor / CodeMirror 一致）。 */
function onCodeKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab') handleTextareaTab(e.target as HTMLTextAreaElement, e)
}

function onWindowKeydown(e: KeyboardEvent): void {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'f') return
  const target = e.target as HTMLElement | null
  if (target?.closest('.findbar, .sidebar-search, .docs-search')) return

  const root = panelRef.value
  const isInside = !!(root && target && root.contains(target))
  if (isInside) {
    e.preventDefault()
    findOpen.value = true
  }
}

onMounted(() => {
  window.addEventListener('keydown', onWindowKeydown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onWindowKeydown)
})

/** urlencoded 字段表：与 Params/Headers 一致的 KeyValueTable 幽灵行（输入自动补行）。 */
function applyUrlencoded(rows: KVRow[]): void {
  const body = props.draft?.request.body as { fields: KeyValue[] } | undefined
  body?.fields.splice(0, body.fields.length, ...(rows as KeyValue[]))
}

function addMultipartField(): void {
  const fields = props.draft?.request.body as { fields: MultipartField[] } | undefined
  fields?.fields.push({ key: '', value_type: 'text', value: '', enabled: true })
}

function removeMultipartField(index: number): void {
  const fields = props.draft?.request.body as { fields: MultipartField[] } | undefined
  fields?.fields.splice(index, 1)
}

/**
 * 纯文本输入的本地镜像 + 防抖回写：`v-model` 直写草稿会让每键触发
 * Pinia 全量 deep watch + dirty 定版（遍历全部打开标签的草稿）。
 * 此处 150ms 防抖回写 store；失焦 / 切接口 / 卸载时强制刷出。
 * 注：切接口前浏览器必先触发 blur（点击驱动），故 blur 刷出足以覆盖；
 * 程序化切换的极端竞态下最多丢 150ms 内的未落盘键入。
 */
function useDebouncedField(
  get: () => string,
  set: (v: string) => void,
  /** 同步触发源：切接口 / 切换 body subtype / store 侧外部写入（如用例回填）。 */
  syncKey: () => unknown,
) {
  const local = ref(get())
  let timer: ReturnType<typeof setTimeout> | undefined
  function flush(): void {
    if (timer) {
      clearTimeout(timer)
      timer = undefined
    }
    const v = local.value
    if (v !== get()) set(v)
  }
  function onInput(v: string): void {
    local.value = v
    if (timer) clearTimeout(timer)
    timer = setTimeout(flush, 150)
  }
  // 同步时 props 已是新值，只能丢弃 pending 并同步。真实交互必先经过 blur
  //（点击驱动切换），@blur 已把旧值刷回旧草稿；程序化切换的极端竞态下
  // 最多丢 150ms 内的未落盘键入。
  watch(syncKey, () => {
    if (timer) {
      clearTimeout(timer)
      timer = undefined
    }
    local.value = get()
  })
  onBeforeUnmount(flush)
  return { local, onInput, flush }
}

const rawSyncKey = () => [
  props.draft?.id,
  activeTab.value,
  rawSubtype.value,
  (bodyAny.value as unknown as { raw?: string } | undefined)?.raw,
]
const rawText = useDebouncedField(
  () => (bodyAny.value as unknown as { raw?: string } | undefined)?.raw ?? '',
  (v) => {
    const b = bodyAny.value as unknown as { raw?: string } | undefined
    if (b) b.raw = v
  },
  rawSyncKey,
)
const gqlSyncKey = () => [
  props.draft?.id,
  graphql.value?.query,
  graphql.value?.operation_name,
]
const gqlQuery = useDebouncedField(
  () => graphql.value?.query ?? '',
  (v) => {
    if (graphql.value) graphql.value.query = v
  },
  gqlSyncKey,
)
const gqlOp = useDebouncedField(
  () => graphql.value?.operation_name ?? '',
  (v) => {
    if (graphql.value) graphql.value.operation_name = v
  },
  gqlSyncKey,
)
const binPath = useDebouncedField(
  () => (bodyAny.value as unknown as { path?: string } | undefined)?.path ?? '',
  (v) => {
    const b = bodyAny.value as unknown as { path?: string } | undefined
    if (b) b.path = v
  },
  () => [props.draft?.id, (bodyAny.value as unknown as { path?: string } | undefined)?.path],
)

/** 暴露给 EndpointEditor：save/send（⌘S / ⌘Enter，inInput 快捷键不经过 blur）
 *  等读草稿的动作在取值前统一刷出防抖镜像，避免静默丢最后 150ms 键入。 */
defineExpose({
  flushPendingInput: (): void => {
    rawText.flush()
    gqlQuery.flush()
    gqlOp.flush()
    binPath.flush()
  },
})
</script>

<template>
  <div ref="panelRef" class="panel">
    <div class="mode-bar">
      <div class="mode-bar-left">
        <SegmentedControl v-model="activeTab" :options="BODY_TABS" size="sm" class="mode-tabs" />
        <CustomSelect
          v-if="activeTab === 'raw'"
          v-model="rawSubtype"
          :options="RAW_SUBTYPE_OPTIONS"
          size="sm"
          class="raw-subtype"
          pop-class="raw-subtype-pop"
          pop-auto-width
        />
      </div>
      <div class="mode-bar-right">
        <!-- JSON 校验状态：从编辑器工具栏外移到模式栏右侧，不占编辑区行位。
             三重守卫：status 缺失（热更失步）时整枚不渲染，杜绝裸点 -->
        <span
          v-if="activeTab === 'raw' && rawSubtype === 'json' && jsonStatus && jsonStatus !== 'empty'"
          class="bp-json-status"
          :class="jsonStatus"
          :title="jsonStatusHint"
        >
          <Icon v-if="jsonStatus === 'ok' || jsonStatus === 'invalid'" :name="jsonStatus === 'ok' ? 'check' : 'x'" :size="11" />
          <span v-else-if="jsonStatus === 'large'" class="bp-json-dot" aria-hidden="true"></span>
          <template v-if="jsonStatusText">{{ jsonStatusText }}</template>
        </span>
        <button
          v-if="activeTab === 'raw' || activeTab === 'graphql'"
          class="bp-icon-btn"
          :class="{ active: findOpen }"
          type="button"
          :title="t('body.findHint')"
          :aria-label="t('body.findHint')"
          @click="toggleFind"
        >
          <Icon name="search" :size="13" />
        </button>
      </div>
    </div>

    <FindBar
      v-if="findOpen && (activeTab === 'raw' || activeTab === 'graphql')"
      v-model:query="query"
      :index="activeMatch"
      :total="total"
      :placeholder="t('body.findPh')"
      @prev="prevMatch"
      @next="nextMatch"
      @close="closeFind"
    />

    <JsonEditor
      v-if="activeTab === 'raw' && rawSubtype === 'json'"
      v-model="bodyAny.raw"
      placeholder='{ "key": "value" }'
      :min-height="160"
      :max-height="480"
      :show-status="false"
      :query="findOpen ? searchQuery : ''"
      :active-match="activeMatch"
      @status-change="jsonStatus = $event"
      @match-count="jsonTotal = $event"
    />
    <textarea
      v-else-if="activeTab === 'raw'"
      :value="rawText.local.value"
      ref="rawTextareaRef"
      class="miyo-input body-input"
      spellcheck="false"
      :placeholder="RAW_PLACEHOLDER[rawSubtype as RawSubtype] ?? t('body.textPh')"
      @input="rawText.onInput(($event.target as HTMLTextAreaElement).value)"
      @blur="rawText.flush()"
      @keydown="onCodeKeydown"
    ></textarea>

    <div v-else-if="activeTab === 'graphql'" class="gql-editor">
      <textarea
        :value="gqlQuery.local.value"
        ref="gqlTextareaRef"
        class="miyo-input body-input"
        spellcheck="false"
        placeholder="query Hero($id: ID!) { hero(id: $id) { name } }"
        @input="gqlQuery.onInput(($event.target as HTMLTextAreaElement).value)"
        @blur="gqlQuery.flush()"
        @keydown="onCodeKeydown"
      ></textarea>
      <JsonEditor
        v-model="graphql.variables"
        placeholder='{ "id": "42" }'
        :min-height="80"
        :max-height="320"
        :auto-height="true"
        :query="findOpen ? searchQuery : ''"
        :active-match="activeMatch"
      />
      <input
        :value="gqlOp.local.value"
        class="miyo-input miyo-input-sm"
        :placeholder="t('body.operationNamePh')"
        @input="gqlOp.onInput(($event.target as HTMLInputElement).value)"
        @blur="gqlOp.flush()"
      />
    </div>

    <KeyValueTable
      v-else-if="activeTab === 'x-www-form-urlencoded'"
      :model-value="urlencodedFields"
      @update:model-value="applyUrlencoded"
    />

    <div v-else-if="activeTab === 'form-data'" class="mp-table">
      <div class="mp-head">
        <span class="mp-col mp-check"></span>
        <span class="mp-col mp-key miyo-mono">{{ t('kv.colKey') }}</span>
        <span class="mp-col mp-type miyo-mono">{{ t('body.colType') }}</span>
        <span class="mp-col mp-value miyo-mono">{{ t('kv.colValue') }}</span>
        <span class="mp-col mp-actions"></span>
      </div>
      <div
        v-for="(f, i) in multipartFields"
        :key="i"
        class="mp-row"
        :class="{ off: f.enabled === false }"
      >
        <span class="mp-col mp-check">
          <input
            v-model="f.enabled"
            type="checkbox"
            class="mp-check-box"
            :aria-label="f.key ? t('body.fieldToggleKey', { v: f.key }) : t('body.fieldToggle')"
          />
        </span>
        <input v-model="f.key" class="mp-input mp-col mp-key" :placeholder="t('kv.colKey')" spellcheck="false" />
        <CustomSelect
          v-model="f.value_type"
          :options="MULTIPART_TYPE_OPTIONS"
          size="sm"
          class="mp-type-select"
        />
        <input
          v-model="f.value"
          class="mp-input mp-col mp-value"
          :placeholder="f.value_type === 'file_path' ? '/path/to/file' : t('kv.colValue')"
          spellcheck="false"
          @keydown.enter.prevent="addMultipartField"
        />
        <span class="mp-col mp-actions">
          <IconButton name="trash" :size="13" tone="danger" :title="t('common.delete')" @click="removeMultipartField(i)" />
        </span>
      </div>
      <button class="mp-add" type="button" @click="addMultipartField">
        <Icon name="plus" :size="13" /> {{ t('body.addField') }}
      </button>
    </div>

    <div v-else-if="activeTab === 'binary'" class="binary-box">
      <label class="binary-label">
        <Icon name="upload" :size="14" /> {{ t('body.filePath') }}
      </label>
      <input
        :value="binPath.local.value"
        class="miyo-input miyo-input-sm binary-input"
        spellcheck="false"
        :placeholder="t('body.filePathPh')"
        @input="binPath.onInput(($event.target as HTMLInputElement).value)"
        @blur="binPath.flush()"
      />
      <p class="binary-hint">
        {{ t('body.binaryHint') }}
      </p>
    </div>

    <p v-else class="body-hint">{{ t('body.noBody') }}</p>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  min-height: 0;
  /* 高度受限（响应出现后 config-box 固定高）时编辑器内容不得溢出到
   * 相邻的请求设置行上（cm-editor 为 positioned 元素，会盖住行内文本） */
  overflow: hidden;
}

/* 编辑器圆角统一 rounded-lg（--radius-md）：请求/响应两区代码块同为完整圆角卡片 */

.mode-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
}

.mode-bar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.mode-bar-right {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

/* JSON 校验状态徽标（编辑器外部渲染）：胶囊 Badge 档（text-xs + px-2.5 py-1），
 * 语义色底/字；语法错误态用红系实感 Badge（红字向白提亮，浅色主题保持原红保对比） */
.bp-json-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 10px;
  border-radius: 9999px;
  font-size: var(--fs-xs);
  font-weight: 500;
  line-height: 1.4;
  white-space: nowrap;
}
.bp-json-status.ok {
  background: var(--success-tint);
  color: var(--success);
}
.bp-json-status.invalid {
  background: color-mix(in srgb, var(--danger) 16%, transparent);
  /* 危险态文字走 --danger-fg（暗底提亮 / 浅底本色） */
  color: var(--danger-fg);
}
.bp-json-status.large {
  background: var(--warning-tint);
  color: var(--warning);
}
.bp-json-dot {
  width: 4px;
  height: 4px;
  border-radius: 9999px;
  background: currentColor;
}

.bp-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 6px);
  background: var(--bg-card);
  color: var(--text-2);
  cursor: pointer;
  transition: all var(--dur) var(--ease);
}
.bp-icon-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
  border-color: var(--border-strong);
}
.bp-icon-btn.active {
  background: var(--accent-tint, rgba(168, 85, 247, 0.15));
  color: var(--accent, #a855f7);
  border-color: var(--accent, #a855f7);
}

.mode-tabs {
  flex: 0 1 auto;
  min-width: 0;
}

.raw-subtype {
  width: 130px;
  flex-shrink: 0;
}

.body-input {
  width: 100%;
  min-height: 120px;
  flex: 1;
  font-family: var(--font-mono);
  font-size: 12.5px;
  resize: vertical;
  /* 与 JsonEditor 代码块同圆角档（rounded-md） */
  border-radius: var(--radius);
}

.gql-editor {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-height: 0;
}

/* ---- form-data 字段表：与 KeyValueTable 同视觉语言（表头 + 行分隔 + 悬停高亮） ---- */
.mp-table {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}

.mp-head,
.mp-row {
  display: flex;
  align-items: center;
}

.mp-head {
  height: 28px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
  font-size: var(--fs-xxs);
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-3);
}

.mp-row {
  min-height: 32px;
  border-bottom: 1px solid var(--border);
  transition: background var(--dur) var(--ease);
}
.mp-row:last-of-type {
  border-bottom: none;
}
.mp-row:hover {
  background: var(--bg-hover);
}
.mp-row.off .mp-input {
  opacity: 0.45;
}

.mp-col {
  flex-shrink: 0;
}
.mp-check {
  width: 36px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.mp-key {
  width: 34%;
  min-width: 120px;
}
.mp-type {
  width: 110px;
}
.mp-value {
  flex: 1;
  min-width: 0;
}
.mp-actions {
  width: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  opacity: 0;
  transition: opacity var(--dur) var(--ease);
}
.mp-row:hover .mp-actions,
.mp-row:focus-within .mp-actions {
  opacity: 1;
}

.mp-check-box {
  accent-color: var(--accent);
  cursor: pointer;
}

.mp-type-select {
  width: 96px;
  flex-shrink: 0;
  margin-right: 6px;
}

.mp-input {
  height: 32px;
  border: none;
  background: transparent;
  color: var(--text-1);
  font-size: 12px;
  outline: none;
  padding: 0 8px;
  min-width: 0;
}
.mp-input::placeholder {
  color: var(--text-3);
}
.mp-input:focus {
  background: var(--bg-elevated);
  box-shadow: inset 0 0 0 1px var(--accent);
}

/* 添加行：轻量虚线整行按钮（hover 提亮为主色），替代笨重的实心通栏按钮 */
.mp-add {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  border: none;
  border-top: 1px dashed var(--border);
  background: transparent;
  color: var(--text-3);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition: color var(--dur) var(--ease), background var(--dur) var(--ease);
}
.mp-add:hover {
  color: var(--accent);
  background: var(--accent-tint);
}

.binary-box {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-height: 0;
}

.binary-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-2);
}

.binary-label svg {
  color: var(--accent);
}

.binary-input {
  width: 100%;
  font-family: var(--font-mono);
}

.binary-hint {
  margin: 0;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-3);
}

.body-hint {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}
</style>
