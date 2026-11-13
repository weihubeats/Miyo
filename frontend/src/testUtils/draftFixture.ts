import type { AuthSpec, BodySpec, Endpoint, HttpMethod, KeyValue } from '../types/miyoApi'

/** 构造一个最小可用的接口草稿（含请求配置），供组件测试使用。 */
export function makeDraft(overrides: Partial<Endpoint> = {}): Endpoint {
  return {
    id: 'ep-test-1',
    project_id: 'proj-test-1',
    folder_id: null,
    name: '测试接口',
    method: 'GET',
    path: '/users',
    description: '',
    status: 'designing',
    sort_order: 0,
    request: {
      params: [],
      headers: [{ key: 'X-Token', value: 'abc', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      path_variables: [],
      auth: { type: 'bearer', token: 'tok123' },
      body: { mode: 'none' },
      timeout_ms: 30000,
      follow_redirects: true,
      tests: null,
      body_docs: {},
  disable_cookies: false,
    },
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
    ...overrides,
  }
}

/** codegen 组件公共入参（CodePanel / CodeExportDialog / CodeExportMenu 解耦 Endpoint 后的 props 形状）。 */
export interface CodegenInputs {
  method: HttpMethod
  url: string
  headers: KeyValue[]
  body: BodySpec
  auth: AuthSpec
}

/** 基于 makeDraft 构造 codegen 入参（可按字段覆盖），供导出组件测试使用。 */
export function makeCodegenInputs(overrides: Partial<CodegenInputs> = {}): CodegenInputs {
  const d = makeDraft()
  return {
    method: d.method,
    url: 'https://api.example.com',
    headers: d.request.headers,
    body: d.request.body,
    auth: d.request.auth,
    ...overrides,
  }
}