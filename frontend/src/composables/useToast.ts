/**
 * useToast：全局 Toast 通知（Vue 3 Composable）。
 *
 * 4 种类型：success / info / warning / error，样式沿用项目 miyo- 设计系统
 * （深色面板 + 类型色左描边，变量名与 miyo-desktop/src/styles.rs 对齐）。
 *
 * 用法：
 * ```ts
 * const toast = useToast()
 * toast.success('已保存')                 // 快捷方法
 * toast.error('请求失败', { message: err.message, duration: 6000 })
 * toast.toast({ type: 'warning', title: '提示', duration: 0 })  // 0 = 不自动关闭
 * toast.dismiss(id)                       // 手动关闭
 * ```
 *
 * 渲染：在 App 根组件挂载 <ToastHost />（见 components/ToastHost.vue）。
 */
import { readonly, ref } from 'vue'
import { tFallback } from '@/stores/locale'

export type ToastType = 'success' | 'info' | 'warning' | 'error'

export interface ToastItem {
  id: number
  type: ToastType
  title: string
  message?: string
  /** 自动关闭毫秒数；0 表示不自动关闭。 */
  duration: number
  /** 可选动作按钮（如「打开文件位置」），点击后自动关闭该条。 */
  action?: ToastAction
}

/** Toast 动作按钮（不可序列化，仅会话内使用）。 */
export interface ToastAction {
  label: string
  run: () => void
}

/** 同屏最大条数，超出后丢弃最旧的。 */
const MAX_VISIBLE = 5

const toasts = ref<ToastItem[]>([])
let nextId = 1
/** id → 自动关闭定时器句柄：dismiss / clearAll 时清掉，避免回调里二次过滤已失效条目。 */
const timers = new Map<number, number>()

/** 类型 → 面板主题色（CSS 变量，双主题自动跟随）。 */
export const TOAST_TYPE_META: Record<ToastType, { color: string }> = {
  success: { color: 'var(--success)' },
  info: { color: 'var(--info)' },
  warning: { color: 'var(--warning)' },
  error: { color: 'var(--danger)' },
}

/** 类型 → 默认标题文案键（调用方未传 title 时按当前语言取词）。 */
const TYPE_TITLE_KEY: Record<ToastType, string> = {
  success: 'toast.typeSuccess',
  info: 'toast.typeInfo',
  warning: 'toast.typeWarning',
  error: 'toast.typeError',
}

/** 关闭某条 toast 并清掉它的自动关闭定时器。 */
export function dismiss(id: number): void {
  clearTimer(id)
  toasts.value = toasts.value.filter((t) => t.id !== id)
}

export function clearAll(): void {
  for (const id of timers.keys()) clearTimer(id)
  toasts.value = []
}

function clearTimer(id: number): void {
  const handle = timers.get(id)
  if (handle === undefined) return
  window.clearTimeout(handle)
  timers.delete(id)
}

function push(item: Omit<ToastItem, 'id'>): number {
  const id = nextId++
  const entry: ToastItem = { ...item, id, duration: item.duration ?? 3000 }
  const kept = toasts.value.slice(-(MAX_VISIBLE - 1))
  // 超过同屏上限被顶掉的最旧条目：连同它的自动关闭定时器一起丢弃
  for (const old of toasts.value) {
    if (!kept.includes(old)) clearTimer(old.id)
  }
  toasts.value = [...kept, entry]
  if (entry.duration > 0) {
    timers.set(id, window.setTimeout(() => dismiss(id), entry.duration))
  }
  return id
}

function toast(opts: {
  type: ToastType
  title?: string
  message?: string
  duration?: number
  action?: ToastAction
}): number {
  const type = opts.type
  const title = opts.title ?? tFallback(TYPE_TITLE_KEY[type])
  return push({
    type,
    title,
    message: opts.message,
    duration: opts.duration ?? 3500,
    action: opts.action,
  })
}

export function useToast() {
  return {
    /** 全部可见 toast（只读，供 ToastHost 渲染）。 */
    toasts: readonly(toasts),
    toast,
    success: (title: string, opts?: { message?: string; duration?: number; action?: ToastAction }) =>
      toast({ type: 'success', title, ...opts }),
    info: (title: string, opts?: { message?: string; duration?: number; action?: ToastAction }) =>
      toast({ type: 'info', title, ...opts }),
    warning: (title: string, opts?: { message?: string; duration?: number; action?: ToastAction }) =>
      toast({ type: 'warning', title, ...opts }),
    error: (title: string, opts?: { message?: string; duration?: number; action?: ToastAction }) =>
      toast({ type: 'error', title, ...opts }),
    dismiss,
    clearAll,
  }
}
