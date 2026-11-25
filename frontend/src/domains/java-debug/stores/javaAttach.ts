/**
 * Java Debug 2.0 Attach store：本地 JVM 进程发现 → Attach → 类 / Bean / 方法
 * 发现 → 受控调用。与 v1 runtime store（javaDebug.ts）并行存在——JavaDebugView
 * 按 mode 切换两种工作台；历史共用 java_invocations（attach 会话有对应
 * java_projects 行，projectId 按 attach_pid 解析）。
 *
 * 归一策略：Agent 协议数据在 store 内映射为 v1 形状（JavaClassInfo /
 * JavaMethodInfo / JavaInvokeResponse），JavaClassBrowser / JavaMethodPanel /
 * JavaResultPanel / JavaHistoryDrawer 全部原样复用——差异点（风险分级、
 * 实例解析、consent）以独立状态暴露给视图层。
 */
import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import { defaultArgFor, mergeArgs } from '@/domains/java-debug/utils/argsSkeleton'
import type {
  JavaAgentClassItem,
  JavaAgentInvokeResponse,
  JavaAgentMethodInfo,
  JavaAttachSessionSnapshot,
  JavaBeanInfo,
  JavaClassCategory,
  JavaClassInfo,
  JavaClassStats,
  JavaMethodInfo,
  JavaProcessInfo,
  JavaProcessListSummary,
  JavaSpringStatus,
} from '@/types/miyoApi'

/** attach 方法 → v1 方法形状（JavaMethodPanel 复用）。 */
export function normalizeMethod(m: JavaAgentMethodInfo): JavaMethodInfo {
  return {
    method_id: m.method_id,
    class_name: m.class_name,
    method_name: m.method_name,
    signature: m.signature,
    return_type: m.return_type,
    return_generic: m.return_generic,
    params: m.params.map((p) => ({
      name: p.name,
      type_name: p.type,
      generic_type: p.generic,
      required: p.primitive,
    })),
    static_method: m.static,
    visibility: m.visibility,
    overloaded: m.overloaded,
    declaring_class: m.declaring_class,
  }
}

/** Agent 类行 → v1 类形状（JavaClassBrowser 复用）。 */
export function normalizeClass(item: JavaAgentClassItem): JavaClassInfo {
  return {
    class_name: item.name,
    package_name: item.package_name,
    simple_name: item.simple_name,
    method_count: -1,
    bean_name: item.bean_name ?? null,
    category: item.category,
    role: item.role,
    category_evidence: item.source?.evidence,
  }
}

