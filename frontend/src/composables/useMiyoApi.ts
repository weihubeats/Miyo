/**
 * useMiyoApi：Tauri IPC 的统一前端封装（Vue 3 Composable）。
 *
 * 职责：
 * 1. 把 `invoke('plugin:miyo|command', args)` 收敛为类型安全的 API 方法（类型来自 miyoApi.d.ts）；
 *    前缀 `plugin:miyo` 对应 miyo-tauri 插件名（`Builder::new("miyo")`），
 *    插件命令必须带命名空间，与 capabilities 里 `miyo:default` 权限对应；
 * 2. 统一错误处理：后端 `{ code, message }` → 携带 code 的 Error；
 * 3. 维护「当前激活项目 / 环境」的响应式缓存，与后端 RwLock 状态保持单向同步；
 * 4. `pending` 标志位供全局 loading 指示。
 *
 * 用法：
 * ```ts
 * const api = useMiyoApi()
 * const projects = await api.getProjects()
 * api.saveProject({ ... })          // 失败时抛出携带 code 的 Error
 * api.setActiveProject(projects[0].id)  // 自动同步 activeProject 响应式缓存
 * ```
 */
import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'
import { useProgress } from './useProgress'
import { tFallback } from '../stores/locale'
import type {
  AuthSpec,
  AgentStatusInfo,
  BackupSummary,
  BodySpec,
  CodeLang,
  CommandError,
  CurlParsed,
  Endpoint,
  CollectionResult,
  CookieEntry,
  EndpointResult,
  Environment,
  EnvExchangeFormat,
  EnvironmentVariable,
  ExecuteRequestArgs,
  ExportedEnv,
  ImportedEnv,
  ExecuteResponse,
  ExportFormat,
  ExportedDoc,
  Folder,
  GlobalParam,
  GrpcInvokeArgs,
  GrpcInvokeResult,
  HttpMethod,
  ImportResult,
  KeyValue,
  LoadResult,
  LogFile,
  MockRule,
  OAuth2Token,
  Project,
  ProjectStat,
  ProxyTestResult,
  TlsSettings,
  RequestExample,
  RequestHistory,
  RequestSpec,
  ProtoFile,
  ResponseExample,
  SeqCounter,
  ServiceCatalog,
  RemoteProjectEntry,
  SyncPullPreview,
  SyncPullResult,
  SyncPushResult,
  SyncRemote,
  SyncRemoteConfig,
  Team,
  TestCase,
  TestCaseStatus,
} from '../types/miyoApi'

/** 插件命令统一前缀：`plugin:{插件名}|{命令名}`。 */
const PLUGIN = 'plugin:miyo'

/** 后端 `{ code, message }` → 前端 Error（code 挂载在 err.code，供程序化分支）。 */
export function toFoxError(raw: unknown): Error {
  if (raw && typeof raw === 'object' && 'message' in raw) {
    const { code, message } = raw as CommandError
    const err = new Error(message)
    Object.defineProperty(err, 'code', { value: code, enumerable: true })
    return err
  }
  return raw instanceof Error ? raw : new Error(String(raw))
}

/** 提示去重：同一会话只提示一次，避免批量环境列表解密失败时重复弹出。 */
let decryptionWarned = false

function warnDecryptionFailed(): void {
  if (decryptionWarned) return
  decryptionWarned = true
  import('./useToast').then(({ useToast }) => {
    useToast().error(tFallback('miyoapi.decryptFailTitle'), {
      message: tFallback('miyoapi.decryptWarning'),
      duration: 0,
    })
  })
}

/**
 * 静默 invoke（无全局进度条）：高频纯读路径用——切 Tab 的示例/用例加载、
 * 历史刷新、Mock 轮询等走 `run()` 会让顶部进度条在 `Promise.all` 下抖动。
 * 写操作继续走 `run()`。
 */
async function quiet<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(`${PLUGIN}|${command}`, args)
  } catch (e) {
    const err = toFoxError(e)
    if ('code' in err && err.code === 'DECRYPT') {
      warnDecryptionFailed()
    }
    throw err
  }
}

