import { describe, expect, it } from 'vitest'
import type { Environment } from '../types/miyoApi'
import {
  BUILTIN_VARIABLES,
  effectiveVariable,
  envBaseUrl,
  envColorClass,
  environmentVariableMap,
  findUnknownVars,
  isSensitiveVarKey,
  joinBaseUrl,
  normalizeBaseUrl,
  parseVarBatch,
  requestTemplateTexts,
  resolveRequestUrl,
  resolveSensitive,
  resolveVariables,
  validateBaseUrl,
  variableListToMap,
} from './environment'
import type { Endpoint } from '../types/miyoApi'

function mkEnv(overrides: Partial<Environment> = {}): Environment {
  return {
    id: 'e1',
    project_id: 'p1',
    name: '测试环境',
    base_url: '',
    variables: [],
    created_at: '',
    updated_at: '',
    ...overrides,
  }
}

describe('envColorClass', () => {
  it('按名称启发式归类（中英文大小写不敏感）', () => {
    expect(envColorClass('开发环境')).toBe('dev')
    expect(envColorClass('Development')).toBe('dev')
    expect(envColorClass('QA')).toBe('test')
    expect(envColorClass('Staging')).toBe('staging')
    expect(envColorClass('production')).toBe('prod')
    expect(envColorClass('全局')).toBe('global')
    expect(envColorClass('自定义')).toBe('')
  })
})

describe('envBaseUrl', () => {
  it('取环境声明的 Base URL（原始文本，保留 {{变量}}）', () => {
    expect(envBaseUrl(mkEnv({ base_url: 'https://pay.example.com' }))).toBe('https://pay.example.com')
    expect(envBaseUrl(mkEnv({ base_url: ' {{host}} ' }))).toBe('{{host}}')
    expect(envBaseUrl(null)).toBe('')
    expect(envBaseUrl(mkEnv())).toBe('')
  })
})

describe('effectiveVariable / variableListToMap', () => {
  it('本地值优先，其次远程值', () => {
    expect(effectiveVariable({ remote_value: 'r', local_value: 'l' })).toBe('l')
    expect(effectiveVariable({ remote_value: ' r ', local_value: '  ' })).toBe('r')
    expect(effectiveVariable({ remote_value: '', local_value: '' })).toBe('')
  })

  it('覆盖开关三态：显式启用取本地（含空串）、显式停用恒取远程、未标注走旧推断', () => {
    // 未标注（旧数据）：本地非空优先。
    expect(effectiveVariable({ remote_value: 'r', local_value: 'l', local_override: null })).toBe('l')
    expect(effectiveVariable({ remote_value: 'r', local_value: ' ', local_override: undefined })).toBe('r')
    // 显式启用：空串也生效（区分「覆盖为空串」与「未设置覆盖」）。
    expect(effectiveVariable({ remote_value: 'r', local_value: '', local_override: true })).toBe('')
    // 显式停用：本地有值也不生效。
    expect(effectiveVariable({ remote_value: 'r', local_value: 'l', local_override: false })).toBe('r')
  })

  it('扁平注入表：enabled 才注入、本地优先、禁用跳过', () => {
    const map = variableListToMap([
      { key: 'token', remote_value: 'abc', local_value: 'LOCAL', enabled: true },
      { key: 'skipped', remote_value: 'x', local_value: '', enabled: false },
      { key: 'junk', remote_value: 'y', local_value: '', enabled: false },
      { key: '', remote_value: 'y', local_value: '', enabled: true },
      { key: '{{sys}}', remote_value: 'y', local_value: '', enabled: true },
    ])
    expect(map.token).toBe('LOCAL')
    expect(map.skipped).toBeUndefined()
    expect(map.junk).toBeUndefined()
    expect(map['']).toBeUndefined()
    expect(map['{{sys}}']).toBeUndefined()
  })

  it('扁平注入表遵循覆盖三态（与后端 effective_variables 同口径）', () => {
    const map = variableListToMap([
      { key: 'a', remote_value: 'r', local_value: 'l', enabled: true, local_override: false },
      { key: 'b', remote_value: 'r', local_value: '', enabled: true, local_override: true },
      { key: 'c', remote_value: 'r', local_value: 'l', enabled: true, local_override: null },
    ])
    expect(map.a).toBe('r')
    expect(map.b).toBeUndefined() // 显式空串覆盖 → 生效值为空，不注入
    expect(map.c).toBe('l')
  })
})

describe('environmentVariableMap', () => {
  it('自动注入 base_url（环境 Base URL 以 base_url 变量名暴露）', () => {
    const env = mkEnv({
      base_url: 'https://pay.example.com',
      variables: [
        { key: 'token', remote_value: 'abc', local_value: 'LOCAL', enabled: true, description: null },
        { key: 'skipped', remote_value: 'x', local_value: '', enabled: false, description: null },
      ],
    })
    const map = environmentVariableMap(env)
    expect(map.token).toBe('LOCAL')
    expect(map.skipped).toBeUndefined()
    expect(map.base_url).toBe('https://pay.example.com')
  })

  it('已显式定义 base_url 变量时不覆盖', () => {
    const env = mkEnv({
      base_url: 'https://env.example.com',
      variables: [
        { key: 'base_url', remote_value: 'https://override.example.com', local_value: '', enabled: true, description: null },
      ],
    })
    expect(environmentVariableMap(env).base_url).toBe('https://override.example.com')
  })

  it('空环境仅无注入', () => {
    expect(environmentVariableMap(mkEnv())).toEqual({})
    expect(environmentVariableMap(null)).toEqual({})
  })
})

