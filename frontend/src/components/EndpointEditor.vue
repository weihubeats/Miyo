<script setup lang="ts">
/**
 * EndpointEditor：接口编辑器（草稿模式）。
 *
 * - 直接编辑 store 草稿对象（Map 值经 Vue 集合响应式代理，嵌套修改即跟踪）；
 * - Base URL 为本地临时值（不落库），发送时与 path 拼接；
 * - 配置区为横向 Tab 系统：Params / Auth / Headers / Body / Examples / Code，
 *   各渲染独立面板组件（前置脚本 Scripts 与请求 Tab 的 Tests 已下线：
 *   断言迁至「工具」抽屉，代码生成即 Code 页签）；
 * - Ctrl+S 保存 / Ctrl+Enter 发送；响应区展示状态码、耗时与正文（JSON 自动美化）。
 */
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useLocaleStore } from '../stores/locale'
import { isDefaultName } from '../stores/locale'
import { useShortcuts, shortcutDef } from '../composables/useShortcuts'

import { isCurlCommand } from '../utils/url'
import { prettyJson } from '../utils/jsonFormat'
import { methodBadgeTone, methodTextTone } from '../utils/methodTone'
import { useVarCandidates } from '../composables/useVarCandidates'
import { useVarAutocomplete } from '../composables/useVarAutocomplete'
import { useEditorSplitter } from '../composables/useEditorSplitter'
import { useRequestExecution } from '../composables/useRequestExecution'
import { useRequestSettings } from '../composables/useRequestSettings'
import { useRequestUrl } from '../composables/useRequestUrl'
import { lazyComponent } from '../composables/lazyComponent'
import { deepClone } from '../utils/clone'
import { extractPathVarNames } from '../utils/pathVariables'
import {
  applyMethodDefaults,
  methodNeedsBody,
} from '../utils/requestBar'
import AuthPanel from './AuthPanel.vue'
import BodyPanel from './BodyPanel.vue'
import CodeExportMenu from './CodeExportMenu.vue'
import CodePanel from './CodePanel.vue'
import GrpcPanel from './GrpcPanel.vue'
import GrpcResponsePanel from './GrpcResponsePanel.vue'
import HeadersPanel from './HeadersPanel.vue'
import CustomNumberInput from './ui/CustomNumberInput.vue'
import CustomSelect from './ui/CustomSelect.vue'
import EmptyState from './ui/EmptyState.vue'
import Skeleton from './ui/Skeleton.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import Modal from './ui/Modal.vue'
import ParamsPanel from './ParamsPanel.vue'
import PathVariablesPanel from './PathVariablesPanel.vue'
import Popconfirm from './ui/Popconfirm.vue'
import ResponsePanel from './ResponsePanel.vue'
import RequestExamplesPanel from './RequestExamplesPanel.vue'
import ScriptsPanel from './ScriptsPanel.vue'
import Tabs from './ui/Tabs.vue'
import TestCaseModal from './TestCaseModal.vue'
import Tooltip from './ui/Tooltip.vue'
import ToolsDrawer from './ToolsDrawer.vue'
import VarSuggest from './ui/VarSuggest.vue'
import type { TabItem } from './ui/Tabs.vue'
import type {
  GrpcResponse,
  HttpMethod,
  RequestSpec,
  ResponseExample,
  TestCaseCategory,
} from '../types/miyoApi'

const store = useWorkspaceStore()
const toast = useToast()
const api = useMiyoApi()
const locale = useLocaleStore()
const t = locale.t

// 低频 / 重型面板按需加载（composables/lazyComponent.ts：骨架 + 可翻译的错误重试层）。
// 切视图 / 开弹窗才出现的：Design、Mock 视图与 Mock 规则弹窗；Docs / 用例视图内部链路
// 引入 CodeMirror 全家桶（约 300KB），首次切到才拉。EnvironmentManager 常驻但不在首屏
// 关键路径——异步化把它挪出主 chunk（挂载后后台取），不加 v-if 以免丢掉弹窗进出场动画。
const DesignPanel = lazyComponent(() => import('./DesignPanel.vue'))
const MockPanel = lazyComponent(() => import('./MockPanel.vue'))
const MockRuleDialog = lazyComponent(() => import('./MockRuleDialog.vue'))
const EnvironmentManager = lazyComponent(() => import('./EnvironmentManager.vue'))
const DocsPanel = lazyComponent(() => import('./DocsPanel.vue'))
const TestCasesPanel = lazyComponent(() => import('./TestCasesPanel.vue'))

/** gRPC 响应按接口 id 分桶（口径同 responses：切接口天然隔离）。 */
const grpcResponses = ref<Map<string, GrpcResponse | null>>(new Map())
/** 响应体动画锚点（DOM 归宿主持有）。 */
const flashEl = ref<HTMLElement | null>(null)

const draft = computed(() => store.activeEndpoint)

const METHODS: HttpMethod[] = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']
/** 方法下拉：HTTP 方法一组 + 分隔标题 + gRPC（协议维度，选中后编辑器形态整体切换）。
 *  tone 走 methodTone 单源映射，选项文字按方法语义色渲染、选中项浅色高亮。 */
const METHOD_OPTIONS = computed(() => [
  ...METHODS.map((m) => ({ value: m, label: m, tone: methodTextTone(m) })),
  { value: '__proto_header__', label: t('editor.methodGroupOther'), header: true },
  { value: 'GRPC' as HttpMethod, label: 'GRPC', tone: methodTextTone('GRPC') },
])

// 地址栏状态机（前缀选择 / 路径归一化 / buildUrl）抽至 composables/useRequestUrl。
const {
  urlDomain,
  resolvedDomain,
  chipClass,
  showEnvManager,
  BASE_URL_MANAGE,
  baseUrlMenuOpen,
  baseUrlOptions,
  sessionBaseIsCustom,
  envBaseUrlUnset,
  onResetSessionBaseUrl,
  baseUrlSideLabel,
  onBaseUrlChange,
  envBadgeLabel,
  urlPlaceholder,
  urlPath,
  applyUrlInput,
  commitUrlPath,
  buildUrl,
} = useRequestUrl({ draft, store, toast, t })

/** Mock 规则弹窗开关（Mock 页签「新建规则」与 MockPanel 共用）。 */
const showMockRules = ref(false)

// 请求执行状态机（分桶 / 计时 / 发送 / 取消 / gRPC 流）抽至
// composables/useRequestExecution——本组件只保留草稿与 URL/输入提交回调。
const {
  sending,
  elapsedText,
  dispose: disposeExecution,
  lastRequestIds,
  response,
  sendError,
  grpcDisplay,
  isGrpc,
  hasResponse,
  send,
  cancelSend,
  loadFullBody,
} = useRequestExecution({
  draft,
  store,
  api,
  toast,
  t,
  commitPendingInput,
  buildUrl,
  grpcResponses,
  flashEl,
})

/** 历史记录「重放」联动：restoreFromHistory(…, { send: true }) 恢复配置后标记
 * 待发送接口，本编辑器激活该接口时消费标记并自动发送一次。
 * watch 源带上 draft id：restore 同步打开标签页后，draft 已指向恢复的接口；
 * immediate 兜底「restore 时编辑器尚未挂载」的时序（如切项目后直接重放）。 */
watch(
  () => [store.autoSendEndpointId, draft.value?.id] as const,
  ([pendingId, currentId]) => {
    if (!pendingId || pendingId !== currentId) return
    store.autoSendEndpointId = null
    if (store.activeView !== 'debug') store.setActiveView('debug')
    if (sending.value) return
    void send()
  },
  { immediate: true },
)


// 请求设置 Popover（超时 / 重定向）状态机抽至 composables/useRequestSettings。
const settingsTriggerEl = ref<HTMLElement | null>(null)
const {
  onTimeoutInput,
  timeoutPlaceholder,
  settingsOpen,
  settingsPos,
  hasCustomSettings,
  toggleSettings,
  closeSettings,
  dispose: disposeSettings,
} = useRequestSettings({
  draft,
  globalTimeoutMs: computed(() => store.globalTimeoutMs),
  t,
  settingsTriggerEl,
})

// ---------- 请求区 / 响应区高度分割（Splitter） ----------
// 状态机（按接口分桶持久化 / grow 基准 / 拖拽键盘交互）抽至
// composables/useEditorSplitter——本组件只保留 DOM 锚点与派生条件。
const editorEl = ref<HTMLElement | null>(null)
/** 请求卡（config-box）DOM：未发送期拖分割条时取其实际盒高做拖拽基准。 */
const configBoxEl = ref<HTMLElement | null>(null)

const {
  requestBodyHeight,
  requestManual,
  requestBodyCollapsed,
  splitterDragging,
  onSplitterDown,
  onSplitterUp,
  onSplitterKeydown,
  toggleRequestBody,
  requestMaxHeight,
} = useEditorSplitter({
  draftId: computed(() => draft.value?.id),
  hasResponse,
  editorEl,
  configBoxEl,
})

/** 切接口自动收起请求设置弹窗（原与 splitter 的 chrome 归属同 watch，拆分后各自监听）。 */
watch(
  () => draft.value?.id,
  () => closeSettings(),
)

// ---------- 配置 Tab 系统 ----------
type ConfigTabKey =
  | 'params'
  | 'auth'
  | 'headers'
  | 'body'
  | 'grpc'
  | 'path'
  | 'scripts'
  | 'examples'
  | 'code'

/** 合法 Tab 集合（历史数据可能持久化过已下线的 Scripts / Tests 页签）。 */
const VALID_TABS: readonly ConfigTabKey[] = ['params', 'auth', 'headers', 'body', 'grpc', 'path', 'scripts', 'examples', 'code']

/** 未保存 active_tab 时的智能默认（不写回草稿，避免标记脏）。 */
const smartTab = ref<ConfigTabKey>('params')

/**
 * 配置 Tab：优先读接口保存的 active_tab；未设置（或为已下线 Tab）时按
 * Method 智能默认。用户点击 / 切换 Method 时写回 request.active_tab。
 */
const activeTab = computed<ConfigTabKey>({
  get: () => {
    const saved = draft.value?.request.active_tab as ConfigTabKey | null
    return saved && VALID_TABS.includes(saved) ? saved : smartTab.value
  },
  set: (tab: ConfigTabKey) => {
    smartTab.value = tab
    if (draft.value) draft.value.request.active_tab = tab
  },
})

const BODY_TAB_LABELS: Record<string, string> = {
  json: 'JSON',
  text: 'Text',
  urlencoded: 'x-www-form-urlencoded',
  multipart: 'form-data',
  graphql: 'GraphQL',
  binary: 'Binary',
}

