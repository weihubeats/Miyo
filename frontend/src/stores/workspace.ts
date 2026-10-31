/**
 * workspace store：工作区的单一状态源（Pinia）。
 *
 * 职责：
 * 1. 持有激活项目 + 文件夹/接口的扁平列表（树形由组件递归组装）；
 * 2. 标签页管理：openTabs（接口 id 有序集合）+ activeTabId + drafts（未保存草稿）；
 * 3. 树/标签/编辑器统一通过本 store 读写，避免跨组件手递 ref。
 *
 * 草稿语义（阶段 3 对齐 Dioxus 版）：打开标签时克隆一份 Endpoint 为草稿，
 * 编辑只改草稿；「保存」调用 save_endpoint 后回写列表并清除脏标记。
 */
import { defineStore } from 'pinia'
import { computed, nextTick, ref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from './locale'
import { planCrossGroupMove, planSameGroupMove, wouldCreateCycle } from './treeOps'
import { deepClone } from '../utils/clone'
import { splitUrl } from '../utils/url'
import {
  BUILTIN_VARIABLES,
  envBaseUrl,
  findUnknownVars,
  mergedVariableMap,
  requestTemplateTexts,
  resolveRequestUrl,
  resolveVariables,
} from '../utils/environment'
import { applyCaseToRequest, restoreBody, snapshotRequest } from '../utils/testCases'
import { applyPathVars } from '../utils/pathVariables'
import type {
  AuthSpec,
  CurlParsed,
  Endpoint,
  Environment,
  EnvironmentVariable,
  ExecuteResponse,
  Folder,
  GlobalParam,
  GrpcEndEvent,
  GrpcInvokeResult,
  GrpcMessageEvent,
  HttpMethod,
  KeyValue,
  OAuth2Token,
  Project,
  RequestExample,
  RequestHistory,
  ResponseExample,
  Scenario,
  TestCase,
  TestCaseCategory,
  TestCaseStatus,
} from '../types/miyoApi'

/**
 * 后端内置默认超时（毫秒）。镜像 miyo-http::client::DEFAULT_TIMEOUT_MS（300s）：
 * 全局设置从未配置时，执行链路（resolve_timeout_ms）实际生效的就是这个值，
 * 前端展示（超时占位符 / 设置页兜底）必须与之一致；后端调整时需同步。
 */
export const FALLBACK_GLOBAL_TIMEOUT_MS = 300_000

/**
 * 无环境（会话）前缀的内置默认值。地址栏前置 URL 下拉的「无环境」行与
 * 「重置为默认前缀」入口同口径：展示、重置、恢复都取这一个常量。
 */
export const DEFAULT_SESSION_BASE_URL = 'http://localhost'

/** 新建接口的默认请求规格（与 miyo-core 模型字段一致）。 */
export function defaultRequestSpec(): Endpoint['request'] {
  return {
    params: [],
    headers: [],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    active_tab: null,
    timeout_ms: null,
    follow_redirects: true,
    tests: null,
    disable_cookies: false,
    body_docs: {},
  }
}

/**
 * 脏检查比较：先做廉价前置短路（标量、数组长度、大字段长度），不等直接判「不同」，
 * 全相等才回落全量 JSON.stringify。编辑场景几乎每次键入都命中短路——大 body
 * 草稿不再逐键双侧整包序列化；只有「改后改回原样」的确认相等路径走全量比较。
 * 短路只能提前返回 false（确有差异），不能提前返回 true，等价性不弱于全量比较。
 */
function eq(a: Endpoint, b: Endpoint): boolean {
  if (
    a.name !== b.name ||
    a.method !== b.method ||
    a.path !== b.path ||
    a.description !== b.description ||
    a.status !== b.status ||
    a.folder_id !== b.folder_id ||
    a.sort_order !== b.sort_order ||
    a.request.active_tab !== b.request.active_tab ||
    a.request.timeout_ms !== b.request.timeout_ms ||
    a.request.follow_redirects !== b.request.follow_redirects ||
    a.request.disable_cookies !== b.request.disable_cookies ||
    a.request.params.length !== b.request.params.length ||
    a.request.headers.length !== b.request.headers.length ||
    a.request.path_variables.length !== b.request.path_variables.length ||
    a.request.body.mode !== b.request.body.mode
  ) {
    return false
  }
  const ab = a.request.body
  const bb = b.request.body
  // 大体积字段长度不等即不同（长度相等仍可能内容不同，交由下方全量比较）
  if (ab.mode === 'json' && bb.mode === 'json') {
    if (ab.raw.length !== bb.raw.length) return false
  } else if (ab.mode === 'text' && bb.mode === 'text') {
    if (ab.raw.length !== bb.raw.length) return false
  } else if (ab.mode === 'urlencoded' && bb.mode === 'urlencoded') {
    if (ab.fields.length !== bb.fields.length) return false
  } else if (ab.mode === 'multipart' && bb.mode === 'multipart') {
    if (ab.fields.length !== bb.fields.length) return false
  } else if (ab.mode === 'binary' && bb.mode === 'binary') {
    if (ab.path !== bb.path) return false
  }
  return JSON.stringify(a) === JSON.stringify(b)
}

/** gRPC 服务端流运行态（stream_id → 状态；消息由 miyo:grpc-event 事件追加）。 */
export interface GrpcStreamState {
  streamId: string
  endpointId: string
  service: string
  method: string
  messages: GrpcMessageEvent[]
  status: 'running' | 'done' | 'failed'
  end: GrpcEndEvent | null
  error: string | null
}

export const useWorkspaceStore = defineStore('workspace', () => {
  const api = useMiyoApi()
  const toast = useToast()
  const locale = useLocaleStore()
  const t = locale.t

  const project = ref<Project | null>(null)
  const folders = ref<NonNullable<Awaited<ReturnType<typeof api.listFolders>>>[number][]>([])
  const endpoints = ref<Endpoint[]>([])
  const loadError = ref<string | null>(null)

  const openTabs = ref<string[]>([])
  const activeTabId = ref<string | null>(null)
  const drafts = ref<Map<string, Endpoint>>(new Map())

  /** 环境：当前项目的列表 + 当前选中（execute_request 的 environment_id 来源）。 */
  const environments = ref<Environment[]>([])
  const activeEnvId = ref<string | null>(null)

  /** 全局变量（跨项目共享，优先级最低的 {{变量}} 兜底表）。 */
  const globalVariables = ref<EnvironmentVariable[]>([])

  /** 全局参数（每个请求自动注入的 query / header）。 */
  const globalParams = ref<GlobalParam[]>([])

  /**
   * 无环境前缀的 Base URL（地址栏域名真源之一）：地址栏粘贴的 origin 写这里，
   * 按项目持久化到 localStorage——否则重启后回落 localhost，用户导入的自定义
   * 前缀域名丢失。选中环境且环境声明了 Base URL 时，展示前缀让位给
   * `{{base_url}}`（见 `urlDomain`），此时**不写**：写入会被环境前缀遮蔽、对
   * 用户不可见，事后切到「无环境」才冒出来，读起来像 bug（`openCurlDraft` /
   * `applyCurlToDraft` / `restoreFromHistory` / 地址栏整条 URL 提交同此规则）。
   */
  const sessionBaseUrl = ref(DEFAULT_SESSION_BASE_URL)
  /** 前缀持久化键前缀（按项目隔离，风格与 `miyo.open-projects` 一致）。 */
  const BASE_URL_KEY_PREFIX = 'miyo.base-url.'

  /**
   * 全局生效的请求超时（毫秒）：全局设置值；未配置时即内置默认（非 null，
   * 占位符展示「留空用全局（N 秒）」）。仅 IPC 失败时可能短暂为 null（回退静态文案）。
   */
  const globalTimeoutMs = ref<number | null>(null)

  /** 拉取全局请求超时（store 创建时 + 设置页保存后刷新）。 */
  async function refreshGlobalTimeoutMs(): Promise<void> {
    try {
      const ms = await api.getHttpTimeoutMs()
      globalTimeoutMs.value = ms ?? FALLBACK_GLOBAL_TIMEOUT_MS
    } catch {
      globalTimeoutMs.value = FALLBACK_GLOBAL_TIMEOUT_MS
    }
  }
  void refreshGlobalTimeoutMs()

  /** 地址栏域名前缀（唯一真实数据源）：选中环境声明 Base URL 时优先，否则回退会话 Base URL。 */
  const urlDomain = computed(() => {
    const env = environments.value.find((e) => e.id === activeEnvId.value)
    const base = envBaseUrl(env)
    if (base) return '{{base_url}}'
    return sessionBaseUrl.value || ''
  })

  /**
   * 从 localStorage 恢复当前项目的前缀（启动 `init` 与项目全新加载时调用）。
   * 无记录时回退内置默认 localhost。
   */
  function restoreSessionBaseUrl(): void {
    const pid = project.value?.id
    if (!pid) return
    let saved: string | null = null
    try {
      saved = localStorage.getItem(BASE_URL_KEY_PREFIX + pid)
    } catch {
      saved = null
    }
    sessionBaseUrl.value = saved ?? DEFAULT_SESSION_BASE_URL
  }

  /** 会话前缀恢复内置默认（「无环境」行的重置入口；watch 负责按项目持久化回写）。 */
  function resetSessionBaseUrl(): void {
    sessionBaseUrl.value = DEFAULT_SESSION_BASE_URL
  }

  // 前缀变更即按项目持久化（cURL 导入 / 地址栏粘贴 / 历史恢复 / 项目切换回写）。
  watch(sessionBaseUrl, (v) => {
    const pid = project.value?.id
    if (!pid) return
    try {
      localStorage.setItem(BASE_URL_KEY_PREFIX + pid, v)
    } catch {
      // 存储不可用时静默：仅影响重启后的前缀恢复
    }
  })

  /** 把当前选中环境的 Base URL 更新为 url（地址栏粘贴完整 URL 时同步环境）。 */
  async function setEnvironmentBaseUrl(url: string): Promise<void> {
    const env = environments.value.find((e) => e.id === activeEnvId.value)
    if (!env) return
    try {
      const saved = await saveEnvironmentVerified({ ...env, base_url: url })
      const idx = environments.value.findIndex((e) => e.id === env.id)
      if (idx !== -1) environments.value[idx] = saved
    } catch (err) {
      toast.error(t('ws.updateEnvFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }

  /** 响应示例（按接口 id 缓存，openEndpoint 时懒加载）。 */
  const examples = ref<Map<string, ResponseExample[]>>(new Map())

  const activeEndpoint = computed(() => {
    if (!activeTabId.value) return null
    return drafts.value.get(activeTabId.value) ?? null
  })

  /**
   * 脏检查定版机制：
   * eq() 是全量 JSON.stringify 比较，而 isDirty 被树行 / 标签页高频调用——
   * 若直接暴露，草稿每敲一键就会触发所有可见节点各自重新序列化整个
   * Endpoint（O(可见节点 × 草稿体积) 的「stringify 风暴」）。
   * 这里用 deep watch + 微任务合并把连续编辑折叠为一次 dirtyTick 自增，
   * 同一定版内的重复调用命中 dirtyCache，序列化每键批次至多一次。
   */
  const dirtyTick = ref(0)
  let dirtyScheduled = false
  watch(
    drafts,
    () => {
      if (dirtyScheduled) return
      dirtyScheduled = true
      nextTick(() => {
        dirtyScheduled = false
        dirtyTick.value++
      })
    },
    { deep: true },
  )
  // 列表整体刷新（load/refresh）同样会改变「已保存态」，推进定版使缓存失效
  watch(endpoints, () => {
    dirtyTick.value++
  })

  /** [定版本号, 结果]：仅当本版未计算过时才做全量比较。 */
  const dirtyCache = new Map<string, [number, boolean]>()

  /**
   * 新建接口的创建时快照：openNewEndpoint 产生的草稿尚无保存态，
   * 原来对「无保存态」一律判脏，导致刚 + 出来、一个字没改的空接口
   * 也显示小圆点、关闭弹二次确认。有快照时与快照比（改后删回原样
   * 也算干净）；保存 / 关闭标签时清理。
   * openCurlDraft / 历史恢复的草稿自带用户内容，不建快照、保持按脏处理。
   */
  const pristineBaselines = new Map<string, Endpoint>()

  /**
   * 保存态索引：isDirty / titleOf 原来每次调用都 `endpoints.find`
   *（O(标签数 × 接口数)），此处随列表刷新重建一次，查询 O(1)。
   */
  const savedIndex = computed(() => new Map(endpoints.value.map((e) => [e.id, e] as const)))

  const isDirty = (id: string): boolean => {
    void dirtyTick.value // 建立响应式依赖：定版推进后调用方重新求值
    const draft = drafts.value.get(id)
    if (!draft) return false
    const saved = savedIndex.value.get(id)
    // 无保存态的新建接口与创建快照比（无快照的野草稿保守按脏）
    const baseline = saved ?? pristineBaselines.get(id)
    if (!baseline) return true
    const cached = dirtyCache.get(id)
    if (cached && cached[0] === dirtyTick.value) return cached[1]
    const result = !eq(draft, baseline)
    dirtyCache.set(id, [dirtyTick.value, result])
    return result
  }

  const draftOf = (id: string): Endpoint | null => drafts.value.get(id) ?? null

  /** 「新建」选择页伪页签：无 draft、不持久化（快照/刷新校验均豁免），
   *  激活时内容区渲染新建选择页。每次「+」新开一枚（对标 Apifox 多枚「新建…」并存）。 */
  const isCreateTab = (id: string): boolean => id.startsWith('create:')

  function openCreateTab(): void {
    const id = `create:${crypto.randomUUID()}`
    openTabs.value.push(id)
    activeTabId.value = id
  }

  /** 标签页标题：新建伪页签 > 草稿名 > 保存名 > method+path。 */
  const titleOf = (id: string): string => {
    if (isCreateTab(id)) return t('tabbar.createTab')
    if (isScenarioTab(id)) {
      return (
        scenarioIndex.value.get(id.slice(SCENARIO_TAB_PREFIX.length))?.name ??
        t('tabbar.scenarioTab')
      )
    }
    const d = drafts.value.get(id)
    if (d?.name) return d.name
    if (d) return `${d.method} ${d.path}`
    const saved = savedIndex.value.get(id)
    if (saved) return saved.name || `${saved.method} ${saved.path}`
    return t('tabbar.unsaved')
  }

  /** 拉取文件夹 + 接口列表；knownProject 传入时跳过重复的 getActiveProject IPC。 */
  async function load(projectId: string, knownProject?: Project): Promise<void> {
    loadError.value = null
    try {
      const [p, f, e] = await Promise.all([
        knownProject ? Promise.resolve(knownProject) : api.getActiveProject(),
        api.listFolders(projectId),
        api.listEndpoints(projectId),
      ])
      project.value = p
      folders.value = f
      endpoints.value = e
    } catch (err) {
      loadError.value = err instanceof Error ? err.message : String(err)
      throw err
    }
  }

  /** 初始化：取激活项目（无则返回 null，调用方负责跳回项目列表）；同时恢复标签栏。 */
  async function init(): Promise<Project | null> {
    // 标签栏恢复与激活项目查询互不依赖，并行一轮往返；
    // 项目内数据（folders/endpoints）与环境/全局变量也无依赖，同样并行。
    const [, p] = await Promise.all([ensureOpenProjectsRestored(), api.getActiveProject()])
    if (!p) return null
    project.value = p
    // 冷启动：恢复该项目的页签 / 草稿现场（localStorage 持久化；快照机制
    // 只覆盖会话内切换，重启后走这里）。
    restoreTabsState(p.id)
    restoreSessionBaseUrl()
    await Promise.all([load(p.id, p), loadEnvironments(), loadScenarios(p.id)])
    return p
  }

  /** 刷新文件夹 + 接口（树操作后调用）。 */
  async function refresh(): Promise<void> {
    if (!project.value) return
    const [f, e] = await Promise.all([
      api.listFolders(project.value.id),
      api.listEndpoints(project.value.id),
    ])
    folders.value = f
    endpoints.value = e
  }

  /**
   * 同步拉取覆盖后全量重载：远端快照以远端 id 重建了全部子对象，
   * 需要重取项目本体 + 文件夹 + 接口 + 环境，并关闭已失效的标签
   * （远端 id 与本地不一致时标签指向的接口已不存在）。
   */
  async function reloadAfterOverwrite(): Promise<void> {
    if (!project.value) return
    const pid = project.value.id
    const [p, f, e] = await Promise.all([
      api.getActiveProject(),
      api.listFolders(pid),
      api.listEndpoints(pid),
    ])
    project.value = p
    folders.value = f
    endpoints.value = e
    const valid = new Set(e.map((x) => x.id))
    for (const id of [...openTabs.value]) {
      // 「新建」伪页签不对应接口，豁免校验（否则每次刷新都被清掉）
      if (!valid.has(id) && !isCreateTab(id)) closeTab(id)
    }
    await loadEnvironments()
    await loadScenarios(pid)
  }

  /**
   * 备份恢复后刷新：恢复会落库为「全新项目」（新 id），当前工作区需重载
   * 才能看到它，否则要重启应用。这里重取项目列表并入标签栏（新项目追加为
   * 一个可切换的标签，当前项目保持激活，不打断用户当前上下文），同时重载
   * 当前项目的文件夹 / 接口 / 环境——恢复的全局变量/参数是保守合并，可能
   * 落到当前项目已展示的数据上。
   */
  async function refreshAfterRestore(): Promise<void> {
    try {
      const all = await api.getProjects()
      const byId = new Map(all.map((p) => [p.id, p]))
      // 标签栏对齐真实项目集：剔除已被恢复删除/替换的旧 id，保留顺序
      const kept = openProjects.value.filter((t) => byId.has(t.id))
      const names = new Map(kept.map((t) => [t.id, byId.get(t.id)!.name]))
      openProjects.value = kept.map((t) => ({ id: t.id, name: names.get(t.id)! }))
      // 备份唤醒的其他项目按 updated_at 追加为标签，方便直接切换
      const seen = new Set(openProjects.value.map((t) => t.id))
      for (const p of [...all].sort((a, b) => b.updated_at.localeCompare(a.updated_at))) {
        if (!seen.has(p.id)) upsertOpenTab(p.id, p.name)
      }
      persistOpenProjects()
    } catch {
      // 项目列表拉取失败：保留现有标签，工作区仍尽力重载
    }
    await refresh()
  }

  /** 切换到另一项目：当前项目 UI 态入快照，目标项目恢复快照或全新加载。
   *  草稿 / 打开的标签页 / 环境选择等在项目间来回切换均不丢失。 */
  async function switchProject(projectId: string): Promise<void> {
    if (project.value?.id === projectId) return
    // 应用可能从项目列表页直接进入：确保持久化的标签已恢复再操作
    await ensureOpenProjectsRestored()
    if (project.value) putSnapshot(project.value.id, snapshotCurrent())
    const p = await api.setActiveProject(projectId)
    if (!p) {
      // 目标项目已不存在（他处删除）：移除标签后抛错
      snapshots.delete(projectId)
      removeOpenTab(projectId)
      throw new Error(t('ws.projectMissing'))
    }
    await activateProject(p)
  }

  // ---------- 多项目标签：快照切换 ----------
  /** 顶部标签栏的项目（有序）。 */
  const openProjects = ref<{ id: string; name: string }[]>([])
  /** 标签列表持久化键（仅存 id 顺序；草稿等 UI 态不跨重启保留）。 */
  const OPEN_TABS_KEY = 'miyo.open-projects'

  /** 非激活项目的完整 UI 快照。 */
  interface ProjectSnapshot {
    project: Project
    folders: Folder[]
    endpoints: Endpoint[]
    environments: Environment[]
    activeEnvId: string | null
    sessionBaseUrl: string
    openTabs: string[]
    drafts: Map<string, Endpoint>
    activeTabId: string | null
    activeView: 'debug' | 'design' | 'docs' | 'cases' | 'mock'
    examples: Map<string, ResponseExample[]>
    requestExamples: Map<string, RequestExample[]>
    testCases: Map<string, TestCase[]>
    histories: RequestHistory[]
    historyOnlyCurrent: boolean
  }
  /** 快照无需响应式：仅在切换瞬间读写。 */
  const snapshots = new Map<string, ProjectSnapshot>()
  /** 快照含 folders/endpoints/drafts 等整份项目态，多项目来回切会线性堆积：
   *  按「最近使用」留最近 3 个（够覆盖常见来回切），超出的丢最旧一份——
   *  被丢的项目下次切回走全新 load()，代价与首次打开一致，只是丢 UI 态。 */
  const SNAPSHOT_LIMIT = 3

  function putSnapshot(projectId: string, snap: ProjectSnapshot): void {
    snapshots.delete(projectId) // 重置插入序：Map 迭代序即「最旧在前」
    snapshots.set(projectId, snap)
    while (snapshots.size > SNAPSHOT_LIMIT) {
      const oldest = snapshots.keys().next().value
      if (oldest === undefined) break
      snapshots.delete(oldest)
    }
  }

  function snapshotCurrent(): ProjectSnapshot {
    return {
      project: project.value!,
      folders: folders.value,
      endpoints: endpoints.value,
      environments: environments.value,
      activeEnvId: activeEnvId.value,
      sessionBaseUrl: sessionBaseUrl.value,
      // 「新建」伪页签是临时 UI 态，不入快照（切项目回来不复活）
      openTabs: openTabs.value.filter((id) => !isCreateTab(id)),
      drafts: drafts.value,
      activeTabId:
        activeTabId.value && !isCreateTab(activeTabId.value) ? activeTabId.value : null,
      activeView: activeView.value,
      examples: examples.value,
      requestExamples: requestExamples.value,
      testCases: testCases.value,
      histories: histories.value,
      historyOnlyCurrent: historyOnlyCurrent.value,
    }
  }

  function persistOpenProjects(): void {
    try {
      localStorage.setItem(OPEN_TABS_KEY, JSON.stringify(openProjects.value.map((t) => t.id)))
    } catch {
      // 存储不可用时静默：仅影响重启后的标签恢复
    }
  }

  function removeOpenTab(id: string): void {
    const idx = openProjects.value.findIndex((t) => t.id === id)
    if (idx !== -1) openProjects.value.splice(idx, 1)
    persistOpenProjects()
  }

  function upsertOpenTab(id: string, name: string): void {
    const idx = openProjects.value.findIndex((t) => t.id === id)
    if (idx === -1) openProjects.value.push({ id, name })
    else openProjects.value[idx].name = name
    persistOpenProjects()
  }

  // 重命名当前项目时同步标签栏名称
  watch(
    () => [project.value?.id, project.value?.name] as const,
    ([id, name]) => {
      if (id && name) upsertOpenTab(id, name)
    },
  )

  /** 激活目标项目：有快照则恢复（数据不重新拉取），否则全新加载。 */
  async function activateProject(p: Project): Promise<void> {
    const snap = snapshots.get(p.id)
    if (snap) {
      snapshots.delete(p.id)
      project.value = snap.project
      folders.value = snap.folders
      endpoints.value = snap.endpoints
      environments.value = snap.environments
      activeEnvId.value = snap.activeEnvId
      sessionBaseUrl.value = snap.sessionBaseUrl
      // 兜底过滤：旧快照若混入「新建」伪页签，恢复时剥掉
      openTabs.value = snap.openTabs.filter((id) => !isCreateTab(id))
      drafts.value = snap.drafts
      activeTabId.value =
        snap.activeTabId && !isCreateTab(snap.activeTabId) ? snap.activeTabId : null
      activeView.value = snap.activeView
      examples.value = snap.examples
      requestExamples.value = snap.requestExamples
      testCases.value = snap.testCases
      histories.value = snap.histories
      historyOnlyCurrent.value = snap.historyOnlyCurrent
      loadError.value = null
      void loadScenarios(p.id)
    } else {
      project.value = p
      // 快照未命中（冷启动 / LRU 淘汰）：尝试从 localStorage 恢复页签现场。
      if (!restoreTabsState(p.id)) {
        openTabs.value = []
        drafts.value = new Map()
        activeTabId.value = null
      }
      examples.value = new Map()
      requestExamples.value = new Map()
      testCases.value = new Map()
      activeView.value = 'debug'
      restoreSessionBaseUrl()
      histories.value = []
      historyOnlyCurrent.value = false
      loadError.value = null
      await Promise.all([load(p.id, p), loadEnvironments(), loadScenarios(p.id)])
    }
    upsertOpenTab(p.id, p.name)
  }

  /** 关闭项目标签：丢弃快照；若是当前项目则切到相邻标签，一个不剩时清空（视图负责跳转）。 */
  function closeProjectTab(id: string): void {
    snapshots.delete(id)
    const idx = openProjects.value.findIndex((t) => t.id === id)
    if (idx !== -1) openProjects.value.splice(idx, 1)
    persistOpenProjects()
    if (project.value?.id !== id) return
    const next = openProjects.value[Math.min(idx, openProjects.value.length - 1)]
    if (next) {
      void switchProject(next.id).catch((err) => {
        toast.error(t('projectTabs.switchFail'), { message: err instanceof Error ? err.message : String(err) })
      })
    } else {
      project.value = null
    }
  }

  /** 启动时恢复持久化的标签列表（名称从项目列表回填，已删除的项目自动剔除）。
   *  与已有条目做并集合并：恢复是异步的，期间用户可能已打开新标签。 */
  async function restoreOpenProjects(): Promise<void> {
    let ids: unknown
    try {
      ids = JSON.parse(localStorage.getItem(OPEN_TABS_KEY) ?? '[]')
    } catch {
      ids = []
    }
    if (Array.isArray(ids) && ids.length) {
      try {
        const all = await api.getProjects()
        const byId = new Map(all.map((p) => [p.id, p]))
        const restored = ids
          .filter((id): id is string => typeof id === 'string' && byId.has(id))
          .map((id) => ({ id, name: byId.get(id)!.name }))
        const seen = new Set(restored.map((t) => t.id))
        // 恢复期间用户已打开的标签排在恢复列表之后，避免丢失
        for (const t of openProjects.value) {
          if (!seen.has(t.id)) restored.push(t)
        }
        openProjects.value = restored
      } catch {
        // 项目列表拉取失败：保留现有标签，下次再试
      }
    }
    persistOpenProjects()
  }

  /** 恢复只做一次（store 生命周期内）；switchProject / init 前调用以确保标签就绪。 */
  let restorePromise: Promise<void> | null = null
  function ensureOpenProjectsRestored(): Promise<void> {
    restorePromise ??= restoreOpenProjects()
    return restorePromise
  }
  // store 创建即触发：应用可能直接落在项目列表页（此时 WorkspaceView 尚未挂载）
  void ensureOpenProjectsRestored()

  // ---------- 页签 / 草稿跨重启恢复 + 最近访问 ----------
  // 项目级持久化（localStorage）：openTabs 顺序 + 草稿体 + 激活页签。
  // 草稿是打开时克隆的完整 Endpoint（自包含），重启后无保存态也可恢复编辑现场；
  // 超限时宁可放弃恢复也不写爆配额（2MB 上限，普通项目远达不到）。
  const TABS_STATE_PREFIX = 'miyo.open-tabs.'
  const TABS_STATE_MAX_BYTES = 2 * 1024 * 1024
  const RECENTS_KEY = 'miyo.recent-endpoints'
  const RECENTS_MAX = 10

  /** 最近打开的接口（跨项目全局，QuickSearch「最近」分组与排序用）。 */
  const recentEndpointIds = ref<string[]>([])
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(RECENTS_KEY) ?? '[]')
    if (Array.isArray(parsed)) {
      recentEndpointIds.value = parsed.filter((v): v is string => typeof v === 'string').slice(0, RECENTS_MAX)
    }
  } catch {
    // 损坏即重置
  }

  function recordRecentEndpoint(id: string): void {
    const next = [id, ...recentEndpointIds.value.filter((v) => v !== id)].slice(0, RECENTS_MAX)
    recentEndpointIds.value = next
    try {
      localStorage.setItem(RECENTS_KEY, JSON.stringify(next))
    } catch {
      // 配额满不影响主流程
    }
  }

  let tabsPersistTimer: ReturnType<typeof setTimeout> | undefined
  function persistTabsState(): void {
    const pid = project.value?.id
    if (!pid) return
    try {
      const draftsObj: Record<string, Endpoint> = {}
      for (const id of openTabs.value) {
        const d = drafts.value.get(id)
        if (d) draftsObj[id] = d
      }
      const payload = JSON.stringify({
        tabs: openTabs.value,
        drafts: draftsObj,
        activeTabId: activeTabId.value,
      })
      if (payload.length > TABS_STATE_MAX_BYTES) return
      localStorage.setItem(TABS_STATE_PREFIX + pid, payload)
    } catch {
      // 写失败（配额等）静默：恢复缺失只是回到旧默认（空页签）
    }
  }

  /** 草稿高频编辑 → 防抖落盘（页签结构变化由下方 watch 直接触发）。 */
  function schedulePersistTabs(): void {
    if (tabsPersistTimer) clearTimeout(tabsPersistTimer)
    tabsPersistTimer = setTimeout(persistTabsState, 600)
  }

  /** 冷启动 / 快照未命中时恢复该项目的页签现场；返回是否恢复成功。 */
  function restoreTabsState(pid: string): boolean {
    try {
      const raw = localStorage.getItem(TABS_STATE_PREFIX + pid)
      if (!raw) return false
      const parsed = JSON.parse(raw) as {
        tabs?: unknown
        drafts?: Record<string, Endpoint>
        activeTabId?: unknown
      }
      if (!Array.isArray(parsed.tabs)) return false
      const tabs: string[] = []
      const draftsMap = new Map<string, Endpoint>()
      for (const id of parsed.tabs) {
        if (typeof id !== 'string') continue
        // 伪页签自描述；接口页签以草稿体为存在依据（自包含，接口列表加载前后都可渲染）
        if (isCreateTab(id) || isScenarioTab(id)) {
          tabs.push(id)
          continue
        }
        const d = parsed.drafts?.[id]
        if (d && typeof d.id === 'string' && typeof d.request === 'object') {
          draftsMap.set(id, d)
          tabs.push(id)
        }
      }
      if (!tabs.length) return false
      openTabs.value = tabs
      drafts.value = draftsMap
      const active =
        typeof parsed.activeTabId === 'string' && tabs.includes(parsed.activeTabId)
          ? parsed.activeTabId
          : tabs[0]!
      activeTabId.value = active
      return true
    } catch {
      return false
    }
  }

  // 页签结构 / 激活态变化即落盘（覆盖打开 / 关闭 / 切换 / 场景页签等所有路径）。
  watch([openTabs, activeTabId], () => persistTabsState())
  // 草稿编辑：防抖合并（与上方 dirtyTick watch 同源触发，各自节流互不影响）。
  watch(
    drafts,
    () => schedulePersistTabs(),
    { deep: true },
  )

  /** 是否存在未保存草稿（退出确认兜底用）。 */
  function hasDirtyDrafts(): boolean {
    return openTabs.value.some((id) => isDirty(id))
  }

  /** 打开接口为标签页；已打开则仅切换。草稿懒克隆自保存态。 */
  function openEndpoint(endpoint: Endpoint): void {
    if (!openTabs.value.includes(endpoint.id)) {
      openTabs.value.push(endpoint.id)
      drafts.value.set(endpoint.id, { ...endpoint, request: deepClone(endpoint.request) })
    }
    activeTabId.value = endpoint.id
    recordRecentEndpoint(endpoint.id)
    // 缓存命中跳过：切 Tab 原来无条件触发三 IPC（示例/用例/测试用例各一次）。
    if (!examples.value.has(endpoint.id)) void loadExamples(endpoint.id)
    if (!requestExamples.value.has(endpoint.id)) void loadRequestExamples(endpoint.id)
    if (!testCases.value.has(endpoint.id)) void loadTestCases(endpoint.id)
  }

  /** 加载接口的响应示例（懒加载 + 缓存）。 */
  async function loadExamples(endpointId: string): Promise<void> {
    try {
      examples.value.set(endpointId, await api.listExamples(endpointId))
    } catch (err) {
      console.error('[workspace.loadExamples]', err)
      examples.value.set(endpointId, [])
    }
  }

  /** 把一次执行响应保存为示例（保存后刷新缓存）；草稿未落库时外键会拒写，先拦截。 */
  async function saveAsExample(
    endpointId: string,
    name: string,
    response: ExecuteResponse,
  ): Promise<void> {
    if (!endpoints.value.some((e) => e.id === endpointId)) {
      toast.warning(t('ws.saveEndpointFirst'))
      return
    }
    const now = new Date().toISOString()
    const example = await api.saveExample({
      id: crypto.randomUUID(),
      endpoint_id: endpointId,
      name,
      status: response.status,
      headers: Object.fromEntries(response.headers),
      body: response.body,
      content_type: response.content_type,
      created_at: now,
      updated_at: now,
    })
    const list = examples.value.get(endpointId) ?? []
    const idx = list.findIndex((x) => x.id === example.id)
    if (idx === -1) list.unshift(example)
    else list[idx] = example
    examples.value.set(endpointId, list)
    toast.success(t('ws.exampleSaved', { name: example.name }))
  }

  async function removeExample(endpointId: string, exampleId: string): Promise<void> {
    await api.deleteExample(exampleId)
    const list = (examples.value.get(endpointId) ?? []).filter((x) => x.id !== exampleId)
    examples.value.set(endpointId, list)
  }

  /** 请求用例（按接口 id 缓存，openEndpoint 时懒加载）。 */
  const requestExamples = ref<Map<string, RequestExample[]>>(new Map())

  /** 加载接口的请求用例（懒加载 + 缓存，失败时按空处理）。 */
  async function loadRequestExamples(endpointId: string): Promise<void> {
    try {
      requestExamples.value.set(endpointId, await api.listRequestExamples(endpointId))
    } catch (err) {
      console.error('[workspace.loadRequestExamples]', err)
      requestExamples.value.set(endpointId, [])
    }
  }

  /** 把当前请求保存为用例快照（request 深拷贝落库，保存后置顶缓存）。 */
  async function saveRequestAsExample(
    endpointId: string,
    name: string,
    request: Endpoint['request'],
  ): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) {
      toast.warning(t('ws.caseNameRequired'))
      return false
    }
    const now = new Date().toISOString()
    const example: RequestExample = {
      id: crypto.randomUUID(),
      endpoint_id: endpointId,
      name: trimmed,
      request: deepClone(request),
      created_at: now,
      updated_at: now,
    }
    try {
      const saved = await api.saveRequestExample(example)
      const list = requestExamples.value.get(endpointId) ?? []
      list.unshift(saved)
      requestExamples.value.set(endpointId, list)
      toast.success(t('ws.requestCaseSaved', { name: saved.name }))
      return true
    } catch (err) {
      toast.error(t('ws.requestCaseSaveFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 把请求用例回填到草稿（request 深拷贝 + 同步 active_tab 智能默认）。 */
  function applyRequestExample(endpointId: string, example: RequestExample): void {
    const draft = drafts.value.get(endpointId)
    if (!draft) return
    draft.request = deepClone(example.request)
  }

  async function deleteRequestExample(endpointId: string, exampleId: string): Promise<void> {
    try {
      await api.deleteRequestExample(exampleId)
    } catch (err) {
      toast.error(t('ws.requestCaseDeleteFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return
    }
    const list = (requestExamples.value.get(endpointId) ?? []).filter((x) => x.id !== exampleId)
    requestExamples.value.set(endpointId, list)
  }

  /** 接口页二级导航：调试 | 设计 | 文档预览 | 测试用例 | Mock。 */
  const activeView = ref<'debug' | 'design' | 'docs' | 'cases' | 'mock'>('debug')
  function setActiveView(view: 'debug' | 'design' | 'docs' | 'cases' | 'mock'): void {
    activeView.value = view
  }

  /** 测试用例（按接口 id 缓存，openEndpoint 时懒加载）。 */
  const testCases = ref<Map<string, TestCase[]>>(new Map())

  /** 最近一次运行结果元信息（状态码 / 耗时），驱动列表「运行结果」列与抽屉联动。 */
  const caseRunMeta = ref<Map<string, { status: number; durationMs: number }>>(new Map())

  /** 当前接口的用例数（导航 Tab 徽标 N）。 */
  const testCaseCount = computed(() => {
    const id = activeTabId.value
    return id ? (testCases.value.get(id)?.length ?? 0) : 0
  })

  /** 加载接口的测试用例（懒加载 + 缓存，失败时按空处理）。 */
  async function loadTestCases(endpointId: string): Promise<void> {
    try {
      testCases.value.set(endpointId, (await api.listTestCases(endpointId)) ?? [])
    } catch (err) {
      console.error('[workspace.loadTestCases]', err)
      testCases.value.set(endpointId, [])
    }
  }

  /** 保存测试用例（新建），返回是否成功。 */
  async function saveTestCase(
    endpointId: string,
    name: string,
    category: TestCaseCategory,
    request: Endpoint['request'],
    path: string,
    method: HttpMethod,
    folder = '',
  ): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) {
      toast.warning(t('ws.caseNameRequired'))
      return false
    }
    const snap = snapshotRequest(request)
    const testCase: TestCase = {
      id: crypto.randomUUID(),
      request_id: endpointId,
      name: trimmed,
      category,
      folder,
      method,
      url_path: path,
      path_variables: snap.path_variables,
      params: snap.params,
      headers: snap.headers,
      body_type: snap.body_type,
      body_content: snap.body_content,
      last_run_status: 'Untested',
      created_at: new Date().toISOString(),
    }
    try {
      const saved = await api.saveTestCase(testCase)
      const list = testCases.value.get(endpointId) ?? []
      list.push(saved)
      testCases.value.set(endpointId, list)
      toast.success(t('ws.testCaseSaved', { name: saved.name }))
      return true
    } catch (err) {
      toast.error(t('ws.testCaseSaveFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 更新用例名称与分组（编辑）。 */
  async function renameTestCase(
    endpointId: string,
    caseId: string,
    name: string,
    category: TestCaseCategory,
    folder = '',
  ): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) {
      toast.warning(t('ws.caseNameRequired'))
      return false
    }
    try {
      await api.updateTestCaseMeta(caseId, trimmed, category, folder)
      const list = testCases.value.get(endpointId)
      const target = list?.find((c) => c.id === caseId)
      if (target) {
        target.name = trimmed
        target.category = category
        target.folder = folder
      }
      return true
    } catch (err) {
      toast.error(t('ws.caseUpdateFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 批量移动用例到文件夹（空 = 移出分组）。 */
  async function setTestCaseFolder(endpointId: string, caseIds: string[], folder: string): Promise<boolean> {
    try {
      await api.setTestCaseFolder(caseIds, folder)
      const list = testCases.value.get(endpointId)
      if (list) {
        for (const c of list) {
          if (caseIds.includes(c.id)) c.folder = folder
        }
      }
      return true
    } catch (err) {
      toast.error(t('ws.caseUpdateFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 重命名用例文件夹（整组跟随；目标组已存在时合并）。 */
  async function renameTestCaseFolder(endpointId: string, oldFolder: string, newFolder: string): Promise<boolean> {
    const trimmed = newFolder.trim()
    if (!trimmed) {
      toast.warning(t('ws.caseNameRequired'))
      return false
    }
    try {
      await api.renameTestCaseFolder(endpointId, oldFolder, trimmed)
      const list = testCases.value.get(endpointId)
      if (list) {
        for (const c of list) {
          if (c.folder === oldFolder) c.folder = trimmed
        }
      }
      toast.success(t('cases.folderRenamed'))
      return true
    } catch (err) {
      toast.error(t('ws.caseUpdateFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 解散分组：组内用例移至未分组（不删用例）。 */
  async function dissolveTestCaseFolder(endpointId: string, folder: string): Promise<boolean> {
    const list = testCases.value.get(endpointId) ?? []
    const ids = list.filter((c) => c.folder === folder).map((c) => c.id)
    if (!ids.length) return true
    return setTestCaseFolder(endpointId, ids, '')
  }

  /** 克隆用例（另存为「名称 副本」，保留原快照）。 */
  async function cloneTestCase(endpointId: string, source: TestCase): Promise<boolean> {
    const testCase: TestCase = {
      ...deepClone(source),
      id: crypto.randomUUID(),
      name: t('examples.copyName', { name: source.name }),
      last_run_status: 'Untested',
      created_at: new Date().toISOString(),
    }
    try {
      const saved = await api.saveTestCase(testCase)
      const list = testCases.value.get(endpointId) ?? []
      list.push(saved)
      testCases.value.set(endpointId, list)
      return true
    } catch (err) {
      toast.error(t('ws.caseCloneFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return false
    }
  }

  /** 删除测试用例。 */
  async function removeTestCase(endpointId: string, caseId: string): Promise<void> {
    try {
      await api.deleteTestCase(caseId)
    } catch (err) {
      toast.error(t('ws.caseDeleteFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return
    }
    const list = (testCases.value.get(endpointId) ?? []).filter((c) => c.id !== caseId)
    testCases.value.set(endpointId, list)
  }

  /** 用例快照 → 回填草稿（method / path / params / headers / body），不切换视图。 */
  function applyTestCaseToDraft(endpointId: string, testCase: TestCase): void {
    const draft = drafts.value.get(endpointId)
    if (!draft) return
    draft.method = testCase.method
    draft.path = testCase.url_path
    applyCaseToRequest(draft.request, testCase)
    if (!draft.request.active_tab) {
      draft.request.active_tab = ['POST', 'PUT', 'PATCH'].includes(testCase.method) ? 'body' : 'params'
    }
  }

  /** 「在调试页打开」：回填草稿并显式切换到调试页。 */
  function openTestCaseInDebug(endpointId: string, testCase: TestCase): void {
    applyTestCaseToDraft(endpointId, testCase)
    activeView.value = 'debug'
  }

  /** 用例快照 → 可发送请求（URL + spec），单跑与集合跑共用。 */
  function buildCaseRequest(testCase: TestCase): { url: string; spec: Endpoint['request'] } {
    const isAbs = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(testCase.url_path)
    const path = testCase.url_path.startsWith('/') ? testCase.url_path : `/${testCase.url_path}`
    // 路径变量必须在拼 URL 时替换掉，否则 `/users/{id}` 会被原样发出去。
    const vars = testCase.path_variables ?? []
    return {
      url: isAbs
        ? applyPathVars(testCase.url_path, vars)
        : `${urlDomain.value}${applyPathVars(path, vars)}`,
      spec: {
        params: testCase.params,
        headers: testCase.headers,
        path_variables: vars,
        auth: { type: 'none' } as AuthSpec,
        body: restoreBody(testCase.body_type, testCase.body_content),
        active_tab: null,
        timeout_ms: null,
        follow_redirects: true,
        tests: null,
      },
    }
  }

  /** 变量合并表（单源 utils/environment.mergedVariableMap，与调试页 envVars 同源）。 */
  function caseVariableMap(): Record<string, string> {
    const env = environments.value.find((e) => e.id === activeEnvId.value) ?? null
    return mergedVariableMap({
      globalVariables: globalVariables.value,
      projectVariables: project.value?.variables,
      env,
    })
  }

  /**
   * 用例路径 → 可运行 URL（导出代码用，与调试页 buildUrl 同源）：
   * 完整地址直用（仅解析变量）；否则环境 Base URL 优先（resolveRequestUrl 解析
   * `{{变量}}` 后拼接），回退会话 Base URL。
   */
  function resolveTestCaseUrl(path: string): string {
    const trimmed = path.trim()
    if (!trimmed) return ''
    if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)) {
      return resolveVariables(trimmed, caseVariableMap())
    }
    const env = environments.value.find((e) => e.id === activeEnvId.value) ?? null
    if (env && envBaseUrl(env)) {
      return resolveRequestUrl(env, trimmed, caseVariableMap()).url
    }
    const rel = trimmed.startsWith('/') ? trimmed : `/${trimmed}`
    return `${urlDomain.value}${rel}`
  }

  /** 运行单个用例：拼 URL 执行请求，回写运行状态。返回响应或 null。 */
  async function runTestCase(endpointId: string, testCase: TestCase, environmentId: string | null): Promise<ExecuteResponse | null> {
    try {
      const { url, spec } = buildCaseRequest(testCase)
      const response = await api.executeRequest({
        url,
        method: testCase.method,
        spec,
        environment_id: environmentId,
        project_id: project.value?.id ?? null,
        endpoint_id: endpointId,
        request_id: null,
      })
      const status: TestCaseStatus = response.status >= 200 && response.status < 400 ? 'Success' : 'Failed'
      caseRunMeta.value.set(testCase.id, {
        status: response.status,
        durationMs: response.duration_ms,
      })
      void updateCaseStatusLocally(endpointId, testCase.id, status)
      return response
    } catch (err) {
      void updateCaseStatusLocally(endpointId, testCase.id, 'Failed')
      throw err
    }
  }

  /** 回写用例运行状态（本地 + 后端）。 */
  async function updateCaseStatusLocally(endpointId: string, caseId: string, status: TestCaseStatus): Promise<void> {
    const list = testCases.value.get(endpointId)
    const target = list?.find((c) => c.id === caseId)
    if (target) target.last_run_status = status
    try {
      await api.updateTestCaseStatus(caseId, status)
    } catch {
      /* 状态回写失败不影响主流程 */
    }
  }

  /** 抽屉「保存修改」：更新用例完整请求内容（本地 + 后端）。 */
  async function updateTestCaseContent(
    endpointId: string,
    caseId: string,
    payload: {
      method: HttpMethod
      urlPath: string
      pathVariables: KeyValue[]
      params: KeyValue[]
      headers: KeyValue[]
      bodyType: string
      bodyContent: string
    },
  ): Promise<void> {
    await api.updateTestCaseContent(caseId, payload)
    const list = testCases.value.get(endpointId)
    const target = list?.find((c) => c.id === caseId)
    if (target) {
      target.method = payload.method
      target.url_path = payload.urlPath
      target.path_variables = payload.pathVariables
      target.params = payload.params
      target.headers = payload.headers
      target.body_type = payload.bodyType
      target.body_content = payload.bodyContent
    }
  }

  /**
   * 运行接口的全部用例：一次 IPC 跑完整个集合（后端并发 5 + 可取消）。
   *
   * 原来前端串行 `for + await runTestCase`：N 个用例 = N 次 IPC ×（变量加载 +
   * 串行等待），且中途不可取消。结果与用例同序回填，可按序更新状态列。
   */
  async function runAllTestCases(
    endpointId: string,
    opts?: { runId?: string; folder?: string; onProgress?: (done: number, total: number) => void },
  ): Promise<{ total: number; success: number; cancelled: boolean }> {
    const all = testCases.value.get(endpointId) ?? []
    const cases = opts?.folder ? all.filter((c) => c.folder === opts.folder) : all
    if (cases.length === 0) return { total: 0, success: 0, cancelled: false }
    const endpoint = savedIndex.value.get(endpointId) ?? drafts.value.get(endpointId)
    const runId = opts?.runId ?? crypto.randomUUID()
    const result = await api.testCollection({
      items: cases.map((c) => {
        const { url, spec } = buildCaseRequest(c)
        return {
          endpoint: endpoint ?? {
            id: endpointId,
            project_id: project.value?.id ?? '',
            folder_id: null,
            name: c.name,
            method: c.method,
            path: c.url_path,
            description: '',
            status: 'designing',
            sort_order: 0,
            request: spec,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
          url,
          spec,
        }
      }),
      environment_id: activeEnvId.value,
      concurrency: 5,
      run_id: runId,
    })
    opts?.onProgress?.(result.results.length, cases.length)
    // 按序回填：results 与输入 cases 同序（取消时只含已完成项）。
    let success = 0
    result.results.forEach((r, i) => {
      const c = cases[i]
      if (!c) return
      const status: TestCaseStatus = r.ok ? 'Success' : 'Failed'
      if (r.ok) success += 1
      caseRunMeta.value.set(c.id, {
        status: r.status ?? 0,
        durationMs: r.duration_ms ?? 0,
      })
      void updateCaseStatusLocally(endpointId, c.id, status)
    })
    return { total: cases.length, success, cancelled: result.cancelled }
  }

  /** 取消在途的集合测试（runId 由调用方持有）。 */
  async function cancelAllTestCases(runId: string): Promise<boolean> {
    try {
      return await api.cancelTestCollection(runId)
    } catch {
      return false
    }
  }

  /** 生成「未命名接口」空草稿（未持久化，保存时生成落库）：只建草稿，不动页签。 */
  function createBlankDraft(folderId: string | null): string {
    const now = new Date().toISOString()
    const blank: Endpoint = {
      id: crypto.randomUUID(),
      project_id: project.value?.id ?? '',
      folder_id: folderId,
      name: t('default.endpointName'),
      method: 'GET',
      path: '/',
      description: '',
      status: 'designing',
      sort_order: 0,
      request: defaultRequestSpec(),
      created_at: now,
      updated_at: now,
    }
    drafts.value.set(blank.id, blank)
    pristineBaselines.set(blank.id, deepClone(blank))
    return blank.id
  }

  /** 打开「新建接口」草稿标签页；默认标题「未命名接口」，创建后自动聚焦全选便于输入。
   *  返回新草稿 id。 */
  function openNewEndpoint(folderId: string | null): string {
    const id = createBlankDraft(folderId)
    if (!openTabs.value.includes(id)) openTabs.value.push(id)
    activeTabId.value = id
    focusTitle()
    return id
  }

  /** 从「新建」伪页签发起新建请求：伪页签**原地替换**为新接口页签（当前页面被替换，
   *  而不是旁边多一枚、「新建」还留着——用户实测报错的交互）。
   *  激活态不是伪页签时退化为普通 openNewEndpoint（追加）。 */
  function openNewEndpointFromCreateTab(): void {
    const createId = activeTabId.value
    const endpointId = createBlankDraft(null)
    if (createId && isCreateTab(createId)) {
      const idx = openTabs.value.indexOf(createId)
      if (idx !== -1) {
        openTabs.value.splice(idx, 1, endpointId)
        activeTabId.value = endpointId
        focusTitle()
        return
      }
    }
    if (!openTabs.value.includes(endpointId)) openTabs.value.push(endpointId)
    activeTabId.value = endpointId
    focusTitle()
  }

  /** 标题聚焦信号：新建接口时自增，编辑器监听并聚焦全选标题（TabBar「+」/ ⌘T / 树内新建共用）。 */
  const focusTitleSignal = ref(0)
  function focusTitle(): void {
    focusTitleSignal.value += 1
  }

  function setDraft(endpoint: Endpoint): void {
    drafts.value.set(endpoint.id, { ...endpoint })
  }

  /** 页签拖拽排序回写：仅接受当前成员的完整排列（防拖拽期间开关页签竞态）。 */
  function reorderTabs(ordered: string[]): void {
    const current = openTabs.value
    if (ordered.length !== current.length) return
    const set = new Set(current)
    if (!ordered.every((id) => set.has(id))) return
    // 重复 id：长度相同但成员重复，同样拒绝（DOM 顺序异常时不污染数据）
    if (new Set(ordered).size !== ordered.length) return
    openTabs.value = ordered
  }

  function closeTab(id: string): void {
    const idx = openTabs.value.indexOf(id)
    if (idx === -1) return
    openTabs.value.splice(idx, 1)
    drafts.value.delete(id)
    pristineBaselines.delete(id)
    dirtyCache.delete(id)
    // 示例缓存随标签释放（每条含完整响应 body），重开标签时懒加载重建。
    examples.value.delete(id)
    requestExamples.value.delete(id)
    testCases.value.delete(id)
    caseRunMeta.value.delete(id)
    if (activeTabId.value === id) {
      activeTabId.value = openTabs.value[idx] ?? openTabs.value[idx - 1] ?? null
    }
  }

  /** 关闭除 keepId 外的全部标签（返回关闭数；脏标签由调用方先确认）。 */
  function closeOtherTabs(keepId: string): number {
    const targets = openTabs.value.filter((id) => id !== keepId)
    targets.forEach((id) => closeTab(id))
    return targets.length
  }

  /** 关闭全部标签（返回关闭数；脏标签由调用方先确认）。 */
  function closeAllTabs(): number {
    const targets = [...openTabs.value]
    targets.forEach((id) => closeTab(id))
    return targets.length
  }

  /** 保存当前草稿：新建（列表无此 id）走创建，否则走更新。 */
  async function saveActiveDraft(): Promise<boolean> {
    const draft = activeEndpoint.value
    if (!draft) return false
    if (!draft.name.trim()) {
      toast.warning(t('editor.nameRequired'))
      return false
    }
    const p = draft.path.trim()
    if (!p.startsWith('/') && !/^(https?|wss?):\/\//i.test(p)) {
      toast.warning(t('ws.pathMustStartWithSlash'))
      return false
    }
    try {
      const saved = await api.saveEndpoint(draft)
      const savedClone = deepClone(saved)
      const idx = endpoints.value.findIndex((e) => e.id === saved.id)
      if (idx === -1) {
        endpoints.value.push(savedClone)
      } else {
        endpoints.value[idx] = savedClone
      }
      drafts.value.set(saved.id, {
        ...saved,
        request: deepClone(saved.request),
      })
      // 落库后以保存态为准，创建快照使命完成
      pristineBaselines.delete(saved.id)
      toast.success(t('ws.endpointSaved', { name: saved.name }))
      return true
    } catch (err) {
      toast.error(t('ws.saveFail'), { message: err instanceof Error ? err.message : String(err) })
      return false
    }
  }

  /**
   * 删除撤销（单级）：删除前快照对象，Toast 提供「撤销」动作。
   * 文件夹含整棵子树（子文件夹 + 其下全部接口），撤销时按原 id 恢复。
   */
  interface DeletedSnapshot {
    kind: 'endpoint' | 'folder'
    label: string
    folders: Folder[]
    endpoints: Endpoint[]
  }
  const lastDeleted = ref<DeletedSnapshot | null>(null)

  /** 撤销上一次删除（快照整体回写 + 刷新列表）。 */
  async function undoDelete(): Promise<boolean> {
    const snap = lastDeleted.value
    if (!snap) return false
    lastDeleted.value = null
    try {
      for (const f of snap.folders) {
        await api.saveFolder({ ...f })
      }
      for (const e of snap.endpoints) {
        await api.saveEndpoint({ ...e })
      }
      await refresh()
      toast.success(t('ws.undoDeleteDone', { name: snap.label }))
      return true
    } catch (err) {
      toast.error(t('ws.undoDeleteFail'), { message: err instanceof Error ? err.message : String(err) })
      return false
    }
  }

  function collectSubtree(folderId: string): { folders: Folder[]; endpoints: Endpoint[] } {
    // 单遍建索引（按 id / 父子 / 直属文件夹分桶）：原来每往下一层都对全量
    // folders、endpoints 各 find/filter 一遍，深树下是 O(节点 × 全量)。
    const folderById = new Map<string, Folder>()
    const childrenOf = new Map<string, Folder[]>()
    for (const f of folders.value) {
      folderById.set(f.id, f)
      const key = f.parent_id ?? ''
      const bucket = childrenOf.get(key)
      if (bucket) bucket.push(f)
      else childrenOf.set(key, [f])
    }
    const epsOf = new Map<string, Endpoint[]>()
    for (const e of endpoints.value) {
      const key = e.folder_id ?? ''
      const bucket = epsOf.get(key)
      if (bucket) bucket.push(e)
      else epsOf.set(key, [e])
    }
    const outFolders: Folder[] = []
    const outEndpoints: Endpoint[] = []
    const walk = (fid: string): void => {
      const f = folderById.get(fid)
      if (f) outFolders.push({ ...f })
      for (const e of epsOf.get(fid) ?? []) outEndpoints.push({ ...e })
      for (const child of childrenOf.get(fid) ?? []) walk(child.id)
    }
    walk(folderId)
    return { folders: outFolders, endpoints: outEndpoints }
  }

  async function deleteEndpoint(endpointId: string): Promise<void> {
    const snapshot = endpoints.value.find((e) => e.id === endpointId)
    await api.deleteEndpoint(endpointId)
    closeTab(endpointId)
    await refresh()
    if (snapshot) {
      lastDeleted.value = {
        kind: 'endpoint',
        label: snapshot.name || snapshot.path,
        folders: [],
        endpoints: [{ ...snapshot }],
      }
      toast.info(t('ws.endpointDeleted', { name: snapshot.name || snapshot.path }), {
        duration: 8000,
        action: { label: t('ws.undo'), run: () => void undoDelete() },
      })
    } else {
      toast.info(t('ws.endpointDeletedPlain'))
    }
  }

  async function duplicateEndpoint(endpointId: string): Promise<void> {
    const dup = await api.duplicateEndpoint(endpointId)
    await refresh()
    openEndpoint(dup)
    toast.info(t('ws.duplicated', { name: dup.name }))
  }

  /** 加载当前项目的环境列表 + 激活环境 + 全局变量 + 全局参数。 */
  // ---------- 场景（集成链路测试） ----------
  /** 场景页签前缀（内容区据此渲染场景编排视图而非接口编辑器）。 */
  const SCENARIO_TAB_PREFIX = 'scenario:'

  const scenarios = ref<Scenario[]>([])
  const scenarioIndex = computed(
    () => new Map(scenarios.value.map((s) => [s.id, s] as const)),
  )

  function isScenarioTab(id: string): boolean {
    return id.startsWith(SCENARIO_TAB_PREFIX)
  }

  async function loadScenarios(pid: string): Promise<void> {
    try {
      scenarios.value = await api.listScenarios(pid)
    } catch {
      scenarios.value = []
    }
  }

  /** 打开场景编排页（伪页签：scenario:{id}，不持久化编辑态）。 */
  function openScenario(id: string): void {
    const tabId = `${SCENARIO_TAB_PREFIX}${id}`
    if (!openTabs.value.includes(tabId)) openTabs.value.push(tabId)
    activeTabId.value = tabId
  }

  /** 新建场景（默认名，落库后打开编排页）。 */
  async function createScenario(): Promise<Scenario | null> {
    if (!project.value) return null
    const now = new Date().toISOString()
    const scenario: Scenario = {
      id: crypto.randomUUID(),
      project_id: project.value.id,
      name: t('scenario.untitled'),
      description: '',
      sort_order: scenarios.value.length,
      steps: [],
      created_at: now,
      updated_at: now,
    }
    try {
      const saved = await api.saveScenario(scenario)
      scenarios.value.push(saved)
      return saved
    } catch (err) {
      toast.error(t('scenario.saveFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }

  /** 保存场景（整体覆盖；本地列表同步）。 */
  async function upsertScenario(scenario: Scenario): Promise<Scenario | null> {
    try {
      const saved = await api.saveScenario(scenario)
      const idx = scenarios.value.findIndex((s) => s.id === saved.id)
      if (idx === -1) scenarios.value.push(saved)
      else scenarios.value[idx] = saved
      return saved
    } catch (err) {
      toast.error(t('scenario.saveFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }

  /** 删除场景（挂着的编排页签一并关闭）。 */
  async function removeScenario(id: string): Promise<void> {
    try {
      await api.deleteScenario(id)
      scenarios.value = scenarios.value.filter((s) => s.id !== id)
      const tabId = `${SCENARIO_TAB_PREFIX}${id}`
      if (openTabs.value.includes(tabId)) closeTab(tabId)
      toast.success(t('scenario.deleted'))
    } catch (err) {
      toast.error(t('scenario.deleteFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }

  async function loadEnvironments(): Promise<void> {
    const pid = project.value?.id
    if (!pid) return
    const [envs, active, global, params] = await Promise.all([
      api.listEnvironments(pid),
      api.getActiveEnvironment(),
      api.getGlobalVariables(),
      api.getGlobalParams(),
    ])
    environments.value = envs
    activeEnvId.value = active?.id ?? null
    globalVariables.value = global
    globalParams.value = params
  }

  /** 保存全局变量并同步本地副本。 */
  async function saveGlobalVariables(variables: EnvironmentVariable[]): Promise<void> {
    await api.saveGlobalVariables(variables)
    globalVariables.value = variables
  }

  /** 保存全局参数并同步本地副本。 */
  async function saveGlobalParams(params: GlobalParam[]): Promise<void> {
    await api.saveGlobalParams(params)
    globalParams.value = params
  }

  /** 切换激活环境（null = 不使用环境）；环境须属于当前项目（后端校验）。 */
  async function setEnvironment(environmentId: string | null): Promise<void> {
    const env = await api.setActiveEnvironment(environmentId)
    activeEnvId.value = env?.id ?? null
  }

  /** 新建环境（项目维度：归属当前项目）。 */
  async function createEnvironment(name: string): Promise<void> {
    const now = new Date().toISOString()
    const env = await api.saveEnvironment({
      id: crypto.randomUUID(),
      project_id: project.value?.id ?? '',
      name,
      base_url: '',
      variables: [],
      created_at: now,
      updated_at: now,
    })
    environments.value.push(env)
    toast.success(t('ws.envCreated', { name: env.name }))
  }

  /**
   * 保存环境并做回显校验。
   * 后端若为旧版本（Environment 还没有 base_url 字段）会静默丢弃提交内容——
   * 保存"成功"却没落库（用户报过「输入 Base URL 点保存没效果」）。
   * 回显与提交不一致时抛错：调用方按失败处理，本地列表不同步。
   */
  async function saveEnvironmentVerified(payload: Environment): Promise<Environment> {
    const saved = await api.saveEnvironment({ ...payload, updated_at: new Date().toISOString() })
    if (payload.base_url && saved.base_url !== payload.base_url) {
      throw new Error(t('ws.envSaveEchoMismatch'))
    }
    return saved
  }

  /** 保存（upsert）环境并同步本地列表。 */
  async function updateEnvironment(
    env: Environment,
    opts?: { silent?: boolean },
  ): Promise<Environment> {
    const saved = await saveEnvironmentVerified(env)
    const idx = environments.value.findIndex((e) => e.id === saved.id)
    if (idx === -1) environments.value.push(saved)
    else environments.value[idx] = saved
    if (!opts?.silent) toast.success(t('ws.envSaved', { name: saved.name }))
    return saved
  }

  /**
   * 远程保存环境但不触碰本地列表：供「非当前工作区项目」的管理弹窗使用
   * （首页设置里跨项目管理环境），避免把其他项目的环境写进当前项目列表。
   * 回显校验与 updateEnvironment 相同。
   */
  async function saveEnvironmentRemote(env: Environment): Promise<Environment> {
    return saveEnvironmentVerified(env)
  }

  /** 删除环境；若删除的是当前激活环境则清空选中。silent 供调用方自带更具体提示时免双弹。 */
  async function deleteEnvironment(
    environmentId: string,
    opts?: { silent?: boolean },
  ): Promise<void> {
    await api.deleteEnvironment(environmentId)
    environments.value = environments.value.filter((e) => e.id !== environmentId)
    if (activeEnvId.value === environmentId) activeEnvId.value = null
    if (!opts?.silent) toast.success(t('ws.envDeleted'))
  }

  /**
   * 导入接口落地：按 folder_hint 复用/新建文件夹，保存接口并附带示例。
   *
   * 原来全串行（N 接口 ×（1 文件夹 + 1 接口 + E 示例）次 IPC 顺序等待）；
   * 现在限并发 4 的 worker 池并行落库，sort_order 预分配保证顺序语义，
   * 结束时一次 refresh 对齐列表（避免中途多次重渲染）。
   */
  /**
   * 批量导入接口（OpenAPI / Postman 解析产物）。
   *
   * `conflict` 决定与现有接口（同 method + path）相遇时的策略：
   * - `duplicate`（默认）：一律新建（旧行为）；
   * - `skip`：跳过冲突项；
   * - `overwrite`：沿用现有 id / 文件夹 / 排序 / 状态标记，覆盖名称与请求
   *   配置，不导入示例（避免示例重复堆积）——保持用户既有组织结构。
   */
  async function importEndpoints(
    items: Array<{ name: string; method: string; path: string; description?: string; request: Endpoint['request']; examples?: Array<{ name: string; status: number; content_type: string; headers: Record<string, string>; body: string }>; folder_hint?: string | null }>,
    conflict: 'skip' | 'overwrite' | 'duplicate' = 'duplicate',
  ): Promise<{ endpoints: number; examples: number; skipped: number; overwritten: number }> {
    if (!project.value) return { endpoints: 0, examples: 0, skipped: 0, overwritten: 0 }
    const projectId = project.value.id
    const now = new Date().toISOString()
    const baseSort = endpoints.value.length
    const baseFolderSort = folders.value.length
    // 冲突索引：同 method + path 视为同一接口（导入比对口径，方法统一大写）。
    const existingByKey = new Map<string, Endpoint>(
      endpoints.value.map((e) => [`${e.method.toUpperCase()} ${e.path}`, e] as const),
    )
    let skipped = 0
    let overwritten = 0
    // folder_hint → id（预扫本地 + 本次新建，避免并发重复建同名文件夹）。
    const folderCache = new Map<string, string>()
    for (const f of folders.value) folderCache.set(`\0${f.name}`, f.id)
    let folderSeq = 0
    const folderMutex = { locked: false }

    async function resolveFolder(hint: string | null | undefined): Promise<string | null> {
      const name = hint?.trim()
      if (!name) return null
      const key = `\0${name}`
      const hit = folderCache.get(key)
      if (hit) return hit
      // 建文件夹串行化（同名并发只建一次；文件夹数量少，串行无压力）。
      while (folderMutex.locked) await new Promise((r) => setTimeout(r, 5))
      folderMutex.locked = true
      try {
        const recheck = folderCache.get(key)
        if (recheck) return recheck
        const folder = await api.saveFolder({
          id: crypto.randomUUID(),
          project_id: projectId,
          parent_id: null,
          name,
          sort_order: baseFolderSort + folderSeq++,
          created_at: now,
          updated_at: now,
        })
        folderCache.set(key, folder.id)
        return folder.id
      } finally {
        folderMutex.locked = false
      }
    }

    let exampleCount = 0
    let createdCount = 0
    async function saveOne(item: (typeof items)[number], index: number): Promise<void> {
      const key = `${item.method.toUpperCase()} ${item.path}`
      const existing = existingByKey.get(key)
      if (existing && conflict === 'skip') {
        skipped++
        return
      }
      const overwriting = existing != null && conflict === 'overwrite'
      if (overwriting) overwritten++
      const folderId = await resolveFolder(item.folder_hint)
      const endpoint = await api.saveEndpoint({
        id: overwriting ? existing!.id : crypto.randomUUID(),
        project_id: projectId,
        // 覆盖保留用户既有归档：文件夹 / 排序 / 状态标记不因导入而丢。
        folder_id: overwriting ? existing!.folder_id : folderId,
        name: item.name,
        method: item.method as Endpoint['method'],
        path: item.path,
        description: item.description ?? '',
        request: item.request,
        sort_order: overwriting ? existing!.sort_order : baseSort + index,
        status: overwriting ? existing!.status : 'designing',
        created_at: existing?.created_at ?? now,
        updated_at: now,
      })
      if (!overwriting) {
        for (const ex of item.examples ?? []) {
          await api.saveExample({
            id: crypto.randomUUID(),
            endpoint_id: endpoint.id,
            name: ex.name,
            status: ex.status,
            headers: ex.headers ?? {},
            body: ex.body,
            content_type: ex.content_type,
            created_at: now,
            updated_at: now,
          })
          exampleCount++
        }
      }
      createdCount++
    }

    // 限并发 worker 池（4 路 IPC 并行，大导入不再线性等待）。
    const CONCURRENCY = 4
    let next = 0
    await Promise.all(
      Array.from({ length: Math.min(CONCURRENCY, items.length) }, async () => {
        for (;;) {
          const i = next++
          if (i >= items.length) return
          await saveOne(items[i], i)
        }
      }),
    )
    await refresh()
    return { endpoints: createdCount, examples: exampleCount, skipped, overwritten }
  }

  /**
   * 发送草稿请求（url 为拼接后的完整地址；环境变量由后端按 environment_id 注入）。
   * 提供 requestId 后该请求可被「取消」（后端中止连接并返回 CANCELLED）。
   *
   * 前端超时联动：后端超时是最后兜底；前端到时主动 cancel，避免弱网下
   * 请求挂起无反馈。超时未配置时不设 timer（行为与原来一致）。
   */
  async function send(
    endpoint: Endpoint,
    url: string,
    requestId?: string,
    timeoutMs?: number | null,
  ): Promise<ExecuteResponse> {
    let spec = endpoint.request
    const auth = spec.auth as AuthSpec | undefined
    if (auth?.type === 'oauth2' && (auth.auth_url?.trim() || auth.token_url?.trim())) {
      const token = await api.oauthAccessToken(auth)
      const base = (auth.token ?? {}) as Partial<Pick<OAuth2Token, 'token_type' | 'refresh_token' | 'expires_at'>>
      spec = { ...spec, auth: { ...auth, token: { ...base, access_token: token } as OAuth2Token } }
    }
    const rid = requestId ?? crypto.randomUUID()
    // 未定义变量汇总警示（非阻断）：{{name}} 不在环境/项目/全局与内置变量池中，
    // 后端将按字面量原样发出——「服务端收到 {{token}}」这类问题发送前暴露。
    const knownVars = new Set([...Object.keys(caseVariableMap()), ...BUILTIN_VARIABLES])
    const unknownVars = findUnknownVars(requestTemplateTexts(endpoint, url), knownVars)
    if (unknownVars.length) {
      toast.warning(t('ws.undefinedVars', { names: unknownVars.join(', ') }))
    }
    // 超时解析与后端 resolve_timeout_ms 保持一致：调用方传入 → 接口级 → 全局
    // （全局未配置时即内置默认 300s，见 refreshGlobalTimeoutMs）。原来只看接口级
    // 字段，导致「留空用全局」的接口在前端不设 timer，而旧接口若残留 30000 又会
    // 提前 30s 取消，与设置页展示的全局值不符。
    const ms = timeoutMs ?? endpoint.request.timeout_ms ?? globalTimeoutMs.value ?? FALLBACK_GLOBAL_TIMEOUT_MS
    let timer: ReturnType<typeof setTimeout> | undefined
    let timedOut = false
    if (ms && ms > 0) {
      timer = setTimeout(() => {
        timedOut = true
        void api.cancelRequest(rid)
      }, ms)
    }
    try {
      return await api.executeRequest({
        url,
        method: endpoint.method,
        spec,
        environment_id: activeEnvId.value,
        project_id: project.value?.id ?? null,
        endpoint_id: endpoint.id,
        request_id: rid,
      })
    } catch (err) {
      if (timedOut) {
        const e = err as Error & { code?: string }
        if (e?.code === 'CANCELLED') throw new Error(t('ws.requestTimeoutCancelled', { ms: String(ms) }))
      }
      throw err
    } finally {
      if (timer) clearTimeout(timer)
    }
  }

  // ---------- gRPC 调试 ----------

  const grpcStreams = ref<Map<string, GrpcStreamState>>(new Map())

  /** 单条流保留的消息上限（截断最旧；详情面板按 sequence 回落最新）。 */
  const MAX_STREAM_MESSAGES = 500

  /** 某接口当前（最近一次）的服务端流状态。 */
  function grpcStreamOf(endpointId: string): GrpcStreamState | null {
    for (const st of grpcStreams.value.values()) {
      if (st.endpointId === endpointId) return st
    }
    return null
  }

  /** miyo:grpc-event 单例绑定（store 生命周期 = 应用生命周期，无需解绑）。 */
  let grpcBound = false
  async function bindGrpcEvents(): Promise<void> {
    if (grpcBound) return
    grpcBound = true
    try {
      await listen<{ stream_id: string } & Record<string, unknown>>('miyo:grpc-event', (event) => {
        const payload = event.payload
        const st = grpcStreams.value.get(payload.stream_id)
        if (!st) return
        if (payload.kind === 'message') {
          st.messages.push(payload as unknown as GrpcMessageEvent)
          // 服务端流可能高频推送：与 RealtimeView 的 MAX_LOG 同口径截断最旧，
          // 防止长时间流式调用的内存 / DOM 无界增长
          if (st.messages.length > MAX_STREAM_MESSAGES) {
            st.messages.splice(0, st.messages.length - MAX_STREAM_MESSAGES)
          }
        } else if (payload.kind === 'end') {
          st.status = 'done'
          st.end = payload as unknown as GrpcEndEvent
        } else if (payload.kind === 'failed') {
          st.status = 'failed'
          st.error = String(payload.message ?? '')
        }
      })
    } catch {
      // 事件通道不可用（如测试环境）：流式消息收不到，不影响 unary
      grpcBound = false
    }
  }

  /**
   * 发送 gRPC 调用：unary 直返响应；服务端流注册运行态并返回 stream_id。
   * 地址 / 服务 / 方法 / 消息 / 元数据均为模板原文，由后端按环境统一渲染。
   */
  async function sendGrpc(endpoint: Endpoint, requestId?: string): Promise<GrpcInvokeResult> {
    const spec = endpoint.request.body.mode === 'grpc' ? endpoint.request.body.spec : null
    if (!spec) throw new Error(t('grpc.specMissing'))
    const rid = requestId ?? crypto.randomUUID()
    void bindGrpcEvents()
    const result = await api.grpcInvoke({
      address: endpoint.path,
      service: spec.service,
      method: spec.method,
      message: spec.message,
      metadata: endpoint.request.headers,
      use_tls: spec.use_tls,
      proto_ids: spec.proto_ids,
      project_id: project.value?.id ?? null,
      timeout_ms: endpoint.request.timeout_ms,
      environment_id: activeEnvId.value,
      endpoint_project_id: project.value?.id ?? null,
      endpoint_id: endpoint.id,
      request_id: rid,
    })
    if (result.kind === 'stream') {
      const st: GrpcStreamState = {
        streamId: result.stream_id,
        endpointId: endpoint.id,
        service: spec.service,
        method: spec.method,
        messages: [],
        status: 'running',
        end: null,
        error: null,
      }
      grpcStreams.value.set(result.stream_id, st)
    }
    return result
  }

  /** 关闭服务端流（后端触发取消令牌 + 中止消费任务）。 */
  async function closeGrpcStream(streamId: string): Promise<void> {
    await api.grpcStreamClose(streamId)
  }

  /** 流结束后清理运行态（消息与结束事件保留在 UI 快照里则由编辑器持有）。 */
  function dropGrpcStream(streamId: string): void {
    grpcStreams.value.delete(streamId)
  }

  /** 树内重命名接口：保存 + 同步列表与打开中的草稿。 */
  async function renameEndpoint(endpointId: string, name: string): Promise<void> {
    const e = endpoints.value.find((x) => x.id === endpointId)
    if (!e) return
    const saved = await api.saveEndpoint({ ...e, name, updated_at: new Date().toISOString() })
    const idx = endpoints.value.findIndex((x) => x.id === endpointId)
    if (idx !== -1) endpoints.value[idx] = saved
    const draft = drafts.value.get(endpointId)
    if (draft) drafts.value.set(endpointId, { ...draft, name })
  }

  /** 将文件夹移动到 newParentId（null=根）的 targetIndex 处：重排相关兄弟组并落库。 */
  async function moveFolder(folderId: string, newParentId: string | null, targetIndex: number): Promise<void> {
    const moved = folders.value.find((f) => f.id === folderId)
    if (!moved) return
    // 防环：不允许把文件夹移入自身或其子孙（避免 parent_id 成环）
    if (wouldCreateCycle(folders.value, folderId, newParentId)) return
    let changed: Map<string, number>
    if (moved.parent_id === newParentId) {
      changed = planSameGroupMove(
        folders.value.filter((f) => f.parent_id === newParentId),
        folderId,
        targetIndex,
      )
    } else {
      changed = planCrossGroupMove(
        folders.value.filter((f) => f.parent_id === moved.parent_id),
        folders.value.filter((f) => f.parent_id === newParentId),
        folderId,
        targetIndex,
      )
      moved.parent_id = newParentId
    }
    if (changed.has(folderId)) moved.sort_order = changed.get(folderId)!
    await Promise.all(
      [...changed.keys()].map((id) => {
        const f = folders.value.find((x) => x.id === id)
        if (!f) return Promise.resolve()
        if (id === folderId) return api.saveFolder({ ...f })
        return api.saveFolder({ ...f, sort_order: changed.get(id)! })
      }),
    )
    await refresh()
  }

  /** 将接口移动到 folderId（null=根）的 targetIndex 处：重排相关兄弟组并落库。 */
  async function moveEndpoint(endpointId: string, newFolderId: string | null, targetIndex: number): Promise<void> {
    const moved = endpoints.value.find((e) => e.id === endpointId)
    if (!moved) return
    let changed: Map<string, number>
    if (moved.folder_id === newFolderId) {
      changed = planSameGroupMove(
        endpoints.value.filter((e) => e.folder_id === newFolderId),
        endpointId,
        targetIndex,
      )
    } else {
      changed = planCrossGroupMove(
        endpoints.value.filter((e) => e.folder_id === moved.folder_id),
        endpoints.value.filter((e) => e.folder_id === newFolderId),
        endpointId,
        targetIndex,
      )
      moved.folder_id = newFolderId
    }
    if (changed.has(endpointId)) moved.sort_order = changed.get(endpointId)!
    // 同步打开中的草稿（文件夹归属 / 顺序）：draft 是 openEndpoint 时的快照，
    // 若不更新，保存草稿（saveActiveDraft 写全量）会用旧 folder_id 覆盖移动，
    // 造成「移动到 B 文件夹，保存后又回到 A」。
    for (const [id, order] of changed) {
      const draft = drafts.value.get(id)
      if (!draft) continue
      draft.sort_order = order
      if (id === endpointId) draft.folder_id = moved.folder_id
    }
    await Promise.all(
      [...changed.keys()].map((id) => {
        const e = endpoints.value.find((x) => x.id === id)
        if (!e) return Promise.resolve()
        if (id === endpointId) return api.saveEndpoint({ ...e })
        return api.saveEndpoint({ ...e, sort_order: changed.get(id)! })
      }),
    )
    await refresh()
  }

  async function saveFolder(folder: FolderInput): Promise<void> {
    await api.saveFolder(folder)
    await refresh()
  }

  /**
   * 批量删除：接口 + 文件夹混合选中。
   * 归一化：选中文件夹子树内的接口/子文件夹不再单独处理（随文件夹级联）；
   * 快照整体可经 `undoDelete` 一次撤销。
   */
  async function batchDelete(epIds: string[], folderIds: string[]): Promise<void> {
    const folderById = new Map(folders.value.map((f) => [f.id, f]))
    const selectedFolders = new Set(folderIds.filter((id) => folderById.has(id)))
    // 去掉被祖先选中的文件夹（随祖先级联删除）。
    const isUnderSelected = (id: string): boolean => {
      let cur = folderById.get(id)?.parent_id ?? null
      while (cur) {
        if (selectedFolders.has(cur)) return true
        cur = folderById.get(cur)?.parent_id ?? null
      }
      return false
    }
    const topFolders = [...selectedFolders].filter((id) => !isUnderSelected(id))
    const topSet = new Set(topFolders)
    // 文件夹子树覆盖的接口（随文件夹级联，不再单独删除）。
    const coveredEps = new Set<string>()
    for (const e of endpoints.value) {
      let cur = e.folder_id
      while (cur) {
        if (topSet.has(cur)) {
          coveredEps.add(e.id)
          break
        }
        cur = folderById.get(cur)?.parent_id ?? null
      }
    }
    // 接口索引一次：直选接口的「存在 + 取对象」原来是 O(所选 × 全量) 的 some/find
    const epIndex = new Map(endpoints.value.map((e) => [e.id, e]))
    const directEps = epIds.filter((id) => !coveredEps.has(id) && epIndex.has(id))
    if (!topFolders.length && !directEps.length) return

    // 子树只收集一次（原来 folders / endpoints 两条 flatMap 各自把整棵子树走一遍）
    const subtrees = topFolders.map((id) => collectSubtree(id))
    const snapFolders = subtrees.flatMap((sub) => sub.folders)
    const snapEndpoints = [
      ...subtrees.flatMap((sub) => sub.endpoints),
      ...directEps.flatMap((id) => {
        const e = epIndex.get(id)
        return e ? [{ ...e }] : []
      }),
    ]
    // 要关标签的接口就是子树覆盖集（coveredEps 单遍已算出）：原来是
    // 「每个选中文件夹 × 全量接口 × 祖先链」再走一遍的 O(文件夹 × 接口 × 深度)。
    for (const id of coveredEps) closeTab(id)
    for (const id of topFolders) await api.deleteFolder(id)
    for (const id of directEps) {
      await api.deleteEndpoint(id)
      closeTab(id)
    }
    await refresh()
    const total = snapFolders.length + snapEndpoints.length
    lastDeleted.value = {
      kind: topFolders.length ? 'folder' : 'endpoint',
      label: t('jsonTree.itemCount', { n: total }),
      folders: snapFolders,
      endpoints: snapEndpoints,
    }
    toast.info(t('ws.deletedCount', { n: total }), {
      duration: 8000,
      action: { label: t('ws.undo'), run: () => void undoDelete() },
    })
  }

  /** 批量移动接口到目标文件夹（末尾追加，单次刷新）。 */
  async function batchMoveEndpoints(ids: string[], folderId: string | null): Promise<void> {
    const list = endpoints.value.filter((e) => ids.includes(e.id))
    if (!list.length) return
    const siblings = endpoints.value.filter((e) => e.folder_id === folderId)
    let order = siblings.reduce((m, e) => Math.max(m, e.sort_order), -1) + 1
    await Promise.all(
      list.map((e) => api.saveEndpoint({ ...e, folder_id: folderId, sort_order: order++ })),
    )
    await refresh()
    toast.success(t('ws.movedCount', { n: list.length }))
  }

  async function deleteFolder(folderId: string): Promise<void> {
    const folder = folders.value.find((f) => f.id === folderId)
    const snap = collectSubtree(folderId)
    for (const e of snap.endpoints) closeTab(e.id)
    await api.deleteFolder(folderId)
    await refresh()
    if (folder && (snap.folders.length || snap.endpoints.length)) {
      lastDeleted.value = {
        kind: 'folder',
        label: folder.name,
        folders: snap.folders,
        endpoints: snap.endpoints,
      }
      toast.info(t('ws.folderDeleted', { name: folder.name, n: snap.endpoints.length }), {
        duration: 8000,
        action: { label: t('ws.undo'), run: () => void undoDelete() },
      })
    } else {
      toast.info(t('ws.folderDeletedPlain'))
    }
  }

  /**
   * cURL 导入：打开为默认标题「未命名接口」的草稿（不落库），保存时生成 id。
   * 展示前缀为环境变量时（环境优先级更高，会话值不可见）origin+path 整条存入
   * path——发送走 `isAbsolutePath` 分支直达导入主机，且**不写任何前缀**：既不覆写
   * 共享环境的 base_url，也不预填会话 Base URL（写入被环境前缀遮蔽时对用户不可见，
   * 事后切到「无环境」会冒出一个从没设过的域名，实证困惑）。只有展示前缀就是
   * 会话 Base URL 时才预填 origin。
   */
  function openCurlDraft(parsed: CurlParsed, folderId: string | null): void {
    const { path, params, origin } = splitUrl(parsed.url)
    const envPrefixed = urlDomain.value.startsWith('{{')
    const now = new Date().toISOString()
    const blank: Endpoint = {
      id: crypto.randomUUID(),
      project_id: project.value?.id ?? '',
      folder_id: folderId,
      name: t('default.endpointName'),
      method: parsed.method,
      path: envPrefixed ? `${origin}${path}` : path,
      description: '',
      status: 'designing',
      sort_order: 0,
      request: {
        params,
        headers: parsed.headers,
        path_variables: [],
        auth: parsed.auth,
        body: parsed.body ?? { mode: 'none' },
        timeout_ms: null,
        follow_redirects: true,
        tests: null,
      },
      created_at: now,
      updated_at: now,
    }
    if (!envPrefixed) sessionBaseUrl.value = origin
    drafts.value.set(blank.id, blank)
    if (!openTabs.value.includes(blank.id)) openTabs.value.push(blank.id)
    activeTabId.value = blank.id
  }

  /**
   * 地址栏粘贴 cURL：把解析结果回填到已有草稿（原地覆盖 method / path /
   * params / headers / auth / body）。origin 同 openCurlDraft 规则：展示前缀为
   * 环境变量时 origin+path 整条存入 path（绝不覆写共享环境 base_url，也不预填
   * 会话 Base URL），否则 origin 写入会话 Base URL、path 保持相对。
   */
  function applyCurlToDraft(draftId: string, parsed: CurlParsed): void {
    const d = drafts.value.get(draftId)
    if (!d) return
    const { path, params, origin } = splitUrl(parsed.url)
    const envPrefixed = urlDomain.value.startsWith('{{')
    d.method = parsed.method
    d.path = envPrefixed ? `${origin}${path}` : path
    d.request.params = params
    d.request.headers = parsed.headers
    d.request.auth = parsed.auth
    d.request.body = parsed.body ?? { mode: 'none' }
    if (!envPrefixed) sessionBaseUrl.value = origin
  }

  // ---------- 请求历史（侧栏「请求历史」页签；发送成功后由编辑器触发刷新） ----------
  const histories = ref<RequestHistory[]>([])
  /** 「仅当前接口」过滤（HistoryPanel 复选框；变更后需重新 loadHistories）。 */
  const historyOnlyCurrent = ref(false)

  /** 首屏条数 / 每次「加载更多」增量（后端 list_request_histories 只有 limit 无 offset）。 */
  const HISTORY_PAGE_SIZE = 50
  /** 本次会话请求的上限（loadMore 累加），按 (project, endpoint) 过滤条件分桶。 */
  const historyLimit = ref(HISTORY_PAGE_SIZE)
  /** 是否还有未加载的历史（返回条数达到当前 limit 视为可能还有）。 */
  const historyHasMore = ref(false)
  /** 最近一次加载失败：只提示一次，成功后复位（避免刷新/加载更多时重复刷屏）。 */
  const historyLoadFailed = ref(false)
  /** 当前历史查询对应的 (project, endpoint) —— 条件变化即重置分页窗口。 */
  const historyQueryKey = ref('')

  /**
   * 拉取历史。后端无 offset：按「增量 limit」分页——首次 50 条，
   * loadMoreHistories 每次 +50 重拉（项目保留上限 500 条，见后端 HISTORY_RETENTION_PER_PROJECT）。
   * 查询条件（项目 / 仅当前接口）变化时自动把窗口重置回首屏，调用方无需传参。
   */
  async function loadHistories(): Promise<void> {
    if (!project.value) return
    const endpointId = historyOnlyCurrent.value ? activeEndpoint.value?.id ?? null : null
    const key = `${project.value.id}|${endpointId ?? ''}`
    if (historyQueryKey.value !== key) {
      historyQueryKey.value = key
      historyLimit.value = HISTORY_PAGE_SIZE
    }
    try {
      const list = (await api.listRequestHistories(project.value.id, historyLimit.value, endpointId)) ?? []
      histories.value = list
      historyHasMore.value = list.length >= historyLimit.value
      historyLoadFailed.value = false
    } catch (err) {
      // 历史为辅助数据，但失败完全静默会让用户以为「没有记录」——提示一次即可
      if (!historyLoadFailed.value) {
        historyLoadFailed.value = true
        toast.error(t('ws.historyLoadFail'), {
          message: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  /** 「加载更多」：窗口 +1 页并重拉（返回条数仍达 limit 则继续可加载）。 */
  async function loadMoreHistories(): Promise<void> {
    if (!historyHasMore.value) return
    historyLimit.value += HISTORY_PAGE_SIZE
    await loadHistories()
  }

  /** 删除单条历史（行内删除；从已加载列表就地移除，分页窗口不重拉）。 */
  async function deleteHistory(id: string): Promise<void> {
    try {
      await api.deleteRequestHistory(id)
      histories.value = histories.value.filter((h) => h.id !== id)
    } catch (err) {
      toast.error(t('ws.historyDeleteFail'), { message: err instanceof Error ? err.message : String(err) })
    }
  }

  async function clearHistories(): Promise<void> {
    if (!project.value) return
    const endpointId = historyOnlyCurrent.value ? activeEndpoint.value?.id ?? null : null
    try {
      const removed = await api.clearRequestHistories(project.value.id, endpointId)
      histories.value = []
      historyHasMore.value = false
      toast.success(t('ws.historyCleared', { n: removed }))
    } catch (err) {
      toast.error(t('ws.historyClearFail'), {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }

  /** 历史摘要（request_summary_json）中可恢复的字段。 */
  interface HistorySummary {
    method?: HttpMethod
    url?: string
    spec?: Partial<Endpoint['request']>
  }

  /**
   * 点击历史记录 → 恢复到主编辑器。
   * - 归属接口存在：打开其草稿标签页并回填；不存在（临时请求）：新建「未命名接口」草稿；
   * - 回填 method / path / params / headers / body（摘要值为变量渲染后的实际发送值）；
   * - 认证保留接口自身配置（摘要入库时后端已置空）；URL origin 预填会话 Base URL；
   *   展示前缀为环境变量时 origin+path 整条存入 path（重发直达原主机，不覆写环境）。
   */
  function restoreFromHistory(h: RequestHistory): void {
    let summary: HistorySummary = {}
    try {
      summary = JSON.parse(h.request_summary_json) as HistorySummary
    } catch {
      // 旧版记录只有 method/url，按降级路径恢复
    }

    // gRPC 记录：url 字段即地址模板（无 scheme，不走 splitUrl/Base URL 语义），
    // 配置在 spec.grpc（service/method/message/use_tls/metadata/timeout_ms）。
    if ((summary.method ?? h.method) === 'GRPC') {
      const g = (summary.spec as { grpc?: { service?: string; method?: string; message?: string; use_tls?: boolean; metadata?: [string, string][]; timeout_ms?: number | null } } | undefined)?.grpc
      const target = h.endpoint_id ? endpoints.value.find((e) => e.id === h.endpoint_id) : null
      let id: string
      if (target) {
        openEndpoint(target)
        id = target.id
      } else {
        id = crypto.randomUUID()
        const now = new Date().toISOString()
        drafts.value.set(id, {
          id,
          project_id: project.value?.id ?? '',
          folder_id: null,
          name: t('default.endpointName'),
          method: 'GRPC',
          path: summary.url ?? h.url,
          description: '',
          status: 'designing',
          sort_order: 0,
          request: defaultRequestSpec(),
          created_at: now,
          updated_at: now,
        })
        if (!openTabs.value.includes(id)) openTabs.value.push(id)
        activeTabId.value = id
      }
      const draft = drafts.value.get(id)
      if (!draft) return
      draft.method = 'GRPC'
      draft.path = summary.url ?? h.url
      draft.request.body = {
        mode: 'grpc',
        spec: {
          service: g?.service ?? '',
          method: g?.method ?? '',
          message: g?.message ?? '{}',
          use_tls: g?.use_tls ?? false,
          proto_ids: [],
        },
      }
      draft.request.headers = (g?.metadata ?? []).map(([key, value]) => ({
        key,
        value,
        enabled: true,
        description: '',
      }))
      draft.request.timeout_ms = g?.timeout_ms ?? null
      draft.request.active_tab = 'grpc'
      toast.info(t('ws.restoredToEditor'))
      return
    }

    const url = summary.url ?? h.url
    const { path, params, origin } = splitUrl(url)
    const envPrefixed = urlDomain.value.startsWith('{{')
    const restoredPath = envPrefixed ? `${origin}${path}` : path

    const target = h.endpoint_id ? endpoints.value.find((e) => e.id === h.endpoint_id) : null
    let id: string
    if (target) {
      openEndpoint(target)
      id = target.id
    } else {
      id = crypto.randomUUID()
      const now = new Date().toISOString()
      drafts.value.set(id, {
        id,
        project_id: project.value?.id ?? '',
        folder_id: null,
        name: t('default.endpointName'),
        method: (summary.method ?? h.method) as HttpMethod,
        path: restoredPath,
        description: '',
        status: 'designing',
        sort_order: 0,
        request: defaultRequestSpec(),
        created_at: now,
        updated_at: now,
      })
      if (!openTabs.value.includes(id)) openTabs.value.push(id)
      activeTabId.value = id
    }
    const draft = drafts.value.get(id)
    if (!draft) return

    draft.method = (summary.method ?? h.method) as HttpMethod
    draft.path = restoredPath
    // 同 openCurlDraft：展示前缀为环境变量时 origin 已整条进 path，不预填会话前缀
    if (origin && !envPrefixed) sessionBaseUrl.value = origin
    const spec = summary.spec
    draft.request.params = spec?.params?.length ? spec.params : params
    if (spec?.headers) draft.request.headers = spec.headers
    if (spec?.body) draft.request.body = spec.body
    if (spec?.path_variables?.length) draft.request.path_variables = spec.path_variables
    toast.info(t('ws.restoredToEditor'))
  }

  return {
    project,
    folders,
    endpoints,
    environments,
    activeEnvId,
    globalVariables,
    globalParams,
    sessionBaseUrl,
    resetSessionBaseUrl,
    urlDomain,
    grpcStreams,
    grpcStreamOf,
    sendGrpc,
    closeGrpcStream,
    dropGrpcStream,
    setEnvironmentBaseUrl,
    loadError,
    openTabs,
    activeTabId,
    activeEndpoint,
    isDirty,
    hasDirtyDrafts,
    recentEndpointIds,
    draftOf,
    titleOf,
    init,
    load,
    refresh,
    reloadAfterOverwrite,
    switchProject,
    openProjects,
    closeProjectTab,
    openEndpoint,
    openNewEndpoint,
    openNewEndpointFromCreateTab,
    openCreateTab,
    reorderTabs,
    isCreateTab,
    focusTitleSignal,
    setDraft,
    closeTab,
    closeOtherTabs,
    closeAllTabs,
    saveActiveDraft,
    deleteEndpoint,
    undoDelete,
    batchDelete,
    batchMoveEndpoints,
    duplicateEndpoint,
    renameEndpoint,
    moveFolder,
    moveEndpoint,
    saveFolder,
    deleteFolder,
    openCurlDraft,
    applyCurlToDraft,
    importEndpoints,
    refreshAfterRestore,
    send,
    loadEnvironments,
    loadScenarios,
    scenarios,
    scenarioIndex,
    isScenarioTab,
    openScenario,
    createScenario,
    upsertScenario,
    removeScenario,
    setEnvironment,
    createEnvironment,
    updateEnvironment,
    saveEnvironmentRemote,
    deleteEnvironment,
    saveGlobalVariables,
    saveGlobalParams,
    examples,
    loadExamples,
    saveAsExample,
    removeExample,
    requestExamples,
    loadRequestExamples,
    saveRequestAsExample,
    applyRequestExample,
    deleteRequestExample,
    activeView,
    setActiveView,
    testCases,
    caseRunMeta,
    testCaseCount,
    loadTestCases,
    saveTestCase,
    renameTestCase,
    setTestCaseFolder,
    renameTestCaseFolder,
    dissolveTestCaseFolder,
    cloneTestCase,
    removeTestCase,
    applyTestCaseToDraft,
    openTestCaseInDebug,
    updateTestCaseContent,
    resolveTestCaseUrl,
    globalTimeoutMs,
    refreshGlobalTimeoutMs,
    runTestCase,
    runAllTestCases,
    cancelAllTestCases,
    histories,
    historyOnlyCurrent,
    historyHasMore,
    loadHistories,
    loadMoreHistories,
    clearHistories,
    deleteHistory,
    restoreFromHistory,
  }
})

import type { Folder as FolderInput } from '../types/miyoApi'
