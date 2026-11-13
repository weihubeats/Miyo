/**
 * useAutoUpdate：定时检查更新（tauri-plugin-updater）。
 *
 * - 仅 Tauri 环境生效，浏览器预览静默跳过；
 * - 启动后延迟一次检查（避开启动关键路径），之后按间隔轮询；
 * - localStorage 节流：距上次**成功**检查不足间隔则跳过（多窗口/频繁重启不重复打扰）；
 *   检查失败不计入节流，30 分钟后重试一次；
 * - 同一版本每天最多提醒一次（替代永久锁存：错过 toast 第二天可恢复）；
 * - 跳过的版本不再提醒，直到出现更新的版本；
 * - 发现新版时暂存 Update 对象，更新详情弹窗 / 关于弹窗打开时直接承接（免二次检查，一键下载安装）；
 * - 有待安装更新时点亮共享标识（顶部更新 Tag + 设置齿轮小红点），安装/跳过/取走后熄灭；
 * - 自动检查失败静默忽略（不弹错，避免噪音；手动检查仍会报错）。
 */
import { readonly, ref, type Ref } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { tFallback } from '@/stores/locale'
import { errorText } from '@/utils/errorText'

export const AUTO_UPDATE_STARTUP_DELAY_MS = 8_000
export const AUTO_UPDATE_POLL_INTERVAL_MS = 6 * 60 * 60 * 1000
/** 检查失败后的重试间隔（不计入 6h 节流）。 */
export const AUTO_UPDATE_RETRY_INTERVAL_MS = 30 * 60 * 1000
/** 同版本重复提醒间隔（替代永久锁存）。 */
export const AUTO_UPDATE_RENOTIFY_INTERVAL_MS = 24 * 60 * 60 * 1000

const LAST_CHECK_KEY = 'miyo:update:last-check'
const NOTIFIED_VERSION_KEY = 'miyo:update:notified-version'
const NOTIFIED_AT_KEY = 'miyo:update:notified-at'
const SKIPPED_VERSION_KEY = 'miyo:update:skipped-version'

function readNumber(key: string): number {
  try {
    const v = Number(localStorage.getItem(key))
    return Number.isFinite(v) ? v : 0
  } catch {
    return 0
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    // 存储不可用时仅本次生效
  }
}

function readString(key: string): string {
  try {
    return localStorage.getItem(key) ?? ''
  } catch {
    return ''
  }
}

/** 待安装的更新（自动检查暂存，关于弹窗打开时取走接管）。 */
let pendingAutoUpdate: Update | null = null

/** 是否有待安装的更新（只读，供设置齿轮小红点 / 设置更新行使用）。 */
const hasPendingUpdateState = ref(false)
export const hasPendingUpdate: Readonly<Ref<boolean>> = readonly(hasPendingUpdateState)

/** 待安装更新的版本号（仅展示，不取走暂存）。 */
export function pendingUpdateVersion(): string | null {
  return pendingAutoUpdate?.version ?? null
}

/** 待安装更新的版本说明（仅展示，不取走暂存；顶部悬停预览用）。 */
export function pendingUpdateNotes(): string {
  return pendingAutoUpdate?.body ?? ''
}

/** 待安装更新的发布日期（RFC 3339；服务器未提供时为 null，悬停预览用）。 */
export function pendingUpdateDate(): string | null {
  return pendingAutoUpdate?.date ?? null
}

/** 打开关于弹窗的请求信号（设置更新行 → App 监听后打开）。 */
const openAboutSignalState = ref(0)
export const openAboutSignal: Readonly<Ref<number>> = readonly(openAboutSignalState)

/** 请求打开关于弹窗（更新详情与安装都在其中）。 */
export function requestOpenAbout(): void {
  openAboutSignalState.value += 1
}

/** 打开独立更新详情弹窗的请求信号（顶部更新 Tag / 设置更新卡片 → App 监听后打开）。 */
const openReleaseSignalState = ref(0)
export const openReleaseSignal: Readonly<Ref<number>> = readonly(openReleaseSignalState)

/** 请求打开独立更新详情弹窗（ReleaseNotesModal：更新说明 + 一键下载重启）。 */
export function requestOpenRelease(): void {
  openReleaseSignalState.value += 1
}

/** 跳过指定版本（不再提醒，直到出现更新的版本；可在设置中取消）。 */
export function skipUpdateVersion(version: string): void {
  if (!version) return
  write(SKIPPED_VERSION_KEY, version)
  // 跳过当前待装版本时一并熄灭小红点（出现更新的版本会重新点亮）
  if (pendingAutoUpdate?.version === version) {
    pendingAutoUpdate.close()
    pendingAutoUpdate = null
    hasPendingUpdateState.value = false
  }
}

