/**
 * theme.ts：全局主题状态机（多内置主题 + 单槽位自定义主题）。
 *
 * - config：持久化主题配置（localStorage `miyo.theme.config`），含模式
 *   （system 跟随系统深浅 / manual 固定）、生效主题 id、深浅记忆对、自定义配置；
 * - themeId：实际生效主题（system 模式按系统深浅在 lastDark/lastLight 间切换）；
 * - resolved：生效主题的深浅基调（dark|light）。DOM 约定：<html data-theme> 恒为
 *   二值基调（Tailwind dark: 变体 / CodeMirror 等既有消费点不变），主题身份写
 *   <html data-theme-id>，自定义主题差异由 theme/apply.ts 写内联 CSS 变量表达；
 * - init：启动时同步读取持久化配置（含旧版 `miyo.theme.mode` 三态键的一次性
 *   迁移）并立即生效，避免首屏闪烁（FOUC）；
 * - apply：写 DOM + 广播 `miyo:theme`（detail 含 mode 与 themeId）。切换瞬间挂
 *   `theme-switching` 守卫类禁用全局过渡（双 rAF 后移除），防止颜色渐变闪烁。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import type { CustomThemeConfig, ThemeBase, ThemeConfigV1, ThemeEventDetail, ThemeId, ThemeMode } from '@/theme/types'
import { DEFAULT_CONFIG, DEFAULT_CUSTOM, isThemeId, parseThemeConfig, themeBase } from '@/theme/presets'
import { applyCustomTheme } from '@/theme/apply'

const STORAGE_KEY = 'miyo.theme.config'
/** 旧版三态键（system|dark|light）：仅在无新配置时读取一次并迁移。 */
const LEGACY_KEY = 'miyo.theme.mode'

/** 全局主题切换事件（CodeMirror 等编辑器监听）。 */
export const THEME_EVENT = 'miyo:theme'

export type { CustomThemeConfig, ThemeBase, ThemeConfigV1, ThemeId, ThemeMode }

function systemPrefersDark(): boolean {
  try {
    return window.matchMedia('(prefers-color-scheme: dark)').matches
  } catch {
    return true
  }
}

function persist(config: ThemeConfigV1): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config))
  } catch {
    // 存储不可用（隐私模式等）：仅本次会话生效
  }
}

/** 旧版三态键 → 新配置的一次性迁移（读后即删）。 */
function migrateLegacyMode(systemDark: boolean): ThemeConfigV1 | null {
  try {
    const legacy = localStorage.getItem(LEGACY_KEY)
    if (!legacy) return null
    localStorage.removeItem(LEGACY_KEY)
    const next: ThemeConfigV1 = { ...DEFAULT_CONFIG }
    if (legacy === 'system') {
      next.mode = 'system'
      next.current = systemDark ? 'dark' : 'light'
    } else if (legacy === 'light') {
      next.current = 'light'
    } else if (legacy !== 'dark') {
      return null // 非法旧值：整体回退默认
    }
    return next
  } catch {
    return null
  }
}

function loadConfig(systemDark: boolean): ThemeConfigV1 {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = parseThemeConfig(JSON.parse(raw))
      if (parsed) return parsed
    }
  } catch {
    // JSON 损坏等：走迁移 / 默认
  }
  return migrateLegacyMode(systemDark) ?? { ...DEFAULT_CONFIG, custom: null }
}

function scheduleGuardRemoval(root: HTMLElement): void {
  // 双 rAF：保证守卫类至少覆盖一次完整渲染帧，过渡建立前已被禁用
  requestAnimationFrame(() => {
    requestAnimationFrame(() => root.classList.remove('theme-switching'))
  })
}

/** 把生效主题写到 <html>（基调 + 身份 + 内联自定义覆盖）并广播编辑器事件。 */
function applyToDom(base: ThemeBase, themeId: ThemeId, custom: CustomThemeConfig | null): void {
  const root = document.documentElement
  root.classList.add('theme-switching')
  root.setAttribute('data-theme', base)
  root.setAttribute('data-theme-id', themeId)
  root.classList.toggle('dark', base === 'dark')
  root.classList.toggle('light', base === 'light')
  applyCustomTheme(root, themeId === 'custom' ? custom : null)
  scheduleGuardRemoval(root)
  window.dispatchEvent(new CustomEvent<ThemeEventDetail>(THEME_EVENT, { detail: { mode: base, themeId } }))
}

