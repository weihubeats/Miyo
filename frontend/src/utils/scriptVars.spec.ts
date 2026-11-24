import { describe, expect, it } from 'vitest'
import { scriptAssignedVars } from './scriptVars'
import { findUnknownVars } from './environment'

describe('scriptAssignedVars', () => {
  it('提取 pm.environment.set / pm.variables.set 的字面量 key', () => {
    const names = scriptAssignedVars({
      pre_request: `
        var ts = Date.now();
        pm.environment.set('timestamp', String(ts));
        pm.variables.set("trace", 'x');
        pm.environment.set('timestamp', '2');
      `,
    })
    expect(names).toEqual(['timestamp', 'trace'])
  })

  it('允许调用点带空白', () => {
    expect(scriptAssignedVars({ pre_request: `pm . environment . set ( 'tok' , v )` })).toEqual([
      'tok',
    ])
  })

  it('变量 key / 动态拼接不提取（保守：仍按未知处理）', () => {
    const names = scriptAssignedVars({
      pre_request: `
        pm.environment.set(name, 'x');
        pm.environment.set('prefix_' + i, 'y');
        pm.environment.get('read_only');
      `,
    })
    expect(names).toEqual([])
  })

  it('空脚本 / 缺省 scripts 返回空表', () => {
    expect(scriptAssignedVars(null)).toEqual([])
    expect(scriptAssignedVars({ pre_request: '   ' })).toEqual([])
  })

  it('去掉脚本写入项后不再误报未知变量', () => {
    const code = `pm.environment.set('timestamp', Date.now());`
    const known = new Set(['base_url', ...scriptAssignedVars({ pre_request: code })])
    expect(findUnknownVars(['http://h/api?t={{timestamp}}'], known)).toEqual([])
    expect(findUnknownVars(['http://h/api?t={{timestamp}}&u={{other}}'], known)).toEqual(['other'])
  })
})