/** 有效行数：与面板表格的可见行一致——只算「有键或有值」的行。
 *
 * 历史坑：这里原来直接用 `request.headers.length`，而表格会把底部空白幽灵行
 * 一并回写给父级（`sync()` 保留末行），于是加一个 `Cookie` 后 badge 显示 2、
 * 表里只有 1 条数据行。三类 kv（params / headers / path）同口径修正。 */
function filledRowCount(rows: { key?: string; value?: string }[] | undefined): number {
  return (rows ?? []).filter((r) => (r.key ?? '').trim() || (r.value ?? '').trim()).length
}

const configTabs = computed<TabItem[]>(() => {
  const d = draft.value
  if (!d) return []
  // gRPC：主配置（服务/方法/消息/proto）+ 元数据（复用请求头），不提供代码生成
  if (d.method === 'GRPC') {
    return [
      { key: 'grpc', label: 'gRPC' },
      { key: 'headers', label: t('editor.tabHeaders'), count: filledRowCount(d.request.headers) },
    ]
  }
  const bodyMode = d.request.body.mode
  return [
    { key: 'params', label: t('editor.tabParams'), count: filledRowCount(d.request.params) },
    { key: 'auth', label: t('editor.tabAuth') },
    { key: 'headers', label: t('editor.tabHeaders'), count: filledRowCount(d.request.headers) },
    {
      key: 'body',
      label:
        bodyMode !== 'none'
          ? `${t('editor.tabBody')} (${BODY_TAB_LABELS[bodyMode] ?? bodyMode})`
          : t('editor.tabBody'),
      count: bodyMode !== 'none' ? 1 : undefined,
    },
    { key: 'path', label: t('editor.tabPath'), count: filledRowCount(d.request.path_variables) },
    { key: 'scripts', label: t('editor.tabScripts') },
    { key: 'examples', label: t('editor.tabExamples') },
    { key: 'code', label: t('editor.tabCode') },
  ]
})

/** 接口切换：无保存的 active_tab 时按 Method 设定智能默认（不落库）。
 * 请求结果按 id 分桶（responses/sendErrors Map），切换接口天然隔离，无需清空。 */
watch(
  () => draft.value?.id,
  () => {
    smartTab.value =
      draft.value && draft.value.method === 'GRPC'
        ? 'grpc'
        : draft.value && methodNeedsBody(draft.value.method)
          ? 'body'
          : 'params'
  },
  { immediate: true },
)

/**
 * 手动切换 Method：POST 系 → Body（空体初始化 `{}` + application/json，有体则保持）；其余 → Params。
 * 仅在【同一接口内】method 变化时应用：打开 / 切换接口时 draft 从 null → 有值
 * 或 id 变化，method 必然「变」一次——那是一次载入而非用户编辑，回写
 * applyMethodDefaults 的副作用（body 初始化 / Content-Type / active_tab）
 * 会让刚打开的接口立刻被判定为「有改动」（isDirty 误报）。
 *
 * 往返还原：首次切换时记录切换前的 request 快照；切回记录的原方法时
 * 整体还原快照——否则 applyMethodDefaults 的副作用不可逆，用户
 * 「GET→POST→GET」后草稿仍与保存态不同，持续显示「有改动」。
 */
let methodRevert: { from: string; snapshot: RequestSpec } | null = null

watch(
  () => [draft.value?.id, draft.value?.method] as const,
  ([id, m], prev) => {
    if (!m || !draft.value) return
    if (id !== prev?.[0]) {
      methodRevert = null
      return
    }
    const d = draft.value
    if (methodRevert && m === methodRevert.from) {
      // 切回上次的方法：整体还原切换前的 request（撤销默认初始化副作用）
      d.request = methodRevert.snapshot
      const restored = d.request.active_tab
      smartTab.value = (restored && VALID_TABS.includes(restored as ConfigTabKey)
        ? (restored as ConfigTabKey)
        : m === 'GRPC'
          ? 'grpc'
          : methodNeedsBody(m)
            ? 'body'
            : 'params')
      methodRevert = null
      return
    }
    methodRevert ??= { from: prev?.[1] ?? m, snapshot: deepClone(d.request) }
    const tab = applyMethodDefaults(d.request, m)
    d.request.active_tab = tab
    smartTab.value = tab
  },
)

// 无损格式化（utils/jsonFormat：保序保重键保数字原文），失败回退原文——
// 原实现 JSON.parse/stringify 会吞重复键、改写数字（1.50→1.5），示例正文与保存原文不一致。
function prettyBody(raw: string): string {
  try {
    return prettyJson(raw)
  } catch {
    return raw
  }
}

function isAbsolutePath(p: string): boolean {
  return p.startsWith('http://') || p.startsWith('https://')
}

// ---------- 地址栏历史下拉（快速回填最近请求的 Method + 地址） ----------
const historyMenu = ref<InstanceType<typeof Menu> | null>(null)
const URL_HISTORY_MAX = 8

/** 最近请求去重（method+url 保序取新），最多 8 条。 */
const urlHistoryItems = computed<MenuItem[]>(() => {
  const seen = new Set<string>()
  const items: MenuItem[] = []
  for (const h of store.histories) {
    const k = `${h.method} ${h.url}`
    if (seen.has(k)) continue
    seen.add(k)
    items.push({ key: h.id, label: h.url, icon: 'history' })
    if (items.length >= URL_HISTORY_MAX) break
  }
  return items
})

function openUrlHistory(event: MouseEvent): void {
  // 历史可能尚未加载（首屏未发送过请求）：无记录时先拉一次再开
  if (!store.histories.length) void store.loadHistories()
  historyMenu.value?.openAt(event.currentTarget as HTMLElement, urlHistoryItems.value, 'left')
}

/** 选中历史：只回填 Method + 地址（完整配置恢复走侧栏历史页签「点击记录」）。
 * 复用 applyUrlInput 的归一化：绝对地址拆 origin / query 并参数，环境前缀语义一致。 */
function onUrlHistorySelect(item: MenuItem): void {
  const h = store.histories.find((x) => x.id === item.key)
  if (!h || !draft.value) return
  if (h.method !== 'GRPC') draft.value.method = h.method as HttpMethod
  applyUrlInput(h.url, true)
}

// ---------- 路径变量缺失提示（URL 栏下方 chips，点击直达 Path 页签） ----------
const missingPathVars = computed<string[]>(() => {
  const d = draft.value
  if (!d || d.method === 'GRPC' || isAbsolutePath(d.path)) return []
  const names = extractPathVarNames(d.path)
  if (!names.length) return []
  const defined = new Set((d.request.path_variables ?? []).map((r) => (r.key ?? '').trim()))
  return names.filter((n) => !defined.has(n))
})

function gotoPathTab(): void {
  activeTab.value = 'path'
}

/** 面包屑：接口所属文件夹名。 */
const folderName = computed(() => {
  if (!draft.value?.folder_id) return ''
  return store.folders.find((f) => f.id === draft.value!.folder_id)?.name ?? ''
})

/** 路径输入框元素引用（快捷按钮聚焦回跳）。 */
const urlInputEl = ref<HTMLInputElement | null>(null)

/** 地址栏 {{变量}} 候选：内置变量 + 环境/项目/全局变量（随 store 变化刷新）。 */
const varCandidates = useVarCandidates()

/** 地址栏 {{ 自动补全（↑↓ 选择、Enter/Tab 插入、Esc 关闭）。 */
const urlAc = useVarAutocomplete(varCandidates)
const { open: urlAcOpen, items: urlAcItems, activeIndex: urlAcIndex, anchor: urlAcAnchor } = urlAc

/** 输入 / 点击（重设光标）后刷新补全状态。 */
function onUrlAcSync(event: Event): void {
  urlAc.onInput(event.target as HTMLInputElement)
}

/** 失焦提交：关闭补全弹层并归一化地址栏输入（逐键不改写，失焦才定稿）。 */
function onUrlBlur(): void {
  urlAc.close()
  commitUrlPath()
}

/** 弹层候选项 → 插入 `{{name}}`（composable 内 dispatch input 同步 v-model）。 */
function onUrlAcPick(index: number): void {
  const el = urlInputEl.value
  if (el) urlAc.pick(el, index)
}

/** 地址栏 cURL 粘贴解析中（防重复触发）。 */
const curlPasting = ref(false)

/**
 * 从「文档片段」里取出路径那一段。
 *
 * 从接口文档 / 聊天 / 终端整段复制时，剪贴板往往不是裸路径，而是「路径 + 名称」「方法 + 路径」
 * 这类带空白的片段（`POST /routes`、`/routes routes`、`api /api/v1/x`）；单行 input 又会把
 * 剪贴板里的换行折成空格（实测：`insertText("a\nb")` → `a b`）。全部照抄就落成 `/routes routes`。
 *
 * 取词规则：
 * 1. 首段是「只剩 origin 的完整地址」（前缀里带了不可见空白被截断，`http://h:1 /routes`）→
 *    与后面的路径段拼回一条完整 URL，别把主机丢掉；
 * 2. 否则取含 `/` 的那一段（路径必含 `/`，标签不会）；
 * 3. 都没有 `/` 时取首段（`routes routes` → `routes`）。
 */
function pickPathToken(text: string): string {
  const tokens = text
    .replace(/[\r\n\t]+/g, ' ')
    .split(/\s+/)
    .filter(Boolean)
  const head = tokens[0] ?? ''
  if (/^(?:https?|wss?):\/\/[^/]+$/i.test(head)) {
    const path = tokens.slice(1).find((token) => token.startsWith('/'))
    if (path) return `${head}${path}`
  }
  return tokens.find((token) => token.includes('/')) ?? head
}

/**
 * 光标处替换当前选区插入：与敲键同一类原生编辑事务，进撤销栈（Cmd+Z 可回退）。
 * 关键：WebKit 的 smart paste 空格只在**原生粘贴**路径生成，insertText 不受影响。
 */
function insertAtByExecCommand(el: HTMLInputElement, start: number, end: number, text: string): boolean {
  try {
    el.setSelectionRange(start, end)
    return document.execCommand('insertText', false, text)
  } catch {
    return false
  }
}

/**
 * 整条替换：选中全字段后走原生插入 —— 与直接敲键同一类编辑事务，
 * 替换本身也进撤销栈（Cmd+Z 能回到替换前的内容）。
 */
function replaceByExecCommand(el: HTMLInputElement, text: string): boolean {
  try {
    el.setSelectionRange(0, el.value.length)
    return document.execCommand('insertText', false, text)
  } catch {
    return false
  }
}

/** 兜底程序化写值（execCommand 不可用的环境）；程序化写值不进撤销栈，仅保证结果正确。 */
function setFieldValueAt(el: HTMLInputElement, text: string, caret: number): void {
  el.value = text
  try {
    el.setSelectionRange(caret, caret)
  } catch {
    /* 非文本输入（类型限制）：跳过光标定位 */
  }
  el.dispatchEvent(new Event('input', { bubbles: true }))
}

