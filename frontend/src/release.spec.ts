/**
 * release.mjs 单测：版本解析 / Cargo 版本行替换 / 多文件同步 / 一致性检查。
 * 文件读写用临时目录 fixture，不触碰真实仓库文件。
 */
import { describe, expect, it } from 'vitest'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  applyCargoVersion,
  checkConsistency,
  isValidSemver,
  resolveTargetVersion,
  syncVersions,
  VERSION_FILES,
} from '@/../../scripts/release.mjs'

function makeFixture(version = '0.0.8'): string {
  const root = mkdtempSync(path.join(tmpdir(), 'miyo-release-'))
  const confDir = path.join(root, 'frontend/src-tauri')
  mkdirSync(confDir, { recursive: true })
  writeFileSync(
    path.join(confDir, 'tauri.conf.json'),
    JSON.stringify({ productName: 'Miyo', version }, null, 2) + '\n',
  )
  writeFileSync(
    path.join(root, 'frontend/package.json'),
    JSON.stringify({ name: 'miyo', version }, null, 2) + '\n',
  )
  writeFileSync(
    path.join(confDir, 'Cargo.toml'),
    [
      '[package]',
      `version = "${version}"`,
      '',
      '[dependencies]',
      'serde = { version = "1", features = ["derive"] }',
      'tauri = "2"',
    ].join('\n'),
  )
  return root
}

describe('resolveTargetVersion', () => {
  it('显式 X.Y.Z 直接采用；patch/minor/major 按语义递增', () => {
    expect(resolveTargetVersion('0.0.8', '1.2.3')).toBe('1.2.3')
    expect(resolveTargetVersion('0.0.8', 'patch')).toBe('0.0.9')
    expect(resolveTargetVersion('0.0.8', 'minor')).toBe('0.1.0')
    expect(resolveTargetVersion('0.0.8', 'major')).toBe('1.0.0')
  })

  it('非法参数抛错', () => {
    expect(() => resolveTargetVersion('0.0.8', '')).toThrow()
    expect(() => resolveTargetVersion('0.0.8', 'v1')).toThrow()
    expect(() => resolveTargetVersion('0.0.8', 'beta')).toThrow()
  })
})

describe('isValidSemver / applyCargoVersion', () => {
  it('semver 校验拒绝前导零与残缺段', () => {
    expect(isValidSemver('0.0.8')).toBe(true)
    expect(isValidSemver('01.0.0')).toBe(false)
    expect(isValidSemver('1.2')).toBe(false)
  })

  it('只替换包自身 version 行，依赖版本不受影响', () => {
    const out = applyCargoVersion(
      ['[package]', 'version = "0.0.8"', '', '[dependencies]', 'serde = { version = "1" }'].join('\n'),
      '0.0.9',
    )
    expect(out).toContain('version = "0.0.9"')
    expect(out).toContain('serde = { version = "1" }')
    // 包版本行只出现一次替换
    expect(out.match(/version = "0\.0\.9"/g)).toHaveLength(1)
  })
})

describe('syncVersions / checkConsistency（临时目录往返）', () => {
  it('三个文件全部同步到目标版本，且 conf/pkg 键序不变', () => {
    const root = makeFixture('0.0.8')
    syncVersions(root, '0.0.9')

    const conf = JSON.parse(readFileSync(path.join(root, VERSION_FILES.conf), 'utf8'))
    expect(conf.version).toBe('0.0.9')
    expect(conf.productName).toBe('Miyo')

    const pkg = JSON.parse(readFileSync(path.join(root, VERSION_FILES.pkg), 'utf8'))
    expect(pkg.version).toBe('0.0.9')

    const cargo = readFileSync(path.join(root, VERSION_FILES.cargo), 'utf8')
    expect(cargo).toContain('version = "0.0.9"')

    const result = checkConsistency(root)
    expect(result.ok).toBe(true)
    expect(result.base).toBe('0.0.9')

    rmSync(root, { recursive: true, force: true })
  })

  it('不一致时 checkConsistency 报告 ok=false 与各处版本', () => {
    const root = makeFixture('0.0.8')
    syncVersions(root, '0.0.9')
    // 手动把 package.json 改回旧值，模拟漏改
    const pkgPath = path.join(root, VERSION_FILES.pkg)
    const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'))
    pkg.version = '0.0.8'
    writeFileSync(pkgPath, JSON.stringify(pkg, null, 2))

    const result = checkConsistency(root)
    expect(result.ok).toBe(false)
    expect(result.versions['package.json']).toBe('0.0.8')
    expect(result.versions['tauri.conf.json']).toBe('0.0.9')

    rmSync(root, { recursive: true, force: true })
  })
})
