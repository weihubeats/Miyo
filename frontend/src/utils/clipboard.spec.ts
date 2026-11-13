import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyText } from '@/utils/clipboard'

// hoisted mock：拦截 utils 内动态 import('@tauri-apps/api/core')。
const { invoke: mockInvoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

type Win = { __TAURI_INTERNALS__?: unknown }
const originalClipboard = navigator.clipboard
const originalExecCommand = document.execCommand
const originalInternals = (window as Win).__TAURI_INTERNALS__

function setClipboard(value: unknown): void {
  Object.defineProperty(navigator, 'clipboard', { value, configurable: true })
}
function setExecCommand(value: unknown): void {
  Object.defineProperty(document, 'execCommand', { value, configurable: true })
}
function setInternals(value: unknown): void {
  ;(window as Win).__TAURI_INTERNALS__ = value as never
}

afterEach(() => {
  setClipboard(originalClipboard)
  setExecCommand(originalExecCommand)
  setInternals(originalInternals)
  mockInvoke.mockReset()
})

describe('copyText', () => {
  it('优先使用 Clipboard API', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    setClipboard({ writeText })
    await expect(copyText('hello')).resolves.toBe(true)
    expect(writeText).toHaveBeenCalledWith('hello')
  })

  it('Clipboard API 不存在时降级 execCommand', async () => {
    setClipboard(undefined)
    const exec = vi.fn().mockReturnValue(true)
    setExecCommand(exec)
    await expect(copyText('fallback')).resolves.toBe(true)
    expect(exec).toHaveBeenCalledWith('copy')
  })

  it('Clipboard API 抛错时降级 execCommand', async () => {
    setClipboard({ writeText: vi.fn().mockRejectedValue(new Error('permission denied')) })
    const exec = vi.fn().mockReturnValue(true)
    setExecCommand(exec)
    await expect(copyText('x')).resolves.toBe(true)
    expect(exec).toHaveBeenCalled()
  })

  it('降级路径也失败时返回 false', async () => {
    setClipboard({ writeText: vi.fn().mockRejectedValue(new Error('permission denied')) })
    setExecCommand(vi.fn().mockReturnValue(false))
    await expect(copyText('boom')).resolves.toBe(false)
  })

  it('execCommand 抛异常时返回 false', async () => {
    setClipboard(undefined)
    setExecCommand(
      vi.fn().mockImplementation(() => {
        throw new Error('not implemented')
      }),
    )
    await expect(copyText('boom')).resolves.toBe(false)
  })
})

describe('copyText（Tauri 原生兜底）', () => {
  it('Tauri WebView 中优先走原生命令（跳过 Clipboard API）', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    setClipboard({ writeText })
    setInternals({})
    mockInvoke.mockResolvedValue(undefined)
    await expect(copyText('native-first')).resolves.toBe(true)
    expect(mockInvoke).toHaveBeenCalledWith('plugin:miyo|clipboard_write_text', {
      text: 'native-first',
    })
    expect(writeText).not.toHaveBeenCalled()
  })

  it('Clipboard API 失败后调用 Tauri 原生剪贴板命令', async () => {
    setClipboard({ writeText: vi.fn().mockRejectedValue(new Error('NotAllowedError')) })
    setInternals({})
    mockInvoke.mockResolvedValue(undefined)
    await expect(copyText('native')).resolves.toBe(true)
    expect(mockInvoke).toHaveBeenCalledWith('plugin:miyo|clipboard_write_text', { text: 'native' })
  })

  it('Tauri 命令失败后继续降级 execCommand', async () => {
    setClipboard(undefined)
    setInternals({})
    setExecCommand(vi.fn().mockReturnValue(true))
    mockInvoke.mockRejectedValue(new Error('ipc error'))
    await expect(copyText('fallback')).resolves.toBe(true)
    expect(mockInvoke).toHaveBeenCalledWith('plugin:miyo|clipboard_write_text', { text: 'fallback' })
    expect(document.execCommand).toHaveBeenCalledWith('copy')
  })
})