<script setup lang="ts">
/**
 * DesignPanel：接口设计（ApiDesign）——API 契约的可视化定义。
 *
 * - 顶部固定操作栏：右侧「保存设计」主按钮 + 未保存黄点提醒（保存逻辑由
 *   EndpointEditor 提供：未命名接口先弹名称/位置确认框）；
 * - 基本信息卡片：名称 / 状态双列网格，Method + Path 组合输入组，描述；
 * - 请求定义：Params / Headers / Body 三 Tab，参数表支持
 *   参数名 | 类型 | 必填 | 说明 | 示例值（KeyValue 设计元数据随 request_json 持久化）；
 * - 返回响应 (Responses)：按状态码维护响应示例（复用 response_examples 存储）；
 * - 右侧实时预览栏：Schema 结构 / Mock 示例双视图，随草稿修改即时刷新。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import ParamDefineTable from './design/ParamDefineTable.vue'
import JsonStructEditor from './design/JsonStructEditor.vue'
import CustomSelect from './ui/CustomSelect.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Popconfirm from './ui/Popconfirm.vue'
import SegmentedControl from './ui/SegmentedControl.vue'
import Tabs from './ui/Tabs.vue'
import type { TabItem } from './ui/Tabs.vue'
import Tooltip from './ui/Tooltip.vue'
import { highlightJSON } from '../utils/highlight'
import { handleTextareaTab } from '../utils/textareaIndent'
import { statusTextOf } from '../utils/testCases'
import { inferSchema, mockJsonFromSchema } from '../utils/schemaInfer'
import type { SchemaRow } from '../utils/schemaInfer'
import type {
  BodySpec,
  Endpoint,
  EndpointStatus,
  FieldDoc,
  HttpMethod,
  KeyValue,
  MultipartField,
  ResponseExample,
} from '../types/miyoApi'

const props = defineProps<{ draft: Endpoint | null }>()

const emit = defineEmits<{ save: [] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const d = computed(() => props.draft)

/** 草稿是否含未保存修改（顶部黄点）。 */
const dirty = computed(() => (d.value ? store.isDirty(d.value.id) : false))

const METHODS: HttpMethod[] = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']
const METHOD_OPTIONS = METHODS.map((m) => ({ value: m, label: m }))

const STATUS_OPTIONS = computed<{ value: EndpointStatus; label: string }[]>(() => [
  { value: 'designing', label: t('design.statusDesigning') },
  { value: 'developing', label: t('design.statusDeveloping') },
  { value: 'testing', label: t('design.statusTesting') },
  { value: 'released', label: t('design.statusReleased') },
  { value: 'deprecated', label: t('design.statusDeprecated') },
])

function onMethodChange(v: string | number): void {
  const target = d.value
  if (target) target.method = String(v) as HttpMethod
}

function onStatusChange(v: string | number): void {
  const target = d.value
  if (target) target.status = String(v) as EndpointStatus
}

// ---------- 请求参数定义（Params / Headers / Body） ----------

type ReqTabKey = 'params' | 'headers' | 'body'

const reqTab = ref<ReqTabKey>('params')

const reqTabs = computed<TabItem[]>(() => [
  { key: 'params', label: t('design.tabParams'), count: d.value?.request.params.length ?? 0 },
  { key: 'headers', label: t('design.tabHeaders'), count: d.value?.request.headers.length ?? 0 },
  { key: 'body', label: t('design.tabBody') },
])

function writeRows(list: KeyValue[], rows: KeyValue[]): void {
  list.splice(0, list.length, ...rows)
}

function onParamsUpdate(rows: KeyValue[]): void {
  if (d.value) writeRows(d.value.request.params, rows)
}

function onHeadersUpdate(rows: KeyValue[]): void {
  if (d.value) writeRows(d.value.request.headers, rows)
}

/** Form Data 字段表更新（urlencoded 容器写回）。 */
function onFormUpdate(rows: KeyValue[]): void {
  const target = d.value
  if (!target) return
  const body = target.request.body
  if (body.mode === 'urlencoded') writeRows(body.fields, rows)
}

/** Body 设计视图模式：仅支持 JSON / Form Data；其余类型引导到调试页。 */
type BodyViewMode = 'json' | 'form'

const bodyMode = computed<BodyViewMode | ''>(() => {
  const mode = d.value?.request.body.mode
  return mode === 'json' ? 'json' : mode === 'urlencoded' ? 'form' : ''
})

const BODY_MODE_OPTIONS = [
  { value: 'json', label: 'JSON' },
  { value: 'form', label: 'Form Data' },
]

/** 旧 Body → JSON 文本（模式切换尽量搬数据，解析不出则给默认模板）。 */
function bodyToJsonRaw(old: BodySpec): string {
  if (old.mode === 'json' || old.mode === 'text') return old.raw || '{\n  \n}'
  if (old.mode === 'urlencoded' || old.mode === 'multipart') {
    const obj: Record<string, string> = {}
    for (const f of old.fields) {
      if (f.enabled && f.key.trim()) obj[f.key] = f.value ?? ''
    }
    return Object.keys(obj).length ? JSON.stringify(obj, null, 2) : '{\n  \n}'
  }
  return '{\n  \n}'
}

