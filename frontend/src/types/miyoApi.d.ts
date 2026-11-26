/**
 * miyo-core / miyo-http / miyo-tauri 模型的 TypeScript 镜像。
 *
 * 领域模型与关键 view DTO 由 ts-rs 自动生成：Rust 侧模型 derive `TS`
 * （miyo-core / miyo-http / miyo-tauri 的 `ts` feature），`cargo test
 * --features ts` 导出到 `types/generated/`；CI `bindings` job 重新导出后
 * `git diff --exit-code` 防漂移——改 Rust 模型不重新导出即红灯。
 *
 * 本文件只保留两类内容：
 * 1. `export type { … } from '@/types/generated/…'` re-export（导入路径不变）；
 * 2. 尚未接入生成的 IPC view 类型（agent / mock / sync / ws 等手写镜像，
 *    渐进迁移：Rust 侧 derive 后把对应手写块换成 re-export 行）。
 */

import type { AuthSpec } from '@/types/generated/AuthSpec';
import type { BodySpec } from '@/types/generated/BodySpec';
import type { Endpoint } from '@/types/generated/Endpoint';
import type { EnvironmentVariable } from '@/types/generated/EnvironmentVariable';
import type { Folder } from '@/types/generated/Folder';
import type { HttpMethod } from '@/types/generated/HttpMethod';
import type { KeyValue } from '@/types/generated/KeyValue';
import type { MockMatchItem } from '@/types/generated/MockMatchItem';
import type { Project } from '@/types/generated/Project';
import type { RequestExample } from '@/types/generated/RequestExample';
import type { RequestHistory } from '@/types/generated/RequestHistory';
import type { RequestSnapshot } from '@/types/generated/RequestSnapshot';
import type { RequestSpec } from '@/types/generated/RequestSpec';
import type { ResponseExample } from '@/types/generated/ResponseExample';
import type { ScenarioFolder } from '@/types/generated/ScenarioFolder';
import type { ScenarioRunSummary } from '@/types/generated/ScenarioRunSummary';
import type { ScriptRunSummary } from '@/types/generated/ScriptRunSummary';
import type { StepRetry } from '@/types/generated/StepRetry';
import type { TestCaseStatus } from '@/types/generated/TestCaseStatus';
export type { AuthSpec };
export type { BodySpec };
export type { Endpoint };
export type { EnvironmentVariable };
export type { Folder };
export type { HttpMethod };
export type { KeyValue };
export type { MockMatchItem };
export type { Project };
export type { RequestExample };
export type { RequestHistory };
export type { RequestSnapshot };
export type { RequestSpec };
export type { ResponseExample };
export type { ScenarioFolder };
export type { ScenarioRunSummary };
export type { ScriptRunSummary };
export type { StepRetry };
export type { TestCaseStatus };

export type { ApiKeyLocation } from '@/types/generated/ApiKeyLocation';
export type { CommandError } from '@/types/generated/CommandError';
export type { DynamicSignatureConfig } from '@/types/generated/DynamicSignatureConfig';
export type { EndpointStatus } from '@/types/generated/EndpointStatus';
export type { Environment } from '@/types/generated/Environment';
export type { FieldDoc } from '@/types/generated/FieldDoc';
export type { FieldType } from '@/types/generated/FieldType';
export type { GlobalParam } from '@/types/generated/GlobalParam';
export type { GlobalParamLocation } from '@/types/generated/GlobalParamLocation';
export type { GraphQLError } from '@/types/generated/GraphQLError';
export type { GraphQLErrorLocation } from '@/types/generated/GraphQLErrorLocation';
export type { GraphQLResponse } from '@/types/generated/GraphQLResponse';
export type { GraphQLSpec } from '@/types/generated/GraphQLSpec';
export type { GrpcSpec } from '@/types/generated/GrpcSpec';
export type { MockRequestLog } from '@/types/generated/MockRequestLog';
export type { MockRule } from '@/types/generated/MockRule';
export type { MultipartField } from '@/types/generated/MultipartField';
export type { MultipartValueType } from '@/types/generated/MultipartValueType';
export type { OAuth2Grant } from '@/types/generated/OAuth2Grant';
export type { OAuth2Token } from '@/types/generated/OAuth2Token';
export type { ProtoFile } from '@/types/generated/ProtoFile';
export type { ScriptTestOutcome } from '@/types/generated/ScriptTestOutcome';
export type { ScriptsSpec } from '@/types/generated/ScriptsSpec';
export type { SeqCounter } from '@/types/generated/SeqCounter';
export type { SignatureAlgorithm } from '@/types/generated/SignatureAlgorithm';
export type { SignatureEncoding } from '@/types/generated/SignatureEncoding';
export type { SyncRemoteKind } from '@/types/generated/SyncRemoteKind';
export type { Team } from '@/types/generated/Team';
export type { TestCaseRun } from '@/types/generated/TestCaseRun';
export type { TestCaseRunSummary } from '@/types/generated/TestCaseRunSummary';

/** OAuth2 授权状态（UI 状态指示器用）。 */
export type OAuth2Status = 'unauthorized' | 'valid' | 'expiring_soon' | 'expired'

/** Multipart 值类型。 */

/** 代理连通性测试结果（Rust `ProxyTestResult`）。 */
export interface ProxyTestResult {
  ok: boolean
  status: number
  duration_ms: number
  message: string
}

/** MCP 工具启停状态（Rust `McpToolStatus`，get/set_mcp_tools 命令）。 */

/** MCP 工具启停状态（Rust `McpToolStatus`，get/set_mcp_tools 命令）。 */
export interface McpToolStatus {
  name: string
  /** 兼容字段：read_only → "read"，其余 → "write"。 */
  kind: 'read' | 'write'
  /** 细分权限等级（read / write / execute / network_execute / destructive）。 */
  permission:
    | 'read'
    | 'write'
    | 'execute'
    | 'network_execute'
    | 'destructive'
  /** 业务模块（project / endpoint / environment / scenario / test_case / run / system）。 */
  module: string
  /** 短标题。 */
  title: string
  enabled: boolean
}

