/**
 * miyo-core / miyo-http 模型的 TypeScript 镜像。
 *
 * 生成方案（二选一）：
 * 方案 A（推荐）：tauri-specta 在构建期自动生成（`miyo-tauri` 插件的 `bindings.ts`），
 *                Rust 侧模型需要 derive `specta::Type`，产物为「命令 + 类型」一体文件；
 * 方案 B：手写维护本文件。注意：修改 Rust 模型后必须同步本文件，并在 CI 里
 *        加一个字段快照比对（如 `ts-json-schema-generator` + `git diff`）防漂移。
 */

/** 统一命令错误（后端 AppError → { code, message }）。 */
export interface CommandError {
  code:
    | 'DATABASE'
    | 'IO'
    | 'HTTP'
    | 'TIMEOUT'
    | 'SSL'
    | 'DNS'
    | 'CONNECTION'
    | 'VALIDATION'
    | 'NOT_FOUND'
    | 'OPENAPI'
    | 'MOCK'
    | 'TEST'
    | 'SCRIPT'
    | 'WEBSOCKET'
    | 'JSON'
    | 'DECRYPT'
    | 'OAUTH2'
    | 'GRPC'
  message: string
}

export type HttpMethod =
  | 'GET'
  | 'POST'
  | 'PUT'
  | 'DELETE'
  | 'PATCH'
  | 'HEAD'
  | 'OPTIONS'
  | 'GRPC'

export type EndpointStatus =
  | 'designing'
  | 'developing'
  | 'testing'
  | 'released'
  | 'deprecated'

export type ApiKeyLocation = 'header' | 'query'

/** 参数字段类型（接口设计页的 Schema 标注，Rust `FieldType`）。 */
export type FieldType = 'string' | 'number' | 'boolean' | 'object'

/** Query / Header / Path 变量等键值对（Rust `KeyValue`）。 */
export interface KeyValue {
  key: string
  value: string
  enabled: boolean
  description: string
  /** 设计元数据：字段类型（旧数据缺省 string）。 */
  field_type?: FieldType
  /** 设计元数据：是否必填（缺省 true）。 */
  required?: boolean
  /** 设计元数据：示例值。 */
  example?: string
}

/** 动态签名算法（Rust `SignatureAlgorithm`，serde snake_case）。 */
export type SignatureAlgorithm = 'md5' | 'sha256' | 'hmac_sha256'

/** 动态签名摘要编码（Rust `SignatureEncoding`，serde snake_case）。 */
export type SignatureEncoding = 'hex_lower' | 'hex_upper' | 'base64'

/** 动态签名鉴权配置（Rust `DynamicSignatureConfig`）。
 * `app_secret` 持久化时经 AES-256-GCM 加密（miyo-secret）。 */
export interface DynamicSignatureConfig {
  app_key: string
  app_secret: string
  /** App-Key 请求头名（默认 "App-Key"）。 */
  key_header: string
  /** App-Timestamp 请求头名（默认 "App-Timestamp"）。 */
  timestamp_header: string
  /** App-Sig 请求头名（默认 "App-Sig"）。 */
  sig_header: string
  algorithm: SignatureAlgorithm
  encoding: SignatureEncoding
  /** 签名载荷模板：{{$key}} {{$secret}} {{$timestamp}}。 */
  payload_template: string
}