/** 旧 Body → Form Data 字段（json 文本可解析出对象时逐键搬移）。 */
function bodyToFormFields(old: BodySpec): KeyValue[] {
  if (old.mode === 'urlencoded') return old.fields.map((f) => ({ ...f }))
  if (old.mode === 'multipart') {
    return old.fields.map((f) => ({
      key: f.key,
      value: f.value,
      enabled: f.enabled,
      description: '',
    }))
  }
  if (old.mode === 'json' || old.mode === 'text') {
    try {
      const parsed: unknown = JSON.parse(old.raw.trim() || '{}')
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        return Object.entries(parsed as Record<string, unknown>).map(([key, value]) => ({
          key,
          value: typeof value === 'string' ? value : JSON.stringify(value),
          enabled: true,
          description: '',
        }))
      }
    } catch {
      // 非法 JSON → 空字段容器
    }
  }
  return []
}

/** 切换 Body 设计模式：json ↔ urlencoded，旧内容互搬不丢数据。 */
function onBodyModeChange(v: string): void {
  const body = d.value?.request.body
  if (!body) return
  const current: BodyViewMode | '' =
    body.mode === 'json' ? 'json' : body.mode === 'urlencoded' ? 'form' : ''
  if (v === current) return
  if (v === 'json') {
    d.value!.request.body = { mode: 'json', raw: bodyToJsonRaw(body) }
  } else if (v === 'form') {
    d.value!.request.body = { mode: 'urlencoded', fields: bodyToFormFields(body) }
  }
}

const BODY_VIEW_HINTS = computed<Record<string, string>>(() => ({
  text: t('design.bodyHintText'),
  graphql: t('design.bodyHintGraphql'),
  multipart: t('design.bodyHintMultipart'),
  binary: t('design.bodyHintBinary'),
  none: t('design.bodyHintNone'),
}))

/** JSON 编辑区内容（直接写草稿 raw）。 */
const bodyRaw = computed({
  get: () => {
    const body = d.value?.request.body
    return body?.mode === 'json' ? body.raw : ''
  },
  set: (v: string) => {
    const body = d.value?.request.body
    if (body?.mode === 'json') body.raw = v
  },
})

const jsonValid = computed(() => {
  if (bodyMode.value !== 'json') return true
  const text = bodyRaw.value.trim()
  if (!text) return true
  try {
    JSON.parse(text)
    return true
  } catch {
    return false
  }
})

// ---------- JSON 结构 / 文本双模式（结构编辑器默认） ----------

type JsonEditView = 'struct' | 'text'

/** 请求 Body JSON 视图（struct 默认；非法对象根回退 text）。 */
const bodyJsonView = ref<JsonEditView>('struct')

/** 响应 Body JSON 视图（struct 默认）。 */
const respJsonView = ref<JsonEditView>('struct')

const JSON_VIEW_OPTIONS = computed(() => [
  { value: 'struct', label: t('design.structMode') },
  { value: 'text', label: t('design.textMode') },
])

/** 请求 body_docs（随草稿持久化）。 */
const bodyDocs = computed({
  get: (): Record<string, FieldDoc> => d.value?.request.body_docs ?? {},
  set: (v: Record<string, FieldDoc>) => {
    if (d.value) d.value.request.body_docs = v
  },
})

/** 结构编辑器写回草稿 raw。 */
function onBodyStructRaw(raw: string): void {
  bodyRaw.value = raw
}

function onBodyStructDocs(docs: Record<string, FieldDoc>): void {
  bodyDocs.value = docs
}

/** 文本 → 结构：非法 JSON / 非对象根时提示并保持文本模式。 */
function switchJsonView(next: string): void {
  if (next === bodyJsonView.value) return
  if (next === 'struct') {
    const text = bodyRaw.value.trim()
    if (text) {
      try {
        const parsed: unknown = JSON.parse(text)
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
          toast.warning(t('design.structNeedObject'))
          return
        }
      } catch {
        toast.warning(t('design.structParseFail'))
        return
      }
    }
  }
  bodyJsonView.value = next as JsonEditView
}

// ---------- 返回响应 (Responses)：复用响应示例存储 ----------

const responses = computed(() =>
  d.value ? (store.examples.get(d.value.id) ?? []) : [],
)

const expandedRespId = ref<string | null>(null)

/** 默认展开首个成功响应（无 2xx 时退回第一条），删除后自动收起。 */
watch(
  responses,
  (list) => {
    if (expandedRespId.value && !list.some((e) => e.id === expandedRespId.value)) {
      expandedRespId.value = null
    }
    if (!expandedRespId.value && list.length) {
      expandedRespId.value = (list.find((e) => e.status < 300) ?? list[0]).id
    }
  },
  { immediate: true },
)

/** 展开示例的编辑文本（未改动时回退格式化后的已存正文）。 */
const respEdits = ref<Map<string, string>>(new Map())

/** 展开示例的结构化 docs 编辑缓冲（未保存；saveRespBody 一并提交）。 */
const respDocsEdits = ref<Map<string, Record<string, FieldDoc>>>(new Map())