/** MCP 权限等级状态（Rust `McpPermissionStatus`，get_mcp_permissions 命令）。 */

/** MCP 权限等级状态（Rust `McpPermissionStatus`，get_mcp_permissions 命令）。 */
export interface McpPermissionStatus {
  level: 'read' | 'write' | 'execute' | 'network_execute' | 'destructive'
  label: string
  description: string
  /** 该等级下的工具数。 */
  toolCount: number
  /** 高风险等级：默认关闭，启用必须用户明确确认。 */
  highRisk: boolean
  enabled: boolean
}

/** TLS 设置（Rust `TlsSettings`，get/set_tls_settings 命令）。 */

/** TLS 设置（Rust `TlsSettings`，get/set_tls_settings 命令）。 */
export interface TlsSettings {
  /** 客户端证书 PEM 文件路径（证书链 + 私钥同文件；`null` = 不使用）。 */
  identityPath?: string | null
  /** 放行自签名 / 无效证书（内网调试）。 */
  acceptInvalidCerts?: boolean
}

/** Agent 控制面状态（Rust `AgentStatusInfo`；字段 snake_case）。 */

/** Agent 控制面状态（Rust `AgentStatusInfo`；字段 snake_case）。 */
export interface AgentStatusInfo {
  running: boolean
  address: string | null
  token_path: string
}

/** 项目（Rust `Project`）。 */

/** 步骤执行条件（不满足 → 整步 skipped 不发送）。 */
export interface StepCondition {
  /** 'var'（缺省）= 运行时变量；'prevStatus' = 最近已执行步骤的状态码。 */
  source?: 'var' | 'prevStatus'
  /** source=var 时的变量名。 */
  name?: string
  /** 与断言算子同集（eq/neq/contains/…/exists/empty）。 */
  op: string
  value?: unknown
}

/** 步骤循环（轮询 / 重复执行）。 */

/** 步骤循环（轮询 / 重复执行）。 */
export interface StepLoop {
  mode: 'count' | 'while'
  /** count 模式轮数（1..=1000）。 */
  count?: number | null
  /** while 模式的继续条件（每轮结束后求值，true 继续）。 */
  while_condition?: StepCondition | null
  /** while 轮数上限（1..=100，缺省 10；达上限仍成立判失败）。 */
  max_loops?: number | null
  /** 轮间隔毫秒（≤60000）。 */
  interval_ms?: number | null
}

/** 场景步骤：引用接口（endpoint_id 有值）或内联请求（request 有值）+ 步骤级提取 / 断言
 *  + 执行控制（条件 / 循环 / 等待 / 并行）。 */
/** 步骤重试策略（Rust `StepRetry`；显式配置才启用，None = 不重试）。 */

/** 引用式步骤的请求参数覆盖。 */
export interface StepOverrides {
  /** 替换该步 URL 模板。 */
  url?: string
  params?: Partial<KeyValue>[]
  headers?: Partial<KeyValue>[]
  path_variables?: Partial<KeyValue>[]
  body?: BodySpec
  /** 步骤级超时（毫秒；null = 不改动，清除靠删除该键）。 */
  timeout_ms?: number | null
}

/** 集成测试场景（Rust `Scenario`）：项目级接口链路。 */

/** 提取规则编辑形状（与后端 ExtractSpec 同构；cookie/body_regex 由 miyo-test extract.rs 解析）。 */
export interface ScenarioExtractRule {
  name: string
  from: 'body' | 'header' | 'cookie' | 'body_regex'
  path: string
}

/** 断言规则编辑形状（与后端 AssertionSpec 同构；javascript 的 expected 为沙箱内执行的代码）。 */

/** 断言规则编辑形状（与后端 AssertionSpec 同构；javascript 的 expected 为沙箱内执行的代码）。 */
export interface ScenarioAssertionRule {
  name?: string | null
  type:
    | 'status'
    | 'header'
    | 'jsonpath'
    | 'response_time_ms'
    | 'body'
    | 'length'
    | 'graphql_errors'
    | 'javascript'
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

/** 场景步骤响应快照（Rust `StepResponse`；正文为 IPC 预览，全文经 get_response_body 按需取回）。 */

/** 场景步骤响应快照（Rust `StepResponse`；正文为 IPC 预览，全文经 get_response_body 按需取回）。 */
export interface ScenarioStepResponse {
  status: number
  headers: [string, string][]
  body: string
  content_type: string
  duration_ms: number
  size_bytes: number
  truncated: boolean
  /** `body` 是否仅为 IPC 预览（超 2MB 截断；全文按 `{runId}:{结果位次}` 取回）。 */
  body_omitted?: boolean
  /** `body` 是否为历史落库截断版（64KB 上限；全文不可再取回）。 */
  history_truncated?: boolean
  /** 实际发出形态（变量渲染 / 认证注入后的最终值；旧后端缺失）。 */
  request?: RequestSnapshot | null
}

/** 场景单步结果（Rust `EndpointResult`）。 */

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
  outcomes: {
    description: string
    passed: boolean
    reason?: string | null
    /** 期望值文本（断言显式给出时随行；旧后端缺失）。 */
    expected?: string | null
    /** 实际值文本（取到实际值时随行；旧后端缺失）。 */
    actual?: string | null
  }[]
  extracted?: Record<string, string>
  /** 实际发送的渲染后 URL（含变量替换；旧后端缺失）。 */
  url?: string | null
  /** 链路步骤位次（启用步骤 0-based；旧后端 / 单接口运行为空）。 */
  step_index?: number | null
  /** 该步因取消而中止（非真实失败；旧后端缺失）。 */
  cancelled?: boolean
  /** 响应快照（发送成功时存在；旧后端缺失）。 */
  response?: ScenarioStepResponse | null
  /** 步骤执行状态（success/failed/blocked/skipped/cancelled；旧后端缺失）。 */
  step_status?: string | null
  /** Blocked 原因（缺哪个变量、产自哪步）；仅 blocked 状态有值。 */
  blocked_reason?: string | null
  /** 循环步骤的已执行轮数（>1 时展示 ×N；非循环 / 单次缺省）。 */
  loop_runs?: number | null
  /** 执行阶段（main 缺省；setup / teardown = 生命周期行）。 */
  phase?: string | null
  /** 步骤开始时刻（RFC3339；旧后端缺失）。 */
  started_at?: string | null
  /** 重试日志（attempt: 状态/错误；无重试为空）。 */
  retry_log?: string[]
}