/** 统一的带错误映射的 invoke 封装（自动加插件命名空间前缀）。 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(`${PLUGIN}|${command}`, args)
  } catch (e) {
    const err = toFoxError(e)
    if ('code' in err && err.code === 'DECRYPT') {
      warnDecryptionFailed()
    }
    throw err
  }
}

/**
 * 模块级单例状态。
 *
 * pending / inflight / activeProject / activeEnvironment 必须全局唯一：
 * 全仓 25+ 处 `useMiyoApi()` 若各自持有一套计数器，任一实例的并发归零都会
 * 提前收尾全局进度条，activeProject 缓存也会分叉（历史缺陷，勿回退为函数内状态）。
 */
const pending = ref(false)

const progress = useProgress()

/** 激活项目 / 环境的响应式缓存（由 setActive* 命令同步）。 */
const activeProject = ref<Project | null>(null)
const activeEnvironment = ref<Environment | null>(null)

/** 并发请求深度：归零时才结束进度条，避免嵌套请求提前收尾。 */
let inflight = 0

/** 执行一个异步任务并维护 pending 状态与顶部进度条。 */
async function run<T>(task: () => Promise<T>): Promise<T> {
  if (inflight === 0) {
    pending.value = true
    progress.start()
  }
  inflight += 1
  try {
    return await task()
  } finally {
    inflight -= 1
    if (inflight === 0) {
      pending.value = false
      progress.done()
    }
  }
}