/** 当前跳过的版本（无则 null）。 */
export function skippedUpdateVersion(): string | null {
  const v = readString(SKIPPED_VERSION_KEY)
  return v || null
}

/** 取消跳过（下次检查到该版本会重新提醒）。 */
export function clearSkippedUpdateVersion(): void {
  try {
    localStorage.removeItem(SKIPPED_VERSION_KEY)
  } catch {
    // 忽略
  }
}
export function takePendingUpdate(): Update | null {
  const u = pendingAutoUpdate
  pendingAutoUpdate = null
  hasPendingUpdateState.value = false
  return u
}

function storePendingUpdate(update: Update): void {
  pendingAutoUpdate?.close()
  pendingAutoUpdate = update
  hasPendingUpdateState.value = true
}

/** 将未安装的更新放回暂存（关于弹窗关闭未安装时调用，小红点重新点亮）。 */
export function restorePendingUpdate(update: Update | null): void {
  if (!update) return
  storePendingUpdate(update)
}

export interface AutoUpdateOptions {
  /** 检查函数（默认走 updater 插件；单测注入）。 */
  checkFn?: () => Promise<Update | null>
  /** 时间源（单测注入）。 */
  now?: () => number
  /** 启动延迟（默认 8s）。 */
  startupDelayMs?: number
  /** 轮询间隔（默认 6h，同时作为成功检查的节流下限）。 */
  pollIntervalMs?: number
  /** 检查失败后的重试间隔（默认 30min；失败不计入节流）。 */
  retryIntervalMs?: number
  /** 同版本重复提醒间隔（默认 24h；替代永久锁存）。 */
  renotifyIntervalMs?: number
  /** 发现新版回调（App 层接 toast + 打开关于弹窗）。 */
  onUpdateAvailable?: (info: { version: string }) => void
}

/** 存量锁存键（调试面板展示/清除用）。 */
export const AUTO_UPDATE_LATCH_KEYS = {
  lastCheck: LAST_CHECK_KEY,
  notifiedVersion: NOTIFIED_VERSION_KEY,
  notifiedAt: NOTIFIED_AT_KEY,
  skipped: SKIPPED_VERSION_KEY,
} as const

/** 调试快照：各锁存的当前值（设置调试区展示用）。 */
export interface UpdateDebugState {
  lastCheck: number
  notifiedVersion: string
  notifiedAt: number
  skipped: string | null
  pending: string | null
}

/** 读取调试快照。 */
export function debugUpdateState(): UpdateDebugState {
  return {
    lastCheck: readNumber(LAST_CHECK_KEY),
    notifiedVersion: readString(NOTIFIED_VERSION_KEY),
    notifiedAt: readNumber(NOTIFIED_AT_KEY),
    skipped: skippedUpdateVersion(),
    pending: pendingUpdateVersion(),
  }
}

/** 清除检查节流与提醒锁存（调试面板 / 取消跳过后立即重查用；跳过记录保留）。 */
export function clearUpdateLatches(): void {
  try {
    localStorage.removeItem(LAST_CHECK_KEY)
    localStorage.removeItem(NOTIFIED_VERSION_KEY)
    localStorage.removeItem(NOTIFIED_AT_KEY)
  } catch {
    // 忽略
  }
}

/** 下一次检查强制失败（调试“模拟失败”用，消费一次即失效）。 */
let debugFailNext = false
export function debugFailNextCheck(): void {
  debugFailNext = true
}

/** 存活检查实例的调试入口（仅最近一次 startAutoUpdate 注册）。 */
interface UpdateDebugApi {
  runCheckNow: () => Promise<DebugCheckResult>
  simulateUpdate: (version: string) => void
}

/** 即时检查的结果（调试面板 / 取消跳过用于反馈）。 */
export type DebugCheckStatus = 'notified' | 'none' | 'failed' | 'no-instance'
export interface DebugCheckResult {
  status: DebugCheckStatus
  /** failed 时的错误信息。 */
  message?: string
}
let activeDebugApi: UpdateDebugApi | null = null

/** 立即执行一次检查（绕过节流与延迟；无运行实例返回 false）。 */
export function triggerUpdateCheckNow(): boolean {
  if (!activeDebugApi) return false
  void activeDebugApi.runCheckNow()
  return true
}

/**
 * 立即真查一次并等待结果（绕过节流；调试面板与取消跳过用，结果落定后再反馈+刷新）。
 * 无运行实例时返回 no-instance。
 */
export async function debugRunCheckNow(): Promise<DebugCheckResult> {
  const api = activeDebugApi
  if (!api) return { status: 'no-instance' }
  return api.runCheckNow()
}