/** 矩阵运行的单行结果（Rust `MatrixRowOutcome`）。 */

/** 矩阵运行的单行结果（Rust `MatrixRowOutcome`）。 */
export interface MatrixRowOutcome {
  key: string
  label: string
  /** 该行运行历史 id（getScenarioRun 可查详情）。 */
  runId: string
  ok: boolean
  okCount: number
  failCount: number
  skippedCount: number
  durationMs?: number | null
}

/** 矩阵运行汇总（Rust `MatrixRunOutcome`；数据驱动 / 环境矩阵共用）。 */

/** 矩阵运行汇总（Rust `MatrixRunOutcome`；数据驱动 / 环境矩阵共用）。 */
export interface MatrixRunOutcome {
  groupId: string
  total: number
  okRows: number
  rows: MatrixRowOutcome[]
}

/** 步骤执行状态（六态；结果行与徽章共用）。 */

/** 步骤执行状态（六态；结果行与徽章共用）。 */
export type WorkflowStepStatus =
  | 'pending'
  | 'running'
  | 'success'
  | 'failed'
  | 'skipped'
  | 'blocked'

/** 场景运行结果（Rust `ScenarioRunResult`）。 */

/** 场景运行结果（Rust `ScenarioRunResult`）。 */
export interface ScenarioRunResult {
  results: ScenarioStepResult[]
  cancelled: boolean
  stopped_on_failure: boolean
  variables: Record<string, string>
  /** 每步执行前的 runtime 快照（与 results 同位序；重跑失败步骤 / 从本步
   * 执行的初始变量来源；旧后端缺失）。 */
  vars_snapshots?: Record<string, string>[]
  /** 运行前预检警告（未定义变量按字面量发送的提示；旧后端缺失）。 */
  warnings?: string[]
  /** 生命周期：Setup 步骤结果（实时运行随 IPC 下发；历史走 results 的 phase 行）。 */
  setup_results?: ScenarioStepResult[]
  teardown_results?: ScenarioStepResult[]
}

/** 场景运行历史（Rust `ScenarioRunSummary`；命令层已剥 results/variables 正文）。 */

/** 运行元信息（meta_json；历史重跑的配置真源）。 */
export interface ScenarioRunMeta {
  version: number
  startedAt: string
  stopOnFailure: boolean
  /** 材料化步骤快照（ScenarioStepInput[]，含当时接口配置 / 内联请求）。 */
  steps: {
    endpoint: Endpoint
    url_template: string
    spec_template: RequestSpec
    extract: { name: string; from: string; path: string }[]
    assertions: {
      name: string | null
      type: string
      path: string | null
      op: string | null
      expected: unknown
    }[]
    enabled: boolean
  }[]
  /** 初始变量（敏感值已脱敏）。 */
  initialVars: Record<string, string>
  warnings: string[]
  /** 失败恢复上下文（v2：每步执行前的 runtime 快照，敏感值脱敏；旧行缺省）。 */
  varsSnapshots?: Record<string, string>[]
  /** 生命周期摘要（v2：setup / teardown 步数与清理是否全部成功）。 */
  lifecycle?: { setupCount: number; teardownCount: number; teardownOk: boolean }
  /** 矩阵行标注（数据驱动 / 环境矩阵；单场景运行缺省）。 */
  matrix?: {
    kind: string
    key: string
    label: string
    index: number
    total: number
    groupId: string
  }
}

/** 场景运行历史详情（Rust `ScenarioRunDetail`；results 与实时运行同形，
 * 响应正文 64KB 内保留可回看；meta 供按当时配置重跑）。 */

/** 场景运行历史详情（Rust `ScenarioRunDetail`；results 与实时运行同形，
 * 响应正文 64KB 内保留可回看；meta 供按当时配置重跑）。 */
export interface ScenarioRunDetail extends ScenarioRunSummary {
  /** 开始时间（meta.startedAt 恢复；旧行缺 meta 时为 null）。 */
  started_at?: string | null
  results: ScenarioStepResult[]
  variables: Record<string, string>
  meta?: ScenarioRunMeta | null
}

/** 同步远端类型。 */

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

/** 推送结果。 */
export interface SyncPushResult {
  revision: string
  /** 快照与远端一致（幂等推送，无实际变更）。 */
  noop: boolean
  /** 被脱敏的项（凭证未离开本机）。 */
  redacted: string[]
}

/** 接口粒度快照差异。 */

/** 接口粒度快照差异。 */
export interface SyncDiff {
  added: string[]
  removed: string[]
  modified: string[]
}

/** 拉取预览（不改本地）。 */

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

/** 拉取应用结果。 */
export interface SyncPullResult {
  revision: string
  applied: boolean
  endpoints: number
  /** 覆盖前本地快照留档路径。 */
  local_backup: string | null
}

/** 项目仪表板统计（Rust `ProjectStat`，list_project_stats 命令）。 */

/** 项目仪表板统计（Rust `ProjectStat`，list_project_stats 命令）。 */
export interface ProjectStat {
  project_id: string
  endpoint_count: number
  scenario_count: number
  request_count: number
  /** 请求总数的成功拆分（2xx/3xx；旧后端缺失按 0）。 */
  ok_count?: number
  /** 请求总数的失败拆分（4xx/5xx / 网络错误；旧后端缺失按 0）。 */
  fail_count?: number
  latest_method: string | null
  latest_path: string | null
}

