/**
 * Pre-request 脚本写入的变量名提取（发送前「未定义变量」警示去噪）。
 *
 * 后端 `execute_request` 的生命周期是：先跑 Pre-request 脚本，再用最终变量表
 * （环境/项目/全局合并表 + `pm.environment.set` / `pm.variables.set` 写入项）
 * 渲染 URL / 参数 / 请求头 / 请求体。因此脚本里 `set('timestamp', …)` 造出的
 * `{{timestamp}}` 虽然在合并表里查不到，却会被真实替换——前端若按合并表判
 * 未知就会误报（实证 issue #193）。
 *
 * 这里只做保守的字面量扫描：`pm.environment.set('k'` / `pm.variables.set("k"`
 * 这类直接写 key 的形态才算已知，变量 key、动态拼接一律忽略（宁可多报一次，
 * 不可漏报真未知）。
 */
export function scriptAssignedVars(scripts?: { pre_request?: string | null } | null): string[] {
  const code = scripts?.pre_request ?? ''
  if (!code.trim()) return []
  const out: string[] = []
  const re = /\bpm\s*\.\s*(?:environment|variables)\s*\.\s*set\s*\(\s*(['"])((?:\\.|(?!\1).)*)\1\s*[,)]/g
  for (let m = re.exec(code); m; m = re.exec(code)) {
    const name = m[2]
    if (name && !out.includes(name)) out.push(name)
  }
  return out
}