export function useMiyoApi() {
  // ---------- 项目 ----------
  const getProjects = () => run(() => call<Project[]>('get_projects'))

  const saveProject = (project: Project) => run(() => call<Project>('save_project', { project }))

  const updateProjectsOrder = (projectIds: string[]) =>
    run(() => call<void>('update_projects_order', { projectIds }))

  /** 项目仪表板统计：单条 IPC 返回全部项目的接口数 + 最近更新接口（替代 N+1）。 */
  const listProjectStats = () => run(() => call<ProjectStat[]>('list_project_stats'))

  /** 读取本地文本文件（拖拽导入：Tauri 拖放只给路径，内容用此命令读取）。 */
  const readTextFile = (path: string) => run(() => call<string>('read_text_file', { path }))

  const deleteProject = (projectId: string) =>
    run(() => call<void>('delete_project', { projectId }))

  async function setActiveProject(projectId: string | null): Promise<Project | null> {
    const project = await run(() => call<Project | null>('set_active_project', { projectId }))
    activeProject.value = project
    return project
  }

  const getActiveProject = () => quiet<Project | null>('get_active_project')

  // ---------- 接口 ----------
  const listEndpoints = (projectId: string) =>
    run(() => call<Endpoint[]>('list_endpoints', { projectId }))

  const getEndpoint = (endpointId: string) => run(() => call<Endpoint>('get_endpoint', { endpointId }))

  const saveEndpoint = (endpoint: Endpoint) =>
    run(() => call<Endpoint>('save_endpoint', { endpoint }))

  const deleteEndpoint = (endpointId: string) =>
    run(() => call<void>('delete_endpoint', { endpointId }))

  const duplicateEndpoint = (endpointId: string) =>
    run(() => call<Endpoint>('duplicate_endpoint', { endpointId }))

  // ---------- 文件夹 ----------
  const listFolders = (projectId: string) =>
    run(() => call<Folder[]>('list_folders', { projectId }))

  const saveFolder = (folder: Folder) => run(() => call<Folder>('save_folder', { folder }))

  const deleteFolder = (folderId: string) =>
    run(() => call<void>('delete_folder', { folderId }))

  // ---------- cURL 导入 ----------
  const parseCurlCommand = (command: string) =>
    run(() => call<CurlParsed>('parse_curl_command', { command }))

  // ---------- 环境（项目维度） ----------
  const listEnvironments = (projectId: string) =>
    run(() => call<Environment[]>('list_environments', { projectId }))

  const saveEnvironment = (environment: Environment) =>
    run(() => call<Environment>('save_environment', { environment }))

  async function setActiveEnvironment(environmentId: string | null): Promise<Environment | null> {
    const environment = await run(() =>
      call<Environment | null>('set_active_environment', { environmentId }),
    )
    activeEnvironment.value = environment
    return environment
  }

  const getActiveEnvironment = () => quiet<Environment | null>('get_active_environment')

  const deleteEnvironment = (environmentId: string) =>
    run(() => call<void>('delete_environment', { environmentId }))

  /** 导出单个环境（变量以明文落盘，与备份 JSON 口径一致）。 */
  const exportEnvironment = (environmentId: string, format: EnvExchangeFormat) =>
    run(() => call<ExportedEnv>('export_environment', { environmentId, format }))

  /** 导入环境预览（不落库；自动识别 Miyo / Postman 格式，归属当前项目）。 */
  const importEnvironment = (text: string, projectId: string) =>
    run(() => call<ImportedEnv>('import_environment', { text, projectId }))

  // ---------- 全局变量 ----------
  const getGlobalVariables = () => run(() => call<EnvironmentVariable[]>('get_global_variables'))

  const saveGlobalVariables = (variables: EnvironmentVariable[]) =>
    run(() => call<void>('save_global_variables', { variables }))

  // ---------- 全局参数 ----------
  const getGlobalParams = () => run(() => call<GlobalParam[]>('get_global_params'))

  const saveGlobalParams = (params: GlobalParam[]) =>
    run(() => call<void>('save_global_params', { params }))

  // ---------- 请求执行 ----------
  const executeRequest = (args: ExecuteRequestArgs) =>
    run(() => call<ExecuteResponse>('execute_request', { args }))

  /** 取消一个在途请求（requestId 不存在或已完成时返回 false）。 */
  const cancelRequest = (requestId: string) =>
    call<boolean>('cancel_request', { requestId })

  /**
   * 按 request_id 取被 IPC 预览截断（`body_omitted`）的完整响应正文。
   * Rust 侧仅缓存最近若干条（LRU + TTL），过期 / 未命中返回 null。
   */
  const getResponseBody = (requestId: string) =>
    run(() => call<string | null>('get_response_body', { requestId }))

  // ---------- 响应示例 ----------
  const listExamples = (endpointId: string) =>
    quiet<ResponseExample[]>('list_examples', { endpointId })

  const saveExample = (example: ResponseExample) =>
    run(() => call<ResponseExample>('save_example', { example }))

  const deleteExample = (exampleId: string) =>
    run(() => call<void>('delete_example', { exampleId }))

  // ---------- 请求用例 ----------
  const listRequestExamples = (endpointId: string) =>
    quiet<RequestExample[]>('list_request_examples', { endpointId })

  const saveRequestExample = (example: RequestExample) =>
    run(() => call<RequestExample>('save_request_example', { example }))

  const deleteRequestExample = (exampleId: string) =>
    run(() => call<void>('delete_request_example', { exampleId }))

  // ---------- 测试用例 ----------
  const listTestCases = (requestId: string) =>
    quiet<TestCase[]>('list_test_cases', { requestId })

  const saveTestCase = (testCase: TestCase) =>
    run(() => call<TestCase>('save_test_case', { testCase }))

  const updateTestCaseMeta = (caseId: string, name: string, category: string) =>
    run(() => call<void>('update_test_case_meta', { caseId, name, category }))

  const updateTestCaseStatus = (caseId: string, status: TestCaseStatus) =>
    run(() => call<void>('update_test_case_status', { caseId, status }))

  const updateTestCaseContent = (
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
  ) =>
    run(() =>
      call<void>('update_test_case_content', {
        caseId,
        method: payload.method,
        urlPath: payload.urlPath,
        pathVariables: payload.pathVariables,
        params: payload.params,
        headers: payload.headers,
        bodyType: payload.bodyType,
        bodyContent: payload.bodyContent,
      }),
    )

  const deleteTestCase = (caseId: string) =>
    run(() => call<void>('delete_test_case', { caseId }))

  // ---------- OAuth2 ----------
  const oauthAuthorize = (auth: AuthSpec) =>
    run(() => call<OAuth2Token>('oauth_authorize', { auth }))

  const oauthAccessToken = (auth: AuthSpec) =>
    run(() => call<string>('oauth_access_token', { auth }))

  /** 非交互式获取令牌（client_credentials / password 模式）。 */
  const oauthFetchToken = (auth: AuthSpec) =>
    run(() => call<OAuth2Token>('oauth_fetch_token', { auth }))

  // ---------- 代码生成 ----------
  const codegenRender = (args: {
    lang: CodeLang
    method: HttpMethod
    url: string
    headers: KeyValue[]
    body: BodySpec
    auth: AuthSpec
  }) => run(() => call<string>('codegen_render', args))

  // ---------- 请求历史 ----------
  const listRequestHistories = (projectId: string, limit?: number, endpointId?: string | null) =>
    quiet<RequestHistory[]>('list_request_histories', { projectId, endpointId, limit })

  const clearRequestHistories = (projectId: string, endpointId?: string | null) =>
    run(() => call<number>('clear_request_histories', { projectId, endpointId }))

  // ---------- Mock 服务 ----------
  const mockStart = () => run(() => call<string>('mock_start'))

  const mockStop = () => run(() => call<void>('mock_stop'))

  const mockStatus = () => quiet<string | null>('mock_status')

  /** 热重载 Mock 定义（运行中原子替换路由与模板，无需重启；未运行报错）。 */
  const mockReload = () => run(() => call<number>('mock_reload'))

  // ---------- Mock 规则 ----------
  const listMockRules = (projectId: string) =>
    run(() => call<MockRule[]>('list_mock_rules', { projectId }))

  const saveMockRule = (rule: MockRule) => run(() => call<MockRule>('save_mock_rule', { rule }))

  const deleteMockRule = (ruleId: string) =>
    run(() => call<void>('delete_mock_rule', { ruleId }))

  // ---------- HTTP 设置（全局代理 / 请求超时） ----------
  const getHttpProxy = () => run(() => call<string | null>('get_http_proxy'))

  const setHttpProxy = (proxy: string | null) =>
    run(() => call<void>('set_http_proxy', { proxy }))

  const getHttpTimeoutMs = () => run(() => call<number | null>('get_http_timeout_ms'))

  const setHttpTimeoutMs = (timeoutMs: number) =>
    run(() => call<void>('set_http_timeout_ms', { timeoutMs }))

  // ---------- HTTP 设置（TLS：客户端证书 / 自签放行） ----------
  const getTlsSettings = () => run(() => call<TlsSettings>('get_tls_settings'))

  const setTlsSettings = (args: { identityPath: string | null; acceptInvalidCerts: boolean }) =>
    run(() => call<void>('set_tls_settings', args))

  // ---------- 自增序列 ----------
  const listSeqCounters = () => run(() => call<SeqCounter[]>('list_seq_counters'))

  const setSeqCounter = (key: string, value: number) =>
    run(() => call<void>('set_seq_counter', { key, value }))

  const deleteSeqCounter = (key: string) =>
    run(() => call<void>('delete_seq_counter', { key }))

  const testHttpProxy = (target?: string | null) =>
    run(() => call<ProxyTestResult>('test_http_proxy', { target: target ?? null }))

  // ---------- MCP / Agent 控制面 ----------
  /** 读取 MCP 启用开关（默认 true）。 */
  const getMcpEnabled = () => quiet<boolean>('get_mcp_enabled')

  /** 设置 MCP 启用：true 启动控制面，false 停止；成功后持久化。 */
  const setMcpEnabled = (enabled: boolean) =>
    run(() => call<void>('set_mcp_enabled', { enabled }))

  /** 读取 MCP 监听起始端口（默认 4110）。 */
  const getMcpPort = () => quiet<number>('get_mcp_port')

  /** 设置 MCP 监听起始端口；运行中会按新端口重启控制面。 */
  const setMcpPort = (port: number) => run(() => call<void>('set_mcp_port', { port }))

  /** 查询控制面运行状态与令牌路径。 */
  const agentStatus = () => quiet<AgentStatusInfo>('agent_status')

  // ---------- 备份/恢复 ----------
  const backupExport = (projectId: string) =>
    run(() => call<string>('backup_export', { projectId }))

  const backupRestore = (text: string) =>
    run(() => call<BackupSummary>('backup_restore', { text }))

  // ---------- 团队 ----------
  /** 全部团队（含默认团队，恒在首位）。 */
  const listTeams = () => quiet<Team[]>('list_teams', {})

  /** 新建团队。 */
  const createTeam = (name: string) => run(() => call<Team>('create_team', { name }))

  const updateTeam = (team: Team) => run(() => call<Team>('update_team', { team }))

  /** 删除团队：有下属项目时后端拒绝。 */
  const deleteTeam = (teamId: string) => run(() => call<void>('delete_team', { teamId }))

  /** 项目移入团队。 */
  const moveProjectToTeam = (projectId: string, teamId: string) =>
    run(() => call<void>('move_project_to_team', { projectId, teamId }))

  // ---------- 团队同步（快照推/拉：远端绑定团队，推/拉按项目触发） ----------
  /** 团队的同步远端列表。 */
  const syncRemoteList = (teamId: string) =>
    quiet<SyncRemote[]>('sync_remote_list', { teamId })

  /** 新建团队同步远端（config 为 RemoteConfig，含 kind tag）。 */
  const syncRemoteCreate = (teamId: string, config: SyncRemoteConfig) =>
    run(() => call<SyncRemote>('sync_remote_create', { teamId, config }))

  const syncRemoteDelete = (remoteId: string) =>
    run(() => call<void>('sync_remote_delete', { remoteId }))

  /** 推送项目快照到团队远端（推送前自动脱敏凭证）。 */
  const syncPush = (remoteId: string, projectId: string, message?: string) =>
    run(() => call<SyncPushResult>('sync_push', { remoteId, projectId, message }))

  /** 拉取预览：差异 + 远端快照摘要，不改本地。 */
  const syncPullPreview = (remoteId: string, projectId: string) =>
    run(() => call<SyncPullPreview>('sync_pull_preview', { remoteId, projectId }))

  /** 应用拉取：覆盖前自动留档本地快照，再以远端覆盖。 */
  const syncPullApply = (remoteId: string, projectId: string) =>
    run(() => call<SyncPullResult>('sync_pull_apply', { remoteId, projectId }))

  /** 远端项目快照清单（首页导入入口）。 */
  const syncRemoteListProjects = (remoteId: string) =>
    run(() => call<RemoteProjectEntry[]>('sync_remote_list_projects', { remoteId }))

  /** 导入远端项目快照为本地新项目（远端有、本地无）。 */
  const syncImportProject = (remoteId: string, projectId: string) =>
    run(() =>
      call<{ projectId: string; name: string; endpoints: number }>('sync_import_project', {
        remoteId,
        projectId,
      }),
    )

  // ---------- 数据目录（设置页；变更重启后生效） ----------
  const getDataDir = () => run(() => call<string>('get_data_dir', {}))

  const getDefaultDataDir = () => run(() => call<string>('get_default_data_dir', {}))

  const setDataDir = (path: string) => run(() => call<void>('set_data_dir', { path }))

  const resetDataDir = () => run(() => call<void>('reset_data_dir', {}))

  // ---------- 导入导出 ----------
  const importDocument = (text: string) =>
    run(() => call<ImportResult>('import_document', { text }))

  const exportOpenapi = (projectId: string) =>
    run(() => call<string>('export_openapi', { projectId }))

  /** 多格式文档导出：endpointId 为 null 时导出整个项目。 */
  const exportDocs = (args: {
    projectId: string
    endpointId: string | null
    format: ExportFormat
  }) => run(() => call<ExportedDoc>('export_docs', args))

  /** 导出测试用例为冒烟测试文档：endpointId 为 null 时导出整个项目；includeResults 附带运行结果，runResults 为前端内存态运行元信息。 */
  const exportSmokeDocs = (args: {
    projectId: string
    endpointId: string | null
    includeResults: boolean
    runResults?: Record<string, { status: number; durationMs: number }>
  }) => run(() => call<ExportedDoc>('export_smoke_docs', args))

  /** 写入磁盘（路径来自原生保存框）。 */
  const writeTextFile = (path: string, contents: string) =>
    run(() => call<void>('save_text_file', { path, contents }))

  // ---------- 测试 / 压测 ----------
  const testEndpoint = (args: { endpoint: Endpoint; url: string; environment_id: string | null }) =>
    run(() => call<EndpointResult>('test_endpoint', { args }))

  const loadTest = (args: {
    url: string
    method: HttpMethod
    spec: RequestSpec
    environment_id: string | null
    concurrency: number
    total: number
    run_id?: string | null
  }) => run(() => call<LoadResult>('load_test', { args }))

  /** 取消在途压测（run_id 不存在或已完成时返回 false）。 */
  const cancelLoadTest = (runId: string) =>
    call<boolean>('cancel_load_test', { runId })

  /** 集合测试：一次 IPC 跑完整个集合（后端并发 + 可取消 + 进度事件）。 */
  const testCollection = (args: {
    items: Array<{ endpoint: Endpoint; url: string; spec: RequestSpec }>
    environment_id: string | null
    concurrency?: number | null
    run_id?: string | null
  }) => run(() => call<CollectionResult>('test_collection', { args }))

  /** 取消在途集合测试（run_id 不存在或已完成时返回 false）。 */
  const cancelTestCollection = (runId: string) =>
    call<boolean>('cancel_test_collection', { runId })

  // ---------- Cookie 管理 ----------
  /** 列出 Jar 中的 Cookie（domain 为空返回全部；否则子串过滤域名）。 */
  const cookieList = (domain?: string | null) =>
    quiet<CookieEntry[]>('cookie_list', { domain: domain ?? null })

  /** 清理 Cookie（domain 为空=全部；否则精确域 + 子域）。返回删除条数。 */
  const cookieClear = (domain?: string | null) =>
    run(() => call<number>('cookie_clear', { domain: domain ?? null }))

  // ---------- 日志查看 ----------
  /** 列出日志文件（最新在前）。 */
  const logFiles = () => quiet<LogFile[]>('log_files')

  /** 读取日志尾部（默认 300 行，上限 2000 行）。 */
  const logTail = (file?: string | null, lines?: number | null) =>
    quiet<string>('log_tail', { file: file ?? null, lines: lines ?? null })

  /** 日志目录绝对路径（供「打开目录」）。 */
  const logDirPath = () => quiet<string>('log_dir_path')

  /** 读取日志保留天数（默认 14）。 */
  const getLogRetentionDays = () => quiet<number>('get_log_retention_days')

  /** 设置日志保留天数（1~365），后端立即按新规则清理一次。 */
  const setLogRetentionDays = (days: number) =>
    quiet<void>('set_log_retention_days', { days })

  // ---------- 实时调试（WebSocket / SSE） ----------
  /** 建立 WS 连接：返回 connection_id，后续事件经 `miyo:ws-event` 推送。 */
  const wsConnect = (args: {
    connection_id?: string | null
    url: string
    headers?: Record<string, string>
    subprotocols?: string[]
    auto_reconnect?: boolean
  }) => run(() => call<string>('ws_connect', { args }))

  /** 发送 WS 帧（binary/ping 的 payload 为 base64）。 */
  const wsSend = (args: { connection_id: string; frame: string; payload: string }) =>
    call<void>('ws_send', { args })

  /** 断开 WS 连接（不存在时返回 false）。 */
  const wsDisconnect = (connectionId: string) =>
    call<boolean>('ws_disconnect', { connectionId })

  /** 订阅 SSE：返回 connection_id，原始文本块经 `miyo:sse-event` 推送。 */
  const sseConnect = (args: {
    connection_id?: string | null
    url: string
    headers?: Record<string, string>
    last_event_id?: string | null
  }) => run(() => call<string>('sse_connect', { args }))

  /** 取消 SSE 订阅（不存在时返回 false）。 */
  const sseDisconnect = (connectionId: string) =>
    call<boolean>('sse_disconnect', { connectionId })

  // ---------- gRPC 调试 ----------
  /** 服务目录：proto 文件编译（免连服务器）或服务端反射。 */
  const grpcListServices = (args: {
    address: string
    use_tls?: boolean
    metadata?: KeyValue[]
    proto_ids?: string[]
    project_id?: string | null
    environment_id?: string | null
    force_reload?: boolean
  }) => run(() => call<ServiceCatalog>('grpc_list_services', { args }))

  /** gRPC 调用：unary 直返响应；服务端流转 stream_id（消息经 miyo:grpc-event 推送）。 */
  const grpcInvoke = (args: GrpcInvokeArgs) =>
    run(() => call<GrpcInvokeResult>('grpc_invoke', { args }))

  /** 关闭服务端流（不存在或已结束时返回 false）。 */
  const grpcStreamClose = (streamId: string) =>
    call<boolean>('grpc_stream_close', { streamId })

  /** 项目级 proto 文件列表。 */
  const grpcListProtoFiles = (projectId: string) =>
    quiet<ProtoFile[]>('grpc_list_proto_files', { projectId })

  /** 批量保存 proto 文件（upsert），返回保存后的完整列表。 */
  const grpcSaveProtoFiles = (projectId: string, files: Array<{ id?: string | null; name: string; content: string }>) =>
    run(() => call<ProtoFile[]>('grpc_save_proto_files', { projectId, files }))

  /** 删除 proto 文件。 */
  const grpcDeleteProtoFile = (protoId: string) =>
    run(() => call<void>('grpc_delete_proto_file', { protoId }))

  return {
    pending,
    activeProject,
    activeEnvironment,
    getProjects,
    saveProject,
    updateProjectsOrder,
    listProjectStats,
    readTextFile,
    deleteProject,
    setActiveProject,
    getActiveProject,
    listEndpoints,
    getEndpoint,
    saveEndpoint,
    deleteEndpoint,
    duplicateEndpoint,
    listFolders,
    saveFolder,
    deleteFolder,
    parseCurlCommand,
    listEnvironments,
    saveEnvironment,
    setActiveEnvironment,
    getActiveEnvironment,
    deleteEnvironment,
    exportEnvironment,
    importEnvironment,
    getGlobalVariables,
    saveGlobalVariables,
    getGlobalParams,
    saveGlobalParams,
    executeRequest,
    cancelRequest,
    getResponseBody,
    listExamples,
    saveExample,
    deleteExample,
    listRequestExamples,
    saveRequestExample,
    deleteRequestExample,
    listTestCases,
    saveTestCase,
    updateTestCaseMeta,
    updateTestCaseStatus,
    updateTestCaseContent,
    deleteTestCase,
    oauthAuthorize,
    oauthAccessToken,
    oauthFetchToken,
    codegenRender,
    listRequestHistories,
    clearRequestHistories,
    mockStart,
    mockStop,
    mockStatus,
    mockReload,
    listMockRules,
    saveMockRule,
    deleteMockRule,
    getHttpProxy,
    setHttpProxy,
    getTlsSettings,
    setTlsSettings,
    getHttpTimeoutMs,
    setHttpTimeoutMs,
    listSeqCounters,
    setSeqCounter,
    deleteSeqCounter,
    testHttpProxy,
    getMcpEnabled,
    setMcpEnabled,
    getMcpPort,
    setMcpPort,
    agentStatus,
    backupExport,
    backupRestore,
    listTeams,
    createTeam,
    updateTeam,
    deleteTeam,
    moveProjectToTeam,
    syncRemoteList,
    syncRemoteCreate,
    syncRemoteDelete,
    syncPush,
    syncPullPreview,
    syncPullApply,
    syncRemoteListProjects,
    syncImportProject,
    getDataDir,
    getDefaultDataDir,
    setDataDir,
    resetDataDir,
    importDocument,
    exportOpenapi,
    exportDocs,
    exportSmokeDocs,
    writeTextFile,
    testEndpoint,
    loadTest,
    cancelLoadTest,
    testCollection,
    cancelTestCollection,
    cookieList,
    cookieClear,
    logFiles,
    logTail,
    logDirPath,
    getLogRetentionDays,
    setLogRetentionDays,
    wsConnect,
    wsSend,
    wsDisconnect,
    sseConnect,
    sseDisconnect,
    grpcListServices,
    grpcInvoke,
    grpcStreamClose,
    grpcListProtoFiles,
    grpcSaveProtoFiles,
    grpcDeleteProtoFile,
  }
}

/** 供 provide/inject 或 store 使用的 Api 类型。 */
export type MiyoApi = ReturnType<typeof useMiyoApi>
