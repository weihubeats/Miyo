/**
 * curlPaste：从粘贴进 Body 的 cURL 命令里抽出请求正文（JSON）。
 *
 * 场景：文档里给的示例往往是整条 `curl -X POST … -d '{"a":1}'`。用户真正要的
 * 是里面那段 body，整条命令塞进 JSON 编辑器只会变成一坨语法错误。这里只做
 * 一件事：把 `-d / --data* / --json` 的载荷取出来，能解析成 JSON 就排版返回。
 *
 * 边界：
 * - 只认正文载荷，不碰 URL / 方法 / header（整条命令的导入在地址栏粘贴路径，
 *   走 `parse_curl_command` 后端解析，不要在这里重复实现）；
 * - `@file` / `@-`（读文件或 stdin）取不到内容，视为没有正文；
 * - 多个 `-d` 按 curl 语义用 `&` 拼接后再解析；
 * - 解析不出 JSON 返回 null（调用方保持原生粘贴）。
 */
import { isCurlCommand } from '@/utils/url'
import { tryPretty } from '@/utils/jsonPaste'

/** 带正文载荷的 curl 参数（含 `--flag=value` 写法）。 */
const DATA_FLAGS = new Set([
  '-d',
  '--data',
  '--data-raw',
  '--data-ascii',
  '--data-binary',
  '--json',
])

/**
 * 抽出 cURL 命令里的 JSON 正文并排版。
 *
 * @returns 美化后的 JSON 文本；不是 cURL、没有正文、正文不是 JSON 时返回 null
 */
export function extractCurlBodyJson(cmd: string): string | null {
  if (!isCurlCommand(cmd)) return null
  const args = splitShellArgs(cmd)
  const payloads: string[] = []
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i]
    if (!arg.startsWith('-')) continue
    const eq = arg.indexOf('=')
    const flag = eq === -1 ? arg : arg.slice(0, eq)
    if (!DATA_FLAGS.has(flag)) continue
    // `--data=…` 自带值；`-d …` 取下一个 token
    const value = eq === -1 ? args[i + 1] : arg.slice(eq + 1)
    if (eq === -1 && value !== undefined) i += 1
    if (value === undefined) continue
    if (value.startsWith('@')) continue // 文件 / stdin：内容不在命令里
    payloads.push(value)
  }
  if (!payloads.length) return null
  // curl 多个 -d 以 `&` 拼接（表单语义）；拼 JSON 片段时其实是无分隔相连，两种都试
  for (const candidate of [payloads.join('&'), payloads.join('')]) {
    const body = candidate.trim()
    if (!body) continue
    const formatted = tryPretty(body)
    if (formatted) return formatted
  }
  return null
}

/**
 * 按 shell 规则切分参数：单引号内原样、双引号内处理 `\"` 与 `\\`、引号外反斜杠
 * 转义下一个字符（含续行 `\⏎`）。
 */
export function splitShellArgs(cmd: string): string[] {
  const out: string[] = []
  let cur = ''
  let started = false
  let quote: '"' | "'" | null = null
  for (let i = 0; i < cmd.length; i += 1) {
    const c = cmd[i]
    if (quote === "'") {
      if (c === "'") quote = null
      else cur += c
      continue
    }
    if (quote === '"') {
      if (c === '\\' && (cmd[i + 1] === '"' || cmd[i + 1] === '\\')) {
        cur += cmd[i + 1]
        i += 1
        continue
      }
      if (c === '"') quote = null
      else cur += c
      continue
    }
    if (c === "'" || c === '"') {
      quote = c
      started = true
      continue
    }
    if (c === '\\') {
      const next = cmd[i + 1]
      if (next === '\n') {
        i += 1 // 续行：反斜杠 + 换行整体丢弃
        continue
      }
      if (next === '\r' && cmd[i + 2] === '\n') {
        i += 2
        continue
      }
      if (next !== undefined) {
        cur += next
        started = true
        i += 1
      }
      continue
    }
    if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
      if (started || cur) {
        out.push(cur)
        cur = ''
        started = false
      }
      continue
    }
    cur += c
    started = true
  }
  if (started || cur) out.push(cur)
  return out
}