/** 项目运行质量聚合（Rust `ProjectQualityStat`，list_project_quality 命令，snake_case 契约）。 */
export interface ProjectQualityStat {
  project_id: string
  scenarios_total: number
  /** 有过至少一次运行记录的场景数（0 = 全部未运行）。 */
  scenarios_ran: number
  /** 最近一次运行通过 / 失败（含取消）的场景数。 */
  scenario_last_ok: number
  scenario_last_fail: number
  /** 全项目最近一次场景运行失败时刻（RFC3339；无失败为 null）。 */
  scenario_last_failed_at: string | null
  cases_total: number
  /** last_run_status 分布（Success / Failed / Untested）。 */
  cases_success: number
  cases_failed: number
  cases_untested: number
  /** 全项目最近一次用例运行失败时刻（RFC3339；无失败为 null）。 */
  case_last_failed_at: string | null
}

/** 仪表板最近活动（Rust `RecentActivity`，list_recent_activities 命令，snake_case 契约）。 */
export interface RecentActivity {
  id: string
  project_id: string
  project_name: string
  /** 归属接口（快捷请求等无接口的记录为空；旧后端缺失）。 */
  endpoint_id?: string | null
  endpoint_name: string | null
  method: string
  url: string
  status: number | null
  duration_ms: number | null
  created_at: string
}

/** 文件夹（Rust `Folder`）。 */

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

/** 文档导出格式（Rust `ExportFormat`，snake_case 序列化）。 */
export type ExportFormat =
  | 'openapi_json'
  | 'openapi_yaml'
  | 'postman'
  | 'markdown'
  | 'html'
  | 'curl_script'

/** 文档导出结果（内容 + 建议文件名）。 */

/** 文档导出结果（内容 + 建议文件名）。 */
export interface ExportedDoc {
  content: string
  suggested_name: string
}

/** 项目备份导出结果：content 为完整备份 JSON；sensitive_count 为明文凭据项数
 *  （与同步推送同口径统计，仅提示不替换；旧后端缺失）。 */

/** 项目备份导出结果：content 为完整备份 JSON；sensitive_count 为明文凭据项数
 *  （与同步推送同口径统计，仅提示不替换；旧后端缺失）。 */
export interface BackupExportDoc {
  content: string
  sensitive_count?: number
}

/** 请求用例（Rust `RequestExample`）：接口请求快照，可一键回填编辑器。 */

/** 测试用例分组（与后端 CATEGORIES 一致）。 */
export type TestCaseCategory = '正向' | '负向' | '边界值' | '安全性' | '其他'

/** 测试用例运行状态（Rust `TestCaseStatus`）。 */

/** 代码生成语言（Rust `Lang`，miyo-tauri `codegen_render` 的 `lang` 取值）。 */
export type CodeLang = 'curl' | 'python' | 'js' | 'go' | 'java' | 'php' | 'rust'

/** 备份恢复摘要（miyo-tauri `backup_restore` 返回）。 */

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
  /** 恢复的集成测试场景数（v5 备份起携带；旧备份缺失按 0）。 */
  scenarios?: number
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

/** 导入文档格式（Rust `ImportFormat`，lowercase 序列化）。 */
export type ImportFormat = 'openapi30' | 'openapi31' | 'swagger20' | 'postman21' | 'unknown'

/** 导入的示例（Rust `ImportedExample`）。 */

/** 导入的示例（Rust `ImportedExample`）。 */
export interface ImportedExample {
  name: string
  status: number
  content_type: string
  headers: Record<string, string>
  body: string
}

/** 导入的接口（Rust `ImportedEndpoint`，miyo-tauri `import_document` 返回）。 */

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

/** 导入解析结果。 */
export interface ImportResult {
  format: ImportFormat
  endpoints: ImportedEndpoint[]
}

/** 单条断言结果（Rust `Outcome`）。 */

/** 单条断言结果（Rust `Outcome`）。 */
export interface Outcome {
  description: string
  passed: boolean
  reason: string | null
  /** 结构化对比值（断言显式给出 expected 时随行）。 */
  expected?: string | null
  /** 实际值文本（取到实际值时随行）。 */
  actual?: string | null
}

/** 单接口 / 场景步骤测试结果（Rust `EndpointResult`）。 */

/** 单接口 / 场景步骤测试结果（Rust `EndpointResult`）。 */
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
  /** 本步提取到的变量（场景链路展示用）。 */
  extracted?: Record<string, string>
  /** 实际发送的渲染后 URL（配置错误未发送时为 null）。 */
  url?: string | null
  /** 场景链路步骤位次（单接口 / 集合运行为 null）。 */
  step_index?: number | null
  /** 在途请求被取消。 */
  cancelled?: boolean
  /** 响应快照（发送成功时存在；failed_fast 无响应）。 */
  response?: ScenarioStepResponse | null
  /** 六态状态（success/failed/blocked/skipped/cancelled；单接口 / 集合为 null）。 */
  step_status?: string | null
  blocked_reason?: string | null
  loop_runs?: number | null
}

/** 压测结果（Rust `LoadResult`）。 */

/** 压测结果（Rust `LoadResult`）。 */
export type { LoadSchedule } from '@/types/generated/LoadSchedule';
export type { LoadConfig } from '@/types/generated/LoadConfig';
export type { LoadResult } from '@/types/generated/LoadResult';
export type { LoadProgress } from '@/types/generated/LoadProgress';

/** 集合测试结果（Rust `CollectionResult`，results 与输入同序）。 */

/** 集合测试结果（Rust `CollectionResult`，results 与输入同序）。 */
export interface CollectionResult {
  results: EndpointResult[]
  cancelled: boolean
}

/** 日志文件元信息（`log_files` 返回）。 */

/** 日志文件元信息（`log_files` 返回）。 */
export interface LogFile {
  name: string
  size_bytes: number
  modified_at: string
}

/** Jar 中的 Cookie 条目（`cookie_list` 返回）。 */

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

/** 集合测试进度（事件 `miyo:test-progress` 载荷）。 */
export interface TestCollectionProgress {
  done: number
  total: number
}

/** 环境交换格式（`export_environment` 入参）。 */

/** 环境交换格式（`export_environment` 入参）。 */
export type EnvExchangeFormat = 'miyo_json' | 'postman_json'

/** 环境导出结果（内容 + 建议文件名）。 */

/** 环境导出结果（内容 + 建议文件名）。 */
export interface ExportedEnv {
  content: string
  suggested_name: string
  /** 明文凭证变量计数（旧后端缺失）。 */
  sensitive_count?: number
}