/** 认证方式（Rust `AuthSpec`，tag = "type"）。 */
export type AuthSpec =
  | { type: 'none' }
  | { type: 'bearer'; token: string }
  | { type: 'basic'; username: string; password: string }
  | { type: 'apikey'; key: string; value: string; in: ApiKeyLocation }
  | {
      type: 'oauth2'
      client_id: string
      client_secret: string
      auth_url: string
      token_url: string
      scope: string
      redirect_uri: string
      /** 授权模式（缺省 = authorization_code，旧数据零迁移）。 */
      grant?: OAuth2Grant
      /** password 模式：资源所有者用户名（其他模式忽略）。 */
      username?: string | null
      /** password 模式：资源所有者密码（其他模式忽略）。 */
      password?: string | null
      token?: OAuth2Token
    }
  /** HTTP Digest 认证（RFC 7616）：401 质询后自动应答重发。 */
  | { type: 'digest'; username: string; password: string }
  /** Hawk 认证（HMAC-SHA-256，每次发送实时计算 mac）。 */
  | { type: 'hawk'; key_id: string; key: string }
  /** AWS Signature V4（兼容 SigV4 风格网关）。 */
  | {
      type: 'awsv4'
      access_key: string
      secret_key: string
      region: string
      service: string
      session_token?: string
    }
  /** 通用 HMAC AK-SK 加签（X-Access-Key / X-Timestamp / X-Nonce / X-Signature）。 */
  | { type: 'hmac'; access_key: string; secret_key: string }
  | { type: 'dynamic_signature'; config: DynamicSignatureConfig }


/** OAuth2 令牌（`expires_at` 为 UTC 时刻）。 */
/** OAuth2 授权模式（Rust `OAuth2Grant`，snake_case 序列化）。 */
export type OAuth2Grant = 'authorization_code' | 'client_credentials' | 'password'

export interface OAuth2Token {
  access_token: string
  token_type?: string
  refresh_token?: string
  expires_at: string
}

/** OAuth2 授权状态（UI 状态指示器用）。 */
export type OAuth2Status = 'unauthorized' | 'valid' | 'expiring_soon' | 'expired'

/** Multipart 值类型。 */
export type MultipartValueType = 'text' | 'file_path'

export interface MultipartField {
  key: string
  value_type: MultipartValueType
  value: string
  enabled: boolean
}

/** GraphQL 请求（Rust `GraphQLSpec`，变量为 JSON 文本，operationName 可空）。 */
export interface GraphQLSpec {
  query: string
  variables: string
  operation_name: string
}

/** 字段文档（Rust `FieldDoc`）：设计页结构化 Body 编辑器的注释 sidecar，key 为 JSON Pointer。 */
export interface FieldDoc {
  /** 字段说明（展示用注释）。 */
  description?: string
  /** 是否必填（样本推断默认 true）。 */
  required?: boolean
}

/** gRPC 调用配置（Rust `GrpcSpec`；地址存 Endpoint.path，metadata 复用 headers）。 */
export interface GrpcSpec {
  /** 全限定服务名，如 `user.v1.UserService`。 */
  service: string
  method: string
  /** 请求消息 protobuf-JSON 文本（空串或 "{}" 表示空消息）。 */
  message: string
  /** 是否使用 TLS（false = 明文 h2c）。 */
  use_tls: boolean
  /** 引用的项目级 proto 文件 id（空 = 使用服务端反射）。 */
  proto_ids: string[]
}

/** 项目级 proto 文件（Rust `ProtoFile`）。 */
export interface ProtoFile {
  id: string
  project_id: string
  name: string
  content: string
  created_at: string
  updated_at: string
}

/** 请求 Body（Rust `BodySpec`，tag = "mode"）。 */
export type BodySpec =
  | { mode: 'none' }
  | { mode: 'json'; raw: string }
  | { mode: 'text'; raw: string }
  | { mode: 'urlencoded'; fields: KeyValue[] }
  | { mode: 'multipart'; fields: MultipartField[] }
  | { mode: 'graphql'; spec: GraphQLSpec }
  | { mode: 'binary'; path: string }
  | { mode: 'grpc'; spec: GrpcSpec }

/** GraphQL 错误位置（Rust `GraphQLErrorLocation`）。 */
export interface GraphQLErrorLocation {
  line: number
  column: number
}

/** GraphQL 错误条目（Rust `GraphQLError`）。 */
export interface GraphQLError {
  message: string
  locations: GraphQLErrorLocation[] | null
  path: (string | number | boolean | null)[] | null
}

