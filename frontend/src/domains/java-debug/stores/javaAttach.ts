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
import type {
  JavaAgentInvokeResponse,
  JavaAgentMethodInfo,
  JavaAttachSessionSnapshot,
  JavaBeanInfo,
  JavaClassInfo,
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
export function normalizeClass(item: {
  name: string
  package_name: string
  simple_name: string
}): JavaClassInfo {
  return {
    class_name: item.name,
    package_name: item.package_name,
    simple_name: item.simple_name,
    method_count: -1,
    bean_name: null,
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
      // 已 attach 的进程在 summary 里带 attached 标记；会话快照缺失时同步
      const attached = summary.value.processes.filter((p) => p.attached).map((p) => p.pid)
      for (const pid of attached) {
        if (!sessions.value.has(pid)) {
          const all = await api.javaAttachSessions()
          for (const snap of all) sessions.value.set(snap.pid, snap)
        }
        break
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
  const includeJdk = ref(false)
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

  async function loadClasses(): Promise<void> {
    if (activePid.value == null) return
    classesLoading.value = true
    try {
      const result = await api.javaAgentClasses(
        activePid.value,
        classSearch.value.trim() || undefined,
        includeJdk.value,
        300,
        0,
      )
      classTotal.value = result.total
      classes.value = result.items.map(normalizeClass)
    } finally {
      classesLoading.value = false
    }
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

  function selectMethod(method: JavaMethodInfo | null, agentMethod?: JavaAgentMethodInfo): void {
    activeMethodId.value = method?.method_id ?? null
    activeAgentMethod.value =
      agentMethod ?? agentMethods.value.find((m) => m.method_id === method?.method_id) ?? null
    if (method) {
      argsJson.value = JSON.stringify(
        method.params.map((p) => defaultFor(p.type_name)),
        null,
        2,
      )
      timeoutOverride.value = null
    }
  }

  function defaultFor(typeName: string): unknown {
    switch (typeName) {
      case 'String':
      case 'char':
      case 'Character':
        return ''
      case 'boolean':
      case 'Boolean':
        return false
      case 'int':
      case 'Integer':
      case 'short':
      case 'Short':
      case 'byte':
      case 'Byte':
      case 'long':
      case 'Long':
      case 'float':
      case 'Float':
      case 'double':
      case 'Double':
        return 0
      default:
        return null
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
    includeJdk,
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
