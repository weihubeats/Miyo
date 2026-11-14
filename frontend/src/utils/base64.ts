/**
 * UTF-8 安全的 base64 编解码（浏览器 btoa/atob 只认 Latin-1，直接编码
 * 中文/emoji 会抛 InvalidCharacterError）。
 *
 * 用途：ws_send 的 binary/ping 帧——后端契约（ws.rs WsSendArgs）要求这两类
 * payload 为 base64，前端必须先把原文按 UTF-8 编成 base64 再发送。
 */

/** UTF-8 字符串 → base64。 */
export function toBase64Utf8(text: string): string {
  const bytes = new TextEncoder().encode(text)
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}

/** base64 → UTF-8 字符串（非法输入抛异常，调用方决定兜底）。 */
export function fromBase64Utf8(b64: string): string {
  const bin = atob(b64)
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0))
  return new TextDecoder().decode(bytes)
}

/** base64 解码失败时回退原文（详情面板展示 binary 帧用，不炸 UI）。 */
export function fromBase64Utf8Safe(b64: string): string {
  try {
    return fromBase64Utf8(b64)
  } catch {
    return b64
  }
}
