/**
 * scenarioIO：场景导入 / 导出（JSON 文件，Miyo 自有格式）。
 *
 * 导出把「引用式步骤」物化为内联请求拷贝——脱离当前项目 / 机器后步骤
 * 仍然可执行（快照语义），不依赖接口集合存在；endpoint_id 保留仅作提示。
 * 导入做形状校验（非全量校验：保存时后端会再验 URL / 方法）。
 */
import type {
  Endpoint,
  Scenario,
  ScenarioInlineRequest,
  ScenarioStep,
} from '../types/miyoApi'

/** 导出文件的结构标记与版本（向后兼容留位）。 */
const EXPORT_MAGIC = 'miyo.scenario'
const EXPORT_VERSION = 1

export interface ScenarioExportFile {
  magic: typeof EXPORT_MAGIC
  version: number
  exportedAt: string
  scenario: {
    name: string
    description: string
    category: string
    steps: ScenarioStep[]
    script?: string | null
  }
}

/**
 * 把引用式步骤物化为内联步骤（找不到接口时保留原引用并标记禁用，
 * 导入方能看见「缺配置」而不是静默丢步骤）。
 */
export function inlineStepFromEndpoint(step: ScenarioStep, endpoint?: Endpoint): ScenarioStep {
  if (step.request || !step.endpoint_id) return { ...step }
  if (!endpoint) {
    return { ...step, enabled: false }
  }
  const inline: ScenarioInlineRequest = {
    method: endpoint.method,
    url: endpointUrlTemplate(endpoint),
    spec: JSON.parse(JSON.stringify(endpoint.request)),
  }
  return {
    endpoint_id: step.endpoint_id,
    enabled: step.enabled,
    extract: JSON.parse(JSON.stringify(step.extract ?? [])),
    assertions: JSON.parse(JSON.stringify(step.assertions ?? [])),
    name: step.name ?? endpoint.name,
    request: inline,
  }
}

/** 接口 → URL 模板（base_url 变量 + 路径；路径变量保留 {{var}} 由执行期代入）。 */
function endpointUrlTemplate(ep: Endpoint): string {
  return `{{base_url}}${ep.path.startsWith('/') ? ep.path : `/${ep.path}`}`
}

/** 构造导出 JSON（pretty，2 空格缩进）。 */
export function buildScenarioExport(
  scenario: Scenario,
  endpointOf: (id: string) => Endpoint | undefined,
): string {
  const file: ScenarioExportFile = {
    magic: EXPORT_MAGIC,
    version: EXPORT_VERSION,
    exportedAt: new Date().toISOString(),
    scenario: {
      name: scenario.name,
      description: scenario.description,
      category: scenario.category,
      steps: scenario.steps.map((s) => inlineStepFromEndpoint(s, endpointOf(s.endpoint_id ?? ''))),
      script: scenario.script ?? null,
    },
  }
  return JSON.stringify(file, null, 2)
}

/** 解析导入文件：形状校验 + 归一化为 Scenario（不含 id / 项目归属，由调用方填充）。 */
export function parseScenarioImport(text: string): {
  name: string
  description: string
  category: string
  steps: ScenarioStep[]
  script: string | null
} {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    throw new Error('invalid-json')
  }
  const file = raw as Partial<ScenarioExportFile>
  const body =
    file && file.magic === EXPORT_MAGIC && file.scenario ? file.scenario : (raw as ScenarioExportFile['scenario'])
  if (!body || typeof body.name !== 'string' || !Array.isArray(body.steps)) {
    throw new Error('invalid-shape')
  }
  const steps: ScenarioStep[] = body.steps.map((s, i) => normalizeImportStep(s, i))
  return {
    name: body.name,
    description: typeof body.description === 'string' ? body.description : '',
    category: typeof body.category === 'string' ? body.category : '',
    steps,
    script: typeof body.script === 'string' && body.script.trim() ? body.script : null,
  }
}

/** 导入步骤归一：缺失字段补默认值；endpoint_id 非法时清空（按内联处理）。 */
function normalizeImportStep(s: unknown, index: number): ScenarioStep {
  const raw = (s ?? {}) as Partial<ScenarioStep>
  const enabled = raw.enabled !== false
  const request = raw.request ? normalizeInlineRequest(raw.request) : null
  const endpointId =
    typeof raw.endpoint_id === 'string' && request === null ? raw.endpoint_id : null
  const name = typeof raw.name === 'string' && raw.name.trim() ? raw.name.trim() : null
  if (!request && !endpointId) {
    // 既无引用又无内联：生成占位内联步骤（禁用），导入方可见可修。
    return {
      endpoint_id: null,
      enabled: false,
      extract: raw.extract ?? [],
      assertions: raw.assertions ?? [],
      name: name ?? `#${index + 1}`,
      request: { method: 'GET', url: '', spec: emptySpec() },
    }
  }
  return {
    endpoint_id: endpointId,
    enabled,
    extract: raw.extract ?? [],
    assertions: raw.assertions ?? [],
    name,
    request,
  }
}

function normalizeInlineRequest(r: unknown): ScenarioInlineRequest | null {
  const raw = (r ?? {}) as Partial<ScenarioInlineRequest>
  if (typeof raw.url !== 'string') return null
  return {
    method: typeof raw.method === 'string' && raw.method.trim() ? raw.method.trim().toUpperCase() : 'GET',
    url: raw.url,
    spec: raw.spec ?? emptySpec(),
  }
}

function emptySpec(): ScenarioInlineRequest['spec'] {
  return {
    params: [],
    headers: [],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    timeout_ms: null,
    follow_redirects: true,
    tests: null,
  }
}