/** 解析后的 GraphQL 响应（Rust `GraphQLResponse`）。 */
export interface GraphQLResponse {
  data: unknown
  errors: GraphQLError[]
}

/** 统一请求结构（Rust `RequestSpec`）。 */
export interface RequestSpec {
  params: KeyValue[]
  headers: KeyValue[]
  path_variables: KeyValue[]
  auth: AuthSpec
  body: BodySpec
  /** 编辑器配置 Tab 记忆（params/auth/headers/body/...），空时按 Method 智能默认。 */
  active_tab?: string | null
  /** 请求超时（毫秒）；null = 使用全局设置中的默认超时。 */
  timeout_ms: number | null
  follow_redirects: boolean
  tests: unknown | null
  /** 禁用 Cookie 自动回放（默认 false = 携带 Jar 中的同域 Cookie）。 */
  disable_cookies?: boolean
  /** JSON Body 字段文档（JSON Pointer → 注释；设计页结构化编辑器用，发送时忽略）。 */
  body_docs?: Record<string, FieldDoc>
  /** Pre / Post 脚本（pm.* 沙箱执行；缺省 = 无脚本，旧数据零迁移）。 */
  scripts?: ScriptsSpec | null
}

/** Pre / Post 请求脚本（Rust `ScriptsSpec`，Postman 风格 pm.* API）。 */
export interface ScriptsSpec {
  /** 请求发送前执行：可改请求头、读写变量。 */
  pre_request: string
  /** 响应到达后执行：可断言（pm.test）、读响应。 */
  post_response: string
}

/** 单条 pm.test 断言结果（Rust `ScriptTestOutcome`）。 */
export interface ScriptTestOutcome {
  name: string
  passed: boolean
  error?: string | null
}

/** 一个阶段的脚本运行摘要（Rust `ScriptRunSummary`）。 */
export interface ScriptRunSummary {
  phase: 'pre_request' | 'post_response'
  /** 脚本本身执行失败（语法 / 运行时 / 超时）；post 阶段失败不吞响应。 */
  error?: string | null
  tests: ScriptTestOutcome[]
  logs: string[]
}

/** 自增序列（Rust `SeqCounter`）；value 为下一次输出值，key 为空表示全局 `$seq`。 */
export interface SeqCounter {
  key: string
  value: number
}

/** 代理连通性测试结果（Rust `ProxyTestResult`）。 */
export interface ProxyTestResult {
  ok: boolean
  status: number
  duration_ms: number
  message: string
}

/** MCP 工具启停状态（Rust `McpToolStatus`，get/set_mcp_tools 命令）。 */
export interface McpToolStatus {
  name: string
  /** read = 只读信息；write = 会落库。 */
  kind: 'read' | 'write'
  enabled: boolean
}

/** TLS 设置（Rust `TlsSettings`，get/set_tls_settings 命令）。 */
export interface TlsSettings {
  /** 客户端证书 PEM 文件路径（证书链 + 私钥同文件；`null` = 不使用）。 */
  identityPath?: string | null
  /** 放行自签名 / 无效证书（内网调试）。 */
  acceptInvalidCerts?: boolean
}

/** Agent 控制面状态（Rust `AgentStatusInfo`；字段 snake_case）。 */
export interface AgentStatusInfo {
  running: boolean
  address: string | null
  token_path: string
}

/** 项目（Rust `Project`）。 */
export interface Project {
  id: string
  name: string
  description: string
  variables: Record<string, string>
  /** 所属团队 id（旧数据 / 旧备份为 null → 默认团队）。 */
  team_id?: string | null
  created_at: string
  updated_at: string
}

/** 团队（Rust `Team`）：现阶段仅作为项目的容器。 */
export interface Team {
  id: string
  name: string
  created_at: string
  updated_at: string
}

