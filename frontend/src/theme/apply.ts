/**
 * 自定义主题 → DOM 应用层。
 *
 * 内置主题的差异全部由 style.css 的 [data-theme-id] 块表达；自定义主题是
 * 「在基调主题之上的覆盖」，通过 <html> 的内联 CSS 变量实现——内联样式优先级
 * 天然压过任何样式表块，且不触碰样式表本身（无重排风暴，切换即时）。
 *
 * 应用顺序（顺序敏感）：
 *   1. 清空全部托管内联变量（回到样式表主题块的原值）；
 *   2. 读取基调主题在样式表中的原值（面板/卡片底色，用于派生透明度）；
 *   3. 写入用户覆盖值与派生值（accent 家族 / 文本梯度 / 圆角 / 字号 / 密度 / 字体）。
 */
import type { CustomThemeConfig } from './types'

interface Rgb {
  r: number
  g: number
  b: number
}

const HEX6_RE = /^#([0-9a-f]{6})$/i
const HEX3_RE = /^#([0-9a-f]{3})$/i

/** 解析 #rgb / #rrggbb；非法返回 null（导入配置已归一化，理论不会命中）。 */
function parseHex(hex: string): Rgb | null {
  let m = HEX6_RE.exec(hex)
  if (m) {
    const n = parseInt(m[1], 16)
    return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff }
  }
  m = HEX3_RE.exec(hex)
  if (m) {
    const [r, g, b] = m[1].split('')
    return {
      r: parseInt(r + r, 16),
      g: parseInt(g + g, 16),
      b: parseInt(b + b, 16),
    }
  }
  return null
}

function toHex({ r, g, b }: Rgb): string {
  const h = (v: number): string => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0')
  return `#${h(r)}${h(g)}${h(b)}`
}

/** 两色插值：ratio 为 b 的占比（0 = 全 a，1 = 全 b）。任一色非法则原样返回 a。 */
export function mix(a: string, b: string, ratio: number): string {
  const ca = parseHex(a)
  const cb = parseHex(b)
  if (!ca || !cb) return a
  return toHex({
    r: ca.r + (cb.r - ca.r) * ratio,
    g: ca.g + (cb.g - ca.g) * ratio,
    b: ca.b + (cb.b - ca.b) * ratio,
  })
}

/** 十六进制色 + 透明度 → rgba() 字符串；非法输入返回原值。 */
export function withAlpha(hex: string, alpha: number): string {
  const c = parseHex(hex)
  if (!c) return hex
  return `rgba(${c.r}, ${c.g}, ${c.b}, ${Math.round(alpha * 1000) / 1000})`
}

const lighten = (hex: string, amt: number): string => mix(hex, '#ffffff', amt)
const darken = (hex: string, amt: number): string => mix(hex, '#000000', amt)

/* ---- 托管内联变量清单（apply 前先整体清空，避免上一次覆盖残留） ---- */
const RADIUS_KEYS = ['--radius-sm', '--radius', '--radius-md', '--radius-lg', '--radius-xl'] as const
const FS_KEYS = ['--fs-2xxs', '--fs-xxs', '--fs-xs', '--fs-sm', '--fs-md', '--fs-lg', '--fs-xl'] as const
/** 密度/字体为单值开关型变量（非清单派生），单独托管。 */
const SINGLE_KEYS = ['--density', '--font-ui'] as const

/** 系统字体栈（font='system' 时覆盖 --font-ui；与内置栈除首段 Plex 外一致）。 */
const SYSTEM_FONT_STACK =
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'HarmonyOS Sans SC', 'Microsoft YaHei', sans-serif"

/** 密度档 → --density 系数（style.css 的控件高度与间距档乘以该值缩放）。 */
const DENSITY_FACTOR: Record<CustomThemeConfig['density'], string> = {
  compact: '0.9',
  default: '1',
  cozy: '1.1',
}
const COLOR_KEYS = [
  '--bg-app',
  '--bg-panel',
  '--bg-card',
  '--bg-elevated',
  '--bg-code',
  '--border',
  '--border-strong',
  '--text-1',
  '--text-2',
  '--text-3',
  '--text-value',
  '--accent',
  '--accent-hover',
  '--accent-tint',
  '--primary',
  '--grad-primary',
  '--grad-primary-hover',
  '--primary-glow',
  '--primary-glow-hover',
  '--focus-ring',
] as const

/** 字号 token 基准表（px），与 style.css :root 的 --fs-* 保持一致。 */
const FS_BASE: Record<(typeof FS_KEYS)[number], number> = {
  '--fs-2xxs': 10,
  '--fs-xxs': 11,
  '--fs-xs': 12,
  '--fs-sm': 13,
  '--fs-md': 14,
  '--fs-lg': 16,
  '--fs-xl': 20,
}

function setVar(root: HTMLElement, key: string, value: string | null): void {
  if (value === null) root.style.removeProperty(key)
  else root.style.setProperty(key, value)
}

/** 清空全部托管内联变量（回到样式表主题块原值）。 */
export function clearCustomTheme(root: HTMLElement): void {
  for (const key of COLOR_KEYS) root.style.removeProperty(key)
  for (const key of RADIUS_KEYS) root.style.removeProperty(key)
  for (const key of FS_KEYS) root.style.removeProperty(key)
  for (const key of SINGLE_KEYS) root.style.removeProperty(key)
}