/** 环境导入预览（不落库；前端确认后经 saveEnvironment 落库）。 */

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

/** SSE 事件（事件 `miyo:sse-event` 载荷；chunk 为原始文本块）。 */
export type SseEventPayload =
  | { kind: 'open'; connection_id: string }
  | { kind: 'chunk'; connection_id: string; chunk: string }
  | { kind: 'error'; connection_id: string; message: string }
  | { kind: 'closed'; connection_id: string }

/** Mock 匹配项（Rust `MockMatchItem`，query / header 匹配键值）。 */

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

/** gRPC 服务目录里的单个服务（Rust `GrpcServiceInfo`）。 */
export interface GrpcServiceInfo {
  service: string
  methods: GrpcMethodInfo[]
}

/** 服务目录（grpcListServices 返回；source = reflection | proto）。 */

/** 服务目录（grpcListServices 返回；source = reflection | proto）。 */
export interface ServiceCatalog {
  services: GrpcServiceInfo[]
  source: string
}

/** gRPC unary 响应（Rust `GrpcResponse`）。 */

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

/** grpcInvoke 结果：unary 直返响应；服务端流转 stream_id（消息走事件）。 */
export type GrpcInvokeResult =
  | { kind: 'unary'; response: GrpcResponse }
  | { kind: 'stream'; stream_id: string }

/** 服务端流单条消息（miyo:grpc-event kind=message，Rust `GrpcMessage`）。 */

/** 服务端流单条消息（miyo:grpc-event kind=message，Rust `GrpcMessage`）。 */
export interface GrpcMessageEvent {
  kind: 'message'
  sequence: number
  message_json: string
  elapsed_ms: number
  size_bytes: number
}

/** 服务端流结束事件（miyo:grpc-event kind=end，Rust `GrpcEvent::End`）。 */

/** 服务端流结束事件（miyo:grpc-event kind=end，Rust `GrpcEvent::End`）。 */
export interface GrpcEndEvent {
  kind: 'end'
  cancelled: boolean
  grpc_status: number
  grpc_message: string
  metadata: [string, string][]
}

/** miyo:grpc-event 载荷（stream_id + 流事件，flatten）。 */

/** miyo:grpc-event 载荷（stream_id + 流事件，flatten）。 */
export type GrpcEventPayload =
  | ({ stream_id: string } & GrpcMessageEvent)
  | ({ stream_id: string } & GrpcEndEvent)
  | { stream_id: string; kind: 'failed'; message: string }

/** grpcInvoke 入参（命令层组装；模板字段由后端按环境渲染）。 */

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

// ---------- AI 测试中心 ----------

/** AI 模型设置回显（apiKey 掩码，明文不出仓）。 */

// ---------- AI 测试中心 ----------

/** AI 模型设置回显（apiKey 掩码，明文不出仓）。 */
export interface AiSettingsView {
  base_url: string
  api_key_masked: string
  api_key_set: boolean
  model: string
  temperature: number | null
  max_tokens: number | null
}

/** 保存 AI 设置入参（api_key 缺省 = 保持原值）。 */

/** 保存 AI 设置入参（api_key 缺省 = 保持原值）。 */
export interface SetAiSettingsArgs {
  baseUrl: string
  apiKey?: string | null
  model: string
  temperature?: number | null
  maxTokens?: number | null
}

/** AI 聊天会话摘要。 */

/** AI 聊天会话摘要。 */
export interface AiSessionView {
  id: string
  project_id: string
  title: string
  created_at: string
  updated_at: string
}

/** AI 聊天消息（持久化视图；role: user / assistant / tool / artifact）。 */

/** AI 聊天消息（持久化视图；role: user / assistant / tool / artifact）。 */
export interface AiMessageView {
  id: string
  seq: number
  role: 'user' | 'assistant' | 'tool' | 'artifact' | string
  content: string | null
  tool_calls: unknown[] | null
  tool_call_id: string | null
  artifact: AiArtifact | null
  created_at: string
}

/** 结构化产物（右栏预览面板数据源）。 */

/** 结构化产物（右栏预览面板数据源）。 */
export interface AiArtifact {
  type:
    | 'scenario_draft'
    | 'test_case_drafts'
    | 'fix_proposal'
    | 'coverage_report'
    | 'dependency_report'
    | 'failure_analysis'
  data: unknown
}

/** miyo:ai-chat 事件载荷（flatten）。 */

/** miyo:ai-chat 事件载荷（flatten）。 */
export type AiChatEvent = {
  sessionId: string
  type:
    | 'delta'
    | 'tool_start'
    | 'tool_result'
    | 'pending'
    | 'artifact'
    | 'done'
    | 'error'
    | 'cancelled'
  delta?: string
  callId?: string
  name?: string
  args?: unknown
  ok?: boolean
  result?: unknown
  risk?: 'high' | 'network' | 'write'
  message?: string
  stopReason?: string
  iterations?: number
  artifactType?: AiArtifact['type']
  data?: unknown
}

/** 依赖分析报告。 */

/** 依赖分析报告。 */
export interface DependencyReport {
  project_id: string
  nodes: {
    endpoint_id: string
    name: string
    method: string
    path: string
    produces: string[]
    consumes: string[]
    var_refs: string[]
  }[]
  edges: {
    from: string
    to: string
    via: string[]
    confidence: 'high' | 'medium'
    evidence: string
  }[]
  unresolved: { endpoint_id: string; name: string; vars: string[] }[]
  summary: string
}

/** 覆盖率报告（七维）。 */

/** 覆盖率报告（七维）。 */
export interface CoverageReport {
  project_id: string
  score: number
  endpoint_total: number
  dimensions: { key: string; label: string; endpoint_count: number }[]
  endpoints: {
    endpoint_id: string
    name: string
    method: string
    path: string
    issues: string[]
    case_count: number
    last_run_status: string
    scenario_covered: boolean
  }[]
  summary: string
}

/** 测试用例草稿（generate_test_cases 产物；保存走 createTestCase）。 */

