#!/usr/bin/env node
/**
 * clean-target.mjs：Cargo target/ 磁盘占用治理（node 跨平台，无外部依赖）。
 *
 * 背景：cargo 永不回收旧产物——deps 里每个历史版本的测试二进制（全量调试信息，
 * 单个 100~400MB）与 incremental 缓存跨轮次无限堆积（本仓实测 target/ 26G）。
 *
 * 用法（package.json 已配快捷键）：
 *   node scripts/clean-target.mjs incremental   # 只清增量缓存（下次构建慢一轮，最安全）
 *   node scripts/clean-target.mjs stale [--days 21]  # 清 target/debug 内超龄文件（默认 21 天）
 *   node scripts/clean-target.mjs full          # 整个 target（三 workspace 全量重编）
 *
 * 说明：
 * - 三个 workspace 经 .cargo/config.toml 共享仓库根 target/，此处只处理根目录一份；
 * - stale 按 mtime 删文件是安全的：cargo 检测到产物缺失会按需重编，最坏多编译几次；
 * - 配合根/两子 workspace 的 [profile.dev] debug="line-tables-only"（见 Cargo.toml），
 *   产物单体积约降 60%，增长速度同步放缓。
 */
import { readdirSync, rmSync, statSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = dirname(fileURLToPath(import.meta.url))
const args = process.argv.slice(2)
const mode = args[0] ?? 'incremental'
const daysArg = args.includes('--days') ? args[args.indexOf('--days') + 1] : args[1]
// 共享 target 在仓库根（.cargo/config.toml target-dir），以脚本位置定位，cwd 无关
const TARGET = resolve(scriptDir, '..', 'target')
const DEBUG = join(TARGET, 'debug')
const DAY_MS = 24 * 60 * 60 * 1000

if (!existsSync(TARGET)) {
  console.log('target/ 不存在，无事可做')
  process.exit(0)
}

let before = 0
try {
  before = statSync(DEBUG).size ?? 0
} catch {
  /* 目录可能不存在 */
}

/** 递归统计目录字节。 */
function dirSize(dir) {
  if (!existsSync(dir)) return 0
  let total = 0
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    const st = statSync(p)
    if (st.isDirectory()) total += dirSize(p)
    else total += st.size
  }
  return total
}

function rmrf(dir) {
  if (!existsSync(dir)) return 0
  const size = dirSize(dir)
  rmSync(dir, { recursive: true, force: true })
  return size
}

/** 按文件 mtime 清扫超龄文件；空目录一并移除。 */
function sweepStale(dir, cutoffMs) {
  let freed = 0
  if (!existsSync(dir)) return 0
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    const st = statSync(p)
    if (st.isDirectory()) {
      freed += sweepStale(p, cutoffMs)
      if (readdirSync(p).length === 0) rmSync(p, { recursive: true, force: true })
    } else if (st.mtimeMs < cutoffMs) {
      freed += st.size
      rmSync(p, { force: true })
    }
  }
  return freed
}

const mb = (n) => `${(n / 1024 / 1024).toFixed(0)}MB`
const now = Date.now()

if (mode === 'incremental') {
  const freed = rmrf(join(DEBUG, 'incremental'))
  console.log(`✓ 已清增量缓存，释放 ${mb(freed)}（下次构建会重预热，最安全的清理档）`)
} else if (mode === 'stale') {
  const days = Number(daysArg ?? 21)
  if (!Number.isFinite(days) || days <= 0) {
    console.error('用法：node scripts/clean-target.mjs stale [--days 21]')
    process.exit(1)
  }
  const cutoff = now - days * DAY_MS
  const freed = ['deps', 'build', '.fingerprint']
    .map((d) => sweepStale(join(DEBUG, d), cutoff))
    .reduce((a, b) => a + b, 0)
  console.log(`✓ 已清扫 ${days} 天前的产物，释放 ${mb(freed)}（缺失产物按需重编）`)
} else if (mode === 'full') {
  const freed = rmrf(TARGET)
  console.log(`✓ 已清空 target/，释放 ${mb(freed)}（三 workspace 下次全量重编）`)
} else {
  console.error('用法：node scripts/clean-target.mjs [incremental|stale [--days N]|full]')
  process.exit(1)
}

const after = existsSync(DEBUG) ? dirSize(DEBUG) : 0
console.log(`target/debug：${mb(before)} → ${mb(after)}`)