/** 场景步骤：引用接口 + 步骤级提取 / 断言（URL/头/体用接口当前配置）。 */
export interface ScenarioStep {
  endpoint_id: string
  enabled: boolean
  /** 提取规则（ExtractSpec 数组形状，JSON 透传）。 */
  extract: unknown
  /** 断言规则（AssertionSpec 数组形状，JSON 透传）。 */
  assertions: unknown
}

/** 集成测试场景（Rust `Scenario`）：项目级接口链路。 */
export interface Scenario {
  id: string
  project_id: string
  name: string
  description: string
  sort_order: number
  steps: ScenarioStep[]
  /** 自定义脚本（脚本视图编辑后持久化）；缺省 = 未自定义，视图实时由 steps 生成。 */
  script?: string | null
  created_at: string
  updated_at: string
}

/** 提取规则编辑形状（与后端 ExtractSpec 同构）。 */
export interface ScenarioExtractRule {
  name: string
  from: 'body' | 'header'
  path: string
}

/** 断言规则编辑形状（与后端 AssertionSpec 同构）。 */
export interface ScenarioAssertionRule {
  name?: string | null
  type: 'status' | 'header' | 'jsonpath' | 'response_time_ms'
  path?: string | null
  op?:
    | 'eq'
    | 'neq'
    | 'contains'
    | 'not_contains'
    | 'gt'
    | 'gte'
    | 'lt'
    | 'lte'
    | 'exists'
    | 'not_exists'
    | 'empty'
    | 'not_empty'
    | null
  expected?: unknown
}

/** 场景单步结果（Rust `EndpointResult`）。 */
export interface ScenarioStepResult {
  endpoint_id: string
  endpoint_name: string
  method: string
  path: string
  ok: boolean
  status?: number | null
  duration_ms?: number | null
  request_error?: string | null
  outcomes: { description: string; passed: boolean; reason?: string | null }[]
  extracted?: Record<string, string>
}

/** 场景运行结果（Rust `ScenarioRunResult`）。 */
export interface ScenarioRunResult {
  results: ScenarioStepResult[]
  cancelled: boolean
  stopped_on_failure: boolean
  variables: Record<string, string>
}

/** 同步远端类型。 */
export type SyncRemoteKind = 'git' | 's3' | 'db'

/** git 远端配置。 */
export interface GitRemoteConfig {
  kind: 'git'
  url: string
  branch: string
  /** 快照文件在仓库内的路径。 */
  path: string
}

export interface S3RemoteConfig {
  kind: 's3'
  bucket: string
  prefix: string
}

export interface DbRemoteConfig {
  kind: 'db'
  dsn_ref: string
  table: string
}

export type SyncRemoteConfig = GitRemoteConfig | S3RemoteConfig | DbRemoteConfig

/** 团队同步远端（Rust `SyncRemote`）：绑定在团队上，推/拉按项目触发。 */
export interface SyncRemote {
  id: string
  team_id: string
  kind: SyncRemoteKind
  config: SyncRemoteConfig
  last_synced_revision: string | null
  created_at: string
  updated_at: string
}

/** 远端项目快照条目（首页导入入口：发现远端新增项目）。 */
export interface RemoteProjectEntry {
  project_id: string
  name: string
  endpoint_count: number
  updated_at: string
  /** 同 id 项目本地已存在（不可重复导入，进项目内拉取覆盖）。 */
  exists_locally: boolean
}

/** 推送结果。 */
export interface SyncPushResult {
  revision: string
  /** 快照与远端一致（幂等推送，无实际变更）。 */
  noop: boolean
  /** 被脱敏的项（凭证未离开本机）。 */
  redacted: string[]
}

/** 接口粒度快照差异。 */
export interface SyncDiff {
  added: string[]
  removed: string[]
  modified: string[]
}

/** 拉取预览（不改本地）。 */
export interface SyncPullPreview {
  revision: string
  diff: SyncDiff
  snapshot_summary: {
    project: string
    endpoints: number
    folders: number
    environments: number
    exported_at: string
  }
}