/** 测试用例草稿（generate_test_cases 产物；保存走 createTestCase）。 */
export interface TestCaseDraft {
  kind: string
  category: string
  name: string
  description: string
  method: string
  url_path: string
  path_variables: { key: string; value: string; enabled: boolean }[]
  params: { key: string; value: string; enabled: boolean }[]
  headers: { key: string; value: string; enabled: boolean }[]
  body_type: string
  body_content: string
  assertions: unknown
  notes: string[]
}

/** 场景草稿物化结果（generate_integration_scenario 产物）。 */

/** 场景草稿物化结果（generate_integration_scenario 产物）。 */
export interface ScenarioDraftResult {
  scenario: Scenario
  adjustments: string[]
  warnings: string[]
  var_dependencies: {
    step_index: number
    step_name: string
    produces: string[]
    consumes: string[]
  }[]
  write_operations: {
    step_index: number
    step_name: string
    method: string
    url: string
    risk: string
  }[]
}

/** 修复草稿（propose_scenario_fix 产物）。 */

/** 修复草稿（propose_scenario_fix 产物）。 */
export interface FixProposal {
  run_id: string
  scenario_id: string
  analysis: {
    ok: boolean
    total_steps: number
    failed: number
    root_cause_summary: string | null
    rerun_advice: { should_rerun: boolean; reason: string } | null
    failed_steps: {
      index: number
      name: string
      method: string
      error_type: string
      error: string | null
      status: number | null
      possible_causes: string[]
      suggestions: string[]
      assertion_failures: unknown[]
      root_cause: boolean | null
    }[]
    disclaimer: string
  }
  changes: {
    step_index: number
    step_name: string
    kind: 'extract' | 'assertion_review' | 'manual' | string
    field: string
    before: string
    after: string
    reason: string
    risk: string
  }[]
  draft_steps: ScenarioStep[] | null
  notes: string | null
  disclaimer: string
}

/** 场景草稿历史条目。 */

/** 场景草稿历史条目。 */
export interface ScenarioDraftView {
  id: string
  scenario_id: string
  base: Scenario
  draft: Scenario
  summary: string
  source: string
  created_at: string
  applied_at: string | null
}

// ---- 场景方言与执行结果 view 类型：保留手写 ----
// - ScenarioStep 的 condition/loop/overrides 在前端是紧凑方言类型
//   （StepCondition/StepLoop/StepOverrides），生成产物是 JsonValue 宽类型；
// - ExecuteResponse/Scenario 的可选字段是旧后端兼容契约（字段缺失时前端兜底）。


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


/** 集成测试场景（Rust `Scenario`）：项目级接口链路。 */
export interface Scenario {
  id: string
  project_id: string
  name: string
  description: string
  /** 分类标签（场景库过滤；空 = 未分类）。 */
  category: string
  /** 所属业务目录（缺省 = 未分类）。 */
  folder_id?: string | null
  /** 标签（场景库过滤 / 批量打标用）。 */
  tags?: string[]
  /** 收藏标记（列表置顶）。 */
  favorite?: boolean
  sort_order: number
  steps: ScenarioStep[]
  /** 自定义脚本（脚本视图编辑后持久化）；缺省 = 未自定义，视图实时由 steps 生成。 */
  script?: string | null
  /** Setup 步骤（主步骤之前执行；失败 → 主步骤跳过，teardown 仍执行）。 */
  setup?: ScenarioStep[]
  /** Teardown 步骤（失败 / 超时 / 取消时尽可能执行；写操作自动幂等键）。 */
  teardown?: ScenarioStep[]
  /** 自动清理开关（false = 不自动执行 teardown；缺省 true）。 */
  teardown_enabled?: boolean
  /** 数据驱动数据集（多组数据独立执行、变量隔离）。 */
  data_sets?: DataSet[]
  created_at: string
  updated_at: string
}


export interface ScenarioStep {
  /** 引用式步骤的接口 id；内联步骤为 null。 */
  endpoint_id: string | null
  enabled: boolean
  /** 提取规则（ExtractSpec 数组形状，JSON 透传）。 */
  extract: unknown
  /** 断言规则（AssertionSpec 数组形状，JSON 透传）。 */
  assertions: unknown
  /** 步骤名（内联必填；引用缺省用接口名展示）。 */
  name?: string | null
  /** 内联请求定义；缺省 = 引用式步骤。 */
  request?: ScenarioInlineRequest | null
  /** Java 方法调用定义；缺省 = HTTP 步骤（与后端 ScenarioJavaInvocation 同构）。 */
  java_invocation?: {
    java_project_id: string
    class_name: string
    method_id: string
    arguments: unknown
    timeout_ms?: number | null
  } | null
  /** 执行条件（不满足 → skipped 不发送）。 */
  condition?: StepCondition | null
  /** 循环配置。 */
  loop?: StepLoop | null
  /** 执行前等待毫秒（≤60000）。 */
  wait_ms?: number | null
  /** 与相邻 parallel 步骤成批并发（≥2 连续标记才成批）。 */
  parallel?: boolean
  /** 引用式步骤的请求参数覆盖（键：url/params/headers/path_variables/body；
   *  未覆盖项沿用接口当前配置；按 key 合并，body 整体替换）。 */
  overrides?: StepOverrides | null
  /** 步骤重试策略（显式配置才启用；缺省 = 不重试）。 */
  retry?: StepRetry | null
}


/** 场景步骤的内联请求定义（步骤自持请求配置，不引用接口）。 */
export interface ScenarioInlineRequest {
  /** 宽松字符串（未知方法运行时报错，徽章兜底中性灰）。 */
  method: string
  /** 请求 URL 模板，支持 {{base_url}} / {{var}}。 */
  url: string
  /** 与接口 RequestSpec 同构（params/headers/body/auth/timeout 等）。 */
  spec: RequestSpec
}


/** 场景库列表项（Rust `ScenarioLibraryItem`；场景字段平铺 + 最近运行摘要）。 */
export interface ScenarioLibraryItem extends Scenario {
  last_run?: ScenarioRunSummary | null
  /** 全量执行次数 / 成功次数 / 失败次数（运行历史会修剪，计数由后端累计）。 */
  run_count?: number
  ok_run_count?: number
  fail_run_count?: number
}