/**
 * 应用自定义主题覆盖。root 上的 data-theme/data-theme-id 必须已先写好
 * （第 2 步读的是基调主题块的样式表原值）。
 */
export function applyCustomTheme(root: HTMLElement, custom: CustomThemeConfig | null): void {
  clearCustomTheme(root)
  if (!custom) return

  const style = getComputedStyle(root)
  const computedCard = style.getPropertyValue('--bg-card').trim()
  const computedPanel = style.getPropertyValue('--bg-panel').trim()
  const dark = custom.base === 'dark'
  const cardOpacity = Math.min(1, Math.max(0, custom.cardOpacity))

  /* ---- 背景三形态：图片（app 透明让固定层透出）/ 纯色 / 渐变 ---- */
  const bg = custom.background
  if (bg.type === 'image') {
    setVar(root, '--bg-app', 'transparent')
    // 背景图模式：面板/卡片按用户透明度降不透明，让图片透出主工作区
    if (cardOpacity < 1) {
      const alpha = Math.max(0.2, cardOpacity)
      if (parseHex(computedPanel)) setVar(root, '--bg-panel', withAlpha(computedPanel, alpha))
      if (parseHex(computedCard)) setVar(root, '--bg-card', withAlpha(computedCard, alpha))
    }
  } else if (bg.type === 'color' && bg.color) {
    setVar(root, '--bg-app', bg.color)
  } else if (bg.type === 'gradient' && bg.gradient) {
    const { from, to, angle } = bg.gradient
    setVar(root, '--bg-app', `linear-gradient(${angle}deg, ${from}, ${to})`)
  }

  /* ---- 用户颜色覆盖 + 派生 ---- */
  const colors = custom.colors
  const card = colors.card ?? computedCard
  if (colors.card) setVar(root, '--bg-card', cardOpacity < 1 ? withAlpha(card, cardOpacity) : card)
  if (colors.card) setVar(root, '--bg-elevated', mix(card, dark ? '#ffffff' : '#000000', 0.06))
  // 代码井底跟随页面底色轻微沉一档（仅显式设置了页面背景且有实体底色时）
  if (colors.bg && bg.type !== 'image') setVar(root, '--bg-code', mix(colors.bg, card, 0.35))

  if (colors.border) {
    setVar(root, '--border', colors.border)
    setVar(root, '--border-strong', dark ? lighten(colors.border, 0.15) : darken(colors.border, 0.12))
  }

  if (colors.text) {
    const pageBg = colors.bg ?? (dark ? '#0f0f11' : '#f4f5f6')
    setVar(root, '--text-1', colors.text)
    setVar(root, '--text-value', mix(colors.text, pageBg, dark ? 0.2 : 0.14))
    setVar(root, '--text-2', mix(colors.text, pageBg, dark ? 0.3 : 0.2))
    setVar(root, '--text-3', mix(colors.text, pageBg, dark ? 0.55 : 0.38))
  }

  if (colors.accent) {
    const accent = colors.accent
    setVar(root, '--accent', accent)
    setVar(root, '--accent-hover', dark ? lighten(accent, 0.12) : darken(accent, 0.12))
    setVar(root, '--accent-tint', withAlpha(accent, dark ? 0.16 : 0.12))
    // 主行动色双轨：--accent 亮（tint/文字/caret），实心按钮底（grad-primary）
    // 取加深档保证白字 ≥4.5:1（impeccable low-contrast 规则）
    setVar(root, '--primary', accent)
    const primarySolid = darken(accent, dark ? 0.25 : 0.1)
    const primaryHover = darken(accent, dark ? 0.32 : 0.16)
    setVar(root, '--grad-primary', `linear-gradient(135deg, ${primarySolid}, ${primarySolid})`)
    setVar(root, '--grad-primary-hover', `linear-gradient(135deg, ${primaryHover}, ${primaryHover})`)
    setVar(root, '--primary-glow', withAlpha(accent, dark ? 0.14 : 0.1))
    setVar(root, '--primary-glow-hover', withAlpha(accent, dark ? 0.2 : 0.14))
    setVar(root, '--focus-ring', withAlpha(accent, 0.45))
  }

  /* ---- 圆角 / 字号 / 密度 / 字体（偏离默认才写，保持内联层最小化） ---- */
  if (custom.radius !== 6) {
    const r = custom.radius
    setVar(root, '--radius-sm', `${Math.max(0, Math.round(r - 2))}px`)
    setVar(root, '--radius', `${Math.round(r)}px`)
    setVar(root, '--radius-md', `${Math.round(r + 2)}px`)
    setVar(root, '--radius-lg', `${Math.round(r + 4)}px`)
    setVar(root, '--radius-xl', `${Math.round(r + 10)}px`)
  }
  if (custom.fontScale !== 1) {
    for (const key of FS_KEYS) setVar(root, key, `${Math.round(FS_BASE[key] * custom.fontScale * 10) / 10}px`)
  }
  if (custom.density !== 'default') {
    setVar(root, '--density', DENSITY_FACTOR[custom.density])
  }
  if (custom.font === 'system') {
    setVar(root, '--font-ui', SYSTEM_FONT_STACK)
  }
}