function respTextOf(ex: ResponseExample): string {
  const editing = respEdits.value.get(ex.id)
  if (editing !== undefined) return editing
  try {
    return JSON.stringify(JSON.parse(ex.body), null, 2)
  } catch {
    return ex.body
  }
}

/** 展示名：去掉与状态徽标重复的状态码前缀（「200 响应」→「响应」，「200 OK」→「OK」）。 */
function respDisplayName(ex: ResponseExample): string {
  const base = ex.name || `${ex.status} ${statusTextOf(ex.status)}`
  const stripped = base.replace(new RegExp(`^${ex.status}\\s+`), '')
  return stripped || base
}

function onRespEdit(ex: ResponseExample, v: string): void {
  respEdits.value.set(ex.id, v)
}

/** 结构编辑器写响应 body 文本。 */
function onRespStructRaw(ex: ResponseExample, raw: string): void {
  respEdits.value.set(ex.id, raw)
}

/** 结构编辑器写响应 docs 缓冲。 */
function onRespStructDocs(ex: ResponseExample, docs: Record<string, FieldDoc>): void {
  respDocsEdits.value.set(ex.id, docs)
}

/** 结构模式下展开示例的正文（未改动回退已存 body 格式化）。 */
function respStructRawOf(ex: ResponseExample): string {
  const editing = respEdits.value.get(ex.id)
  if (editing !== undefined) return editing
  try {
    return JSON.stringify(JSON.parse(ex.body), null, 2)
  } catch {
    return ex.body
  }
}

/** JSON 编辑区 Tab 缩进而非跳出焦点。 */
function onCodeKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab') handleTextareaTab(e.target as HTMLTextAreaElement, e)
}

function toggleResp(id: string): void {
  expandedRespId.value = expandedRespId.value === id ? null : id
}

/** 响应行时间：ISO → 本地时区「YYYY-MM-DD HH:mm」（原始 slice 会显示 UTC）。 */
function respTimeOf(iso: string): string {
  const dt = new Date(iso)
  if (Number.isNaN(dt.getTime())) return ''
  const p = (n: number): string => String(n).padStart(2, '0')
  return `${dt.getFullYear()}-${p(dt.getMonth() + 1)}-${p(dt.getDate())} ${p(dt.getHours())}:${p(dt.getMinutes())}`
}

/** 切换接口草稿时清空未保存的响应编辑缓冲。 */
watch(
  () => d.value?.id,
  () => {
    respEdits.value.clear()
    respDocsEdits.value.clear()
  },
)

async function cacheExample(saved: ResponseExample): Promise<void> {
  if (!d.value) return
  const list = store.examples.get(d.value.id) ?? []
  const idx = list.findIndex((x) => x.id === saved.id)
  if (idx === -1) list.unshift(saved)
  else list[idx] = saved
  store.examples.set(d.value.id, [...list])
}

/** 新增状态码响应行（自定义输入）。 */
const newRespStatus = ref<number | null>(200)
const newRespName = ref('')

/** 标题行快捷创建按键：常用状态码一键建档。 */
const STATUS_PRESETS: { status: number; label: string }[] = [
  { status: 200, label: '+ 200 OK' },
  { status: 400, label: '+ 400 Bad Request' },
  { status: 500, label: '+ 500 Error' },
]

