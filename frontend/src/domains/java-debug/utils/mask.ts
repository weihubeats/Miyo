/**
 * 敏感参数脱敏：参数名 / JSON 键命中敏感词即把值替换为掩码。
 *
 * 只做展示层脱敏（结果面板 / 历史抽屉），落库原始值不动——
 * 审计需要完整数据，脱敏是「人不该一眼看到密码」的展示边界。
 */

/** 敏感参数名关键词（小写不区分；password / secret / token / 凭据类）。 */
const SENSITIVE_NAME_PATTERN =
  /password|passwd|pwd|secret|token|credential|authorization|auth\b|api_?key|access_?key|private_?key/i

/** 是否敏感参数名。 */
export function isSensitiveName(name: string): boolean {
  return SENSITIVE_NAME_PATTERN.test(name)
}

const MASK = '******'

/** 递归掩码：对象键命中敏感词替换值；数组逐项处理。原始值不变（返回新结构）。 */
export function maskValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(maskValue)
  }
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = isSensitiveName(k) ? MASK : maskValue(v)
    }
    return out
  }
  return value
}

/**
 * 掩码一组位置参数：数组按位置对应参数名，敏感形参整值掩码；
 * 对象参数再递归按键掩码。`paramNames` 缺省时只做对象键掩码。
 */
export function maskArgs(args: unknown, paramNames?: string[]): unknown {
  if (Array.isArray(args) && paramNames?.length) {
    return args.map((v, i) => {
      if (paramNames[i] && isSensitiveName(paramNames[i])) return MASK
      return maskValue(v)
    })
  }
  return maskValue(args)
}