/** 兜底整条替换 = 写值 + 光标到末尾。 */
function setFieldValue(el: HTMLInputElement, text: string): void {
  setFieldValueAt(el, text, text.length)
}

/** 归一化 shell 续行 + 去终端提示符，与 CurlImportDialog 保持一致。 */
function normalizeCurlCommand(raw: string): string {
  return raw
    .replace(/^\s*[$#>]+[ \t]+/, '')
    .replace(/\\[ \t]*\r?\n/g, ' ')
    .trim()
}

/** 地址栏粘贴 cURL 命令 → 解析后回填当前草稿（method / URL / headers / body / auth）。 */
async function importCurlText(raw: string): Promise<void> {
  if (!draft.value || curlPasting.value) return
  const targetId = draft.value.id
  curlPasting.value = true
  try {
    const parsed = await api.parseCurlCommand(normalizeCurlCommand(raw))
    if (draft.value?.id !== targetId) return
    store.applyCurlToDraft(targetId, parsed)
    if (parsed.ignored?.length) {
      toast.success(t('editor.curlAutoImported'), {
        message: t('curldlg.ignoredHint', { v: parsed.ignored.join(' ') }),
      })
    } else {
      toast.success(t('editor.curlAutoImported'))
    }
  } catch (err) {
    toast.error(t('editor.curlParseFail', { v: err instanceof Error ? err.message : String(err) }))
  } finally {
    curlPasting.value = false
  }
}

/**
 * 地址栏粘贴分四路：
 * - cURL 命令 → 拦截，走导入流程；
 * - 含空白的普通文本（从文档/聊天整段复制：`POST /routes`、`/routes routes`、末尾带换行）
 *   → 取其中像路径的那一段（`pickPathToken`）并**整条替换**：这类片段本身就是「一段完整
 *   地址」，而照抄整段会落成 `/routes routes`（单行 input 会把换行折成空格）；
 * - 完整 URL（http/https/ws/wss 起始）→ 拦截，整条替换并立即拆分
 *   origin / path / query→params（粘一个完整地址即「这就是请求地址」，保持旧粘贴体验）；
 * - 其余「干净」普通文本 → **仍 `preventDefault`，不再放行原生粘贴**（根治「从地址栏复制
 *   再粘回来多一个空格」）：WebKit（Tauri 的 WKWebView 同源）对单行 input 的原生粘贴
 *   带 smart copy/paste——插入点邻接文字（`/routes` 后粘 `routes`）时，粘贴机制写值时
 *   自补一个分隔空格 → `/routes routes`。这个空格由粘贴过程生成，不在 clipboardData
 *   里，且是否补与剪贴板来源相关（从地址栏复制再粘回来必现），拦截内容不如拦截行为：
 *   改由自己插（尊重选区/光标）。插入仍走 `execCommand('insertText')` 原生编辑事务
 *   （实证：insertText 记入撤销栈、`undo` 可回退；而程序化写 `el.value` 会清空撤销栈）；
 *   插入点与字段首尾若带**不可见空白**（旧版本粘出来的残留，看着就是 `/routes`），
 *   先把它排掉再插，否则残留会续命成 `/routes routes`。
 * 回归 bug 四枚：① 旧实现对普通文本也 `preventDefault` + `applyUrlInput(text, true)`，
 * 无视光标/选区整条替换（已有 `/aa/bb/` 时粘 `/cc` 前缀被顶掉）；② 程序化赋值清空原生
 * 撤销栈，Cmd+Z 回退不了；③ 文档片段 / 字段残留空白照抄落值（`/routes routes`）；④
 * 放行原生粘贴时 WKWebView smart paste 在插入点邻接文字自补分隔空格（同 `/routes routes`，
 * 见上）。
 */
function onUrlPaste(event: ClipboardEvent): void {
  const raw = event.clipboardData?.getData('text') ?? ''
  if (!raw || !draft.value || curlPasting.value) return

  if (isCurlCommand(raw)) {
    event.preventDefault()
    void importCurlText(raw)
    return
  }

  // 含空白 = 整段复制的文档片段，只取路径那一段；纯裸路径（无空白）原样保留
  const fragmented = /\s/.test(raw)
  const text = fragmented ? pickPathToken(raw) : raw
  if (!text) {
    event.preventDefault() // 纯空白（如「复制尾部空格」）：地址栏不该收空白，直接丢弃
    return
  }

  if (/^(?:https?|wss?):\/\//i.test(text)) {
    event.preventDefault()
    applyUrlInput(text, true)
    return
  }

  const el = event.target as HTMLInputElement
  const start = el.selectionStart ?? el.value.length
  const end = el.selectionEnd ?? start
  event.preventDefault()
  el.focus()

  if (fragmented) {
    if (!replaceByExecCommand(el, text)) setFieldValue(el, text)
    return
  }

  // 光标处插入（尊重选区）。先排掉「紧邻插入点」与字段首尾的空白：这类残留肉眼
  // 不可见（看着就是 `/routes`），随插入落值会变成 `/routes routes`。字段中段
  // 的空白（用户自己敲的）不动。
  const head = el.value.slice(0, start).replace(/^\s+|\s+$/g, '')
  const tail = el.value.slice(end).replace(/^\s+|\s+$/g, '')
  const next = `${head}${text}${tail}`
  const caret = Math.min(next.length, head.length + text.length)

  if (head.length === start && tail.length === el.value.length - end) {
    // 字段干净（没排掉任何字符）：只替换选区本身，同敲键事务，撤销栈完好
    if (!insertAtByExecCommand(el, start, end, text)) setFieldValueAt(el, next, caret)
    return
  }

  // 有残留：先排掉再插，走整条重写事务（单个撤销事务，可整体回退）
  if (!replaceByExecCommand(el, next)) setFieldValueAt(el, next, caret)
}

/** 提交地址栏输入并归一化（失焦 / 发送 / 保存前调用；重复执行幂等）。 */
/** 读取草稿前统一收口：提交地址栏镜像 + 刷出 Body 防抖镜像。
 *  save/send/代码生成/压测/命名确认都经此，保证 ⌘S / ⌘Enter 在输入框内
 *  触发时（不经过 blur）也能拿到包含最后 150ms 键入的完整草稿。 */
const bodyPanelRef = ref<InstanceType<typeof BodyPanel> | null>(null)

function commitPendingInput(): void {
  commitUrlPath()
  flushPendingBodyInput()
}

/** 刷出 BodyPanel 尚未落盘的防抖镜像（raw / GraphQL query+op / binary path）。 */
function flushPendingBodyInput(): void {
  bodyPanelRef.value?.flushPendingInput()
}

// ---------- 响应示例 ----------
const viewingExample = ref<ResponseExample | null>(null)
const activeExamples = computed(() => store.examples.get(draft.value?.id ?? '') ?? [])

const showExampleDialog = ref(false)
const exampleName = ref('')

function saveExample(): void {
  if (!draft.value || !response.value) return
  exampleName.value = `${draft.value.method} ${new Date().toLocaleTimeString(locale.resolved === 'zh' ? 'zh-CN' : 'en-US')}`
  showExampleDialog.value = true
}

async function confirmSaveExample(): Promise<void> {
  if (!draft.value || !response.value) return
  const name = exampleName.value.trim()
  if (!name) {
    toast.warning(t('editor.exampleNameRequired'))
    return
  }
  try {
    let resp = response.value
    if (resp.body_omitted) {
      // IPC 只传了 2MB 预览：按 request_id 取 Rust 侧缓存的全文再落库。
      const rid = lastRequestIds.value.get(draft.value.id)
      const full = rid ? await api.getResponseBody(rid) : null
      if (full != null) resp = { ...resp, body: full, body_omitted: false }
      else toast.warning(t('response.previewCapped'))
    }
    await store.saveAsExample(draft.value.id, name, resp)
    showExampleDialog.value = false
  } catch (err) {
    toast.error(t('editor.exampleSaveFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

function viewExample(ex: ResponseExample): void {
  viewingExample.value = viewingExample.value?.id === ex.id ? null : ex
}

async function removeExample(ex: ResponseExample): Promise<void> {
  if (!draft.value) return
  try {
    await store.removeExample(draft.value.id, ex.id)
    if (viewingExample.value?.id === ex.id) viewingExample.value = null
  } catch (err) {
    toast.error(t('editor.exampleDeleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

// ---------- 工具抽屉（生成代码 / 测试 / 压测） ----------
const showTools = ref(false)

// ---------- 编辑器动作下拉（低频操作收纳：工具抽屉入口） ----------
const actionsMenu = ref<InstanceType<typeof Menu> | null>(null)

function openActionsMenu(event: MouseEvent): void {
  actionsMenu.value?.openAt(event.currentTarget as HTMLElement, [
    { key: 'tools', label: t('editor.tools'), icon: 'gauge' },
  ], 'right')
}

function onActionsMenuSelect(item: MenuItem): void {
  if (item.key === 'tools') showTools.value = true
}

const requestUrl = computed(() => (draft.value ? buildUrl() : ''))

/** 路径输入框 Enter → 发送；Esc → 清空路径（setter 忽略空串，直接写草稿）。 */
function onUrlKeydown(event: KeyboardEvent): void {
  const el = event.target as HTMLInputElement
  // 补全弹层打开时接管 ↑↓/Enter/Tab/Esc，避免 Enter 直接触发发送
  if (urlAc.handleKeydown(event, el)) return
  if (event.key === 'Enter') {
    event.preventDefault()
    event.stopPropagation()
    if (!sending.value) void send()
  } else if (event.key === 'Escape') {
    // Esc = 收起（失焦），不清空：程序化赋值不进撤销栈，误触会静默丢整条路径；
    // 清空只留给显式 ✕ 按钮（clearPath）。
    event.preventDefault()
    el.blur()
  }
}

/** 快捷按钮：清空路径。 */
function clearPath(): void {
  if (draft.value) draft.value.path = ''
}

/** 快捷按钮：复制完整请求地址。 */
async function copyRequestUrl(): Promise<void> {
  if (!requestUrl.value) return
  await navigator.clipboard.writeText(requestUrl.value)
  toast.info(t('common.copied'))
}

/** 快捷按钮：复制当前 Base URL（解析后优先，未解析退回字面量）。 */
async function copyBaseUrl(): Promise<void> {
  const v = resolvedDomain.value || urlDomain.value
  if (!v) return
  await navigator.clipboard.writeText(v)
  toast.info(t('editor.baseCopied'))
}

// ---------- 二级导航 + 保存为用例 ----------
const SUB_NAV = computed<{ key: 'debug' | 'design' | 'docs' | 'cases' | 'mock'; label: string }[]>(() => [
  { key: 'debug', label: t('editor.navDebug') },
  { key: 'design', label: t('editor.navDesign') },
  { key: 'docs', label: t('editor.navDocs') },
  { key: 'cases', label: t('editor.navCases') },
  { key: 'mock', label: 'Mock' },
])

/**
 * 二级导航按接口独立记忆：切到「测试用例」再切去别的接口、切回来仍是
 * 「测试用例」，不会被打回「调试」页（用户实测诉求）。
 *
 * 映射放模块级：编辑器随页签 v-if 卸载重挂也要记得（与分割态同口径）。
 * `lastViewByEndpoint` 记每个接口最后停在哪个二级页；打开接口时据此恢复。
 * 只有显式点击导航（用户切页）才更新——打开接口引发的 activeView 变化不算，
 * 否则「当前接口」会被后切进来自动恢复的页覆盖。
 */
const lastViewByEndpoint = new Map<string, 'debug' | 'design' | 'docs' | 'cases' | 'mock'>()

function rememberView(endpointId: string | null | undefined, view: string): void {
  if (!endpointId) return
  lastViewByEndpoint.set(endpointId, view as 'debug' | 'design' | 'docs' | 'cases' | 'mock')
}

/** 打开接口：恢复该接口上次停留的二级页（从未切过 → 调试页）。 */
watch(
  () => draft.value?.id,
  (id) => {
    if (!id) return
    store.setActiveView(lastViewByEndpoint.get(id) ?? 'debug')
  },
  { immediate: true },
)

/** 用户显式点导航：先记到当前接口，再切页。 */
function selectView(view: 'debug' | 'design' | 'docs' | 'cases' | 'mock'): void {
  rememberView(draft.value?.id, view)
  store.setActiveView(view)
}

const saveMenuEl = ref<InstanceType<typeof Menu> | null>(null)

function openSaveMenu(event: MouseEvent): void {
  saveMenuEl.value?.openAt(event.currentTarget as HTMLElement, [
    { key: 'save-case', label: t('editor.saveAsCase'), icon: 'list' },
  ])
}

function onSaveMenuSelect(item: MenuItem): void {
  if (item.key === 'save-case') openSaveCaseModal()
}

const showTestCaseModal = ref(false)
const pendingCaseName = ref('')

/** 保存为用例：提取当前请求快照（URL / Params / Headers / Body）存入 test_cases。 */
function openSaveCaseModal(): void {
  const d = draft.value
  if (!d) return
  pendingCaseName.value = !isDefaultName('endpoint', d.name) ? `${d.method} ${d.path}` : ''
  showTestCaseModal.value = true
}

async function onSaveCaseSubmit(payload: {
  name: string
  category: TestCaseCategory
}): Promise<void> {
  const d = draft.value
  if (!d) return
  await store.saveTestCase(d.id, payload.name, payload.category, d.request, d.path, d.method)
}
/** 保存：名称为空或仍是默认「未命名接口」时，先弹「名称 + 保存位置」确认框，确认后再落库。 */
const showNameDialog = ref(false)
const pendingName = ref('')
const pendingFolderId = ref('')

/** 保存位置（文件夹）选项：树形展平，子目录按层级缩进展示。 */
interface FolderOption {
  value: string
  label: string
  depth: number
}
const folderOptions = computed<FolderOption[]>(() => {
  const out: FolderOption[] = []
  const walk = (parentId: string | null, depth: number): void => {
    store.folders
      .filter((f) => f.parent_id === parentId)
      .sort((a, b) => a.sort_order - b.sort_order)
      .forEach((f) => {
        out.push({ value: f.id, label: f.name, depth })
        walk(f.id, depth + 1)
      })
  }
  walk(null, 0)
  return out
})

async function save(): Promise<void> {
  if (!draft.value) return
  commitPendingInput()
  const name = draft.value.name.trim()
  if (isDefaultName('endpoint', name)) {
    pendingName.value = ''
    pendingFolderId.value = draft.value.folder_id ?? ''
    showNameDialog.value = true
    return
  }
  const ok = await store.saveActiveDraft()
  if (ok) {
    methodRevert = null
  }
}

async function confirmName(): Promise<void> {
  if (!draft.value) return
  commitPendingInput()
  const name = pendingName.value.trim()
  if (!name) {
    toast.warning(t('editor.nameRequired'))
    return
  }
  draft.value.name = name
  draft.value.folder_id = pendingFolderId.value || null
  showNameDialog.value = false
  const ok = await store.saveActiveDraft()
  if (ok) {
    methodRevert = null
  }
}

/**
 * 全局快捷键（集中注册表，见 useShortcuts；帮助面板自动收录）。
 * 默认键位来自 SHORTCUT_DEFAULTS，用户可在设置 → 快捷键中自定义。
 * inInput: true 保持原行为——原来是裸 window 监听，输入框内同样生效。
 */
useShortcuts([
  shortcutDef('editor.save', () => save()),
  shortcutDef('editor.send', () => void send()),
  shortcutDef('editor.new-request-t', () => store.openNewEndpoint(null)),
  shortcutDef('editor.new-request-n', () => store.openNewEndpoint(null)),
])

/** 新建接口后自动聚焦地址输入框（TabBar「+」/ ⌘T ⌘N / 树内新建共用），便于直接输入路径。 */
watch(
  () => store.focusTitleSignal,
  () => {
    void nextTick(() => {
      urlInputEl.value?.focus()
    })
  },
)

onUnmounted(() => {
  onSplitterUp()
  disposeExecution()
  disposeSettings()
})
</script>

<template>
  <div ref="editorEl" v-if="draft" class="editor">
    <div class="editor-row breadcrumb-row">
      <span class="crumb">
        <!-- 项目名不再重复展示：顶栏 ProjectTabs 已有全局项目上下文 -->
        <template v-if="folderName">
          <span class="crumb-part">{{ folderName }}</span>
          <span class="crumb-sep">/</span>
        </template>
        <input
          v-model="draft.name"
          class="crumb-name"
          :placeholder="t('editor.endpointNamePh')"
          spellcheck="false"
          maxlength="60"
          :title="t('editor.endpointNameHint')"
        />
      </span>
      <span class="breadcrumb-spacer"></span>
    </div>

    <!-- 二级导航：调试 | 设计 | 文档预览 | 测试用例 (N) | Mock -->
    <div class="sub-nav" role="tablist">
      <button
        v-for="nav in SUB_NAV"
        :key="nav.key"
        class="sub-nav-item"
        :class="{ active: store.activeView === nav.key }"
        type="button"
        role="tab"
        @click="selectView(nav.key)"
      >
        {{ nav.label }}
        <span v-if="nav.key === 'cases'" class="sub-nav-badge">{{ store.testCaseCount }}</span>
      </button>
    </div>

    <template v-if="store.activeView === 'debug'">
      <div class="editor-row">
      <div class="request-bar" :class="{ 'is-sending': sending }">
        <div v-if="sending" class="req-progress" aria-hidden="true"></div>
        <CustomSelect
          class="method-select"
          :model-value="draft.method"
          :options="METHOD_OPTIONS"
          pop-class="cs-method-pop"
          :pop-min-width="130"
          @update:model-value="draft.method = String($event) as HttpMethod"
        >
          <template #display="{ label }">
            <span class="method-badge" :class="methodBadgeTone(draft.method)">{{ label }}</span>
          </template>
        </CustomSelect>
        <span v-if="!isAbsolutePath(draft.path)" class="req-bar-divider"></span>
        <div v-if="!isAbsolutePath(draft.path)" class="base-url-wrap">
          <Tooltip
            :content="t('editor.baseUrlHint')"
            placement="bottom"
            :disabled="baseUrlMenuOpen"
            class="base-url-tip"
          >
            <CustomSelect
              class="base-url-select"
              :class="chipClass"
              pop-class="base-url-pop"
              :pop-min-width="360"
              :model-value="store.activeEnvId ?? ''"
              :options="baseUrlOptions"
              :placeholder="t('envbar.noEnv')"
              @change="onBaseUrlChange"
              @open="baseUrlMenuOpen = true"
              @close="baseUrlMenuOpen = false"
            >
              <template #display>
                <Icon name="globe" :size="14" class="env-badge-icon" />
                <span class="env-badge-text">{{ envBadgeLabel }}</span>
              </template>
              <template #option="{ option }">
                <span v-if="option.value === BASE_URL_MANAGE" class="base-url-manage">
                  {{ option.label }}
                </span>
                <template v-else>
                  <span class="base-url-opt-url">{{ option.label }}</span>
                  <span v-if="envBaseUrlUnset(String(option.value))" class="base-url-opt-hint">
                    {{ t('editor.baseUrlUnset') }}
                  </span>
                  <span class="base-url-opt-name">{{ baseUrlSideLabel(String(option.value)) }}</span>
                </template>
              </template>
              <!-- 会话前缀偏离默认时，「无环境」行给重置入口（否则该值只增不减，被读成 bug） -->
              <template #actions="{ option }">
                <button
                  v-if="option.value === '' && sessionBaseIsCustom"
                  type="button"
                  class="base-url-reset"
                  :title="t('editor.baseUrlReset')"
                  :aria-label="t('editor.baseUrlReset')"
                  @click.stop="onResetSessionBaseUrl"
                >
                  <Icon name="refresh" :size="12" />
                </button>
              </template>
            </CustomSelect>
          </Tooltip>
          <Tooltip
            v-if="resolvedDomain || urlDomain"
            :content="t('editor.copyBaseUrl')"
            placement="bottom"
            :disabled="baseUrlMenuOpen"
            class="base-url-copy"
          >
            <button type="button" class="base-url-copy-btn" @click.stop="copyBaseUrl">
              <Icon name="copy" :size="14" />
            </button>
          </Tooltip>
        </div>
        <div class="url-input-wrap">
          <input
            ref="urlInputEl"
            v-model="urlPath"
            class="url-input"
            spellcheck="false"
            :placeholder="urlPlaceholder"
            @keydown="onUrlKeydown"
            @paste="onUrlPaste"
            @input="onUrlAcSync"
            @click="onUrlAcSync"
            @blur="onUrlBlur"
          />
          <VarSuggest
            v-if="urlAcOpen"
            :anchor="urlAcAnchor"
            :items="urlAcItems"
            :active-index="urlAcIndex"
            @pick="onUrlAcPick"
          />
          <Tooltip :content="t('editor.urlHistory')" placement="top" class="url-qbtn">
            <button
              type="button"
              class="url-qbtn-btn"
              :aria-label="t('editor.urlHistory')"
              @click="openUrlHistory"
            >
              <Icon name="history" :size="14" />
            </button>
          </Tooltip>
          <template v-if="urlPath">
            <Tooltip :content="t('editor.copyUrl')" placement="top" class="url-qbtn url-qbtn-copy">
              <button type="button" class="url-qbtn-btn" @click="copyRequestUrl">
                <Icon name="copy" :size="14" />
              </button>
            </Tooltip>
            <Tooltip :content="t('editor.clearPath')" placement="top" class="url-qbtn url-qbtn-clear">
              <button type="button" class="url-qbtn-btn" @click="clearPath">
                <Icon name="x" :size="14" />
              </button>
            </Tooltip>
          </template>
        </div>
        <button v-if="!sending" class="miyo-btn miyo-btn-send bar-send" type="button" @click="send">
          <Icon name="send" :size="16" />
          {{ t('editor.send') }}
        </button>
        <button
          v-else
          class="miyo-btn miyo-btn-danger bar-send is-sending"
          type="button"
          :title="t('editor.sendingCancel')"
          @click="cancelSend"
        >
          <span class="btn-spinner" aria-hidden="true"></span>
          <span>{{ t('editor.sending') }} <span class="bar-send-elapsed">{{ elapsedText }}</span></span>
          <Icon name="stop" :size="16" />
        </button>
      </div>
      <div class="editor-actions">
        <!-- 低频工具（断言 / 压测）收进「⋯」下拉：给地址栏让出横向空间 -->
        <Menu ref="actionsMenu" @select="onActionsMenuSelect" />
        <Tooltip :content="t('common.moreActions')" placement="bottom">
          <button
            type="button"
            class="miyo-btn miyo-btn-sm actions-more"
            :aria-label="t('common.moreActions')"
            @click="openActionsMenu"
          >
            <Icon name="more-horizontal" :size="15" />
          </button>
        </Tooltip>
        <CodeExportMenu
          v-if="!isGrpc"
          :method="draft.method"
          :url="requestUrl"
          :headers="draft.request.headers"
          :body="draft.request.body"
          :auth="draft.request.auth"
        />
        <div class="save-group">
          <button class="miyo-btn save-main" type="button" @click="save">
            <Icon name="save" :size="16" /> {{ t('editor.saveHint') }}
          </button>
          <button
            class="miyo-btn save-arrow"
            type="button"
            :title="t('editor.saveMore')"
            @click="openSaveMenu($event)"
          >
            <Icon name="chevron-down" :size="14" />
          </button>
        </div>
      </div>
    </div>

    <!-- 路径变量缺失提示：URL 栏下轻量条，点击直达 Path 页签补配置 -->
    <div v-if="missingPathVars.length" class="pathvar-hint">
      <Icon name="alert-triangle" :size="12" class="pathvar-hint-icon" />
      <span class="pathvar-hint-label">{{ t('editor.pathMissing') }}</span>
      <button
        v-for="n in missingPathVars"
        :key="n"
        class="pathvar-chip miyo-mono"
        type="button"
        :title="t('editor.pathMissingGo')"
        @click="gotoPathTab"
      >
        {{ n }}
      </button>
    </div>

    <!-- 请求区 + 响应区合并为一张卡（Single Border Architecture）：
         两卡各自带完整边框、又相距 14px 时会出现两条平行线，且拖拽热区与
         两条线都不重合 → 「看着有线、拖起来没反应」。合并后中间只剩分割条
         那一条接缝，热区、视觉线、拖动行为三者重合。 -->
    <div class="req-resp-stack" :class="{ 'no-response': !hasResponse }">
      <div
        ref="configBoxEl"
        class="config-box"
        :class="{ collapsed: requestBodyCollapsed, grow: !hasResponse && !requestManual }"
        :style="hasResponse || requestManual ? { height: `${requestBodyHeight}px` } : undefined"
      >
        <div class="config-head">
          <Tabs v-model="activeTab" :tabs="configTabs" size="sm" />
          <span class="config-head-spacer"></span>
          <!-- 响应区最大化 ⇄ 恢复：请求区塌到最小（分割条双击同语义），响应区独占 -->
          <Tooltip :content="requestBodyCollapsed ? t('editor.requestRestore') : t('editor.responseMax')" placement="bottom">
            <button
              type="button"
              class="rs-trigger req-max-toggle"
              :aria-label="requestBodyCollapsed ? t('editor.requestRestore') : t('editor.responseMax')"
              @click="toggleRequestBody"
            >
              <Icon :name="requestBodyCollapsed ? 'chevrons-up-down' : 'chevrons-down-up'" :size="13" />
            </button>
          </Tooltip>
          <button
            ref="settingsTriggerEl"
            type="button"
            class="rs-trigger rs-settings"
            :class="{ open: settingsOpen }"
            aria-haspopup="dialog"
            :aria-expanded="settingsOpen"
            :title="t('editor.reqSettings')"
            @click="toggleSettings"
          >
            <Icon name="settings" :size="13" />
            <span>{{ t('editor.reqSettings') }}</span>
            <span v-if="hasCustomSettings && !settingsOpen" class="rs-dot" aria-hidden="true"></span>
          </button>
        </div>

        <!-- 请求设置面板：Teleport 到 body（config-box 是滚动容器，内部绝对定位会被裁剪），
             位置由 toggleSettings 依触发钮实测矩形计算，空间不足自动上翻。 -->
        <Teleport to="body">
          <div
            v-if="settingsOpen"
            class="rs-pop"
            data-settings-pop
            role="dialog"
            :aria-label="t('editor.reqSettings')"
            :style="{ left: `${settingsPos.left}px`, top: `${settingsPos.top}px` }"
          >
            <label class="rs-field" :title="timeoutPlaceholder">
              <span class="rs-text">{{ t('editor.timeoutLabel') }}</span>
              <CustomNumberInput
                class="rs-timeout"
                size="sm"
                tone="inset"
                :model-value="draft.request.timeout_ms ?? ''"
                :placeholder="timeoutPlaceholder"
                @update:model-value="onTimeoutInput"
              />
              <span class="rs-unit">{{ t('editor.msUnit') }}</span>
            </label>
            <label class="rs-check" :title="t('editor.followRedirects')">
              <input
                :checked="draft.request.follow_redirects"
                type="checkbox"
                @change="draft.request.follow_redirects = ($event.target as HTMLInputElement).checked"
              />
              {{ t('editor.followRedirects') }}
            </label>
          </div>
        </Teleport>

        <ParamsPanel v-if="activeTab === 'params'" :draft="draft" />
        <AuthPanel v-else-if="activeTab === 'auth'" :draft="draft" />
        <HeadersPanel v-else-if="activeTab === 'headers'" :draft="draft" />
        <BodyPanel ref="bodyPanelRef" v-else-if="activeTab === 'body'" :draft="draft" />
        <GrpcPanel v-else-if="activeTab === 'grpc'" :draft="draft" />
        <PathVariablesPanel v-else-if="activeTab === 'path'" :draft="draft" />
        <RequestExamplesPanel v-else-if="activeTab === 'examples'" :draft="draft" />
        <ScriptsPanel v-else-if="activeTab === 'scripts'" :draft="draft" />
        <CodePanel
          v-else
          :method="draft.method"
          :url="requestUrl"
          :headers="draft.request.headers"
          :body="draft.request.body"
          :auth="draft.request.auth"
        />
      </div>

    <!-- 物理分割条 = 唯一拖拽热区，也是请求面板与响应区之间唯一的一条视觉分界线：
           两卡已被 .req-resp-stack 合并为一张卡，此处即那条接缝。
           ::after 为上下外扩的透明命中热区；::before 承载 hover/拖拽时的可拖提示线。
           未发送期同样可拖：enterManualSplit 切固定高，编辑器 availH 经 ResizeObserver
           跟随——看到底线即可拖。 -->
      <div
        class="rp-splitter"
        :class="{ dragging: splitterDragging }"
        role="separator"
        tabindex="0"
        aria-orientation="horizontal"
        :aria-label="t('editor.splitterHint')"
        :aria-valuenow="Math.round(requestBodyHeight)"
        :aria-valuemin="80"
        :aria-valuemax="Math.round(requestMaxHeight())"
        :title="t('editor.splitterHint')"
        @mousedown="onSplitterDown"
        @dblclick="toggleRequestBody"
        @keydown="onSplitterKeydown"
      >
        <!-- 常驻微把手：静止期低透明度提示「这里可以拖」（可发现性）；hover/拖拽加深 -->
        <span class="rp-splitter-grip" aria-hidden="true"></span>
      </div>

      <template v-if="hasResponse">
        <div class="response-zone">
          <!-- Apifox 式请求中占位：转圈 + 实时计时 + 骨架 shimmer，一眼可知「这次发出去了」。 -->
          <div v-if="sending && !response && !sendError" class="req-loading" role="status" aria-live="polite">
            <span class="req-loading-ring" aria-hidden="true"></span>
            <p class="req-loading-title">{{ t('editor.sendingTitle') }}</p>
            <p class="req-loading-sub">{{ draft.method }} {{ requestUrl }}</p>
            <div class="req-loading-skel">
              <Skeleton :lines="5" height="12px" />
            </div>
            <button class="miyo-btn miyo-btn-sm req-loading-cancel" type="button" @click="cancelSend">
              {{ t('editor.cancelRequest') }}
            </button>
          </div>
          <div v-else ref="flashEl" class="response-anim" :class="{ 'is-stale': sending }">
            <GrpcResponsePanel
              v-if="isGrpc && (grpcDisplay.response || grpcDisplay.stream)"
              :method="draft.method"
              :response="grpcDisplay.response"
              :stream="grpcDisplay.stream"
            />
            <ResponsePanel
              v-else-if="!isGrpc && response"
              :response="response"
              :load-full-body="loadFullBody"
              :examples-count="activeExamples.length"
              @save-example="saveExample"
              @drag-start="onSplitterDown"
            >
              <!-- 响应示例移入响应面板页签：不再作为独立卡片压在响应区下方，
                   既省垂直空间，也不再把主视区顶出去。数据仍由本组件持有。 -->
              <template #examples>
                <div class="examples">
                  <div v-for="ex in activeExamples" :key="ex.id" class="example-row">
                    <button
                      class="example-main"
                      type="button"
                      :class="{ open: viewingExample?.id === ex.id }"
                      @click="viewExample(ex)"
                    >
                      <span class="example-status" :class="{ err: ex.status >= 400 }">{{ ex.status }}</span>
                      <span class="example-name">{{ ex.name }}</span>
                      <span class="example-meta">{{ ex.created_at.slice(0, 16).replace('T', ' ') }}</span>
                    </button>
                    <Popconfirm
                      :title="t('editor.deleteExampleConfirm', { name: ex.name })"
                      @confirm="removeExample(ex)"
                    >
                      <IconButton name="trash" :size="13" tone="danger" :title="t('editor.deleteExample')" />
                    </Popconfirm>
                  </div>
                  <pre v-if="viewingExample" class="example-body">{{ prettyBody(viewingExample.body) }}</pre>
                </div>
              </template>
            </ResponsePanel>
            <div v-else-if="sendError" class="send-error" role="alert">
              <span>{{ t('editor.sendFail', { v: sendError }) }}</span>
            </div>
            <EmptyState
              v-else
              class="response-empty"
              icon="send"
              :title="t('editor.notSent')"
              :description="t('editor.notSentHint')"
            />
          </div>
        </div>
      </template>
      <!-- 未发送期占位低条：手动拖过分割条（请求卡固定高）时改为吸收剩余空间，
           保持「请求卡底边分割条 = 视觉分界线」且低条始终贴到工作区底部 -->
      <button
        v-else
        class="response-hint"
        :class="{ 'flex-fill': requestManual }"
        type="button"
        @click="urlInputEl?.focus()"
      >
        <span class="response-hint-inner">
          <Icon name="send" :size="20" />
          <span>{{ t('editor.responseHint') }}</span>
        </span>
      </button>
    </div>

    </template>
    <!-- 设计 / 文档 / 用例 / Mock：占满工作区主体的**独立视图**（不是并排面板）。
         这里保持 v-if（按需挂载）：懒加载面板提前挂载没有收益，还会让懒模块的
         请求越过测试环境 teardown（CI vitest 的 EnvironmentTeardownError 就源于此）。
         请求/响应栈用 v-show 常驻（见上方 template），分割高度与响应结果本就不丢；
         本组件另按接口 id 分桶记住自己的高度，切回来自动恢复。 -->
    <DesignPanel v-else-if="store.activeView === 'design'" :draft="draft" @save="save" />
    <DocsPanel
      v-else-if="store.activeView === 'docs'"
      :draft="draft"
      :url="requestUrl"
    />
    <TestCasesPanel v-else-if="store.activeView === 'cases'" :draft="draft" />
    <MockPanel
      v-else-if="store.activeView === 'mock'"
      :draft="draft"
      @open-manager="showMockRules = true"
    />
  </div>
  <div v-else class="editor-empty">
    <Icon name="file" :size="26" class="editor-empty-icon" aria-hidden="true" />
    <p>{{ t('editor.empty') }}</p>
    <button class="miyo-btn miyo-btn-primary" type="button" @click="store.openNewEndpoint(null)">
      <Icon name="plus" :size="13" /> {{ t('editor.emptyNew') }}
    </button>
  </div>

  <Modal v-model:open="showNameDialog" :title="t('editor.saveEndpoint')" width="420px">
    <p class="name-hint">{{ t('editor.nameHint') }}</p>
    <input
      v-model="pendingName"
      class="miyo-input name-dialog-input"
      :placeholder="t('editor.namePh')"
      spellcheck="false"
      @keyup.enter="confirmName"
    />
    <p class="name-hint folder-hint">{{ t('editor.saveFolderHint') }}</p>
    <CustomSelect
      class="save-folder-select"
      :model-value="pendingFolderId"
      :options="folderOptions"
      :placeholder="t('editor.saveFolderPh')"
      @update:model-value="pendingFolderId = String($event)"
    >
      <template #display="{ label }">
        <span class="save-folder-display">{{ label || t('editor.saveFolderPh') }}</span>
      </template>
      <template #option="{ option }">
        <span :style="{ paddingLeft: `${(option as FolderOption).depth * 16 + 4}px` }">
          {{ option.label }}
        </span>
      </template>
    </CustomSelect>
    <template #footer>
      <button class="miyo-btn" type="button" @click="showNameDialog = false">{{ t('common.cancel') }}</button>
      <button class="miyo-btn miyo-btn-primary" type="button" @click="confirmName">
        <Icon name="save" :size="14" /> {{ t('common.save') }}
      </button>
    </template>
  </Modal>

  <Modal v-model:open="showExampleDialog" :title="t('editor.saveExampleTitle')" width="360px">
    <p class="name-hint">{{ t('editor.exampleNameHint') }}</p>
    <input
      v-model="exampleName"
      class="miyo-input name-dialog-input"
      :placeholder="t('editor.exampleNamePh')"
      spellcheck="false"
      @keyup.enter="confirmSaveExample"
    />
    <template #footer>
      <button class="miyo-btn" type="button" @click="showExampleDialog = false">{{ t('common.cancel') }}</button>
      <button class="miyo-btn miyo-btn-primary" type="button" @click="confirmSaveExample">{{ t('common.save') }}</button>
    </template>
  </Modal>

  <ToolsDrawer :open="showTools" :draft="draft" :url="requestUrl" @close="showTools = false" />

  <EnvironmentManager v-model:open="showEnvManager" />

  <TestCaseModal
    :open="showTestCaseModal"
    :title="t('editor.saveAsCase')"
    :name="pendingCaseName"
    @update:open="showTestCaseModal = $event"
    @submit="onSaveCaseSubmit"
  />

  <MockRuleDialog v-if="showMockRules" @close="showMockRules = false" />

  <Menu ref="saveMenuEl" @select="onSaveMenuSelect" />
  <Menu ref="historyMenu" @select="onUrlHistorySelect" />
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  /* 卡片间距：请求区面板与响应区面板之间 14px（gap-3.5） */
  gap: 14px;
  padding: 12px;
  overflow-y: auto;
  height: 100%;
}

.editor-row {
  display: flex;
  gap: 8px;
  align-items: center;
}

.sub-nav {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-2);
  align-self: flex-start;
}

.sub-nav-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 14px;
  border: none;
  border-radius: 6px;
  background: none;
  font-family: inherit;
  font-size: 12.5px;
  color: var(--text-2);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.sub-nav-item:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.sub-nav-item.active {
  color: var(--text-1);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-sm);
}

.sub-nav-badge {
  min-width: 16px;
  padding: 0 5px;
  border-radius: 999px;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 16px;
  text-align: center;
  color: var(--accent);
  background: var(--accent-tint, rgba(168, 85, 247, 0.16));
}

.save-group {
  display: flex;
  align-items: stretch;
}
.save-main {
  border-radius: 7px 0 0 7px;
}
.save-arrow {
  border-radius: 0 7px 7px 0;
  border-left: 1px solid var(--border-strong);
  padding: 0 7px;
}

.method-select {
  width: 108px;
  flex-shrink: 0;
  font-weight: 700;
}

/* 方法文本色统一走 utils/methodTone.ts（text-method-*），此处不再定义。 */

/* 统一请求栏（Input Group）：方法徽章 + 基础URL + 路径输入同处一容器，
 * align-items 居中衔接（圆角 --radius-md，边框走 border-editor 的半透细线档） */
.request-bar {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  height: var(--h-md);
  border: 1px solid var(--border-editor);
  border-radius: var(--radius-md);
  background: var(--bg-card);
  overflow: hidden;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.request-bar:hover {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent-tint);
}
.request-bar:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

/* Method Badge：与侧栏标签块一致——淡底 + 方法色 + mono semibold（色源 methodBadgeTone） */
.request-bar .method-select {
  width: auto;
  flex: 0 0 auto;
  border: none;
  background: transparent;
}
.request-bar .method-select :deep(.cs-trigger) {
  height: 100%;
  padding: 0 10px;
  border: none;
  background: transparent;
  box-shadow: none;
  border-radius: 0;
}
.method-badge {
  display: inline-flex;
  align-items: center;
  padding: 3px 10px;
  border-radius: var(--radius);
  font-family: var(--font-mono);
  font-weight: 600;
  font-size: 12px;
  letter-spacing: 0.04em;
  line-height: 1.4;
}

/* 短分隔线：容器已改 align-items 居中，需自带高度（16px 居中短档，避免整高通顶断层） */
.req-bar-divider {
  width: 1px;
  height: 16px;
  flex-shrink: 0;
  background: var(--border);
}

/* Tooltip 触发包裹层需允许收缩，避免挤压路径输入 */
.request-bar :deep(.tt-trigger) {
  min-width: 0;
}

/* Base URL 前置选择器（Apifox 式）：占满请求栏高度，点击展开环境基址列表 */
.base-url-wrap {
  position: relative;
  display: inline-flex;
  align-self: stretch;
  min-width: 0;
  max-width: 300px;
}
.base-url-tip {
  display: inline-flex;
  flex: 1;
  min-width: 0;
}
.request-bar .base-url-select {
  flex: 1;
  min-width: 0;
  max-width: none;
}
.request-bar .base-url-select :deep(.cs-trigger) {
  height: 100%;
  gap: 6px;
  border: none;
  background: transparent;
  border-radius: 0;
  box-shadow: none;
  font-family: var(--font-mono);
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
  /* 右侧预留复制按钮 + caret 位，防域名文字钻入按钮下 */
  padding: 0 44px 0 10px;
}
.request-bar .base-url-select :deep(.cs-trigger:hover) {
  background: var(--bg-hover);
  color: var(--text-1);
}
.request-bar .base-url-select :deep(.cs.open .cs-trigger) {
  box-shadow: none;
}
.request-bar .base-url-select :deep(.cs-value) {
  color: inherit;
}
.request-bar .base-url-select.env :deep(.cs-value) {
  color: var(--accent);
}
.request-bar .base-url-select.warn :deep(.cs-value) {
  color: var(--warning);
}

.env-badge-icon {
  flex-shrink: 0;
  opacity: 0.8;
}

.env-badge-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Base URL 复制按钮：悬浮在触发器右侧（caret 左），不挤占域名展示宽度 */
.base-url-copy {
  position: absolute;
  top: 50%;
  right: 20px;
  transform: translateY(-50%);
  display: inline-flex;
  z-index: 1;
}
.base-url-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: 5px;
  padding: 0;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.base-url-copy-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* 下拉选项：左前置 URL，右环境名（Apifox 样式） */
:global(.cs-pop.base-url-pop) {
  padding: 6px;
  border-radius: var(--radius-lg);
}
:global(.cs-pop.base-url-pop .cs-opt) {
  height: auto;
  min-height: 34px;
  padding: 6px 8px;
  gap: 12px;
  border-radius: var(--radius-sm);
  font-family: var(--font-ui);
  white-space: normal;
}
:global(.cs-pop.base-url-pop .cs-opt.hl) {
  background: var(--bg-hover);
}
/* 选中：accent 轻底 + 对勾高亮，文字保持正文色（默认 .sel 会整行染 accent） */
:global(.cs-pop.base-url-pop .cs-opt.sel) {
  color: var(--text-1);
  background: var(--accent-tint);
}
:global(.cs-pop.base-url-pop .cs-opt.sel .cs-opt-check) {
  color: var(--accent);
}
:global(.cs-pop.base-url-pop .cs-opt-check) {
  width: 16px;
}
:global(.cs-pop.base-url-pop .cs-opt-label) {
  display: flex;
  align-items: center;
  gap: 16px;
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
}
.base-url-opt-url {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
}
.base-url-opt-name {
  flex: 0 0 auto;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-3);
  font-size: var(--fs-xs);
  text-align: right;
}
.base-url-manage {
  font-size: 12px;
  font-family: var(--font-ui);
  color: var(--text-3);
}
/* 环境未配 Base URL：该行左列是回退的会话前缀（非该环境自身配置），给微标记去歧义 */
.base-url-opt-hint {
  flex: 0 0 auto;
  padding: 0 5px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-3);
  font-size: var(--fs-xxs);
  line-height: 15px;
}
/* 「无环境」行的会话前缀重置入口：仅在值偏离内置默认时渲染 */
.base-url-reset {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.base-url-reset:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
/* 分组标题与本弹层选项文本列对齐（.cs-opt 内距 6px 8px，默认 12px 会错位） */
:global(.cs-pop.base-url-pop .cs-opt-header) {
  padding-left: 8px;
  padding-right: 8px;
}
:global(.cs-pop.base-url-pop .cs-opt:hover .base-url-manage),
:global(.cs-pop.base-url-pop .cs-opt.hl .base-url-manage) {
  color: var(--text-2);
}
:global(.cs-pop.base-url-pop .cs-opt:last-child) {
  margin-top: 4px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-sm) var(--radius-sm);
}

.request-bar .url-input-wrap {
  position: relative;
  display: flex;
  align-items: center;
  flex: 1;
  min-width: 0;
  height: 100%;
}

.request-bar .url-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  background: transparent;
  box-shadow: none;
  border-radius: 0;
  /* 右侧快捷图标区 84px（历史/复制/清空三枚）+ gap 8px：正文与图标组留出间距 */
  padding: 0 90px 0 10px;
  font-family: var(--font-mono);
}

/* 地址栏快捷按钮：悬浮输入框时浮现（Tooltip 触发器 span 承载绝对定位）
 * 从右到左：历史(6) → 清空(32) → 复制(58) */
.url-qbtn {
  position: absolute;
  top: 50%;
  right: 6px;
  transform: translateY(-50%);
  width: 26px;
  height: 26px;
  border-radius: 6px;
  color: var(--text-2);
  cursor: pointer;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.url-qbtn-copy {
  right: 58px;
}
.url-qbtn-clear {
  right: 32px;
}
.url-qbtn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.url-qbtn-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  border: none;
  background: none;
  color: inherit;
  cursor: pointer;
  border-radius: 6px;
  padding: 0;
}

/* 路径变量缺失提示条：URL 栏下轻量单行，chips 点击直达 Path 页签 */
.pathvar-hint {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  margin: 6px 16px 0;
  padding: 4px 10px;
  border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
  border-radius: var(--radius-md);
  background: var(--warning-tint);
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.pathvar-hint-icon {
  flex-shrink: 0;
  color: var(--warning);
}
.pathvar-hint-label {
  flex-shrink: 0;
  color: var(--text-2);
}
.pathvar-chip {
  display: inline-flex;
  align-items: center;
  height: 20px;
  padding: 0 8px;
  border: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
  border-radius: 999px;
  background: var(--bg-card);
  font-size: var(--fs-xxs);
  color: var(--text-1);
  cursor: pointer;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
}
.pathvar-chip:hover {
  border-color: var(--warning);
  background: var(--bg-hover);
}

/* 请求栏右侧「发送」按钮：与输入组无缝贴合 */
.bar-send {
  height: 100%;
  flex-shrink: 0;
  border-radius: 0;
  padding: 0 16px;
  font-weight: 600;
}
/* 发送中按钮：转圈 + 计时 + 呼吸脉冲（计时只在此处显示）。 */
.bar-send.is-sending {
  min-width: 168px;
  animation: bar-send-pulse 1.4s ease-in-out infinite;
}
.bar-send-elapsed {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.btn-spinner {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, 0.35);
  border-top-color: #fff;
  animation: btn-spin 0.7s linear infinite;
}
@keyframes btn-spin {
  to { transform: rotate(360deg); }
}
@keyframes bar-send-pulse {
  0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--danger) 45%, transparent); }
  50% { box-shadow: 0 0 0 7px transparent; }
}

/* 请求栏发送中：顶部流光 + 边框高亮（纯动画，无新增展示块）。 */
.request-bar {
  position: relative;
}
.request-bar.is-sending {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}
.req-progress {
  position: absolute;
  left: 0;
  right: 0;
  top: 0;
  height: 2px;
  overflow: hidden;
  background: transparent;
  z-index: 2;
}
.req-progress::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  width: 40%;
  border-radius: 999px;
  background: linear-gradient(90deg, transparent, var(--accent), transparent);
  box-shadow: 0 0 8px var(--accent);
  animation: req-slide 0.9s ease-in-out infinite;
}
@keyframes req-slide {
  from { left: -40%; }
  to { left: 100%; }
}

/* Apifox 式请求中占位：大转圈 + 跳动计时 + 骨架 shimmer。 */
.req-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 36px 20px 28px;
  border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
  border-radius: var(--radius);
  background: var(--bg-card);
  text-align: center;
}
.req-loading-ring {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  border: 3px solid color-mix(in srgb, var(--accent) 25%, transparent);
  border-top-color: var(--accent);
  animation: btn-spin 0.7s linear infinite;
}
.req-loading-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.req-loading-sub {
  margin: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.req-loading-skel {
  width: 100%;
  margin-top: 6px;
  opacity: 0.85;
}
.req-loading-cancel {
  margin-top: 4px;
}

/* 响应体动画钩子：仅过渡 + 到达闪光，不新增展示块。 */
.response-anim {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  border-radius: var(--radius);
  transition:
    opacity var(--dur) var(--ease),
    filter var(--dur) var(--ease);
}
/* 新请求在途时旧响应降 dim（过渡动画，告知「正在刷新」。 */
.response-anim.is-stale {
  opacity: 0.55;
  filter: saturate(0.7);
}
/* 新响应到达：面板光晕闪光 + 状态徽章弹跳（Apifox 式到达感）。 */
.response-anim.flash {
  animation: resp-flash 1.1s ease-out;
}
.response-anim.flash :deep(.rp-status) {
  animation: status-pop 0.55s ease-out;
}
@keyframes resp-flash {
  0% {
    box-shadow:
      0 0 0 2px var(--accent),
      0 0 28px var(--accent-tint);
    background: var(--accent-tint);
  }
  60% { box-shadow: 0 0 0 1px var(--accent); }
  100% { box-shadow: 0 0 0 0 transparent; background: transparent; }
}
@keyframes status-pop {
  0% { transform: scale(0.6); }
  55% { transform: scale(1.14); }
  100% { transform: scale(1); }
}
@media (prefers-reduced-motion: reduce) {
  .bar-send.is-sending,
  .btn-spinner,
  .req-progress::after,
  .req-loading-ring,
  .response-anim.flash,
  .response-anim.flash :deep(.rp-status) {
    animation: none;
  }
}

/* ---- 面包屑行（接口名称移至此处） ---- */
.breadcrumb-row {
  gap: 8px;
}

.crumb {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
  font-size: var(--fs-sm);
}

.crumb-part {
  color: var(--text-2);
  white-space: nowrap;
}

.crumb-sep {
  color: var(--text-3);
}

/* 接口标题：内联编辑样式——常态为面包屑文本，hover 显示虚线提示可编辑，聚焦高亮 */
.crumb-name {
  min-width: 60px;
  max-width: 280px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  background: transparent;
  border: none;
  border-bottom: 1px dashed transparent;
  border-radius: 0;
  padding: 1px 2px;
  cursor: text;
  transition: border-bottom-color var(--dur) var(--ease);
}

.crumb-name:hover {
  border-bottom-color: var(--text-3);
  background: transparent;
}

.crumb-name:focus {
  outline: none;
  border-bottom-color: var(--accent);
  background: transparent;
}

.breadcrumb-spacer {
  flex: 1;
}

.editor-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  /* 与发送按钮（主 CTA）之间保持 12px 独立间隔（ml-3） */
  margin-left: 12px;
}

