/**
 * testCases.ts 单测：请求快照提取 + 用例回填（各 Body 类型 / 容错降级）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { KeyValue, RequestSpec, TestCase } from '@/types/miyoApi'
import { useLocaleStore } from '@/stores/locale'
import {
  applyCaseToRequest,
  bodyContentOf,
  bodyTypeLabel,
  bodyTypeOf,
  caseCategoryLabel,
  formatDuration,
  matchTestCaseKeyword,
  restoreBody,
  snapshotRequest,
  statusTextOf,
  statusToneOf,
} from '@/utils/testCases'

function req(overrides: Partial<RequestSpec> = {}): RequestSpec {
  return {
    params: [{ key: 'page', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    headers: [{ key: 'X-Trace', value: 'on', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    path_variables: [],
    auth: { type: 'none' },
    body: { mode: 'none' },
    active_tab: null,
    timeout_ms: 30000,
    follow_redirects: true,
    tests: null,
    disable_cookies: false,
    ...overrides,
  }
}

describe('formatDuration / statusTextOf / statusToneOf', () => {
  it('耗时：长浮点取整毫秒，≥1s 转秒，空值 -', () => {
    expect(formatDuration(206.91179200000002)).toBe('207ms')
    expect(formatDuration(145)).toBe('145ms')
    expect(formatDuration(1500)).toBe('1.50s')
    expect(formatDuration(60_000)).toBe('60.00s')
    expect(formatDuration(0)).toBe('0ms')
    expect(formatDuration(null)).toBe('-')
    expect(formatDuration(undefined)).toBe('-')
  })

  it('状态文案与色调：2xx 绿 / 4xx 琥珀 / 5xx 玫红 / 3xx 蓝', () => {
    expect(statusTextOf(200)).toBe('OK')
    expect(statusTextOf(500)).toBe('Internal Error')
    expect(statusTextOf(599)).toBe('599')
    expect(statusToneOf(200)).toBe('ok')
    expect(statusToneOf(404)).toBe('warn')
    expect(statusToneOf(500)).toBe('err')
    expect(statusToneOf(302)).toBe('info')
  })
})

describe('bodyTypeOf / bodyContentOf', () => {
  it('各 Body 模式映射到类型标识与文本快照', () => {
    expect(bodyTypeOf({ mode: 'json', raw: '{"a":1}' })).toBe('json')
    expect(bodyTypeOf({ mode: 'text', raw: 'hello' })).toBe('raw')
    expect(bodyTypeOf({ mode: 'urlencoded', fields: [] })).toBe('urlencoded')
    expect(bodyTypeOf({ mode: 'multipart', fields: [] })).toBe('form-data')
    expect(bodyTypeOf({ mode: 'graphql', spec: { query: 'q', variables: '', operation_name: '' } })).toBe('graphql')
    expect(bodyTypeOf({ mode: 'binary', path: '/tmp/a.png' })).toBe('binary')
    expect(bodyTypeOf({ mode: 'none' })).toBe('none')

    expect(bodyContentOf({ mode: 'json', raw: '{"a":1}' })).toBe('{"a":1}')
    expect(bodyContentOf({ mode: 'binary', path: '/tmp/a.png' })).toBe('/tmp/a.png')
    expect(bodyContentOf({ mode: 'none' })).toBe('')
  })

  it('bodyTypeLabel 有可读文案', () => {
    expect(bodyTypeLabel('json')).toBe('JSON')
    expect(bodyTypeLabel('form-data')).toBe('Form-Data')
    expect(bodyTypeLabel('unknown')).toBe('unknown')
  })
})

describe('caseCategoryLabel（分组展示文案）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
    useLocaleStore().setMode('zh')
  })

  it('存库原值 → 展示文案；未知原值原样返回', () => {
    expect(caseCategoryLabel('正向')).toBe('正向')
    expect(caseCategoryLabel('边界值')).toBe('边界值')
    expect(caseCategoryLabel('安全性')).toBe('安全性')
    expect(caseCategoryLabel('未知分组')).toBe('未知分组')
  })

  it('bodyTypeLabel：none 显示「无 Body」', () => {
    expect(bodyTypeLabel('none')).toBe('无 Body')
  })
})

describe('snapshotRequest', () => {
  it('深拷贝 params / headers（后续修改互不影响）', () => {
    const r = req()
    const snap = snapshotRequest(r)
    r.params[0].value = '99'
    r.headers[0].key = 'mutated'
    expect(snap.params[0].value).toBe('1')
    expect(snap.headers[0].key).toBe('X-Trace')
  })
})

describe('restoreBody', () => {
  it('json 保留原文（可回填），raw → text，none → 无 Body', () => {
    expect(restoreBody('json', '{"a":1}')).toEqual({ mode: 'json', raw: '{"a":1}' })
    expect(restoreBody('raw', 'plain')).toEqual({ mode: 'text', raw: 'plain' })
    expect(restoreBody('none', '')).toEqual({ mode: 'none' })
  })

  it('urlencoded / form-data 还原字段', () => {
    const kv = [{ key: 'a', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' }]
    expect(restoreBody('urlencoded', JSON.stringify(kv))).toEqual({ mode: 'urlencoded', fields: kv })
    expect(restoreBody('form-data', JSON.stringify(kv))).toEqual({
      mode: 'multipart',
      fields: [{ key: 'a', value: '1', value_type: 'text', enabled: true }],
    })
  })

  it('graphql / binary 还原', () => {
    expect(restoreBody('graphql', 'query { user }')).toEqual({
      mode: 'graphql',
      spec: { query: 'query { user }', variables: '', operation_name: '' },
    })
    expect(restoreBody('binary', '/tmp/a.png')).toEqual({ mode: 'binary', path: '/tmp/a.png' })
  })

  it('字段 JSON 损坏时降级为空数组，未知类型降级为 raw 文本', () => {
    expect(restoreBody('urlencoded', 'not-json')).toEqual({ mode: 'urlencoded', fields: [] })
    expect(restoreBody('unknown-type', 'x')).toEqual({ mode: 'text', raw: 'x' })
  })
})

describe('applyCaseToRequest', () => {
  it('整体替换 params / headers / body', () => {
    const r = req()
    applyCaseToRequest(r, {
      params: [{ key: 'debug', value: '0', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      headers: [],
      body_type: 'json',
      body_content: '{"amount":100}',
    })
    expect(r.params).toEqual([{ key: 'debug', value: '0', enabled: true, description: '', field_type: 'string', required: true, example: '' }])
    expect(r.headers).toEqual([])
    expect(r.body).toEqual({ mode: 'json', raw: '{"amount":100}' })
  })

  it('回填不污染原快照数据（深拷贝）', () => {
    const snap: {
      params: KeyValue[]
      headers: KeyValue[]
      body_type: string
      body_content: string
    } = {
      params: [{ key: 'a', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      headers: [],
      body_type: 'json',
      body_content: '{}',
    }
    const r = req()
    applyCaseToRequest(r, snap)
    r.params[0].value = 'changed'
    expect(snap.params[0].value).toBe('1')
  })
})

describe('matchTestCaseKeyword', () => {
  function testCase(): TestCase {
    return {
      id: 'c1',
      request_id: 'ep-1',
      name: '内部划转-SGB',
      category: '正向',
      method: 'POST',
      url_path: '/funds/transfer',
      path_variables: [],
      params: [{ key: 'debug', value: '0', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      headers: [{ key: 'X-Trace', value: 'on', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      body_type: 'json',
      body_content: '{"amount":100,"currency":"CNY"}',
      last_run_status: 'Untested',
      created_at: '2026-01-01T00:00:00.000Z',
    }
  }

  it('空关键字恒匹配', () => {
    expect(matchTestCaseKeyword(testCase(), '')).toBe(true)
    expect(matchTestCaseKeyword(testCase(), '   ')).toBe(true)
  })

  it('请求 Body 文本参与匹配（大小写不敏感）', () => {
    expect(matchTestCaseKeyword(testCase(), 'amount')).toBe(true)
    expect(matchTestCaseKeyword(testCase(), 'AMOUNT')).toBe(true)
    expect(matchTestCaseKeyword(testCase(), 'cny')).toBe(true)
    expect(matchTestCaseKeyword(testCase(), 'no-such-body')).toBe(false)
  })

  it('名称 / 方法 / 路径 / 参数键值参与匹配', () => {
    const c = testCase()
    expect(matchTestCaseKeyword(c, '划转')).toBe(true)
    expect(matchTestCaseKeyword(c, 'post')).toBe(true)
    expect(matchTestCaseKeyword(c, '/funds/')).toBe(true)
    expect(matchTestCaseKeyword(c, 'x-trace')).toBe(true)
    expect(matchTestCaseKeyword(c, 'debug')).toBe(true)
    expect(matchTestCaseKeyword(c, 'zzz')).toBe(false)
  })
})
