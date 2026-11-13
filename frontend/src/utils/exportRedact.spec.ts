/**
 * exportRedact：前端导出脱敏单源——键值对 / JSON 嵌套 / URL query /
 * auth 全变体 / 内联 spec 的脱敏与清单登记。
 */
import { describe, expect, it } from 'vitest'
import {
  EXPORT_PLACEHOLDER,
  isSensitiveKeyName,
  redactAuthForExport,
  redactJsonText,
  redactKeyValuePairs,
  redactSpecForExport,
  redactUrlQuery,
} from './exportRedact'

describe('isSensitiveKeyName', () => {
  it('内建词根命中（大小写不敏感）', () => {
    expect(isSensitiveKeyName('Authorization')).toBe(true)
    expect(isSensitiveKeyName('x-api-key')).toBe(true)
    expect(isSensitiveKeyName('AccessToken')).toBe(true)
    expect(isSensitiveKeyName('X-Signature')).toBe(true)
    expect(isSensitiveKeyName('X-Trace-Id')).toBe(false)
    expect(isSensitiveKeyName('baseUrl')).toBe(false)
  })

  it('不收录裸 sign 词根（design / assigned 不误伤）', () => {
    expect(isSensitiveKeyName('assignedTo')).toBe(false)
    expect(isSensitiveKeyName('design')).toBe(false)
  })
})

describe('redactUrlQuery', () => {
  it('敏感参数值替换占位符，非敏感保留', () => {
    expect(redactUrlQuery('https://a.com/p?access_token=sec&page=2')).toBe(
      `https://a.com/p?access_token=${EXPORT_PLACEHOLDER}&page=2`,
    )
    expect(redactUrlQuery('https://a.com/p')).toBe('https://a.com/p')
  })
})

describe('redactKeyValuePairs', () => {
  it('敏感键值替换，其他不动', () => {
    const out = redactKeyValuePairs([
      { key: 'Authorization', value: 'Bearer t' },
      { key: 'X-Trace', value: 'keep' },
      { key: 'X-Empty', value: '' },
    ])
    expect(out[0].value).toBe(EXPORT_PLACEHOLDER)
    expect(out[1].value).toBe('keep')
    expect(out[2].value).toBe('')
  })
})

describe('redactJsonText', () => {
  it('JSON 嵌套敏感字段 + JWT 形态脱敏，非 JSON 原文返回', () => {
    const jwt =
      'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U'
    const out = redactJsonText(
      JSON.stringify({ data: { items: [{ access_token: 't1', name: 'keep' }] }, jwt, count: 3 }),
    )
    const parsed = JSON.parse(out) as Record<string, unknown>
    expect((parsed.data as { items: { access_token: string }[] }).items[0].access_token).toBe(
      EXPORT_PLACEHOLDER,
    )
    expect((parsed.data as { items: { name: string }[] }).items[0].name).toBe('keep')
    expect(parsed.jwt).toBe(EXPORT_PLACEHOLDER)
    expect(parsed.count).toBe(3)
    expect(redactJsonText('plain body text')).toBe('plain body text')
  })
})