describe('normalizeBaseUrl', () => {
  it('去掉尾部斜杠但保留协议本身', () => {
    expect(normalizeBaseUrl('https://x.com/')).toBe('https://x.com')
    expect(normalizeBaseUrl('https://x.com///')).toBe('https://x.com')
    expect(normalizeBaseUrl('https://')).toBe('https://')
  })
})

describe('joinBaseUrl', () => {
  it('相对路径拼基址、去双斜杠、完整 URL 直用', () => {
    expect(joinBaseUrl('https://x.com', '/users')).toBe('https://x.com/users')
    expect(joinBaseUrl('https://x.com/', 'users')).toBe('https://x.com/users')
    expect(joinBaseUrl('', '/users')).toBe('/users')
    expect(joinBaseUrl('https://x.com/', 'https://full.example.com/a')).toBe('https://full.example.com/a')
  })
})

describe('resolveRequestUrl（请求拼接核心）', () => {
  it('环境 Base URL + 相对路径', () => {
    const env = mkEnv({ base_url: 'https://pay.example.com' })
    const r = resolveRequestUrl(env, '/orders')
    expect(r).toMatchObject({ url: 'https://pay.example.com/orders' })
  })

  it('完整 URL 路径直用，不拼基址', () => {
    const env = mkEnv({ base_url: 'https://pay.example.com' })
    const r = resolveRequestUrl(env, 'https://full.example.com/a')
    expect(r).toMatchObject({ url: 'https://full.example.com/a' })
  })

  it('无基址环境：仅返回路径', () => {
    const r = resolveRequestUrl(mkEnv(), '/users')
    expect(r).toMatchObject({ url: '/users' })
  })

  it('基址中的 {{变量}} 以环境变量表解析', () => {
    const env = mkEnv({
      base_url: '{{host}}',
      variables: [
        { key: 'host', remote_value: 'https://dev.example.com', local_value: '', enabled: true, description: null },
      ],
    })
    expect(resolveRequestUrl(mkEnv({ base_url: '{{host}}' }), '/ping').url).toBe('{{host}}/ping')
    const r = resolveRequestUrl(env, '/ping')
    expect(r.url).toBe('https://dev.example.com/ping')
  })

  it('调用方变量参与基址解析（extraVars；环境变量优先）', () => {
    const env = mkEnv({ base_url: '{{host}}' })
    const r = resolveRequestUrl(env, '/ping', { host: 'https://extra.example.com' })
    expect(r.url).toBe('https://extra.example.com/ping')
    const envWithHost = mkEnv({
      base_url: '{{host}}',
      variables: [
        { key: 'host', remote_value: 'https://env.example.com', local_value: '', enabled: true, description: null },
      ],
    })
    expect(resolveRequestUrl(envWithHost, '/ping', { host: 'https://extra.example.com' }).url).toBe('https://env.example.com/ping')
  })

  it('路径以斜杠开头不产生双斜杠', () => {
    const r = resolveRequestUrl(mkEnv({ base_url: 'https://acq.example.com' }), '/')
    expect(r.url).toBe('https://acq.example.com/')
  })
})

describe('resolveVariables', () => {
  const vars = { base: '{{host}}/api', host: 'https://x.com', empty: '' }

  it('递归解析已知变量', () => {
    expect(resolveVariables('{{base}}/posts', vars)).toBe('https://x.com/api/posts')
  })

  it('未知或空值变量原样保留', () => {
    expect(resolveVariables('{{nope}}', vars)).toBe('{{nope}}')
    expect(resolveVariables('{{empty}}', vars)).toBe('{{empty}}')
  })

  it('循环引用在深度上限处停止', () => {
    const cyclic = { a: '{{b}}', b: '{{a}}' }
    const out = resolveVariables('{{a}}', cyclic)
    expect(out).toContain('{{')
  })
})