export const useThemeStore = defineStore('theme', () => {
  const systemDark = ref(systemPrefersDark())
  const config = ref<ThemeConfigV1>(loadConfig(systemDark.value))

  // 防御：custom 槽位缺失时不得指向 custom
  if (config.value.current === 'custom' && !config.value.custom) config.value.current = 'dark'

  // 旧键迁移 / 首次启动：立即落盘新配置。否则迁移读后即删的旧键 + 未持久化的
  // 新键会让用户在下一次启动时回落默认主题。
  try {
    if (localStorage.getItem(STORAGE_KEY) === null) persist(config.value)
  } catch {
    // 存储不可用：仅本次会话生效
  }

  /** 实际生效主题 id：system 模式按系统深浅取记忆对，manual 取 current。 */
  const themeId = computed<ThemeId>(() => {
    if (config.value.mode === 'system') return systemDark.value ? config.value.lastDark : config.value.lastLight
    return config.value.current
  })

  const custom = computed<CustomThemeConfig | null>(() => config.value.custom)

  /** 生效主题的深浅基调（custom 跟随其 base）。 */
  const resolved = computed<ThemeBase>(() => themeBase(themeId.value, custom.value))

  let media: MediaQueryList | null = null
  let onSystemChange: ((e: MediaQueryListEvent) => void) | null = null

  /** 应用当前生效主题到 DOM。 */
  function apply(): void {
    applyToDom(resolved.value, themeId.value, custom.value)
  }

  /** 系统偏好变化 → 同步 systemDark，仅 system 模式需要实时重刷。 */
  function syncSystemListener(): void {
    media?.removeEventListener('change', onSystemChange!)
    media = null
    onSystemChange = null
    if (config.value.mode !== 'system') return
    media = window.matchMedia('(prefers-color-scheme: dark)')
    onSystemChange = (e) => {
      systemDark.value = e.matches
      apply()
    }
    media.addEventListener('change', onSystemChange)
  }

  /** 切换 system / manual 模式并持久化。 */
  function setMode(next: ThemeMode): void {
    config.value = { ...config.value, mode: next }
    persist(config.value)
    syncSystemListener()
    apply()
  }

  /** 选中主题：记入 current 与对应深浅记忆对，并回到 manual 模式。 */
  function setThemeId(id: ThemeId): void {
    if (!isThemeId(id)) return
    let next = { ...config.value, mode: 'manual' as ThemeMode, current: id }
    if (id === 'custom' && !next.custom) {
      // 首次进入自定义：以当前基调为起点，避免选卡瞬间深浅跳变
      next = { ...next, custom: { ...DEFAULT_CUSTOM, base: resolved.value } }
    }
    const base = themeBase(id, next.custom)
    next = base === 'light' ? { ...next, lastLight: id } : { ...next, lastDark: id }
    config.value = next
    persist(config.value)
    apply()
  }

  /** 就地更新自定义主题（UI 滑杆/取色器实时调用，立即生效）。 */
  function updateCustom(patch: Partial<CustomThemeConfig>): void {
    const base = config.value.custom ?? { ...DEFAULT_CUSTOM, base: resolved.value }
    config.value = { ...config.value, current: 'custom', custom: { ...base, ...patch, version: 1 } }
    const light = (config.value.custom?.base ?? 'light') === 'light'
    config.value = light ? { ...config.value, lastLight: 'custom' } : { ...config.value, lastDark: 'custom' }
    persist(config.value)
    apply()
  }

  /** 恢复默认背景（自定义主题内）。 */
  function resetBackground(): void {
    if (!config.value.custom) return
    updateCustom({ background: { type: 'default' } })
  }

  /** 恢复默认主题（石墨深色，清空自定义槽位）。 */
  function resetTheme(): void {
    config.value = { ...DEFAULT_CONFIG, custom: null }
    persist(config.value)
    syncSystemListener()
    apply()
  }

  /** 导出当前主题配置（JSON 字符串）。 */
  function exportConfig(): string {
    return JSON.stringify(config.value, null, 2)
  }

  /** 导入主题配置：校验通过整体替换并立即生效。 */
  function importConfig(raw: string): boolean {
    try {
      const parsed = parseThemeConfig(JSON.parse(raw))
      if (!parsed) return false
      if (parsed.current === 'custom' && !parsed.custom) parsed.current = 'dark'
      config.value = parsed
      persist(config.value)
      syncSystemListener()
      apply()
      return true
    } catch {
      return false
    }
  }

  /** 应用启动初始化：挂系统监听并立即生效（防闪烁）。 */
  function init(): void {
    systemDark.value = systemPrefersDark()
    syncSystemListener()
    apply()
  }

  return {
    config,
    mode: computed<ThemeMode>(() => config.value.mode),
    themeId,
    custom,
    resolved,
    systemDark,
    setMode,
    setThemeId,
    updateCustom,
    resetBackground,
    resetTheme,
    exportConfig,
    importConfig,
    init,
    apply,
  }
})