async function addResponse(preset?: number): Promise<void> {
  const endpointId = d.value?.id
  if (!endpointId) return
  // 草稿未落库时 response_examples 外键会拒写，先引导保存接口。
  if (!store.endpoints.some((e) => e.id === endpointId)) {
    toast.warning(t('ws.saveEndpointFirst'))
    return
  }
  // .number 修饰符在输入框清空时可能落回空串，统一 Number 归一。
  const status = Number(preset ?? newRespStatus.value)
  if (!status || status < 100 || status > 599) {
    toast.warning(t('design.statusRange'))
    return
  }
  // 同状态码只保留一条（预览按 status 键控，重复会静默覆盖）。
  if (responses.value.some((e) => e.status === status)) {
    toast.warning(t('design.statusDup', { v: status }))
    return
  }
  try {
    const now = new Date().toISOString()
    const saved = await api.saveExample({
      id: crypto.randomUUID(),
      endpoint_id: endpointId,
      // 预设按键不消费自定义名称输入框里已输入的文字，仅自定义添加时取用。
      name:
        (preset === undefined ? newRespName.value.trim() : '') ||
        t('design.respDefaultName', { v: status }),
      status,
      headers: {},
      body: '',
      content_type: 'application/json',
      created_at: now,
      updated_at: now,
    })
    await cacheExample(saved)
    if (preset === undefined) {
      newRespStatus.value = 200
      newRespName.value = ''
    }
    expandedRespId.value = saved.id
    toast.success(t('design.respAdded', { v: status }))
  } catch (err) {
    toast.error(t('design.addFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

async function saveRespBody(ex: ResponseExample): Promise<void> {
  const text = respEdits.value.get(ex.id)
  const docs = respDocsEdits.value.get(ex.id)
  if (text === undefined && docs === undefined) return
  const body = text ?? ex.body
  try {
    if (body.trim()) JSON.parse(body) // 仅校验，不强制格式化
  } catch {
    toast.warning(t('design.invalidJsonHint'))
  }
  try {
    const saved = await api.saveExample({
      ...ex,
      body,
      ...(docs !== undefined ? { docs } : {}),
      updated_at: new Date().toISOString(),
    })
    await cacheExample(saved)
    respEdits.value.delete(ex.id)
    respDocsEdits.value.delete(ex.id)
    toast.success(t('design.respUpdated', { v: ex.status }))
  } catch (err) {
    toast.error(t('design.saveFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

async function removeResp(ex: ResponseExample): Promise<void> {
  if (!d.value) return
  try {
    await store.removeExample(d.value.id, ex.id)
    if (expandedRespId.value === ex.id) expandedRespId.value = null
  } catch (err) {
    toast.error(t('design.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

// ---------- 右侧实时预览：Schema 结构 / Mock 示例 ----------

type PreviewView = 'schema' | 'mock'

const previewView = ref<PreviewView>('schema')

const PREVIEW_VIEW_OPTIONS = computed(() => [
  { value: 'schema', label: 'Schema' },
  { value: 'mock', label: t('design.mockExample') },
])

/** 叶子 Schema 行 → 类型标注（schema 视图）或示例值（mock 视图）。 */
function shapeOf(rows: SchemaRow[], mock: boolean): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const row of rows) {
    const key = row.name.trim()
    if (!key) continue
    if (row.type === 'object') {
      out[key] = shapeOf(row.children, mock)
    } else if (row.type === 'array') {
      out[key] =
        row.itemType === 'object' ? [shapeOf(row.children, mock)] : [`<${row.itemType ?? 'any'}>`]
    } else {
      out[key] = mock ? row.example || row.type : row.type
    }
  }
  return out
}

/** 按 JSON Pointer 路径给 shape 叶子挂 description（schema 视图）。 */
function applyDocsToShape(
  shape: Record<string, unknown>,
  docs: Record<string, FieldDoc> | undefined,
  prefix = '',
): void {
  if (!docs) return
  for (const [key, val] of Object.entries(shape)) {
    const ptr = `${prefix}/${key.replace(/~/g, '~0').replace(/\//g, '~1')}`
    const doc = docs[ptr]
    if (doc?.description) {
      if (val && typeof val === 'object' && !Array.isArray(val)) {
        applyDocsToShape(val as Record<string, unknown>, docs, ptr)
      } else {
        shape[key] = `${String(val)} // ${doc.description}`
      }
    } else if (val && typeof val === 'object' && !Array.isArray(val)) {
      applyDocsToShape(val as Record<string, unknown>, docs, ptr)
    }
  }
}

/** 样本文本 → schema / mock 双形态；不可解析返回 null。 */
function sampleShape(
  text: string,
  mock: boolean,
  docs?: Record<string, FieldDoc>,
): Record<string, unknown> | null {
  const trimmed = text.trim()
  if (!trimmed) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(trimmed)
  } catch {
    return null
  }
  const rows = inferSchema(parsed)
  if (mock) {
    return mockJsonFromSchema(rows)
  }
  const shaped = shapeOf(rows, false)
  if (Object.keys(shaped).length && docs && Object.keys(docs).length) {
    applyDocsToShape(shaped, docs)
  }
  return Object.keys(shaped).length ? shaped : null
}

function kvShape(list: KeyValue[]): Record<string, unknown>[] {
  return list
    .filter((kv) => kv.enabled && kv.key.trim())
    .map((kv) => ({
      name: kv.key,
      type: kv.field_type ?? 'string',
      required: kv.required ?? true,
      ...(kv.description ? { description: kv.description } : {}),
      ...(kv.example?.trim() ? { example: kv.example } : {}),
    }))
}

/** 表单类 Body（urlencoded / multipart）→ 扁平对象形态。 */
function formBodyShape(fields: KeyValue[] | MultipartField[], mock: boolean): Record<string, unknown> {
  const obj: Record<string, unknown> = {}
  for (const f of fields) {
    if (!f.enabled || !f.key.trim()) continue
    const isFile = 'value_type' in f && f.value_type === 'file_path'
    obj[f.key] = mock ? f.value || `<${isFile ? 'file' : 'string'}>` : 'string'
  }
  return obj
}

function bodyShape(mock: boolean): unknown {
  const body = d.value?.request.body
  if (!body) return undefined
  if (body.mode === 'json') return sampleShape(body.raw, mock, bodyDocs.value) ?? '<invalid-json>'
  if (body.mode === 'urlencoded') return formBodyShape(body.fields, mock)
  if (body.mode === 'multipart') return formBodyShape(body.fields, mock)
  if (!mock) return `<${body.mode}>`
  return undefined
}

/** 预览对象：标准 API 定义结构，随草稿实时刷新。 */
const preview = computed<Record<string, unknown> | null>(() => {
  const target = d.value
  if (!target) return null
  const mock = previewView.value === 'mock'
  const req: Record<string, unknown> = {}
  const query = kvShape(target.request.params)
  const headers = kvShape(target.request.headers)
  if (query.length) req.query = query
  if (headers.length) req.headers = headers
  const body = bodyShape(mock)
  if (body !== undefined) req.body = body

  const responsesShape: Record<string, unknown> = {}
  for (const ex of responses.value) {
    const docs = respDocsEdits.value.get(ex.id) ?? ex.docs
    // 未保存缓冲优先（结构/文本编辑实时反映到预览）
    const text = respStructRawOf(ex)
    responsesShape[String(ex.status)] = sampleShape(text, mock, docs) ?? { example: text || '<empty>' }
  }

  const out: Record<string, unknown> = {
    method: target.method,
    path: target.path,
  }
  if (Object.keys(req).length) out.request = req
  if (Object.keys(responsesShape).length) out.responses = responsesShape
  return out
})

const previewHtml = computed(() => {
  if (!preview.value) return ''
  return highlightJSON(JSON.stringify(preview.value, null, 2))
})

/** 复制按钮反馈态：Tooltip 短暂切换为「已复制」。 */
const copied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | null = null

async function copyPreview(): Promise<void> {
  if (!preview.value) return
  try {
    await navigator.clipboard.writeText(JSON.stringify(preview.value, null, 2))
    copied.value = true
    if (copiedTimer) clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => {
      copied.value = false
    }, 1600)
  } catch {
    toast.error(t('design.copyFail'))
  }
}

/** 保存按钮防连点：点击后短暂置忙，草稿转干净（保存成功）时提前解锁。 */
const saveBusy = ref(false)
let saveBusyTimer: ReturnType<typeof setTimeout> | null = null

function onSaveClick(): void {
  if (saveBusy.value) return
  saveBusy.value = true
  emit('save')
  if (saveBusyTimer) clearTimeout(saveBusyTimer)
  saveBusyTimer = setTimeout(() => {
    saveBusy.value = false
  }, 800)
}

watch(dirty, (v) => {
  if (!v) saveBusy.value = false
})

/**
 * 路径输入失焦归一化：`?query` 拆入 Params（同键更新），相对路径补前导 `/`；
 * 完整 http(s) 地址原样保留（curl 导入的绝对 URL 存整条）。
 */
function normalizePathInput(): void {
  const target = d.value
  if (!target) return
  let v = target.path.trim()
  if (!v) return
  const q = v.indexOf('?')
  if (q !== -1) {
    const qs = v.slice(q + 1)
    v = v.slice(0, q) || '/'
    for (const [key, val] of new URLSearchParams(qs).entries()) {
      const existing = target.request.params.find((p) => p.key === key)
      if (existing) existing.value = val
      else target.request.params.push({ key, value: val, enabled: true, description: '' })
    }
  }
  const isAbs = /^(https?|wss?):\/\//i.test(v)
  if (!isAbs && !v.startsWith('/')) v = `/${v}`
  target.path = v
}

onBeforeUnmount(() => {
  if (copiedTimer) clearTimeout(copiedTimer)
  if (saveBusyTimer) clearTimeout(saveBusyTimer)
})
</script>

<template>
  <div v-if="d" class="design">
    <!-- ---- 顶部操作栏：面包屑上下文 + 保存 ---- -->
    <header class="topbar doc-card">
      <div class="topbar-crumb">
        <span class="method-pill" :class="`mp-${d.method.toLowerCase()}`">{{ d.method }}</span>
        <code class="crumb-path">{{ d.path }}</code>
        <span class="crumb-sep">/</span>
        <span class="crumb-label">{{ t('design.title') }}</span>
      </div>
      <div class="topbar-actions">
        <span v-if="dirty" class="dirty-hint" :title="t('design.unsavedHint')">
          <span class="dirty-dot"></span>
          {{ t('design.unsaved') }}
        </span>
        <button
          type="button"
          class="save-btn"
          :disabled="saveBusy"
          @click="onSaveClick"
        >
          <Icon name="save" :size="13" />
          {{ t('design.saveDesign') }}
        </button>
      </div>
    </header>

    <div class="design-body">
      <!-- ---- 左：设计主体 ---- -->
      <div class="design-main">
        <!-- 基本信息 -->
        <section class="doc-card blk">
          <h4 class="doc-sec-title">{{ t('design.basicInfo') }}</h4>
          <div class="grid2">
            <label class="fld">
              <span class="fld-label">{{ t('design.endpointName') }}</span>
              <input v-model="d.name" class="miyo-input" :placeholder="t('editor.namePh')" spellcheck="false" />
            </label>
            <label class="fld">
              <span class="fld-label">{{ t('design.lifecycle') }}</span>
              <CustomSelect
                :model-value="d.status"
                :options="STATUS_OPTIONS"
                @update:model-value="onStatusChange"
              />
            </label>
          </div>

          <label class="fld">
            <span class="fld-label">{{ t('design.requestPath') }}</span>
            <div class="path-group">
              <CustomSelect
                class="method-select"
                :model-value="d.method"
                :options="METHOD_OPTIONS"
                @update:model-value="onMethodChange"
              >
                <template #display="{ label }">
                  <span class="method-label" :class="`mp-${d.method.toLowerCase()}`">{{ label }}</span>
                </template>
              </CustomSelect>
              <span class="pg-divider"></span>
              <input
                v-model="d.path"
                class="path-input"
                placeholder="/api/v1/resource"
                spellcheck="false"
                @change="normalizePathInput"
              />
            </div>
          </label>

          <label class="fld">
            <span class="fld-label">{{ t('design.description') }}</span>
            <textarea
              v-model="d.description"
              class="desc-area"
              rows="3"
              :placeholder="t('design.descriptionPh')"
              spellcheck="false"
            ></textarea>
          </label>
        </section>

        <!-- 请求定义 -->
        <section class="doc-card blk">
          <h4 class="doc-sec-title">{{ t('design.requestDef') }}</h4>
          <Tabs v-model="reqTab" :tabs="reqTabs" size="sm" />

          <template v-if="reqTab === 'params'">
            <ParamDefineTable :rows="d.request.params" :key-placeholder="t('design.paramNamePh')" @update:model-value="onParamsUpdate" />
          </template>
          <template v-else-if="reqTab === 'headers'">
            <ParamDefineTable :rows="d.request.headers" :key-placeholder="t('design.headerNamePh')" @update:model-value="onHeadersUpdate" />
          </template>

          <!-- Body 设计器 -->
          <template v-else>
            <div class="body-head">
              <SegmentedControl
                size="sm"
                :options="BODY_MODE_OPTIONS"
                :model-value="bodyMode || null"
                @update:model-value="onBodyModeChange($event)"
              />
              <span v-if="bodyMode === 'json'" class="json-state" :class="{ bad: !jsonValid }">
                {{ jsonValid ? t('design.jsonValid') : t('design.jsonInvalid') }}
              </span>
              <SegmentedControl
                v-if="bodyMode === 'json'"
                size="sm"
                :options="JSON_VIEW_OPTIONS"
                :model-value="bodyJsonView"
                @update:model-value="switchJsonView($event)"
              />
            </div>

            <p v-if="!bodyMode" class="body-hint">{{ BODY_VIEW_HINTS[d.request.body.mode] ?? '' }}</p>

            <JsonStructEditor
              v-if="bodyMode === 'json' && bodyJsonView === 'struct'"
              :key="d.id"
              :raw="bodyRaw"
              :docs="bodyDocs"
              @update:raw="onBodyStructRaw"
              @update:docs="onBodyStructDocs"
            />

            <textarea
              v-if="bodyMode === 'json' && bodyJsonView === 'text'"
              v-model="bodyRaw"
              class="body-json mono"
              spellcheck="false"
              placeholder='{ "field": "value" }'
              @keydown="onCodeKeydown"
            ></textarea>

            <ParamDefineTable
              v-else-if="bodyMode === 'form'"
              :rows="d.request.body.mode === 'urlencoded' ? d.request.body.fields : []"
              :key-placeholder="t('design.fieldNamePh')"
              :show-example="false"
              @update:model-value="onFormUpdate"
            />
          </template>
        </section>

        <!-- 返回响应 -->
        <section class="doc-card blk">
          <div class="resp-head">
            <h4 class="doc-sec-title">{{ t('design.responses') }}</h4>
            <span class="resp-count">{{ responses.length }}</span>
            <span class="resp-head-spacer"></span>
            <div class="resp-presets">
              <button
                v-for="p in STATUS_PRESETS"
                :key="p.status"
                type="button"
                class="resp-preset"
                :class="{ err: p.status >= 400 }"
                :disabled="responses.some((e) => e.status === p.status)"
                @click="addResponse(p.status)"
              >
                {{ p.label }}
              </button>
            </div>
          </div>

          <div v-if="responses.length" class="resp-list">
            <div v-for="ex in responses" :key="ex.id" class="resp-item">
              <button type="button" class="resp-row" @click="toggleResp(ex.id)">
                <Icon
                  name="chevron-right"
                  :size="12"
                  class="resp-caret"
                  :class="{ open: expandedRespId === ex.id }"
                />
                <span class="resp-status" :class="{ err: ex.status >= 400 }">{{ ex.status }}</span>
                <span class="resp-name">{{ respDisplayName(ex) }}</span>
                <span class="resp-meta">{{ respTimeOf(ex.updated_at) }}</span>
              </button>
              <div v-if="expandedRespId === ex.id" class="resp-editor">
                <div class="resp-body-head">
                  <SegmentedControl
                    size="sm"
                    :options="JSON_VIEW_OPTIONS"
                    :model-value="respJsonView"
                    @update:model-value="respJsonView = $event as JsonEditView"
                  />
                </div>
                <JsonStructEditor
                  v-if="respJsonView === 'struct'"
                  :raw="respStructRawOf(ex)"
                  :docs="respDocsEdits.get(ex.id) ?? ex.docs"
                  @update:raw="onRespStructRaw(ex, $event)"
                  @update:docs="onRespStructDocs(ex, $event)"
                />
                <textarea
                  v-else
                  class="body-json mono"
                  :value="respTextOf(ex)"
                  spellcheck="false"
                  :placeholder="t('design.respBodyPh')"
                  @input="onRespEdit(ex, ($event.target as HTMLTextAreaElement).value)"
                  @keydown="onCodeKeydown"
                ></textarea>
                <div class="resp-actions">
                  <button
                    type="button"
                    class="miyo-btn miyo-btn-sm"
                    :disabled="!respEdits.has(ex.id) && !respDocsEdits.has(ex.id)"
                    @click="saveRespBody(ex)"
                  >
                    <Icon name="save" :size="12" /> {{ t('design.saveChanges') }}
                  </button>
                  <Popconfirm :title="t('design.deleteRespConfirm', { v: ex.status })" @confirm="removeResp(ex)">
                    <IconButton name="trash" :size="13" tone="danger" :title="t('design.deleteResp')" />
                  </Popconfirm>
                </div>
              </div>
            </div>
          </div>
          <p v-else class="resp-empty">{{ t('design.respEmpty') }}</p>
          <div class="resp-add">
            <input
              v-model.number="newRespStatus"
              class="miyo-input resp-status-input mono"
              type="number"
              min="100"
              max="599"
              :title="t('design.customStatus')"
              :placeholder="t('design.statusCode')"
            />
            <input
              v-model="newRespName"
              class="miyo-input resp-name-input"
              :placeholder="t('design.customRespNamePh')"
              spellcheck="false"
              @keyup.enter="addResponse()"
            />
            <button type="button" class="miyo-btn miyo-btn-sm" @click="addResponse()">
              <Icon name="plus" :size="12" /> {{ t('design.add') }}
            </button>
          </div>
        </section>
      </div>

      <!-- ---- 右：实时预览 / Mock 生成 ---- -->
      <aside class="preview doc-card">
        <div class="preview-head">
          <h4 class="doc-sec-title">{{ t('design.livePreview') }}</h4>
          <SegmentedControl
            class="preview-pills"
            :model-value="previewView"
            size="sm"
            :options="PREVIEW_VIEW_OPTIONS"
            @update:model-value="previewView = $event as PreviewView"
          />
          <Tooltip :content="copied ? t('design.copied') : t('design.copySchema')" placement="bottom">
            <IconButton name="copy" :size="13" :label="t('design.copySchema')" @click="copyPreview" />
          </Tooltip>
        </div>
        <pre v-if="previewHtml" class="preview-code mono" v-html="previewHtml"></pre>
        <p v-else class="preview-empty">{{ t('design.previewEmpty') }}</p>
        <p class="preview-note">
          {{ previewView === 'mock' ? t('design.previewMockHint') : t('design.previewSchemaHint') }}
        </p>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.design {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 1280px;
}

/* ---- 顶部操作栏 ---- */

.topbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  position: sticky;
  top: 0;
  z-index: 5;
  backdrop-filter: blur(6px);
}

.topbar-crumb {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.crumb-path {
  font-family: var(--font-mono);
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.crumb-sep {
  color: var(--text-3);
}

.crumb-label {
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
}

.topbar-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

/* Method 徽标（顶栏 & 路径组共用） */
.method-pill {
  flex-shrink: 0;
  padding: 2px 9px;
  border-radius: 6px;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.04em;
}
.mp-get,
.mp-head {
  color: var(--get);
  background: color-mix(in srgb, var(--get) 10%, transparent);
}
.mp-post {
  color: var(--post);
  background: color-mix(in srgb, var(--post) 10%, transparent);
}
.mp-put {
  color: var(--put);
  background: color-mix(in srgb, var(--put) 10%, transparent);
}
.mp-delete {
  color: var(--delete);
  background: color-mix(in srgb, var(--delete) 10%, transparent);
}
.mp-patch {
  color: var(--patch);
  background: color-mix(in srgb, var(--patch) 12%, transparent);
}
.mp-options {
  color: var(--text-2);
  background: var(--bg-hover);
}

/* 未保存提醒：黄色小点 + 文本 */
.dirty-hint {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--warning);
}

.dirty-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
  box-shadow: 0 0 6px color-mix(in srgb, var(--warning) 60%, transparent);
}

/* 主保存按钮：主题紫实底 */
.save-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 16px;
  border: none;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: #fff;
  font-family: inherit;
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  box-shadow: var(--shadow-sm);
  transition:
    background var(--dur) var(--ease),
    transform var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.save-btn:hover {
  background: var(--accent-hover);
  box-shadow: var(--shadow);
}
.save-btn:active {
  transform: translateY(1px);
}
.save-btn:disabled {
  opacity: 0.6;
  cursor: default;
  transform: none;
}
.save-btn:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: 2px;
}

/* ---- 双栏布局 ---- */

.design-body {
  display: grid;
  grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
  gap: 16px;
  align-items: start;
}

@media (max-width: 1080px) {
  .design-body {
    grid-template-columns: 1fr;
  }
}

.design-main {
  display: flex;
  flex-direction: column;
  /* space-y-6：卡片间保留充足呼吸感 */
  gap: 24px;
  min-width: 0;
}

.blk {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 18px;
  border-radius: 10px;
}

/* ---- 基本信息 ---- */

.grid2 {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

@media (max-width: 720px) {
  .grid2 {
    grid-template-columns: 1fr;
  }
}

.fld {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.fld-label {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* Method + Path 组合输入组 */
.path-group {
  display: flex;
  align-items: stretch;
  height: 32px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius);
  overflow: hidden;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.path-group:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

.path-group .method-select {
  width: 104px;
  flex-shrink: 0;
  background: var(--bg-code);
}
.path-group .method-select :deep(.cs-trigger) {
  height: 100%;
  border: none;
  background: transparent;
  box-shadow: none;
  border-radius: 0;
  font-weight: 700;
}

.method-label {
  font-weight: 700;
}
.method-label.mp-get {
  color: var(--get);
}
.method-label.mp-post {
  color: var(--post);
}
.method-label.mp-put {
  color: var(--put);
}
.method-label.mp-delete {
  color: var(--delete);
}
.method-label.mp-patch {
  color: var(--patch);
}
.method-label.mp-options,
.method-label.mp-head {
  color: var(--text-2);
}

.pg-divider {
  width: 1px;
  flex-shrink: 0;
  background: var(--border);
}

.path-input {
  flex: 1;
  min-width: 0;
  padding: 0 10px;
  border: none;
  outline: none;
  /* 代码面走 --bg-code：深浅双主题同源（原 #0a0a0a + 组件级浅色覆写） */
  background: var(--bg-code);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: 13px;
}

.desc-area {
  width: 100%;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  color: var(--text-1);
  font-family: inherit;
  font-size: 12.5px;
  line-height: 1.6;
  resize: vertical;
  transition: border-color var(--dur) var(--ease);
}
.desc-area:focus {
  outline: none;
  border-color: var(--accent);
}

/* ---- 请求定义 ---- */

.body-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.json-state {
  font-size: var(--fs-xs);
  color: var(--success);
}
.json-state.bad {
  color: var(--danger);
}

.body-hint {
  margin: 0;
  padding: 12px;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius);
  font-size: 12px;
  color: var(--text-3);
}

.mono {
  font-family: var(--font-mono);
}

.body-json {
  width: 100%;
  min-height: 150px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  /* 代码面走 --bg-code：深浅双主题同源（原 #0a0a0a + 组件级浅色覆写） */
  background: var(--bg-code);
  color: var(--text-1);
  font-size: 12px;
  line-height: 1.6;
  resize: vertical;
  transition: border-color var(--dur) var(--ease);
}
.body-json:focus {
  outline: none;
  border-color: var(--accent);
}

/* ---- 返回响应 ---- */

.resp-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.resp-count {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.resp-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.resp-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.resp-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  color: var(--text-2);
  font-size: 12.5px;
  cursor: pointer;
  text-align: left;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.resp-row:hover {
  border-color: var(--border-strong);
  background: var(--bg-hover);
}

.resp-caret {
  flex-shrink: 0;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease);
}
.resp-caret.open {
  transform: rotate(90deg);
}

.resp-status {
  font-family: var(--font-mono);
  font-weight: 700;
  font-size: var(--fs-xs);
  color: var(--success);
}
.resp-status.err {
  color: var(--danger);
}

.resp-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-1);
}

.resp-meta {
  flex-shrink: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.resp-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.resp-body-head {
  display: flex;
  justify-content: flex-end;
}

.resp-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.resp-empty {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}

.resp-add {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding-top: 4px;
  border-top: 1px solid var(--border);
}

.resp-preset {
  padding: 2px 9px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-code);
  color: var(--success);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.resp-preset.err {
  color: var(--danger);
}
.resp-preset:not(:disabled):hover {
  border-color: var(--accent);
  background: var(--accent-tint);
}
.resp-preset:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.resp-status-input {
  width: 76px;
  height: 26px;
}

.resp-name-input {
  flex: 1;
  min-width: 120px;
  height: 26px;
}

/* ---- 右侧实时预览 ---- */

.preview {
  display: flex;
  flex-direction: column;
  gap: 10px;
  position: sticky;
  top: 62px;
  border-radius: 10px;
}

.preview-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.preview-head h4 {
  flex: 1;
  margin: 0;
}

.preview-code {
  margin: 0;
  max-height: 520px;
  overflow: auto;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-code);
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
}

/* v-html 高亮 span 无 data-v 属性，必须 :deep；色阶走全局 --tok-*（深/浅各一套） */
.preview-code :deep(.hl-k) {
  color: var(--tok-key);
}
.preview-code :deep(.hl-s) {
  color: var(--tok-str);
}
.preview-code :deep(.hl-n) {
  color: var(--tok-num);
}
.preview-code :deep(.hl-b) {
  color: var(--tok-bool);
}
.preview-code :deep(.hl-null) {
  color: var(--tok-null);
  font-style: italic;
}
.preview-code :deep(.hl-p) {
  color: var(--tok-punct);
}
.preview-code :deep(.hl-c) {
  color: var(--tok-gutter);
  font-style: italic;
}
.preview-code :deep(.hl-v) {
  color: var(--tok-bool);
}

.preview-empty {
  margin: 0;
  padding: 20px 12px;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-md);
  font-size: 12px;
  color: var(--text-3);
  text-align: center;
}

.preview-note {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.5;
}
</style>