/** 拉取应用结果。 */
export interface SyncPullResult {
  revision: string
  applied: boolean
  endpoints: number
  /** 覆盖前本地快照留档路径。 */
  local_backup: string | null
}

/** 项目仪表板统计（Rust `ProjectStat`，list_project_stats 命令）。 */
export interface ProjectStat {
  project_id: string
  endpoint_count: number
  latest_method: string | null
  latest_path: string | null
}

/** 文件夹（Rust `Folder`）。 */
export interface Folder {
  id: string
  project_id: string
  parent_id: string | null
  name: string
  sort_order: number
  created_at: string
  updated_at: string
}

/** 接口（Rust `Endpoint`）。 */
export interface Endpoint {
  id: string
  project_id: string
  folder_id: string | null
  name: string
  method: HttpMethod
  path: string
  description: string
  status: EndpointStatus
  sort_order: number
  request: RequestSpec
  created_at: string
  updated_at: string
}

/** 环境变量（Rust `EnvironmentVariable`）：本地值优先覆盖远程值。 */
export interface EnvironmentVariable {
  key: string
  /** 远程 / 公共值。 */
  remote_value: string
  /** 本地私有覆盖值（非空时优先于 remote_value）。 */
  local_value: string
  /** 是否参与注入。 */
  enabled: boolean
  description?: string | null
}

/** 全局参数注入位置（Rust `GlobalParamLocation`）。 */
export type GlobalParamLocation = 'query' | 'header'

/** 全局参数（Rust `GlobalParam`）：每个请求自动注入的 key/value，无需手动写 {{}}。 */
export interface GlobalParam {
  key: string
  value: string
  enabled: boolean
  /** 注入位置：query = URL 查询参数；header = 请求头。 */
  location: GlobalParamLocation
}

/** 环境（Rust `Environment`，项目维度：哪个项目就编辑哪个项目的环境）。 */
export interface Environment {
  id: string
  /** 归属项目 id。 */
  project_id: string
  name: string
  /** 前置 Base URL（如 `http://dev-test01.redotpay.inet:8092`，可含 `{{变量}}`）。 */
  base_url: string
  /** 结构化环境变量列表。 */
  variables: EnvironmentVariable[]
  created_at: string
  updated_at: string
}

/** 执行请求入参（Rust `ExecuteRequestArgs`）。 */
export interface ExecuteRequestArgs {
  url: string
  method: HttpMethod
  spec: RequestSpec
  environment_id: string | null
  project_id?: string | null
  endpoint_id?: string | null
  /** 请求取消标识（前端生成；提供后可通过 cancelRequest 中止在途请求）。 */
  request_id?: string | null
}

/** 请求历史（Rust `RequestHistory`，miyo-tauri `list_request_histories` 返回）。 */
export interface RequestHistory {
  id: string
  project_id: string
  endpoint_id: string | null
  method: string
  url: string
  status: number | null
  duration_ms: number | null
  request_summary_json: string
  response_summary_json: string
  created_at: string
}

/** 本次请求实际发出形态的快照（Rust `RequestSnapshot`）。
 * 变量渲染、认证注入、全局参数与 Cookie 回放之后的最终值；
 * 临时随响应返回，不落历史，供「请求」页签排查错误。
 */
export interface RequestSnapshot {
  method: string
  /** 实际发送的完整 URL（含 query 参数）。 */
  url: string
  /** 渲染前 URL 模板（含 `{{变量}}`）；与实际 URL 相同时调用方置空。 */
  url_template?: string | null
  /** 实际发送的请求头（含认证注入与自动 Content-Type / Content-Length）。 */
  headers: [string, string][]
  /** Cookie Jar 自动回放的 `[name, value]`。 */
  cookies: [string, string][]
  /** 请求体预览（文本）；流式 / 二进制体为摘要说明。 */
  body: string
  /** `body` 是否非完整原文（截断或摘要）。 */
  body_omitted: boolean
}