/* 动作区统一 32px（与发送按钮同高）；⋯ 下拉为纯图标钮 */
.actions-more {
  height: var(--h-md);
  padding: 0 8px;
}

/* 请求卡 + 响应卡合并容器：两卡之间无 gap，接缝由 .rp-splitter 独占一条线。
 * 合并前两卡各带完整边框、相距 editor 的 14px gap，形成两条平行线；
 * 拖拽热区与两条线都不重合，导致「看着有线却拖不动」。 */
.req-resp-stack {
  position: relative;
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-height: 0;
  /* 外框与圆角由容器统一提供，两卡自身不再画框（否则接缝处又会多出线） */
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-panel);
  background: var(--bg-card);
  overflow: hidden;
}
/* 未发送期：请求卡吃满剩余高度，占位低条贴底——外框仍由容器负责 */
.req-resp-stack.no-response {
  flex: 1 1 auto;
}
/* 核心输入区卡片：比全局背景浅一档，微阴影 + 圆角增强悬浮感 */
.config-box {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex-shrink: 0;
  min-height: 0;
  overflow-y: auto;
  background: var(--bg-card);
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-panel);
  padding: 10px 12px;
}

/* 无响应阶段：请求卡吃掉剩余纵向空间（Body 编辑器填满卡片，
 * 分割条拖动即时跟随），响应空态收为固定低条——
 * 避免「请求区贴内容 + 空态占 2/3 屏」的失衡；出现响应后仍转分割条分配的固定高度 */