export const useJavaAttachStore = defineStore('javaAttach', () => {
  const api = useMiyoApi()
  const t = useLocaleStore().t
  const toast = useToast()
  const IPC_PREFIX = 'plugin:miyo|'

  // ---------- 进程发现 ----------
  const summary = ref<JavaProcessListSummary | null>(null)
  const processes = computed<JavaProcessInfo[]>(() => summary.value?.processes ?? [])
  const processSearch = ref('')
  const autoRefresh = ref(true)
  const scanning = ref(false)

  const filteredProcesses = computed(() => {
    const term = processSearch.value.trim().toLowerCase()
    if (!term) return processes.value
    const numeric = Number(term)
    return processes.value.filter((p) => {
      if (!Number.isNaN(numeric) && p.pid === numeric) return true
      return (
        p.display_name.toLowerCase().includes(term) ||
        p.main_class.toLowerCase().includes(term) ||
        p.command_line.toLowerCase().includes(term)
      )
    })
  })

  let scanTimer: ReturnType<typeof setInterval> | null = null

  function stopAutoRefresh(): void {
    if (scanTimer !== null) {
      clearInterval(scanTimer)
      scanTimer = null
    }
  }

  // ---------- Attach 组件（agent / launcher jar）自动获取 ----------

  interface JavaComponentItem {
    name: string
    present: boolean
    path: string | null
    download_url: string
  }
  interface JavaComponentStatus {
    ready: boolean
    base_dir: string
    items: JavaComponentItem[]
  }

  const componentStatus = ref<JavaComponentStatus | null>(null)
  const componentsDownloading = ref(false)

  /** 查询组件状态（幂等）。 */
  async function loadComponentStatus(): Promise<JavaComponentStatus | null> {
    try {
      componentStatus.value = await tauriInvoke<JavaComponentStatus>(
        `${IPC_PREFIX}java_component_status`,
      )
      return componentStatus.value
    } catch {
      componentStatus.value = null
      return null
    }
  }

  /** 下载缺失组件（后端 sha256 校验 + 原子落盘；成功后刷新进程诊断）。 */
  async function downloadComponents(): Promise<boolean> {
    componentsDownloading.value = true
    try {
      componentStatus.value = await tauriInvoke<JavaComponentStatus>(
        `${IPC_PREFIX}java_component_download`,
      )
      // 组件就绪后刷新进程诊断（launcher_ready 变化）
      void loadProcesses()
      return componentStatus.value.ready
    } catch (e) {
      toast.error(t('java2.componentsDownloadFailed'), { message: errorText(e) })
      return false
    } finally {
      componentsDownloading.value = false
    }
  }

  /** 组件缺失 → 自动下载（插件激活时调用；幂等，已就绪零开销）。 */
  async function ensureComponents(): Promise<void> {
    const status = await loadComponentStatus()
    if (status && !status.ready && !componentsDownloading.value) {
      void downloadComponents()
    }
  }

  /** 扫描本机 Java 进程（失败不抛——返回 null，诊断交给 summary.hints）。 */
  async function loadProcesses(): Promise<JavaProcessListSummary | null> {
    scanning.value = true
    try {
      summary.value = await api.javaDiscoverProcesses()
      // 会话快照补齐：**只按后端会话表回填，不按扫描结果的 attached 回填**。
      //
      // 旧实现见「attached=true 且本地无会话 → 拉 sessions 并写回」，这会在
      // 断开瞬间出问题：detach() 已移除本地会话，但这一轮扫描结果还是断开前的
      // 快照（attached=true），于是把 pid 重新塞回 sessions，界面上「已连接」
      // 复活（真实反馈：点了断开，左侧仍是已连接）。
      // 现在一律以 javaAttachSessions() 为真相来源，扫描结果不参与会话重建。
      const all = await api.javaAttachSessions()
      const live = new Set(all.map((s) => s.pid))
      for (const snap of all) sessions.value.set(snap.pid, snap)
      // 后端已经没有的会话，本地也清掉（防止轮询间隔内残留）
      for (const pid of [...sessions.value.keys()]) {
        if (!live.has(pid)) sessions.value.delete(pid)
      }
      return summary.value
    } catch {
      return null
    } finally {
      scanning.value = false
    }
  }

  /** 自动刷新开关（5s 轮询；attach 工作台挂载时开启）。 */
  function setAutoRefresh(on: boolean): void {
    autoRefresh.value = on
    stopAutoRefresh()
    if (on) {
      scanTimer = setInterval(() => {
        void loadProcesses()
      }, 5_000)
    }
  }

  watch(autoRefresh, (on) => {
    if (!on) stopAutoRefresh()
  })

  // ---------- Attach 会话 ----------
  const sessions = ref<Map<number, JavaAttachSessionSnapshot>>(new Map())
  const activePid = ref<number | null>(null)
  const attachingPid = ref<number | null>(null)

  const activeSession = computed(() =>
    activePid.value != null ? (sessions.value.get(activePid.value) ?? null) : null,
  )
  const attached = computed(() => activePid.value != null && sessions.value.has(activePid.value))

  /** attach 目标（UI 必须先经 JavaAttachConfirmDialog 二次确认再调用）。 */
  async function attach(pid: number): Promise<boolean> {
    attachingPid.value = pid
    try {
      const snapshot = await api.javaAttachProcess(pid)
      sessions.value.set(pid, snapshot)
      activePid.value = pid
      void loadSpringStatus()
      await loadClasses()
      return true
    } finally {
      attachingPid.value = null
    }
  }

  /** 断开连接（Agent 服务继续运行；重连无需重新注入）。 */
  async function detach(pid: number): Promise<void> {
    await api.javaDetachProcess(pid)
    sessions.value.delete(pid)
    if (activePid.value === pid) {
      activePid.value = null
      resetDiscovery()
    }
    // 主动刷新进程列表：自动刷新关闭时也要立刻反映「已断开」，
    // 不能只依赖 5s 轮询（实测：断开后左侧仍显示已连接）
    void loadProcesses()
  }

  /** 卸载 Agent（诚实语义：已加载的类无法卸载，随 JVM 退出回收）。 */
  async function unloadAgent(pid: number): Promise<void> {
    await api.javaUnloadAgent(pid)
    sessions.value.delete(pid)
    if (activePid.value === pid) {
      activePid.value = null
      resetDiscovery()
    }
  }

  function resetDiscovery(): void {
    classes.value = []
    classTotal.value = 0
    classStats.value = null
    classLaunch.value = null
    classSearch.value = ''
    activeClassName.value = null
    methods.value = []
    agentMethods.value = []
    activeAgentMethod.value = null
    beans.value = []
    springStatus.value = null
    lastResponse.value = null
  }

  // ---------- 类 / 方法发现 ----------
  const classes = ref<JavaClassInfo[]>([])
  const classSearch = ref('')
  const classTotal = ref(0)
  const classesLoading = ref(false)
  const classesLoadingMore = ref(false)
  /** 分类统计（业务 / Bean / 依赖 / JDK / 未识别）。 */
  const classStats = ref<JavaClassStats | null>(null)
  /** 分类证据摘要（启动信息 / 应用目录 / Bean 数）。 */
  const classLaunch = ref<Record<string, unknown> | null>(null)
  /** 类别筛选（空 = 默认：业务类 + Spring Bean）。 */
  const categoryFilter = ref<JavaClassCategory[]>([])
  /** 包名前缀筛选（多形态：`com.foo, com.bar.*`）。 */
  const packageFilter = ref('')
  /** 高级浏览：显示第三方依赖。 */
  const includeDependencies = ref(false)
  const includeJdk = ref(false)
  /** 用户配置的业务包前缀（分类弱证据；持久化）。 */
  const businessPackages = ref<string[]>(loadBusinessPackages())
  /** 每页条数（滚动到底增量加载）。 */
  const CLASS_PAGE_SIZE = 300
  const classHasMore = computed(() => classes.value.length < classTotal.value)
  const activeClassName = ref<string | null>(null)
  const methods = ref<JavaMethodInfo[]>([])
  /** 原始 agent 方法（风险分级 / param_types；未归一）。 */
  const agentMethods = ref<JavaAgentMethodInfo[]>([])
  const methodsLoading = ref(false)
  const activeMethodId = ref<string | null>(null)

  /** 当前原始方法（含风险分级 / 实例提示；v1 形状并存）。 */
  const activeAgentMethod = ref<JavaAgentMethodInfo | null>(null)
  const activeMethod = computed(() => activeMethodId.value != null
    ? (methods.value.find((m) => m.method_id === activeMethodId.value) ?? null)
    : null)

  const BUSINESS_PACKAGES_KEY = 'miyo.javaAttach.businessPackages'

  function loadBusinessPackages(): string[] {
    try {
      const raw = localStorage.getItem(BUSINESS_PACKAGES_KEY)
      const parsed = raw ? (JSON.parse(raw) as unknown) : null
      return Array.isArray(parsed)
        ? parsed.filter((v): v is string => typeof v === 'string' && v.trim().length > 0)
        : []
    } catch {
      return []
    }
  }

  function persistBusinessPackages(): void {
    try {
      localStorage.setItem(BUSINESS_PACKAGES_KEY, JSON.stringify(businessPackages.value))
    } catch {
      // localStorage 不可用：配置退化为会话内存
    }
  }

  /** 业务包前缀 → agent 参数（逗号分隔）。 */
  function businessPackagesParam(): string | undefined {
    const list = businessPackages.value.map((v) => v.trim()).filter(Boolean)
    return list.length > 0 ? list.join(',') : undefined
  }

  /**
   * 业务包配置变更：持久化 + 通知 agent 全量重建索引。
   *
   * 必须 reindex 而不是仅刷新列表：已索引的类缓存了旧判定，只改参数不会
   * 重新分类（增量刷新只看新增类）。
   */
  async function saveBusinessPackages(list: string[]): Promise<void> {
    businessPackages.value = list.map((v) => v.trim()).filter(Boolean)
    persistBusinessPackages()
    if (activePid.value != null) {
      try {
        await api.javaAgentClassesReindex(activePid.value, businessPackagesParam())
      } catch (e) {
        toast.error(t('java2.classesReindexFailed'), { message: errorText(e) })
      }
    }
    await loadClasses()
  }

  /** 恢复默认（清空业务包、类别筛选与高级浏览开关）。 */
  async function restoreClassFilterDefaults(): Promise<void> {
    categoryFilter.value = []
    packageFilter.value = ''
    includeDependencies.value = false
    includeJdk.value = false
    await saveBusinessPackages([])
  }

  function categoryParam(): string | undefined {
    const list = [...categoryFilter.value]
    // 空数组 = agent 默认（Spring Bean + 业务类），不传 category
    return list.length > 0 ? list.join(',') : undefined
  }

  /** 首屏 / 条件变化时加载第一页（并刷新统计）。 */
  async function loadClasses(): Promise<void> {
    if (activePid.value == null) return
    classesLoading.value = true
    try {
      const result = await api.javaAgentClasses(
        activePid.value,
        classSearch.value.trim() || undefined,
        includeJdk.value,
        CLASS_PAGE_SIZE,
        0,
        {
          category: categoryParam(),
          packagePrefix: packageFilter.value.trim() || undefined,
          includeDependencies: includeDependencies.value,
          businessPackages: businessPackagesParam(),
        },
      )
      classTotal.value = result.total
      classes.value = result.items.map(normalizeClass)
      if (result.stats) classStats.value = result.stats
      if (result.launch) classLaunch.value = result.launch
    } catch {
      classes.value = []
      classTotal.value = 0
    } finally {
      classesLoading.value = false
    }
  }

  /** 增量加载下一页（滚动到底触发；失败保留已加载部分）。 */
  async function loadMoreClasses(): Promise<void> {
    if (activePid.value == null || classesLoadingMore.value || !classHasMore.value) return
    classesLoadingMore.value = true
    try {
      const result = await api.javaAgentClasses(
        activePid.value,
        classSearch.value.trim() || undefined,
        includeJdk.value,
        CLASS_PAGE_SIZE,
        classes.value.length,
        {
          category: categoryParam(),
          packagePrefix: packageFilter.value.trim() || undefined,
          includeDependencies: includeDependencies.value,
          businessPackages: businessPackagesParam(),
        },
      )
      // 去重：并发刷新可能让分页重叠
      const known = new Set(classes.value.map((c) => c.class_name))
      for (const item of result.items) {
        const row = normalizeClass(item)
        if (!known.has(row.class_name)) classes.value.push(row)
      }
      classTotal.value = result.total
    } catch {
      // 下一页失败不打断浏览
    } finally {
      classesLoadingMore.value = false
    }
  }

  /** 类别勾选切换（空数组 = 默认：业务类 + Spring Bean）。 */
  function toggleCategory(category: JavaClassCategory): void {
    const current = categoryFilter.value
    categoryFilter.value = current.includes(category)
      ? current.filter((c) => c !== category)
      : [...current, category]
    void loadClasses()
  }

  async function selectClass(className: string | null): Promise<void> {
    activeClassName.value = className
    activeMethodId.value = null
    activeAgentMethod.value = null
    methods.value = []
    if (!className || activePid.value == null) return
    methodsLoading.value = true
    try {
      const result = await api.javaAgentMethods(activePid.value, className)
      agentMethods.value = result.methods
      methods.value = result.methods.map(normalizeMethod)
    } finally {
      methodsLoading.value = false
    }
  }

  /**
   * 选中方法：生成参数骨架，并优先沿用**上次调用同一方法时填过的参数**。
   *
   * 真实反馈（2026-11）：「方法参数很麻烦，每次需要点击编辑」——原先每次切方法/
   * 切类都重置成 `null`，同一方法重跑也要从头填。现在按 `pid|method_id` 记住上次
   * 参数，切回来直接可用（类型不兼容的位置回落到骨架默认）。
   */
  function selectMethod(method: JavaMethodInfo | null, agentMethod?: JavaAgentMethodInfo): void {
    activeMethodId.value = method?.method_id ?? null
    activeAgentMethod.value =
      agentMethod ?? agentMethods.value.find((m) => m.method_id === method?.method_id) ?? null
    if (method) {
      const skeleton = method.params.map((p) => defaultArgFor(p.type_name, p.generic_type))
      const previous = readLastArgs(method)
      const merged = mergeArgs(previous, skeleton)
      argsJson.value = JSON.stringify(merged, null, 2)
      timeoutOverride.value = null
    }
  }

  // ---------- 参数记忆（pid + method_id → 上次参数 JSON） ----------
  const LAST_ARGS_KEY = 'miyo.javaAttach.lastArgs'
  /** 上限保护：调试会踩到大量方法，只留最近 200 条。 */
  const LAST_ARGS_LIMIT = 200

  function loadLastArgs(): Record<string, string> {
    try {
      const raw = localStorage.getItem(LAST_ARGS_KEY)
      return raw ? (JSON.parse(raw) as Record<string, string>) : {}
    } catch {
      return {}
    }
  }

  let lastArgsByMethod = loadLastArgs()

  function lastArgsKey(method: JavaMethodInfo): string {
    return `${activePid.value ?? ''}|${method.method_id}`
  }

  function readLastArgs(method: JavaMethodInfo): unknown[] | null {
    const raw = lastArgsByMethod[lastArgsKey(method)]
    if (!raw) return null
    try {
      const parsed = JSON.parse(raw) as unknown
      return Array.isArray(parsed) ? parsed : null
    } catch {
      return null
    }
  }

  /** 记住本次参数（调用成功后回写；失败也记，方便改一处重试）。 */
  function rememberArgs(method: JavaMethodInfo, json: string): void {
    const key = lastArgsKey(method)
    // 重新插入到末尾，保持「最近使用」顺序
    delete lastArgsByMethod[key]
    lastArgsByMethod[key] = json
    const keys = Object.keys(lastArgsByMethod)
    if (keys.length > LAST_ARGS_LIMIT) {
      for (const k of keys.slice(0, keys.length - LAST_ARGS_LIMIT)) delete lastArgsByMethod[k]
    }
    try {
      localStorage.setItem(LAST_ARGS_KEY, JSON.stringify(lastArgsByMethod))
    } catch {
      // localStorage 不可用：退化为会话内存
    }
  }

  /** 清空参数记忆（设置页「恢复默认」用）。 */
  function clearRememberedArgs(): void {
    lastArgsByMethod = {}
    try {
      localStorage.removeItem(LAST_ARGS_KEY)
    } catch {
      // 忽略
    }
  }

  // ---------- Spring Bean ----------
  const springStatus = ref<JavaSpringStatus | null>(null)
  const beans = ref<JavaBeanInfo[]>([])
  const beanSearch = ref('')
  const beansLoading = ref(false)
  const beanTotal = ref(0)

  async function loadSpringStatus(): Promise<void> {
    if (activePid.value == null) return
    try {
      springStatus.value = await api.javaAgentSpringStatus(activePid.value)
    } catch {
      springStatus.value = null
    }
  }

  async function loadBeans(): Promise<void> {
    if (activePid.value == null) return
    beansLoading.value = true
    try {
      const result = await api.javaAgentBeans(
        activePid.value,
        beanSearch.value.trim() || undefined,
        undefined,
        500,
      )
      beanTotal.value = result.total
      beans.value = result.items
    } catch {
      beans.value = []
      beanTotal.value = 0
      // 无 Spring 上下文：状态面板给出原因（不静默——调 status 拿 note）
      await loadSpringStatus()
    } finally {
      beansLoading.value = false
    }
  }

  // ---------- 参数 / 执行 ----------
  const argsJson = ref('[]')
  const editMode = ref<'form' | 'json'>('form')
  const timeoutOverride = ref<number | null>(null)
  /** 实例解析策略（auto=按方法自动；new 需 consent）。 */
  const instanceKind = ref<'auto' | 'static' | 'bean' | 'new'>('auto')
  const newConsent = ref(false)
  const invoking = ref(false)
  const lastResponse = ref<JavaAgentInvokeResponse | null>(null)
  const lastStartedAt = ref<number | null>(null)

  /** 执行（instance 解析在 store 内定稿；consent 由 UI 二次确认后置位）。 */
  async function invoke(): Promise<JavaAgentInvokeResponse | null> {
    if (activePid.value == null || !activeMethod.value) return null
    const agentMethod = activeAgentMethod.value
    const method = activeMethod.value
    invoking.value = true
    lastStartedAt.value = Date.now()
    try {
      let kind: 'static' | 'bean' | 'new'
      if (instanceKind.value === 'auto') {
        kind = method.static_method ? 'static' : 'bean'
      } else {
        kind = instanceKind.value
      }
      const response = await api.javaAgentInvoke(activePid.value, {
        methodId: method.method_id,
        class: method.class_name,
        method: method.method_name,
        paramTypes: agentMethod?.param_types ?? [],
        arguments: JSON.parse(argsJson.value || '[]'),
        instance: {
          kind,
          consent: kind === 'new' ? newConsent.value : undefined,
        },
        timeoutMs: timeoutOverride.value ?? undefined,
        methodSignature: method.signature,
      })
      lastResponse.value = response
      // 记住本次参数：同一方法重跑时自动回填（免去每次点「编辑 JSON」）
      rememberArgs(method, argsJson.value)
      void loadHistory()
      return response
    } finally {
      invoking.value = false
    }
  }

  async function cancelLast(): Promise<void> {
    if (activePid.value == null || !lastResponse.value) return
    await api.javaAgentCancel(activePid.value, lastResponse.value.invocation_id)
  }

  // ---------- 历史（attach 会话 → java_projects 行 → 同表） ----------
  const history = ref<import('@/types/miyoApi').JavaInvocationRecord[]>([])
  const historyLoading = ref(false)
  /** attach 会话对应的调试项目 id（历史归属；attach 后由 loadProjects 解析）。 */
  const historyProjectId = ref<string | null>(null)

  async function resolveHistoryProject(): Promise<void> {
    if (activePid.value == null) {
      historyProjectId.value = null
      return
    }
    try {
      const projects = await api.listJavaProjects()
      const match = projects.find((p) => p.mode === 'attach' && p.attach_pid === activePid.value)
      historyProjectId.value = match?.id ?? null
    } catch {
      historyProjectId.value = null
    }
  }

  async function loadHistory(limit = 100): Promise<void> {
    await resolveHistoryProject()
    if (!historyProjectId.value) {
      history.value = []
      return
    }
    historyLoading.value = true
    try {
      history.value = await api.listJavaHistory(historyProjectId.value, limit)
    } finally {
      historyLoading.value = false
    }
  }

  // ---------- 收藏（localStorage；per pid） ----------
  const FAVORITES_KEY = 'miyo.javaAttach.favorites'
  const favoriteClasses = ref<string[]>(loadFavorites())

  function loadFavorites(): string[] {
    try {
      const raw = localStorage.getItem(FAVORITES_KEY)
      return raw ? (JSON.parse(raw) as string[]) : []
    } catch {
      return []
    }
  }

  function persistFavorites(): void {
    try {
      localStorage.setItem(FAVORITES_KEY, JSON.stringify(favoriteClasses.value))
    } catch {
      // localStorage 不可用：收藏退化为会话内存
    }
  }

  function toggleFavorite(className: string): void {
    const key = `${activePid.value ?? ''}|${className}`
    if (favoriteClasses.value.includes(key)) {
      favoriteClasses.value = favoriteClasses.value.filter((k) => k !== key)
    } else {
      favoriteClasses.value = [key, ...favoriteClasses.value]
    }
    persistFavorites()
  }

  function isFavorite(className: string): boolean {
    return favoriteClasses.value.includes(`${activePid.value ?? ''}|${className}`)
  }

  /** 手动 pid 探测（attach 前可达性确认）。 */
  async function probeManualPid(pid: number): Promise<JavaProcessInfo | null> {
    try {
      const info = await api.javaProbePid(pid)
      // 探测结果并入列表（manual_pid）
      await loadProcesses()
      return info
    } catch {
      return null
    }
  }

  // 会话切换：清空发现状态，重载历史
  watch(activePid, (next, prev) => {
    if (next !== prev) {
      resetDiscovery()
      instanceKind.value = 'auto'
      newConsent.value = false
      void loadHistory()
      if (next != null) {
        void loadSpringStatus()
        void loadClasses()
      }
    }
  })

  /** 视图卸载 / 插件停用回收。 */
  function dispose(): void {
    setAutoRefresh(false)
    stopAutoRefresh()
    resetDiscovery()
    invoking.value = false
    lastResponse.value = null
    history.value = []
  }

  return {
    // 发现
    summary,
    processes,
    // 组件（agent / launcher jar）自动获取
    componentStatus,
    componentsDownloading,
    loadComponentStatus,
    downloadComponents,
    ensureComponents,
    filteredProcesses,
    processSearch,
    autoRefresh,
    scanning,
    loadProcesses,
    setAutoRefresh,
    probeManualPid,
    // 会话
    sessions,
    activePid,
    activeSession,
    attached,
    attachingPid,
    attach,
    detach,
    unloadAgent,
    // 类 / 方法
    classes,
    classSearch,
    classTotal,
    classesLoading,
    classesLoadingMore,
    classHasMore,
    classStats,
    classLaunch,
    categoryFilter,
    packageFilter,
    includeDependencies,
    includeJdk,
    businessPackages,
    saveBusinessPackages,
    restoreClassFilterDefaults,
    toggleCategory,
    loadMoreClasses,
    activeClassName,
    methods,
    agentMethods,
    methodsLoading,
    activeMethodId,
    activeMethod,
    activeAgentMethod,
    loadClasses,
    selectClass,
    selectMethod,
    /** 清空「同一方法记住的上次参数」（设置页 / 恢复默认用）。 */
    clearRememberedArgs,
    // Bean
    springStatus,
    beans,
    beanSearch,
    beansLoading,
    beanTotal,
    loadSpringStatus,
    loadBeans,
    // 参数 / 执行
    argsJson,
    editMode,
    timeoutOverride,
    instanceKind,
    newConsent,
    invoking,
    lastResponse,
    lastStartedAt,
    invoke,
    cancelLast,
    // 历史 / 收藏
    history,
    historyLoading,
    historyProjectId,
    loadHistory,
    favoriteClasses,
    toggleFavorite,
    isFavorite,
    dispose,
  }
})