describe('redactAuthForExport', () => {
  it('全变体凭据字段脱敏；ApiKey 的 key（头名）保留', () => {
    expect(redactAuthForExport({ type: 'bearer', token: 't1' })).toEqual({
      type: 'bearer',
      token: EXPORT_PLACEHOLDER,
    })
    expect(redactAuthForExport({ type: 'basic', username: 'u', password: 'p' })).toEqual({
      type: 'basic',
      username: 'u',
      password: EXPORT_PLACEHOLDER,
    })
    expect(
      redactAuthForExport({ type: 'apikey', key: 'X-API-Key', value: 'v1', in: 'header' }),
    ).toEqual({ type: 'apikey', key: 'X-API-Key', value: EXPORT_PLACEHOLDER, in: 'header' })
    expect(redactAuthForExport({ type: 'hawk', key_id: 'kid', key: 'hk' })).toEqual({
      type: 'hawk',
      key_id: 'kid',
      key: EXPORT_PLACEHOLDER,
    })
    const aws = redactAuthForExport({
      type: 'awsv4',
      access_key: 'ak',
      secret_key: 'sk',
      region: 'r',
      service: 's',
      session_token: 'st',
    })
    expect(aws).toMatchObject({ secret_key: EXPORT_PLACEHOLDER, session_token: EXPORT_PLACEHOLDER })
    const oauth = redactAuthForExport({
      type: 'oauth2',
      client_id: 'cid',
      client_secret: 'cs',
      auth_url: '',
      token_url: '',
      scope: '',
      redirect_uri: '',
      grant: 'authorization_code',
      token: { access_token: 'at', token_type: 'Bearer', expires_at: '2026-11-12T00:00:00Z' },
    })
    expect(oauth).toMatchObject({ client_secret: EXPORT_PLACEHOLDER, token: undefined })
    expect(redactAuthForExport({ type: 'none' })).toEqual({ type: 'none' })
  })
})

describe('redactSpecForExport', () => {
  it('headers / params / auth / body(json) / url query 脱敏并登记清单', () => {
    const [spec, masked] = redactSpecForExport(
      {
        url: '{{base_url}}/login?access_token=sec',
        params: [{ key: 'page', value: '2' }],
        headers: [{ key: 'Authorization', value: 'Bearer t1' }],
        path_variables: [],
        auth: { type: 'bearer', token: 't2' },
        body: { mode: 'json', raw: '{"password":"pw","keep":1}' },
      },
      'step1',
    )
    expect(spec.headers?.[0].value).toBe(EXPORT_PLACEHOLDER)
    expect(spec.params?.[0].value).toBe('2')
    expect(
      spec.auth?.type === 'bearer' ? spec.auth.token : null,
    ).toBe(EXPORT_PLACEHOLDER)
    expect((spec.body as { raw: string }).raw).not.toContain('pw')
    expect((spec.body as { raw: string }).raw).toContain('keep')
    expect(spec.url).toContain(EXPORT_PLACEHOLDER)
    expect(masked.length).toBeGreaterThanOrEqual(3)
  })

  it('urlencoded / multipart / graphql body 脱敏', () => {
    const [urlenc] = redactSpecForExport(
      { body: { mode: 'urlencoded', fields: [{ key: 'password', value: 'pw' }, { key: 'u', value: 'a' }] } },
      's',
    )
    expect((urlenc.body as { fields: { value: string }[] }).fields[0].value).toBe(EXPORT_PLACEHOLDER)
    expect((urlenc.body as { fields: { value: string }[] }).fields[1].value).toBe('a')

    const [mp] = redactSpecForExport(
      {
        body: {
          mode: 'multipart',
          fields: [
            { key: 'secret', value: 'v', value_type: 'text' },
            { key: 'avatar', value: '/tmp/a.png', value_type: 'file_path' },
          ],
        },
      },
      's',
    )
    expect((mp.body as { fields: { value: string }[] }).fields[0].value).toBe(EXPORT_PLACEHOLDER)
    expect((mp.body as { fields: { value: string }[] }).fields[1].value).toBe('/tmp/a.png')

    const [gql] = redactSpecForExport(
      { body: { mode: 'graphql', spec: { variables: '{"password":"pw"}', query: '{ a }' } } },
      's',
    )
    expect((gql.body as { spec: { variables: string } }).spec.variables).not.toContain('pw')
  })

  it('无敏感值的 spec 返回空清单', () => {
    const [, masked] = redactSpecForExport(
      {
        url: '{{base_url}}/ping',
        headers: [{ key: 'X-Trace', value: 't' }],
        auth: { type: 'none' },
        body: { mode: 'text', raw: 'hello' },
      },
      's',
    )
    expect(masked).toEqual([])
  })
})
