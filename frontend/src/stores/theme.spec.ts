/**
 * theme store 单测：
 * - 持久化：setMode 写入 localStorage，init 恢复偏好；
 * - 解析：system 跟随 matchMedia，dark/light 直接生效；
 * - DOM 应用：apply 把 data-theme 写入 <html> 并广播 miyo:theme 事件；
 * - 系统监听：system 模式切换 matchMedia 触发主题更新。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useThemeStore, THEME_EVENT, type ThemeMode } from '@/stores/theme'

interface StubMQL {
  matches: boolean
  media: string
  addEventListener(type: string, cb: (e: MediaQueryListEvent) => void): void
  removeEventListener(type: string, cb: (e: MediaQueryListEvent) => void): void
  dispatchChange(next: boolean): void
}

function stubMatchMedia(matches: boolean): StubMQL {
  const listeners = new Set<(e: MediaQueryListEvent) => void>()
  const mql: StubMQL = {
    matches,
    media: '(prefers-color-scheme: dark)',
    addEventListener(_: string, cb: (e: MediaQueryListEvent) => void) {
      listeners.add(cb)
    },
    removeEventListener(_: string, cb: (e: MediaQueryListEvent) => void) {
      listeners.delete(cb)
    },
    dispatchChange(next: boolean) {
      matches = next
      for (const cb of listeners) cb({ matches: next } as MediaQueryListEvent)
    },
  }
  vi.stubGlobal('matchMedia', vi.fn(() => mql))
  return mql
}

function stubLocalStorage(): Storage {
  const map = new Map<string, string>()
  const store = {
    getItem: vi.fn((k: string) => map.get(k) ?? null),
    setItem: vi.fn((k: string, v: string) => map.set(k, v)),
    removeItem: vi.fn((k: string) => map.delete(k)),
    clear: vi.fn(() => map.clear()),
    key: vi.fn((i: number) => [...map.keys()][i] ?? null),
    get length() {
      return map.size
    },
  }
  vi.stubGlobal('localStorage', store)
  return store as unknown as Storage
}

describe('theme store', () => {
  beforeEach(() => {
    stubLocalStorage()
    setActivePinia(createPinia())
    document.documentElement.removeAttribute('data-theme')
    document.documentElement.classList.remove('dark', 'light')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('默认深色并应用到 DOM', () => {
    stubLocalStorage()
    stubMatchMedia(false)
    const store = useThemeStore()
    expect(store.mode).toBe('dark')
    expect(store.resolved).toBe('dark')
    store.init()
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('setMode 切换主题、持久化并写 DOM + 广播事件', () => {
    stubLocalStorage()
    stubMatchMedia(false)
    const store = useThemeStore()
    store.init()
    const spy = vi.fn()
    window.addEventListener(THEME_EVENT, spy)

    store.setMode('light')
    expect(store.resolved).toBe('light')
    expect(localStorage.getItem('miyo.theme.mode')).toBe('light')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(spy).toHaveBeenCalledTimes(1)
    expect((spy.mock.calls[0][0] as CustomEvent).detail.mode).toBe('light')
  })

  it('init 从 localStorage 恢复持久化偏好', () => {
    stubLocalStorage()
    localStorage.setItem('miyo.theme.mode', 'system')
    stubMatchMedia(true)
    const store = useThemeStore()
    store.init()
    expect(store.mode).toBe('system')
    expect(store.resolved).toBe('dark')
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
  })

  it('system 模式监听 matchMedia，系统切换自动同步', () => {
    stubLocalStorage()
    const mql = stubMatchMedia(true)
    const store = useThemeStore()
    store.setMode('system')
    expect(store.resolved).toBe('dark')
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')

    mql.dispatchChange(false)
    expect(store.resolved).toBe('light')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
  })

  it('非 system 模式不跟随系统变化', () => {
    stubLocalStorage()
    const mql = stubMatchMedia(true)
    const store = useThemeStore()
    store.setMode('light')
    expect(store.resolved).toBe('light')

    mql.dispatchChange(true)
    expect(store.resolved).toBe('light')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
  })

  it('非法持久化值回退默认深色', () => {
    stubLocalStorage()
    localStorage.setItem('miyo.theme.mode', 'neon' as ThemeMode)
    stubMatchMedia(false)
    const store = useThemeStore()
    expect(store.mode).toBe('dark')
  })
})
