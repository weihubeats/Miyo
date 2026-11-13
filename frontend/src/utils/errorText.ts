/**
 * 错误文案提取（Toast / 日志统一出口）。
 *
 * 历史缺陷：大量调用点写 `e instanceof Error ? e.message : String(e)`——
 * 当拒绝值不是 `Error` 实例（Tauri 早期 reject 的 `{code,message}` 对象、
 * 某些插件 / 桥接层抛出的 `{ message }` 类对象）时，`String(e)` 得到
 * `"[object Object]"`，用户在界面上只看到这串无意义文本，真实原因丢失
 * （AI 测试中心「[object Object]」报错即此形态）。
 *
 * `errorText()` 依次尝试：Error → message 字符串 → 后端 `{code,message}`
 * → 单键对象展开 → JSON 序列化 → `String()` 兜底，保证任何抛出物都给出
 * 可读文案，绝不产出 `[object Object]`。
 */

/** 从任意抛出值中提取可读错误文案（永不返回 `[object Object]`）。 */
export function errorText(e: unknown): string {
  if (e == null) return ''
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message || e.name || 'Error'
  if (typeof e === 'object') {
    const obj = e as Record<string, unknown>
    // 后端 CommandError / ApiErrorBody 形状
    if (typeof obj.message === 'string' && obj.message) {
      const code = typeof obj.code === 'string' && obj.code ? `${obj.code}: ` : ''
      return `${code}${obj.message}`
    }
    // 其他非空字符串字段（error / reason / detail 等）
    for (const key of ['error', 'reason', 'detail', 'msg']) {
      if (typeof obj[key] === 'string' && obj[key]) return obj[key] as string
    }
    try {
      const json = JSON.stringify(e)
      if (json && json !== '{}') return json
    } catch {
      // 循环引用等：落到下一行
    }
  }
  return String(e)
}

/**
 * 提取错误码（后端 `{ code }` 或挂载在 Error 上的 `code`）。
 * 供需要按 code 分支的调用点使用；无码返回 `''`。
 */
export function errorCode(e: unknown): string {
  if (e && typeof e === 'object' && 'code' in e) {
    const code = (e as { code?: unknown }).code
    if (typeof code === 'string') return code
  }
  return ''
}