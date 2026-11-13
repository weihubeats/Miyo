import { describe, expect, it } from 'vitest'

import { applyPathVars, extractPathVarNames } from '@/utils/pathVariables'

/**
 * 路径变量公共逻辑回归。
 *
 * 这两支函数是「用例保存 / 用例运行 / 调试页发送 / OpenAPI 导入后的展示」
 * 共用的单一实现，与后端 `miyo_core::util::{extract_path_variables,
 * replace_path_variables}` 语义对齐。旧代码里前端有三份拷贝（PathVariablesPanel、
 * EndpointEditor、workspace store），改一处漏两处——故集中在此加护栏。
 */

describe('extractPathVarNames', () => {
  it('识别 {id} 花括号写法', () => {
    expect(extractPathVarNames('/users/{id}')).toEqual(['id'])
    expect(extractPathVarNames('/orgs/{org}/repos/{repo}')).toEqual(['org', 'repo'])
  })

  it('识别 {{id}} 本应用写法且不被拆开', () => {
    expect(extractPathVarNames('/users/{{id}}')).toEqual(['id'])
    expect(extractPathVarNames('/a/{{x}}/b/{y}')).toEqual(['x', 'y'])
  })

  it('识别 :id Postman 写法', () => {
    expect(extractPathVarNames('/users/:id')).toEqual(['id'])
    expect(extractPathVarNames('/orgs/:org/repos/:repo')).toEqual(['org', 'repo'])
  })

  it('按出现顺序去重', () => {
    expect(extractPathVarNames('/{b}/{a}/{b}')).toEqual(['b', 'a'])
  })

  it('忽略 origin 的端口与 query/hash', () => {
    expect(extractPathVarNames('https://api.x.com/users/{id}')).toEqual(['id'])
    expect(extractPathVarNames('https://api.x.com:8443/users/{id}?a=:b#c')).toEqual(['id'])
  })

  it('无变量时返回空数组', () => {
    expect(extractPathVarNames('/users/list')).toEqual([])
    expect(extractPathVarNames('')).toEqual([])
    expect(extractPathVarNames('/')).toEqual([])
  })

  it('`:name` 需在路径段开头', () => {
    expect(extractPathVarNames('/items/a:b')).toEqual([])
    expect(extractPathVarNames('/items/:id')).toEqual(['id'])
  })
})

describe('applyPathVars', () => {
  it('代入 {id} 与 {{id}}', () => {
    expect(applyPathVars('/users/{id}', [{ key: 'id', value: '10', enabled: true }])).toBe(
      '/users/10',
    )
    expect(applyPathVars('/users/{{id}}', [{ key: 'id', value: '10', enabled: true }])).toBe(
      '/users/10',
    )
  })

  it('长 key 先替换避免前缀冲突', () => {
    const out = applyPathVars('/x/{id}/{userId}', [
      { key: 'id', value: 'A', enabled: true },
      { key: 'userId', value: 'B', enabled: true },
    ])
    expect(out).toBe('/x/A/B')
  })

  it('跳过未启用或空取值的行（路径原样送出）', () => {
    expect(applyPathVars('/users/{id}', [{ key: 'id', value: '', enabled: true }])).toBe(
      '/users/{id}',
    )
    expect(applyPathVars('/users/{id}', [{ key: 'id', value: '10', enabled: false }])).toBe(
      '/users/{id}',
    )
    expect(applyPathVars('/users/{id}', undefined)).toBe('/users/{id}')
  })

  it('同名前缀不误伤（id 不应命中 userId 的花括号）', () => {
    // key=id 时只应替换 `{id}`，`{userId}` 由 key=userId 的行负责。
    const out = applyPathVars('/u/{userId}', [{ key: 'id', value: 'X', enabled: true }])
    expect(out).toBe('/u/{userId}')
  })
})