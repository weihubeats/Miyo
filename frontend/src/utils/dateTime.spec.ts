/**
 * dateTime 单测：语言标签解析与时间格式（跟随 locale store，不写死 zh-CN）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { formatDateTime, formatTime, localeTag } from '@/utils/dateTime'
import { useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('dateTime utils', () => {
  it('system 模式跟随 navigator.language', () => {
    vi.spyOn(window.navigator, 'language', 'get').mockReturnValue('en-US')
    expect(localeTag()).toBe('en-US')
    vi.spyOn(window.navigator, 'language', 'get').mockReturnValue('zh-CN')
    expect(localeTag()).toBe('zh-CN')
  })

  it('显式模式映射 zh-CN / en-US', () => {
    const store = useLocaleStore()
    store.setMode('zh')
    expect(localeTag()).toBe('zh-CN')
    store.setMode('en')
    expect(localeTag()).toBe('en-US')
  })

  it('formatTime 默认 24 小时制，formatDateTime 带日期', () => {
    useLocaleStore().setMode('zh')
    const d = new Date(2026, 0, 2, 3, 4, 5)
    expect(formatTime(d)).toBe('03:04:05')
    expect(formatDateTime(d)).toContain('2026')
  })

  it('接受 ISO 字符串与毫秒时间戳', () => {
    useLocaleStore().setMode('en')
    const iso = '2026-01-02T03:04:05.000Z'
    expect(formatDateTime(iso)).toContain('2026')
    expect(formatTime(Date.parse(iso))).toMatch(/\d{2}:\d{2}:\d{2}/)
  })
})
