import { describe, expect, it } from 'vitest'
import { formatBytes, formatDuration, formatDurationShort } from '@/utils/format'

describe('formatDuration', () => {
  it('空值与 NaN 显示占位符', () => {
    expect(formatDuration(null)).toBe('-')
    expect(formatDuration(undefined)).toBe('-')
    expect(formatDuration(Number.NaN)).toBe('-')
  })

  it('按量级自动选择单位', () => {
    expect(formatDuration(1744)).toBe('1.74 s')
    expect(formatDuration(2000)).toBe('2.00 s')
    expect(formatDuration(0.04)).toBe('40 µs')
    expect(formatDuration(0)).toBe('<1 ms')
  })
})

describe('formatDurationShort', () => {
  it('空值与 NaN 显示占位符', () => {
    expect(formatDurationShort(null)).toBe('-')
    expect(formatDurationShort(Number.NaN)).toBe('-')
  })

  it('ms 取整、秒一位小数（列表紧凑格式）', () => {
    expect(formatDurationShort(362)).toBe('362 ms')
    expect(formatDurationShort(10)).toBe('10 ms')
    expect(formatDurationShort(2830)).toBe('2.8 s')
    expect(formatDurationShort(13_640)).toBe('13.6 s')
    expect(formatDurationShort(0.04)).toBe('40 µs')
    expect(formatDurationShort(0)).toBe('<1 ms')
  })
})

describe('formatBytes', () => {
  it('B / KB / MB 自动单位', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2048)).toBe('2.0 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.00 MB')
  })
})
