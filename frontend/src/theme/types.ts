/**
 * 主题系统类型（多主题 + 单槽位自定义主题）。
 *
 * - ThemeId：6 个内置主题 + 'custom'（单槽位自定义，见 ThemeAppearance 设置区）；
 * - ThemeBase：深/浅基调。DOM 约定：<html data-theme> 恒为二值 base（Tailwind
 *   dark: 变体、CodeMirror 等既有消费点不变），主题身份由 <html data-theme-id> 表达；
 * - ThemeConfigV1：持久化结构（localStorage `miyo.theme.config`），version 字段
 *   预留后续迁移；system 模式在 lastDark/lastLight 两主题间跟随系统深浅切换。
 */

export type BuiltinThemeId =
  | 'light'
  | 'dark'
  | 'ocean'
  | 'ocean-light'
  | 'forest'
  | 'forest-dark'
  | 'sand'
  | 'lavender'
export type ThemeId = BuiltinThemeId | 'custom'
export type ThemeBase = 'dark' | 'light'

/** system = 跟随系统深浅（lastDark/lastLight）；manual = 固定使用 current。 */
export type ThemeMode = 'system' | 'manual'

/** 自定义背景类型：default 跟随主题色板 / 纯色 / 渐变 / 本地图片。 */
export type BackgroundType = 'default' | 'color' | 'gradient' | 'image'

/** 界面密度：紧凑 / 标准 / 宽松（缩放控件高度与间距档，字号不受影响）。 */
export type ThemeDensity = 'compact' | 'default' | 'cozy'

/** 界面字体：default 内置 IBM Plex Sans / system 跟随操作系统。 */
export type ThemeFont = 'default' | 'system'

/**
 * 视觉风格（与颜色主题正交的第二维度）：style.css 的 [data-style] 块通过
 * 组件级语义 token（btn/input/card/menu 等）表达组件视觉语言，
 * <html data-style> 承载。旧配置缺省 → 'studio'。
 */
export type ThemeStyle = 'studio' | 'minimal' | 'soft'

export interface GradientBackground {
  from: string
  to: string
  /** 线性渐变角度（deg）。 */
  angle: number
}

export interface ImageBackground {
  /** 压缩后的 dataURL（见 backgroundImage.ts 的降采样与编码约束）。 */
  data: string
  fit: 'cover' | 'contain'
  /** CSS background-position 关键字（center/top/bottom/left/right）。 */
  position: string
  /** 图片层不透明度 0..1。 */
  opacity: number
  /** 背景模糊半径（px，0..24）。 */
  blur: number
}

export interface BackgroundConfig {
  type: BackgroundType
  color?: string
  gradient?: GradientBackground
  image?: ImageBackground | null
}

/** 用户可覆盖的 5 个核心颜色（#rgb / #rrggbb；缺省 = 跟随所选基调主题）。 */
export interface CustomColors {
  bg?: string
  card?: string
  border?: string
  text?: string
  accent?: string
}

export interface CustomThemeConfig {
  version: 1
  /** 自定义主题的深浅基调：决定 data-theme 与未覆盖令牌的来源主题块。 */
  base: ThemeBase
  colors: CustomColors
  background: BackgroundConfig
  /** 卡片/面板不透明度 0..1（1 = 不透明；背景图模式下面板同步降不透明）。 */
  cardOpacity: number
  /** 基准圆角 px（默认 6，映射 --radius 四档缩放，0..16）。 */
  radius: number
  /** 字号缩放（默认 1，0.85..1.3，作用于 --fs-* 全档）。 */
  fontScale: number
  /** 界面密度（默认 'default'，映射 --density 系数缩放控件高度与间距档）。 */
  density: ThemeDensity
  /** 界面字体（默认 'default' = 内置 IBM Plex Sans）。 */
  font: ThemeFont
}

/** 持久化主题配置（localStorage `miyo.theme.config`）。 */
export interface ThemeConfigV1 {
  version: 1
  mode: ThemeMode
  /** manual 模式生效主题。 */
  current: ThemeId
  /** system 模式系统浅色时生效主题。 */
  lastLight: ThemeId
  /** system 模式系统深色时生效主题。 */
  lastDark: ThemeId
  /** 视觉风格（v1 后增字段；旧配置缺省回退 'studio'，无需迁移）。 */
  style: ThemeStyle
  custom: CustomThemeConfig | null
}

/** 主题切换全局事件 detail（CodeMirror 等编辑器监听；mode = 二值 base）。 */
export interface ThemeEventDetail {
  mode: ThemeBase
  themeId: ThemeId
}