describe('requestTemplateTexts / findUnknownVars（发送前未定义变量扫描）', () => {
  const ep = (body: Endpoint['request']['body'], headers: Endpoint['request']['headers'] = [], params: Endpoint['request']['params'] = []): Endpoint => {
    const now = '2026-01-01T00:00:00.000Z'
    return {
      id: 'ep-1',
      project_id: 'p-1',
      folder_id: null,
      name: 'x',
      method: 'POST',
      path: '/x',
      description: '',
      status: 'developing',
      sort_order: 0,
      request: {
        params,
        headers,
        path_variables: [],
        auth: { type: 'none' },
        body,
        active_tab: null,
        timeout_ms: null,
        follow_redirects: true,
        tests: null,
        disable_cookies: false,
        body_docs: {},
      },
      created_at: now,
      updated_at: now,
    }
  }

  it('扫描 URL / 头 / 参数 / Body 各模式模板文本', () => {
    const texts = requestTemplateTexts(
      ep(
        { mode: 'json', raw: '{"token":"{{tok}}"}' },
        [{ key: 'Authorization', value: 'Bearer {{auth}}', enabled: true, description: '' }],
        [{ key: '{{qk}}', value: '1', enabled: true, description: '' }],
      ),
      '/api/{{ver}}/login',
    )
    expect(texts.some((t) => t.includes('{{ver}}'))).toBe(true)
    expect(texts.some((t) => t.includes('{{tok}}'))).toBe(true)
    expect(texts.some((t) => t.includes('{{auth}}'))).toBe(true)
    expect(texts.some((t) => t.includes('{{qk}}'))).toBe(true)
  })

  it('未定义 = 引用了但不在已知池（含内置变量豁免），保序去重', () => {
    const known = new Set<string>(['ver', ...BUILTIN_VARIABLES])
    const unknown = findUnknownVars(
      ['/api/{{ver}}/u?tok={{tok}}', '{{}}', '{{tok}}', '{{}}'],
      known,
    )
    expect(unknown).toEqual(['tok'])
  })
})

describe('isSensitiveVarKey（敏感变量名启发式）', () => {
  it('命中 token / secret / password / api_key / authorization 等词根', () => {
    expect(isSensitiveVarKey('access_token')).toBe(true)
    expect(isSensitiveVarKey('CLIENT_SECRET')).toBe(true)
    expect(isSensitiveVarKey('db_password')).toBe(true)
    expect(isSensitiveVarKey('api-key')).toBe(true)
    expect(isSensitiveVarKey('Authorization')).toBe(true)
    expect(isSensitiveVarKey('session_id')).toBe(true)
  })

  it('普通变量名不误伤', () => {
    expect(isSensitiveVarKey('base_url')).toBe(false)
    expect(isSensitiveVarKey('page_size')).toBe(false)
    expect(isSensitiveVarKey('user_name')).toBe(false)
    expect(isSensitiveVarKey('')).toBe(false)
    expect(isSensitiveVarKey(null)).toBe(false)
  })
})

describe('resolveSensitive（显式标注优先，未标注回退启发式）', () => {
  it('三态：显式 true/false 生效，未标注按键名启发式', () => {
    expect(resolveSensitive({ key: 'page_size', sensitive: true })).toBe(true)
    // 显式非敏感可豁免键名误报
    expect(resolveSensitive({ key: 'my_token', sensitive: false })).toBe(false)
    expect(resolveSensitive({ key: 'my_token', sensitive: null })).toBe(true)
    expect(resolveSensitive({ key: 'my_token', sensitive: undefined })).toBe(true)
    expect(resolveSensitive({ key: 'page_size', sensitive: null })).toBe(false)
  })
})

describe('validateBaseUrl（Base URL 格式校验）', () => {
  it('空值 / http(s) / {{变量}} 合法，其余返回错误键', () => {
    expect(validateBaseUrl('')).toBeNull()
    expect(validateBaseUrl('   ')).toBeNull()
    expect(validateBaseUrl('https://api.example.com')).toBeNull()
    expect(validateBaseUrl('HTTP://api.example.com')).toBeNull()
    expect(validateBaseUrl('{{host}}')).toBeNull()
    expect(validateBaseUrl('api.example.com')).toBe('envmgr.baseUrlInvalid')
    expect(validateBaseUrl('localhost:8080')).toBe('envmgr.baseUrlInvalid')
    expect(validateBaseUrl('ftp://x.com')).toBe('envmgr.baseUrlInvalid')
  })
})

describe('parseVarBatch（批量导入解析）', () => {
  it('JSON 对象形态', () => {
    const { rows, invalid } = parseVarBatch('{"host": "https://x.com", "port": 8080}')
    expect(rows).toEqual([
      { key: 'host', remote_value: 'https://x.com' },
      { key: 'port', remote_value: '8080' },
    ])
    expect(invalid).toEqual([])
  })

  it('k=v / k: v 行形态（含 trim 与空行跳过）', () => {
    const { rows, invalid } = parseVarBatch('host=https://x.com\n port : 8080 \n\n')
    expect(rows).toEqual([
      { key: 'host', remote_value: 'https://x.com' },
      { key: 'port', remote_value: '8080' },
    ])
    expect(invalid).toEqual([])
  })

  it('空 key / {{ }} / $ 开头 / 无分隔行进 invalid；重复 key 后者覆盖', () => {
    const { rows, invalid } = parseVarBatch('=v\n{{sys}}=x\n$key=y\nnoDelim\nhost=a\nhost=b')
    expect(rows).toEqual([{ key: 'host', remote_value: 'b' }])
    expect(invalid).toEqual(['=v', '{{sys}}=x', '$key=y', 'noDelim'])
  })

  it('非法 JSON 落入行解析并进 invalid', () => {
    const { rows, invalid } = parseVarBatch('{not json')
    expect(rows).toEqual([])
    expect(invalid.length).toBeGreaterThan(0)
  })
})