/** 执行请求出参（Rust `ExecuteResponse`）。 */
export interface ExecuteResponse {
  status: number
  headers: [string, string][]
  body: string
  content_type: string
  duration_ms: number
  size_bytes: number
  truncated: boolean
  /**
   * `body` 仅为 IPC 预览（Rust 侧超 2MB 时截断，全文经 `get_response_body` 按需取）。
   * 可选：旧后端返回缺失时按 `false` 处理。
   */
  body_omitted?: boolean
  /** 本次请求实际发出的形态（旧后端 / 手工构造场景缺失）。 */
  request?: RequestSnapshot | null
  /** Pre / Post 脚本运行摘要（无脚本时缺失，旧后端兼容）。 */
  scripts?: ScriptRunSummary[]
}

/** cURL 命令解析结果（Rust `CurlParsed`，miyo-tauri `parse_curl_command` 返回）。 */
export interface CurlParsed {
  url: string
  method: HttpMethod
  headers: KeyValue[]
  body: BodySpec | null
  auth: AuthSpec
  /** 被忽略的参数原文（去重保序；旧后端返回缺失时按空处理）。 */
  ignored?: string[]
}

/** 响应示例（Rust `ResponseExample`，miyo-tauri `list_examples` 等返回）。 */
export interface ResponseExample {
  id: string
  endpoint_id: string
  name: string
  status: number
  headers: Record<string, string>
  body: string
  content_type: string
  /** 响应字段文档（JSON Pointer → 注释；设计页结构化编辑器用）。 */
  docs?: Record<string, FieldDoc>
  created_at: string
  updated_at: string
}

/** 文档导出格式（Rust `ExportFormat`，snake_case 序列化）。 */
export type ExportFormat =
  | 'openapi_json'
  | 'openapi_yaml'
  | 'postman'
  | 'markdown'
  | 'html'
  | 'curl_script'

/** 文档导出结果（内容 + 建议文件名）。 */
export interface ExportedDoc {
  content: string
  suggested_name: string
}

/** 请求用例（Rust `RequestExample`）：接口请求快照，可一键回填编辑器。 */
export interface RequestExample {
  id: string
  endpoint_id: string
  name: string
  request: RequestSpec
  created_at: string
  updated_at: string
}

/** 测试用例分组（与后端 CATEGORIES 一致）。 */
export type TestCaseCategory = '正向' | '负向' | '边界值' | '安全性' | '其他'

/** 测试用例运行状态（Rust `TestCaseStatus`）。 */
export type TestCaseStatus = 'Success' | 'Failed' | 'Untested'

/** 测试用例（Rust `TestCase`，miyo-tauri `list_test_cases` 等返回）。 */
export interface TestCase {
  id: string
  /** 关联的主接口 ID（endpoints.id）。 */
  request_id: string
  name: string
  category: TestCaseCategory
  method: HttpMethod
  url_path: string
  /** URL 路径变量取值（`/users/{id}` 里的 id）；随用例快照保存，否则单跑会发字面量 `{id}`。 */
  path_variables: KeyValue[]
  params: KeyValue[]
  headers: KeyValue[]
  /** body 类型标识：json / form-data / raw / urlencoded / graphql / binary / none。 */
  body_type: string
  body_content: string
  last_run_status: TestCaseStatus
  created_at: string
}

/** 代码生成语言（Rust `Lang`，miyo-tauri `codegen_render` 的 `lang` 取值）。 */
export type CodeLang = 'curl' | 'python' | 'js' | 'go' | 'java' | 'php' | 'rust'