/**
 * 模拟指定版本可用（调试用：走与真实检查相同的去重/跳过/通知链，
 * toast + 小红点 + 关于承接全生效；模拟数据不可安装）。
 */
export function debugSimulateUpdate(version: string): boolean {
  if (!activeDebugApi) return false
  activeDebugApi.simulateUpdate(version)
  return true
}

/** 启动定时检查，返回停止函数（卸载时调用）。 */
export function startAutoUpdate(opts: AutoUpdateOptions = {}): () => void {
  const {
    checkFn = check,
    now = Date.now,
    startupDelayMs = AUTO_UPDATE_STARTUP_DELAY_MS,
    pollIntervalMs = AUTO_UPDATE_POLL_INTERVAL_MS,
    retryIntervalMs = AUTO_UPDATE_RETRY_INTERVAL_MS,
    renotifyIntervalMs = AUTO_UPDATE_RENOTIFY_INTERVAL_MS,
    onUpdateAvailable,
  } = opts

  if (!('__TAURI_INTERNALS__' in window)) return () => undefined

  let stopped = false
  let interval: ReturnType<typeof setInterval> | undefined
  let retryTimer: ReturnType<typeof setTimeout> | undefined
  let registeredApi: UpdateDebugApi | null = null

  /** 检查失败后约一次重试（不叠加、不计入节流）。 */
  function scheduleRetry(): void {
    if (stopped || retryTimer !== undefined) return
    retryTimer = setTimeout(() => {
      retryTimer = undefined
      void doCheck()
    }, retryIntervalMs)
  }

  async function doCheck(
    bypassThrottle = false,
  ): Promise<{ status: Exclude<DebugCheckStatus, 'no-instance'>; message?: string }> {
    if (stopped) return { status: 'none' }
    // 节流：距上次成功检查不足一个轮询间隔则跳过（调试立即检查可绕过）
    if (!bypassThrottle && now() - readNumber(LAST_CHECK_KEY) < pollIntervalMs) return { status: 'none' }
    let update: Update | null = null
    try {
      if (debugFailNext) {
        debugFailNext = false
        throw new Error('debug: injected check failure')
      }
      update = await checkFn()
    } catch (err) {
      scheduleRetry()
      return { status: 'failed', message: errorText(err) }
    }
    write(LAST_CHECK_KEY, String(now()))
    return { status: handleUpdateResult(update) ? 'notified' : 'none' }
  }

  /**
   * 成功检查后的通知链（真实检查与模拟共用），返回是否发出提醒。
   * force：调试模拟专用——绕过同版本 24h 去重与跳过锁存，
   * 保证「模拟新版」点击确定性生效（否则模拟按钮会被门禁静默吞掉）。
   */
  function handleUpdateResult(update: Update | null, force = false): boolean {
    if (stopped) {
      update?.close()
      return false
    }
    if (!update?.available) {
      update?.close()
      return false
    }
    if (!force) {
      // 通知去重：同版本距上次提醒不足一天则跳过（无提醒记录的老锁存视为过期，恢复提醒一次）
      const notified = readString(NOTIFIED_VERSION_KEY)
      const notifiedAt = readNumber(NOTIFIED_AT_KEY)
      if (notified === update.version && now() - notifiedAt < renotifyIntervalMs) {
        update.close()
        return false
      }
      // 用户跳过的版本不再提醒（出现更新的版本时恢复提醒）
      if (readString(SKIPPED_VERSION_KEY) === update.version) {
        update.close()
        return false
      }
    }
    write(NOTIFIED_VERSION_KEY, update.version)
    write(NOTIFIED_AT_KEY, String(now()))
    storePendingUpdate(update)
    onUpdateAvailable?.({ version: update.version })
    return true
  }

  function simulateUpdate(version: string): void {
    const v = version.trim()
    if (!v || stopped) return
    handleUpdateResult(
      {
        available: true,
        version: v,
        body: tFallback('settingsdbg.simulatedNotes'),
        close: () => undefined,
      } as unknown as Update,
      true,
    )
  }

  const debugApi: UpdateDebugApi = {
    runCheckNow: () => doCheck(true),
    simulateUpdate,
  }
  registeredApi = debugApi
  activeDebugApi = debugApi

  const timer = setTimeout(() => {
    void doCheck()
    interval = setInterval(() => {
      void doCheck()
    }, pollIntervalMs)
  }, startupDelayMs)

  return () => {
    stopped = true
    clearTimeout(timer)
    if (interval) clearInterval(interval)
    if (retryTimer !== undefined) {
      clearTimeout(retryTimer)
      retryTimer = undefined
    }
    if (activeDebugApi === registeredApi) activeDebugApi = null
  }
}
