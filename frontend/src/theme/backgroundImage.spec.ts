/**
 * theme/backgroundImage 单测：前置校验（类型/大小）与 dataURL 容量估算。
 * 解码/编码依赖 canvas 与 createImageBitmap（jsdom 不提供），仅覆盖纯逻辑层。
 */
import { describe, expect, it } from 'vitest'
import {
  MAX_RAW_BYTES,
  estimateDataUrlBytes,
  processImageFile,
  validateImageFile,
} from '@/theme/backgroundImage'

describe('theme/backgroundImage', () => {
  it('validateImageFile 校验类型与原始大小', () => {
    expect(validateImageFile({ type: 'image/png', size: 100 })).toBeNull()
    expect(validateImageFile({ type: 'image/jpeg', size: 100 })).toBeNull()
    expect(validateImageFile({ type: 'image/webp', size: 100 })).toBeNull()
    expect(validateImageFile({ type: 'image/gif', size: 100 })).toBe('type')
    expect(validateImageFile({ type: 'image/svg+xml', size: 100 })).toBe('type')
    expect(validateImageFile({ type: 'image/png', size: MAX_RAW_BYTES + 1 })).toBe('size')
    // 大小恰好等于上限：放行
    expect(validateImageFile({ type: 'image/png', size: MAX_RAW_BYTES })).toBeNull()
  })

  it('estimateDataUrlBytes 估算 base64 有效载荷字节数', () => {
    // 'AAAA' → 4 字符 × 3/4 = 3 字节
    expect(estimateDataUrlBytes('data:image/webp;base64,AAAA')).toBe(3)
    expect(estimateDataUrlBytes('data:image/png;base64,AAAAAA==')).toBe(4) // 8 字符含 padding → 4 字节
  })

  it('processImageFile 前置校验失败直接返回错误码', async () => {
    const bad = { type: 'image/gif', size: 1 } as File
    const res = await processImageFile(bad)
    expect(res.ok).toBe(false)
    if (!res.ok) expect(res.code).toBe('type')

    const huge = { type: 'image/png', size: MAX_RAW_BYTES + 1 } as File
    const res2 = await processImageFile(huge)
    expect(res2.ok).toBe(false)
    if (!res2.ok) expect(res2.code).toBe('size')
  })
})
