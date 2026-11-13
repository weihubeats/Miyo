/**
 * dateTime.ts：日期时间展示格式化（跟随 locale store）。
 *
 * 之前各处要么写死 'zh-CN'（RealtimeView 日志在英文界面仍出中文时间），
 * 要么直接调 toLocaleString()（随浏览器默认语言漂移，与界面语言脱节）。
 * 统一走这里：
 * - system 模式：用 navigator.language（保留地区精度，如 zh-CN / en-US）；
 * - 显式模式：映射到 zh-CN / en-US；
 * - 在渲染上下文调用会建立对 locale store 的响应式依赖，切语言即时刷新。
 */
import { useLocaleStore } from '@/stores/locale'

/** 当前生效的 BCP 47 语言标签。无活动 Pinia（纯函数单测）时回退中文。 */
export function localeTag(): string {
  try {
    const store = useLocaleStore()
    if (store.mode === 'system') {
      try {
        return navigator.language || 'zh-CN'
      } catch {
        return 'zh-CN'
      }
    }
    return store.resolved === 'zh' ? 'zh-CN' : 'en-US'
  } catch {
    return 'zh-CN'
  }
}

/** 时间（默认 24 小时制 HH:mm:ss），用于实时日志、示例名等。 */
export function formatTime(
  value: string | number | Date,
  opts: Intl.DateTimeFormatOptions = { hour12: false },
): string {
  return new Date(value).toLocaleTimeString(localeTag(), opts)
}

/** 日期 + 时间，用于历史记录、授权有效期等时间戳展示。 */
export function formatDateTime(
  value: string | number | Date,
  opts?: Intl.DateTimeFormatOptions,
): string {
  return new Date(value).toLocaleString(localeTag(), opts)
}
