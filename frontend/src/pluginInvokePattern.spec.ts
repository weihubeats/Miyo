/**
 * 命令调用命名空间回归测试。
 *
 * miyo 的 247 个命令都注册在 `miyo-tauri` 的插件 `invoke_handler` 里，前端必须用
 * `plugin:miyo|<command>` 调用；裸命令名会被壳侧判为未知命令直接拒绝。
 *
 * 这条规则被违反过一次（v0.0.68 实锤）：`main.ts` 的渲染心跳写成
 * `send('render_heartbeat')`（`send = __TAURI_INTERNALS__.invoke`），Promise 被
 * `.catch` 静默吞掉 → 心跳计数恒为 0 → 渲染守卫把正常窗口判成「画面冻结」，
 * 用户正常使用中突然弹「界面无法刷新」。运行时不报错、单测全绿，只有静态
 * 扫描能拦住，故以 Rust 侧命令清单（build.rs `COMMANDS`，唯一真源）为基准加门禁。
 *
 * 判定：源码里出现某个命令名字面量时，它必须满足其一 ——
 * 1. 前面紧跟 `plugin:miyo|`（已带命名空间，或模板串 `${IPC_PREFIX}` 内的完整前缀）；
 * 2. 作为 API 层封装函数的入参（`call` / `quiet` / `callIpc`，它们内部补前缀）。
 *
 * 注意 `x.invoke(...)` 不算：插件沙箱的 `commands.invoke` 是另一套权限模型。
 */
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

/** 从 Rust 侧权限构建脚本读取命令清单（SSOT，避免在前端维护第二份）。 */
function registeredCommands(): string[] {
  const buildRs = readFileSync(
    join(process.cwd(), '..', 'crates', 'miyo-tauri', 'build.rs'),
    'utf8',
  )
  const block = /const COMMANDS: &\[&str\] = &\[([\s\S]*?)\];/.exec(buildRs)?.[1] ?? ''
  return [...block.matchAll(/"([a-z0-9_]+)"/g)].map((m) => m[1]!)
}

function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) {
      if (name === 'node_modules' || name === 'dist') continue
      out.push(...sourceFiles(p))
      continue
    }
    if (!name.endsWith('.ts') && !name.endsWith('.vue')) continue
    if (name.endsWith('.spec.ts') || name.endsWith('.d.ts')) continue
    out.push(p)
  }
  return out
}

/**
 * 前缀由这些包装函数补上：
 * - `useMiyoApi.ts` 的 `call` / `quiet`（`${PLUGIN}|${command}`）；
 * - `plugins/manager.ts` 的 `callIpc`，以及 `plugins/sdk.ts` 的 `callWithSession`
 *   （插件侧桥接同一套 `plugin:miyo|` 命令，带会话令牌重试）。
 * 泛型参数里可能含 `>`（如 `Record<string, unknown>`），故用 `[^()]*` 而非 `[^>]*`。
 */
const PREFIXING_WRAPPER = /(?:call|quiet|callIpc|callWithSession)(?:<[^()]*>)?\(\s*$/

describe('命令调用命名空间', () => {
  it('build.rs 命令清单可读', () => {
    expect(registeredCommands().length).toBeGreaterThan(200)
  })

  it('裸命令名必须经 API 层封装或带 plugin:miyo| 前缀', () => {
    const commands = registeredCommands()
    const offenders: string[] = []
    for (const file of sourceFiles(join(process.cwd(), 'src'))) {
      // 去掉注释：文档里允许出现裸命令名作示例
      const code = readFileSync(file, 'utf8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*\/\/.*$/gm, '')
      for (const command of commands) {
        const pattern = new RegExp(`(?<![\\w|])['"\`]${command}['"\`]`, 'g')
        for (const match of code.matchAll(pattern)) {
          const before = code.slice(Math.max(0, match.index - 140), match.index)
          if (PREFIXING_WRAPPER.test(before)) continue
          if (/plugin:miyo\|\s*$/.test(before)) continue
          const line = code.slice(0, match.index).split('\n').length
          offenders.push(`${file.replace(process.cwd(), '.')}:${line} ${command}`)
        }
      }
    }
    expect(offenders, '命令必须写成 plugin:miyo|<command>（裸名会被壳侧拒绝）').toEqual([])
  })
})