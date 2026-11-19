/**
 * 权限目录（前端展示镜像）单测：High 风险筛选——安装 / 更新确认页
 * 「High 单独确认」开关的数据来源。目录条目与 Rust PERMISSIONS 一一对应，
 * 未知权限不算 High（安装端会直接拒绝）。
 */
import { describe, expect, it } from 'vitest'
import { highRiskPermissions, lookupPermission } from '@/plugins/permissions'

describe('highRiskPermissions', () => {
  it('筛出 High 风险声明（java / runtime 家族）', () => {
    const out = highRiskPermissions([
      'std.storage',
      'std.log',
      'runtime.process.start',
      'java.method.invoke',
    ])
    expect(out).toEqual(['runtime.process.start', 'java.method.invoke'])
  })

  it('纯 Low / Medium 声明返回空数组（无需单独确认）', () => {
    expect(highRiskPermissions(['std.storage', 'std.env.read', 'java.process.discover'])).toEqual(
      [],
    )
    expect(highRiskPermissions([])).toEqual([])
  })

  it('未知权限不算 High（保持声明顺序；安装端另有目录校验拒绝）', () => {
    expect(highRiskPermissions(['std.not.exist', 'java.class.inspect'])).toEqual([
      'java.class.inspect',
    ])
  })

  it('聚合兼容项 java.debug 归 High（存量行展开语义不变）', () => {
    expect(highRiskPermissions(['java.debug'])).toEqual(['java.debug'])
  })

  it('目录风险等级与前端类型口径一致（抽查）', () => {
    expect(lookupPermission('java.method.invoke')?.risk).toBe('high')
    expect(lookupPermission('runtime.process.stop')?.risk).toBe('medium')
    expect(lookupPermission('std.storage')?.risk).toBe('low')
  })
})
