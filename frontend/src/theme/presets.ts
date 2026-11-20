/**
 * 内置主题预设与配置校验。
 *
 * - swatch 三色（页面背景 / 卡片 / 强调色）仅供设置页预览卡渲染，须与
 *   style.css 对应 [data-theme-id] 块中的 --bg-app/--bg-card/--accent 保持一致；
 * - DEFAULT_CONFIG 为首次启动与「恢复默认主题」的基准（石墨深色）；
 * - parseThemeConfig 负责导入 JSON / 读取 localStorage 的结构与取值校验，
 *   任何非法字段回退默认值（不抛错，返回 null 表示整体不可用）；
 * - density / font 为 v1 后增字段：旧配置缺省时回退默认值，无需迁移。
 */
import type {
  BackgroundConfig,
  BuiltinThemeId,
  CustomColors,
  CustomThemeConfig,
  ThemeBase,
  ThemeConfigV1,
  ThemeDensity,
  ThemeFont,
  ThemeId,
} from './types'

export interface ThemePreset {
  id: BuiltinThemeId
  base: ThemeBase
  swatch: { app: string; card: string; accent: string }
}

/** 八个内置主题：深色 3（石墨/海盐蓝/森林绿）+ 浅色 5（极简/海盐浅蓝/森林浅绿/暖沙/紫雾）。 */
export const BUILTIN_THEMES: ThemePreset[] = [
  { id: 'light', base: 'light', swatch: { app: '#f4f5f6', card: '#ffffff', accent: '#6d5ce6' } },
  { id: 'dark', base: 'dark', swatch: { app: '#0f0f10', card: '#1d1d21', accent: '#8380e8' } },
  { id: 'ocean', base: 'dark', swatch: { app: '#0f151d', card: '#18212e', accent: '#5b8fd9' } },
  { id: 'ocean-light', base: 'light', swatch: { app: '#f2f5f9', card: '#ffffff', accent: '#2e6da8' } },
  { id: 'forest-dark', base: 'dark', swatch: { app: '#0e1411', card: '#182019', accent: '#4ca77c' } },
  { id: 'forest', base: 'light', swatch: { app: '#eff3ee', card: '#ffffff', accent: '#3e7d54' } },
  { id: 'sand', base: 'light', swatch: { app: '#f5efe6', card: '#fffdf8', accent: '#a06b3d' } },
  { id: 'lavender', base: 'light', swatch: { app: '#f1eff6', card: '#fcfbfe', accent: '#8a76d8' } },
]

const BUILTIN_IDS = new Set<string>(BUILTIN_THEMES.map((t) => t.id))

export function isBuiltinThemeId(v: string): v is BuiltinThemeId {
  return BUILTIN_IDS.has(v)
}

export function isThemeId(v: string): v is ThemeId {
  return v === 'custom' || isBuiltinThemeId(v)
}

export function themeBase(id: ThemeId, custom: CustomThemeConfig | null): ThemeBase {
  if (id === 'custom') return custom?.base ?? 'dark'
  return BUILTIN_THEMES.find((t) => t.id === id)?.base ?? 'dark'
}

export const DEFAULT_CONFIG: ThemeConfigV1 = {
  version: 1,
  mode: 'manual',
  current: 'dark',
  lastLight: 'light',
  lastDark: 'dark',
  custom: null,
}

export const DEFAULT_CUSTOM: CustomThemeConfig = {
  version: 1,
  base: 'light',
  colors: {},
  background: { type: 'default' },
  cardOpacity: 1,
  radius: 6,
  fontScale: 1,
  density: 'default',
  font: 'default',
}

const HEX_RE = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i

function clamp(n: unknown, min: number, max: number, fallback: number): number {
  return typeof n === 'number' && Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback
}

function normalizeColor(v: unknown): string | undefined {
  return typeof v === 'string' && HEX_RE.test(v.trim()) ? v.trim().toLowerCase() : undefined
}

function normalizeBackground(raw: unknown): BackgroundConfig {
  const fb: BackgroundConfig = { type: 'default' }
  if (!raw || typeof raw !== 'object') return fb
  const r = raw as Record<string, unknown>
  const type = r.type
  if (type === 'color') {
    const color = normalizeColor(r.color)
    return color ? { type, color } : fb
  }
  if (type === 'gradient' && r.gradient && typeof r.gradient === 'object') {
    const g = r.gradient as Record<string, unknown>
    const from = normalizeColor(g.from)
    const to = normalizeColor(g.to)
    if (from && to) {
      return {
        type,
        gradient: { from, to, angle: clamp(g.angle, 0, 360, 135) },
      }
    }
    return fb
  }
  if (type === 'image' && r.image && typeof r.image === 'object') {
    const img = r.image as Record<string, unknown>
    const data = typeof img.data === 'string' && img.data.startsWith('data:image/') ? img.data : null
    if (!data) return fb
    const fit = img.fit === 'contain' ? 'contain' : 'cover'
    const pos = typeof img.position === 'string' ? img.position : 'center'
    return {
      type,
      image: {
        data,
        fit,
        position: pos,
        opacity: clamp(img.opacity, 0, 1, 1),
        blur: clamp(img.blur, 0, 24, 0),
      },
    }
  }
  return fb
}

function normalizeCustom(raw: unknown): CustomThemeConfig | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  const base: ThemeBase = r.base === 'light' ? 'light' : 'dark'
  const colorsRaw = (r.colors && typeof r.colors === 'object' ? r.colors : {}) as Record<string, unknown>
  const colors: CustomColors = {}
  for (const key of ['bg', 'card', 'border', 'text', 'accent'] as const) {
    const v = normalizeColor(colorsRaw[key])
    if (v) colors[key] = v
  }
  const custom: CustomThemeConfig = {
    version: 1,
    base,
    colors,
    background: normalizeBackground(r.background),
    cardOpacity: clamp(r.cardOpacity, 0, 1, DEFAULT_CUSTOM.cardOpacity),
    radius: clamp(r.radius, 0, 16, DEFAULT_CUSTOM.radius),
    fontScale: clamp(r.fontScale, 0.85, 1.3, DEFAULT_CUSTOM.fontScale),
    density: normalizeDensity(r.density),
    font: normalizeFont(r.font),
  }
  return custom
}

/** 密度取值归一：非法值回退 'default'（旧配置缺字段同样走默认）。 */
function normalizeDensity(v: unknown): ThemeDensity {
  return v === 'compact' || v === 'cozy' ? v : 'default'
}

/** 字体取值归一：非法值回退 'default'。 */
function normalizeFont(v: unknown): ThemeFont {
  return v === 'system' ? 'system' : 'default'
}

/**
 * 校验并规范化一份主题配置（导入 JSON / localStorage 读取共用）。
 * 返回 null 表示整体不可用（调用方回退默认配置或报格式错误）。
 */
export function parseThemeConfig(raw: unknown): ThemeConfigV1 | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (r.version !== 1) return null

  const mode = r.mode === 'system' ? 'system' : 'manual'
  const pickId = (v: unknown, fallback: ThemeId): ThemeId =>
    typeof v === 'string' && isThemeId(v) ? v : fallback

  const customRaw = r.custom
  const custom = customRaw == null ? null : normalizeCustom(customRaw)
  if (customRaw != null && custom === null) return null

  return {
    version: 1,
    mode,
    current: pickId(r.current, DEFAULT_CONFIG.current),
    lastLight: pickId(r.lastLight, DEFAULT_CONFIG.lastLight),
    lastDark: pickId(r.lastDark, DEFAULT_CONFIG.lastDark),
    custom,
  }
}
