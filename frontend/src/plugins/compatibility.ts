/**
 * 版本兼容性（前端 UX 层预检；真源在 Rust `miyo-plugin::compat`，启用/更新由后端强制）。
 * 严格 x.y.z 逐段比较，与 manifest 校验口径一致。
 */

/** 解析 x.y.z；非法返回 null。 */
export function parseVersion(v: string): [number, number, number] | null {
  const parts = v.split('.')
  if (parts.length !== 3) return null
  const out: number[] = []
  for (const p of parts) {
    if (!p || p.length > 3 || !/^\d+$/.test(p)) return null
    out.push(Number(p))
    if (Number(p) > 999) return null
  }
  return [out[0], out[1], out[2]]
}

/** 宿主版本是否满足插件最低要求（逐段比较，0.0.9 < 0.0.10）。 */
export function isCompatible(minAppVersion: string, currentAppVersion: string): boolean {
  const min = parseVersion(minAppVersion)
  const cur = parseVersion(currentAppVersion)
  if (!min || !cur) return false
  for (let i = 0; i < 3; i++) {
    if (cur[i] !== min[i]) return cur[i] > min[i]
  }
  return true
}

/** SDK 协议版本一致性（宿主支持版本见 SUPPORTED_SDK_VERSION）。 */
export const SUPPORTED_SDK_VERSION = 1

export function isSdkSupported(sdkVersion: number): boolean {
  return sdkVersion === SUPPORTED_SDK_VERSION
}