.config-box.grow {
  flex: 1 1 auto;
  /* 窗口偏矮时不再一路压扁请求卡：保底到 Body 编辑器可用的整卡高度
   * （页签行 + 模式条 + 编辑器），缺的纵向空间由可压缩的空态低条
   * （120px 下限）与 .editor 纵向滚动承担——避免「打开 POST 接口 Body 只剩一条线」 */
  min-height: 320px;
}

/* ---- 页签行：左页签 + 右侧「请求设置」触发钮 ----
 * 超时/重定向是低频设置，收进 Popover（Teleport 到 body）；
 * 请求卡与响应区之间的接缝处不再常驻一行，Body 编辑器正下方就是分割条。 */
.config-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

/* 配置页签占位后推：右侧「响应区最大化 / 请求设置」靠右成组 */
.config-head-spacer {
  flex: 1;
  min-width: 4px;
}

/* 响应区最大化按钮：与请求设置同形态的纯图标钮 */
.req-max-toggle {
  padding: 0 6px;
}

.rs-trigger {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
  margin-left: auto;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xs);
  cursor: pointer;
  user-select: none;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}

.rs-trigger:hover,
.rs-trigger.open {
  background: var(--bg-hover);
  color: var(--text-1);
}

/* 非默认设置角标：该请求设了超时或关闭了重定向 */
.rs-dot {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent);
}

