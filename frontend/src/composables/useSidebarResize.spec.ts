/**
 * useSidebarResize 单测：折叠态切换 + localStorage 持久化（宽度逻辑原行为不动）。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { useSidebarResize } from '@/composables/useSidebarResize'

describe('useSidebarResize 折叠', () => {
  beforeEach(() => {
    localStorage.removeItem('miyo.sidebar-collapsed')
    localStorage.removeItem('miyo.sidebar-width')
  })

  it('默认不折叠；toggle 后折叠并持久化，再 toggle 恢复', () => {
    const s = useSidebarResize()
    expect(s.sidebarCollapsed.value).toBe(false)

    s.toggleSidebarCollapsed()
    expect(s.sidebarCollapsed.value).toBe(true)
    expect(localStorage.getItem('miyo.sidebar-collapsed')).toBe('1')

    // 新实例读取持久化（模拟重启）
    const s2 = useSidebarResize()
    expect(s2.sidebarCollapsed.value).toBe(true)

    s2.toggleSidebarCollapsed()
    expect(s2.sidebarCollapsed.value).toBe(false)
    expect(localStorage.getItem('miyo.sidebar-collapsed')).toBe('0')
  })

  it('存储损坏时回退为不折叠', () => {
    // jsdom 的 localStorage 对非法值返回 null，模拟 via getItem 前置污染无法触发
    // 异常；这里只验证读取路径健壮（值非法时视为未折叠）。
    localStorage.setItem('miyo.sidebar-collapsed', 'yes')
    const s = useSidebarResize()
    expect(s.sidebarCollapsed.value).toBe(false)
  })
})
