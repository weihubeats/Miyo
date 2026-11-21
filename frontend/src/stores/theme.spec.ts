/**
 * theme store 单测（多主题 + 单槽位自定义主题配置模型）：
 * - 持久化：setThemeId/setMode/updateCustom 写入 localStorage `miyo.theme.config`，init 恢复；
 * - 迁移：旧版 `miyo.theme.mode` 三态键（system/dark/light）一次性迁移到新配置；
 * - 解析：system 模式按系统深浅在 lastDark/lastLight 记忆对间切换，manual 固定 current；
 * - DOM 应用：apply 写 data-theme（二值基调）+ data-theme-id（主题身份）并广播事件；
 * - 自定义主题：首选 custom 自动初始化槽位；updateCustom 立即生效并记入记忆对；
 * - 导入：合法配置整体替换，非法 JSON / 结构返回失败且不改变现状。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useThemeStore, THEME_EVENT } from '@/stores/theme'

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
    stubMatchMedia(false)
    setActivePinia(createPinia())
    document.documentElement.removeAttribute('data-theme')
    document.documentElement.removeAttribute('data-theme-id')
    document.documentElement.classList.remove('dark', 'light')
    document.documentElement.style.removeProperty('--bg-app')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('默认石墨深色并应用到 DOM（data-theme + data-theme-id）', () => {
    const store = useThemeStore()
    expect(store.themeId).toBe('dark')
    expect(store.resolved).toBe('dark')
    store.init()
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('setThemeId 切换内置主题、持久化并写 DOM + 广播事件', () => {
    const store = useThemeStore()
    store.init()
    const spy = vi.fn()
    window.addEventListener(THEME_EVENT, spy)

    store.setThemeId('ocean')
    expect(store.themeId).toBe('ocean')
    expect(store.resolved).toBe('dark')
    expect(store.config.current).toBe('ocean')
    expect(store.config.lastDark).toBe('ocean')
    const saved = JSON.parse(localStorage.getItem('miyo.theme.config')!)
    expect(saved.current).toBe('ocean')
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('ocean')
    expect(spy).toHaveBeenCalledTimes(1)
    expect((spy.mock.calls[0][0] as CustomEvent).detail).toEqual({ mode: 'dark', themeId: 'ocean' })

    store.setThemeId('forest')
    expect(store.resolved).toBe('light')
    expect(store.config.lastLight).toBe('forest')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
  })

  it('setMode(system|manual) 持久化', () => {
    const store = useThemeStore()
    store.init()
    store.setMode('system')
    expect(store.mode).toBe('system')
    expect(JSON.parse(localStorage.getItem('miyo.theme.config')!).mode).toBe('system')
  })

  it('init 从 localStorage 恢复持久化配置', () => {
    localStorage.setItem(
      'miyo.theme.config',
      JSON.stringify({ version: 1, mode: 'manual', current: 'sand', lastLight: 'sand', lastDark: 'ocean', custom: null }),
    )
    const store = useThemeStore()
    store.init()
    expect(store.themeId).toBe('sand')
    expect(store.resolved).toBe('light')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('sand')
  })

  it('旧版 miyo.theme.mode 键一次性迁移（dark / light / system），读后删除', () => {
    localStorage.setItem('miyo.theme.mode', 'light')
    let store = useThemeStore()
    store.init()
    expect(store.themeId).toBe('light')
    expect(store.mode).toBe('manual')
    expect(localStorage.getItem('miyo.theme.mode')).toBeNull()
    expect(JSON.parse(localStorage.getItem('miyo.theme.config')!).current).toBe('light')

    setActivePinia(createPinia())
    localStorage.removeItem('miyo.theme.config') // 场景间互不污染（同一 stub 实例）
    localStorage.setItem('miyo.theme.mode', 'system')
    store = useThemeStore()
    store.init()
    expect(store.mode).toBe('system')
    // system 无系统深浅时（stub false）解析为浅色档主题
    expect(store.themeId).toBe('light')

    setActivePinia(createPinia())
    localStorage.removeItem('miyo.theme.config')
    localStorage.setItem('miyo.theme.mode', 'dark')
    store = useThemeStore()
    store.init()
    expect(store.themeId).toBe('dark')
  })

  it('system 模式按系统深浅在记忆对间切换', () => {
    const mql = stubMatchMedia(true)
    const store = useThemeStore()
    store.setThemeId('ocean') // 深色档记忆
    store.setThemeId('lavender') // 浅色档记忆
    store.setMode('system')
    expect(store.themeId).toBe('ocean')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('ocean')

    mql.dispatchChange(false)
    expect(store.themeId).toBe('lavender')
    expect(store.resolved).toBe('light')
    expect(document.documentElement.getAttribute('data-theme-id')).toBe('lavender')
  })

  it('非 system 模式不跟随系统变化', () => {
    const mql = stubMatchMedia(true)
    const store = useThemeStore()
    store.setThemeId('forest')
    expect(store.resolved).toBe('light')
    mql.dispatchChange(true)
    expect(store.resolved).toBe('light')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
  })

  it('非法持久化配置回退默认（保留合法部分而非整体崩）', () => {
    localStorage.setItem('miyo.theme.config', '{"version":2}')
    const store = useThemeStore()
    expect(store.themeId).toBe('dark')
    localStorage.setItem('miyo.theme.config', '{broken json')
    setActivePinia(createPinia())
    expect(useThemeStore().themeId).toBe('dark')
  })

  describe('自定义主题（单槽位）', () => {
    it('首选 custom 自动初始化槽位（继承当前基调）', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('custom')
      expect(store.themeId).toBe('custom')
      expect(store.custom).not.toBeNull()
      expect(store.custom!.base).toBe('dark') // 当前基调是深色
      expect(document.documentElement.getAttribute('data-theme-id')).toBe('custom')
    })

    it('updateCustom 写色板/圆角并立即生效（内联变量优先于样式表）', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('custom')
      store.updateCustom({
        base: 'light',
        colors: { accent: '#123456' },
        radius: 10,
      })
      expect(store.custom!.colors.accent).toBe('#123456')
      const root = document.documentElement
      expect(root.style.getPropertyValue('--primary')).toBe('#123456')
      expect(root.style.getPropertyValue('--radius-lg')).toBe('14px')
      // 记忆对同步：浅色基调记入 lastLight
      expect(store.config.lastLight).toBe('custom')
    })

    it('resetTheme 恢复默认石墨深色并清空自定义槽位', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('custom')
      store.updateCustom({ radius: 16 })
      store.resetTheme()
      expect(store.themeId).toBe('dark')
      expect(store.custom).toBeNull()
      expect(document.documentElement.style.getPropertyValue('--radius-xl')).toBe('')
    })

    it('resetBackground 只重置背景形态', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('custom')
      store.updateCustom({ background: { type: 'color', color: '#abcdef' }, radius: 12 })
      store.resetBackground()
      expect(store.custom!.background.type).toBe('default')
      expect(store.custom!.radius).toBe(12)
    })

    it('密度与字体：updateCustom 写内联 --density/--font-ui，恢复默认档清除内联', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('custom')
      const root = document.documentElement
      store.updateCustom({ density: 'compact', font: 'system' })
      expect(store.custom!.density).toBe('compact')
      expect(store.custom!.font).toBe('system')
      expect(root.style.getPropertyValue('--density')).toBe('0.9')
      expect(root.style.getPropertyValue('--font-ui')).toContain('-apple-system')
      // 回到默认档：内联清除，样式表基准值生效
      store.updateCustom({ density: 'default', font: 'default' })
      expect(root.style.getPropertyValue('--density')).toBe('')
      expect(root.style.getPropertyValue('--font-ui')).toBe('')
    })

    it('旧配置缺 density/font 字段时回退默认值（向后兼容）', () => {
      localStorage.setItem(
        'miyo.theme.config',
        JSON.stringify({
          version: 1,
          mode: 'manual',
          current: 'custom',
          custom: { version: 1, base: 'dark', colors: {}, background: { type: 'default' }, cardOpacity: 1, radius: 6, fontScale: 1 },
        }),
      )
      const store = useThemeStore()
      store.init()
      expect(store.custom!.density).toBe('default')
      expect(store.custom!.font).toBe('default')
    })
  })

  describe('视觉风格（与颜色主题正交）', () => {
    it('旧配置缺 style 字段回退 studio，并写入 <html data-style>', () => {
      localStorage.setItem(
        'miyo.theme.config',
        JSON.stringify({ version: 1, mode: 'manual', current: 'dark' }),
      )
      const store = useThemeStore()
      store.init()
      expect(store.style).toBe('studio')
      expect(document.documentElement.getAttribute('data-style')).toBe('studio')
    })

    it('setStyle 切换三态：持久化 + data-style 实时更新 + 守卫类防闪烁', () => {
      const store = useThemeStore()
      store.init()
      const root = document.documentElement
      store.setStyle('minimal')
      expect(store.style).toBe('minimal')
      expect(root.getAttribute('data-style')).toBe('minimal')
      expect(JSON.parse(localStorage.getItem('miyo.theme.config')!).style).toBe('minimal')
      store.setStyle('soft')
      expect(root.getAttribute('data-style')).toBe('soft')
      store.setStyle('studio')
      expect(root.getAttribute('data-style')).toBe('studio')
    })

    it('非法 style 值不生效（保持当前档）', () => {
      const store = useThemeStore()
      store.init()
      store.setStyle('glass' as never)
      expect(store.style).toBe('studio')
    })

    it('style 与颜色主题独立：切换主题不重置风格', () => {
      const store = useThemeStore()
      store.init()
      store.setStyle('soft')
      store.setThemeId('ocean')
      expect(store.style).toBe('soft')
      expect(store.themeId).toBe('ocean')
      expect(document.documentElement.getAttribute('data-style')).toBe('soft')
      expect(document.documentElement.getAttribute('data-theme-id')).toBe('ocean')
    })

    it('exportConfig / importConfig 往返携带 style；旧导入缺省回退 studio', () => {
      const store = useThemeStore()
      store.init()
      store.setStyle('minimal')
      const exported = store.exportConfig()
      expect(JSON.parse(exported).style).toBe('minimal')
      // 旧版导出（无 style 字段）导入 → studio
      const legacy = JSON.stringify({ version: 1, mode: 'manual', current: 'light' })
      expect(store.importConfig(legacy)).toBe(true)
      expect(store.style).toBe('studio')
    })
  })

  describe('导入 / 导出', () => {
    it('exportConfig → importConfig 往返恢复', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('sand')
      const json = store.exportConfig()

      setActivePinia(createPinia())
      const other = useThemeStore()
      expect(other.importConfig(json)).toBe(true)
      expect(other.themeId).toBe('sand')
    })

    it('非法导入返回 false 且不改变现状；未知主题 id 宽松回退默认', () => {
      const store = useThemeStore()
      store.init()
      store.setThemeId('ocean')
      expect(store.importConfig('not json')).toBe(false)
      expect(store.importConfig('{"version":3}')).toBe(false)
      expect(store.themeId).toBe('ocean')
      // 字段级未知值宽松回退（向前兼容），整体结构（version/形状）严格拒绝
      expect(store.importConfig('{"version":1,"current":"nonexistent"}')).toBe(true)
      expect(store.themeId).toBe('dark')
    })
  })
})