/** 删除场景目录时的场景处理策略：仅空目录 / 场景移到未分类 / 连同场景删除。 */
export type ScenarioFolderDeleteMode = 'empty' | 'keep' | 'cascade'

/** 目录删除结果（Rust `FolderDeleteOutcome`）。 */
export interface ScenarioFolderDeleteOutcome {
  deletedFolders: number
  movedScenarios: number
  deletedScenarios: number
}

// TestCase 同上：assertions / extractors 是 miyo-test 方言 JSON 透传，
// 前端以 ScenarioAssertionRule / ScenarioExtractRule 紧类型消费。


/** 测试用例（Rust `TestCase`，miyo-tauri `list_test_cases` 等返回）。 */
export interface TestCase {
  id: string
  /** 关联的主接口 ID（endpoints.id）。 */
  request_id: string
  name: string
  category: TestCaseCategory
  /** 用例文件夹（自由分组名，如「Dlocal 渠道」）；空 = 未分组。 */
  folder?: string
  method: HttpMethod
  url_path: string
  /** URL 路径变量取值（`/users/{id}` 里的 id）；随用例快照保存，否则单跑会发字面量 `{id}`。 */
  path_variables: KeyValue[]
  params: KeyValue[]
  headers: KeyValue[]
  /** body 类型标识：json / form-data / raw / urlencoded / graphql / binary / none。 */
  body_type: string
  body_content: string
  /** 断言规则（miyo-test AssertionSpec 形状；空数组 = 无断言，2xx-3xx 即通过）。 */
  assertions?: ScenarioAssertionRule[]
  /** 变量提取规则（miyo-test ExtractSpec 形状）。 */
  extractors?: ScenarioExtractRule[]
  /** 用例级认证覆盖（null = 继承快照原状）。 */
  auth?: AuthSpec | null
  /** 用例级超时覆盖毫秒（null = 沿用接口 → 全局 → 300s）。 */
  timeout_ms?: number | null
  /** 禁用 Cookie 回放。 */
  disable_cookies?: boolean
  /** 组内排序序号（拖拽写回；同值按 created_at 稳定序）。 */
  sort_order?: number
  last_run_status: TestCaseStatus
  /** 最近一次执行时间 / 断言通过数 / 失败数（落历史时回写）。 */
  last_run_at?: string | null
  last_run_ok?: number | null
  last_run_fail?: number | null
  created_at: string
}


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


/** 数据驱动的一组数据集（Rust `DataSet`）。 */
export interface DataSet {
  id: string
  name: string
  source:
    | { kind: 'inline'; rows: Record<string, unknown>[] }
    | { kind: 'json'; text: string }
    | { kind: 'csv'; text: string }
    | { kind: 'environment' }
  sort_order?: number
}


// ---------------------------------------------------------------------------
// Java 方法实时调试（Rust `miyo-core::model::JavaDebugProject` / miyo-java 协议）
// ---------------------------------------------------------------------------

/** Java 调试项目（连接 Runtime 的配置；token 列表 / 保存返回一律脱敏，仅在编辑时于本机输入）。 */
export interface JavaDebugProject {
  id: string
  project_id?: string | null
  name: string
  /** 连接模式：runtime = SDK 嵌入端点（v1 兼容）；attach = 本地 JVM 动态注入。 */
  mode: 'runtime' | 'attach'
  /** attach 模式的目标进程 pid。 */
  attach_pid?: number | null
  /** 目标 JVM 启动时刻（epoch ms；重启识别）。 */
  vm_start_ms?: number | null
  runtime_endpoint: string
  environment_id?: string | null
  /** 仅在保存请求里有意义：缺省 = 保持现有 token，空串 = 显式清除；回显恒为 null。 */
  auth_token?: string | null
  timeout_ms: number
  max_result_bytes: number
  max_concurrent: number
  description: string
  last_connected_at?: string | null
  /** 最近一次握手缓存（非权威；连接探测后刷新）。 */
  last_info?: JavaRuntimeInfo | null
  created_at: string
  updated_at: string
}

/** 本机发现的 Runtime 候选（miyo_java::discover::JavaDiscoveredRuntime）。 */
export interface JavaDiscoveredRuntime {
  /** registry = SDK 注册文件；port_probe = 端口指纹探测。 */
  source: 'registry' | 'port_probe'
  endpoint: string
  /** 探测路径读不到应用名（需 token），为空串。 */
  app_name: string
  java_version: string
  pid?: number | null
  sdk_version: string
  /** 端口指纹探测确认存活；false = 注册文件残留（进程可能已退出）。 */
  alive: boolean
}

/** Runtime 自述信息（GET /miyo/debug/info）。 */
export interface JavaRuntimeInfo {
  protocol_version: number
  runtime_version: string
  app_name: string
  java_version: string
  jvm_version: string
  uptime_ms: number
  class_count: number
  method_count: number
  instance_provider: string
}

/** 白名单类。 */
export interface JavaClassInfo {
  class_name: string
  package_name: string
  simple_name: string
  method_count: number
  bean_name?: string | null
  /** attach 模式带分类；runtime（SDK 白名单）模式恒为 undefined。 */
  category?: JavaClassCategory
  role?: JavaClassRole
  category_evidence?: string
}

/** 方法参数。 */
export interface JavaParamInfo {
  name: string
  type_name: string
  generic_type: string
  required: boolean
}

/** 白名单方法（method_id 为调用唯一依据，重载消歧）。 */
export interface JavaMethodInfo {
  method_id: string
  class_name: string
  method_name: string
  signature: string
  return_type: string
  return_generic: string
  params: JavaParamInfo[]
  static_method: boolean
  visibility: string
  overloaded: boolean
  declaring_class: string
}

/** 异常详情。 */
export interface JavaExceptionInfo {
  exception_type: string
  message?: string | null
  stack_trace: string[]
  cause_chain: string[]
}