/** 备份恢复摘要（miyo-tauri `backup_restore` 返回）。 */
export interface BackupSummary {
  id: string
  name: string
  folders: number
  endpoints: number
  environments: number
  mock_rules: number
  response_examples: number
  request_examples?: number
  /** 恢复时应用的全局设置键（代理/超时/序列，未配置才应用）。 */
  settings_applied?: string[]
  /** 已有配置而跳过的全局设置键。 */
  settings_skipped?: string[]
  /** 按 key 补缺合并的全局变量数。 */
  global_variables_merged?: number
  /** 按 key 补缺合并的全局参数数。 */
  global_params_merged?: number
}

/** 导入文档格式（Rust `ImportFormat`，lowercase 序列化）。 */
export type ImportFormat = 'openapi30' | 'openapi31' | 'swagger20' | 'postman21' | 'unknown'

/** 导入的示例（Rust `ImportedExample`）。 */
export interface ImportedExample {
  name: string
  status: number
  content_type: string
  headers: Record<string, string>
  body: string
}

/** 导入的接口（Rust `ImportedEndpoint`，miyo-tauri `import_document` 返回）。 */
export interface ImportedEndpoint {
  name: string
  method: HttpMethod
  path: string
  description: string
  request: RequestSpec
  examples: ImportedExample[]
  folder_hint: string | null
}

/** 导入解析结果。 */
export interface ImportResult {
  format: ImportFormat
  endpoints: ImportedEndpoint[]
}

/** 单条断言结果（Rust `Outcome`）。 */
export interface Outcome {
  description: string
  passed: boolean
  reason: string | null
}

/** 单接口测试结果（Rust `EndpointResult`）。 */
export interface EndpointResult {
  endpoint_id: string
  endpoint_name: string
  method: string
  path: string
  ok: boolean
  status: number | null
  duration_ms: number | null
  request_error: string | null
  outcomes: Outcome[]
}

/** 压测结果（Rust `LoadResult`）。 */
export interface LoadResult {
  total: number
  ok: number
  failed: number
  total_ms: number
  avg_ms: number
  p50_ms: number
  p90_ms: number
  p99_ms: number
  rps: number
  errors: string[]
  /** 是否被用户中途取消（取消时 total < 请求的 total）。 */
  cancelled?: boolean
}

/** 压测进度（事件 `miyo:load-progress` 载荷，Rust `LoadProgress`）。 */
export interface LoadProgress {
  done: number
  total: number
  ok: number
  failed: number
}

/** 集合测试结果（Rust `CollectionResult`，results 与输入同序）。 */
export interface CollectionResult {
  results: EndpointResult[]
  cancelled: boolean
}

/** 日志文件元信息（`log_files` 返回）。 */
export interface LogFile {
  name: string
  size_bytes: number
  modified_at: string
}

/** Jar 中的 Cookie 条目（`cookie_list` 返回）。 */
export interface CookieEntry {
  name: string
  value: string
  domain: string
  path: string
  /** RFC3339 到期时间；会话 Cookie 为 null。 */
  expires_at: string | null
  secure: boolean
  http_only: boolean
}

/** 集合测试进度（事件 `miyo:test-progress` 载荷）。 */
export interface TestCollectionProgress {
  done: number
  total: number
}

/** 环境交换格式（`export_environment` 入参）。 */
export type EnvExchangeFormat = 'miyo_json' | 'postman_json'

/** 环境导出结果（内容 + 建议文件名）。 */
export interface ExportedEnv {
  content: string
  suggested_name: string
}

/** 环境导入预览（不落库；前端确认后经 saveEnvironment 落库）。 */
export interface ImportedEnv {
  /** 'miyo' | 'postman' */
  format: string
  name: string
  variables: EnvironmentVariable[]
  /** 环境 Base URL（Postman 格式无此信息，为空串）。 */
  base_url: string
}

/** WS 事件（事件 `miyo:ws-event` 载荷）。 */
export type WsEventPayload =
  | { kind: 'state'; connection_id: string; state: string }
  | {
      kind: 'message'
      connection_id: string
      direction: 'in'
      frame: 'text' | 'binary' | 'ping'
      text: string
    }
  | { kind: 'failed'; connection_id: string; message: string }