/* 请求设置 Popover（Teleport 到 body，fixed 定位） */
.rs-pop {
  position: fixed;
  z-index: 90;
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 220px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  box-shadow: 0 8px 24px rgb(0 0 0 / 0.14);
  animation: rs-pop 130ms var(--ease);
}

@keyframes rs-pop {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}

/* 跟随重定向勾选框：appearance 自绘（14px 圆角框 + 对勾），与 inset 输入框同一描边语言 */
.rs-check input[type='checkbox'] {
  appearance: none;
  width: 14px;
  height: 14px;
  margin: 0;
  flex-shrink: 0;
  display: inline-grid;
  place-content: center;
  border: 1px solid var(--border-editor);
  border-radius: 4px;
  /* 井底走 --check-bg（深 26% 黑 / 浅 5% 黑） */
  background: var(--check-bg);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.rs-check input[type='checkbox']::before {
  content: '';
  width: 8px;
  height: 8px;
  transform: scale(0);
  clip-path: polygon(14% 44%, 0 65%, 50% 100%, 100% 16%, 80% 0%, 43% 62%);
  background: #fff;
  transition: transform 120ms var(--ease);
}
.rs-check input[type='checkbox']:checked {
  background: var(--accent);
  border-color: var(--accent);
}
.rs-check input[type='checkbox']:checked::before {
  transform: scale(1);
}
.rs-field {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.rs-text {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.rs-timeout {
  width: 96px;
}
.rs-unit {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.rs-check {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  user-select: none;
}
.rs-check input {
  accent-color: var(--accent);
}

/* ---- 响应占位区（未发送）：干净黑底 + white/8 顶边细线，占满请求卡以下空间；
 * 图标 + 文案整组绝对居中（flex items-center justify-center），不贴底不贴顶 ---- */
.response-hint {
  margin: 0;
  /* 固定低条（可压缩至 120）：纵向空间让给 .config-box.grow 请求卡 */
  flex: 0 1 240px;
  min-height: 120px;
  /* button 化（点击聚焦 URL 栏）：清默认外观，保留既有表面样式 */
  width: 100%;
  padding: 0;
  appearance: none;
  font: inherit;
  border: none;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-app);
  border-top: 1px solid var(--border-panel);
  border-radius: var(--radius-md);
  user-select: none;
  cursor: pointer;
}
.response-hint:hover .response-hint-inner,
.response-hint:focus-visible .response-hint-inner {
  color: var(--text-2);
}
/* 未发送期手动分割后：请求卡固定高，占位低条 flex:1 吸收剩余空间补位到底 */
.response-hint.flex-fill {
  flex: 1 1 auto;
}
.response-hint:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}
.response-hint-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  color: var(--text-3);
}
.response-hint-inner svg {
  opacity: 0.75;
}
.response-hint-inner span {
  margin: 0;
  font-size: var(--fs-xs);
}

/* ---- 请求区 / 响应区分割条（Single Border Architecture：唯一分隔线）----
 * 请求编辑器底部、响应面板顶部均无边框；仅分割条提供 1px 视觉分隔。
 * 负 margin 抵消 .editor 的 gap，让请求编辑器底边紧贴分割条（2px），
 * 响应面板与分割条之间保留 6px 呼吸空间。
 */
/* 两卡接缝处的分割条：它是请求区与响应区之间唯一的一条线，也是唯一拖拽热区。
 * 合并进 .req-resp-stack 后上下已无 gap，故不再用负 margin「骗」位置——
 * 负 margin 会让实际盒与视觉边界错位，出现「看着线在那里、拖动却没反应」。 */
/* 两卡接缝处的分割条：它是请求区与响应区之间唯一的那条线，也是唯一拖拽热区。
 * 高 1px = 线本身（不再额外占位），让「线」与「把手」是同一个像素行；
 * 若给它独立高度，线就会被推离 Body 编辑器底边。
 * 命中热区由 ::after 向上下外扩承担，不靠盒高。 */
.rp-splitter {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 1px;
  flex-shrink: 0;
  margin: 0;
  /* 静止时：这 1px 就是那条接缝线（走主题变量，双主题可见）。 */
  background: var(--border-panel);
  cursor: row-resize;
  user-select: none;
  touch-action: none;
}
/* 命中热区：以 1px 线为中心上下各扩 12px（合计 25px），
 * 即使起点略偏也能抓中，直接解决「拖不动 / 拖错地方」。
 * 本层必须完全透明——有任何背景都会变成第二条视觉线。 */
.rp-splitter::after {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  top: -12px;
  bottom: -12px;
}
/* 拖拽反馈层（严格单线）：
 * ::after 已被透明热区占用，故用 ::before 承载「高亮」。
 * 以自身（即真实分割线）为中心向上下各扩，让高亮线与真实线完全重合——
 * 「高亮哪条线，拖的就是哪条线」。 */
.rp-splitter::before {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  top: 50%;
  /* 默认 0 高：完全不可见，不引入第二条线 */
  height: 0;
  transform: translateY(-50%);
  background: var(--accent);
  opacity: 0;
  border-radius: 1px;
  transition:
    height 0.15s ease,
    opacity 0.15s ease;
  pointer-events: none;
}
.rp-splitter:hover::before,
.rp-splitter.dragging::before {
  height: 3px;
  opacity: 0.85;
}
/* 拖拽中更实、更明显（此时用户已明确进入调整状态） */
.rp-splitter.dragging::before {
  opacity: 1;
}
/* 常驻微把手：静止期低透明度短横条，hover / 拖拽 / 键盘聚焦时加深——
 * 1px 接缝线在浅色主题下易被忽略（可发现性），把手给出明确的可拖暗示。
 * 不抢焦点：pointer-events none，热区仍是整条 ::after。 */
.rp-splitter-grip {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 26px;
  height: 4px;
  transform: translate(-50%, -50%);
  border-radius: 999px;
  background: var(--text-3);
  opacity: 0.32;
  transition: opacity 0.15s ease;
  pointer-events: none;
}
.rp-splitter:hover .rp-splitter-grip,
.rp-splitter.dragging .rp-splitter-grip,
.rp-splitter:focus-visible .rp-splitter-grip {
  opacity: 0.7;
}
/* 把手居中会盖住 ::before 的高亮线——高亮时把手让位为浅色（线仍清晰） */
.rp-splitter.dragging .rp-splitter-grip {
  background: var(--bg-elevated);
  opacity: 0.9;
}
/* 仅在键盘聚焦时给描边：鼠标点击（含按下拖拽）不应留下「选中框」，
 * 那会让人误以为拖出了什么选框。:focus-visible 在鼠标交互下本就不触发，
 * 但 mousedown 里 preventDefault 后仍需显式排除，避免拖完残留。 */
.rp-splitter:focus:not(:focus-visible) {
  outline: none;
}
.rp-splitter:focus-visible {
  outline: 2px solid var(--focus-ring);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}

.kv-remove {
  color: var(--miyo-text-muted);
}

.body-mode-select {
  width: 200px;
}

.mp-type {
  width: 110px;
  flex-shrink: 0;
}

.oauth-form {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.oauth-hint {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: var(--miyo-text-muted);
}

.oauth-status.ok {
  color: var(--miyo-success);
}

.response-save {
  margin-left: auto;
}

.examples {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.example-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.example-main {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--miyo-border);
  background: var(--miyo-input-bg);
  border-radius: 6px;
  padding: 5px 10px;
  cursor: pointer;
  color: var(--miyo-text, #f9fafb);
  font-size: 12.5px;
  text-align: left;
}

.example-main.open {
  border-color: var(--miyo-info);
}

.example-status {
  font-weight: 700;
  font-size: var(--fs-xxs);
  color: var(--miyo-success);
}

.example-status.err {
  color: var(--miyo-danger);
}

.example-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.example-meta {
  font-size: var(--fs-xxs);
  color: var(--miyo-text-muted);
}

.example-body {
  margin: 0;
  padding: 10px 12px;
  background: var(--miyo-input-bg);
  border: 1px solid var(--miyo-border);
  border-radius: 6px;
  font-family: var(--font-mono);
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 240px;
  overflow-y: auto;
}

.send-error {
  padding: 10px 12px;
  border-radius: var(--radius);
  background: var(--danger-tint);
  border: 1px solid var(--danger-border);
  color: var(--danger);
  font-size: 12.5px;
}

/* ---- 响应容器：flex:1 填满分割条以下所有空间 ---- */
.response-zone {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

.response-empty {
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius);
  background: var(--bg-card);
}

.editor-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  height: 100%;
  color: var(--text-3);
}

.editor-empty-icon {
  color: var(--text-4, var(--text-3));
  opacity: 0.7;
}

.editor-empty .miyo-btn {
  margin-top: 4px;
}

/* ---- 名称输入对话框 ---- */

.name-hint {
  margin: 0 0 8px;
  font-size: 12.5px;
  color: var(--text-2);
}

.name-dialog-input {
  width: 100%;
  height: var(--h-md);
}

.folder-hint {
  margin-top: 12px;
}

.save-folder-select {
  width: 100%;
}

.save-folder-display {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 合并容器（.req-resp-stack）内的覆盖：必须排在所有基础规则之后 ----
 * 目标：请求卡与响应卡拼成一张视觉整卡，接缝处只留一条线。
 * 放在文件末尾而非上方：scoped CSS 同权重下后者胜，写在前面的规则会被
 * .config-box / .response-hint / .response-zone 的基础 border 覆盖掉。
 * （本条曾在文件中段插入，同样被后面的基础规则反超 → 两条线复发。） */
.req-resp-stack > .config-box {
  /* 外框由容器提供；卡片底边紧贴分割条（那条接缝线由 .rp-splitter 绘制）。 */
  border: none;
  border-radius: 0;
  box-shadow: none;
  /* 恢复常规内距，Body 编辑器不贴卡底。 */
  padding-bottom: 10px;
}
.req-resp-stack .response-zone,
.req-resp-stack .response-hint {
  /* 接缝线由分割条承担，响应侧不再重复画线（否则又是两条）。 */
  border-top: none;
}
/* 子组件内部的类需用 :deep() 穿透——scoped 会把选择器编译成带本组件
 * 属性选择器的形式（.req-resp-stack .rp[data-v-本组件]），而 .rp 元素带的是
 * ResponsePanel 自己的 scope 属性，两者 hash 不同 → 规则永不匹配。
 * （这一条曾被漏掉，导致响应卡自带上边框与合并容器的边框叠成两条线。） */
.req-resp-stack :deep(.rp),
.req-resp-stack :deep(.response-empty) {
  border: none;
  border-radius: 0;
  box-shadow: none;
  background: transparent;
}
/* 未发送期占位低条：接缝线一条即可，其余表面交给容器 */
.req-resp-stack .response-hint {
  border-radius: 0;
}
</style>