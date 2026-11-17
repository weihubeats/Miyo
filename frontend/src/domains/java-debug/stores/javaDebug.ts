/**
 * Java 调试工作台 store：项目选择、连接状态、类 / 方法发现、参数草稿、
 * 执行结果与历史。与 workspace store 解耦（独立领域），环境变量复用
 * workspace store 的 mergedVariableMap 供参数 {{var}} 提示。
 */
import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useMiyoApi } from '@/composables/useMiyoApi'
import type {
  JavaClassInfo,
  JavaDebugProject,
  JavaDiscoveredRuntime,
  JavaInvokeResponse,
  JavaMethodInfo,
  JavaInvocationRecord,
  JavaRuntimeInfo,
} from '@/types/miyoApi.d'

/** 最近使用方法（会话内存；容量 12）。 */
export interface RecentMethod {
  projectId: string
  methodId: string
  className: string
  signature: string
  at: number
}

export const useJavaDebugStore = defineStore('javaDebug', () => {
  const api = useMiyoApi()

  // ---------- 项目 ----------
  const projects = ref<JavaDebugProject[]>([])
  const activeProjectId = ref<string | null>(null)
  const projectsLoading = ref(false)

  const activeProject = computed(
    () => projects.value.find((p) => p.id === activeProjectId.value) ?? null,
  )

  /** 连接状态：idle 未探测 / ok 握手成功 / fail 失败（message 带原因）。 */
  const connection = ref<{
    state: 'idle' | 'ok' | 'fail'
    message: string
    info: JavaRuntimeInfo | null
  }>({ state: 'idle', message: '', info: null })

  // ---------- 发现 ----------
  const classes = ref<JavaClassInfo[]>([])
  const classesLoading = ref(false)
  const classSearch = ref('')
  const activeClassName = ref<string | null>(null)
  const methods = ref<JavaMethodInfo[]>([])
  const methodsLoading = ref(false)
  const activeMethodId = ref<string | null>(null)

  const activeMethod = computed(
    () => methods.value.find((m) => m.method_id === activeMethodId.value) ?? null,
  )

  // ---------- 参数草稿 ----------
  /** 参数 JSON 文本（可视化表单与原始 JSON 双向同步）。 */
  const argsJson = ref('[]')
  /** 表单模式 / JSON 模式。 */
  const editMode = ref<'form' | 'json'>('form')
  /** 超时覆盖（空 = 项目默认）。 */
  const timeoutOverride = ref<number | null>(null)

  // ---------- 执行 ----------
  const invoking = ref(false)
  const lastResponse = ref<JavaInvokeResponse | null>(null)
  /** 执行开始时刻（本地时钟，展示执行信息用）。 */
  const lastStartedAt = ref<number | null>(null)

  // ---------- 历史 ----------
  const history = ref<JavaInvocationRecord[]>([])
  const historyLoading = ref(false)

  // ---------- 最近使用 ----------
  const recentMethods = ref<RecentMethod[]>([])
  const RECENT_MAX = 12

  function pushRecent(method: JavaMethodInfo): void {
    recentMethods.value = [
      {
        projectId: activeProjectId.value ?? '',
        methodId: method.method_id,
        className: method.class_name,
        signature: method.method_name,
        at: Date.now(),
      },
      ...recentMethods.value.filter(
        (r) => !(r.projectId === activeProjectId.value && r.methodId === method.method_id),
      ),
    ].slice(0, RECENT_MAX)
  }

  /** 收藏（localStorage 持久化；per 项目）。 */
  const FAVORITES_KEY = 'miyo.java.favorites'
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
      // localStorage 不可用（隐私模式等）：收藏退化为会话内存。
    }
  }

  function toggleFavorite(className: string): void {
    const key = `${activeProjectId.value ?? ''}|${className}`
    if (favoriteClasses.value.includes(key)) {
      favoriteClasses.value = favoriteClasses.value.filter((k) => k !== key)
    } else {
      favoriteClasses.value = [key, ...favoriteClasses.value]
    }
    persistFavorites()
  }

  function isFavorite(className: string): boolean {
    return favoriteClasses.value.includes(`${activeProjectId.value ?? ''}|${className}`)
  }

  // ---------- 参数模板（localStorage；per 项目 + 方法） ----------
  const TEMPLATE_KEY = 'miyo.java.argsTemplates'
  const argsTemplate = ref<string | null>(null)

  function loadTemplateMap(): Record<string, string> {
    try {
      const raw = localStorage.getItem(TEMPLATE_KEY)
      return raw ? (JSON.parse(raw) as Record<string, string>) : {}
    } catch {
      return {}
    }
  }

  function templateKey(methodId: string): string {
    return `${activeProjectId.value ?? ''}|${methodId}`
  }

  /** 保存当前参数为模板（随 activeMethod）。 */
  function saveArgsTemplate(): void {
    if (!activeMethodId.value) return
    const map = loadTemplateMap()
    map[templateKey(activeMethodId.value)] = argsJson.value
    try {
      localStorage.setItem(TEMPLATE_KEY, JSON.stringify(map))
    } catch {
      // localStorage 不可用：模板退化为会话内存。
    }
    argsTemplate.value = argsJson.value
  }

  function readArgsTemplate(methodId: string): string | null {
    return loadTemplateMap()[templateKey(methodId)] ?? null
  }

  // ---------- 动作 ----------

  async function loadProjects(): Promise<void> {
    projectsLoading.value = true
    try {
      projects.value = await api.listJavaProjects()
      if (activeProjectId.value && !projects.value.some((p) => p.id === activeProjectId.value)) {
        activeProjectId.value = projects.value[0]?.id ?? null
      }
      if (!activeProjectId.value) {
        activeProjectId.value = projects.value[0]?.id ?? null
      }
    } finally {
      projectsLoading.value = false
    }
  }

  async function connect(): Promise<boolean> {
    if (!activeProject.value) return false
    connection.value = { state: 'idle', message: '', info: null }
    try {
      const info = await api.javaTestConnection(activeProject.value.id)
      connection.value = { state: 'ok', message: '', info }
      const idx = projects.value.findIndex((p) => p.id === activeProjectId.value)
      if (idx >= 0) {
        projects.value[idx] = { ...projects.value[idx], last_info: info }
      }
      return true
    } catch (e) {
      connection.value = { state: 'fail', message: errorText(e), info: null }
      return false
    }
  }

  // ---------- 本地 Runtime 发现 ----------
  const discovering = ref(false)
  const discovered = ref<JavaDiscoveredRuntime[]>([])
  const discoveredAt = ref<number | null>(null)

  /** 扫描本机 Runtime（注册目录 + 端口指纹探测；失败不抛，返回空列表）。 */
  async function discover(): Promise<JavaDiscoveredRuntime[]> {
    discovering.value = true
    try {
      discovered.value = await api.javaDiscoverLocal()
      discoveredAt.value = Date.now()
      return discovered.value
    } catch {
      discovered.value = []
      return []
    } finally {
      discovering.value = false
    }
  }

  // ---------- 心跳（连接期间 30s 探活，Runtime 关停即时感知） ----------
  const HEARTBEAT_MS = 30_000
  let heartbeatTimer: ReturnType<typeof setInterval> | null = null

  function stopHeartbeat(): void {
    if (heartbeatTimer !== null) {
      clearInterval(heartbeatTimer)
      heartbeatTimer = null
    }
  }

  function startHeartbeat(): void {
    stopHeartbeat()
    heartbeatTimer = setInterval(async () => {
      if (connection.value.state !== 'ok' || !activeProject.value) {
        stopHeartbeat()
        return
      }
      try {
        const info = await api.javaTestConnection(activeProject.value.id)
        connection.value = { state: 'ok', message: '', info }
      } catch (e) {
        // 心跳失败即标记断开（HTTP 客户端无长连接，探测失败 = 端点不可达）。
        connection.value = { state: 'fail', message: errorText(e), info: null }
        stopHeartbeat()
      }
    }, HEARTBEAT_MS)
  }

  // store 常驻 pinia（无组件作用域），onScopeDispose 不触发——
  // 心跳只在 connected 期间运行，断开 / 切项目即停，无泄漏路径。
  watch(
    () => connection.value.state,
    (state) => {
      if (state === 'ok') startHeartbeat()
      else stopHeartbeat()
    },
  )

  function disconnect(): void {
    connection.value = { state: 'idle', message: '', info: null }
    classes.value = []
    methods.value = []
    activeClassName.value = null
    activeMethodId.value = null
  }

  async function loadClasses(): Promise<void> {
    if (!activeProject.value || connection.value.state !== 'ok') return
    classesLoading.value = true
    try {
      classes.value = await api.javaListClasses(activeProject.value.id)
    } finally {
      classesLoading.value = false
    }
  }

  async function selectClass(className: string | null): Promise<void> {
    activeClassName.value = className
    activeMethodId.value = null
    methods.value = []
    if (!className || !activeProject.value) return
    methodsLoading.value = true
    try {
      methods.value = await api.javaListMethods(activeProject.value.id, className)
    } finally {
      methodsLoading.value = false
    }
  }

  function selectMethod(method: JavaMethodInfo | null): void {
    activeMethodId.value = method?.method_id ?? null
    if (method) {
      // 参数默认值：优先用已存模板，否则按形参生成 JSON 骨架（基本类型零值，引用类型 null）。
      const template = readArgsTemplate(method.method_id)
      argsJson.value =
        template ??
        JSON.stringify(
          method.params.map((p) => defaultFor(p.type_name)),
          null,
          2,
        )
      argsTemplate.value = template
      timeoutOverride.value = null
      pushRecent(method)
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
        return 0
      case 'long':
      case 'Long':
        return 0
      case 'float':
      case 'Float':
      case 'double':
      case 'Double':
        return 0
      case 'BigDecimal':
      case 'BigInteger':
        return 0
      default:
        return null
    }
  }

  async function invoke(): Promise<JavaInvokeResponse | null> {
    if (!activeProject.value || !activeMethod.value) return null
    invoking.value = true
    lastStartedAt.value = Date.now()
    try {
      const response = await api.javaInvoke(activeProject.value.id, {
        methodId: activeMethod.value.method_id,
        arguments: JSON.parse(argsJson.value || '[]'),
        timeoutMs: timeoutOverride.value ?? undefined,
        methodSignature: activeMethod.value.signature,
      })
      lastResponse.value = response
      void loadHistory()
      return response
    } finally {
      invoking.value = false
    }
  }

  async function cancelLast(): Promise<void> {
    if (!activeProject.value || !lastResponse.value) return
    await api.javaCancelInvocation(activeProject.value.id, lastResponse.value.invocation_id)
  }

  async function loadHistory(limit = 100): Promise<void> {
    if (!activeProject.value) return
    historyLoading.value = true
    try {
      history.value = await api.listJavaHistory(activeProject.value.id, limit)
    } finally {
      historyLoading.value = false
    }
  }

  /** 从历史回填（重新执行 / 复制参数）。 */
  function applyFromRecord(record: JavaInvocationRecord): void {
    const method = methods.value.find((m) => m.method_id === record.method_id)
    if (method) {
      activeMethodId.value = method.method_id
    }
    argsJson.value = JSON.stringify(record.arguments ?? [], null, 2)
    editMode.value = 'json'
    lastResponse.value = null
  }

  // 项目切换：清空发现与结果，重载历史。
  watch(activeProjectId, (next, prev) => {
    if (next !== prev) {
      connection.value = { state: 'idle', message: '', info: null }
      classes.value = []
      methods.value = []
      activeClassName.value = null
      activeMethodId.value = null
      lastResponse.value = null
      history.value = []
      void loadHistory()
    }
  })

  return {
    // 项目
    projects,
    projectsLoading,
    activeProjectId,
    activeProject,
    connection,
    // 发现
    classes,
    classesLoading,
    classSearch,
    activeClassName,
    methods,
    methodsLoading,
    activeMethodId,
    activeMethod,
    // 参数
    argsJson,
    editMode,
    timeoutOverride,
    // 执行
    invoking,
    lastResponse,
    lastStartedAt,
    // 历史 / 收藏
    history,
    historyLoading,
    recentMethods,
    favoriteClasses,
    // 发现
    discovering,
    discovered,
    discoveredAt,
    // 参数模板
    argsTemplate,
    // 动作
    loadProjects,
    connect,
    disconnect,
    discover,
    loadClasses,
    loadHistory,
    selectClass,
    selectMethod,
    saveArgsTemplate,
    invoke,
    cancelLast,
    applyFromRecord,
    toggleFavorite,
    isFavorite,
  }
})

function errorText(e: unknown): string {
  if (e instanceof Error && e.message) return e.message
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return String((e as { message: unknown }).message)
  }
  return String(e)
}