/** SSE 事件（事件 `miyo:sse-event` 载荷；chunk 为原始文本块）。 */
export type SseEventPayload =
  | { kind: 'open'; connection_id: string }
  | { kind: 'chunk'; connection_id: string; chunk: string }
  | { kind: 'error'; connection_id: string; message: string }
  | { kind: 'closed'; connection_id: string }

/** Mock 匹配项（Rust `MockMatchItem`，query / header 匹配键值）。 */
export interface MockMatchItem {
  key: string
  value: string
}

/** Mock 规则（Rust `MockRule`，miyo-tauri `list_mock_rules` 等返回）。 */
export interface MockRule {
  id: string
  project_id: string
  endpoint_id: string | null
  name: string
  method: HttpMethod
  path: string
  match_query: MockMatchItem[]
  match_headers: MockMatchItem[]
  response_status: number
  response_headers: Record<string, string>
  response_body_template: string
  delay_ms: number
  /** 故障注入比例（0-100，0 = 关闭）。旧后端返回缺失时按 0 处理。 */
  fault_rate_pct?: number
  /** 故障注入状态码（默认 500）。 */
  fault_status?: number
  enabled: boolean
  priority: number
  created_at: string
  updated_at: string
}
// ---------- gRPC 调试（Rust miyo-grpc / commands/grpc.rs） ----------

/** gRPC 服务目录里的单个方法（Rust `GrpcMethodInfo`）。 */
export interface GrpcMethodInfo {
  method: string
  input_type: string
  output_type: string
  client_streaming: boolean
  server_streaming: boolean
}

/** gRPC 服务目录里的单个服务（Rust `GrpcServiceInfo`）。 */
export interface GrpcServiceInfo {
  service: string
  methods: GrpcMethodInfo[]
}

/** 服务目录（grpcListServices 返回；source = reflection | proto）。 */
export interface ServiceCatalog {
  services: GrpcServiceInfo[]
  source: string
}

/** gRPC unary 响应（Rust `GrpcResponse`）。 */
export interface GrpcResponse {
  message_json: string
  metadata: [string, string][]
  grpc_status: number
  grpc_message: string
  duration_ms: number
  size_bytes: number
}

/** grpcInvoke 结果：unary 直返响应；服务端流转 stream_id（消息走事件）。 */
export type GrpcInvokeResult =
  | { kind: 'unary'; response: GrpcResponse }
  | { kind: 'stream'; stream_id: string }

/** 服务端流单条消息（miyo:grpc-event kind=message，Rust `GrpcMessage`）。 */
export interface GrpcMessageEvent {
  kind: 'message'
  sequence: number
  message_json: string
  elapsed_ms: number
  size_bytes: number
}

/** 服务端流结束事件（miyo:grpc-event kind=end，Rust `GrpcEvent::End`）。 */
export interface GrpcEndEvent {
  kind: 'end'
  cancelled: boolean
  grpc_status: number
  grpc_message: string
  metadata: [string, string][]
}

/** miyo:grpc-event 载荷（stream_id + 流事件，flatten）。 */
export type GrpcEventPayload =
  | ({ stream_id: string } & GrpcMessageEvent)
  | ({ stream_id: string } & GrpcEndEvent)
  | { stream_id: string; kind: 'failed'; message: string }

/** grpcInvoke 入参（命令层组装；模板字段由后端按环境渲染）。 */
export interface GrpcInvokeArgs {
  address: string
  service: string
  method: string
  message?: string
  metadata?: KeyValue[]
  use_tls?: boolean
  proto_ids?: string[]
  project_id?: string | null
  timeout_ms?: number | null
  environment_id?: string | null
  endpoint_project_id?: string | null
  endpoint_id?: string | null
  request_id?: string | null
  force_reload?: boolean
}
