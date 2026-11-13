/**
 * methodTone 单测：徽章/文本语义类映射与未知方法兜底。
 */
import { describe, expect, it } from 'vitest'
import {
  METHOD_BADGE_TONE,
  METHOD_TEXT_TONE,
  METHOD_TONE,
  methodBadgeTone,
  methodTextTone,
  methodTone,
} from '@/utils/methodTone'

describe('methodTone', () => {
  it('常用方法映射到对应语义工具类', () => {
    expect(methodTone('GET')).toContain('text-method-get')
    expect(methodTone('post')).toContain('bg-method-post/10')
    expect(methodTone('Put')).toContain('border-method-put/20')
    expect(methodTone('DELETE')).toContain('text-method-delete')
    expect(methodTone('PATCH')).toContain('text-method-patch')
  })

  it('未知方法兜底中性灰', () => {
    expect(methodTone('BREW')).toBe(METHOD_TONE.options)
    expect(methodTextTone('BREW')).toBe(METHOD_TEXT_TONE.options)
  })

  it('缺值兜底中性灰（旧运行历史无 method 字段，不得崩渲染）', () => {
    expect(methodBadgeTone(undefined)).toBe(METHOD_BADGE_TONE.options)
    expect(methodBadgeTone(null)).toBe(METHOD_BADGE_TONE.options)
    expect(methodTone('')).toBe(METHOD_TONE.options)
  })

  it('纯文本式只含文字色', () => {
    expect(methodTextTone('GET')).toBe('text-method-get')
    // HEAD / OPTIONS 与未知方法一样走中性灰（HEAD 无语义色）
    expect(methodTextTone('HEAD')).toBe('text-method-neutral')
    expect(methodTextTone('GRAPHQL')).toBe('text-method-patch')
  })

  it('映射表键覆盖全部徽章场景', () => {
    for (const m of ['get', 'post', 'put', 'delete', 'patch', 'graphql', 'head', 'options']) {
      expect(METHOD_TONE[m]).toBeTruthy()
    }
  })
})

describe('methodTone：GRPC', () => {
  it('GRPC 徽章 / 文本色走 grpc 青色令牌，未知方法仍兜底中性灰', () => {
    expect(methodTone('GRPC')).toBe('text-method-grpc bg-method-grpc/10 border-method-grpc/20')
    expect(methodTextTone('GRPC')).toBe('text-method-grpc')
    expect(methodTone('grpc')).toBe(methodTone('GRPC'))
  })
})
