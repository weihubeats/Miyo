/**
 * useAutoUpdate 单测：启动延迟 / 轮询 / 节流 / 同版本去重 / 失败静默。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Update } from '@tauri-apps/plugin-updater'
import { startAutoUpdate, takePendingUpdate } from '@/domains/workspace/composables/useAutoUpdate'
import {
  clearSkippedUpdateVersion,
  clearUpdateLatches,
  debugFailNextCheck,
  debugRunCheckNow,
  debugSimulateUpdate,
  debugUpdateState,
  hasPendingUpdate,
  openAboutSignal,
  pendingUpdateVersion,
  requestOpenAbout,
  restorePendingUpdate,
  skipUpdateVersion,
  skippedUpdateVersion,
  triggerUpdateCheckNow,
} from '@/domains/workspace/composables/useAutoUpdate'

function fakeUpdate(version: string): Update {
  return { available: true, version, body: 'notes', close: vi.fn() } as unknown as Update
}

function fakeCurrent(version: string): Update {
  return { available: false, version, close: vi.fn() } as unknown as Update
}

beforeEach(() => {
  vi.useFakeTimers()
  localStorage.clear()
  // 标记为 Tauri 环境，否则 startAutoUpdate 直接返回空操作
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
  // 排空模块级暂存，避免测试间污染
  takePendingUpdate()?.close()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('useAutoUpdate', () => {
  it('非 Tauri 环境直接返回空操作，不检查', async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
    const checkFn = vi.fn(async () => fakeUpdate('9.9.9'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 1000 })
    await vi.advanceTimersByTimeAsync(5000)
    expect(checkFn).not.toHaveBeenCalled()
    stop()
  })

  it('启动延迟后检查一次，发现新版回调并暂存', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 8000, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(7999)
    expect(checkFn).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(checkFn).toHaveBeenCalledTimes(1)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    expect(takePendingUpdate()?.version).toBe('1.2.0')
    expect(takePendingUpdate()).toBeNull()
    stop()
  })

  it('无更新时不回调、不暂存', async () => {
    const checkFn = vi.fn(async () => fakeCurrent('0.0.17'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).not.toHaveBeenCalled()
    expect(takePendingUpdate()).toBeNull()
    stop()
  })

  it('同一版本只提醒一次，6 小时轮询会再次检查', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({
      checkFn,
      startupDelayMs: 100,
      pollIntervalMs: 6 * 60 * 60 * 1000,
      onUpdateAvailable,
    })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(1)
    // 轮询触发：检查执行，但同版本不再回调（旧暂存被关闭释放）
    await vi.advanceTimersByTimeAsync(6 * 60 * 60 * 1000)
    expect(checkFn).toHaveBeenCalledTimes(2)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(1)
    stop()
  })

  it('距上次检查不足间隔则跳过（多窗口/重启不重复打扰）', async () => {
    localStorage.setItem('miyo:update:last-check', String(Date.now()))
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100 })
    await vi.advanceTimersByTimeAsync(200)
    expect(checkFn).not.toHaveBeenCalled()
    stop()
  })

  it('检查失败不计入节流，30 分钟后重试', async () => {
    const checkFn = vi
      .fn()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValue(fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(checkFn).toHaveBeenCalledTimes(1)
    expect(onUpdateAvailable).not.toHaveBeenCalled()
    // 30 分钟重试（若失败计入 6h 节流，此处不会再查）
    await vi.advanceTimersByTimeAsync(30 * 60 * 1000)
    expect(checkFn).toHaveBeenCalledTimes(2)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    stop()
  })

  it('同版本超过 24 小时后再次提醒（替代永久锁存）', async () => {
    localStorage.setItem('miyo:update:notified-version', '1.2.0')
    localStorage.setItem('miyo:update:notified-at', String(Date.now() - 25 * 60 * 60 * 1000))
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(1)
    stop()
  })

  it('待装标识：暂存点亮、取走熄灭、放回重亮、跳过熄灭', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100 })
    expect(hasPendingUpdate.value).toBe(false)
    await vi.advanceTimersByTimeAsync(200)
    expect(hasPendingUpdate.value).toBe(true)
    expect(pendingUpdateVersion()).toBe('1.2.0')
    const u = takePendingUpdate()
    expect(u?.version).toBe('1.2.0')
    expect(hasPendingUpdate.value).toBe(false)
    restorePendingUpdate(u)
    expect(hasPendingUpdate.value).toBe(true)
    skipUpdateVersion('1.2.0')
    expect(hasPendingUpdate.value).toBe(false)
    expect(pendingUpdateVersion()).toBeNull()
    stop()
  })

  it('requestOpenAbout 递增打开信号', () => {
    const before = openAboutSignal.value
    requestOpenAbout()
    expect(openAboutSignal.value).toBe(before + 1)
  })

  it('triggerUpdateCheckNow 绕过节流立即检查；停止后返回 false', async () => {
    localStorage.setItem('miyo:update:last-check', String(Date.now()))
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    // 节流中：定时检查被跳过
    expect(checkFn).not.toHaveBeenCalled()
    expect(triggerUpdateCheckNow()).toBe(true)
    await vi.advanceTimersByTimeAsync(0)
    expect(checkFn).toHaveBeenCalledTimes(1)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    stop()
    expect(triggerUpdateCheckNow()).toBe(false)
  })

  it('debugSimulateUpdate 走完整通知链；停止后返回 false', async () => {
    const checkFn = vi.fn(async () => fakeCurrent('0.0.1'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    expect(debugSimulateUpdate('9.9.9')).toBe(true)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '9.9.9' })
    expect(hasPendingUpdate.value).toBe(true)
    expect(pendingUpdateVersion()).toBe('9.9.9')
    stop()
    expect(debugSimulateUpdate('9.9.9')).toBe(false)
  })

  it('debugSimulateUpdate 绕过跳过锁存（调试按钮确定性生效）', () => {
    const checkFn = vi.fn()
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    skipUpdateVersion('9.9.9')
    expect(debugSimulateUpdate('9.9.9')).toBe(true)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '9.9.9' })
    expect(hasPendingUpdate.value).toBe(true)
    stop()
  })

  it('debugSimulateUpdate 绕过同版本 24h 去重（重复模拟仍提醒）', () => {
    const checkFn = vi.fn()
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    expect(debugSimulateUpdate('9.9.9')).toBe(true)
    expect(debugSimulateUpdate('9.9.9')).toBe(true)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(2)
    expect(hasPendingUpdate.value).toBe(true)
    expect(pendingUpdateVersion()).toBe('9.9.9')
    stop()
  })

  it('debugFailNextCheck 让下一次检查失败（消费一次即失效）', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    debugFailNextCheck()
    await vi.advanceTimersByTimeAsync(200)
    // 注入失败抛在 checkFn 之前：真实检查未执行、无通知
    expect(checkFn).not.toHaveBeenCalled()
    expect(onUpdateAvailable).not.toHaveBeenCalled()
    // 失败安排 30 分钟重试：重试走真实检查并成功通知
    await vi.advanceTimersByTimeAsync(30 * 60 * 1000)
    expect(checkFn).toHaveBeenCalledTimes(1)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    stop()
  })

  it('debugRunCheckNow 等待结果：notified / none / failed / no-instance', async () => {
    // notified（绕过节流）
    localStorage.setItem('miyo:update:last-check', String(Date.now()))
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await expect(debugRunCheckNow()).resolves.toEqual({ status: 'notified' })
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    stop()
    // 停止后无实例
    await expect(debugRunCheckNow()).resolves.toEqual({ status: 'no-instance' })
  })

  it('debugRunCheckNow 无更新时返回 none', async () => {
    const checkFn = vi.fn(async () => fakeCurrent('0.0.1'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100 })
    await expect(debugRunCheckNow()).resolves.toEqual({ status: 'none' })
    stop()
  })

  it('debugRunCheckNow 失败时返回 failed 并带信息', async () => {
    const checkFn = vi.fn(async (): Promise<Update | null> => {
      throw new Error('offline')
    })
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100 })
    await expect(debugRunCheckNow()).resolves.toEqual({ status: 'failed', message: 'offline' })
    stop()
  })

  it('debugUpdateState / clearUpdateLatches 读写锁存', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100 })
    await vi.advanceTimersByTimeAsync(200)
    const s = debugUpdateState()
    expect(s.pending).toBe('1.2.0')
    expect(s.notifiedVersion).toBe('1.2.0')
    expect(s.lastCheck).toBeGreaterThan(0)
    clearUpdateLatches()
    const cleared = debugUpdateState()
    expect(cleared.lastCheck).toBe(0)
    expect(cleared.notifiedVersion).toBe('')
    stop()
  })

  it('取消跳过链：清跳过 + 清锁存 + 立即重查后重新提醒', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(1)
    skipUpdateVersion('1.2.0')
    // 设置 unskipVersion 的三步
    clearSkippedUpdateVersion()
    clearUpdateLatches()
    triggerUpdateCheckNow()
    await vi.advanceTimersByTimeAsync(0)
    expect(onUpdateAvailable).toHaveBeenCalledTimes(2)
    expect(hasPendingUpdate.value).toBe(true)
    expect(pendingUpdateVersion()).toBe('1.2.0')
    stop()
  })

  it('检查失败静默忽略，不抛错', async () => {
    const checkFn = vi.fn(async () => {
      throw new Error('offline')
    })
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).not.toHaveBeenCalled()
    stop()
  })

  it('停止后不再检查', async () => {
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, pollIntervalMs: 1000 })
    stop()
    await vi.advanceTimersByTimeAsync(5000)
    expect(checkFn).not.toHaveBeenCalled()
  })

  it('跳过的版本不再提醒，取消跳过后恢复提醒', async () => {
    skipUpdateVersion('1.2.0')
    expect(skippedUpdateVersion()).toBe('1.2.0')
    const checkFn = vi.fn(async () => fakeUpdate('1.2.0'))
    const onUpdateAvailable = vi.fn()
    const stop = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(checkFn).toHaveBeenCalledTimes(1)
    expect(onUpdateAvailable).not.toHaveBeenCalled()
    expect(takePendingUpdate()).toBeNull()
    stop()

    // 取消跳过 + 清除节流后，下次检查重新提醒
    clearSkippedUpdateVersion()
    expect(skippedUpdateVersion()).toBeNull()
    localStorage.removeItem('miyo:update:last-check')
    const stop2 = startAutoUpdate({ checkFn, startupDelayMs: 100, onUpdateAvailable })
    await vi.advanceTimersByTimeAsync(200)
    expect(onUpdateAvailable).toHaveBeenCalledWith({ version: '1.2.0' })
    stop2()
  })
})
