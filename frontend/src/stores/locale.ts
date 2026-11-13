/**
 * locale.ts：全局语言状态机（跟随系统 / 中文 / English），对标 theme.ts。
 *
 * - localeMode：用户偏好（system | zh | en），持久化到 localStorage；
 * - resolved：实际生效语言（zh | en），system 时按 navigator.language 推导；
 * - t(key, params?)：模板与 TS 文案统一入口，模板内调用随 resolved 自动重渲染；
 * - init：挂载前补齐启动语言字典 + 写 <html lang>（返回 promise，主入口 await），避免首屏闪文案。
 */
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { zh, type ZhKey } from '@/i18n/zh'

export type LocaleMode = 'system' | 'zh' | 'en'
export type ResolvedLocale = 'zh' | 'en'
export type I18nKey = ZhKey
export type I18nParams = Record<string, string | number>

type Dict = Record<string, string>

/**
 * 字典按需加载：
 * - zh 静态常驻（缺键回退、`tFallback` 无 Pinia 兜底都依赖同步可用）；
 * - en 走动态 import 独立 chunk，启动语言为 en 时经下方顶层 await 在模块求值
 *   阶段补齐（主入口与首屏文案拿到的仍是完整字典），zh 启动则不进首屏链路，
 *   由 `loadLocale` 在首次切换 / 空闲预取时拉取。
 * 两本字典 ~124KB 源码不再全进主包。
 */
const zhDict = zh as unknown as Dict
let enDict: Dict | null = null
let enLoading: Promise<void> | null = null

/** 同步可用性：切换语言前先确认字典就绪，避免短暂回落到另一语言文案。 */
export function hasLocale(locale: ResolvedLocale): boolean {
  return locale === 'zh' || enDict !== null
}

/** 按需加载字典（幂等，并发调用共享同一 promise）。 */
export function loadLocale(locale: ResolvedLocale): Promise<void> {
  if (locale === 'zh' || enDict) return Promise.resolve()
  if (!enLoading) {
    enLoading = import('@/i18n/en')
      .then((mod) => {
        enDict = mod.en as unknown as Dict
      })
      .finally(() => {
        enLoading = null
      })
  }
  return enLoading
}

const STORAGE_KEY = 'miyo.locale.mode'
const DEFAULT_MODE: LocaleMode = 'system'

function isMode(v: string | null): v is LocaleMode {
  return v === 'system' || v === 'zh' || v === 'en'
}

function readStored(): LocaleMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY)
    return isMode(v) ? v : DEFAULT_MODE
  } catch {
    return DEFAULT_MODE
  }
}

function systemLocale(): ResolvedLocale {
  try {
    return (navigator.language || 'en').toLowerCase().startsWith('zh') ? 'zh' : 'en'
  } catch {
    return 'zh'
  }
}

function applyToDom(locale: ResolvedLocale): void {
  try {
    document.documentElement.setAttribute('lang', locale === 'zh' ? 'zh-CN' : 'en')
  } catch {
    // 非 DOM 环境（单测）忽略
  }
}

/**
 * 启动语言（持久化偏好 + 系统语言）为 en 时，需在首帧前补齐 en 字典。
 * 不用模块顶层 await：会撞 es2020 构建目标（esbuild 报 Top-level await is not
 * available），改由 main.ts 的异步 bootstrap 在挂载前 await；zh 启动则 en 不进
 * 首屏链路，由 loadLocale 在切换 / 空闲预取时再拉。
 */
const BOOT_MODE = readStored()
const BOOT_LOCALE: ResolvedLocale = BOOT_MODE === 'system' ? systemLocale() : BOOT_MODE

/** 补齐启动语言字典（zh 常驻即返回）。main.ts 挂载前与测试 setup 各调一次。 */
export function preloadBootLocale(): Promise<void> {
  return loadLocale(BOOT_LOCALE)
}

export function translate(locale: ResolvedLocale, key: string, params?: I18nParams): string {
  const dict = locale === 'zh' ? zhDict : enDict
  let text: string = dict?.[key] ?? zhDict[key] ?? key
  if (params) {
    // 单次正则替换：原实现对每个占位符 split().join()，占位符多时重复扫描整串。
    text = text.replace(/\{(\w+)\}/g, (token, name: string) =>
      Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : token,
    )
  }
  return text
}

/**
 * 非组件环境的兜底翻译：供纯 utils（解析器抛错等）在任意调用点取当前语言文案；
 * 无活动 Pinia（部分纯函数单测）时回退中文，保证不崩。
 */
export function tFallback(key: string, params?: I18nParams): string {
  try {
    return useLocaleStore().t(key, params)
  } catch {
    return translate('zh', key, params)
  }
}

/**
 * 新建实体默认名（创建时按当前语言取值）与判定（中英双语都认，
 * 用户切语言后旧草稿仍能正确触发「命名确认」）。
 */
export type DefaultNameKind = 'endpoint' | 'project' | 'example' | 'folder'

const DEFAULT_NAME_KEYS: Record<DefaultNameKind, string> = {
  endpoint: 'default.endpointName',
  project: 'default.projectName',
  example: 'default.exampleName',
  folder: 'default.folderName',
}

export function defaultName(kind: DefaultNameKind, locale: ResolvedLocale): string {
  return translate(locale, DEFAULT_NAME_KEYS[kind])
}

export function isDefaultName(kind: DefaultNameKind, name: string): boolean {
  if (!name.trim()) return true
  return translate('zh', DEFAULT_NAME_KEYS[kind]) === name || translate('en', DEFAULT_NAME_KEYS[kind]) === name
}

export const useLocaleStore = defineStore('locale', () => {
  const mode = ref<LocaleMode>(readStored())

  /** 实际生效语言：system 跟随系统，否则取用户偏好。 */
  const resolved = computed<ResolvedLocale>(() =>
    mode.value === 'system' ? systemLocale() : mode.value,
  )

  /** 翻译函数（模板内调用，resolved 变化自动重渲染）。 */
  function t(key: I18nKey | string, params?: I18nParams): string {
    return translate(resolved.value, key, params)
  }

  function apply(): void {
    applyToDom(resolved.value)
  }

  function commit(next: LocaleMode): void {
    mode.value = next
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // 存储不可用：仅本次会话生效
    }
    apply()
  }

  /**
   * 切换语言偏好并持久化，即时生效。
   * 目标字典已就绪时同步提交（zh 常驻、en 启动即加载或已预取），
   * 否则等动态 import 落地再提交——避免先切过去却闪出另一语言文案。
   */
  function setMode(next: LocaleMode): void {
    const target: ResolvedLocale = next === 'system' ? systemLocale() : next
    if (hasLocale(target)) {
      commit(next)
      return
    }
    loadLocale(target)
      .catch(() => undefined)
      .then(() => commit(next))
  }

  /**
   * 应用启动初始化：先补齐启动语言字典（en 动态 import）→ 立即生效 →
   * 空闲预取另一语言。返回 promise，主入口在挂载前 await，首帧文案不回落另一语言。
   * 预取让「切换语言」零等待，也让 isDefaultName 的中英默认名判定立刻可用。
   */
  async function init(): Promise<void> {
    await preloadBootLocale()
    apply()
    const other: ResolvedLocale = resolved.value === 'zh' ? 'en' : 'zh'
    if (hasLocale(other)) return
    const schedule: (cb: () => void) => number =
      typeof window.requestIdleCallback === 'function'
        ? (cb) => window.requestIdleCallback(() => cb())
        : (cb) => window.setTimeout(cb, 300)
    schedule(() => {
      void loadLocale(other)
    })
  }

  return { mode, resolved, t, setMode, init, apply }
})