/** 调用结果状态。 */
export type JavaInvocationStatus =
  | 'success'
  | 'failed'
  | 'timeout'
  | 'cancelled'
  | 'runtime_disconnected'

/** 方法调用响应。 */
export interface JavaInvokeResponse {
  invocation_id: string
  method_id: string
  status: JavaInvocationStatus
  result?: unknown | null
  result_truncated: boolean
  exception?: JavaExceptionInfo | null
  duration_ms?: number | null
  started_at?: string | null
  finished_at?: string | null
}

/** 调用历史记录（大字段截断落库）。 */
export interface JavaInvocationRecord {
  id: string
  project_id: string
  class_name: string
  method_id: string
  method_signature: string
  arguments: unknown
  status: string
  result?: unknown | null
  result_truncated: boolean
  exception?: JavaExceptionInfo | null
  duration_ms?: number | null
  started_at?: string | null
  finished_at?: string | null
  created_at: string
}

// ---------------------------------------------------------------------------
// Java Debug 2.0：本地 JVM Attach（miyo-java-attach crate）
// ---------------------------------------------------------------------------

/** 本机 Java 进程（发现层输出；命令行已脱敏）。 */
export interface JavaProcessInfo {
  pid: number
  main_class: string
  display_name: string
  command_line: string
  java_executable: string
  java_home: string
  java_version: string
  jvm_vendor: string
  /** jdk / jre / unknown。 */
  kind: string
  started_at: string
  elapsed: string
  uid: string
  attachable: boolean
  attach_block_reason?: string | null
  /** 本 Miyo 会话已 attach。 */
  attached: boolean
  source: 'os_scan' | 'manual_pid'
}

/** 进程发现汇总（含工具链可用性）。 */
export interface JavaProcessListSummary {
  processes: JavaProcessInfo[]
  toolchain_count: number
  launcher_ready: boolean
  agent_ready: boolean
  scan_ms: number
  hints: string[]
}

/** Agent 类行（原始形状；store 归一为 JavaClassInfo 后进组件）。 */
/** 类分类（agent 侧多维度判定：codeSource / 启动信息 / Bean 角色）。 */
export type JavaClassCategory =
  | 'application'
  | 'spring_bean'
  | 'dependency'
  | 'jdk_internal'
  | 'generated'
  | 'unknown'

/** 业务子类型（仅 application / spring_bean 有意义）。 */
export type JavaClassRole =
  | 'controller'
  | 'service'
  | 'manager'
  | 'repository'
  | 'dto'
  | 'config'
  | 'other'

export interface JavaAgentClassItem {
  name: string
  package_name: string
  simple_name: string
  jdk_internal: boolean
  array: boolean
  synthetic: boolean
  loader: { id: string; name: string; identity: number }
  /** 分类结果（旧 agent / runtime 模式可能缺失）。 */
  category?: JavaClassCategory
  role?: JavaClassRole
  /** Spring Bean 名（该类是某个 Bean 的实现类时）。 */
  bean_name?: string | null
  /** 代码来源（「为什么这样归类」）。 */
  source?: {
    kind: string
    location: string | null
    label: string
    evidence: string
  } | null
}

/** 分类统计（业务 / Bean / 依赖 / JDK / 未识别 + 角色分布）。 */
export interface JavaClassStats {
  total: number
  loaded_class_count: number
  by_category: Record<string, number>
  by_role: Record<string, number>
  generation: number
  indexed_at: number
}

/** 类清单响应（分页 + 统计 + 启动信息证据）。 */
export interface JavaClassListResponse {
  total: number
  returned: number
  offset: number
  items: JavaAgentClassItem[]
  categories?: string[]
  stats?: JavaClassStats
  incremental?: boolean
  refresh?: Record<string, number>
  launch?: Record<string, unknown>
}

/** Agent 握手信息。 */
export interface JavaAgentHandshakeInfo {
  protocol: number
  agent_version: string
  pid: number
  vm_start_ms: number
  java_version: string
  java_vm_version: string
  jvm_vendor: string
  java_home: string
  max_concurrent: number
  max_timeout_ms: number
  uptime_ms: number
}

/** attach 会话快照（不含 token）。 */
export interface JavaAttachSessionSnapshot {
  pid: number
  vm_start_ms: number
  agent_version: string
  protocol: number
  handshake: JavaAgentHandshakeInfo
  launcher_java_home: string
  attached_at: string
  /** Agent 已在目标 JVM 运行（本次仅换发 token，无新注入）。 */
  reattached: boolean
}

/** Agent 方法信息（attach 模式：方法级风险分级 + 实例解析提示）。 */
export interface JavaAgentMethodInfo {
  method_id: string
  class_name: string
  declaring_class: string
  method_name: string
  signature: string
  params: { name: string; type: string; generic: string; primitive: boolean }[]
  param_types: string[]
  return_type: string
  return_generic: string
  static: boolean
  visibility: string
  synthetic: boolean
  bridge: boolean
  varargs: boolean
  overloaded: boolean
  /** blocked / destructive / instance / safe。 */
  risk: string
  risk_reason?: string | null
}

/** attach 调用响应（v1 形状 + attach 附加字段）。 */
export interface JavaAgentInvokeResponse extends JavaInvokeResponse {
  instance_kind?: string
  instance_type?: string
  bean_name?: string
  void?: boolean
  interrupt_sent?: boolean
  terminated?: boolean
}

/** Spring 上下文状态（策略梯子 + 分级支持）。 */
export interface JavaSpringStatus {
  spring_present: boolean
  framework_version?: string | null
  boot_version?: string | null
  contexts: {
    id: string
    class: string
    active: boolean
    startup_date: number
    parent?: string | null
    bean_count: number
  }[]
  capture_state: string
  capture_count?: number
  note?: string
}

/** Spring Bean 条目。 */
export interface JavaBeanInfo {
  name: string
  scope: string
  lazy: boolean
  primary: boolean
  bean_class?: string | null
  /** 单例已实例化（actual_type 可用；未实例化的调用会触发创建）。 */
  instantiated: boolean
  actual_type?: string | null
  definition_type?: string | null
  proxy?: boolean
  proxy_kind?: string
  error?: string
}
