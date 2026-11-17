/**
 * 自定义主题背景图处理：校验 → 降采样 → 重编码 → 容量约束。
 *
 * 目标是控内存与 localStorage 占用：
 * - 只收 PNG / JPG / WEBP，原始文件 ≤ 10MB；
 * - 解码后最长边压到 1600px（应用是窗口化 UI，更大分辨率无意义）；
 * - 优先重编码为 WebP（q≈0.82），环境不支持 WebP 编码时回退 JPEG；
 * - 编码结果 > 1.5MB 视为压缩失败拒绝入库（防止超大 base64 撑爆 localStorage）。
 *
 * 错误以稳定 code 返回，用户可见文案由 UI 层经 i18n 映射。
 */

export const ALLOWED_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp'] as const
export const MAX_RAW_BYTES = 10 * 1024 * 1024
export const MAX_EDGE = 1600
export const MAX_DATAURL_BYTES = 1.5 * 1024 * 1024

export type ImageProcessErrorCode = 'type' | 'size' | 'decode' | 'compress'

export type ImageProcessResult =
  | { ok: true; dataUrl: string; width: number; height: number }
  | { ok: false; code: ImageProcessErrorCode }

/** dataURL 的近似字节数（base64 有效载荷 ≈ 3/4 长度，扣除 padding）。 */
export function estimateDataUrlBytes(dataUrl: string): number {
  const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1)
  const padding = base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0
  return Math.floor((base64.length * 3) / 4) - padding
}

/** 前置校验（类型 + 原始大小）。通过返回 null，否则返回错误 code。 */
export function validateImageFile(file: { type: string; size: number }): ImageProcessErrorCode | null {
  if (!(ALLOWED_IMAGE_TYPES as readonly string[]).includes(file.type)) return 'type'
  if (file.size > MAX_RAW_BYTES) return 'size'
  return null
}

function decodeBitmap(file: Blob): Promise<ImageBitmap> {
  if (typeof createImageBitmap === 'function') return createImageBitmap(file)
  // 兜底：老 WebView 无 createImageBitmap 时走 <img> 解码
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file)
    const img = new Image()
    img.onload = () => {
      URL.revokeObjectURL(url)
      resolve(img as unknown as ImageBitmap)
    }
    img.onerror = () => {
      URL.revokeObjectURL(url)
      reject(new Error('decode failed'))
    }
    img.src = url
  })
}

function encodeCanvas(canvas: HTMLCanvasElement): string {
  // WebP 优先；不支持 WebP 编码的环境 toDataURL 会静默回退 PNG（体积失控），
  // 此时显式改走 JPEG。
  const webp = canvas.toDataURL('image/webp', 0.82)
  if (webp.startsWith('data:image/webp')) return webp
  return canvas.toDataURL('image/jpeg', 0.85)
}

/** 处理一张背景图：校验、降采样、重编码、容量校验一体完成。 */
export async function processImageFile(file: File): Promise<ImageProcessResult> {
  const pre = validateImageFile(file)
  if (pre) return { ok: false, code: pre }

  let bitmap: ImageBitmap
  try {
    bitmap = await decodeBitmap(file)
  } catch {
    return { ok: false, code: 'decode' }
  }

  const srcW = 'width' in bitmap ? bitmap.width : 0
  const srcH = 'height' in bitmap ? bitmap.height : 0
  if (!srcW || !srcH) return { ok: false, code: 'decode' }

  const scale = Math.min(1, MAX_EDGE / Math.max(srcW, srcH))
  const w = Math.max(1, Math.round(srcW * scale))
  const h = Math.max(1, Math.round(srcH * scale))

  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext('2d')
  if (!ctx) return { ok: false, code: 'compress' }
  ctx.drawImage(bitmap as CanvasImageSource, 0, 0, w, h)
  if ('close' in bitmap && typeof bitmap.close === 'function') bitmap.close()

  const dataUrl = encodeCanvas(canvas)
  if (estimateDataUrlBytes(dataUrl) > MAX_DATAURL_BYTES) return { ok: false, code: 'compress' }

  return { ok: true, dataUrl, width: w, height: h }
}
