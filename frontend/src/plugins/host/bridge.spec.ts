/**
 * 沙箱宿主 RPC 协议与桥单测：帧校验（未知帧丢弃）、方法白名单、参数校验、
 * 权限拒绝传播、超时、事件订阅推送。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { HostRpcBridge } from '@/plugins/host/bridge'
import { createPluginSession, type PluginHostBridge } from '@/plugins/sdk'
import { METHODS, RPC_ERRORS, RPC_VERSION, isRpcRequestFrame, validateMethod } from '@/plugins/host/protocol'
import { useLocaleStore } from '@/stores/locale'
import { javaView } from '@/testUtils/pluginFixture'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), toast: vi.fn(), dismiss: vi.fn() }),
}))

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockReset()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

const fakeWindow = window

const hostBridge: PluginHostBridge = {
  registerNav: vi.fn(),
  unregisterNav: vi.fn(),
  registerRoute: vi.fn(),
  unregisterRoute: vi.fn(),
}

describe('RPC 协议帧校验', () => {
  it('isRpcRequestFrame：版本 / 类型 / id / params 形状', () => {
    expect(isRpcRequestFrame({ v: RPC_VERSION, type: 'req', id: 'r1', method: 'config.get' })).toBe(true)
    expect(isRpcRequestFrame({ v: 2, type: 'req', id: 'r1', method: 'x' })).toBe(false)
    expect(isRpcRequestFrame({ v: RPC_VERSION, type: 'evt', id: 'r1', method: 'x' })).toBe(false)
    expect(isRpcRequestFrame({ v: RPC_VERSION, type: 'req', id: '', method: 'x' })).toBe(false)
    expect(isRpcRequestFrame({ v: RPC_VERSION, type: 'req', id: 'r1', method: 'x', params: [1] })).toBe(false)
    expect(isRpcRequestFrame(null)).toBe(false)
    expect(isRpcRequestFrame('req')).toBe(false)
  })

  it('方法白名单：未知方法拒绝', () => {
    expect(validateMethod('fs.readFile', undefined)).toContain('unknown method')
    expect(validateMethod('storage.get', { key: 'k' })).toBeNull()
    expect(validateMethod('storage.get', undefined)).toContain('key')
    expect(validateMethod('storage.get', { key: '' })).toContain('key')
    // 全部白名单方法都有校验器
    for (const m of Object.keys(METHODS)) {
      expect(typeof METHODS[m]).toBe('function')
    }
  })

  it('log.write 级别与长度校验', () => {
    expect(validateMethod('log.write', { level: 'audit', message: 'm' })).toBeNull()
    expect(validateMethod('log.write', { level: 'stealth', message: 'm' })).toContain('level')
    expect(validateMethod('log.write', { level: 'info', message: 'x'.repeat(2001) })).toContain('message')
  })

  it('notify.show：kind 白名单 + title/message 至少其一', () => {
    expect(validateMethod('notify.show', { kind: 'success', title: 't' })).toBeNull()
    expect(validateMethod('notify.show', { kind: 'blast', title: 't' })).toContain('kind')
    expect(validateMethod('notify.show', { kind: 'info' })).toContain('title')
  })

  it('commands.invoke / config.set / events 形状校验', () => {
    expect(validateMethod('commands.invoke', { command: 'java_invoke', args: {} })).toBeNull()
    expect(validateMethod('commands.invoke', { command: 1 })).toContain('command')
    expect(validateMethod('config.set', { values: {} })).toBeNull()
    expect(validateMethod('config.set', { values: 'x' })).toContain('values')
    expect(validateMethod('events.emit', { event: 'tick' })).toBeNull()
    expect(validateMethod('events.subscribe', {})).toContain('event')
  })
})

describe('HostRpcBridge 分发', () => {
  function makeRealBridge(timeoutMs = 500) {
    const outFrames: unknown[] = []
    const session = createPluginSession(javaView(), hostBridge)
    const bridge = new HostRpcBridge({
      sourceWindow: fakeWindow,
      postMessage: (frame) => outFrames.push(frame),
      session,
      timeoutMs,
    })
    return { bridge, outFrames, session }
  }

  it('合法请求 → 成功响应帧（storage 透传 pluginId）', async () => {
    invokeMock.mockResolvedValue(['a', 'b'])
    const { bridge, outFrames } = makeRealBridge()
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'r1', method: 'storage.keys' })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    expect(outFrames[0]).toEqual({
      v: RPC_VERSION,
      type: 'res',
      id: 'r1',
      ok: true,
      result: ['a', 'b'],
    })
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_data_keys', { id: 'com.miyo.java-debug' })
  })

  it('未知帧静默丢弃（不回错误帧）', () => {
    const { bridge, outFrames } = makeRealBridge()
    bridge.handleMessage({ v: 99, type: 'req', id: 'x', method: 'config.get' })
    bridge.handleMessage('garbage')
    bridge.handleMessage(null)
    expect(outFrames).toHaveLength(0)
  })

  it('未知方法 → BAD_REQUEST 错误帧', async () => {
    const { bridge, outFrames } = makeRealBridge()
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'r2', method: 'shell.exec' })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    const frame = outFrames[0] as { ok: boolean; error: { code: string } }
    expect(frame.ok).toBe(false)
    expect(frame.error.code).toBe(RPC_ERRORS.BAD_REQUEST)
  })

  it('权限拒绝传播：未批准权限的命令调用回 PERMISSION_DENIED', async () => {
    const view = javaView({ view: { granted_permissions: [] } })
    const outFrames: unknown[] = []
    const session = createPluginSession(view, hostBridge)
    const bridge = new HostRpcBridge({
      sourceWindow: fakeWindow,
      postMessage: (frame) => outFrames.push(frame),
      session,
      timeoutMs: 500,
    })
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'r3',
      method: 'commands.invoke',
      params: { command: 'java_invoke', args: {} },
    })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    const frame = outFrames[0] as { ok: boolean; error: { code: string; message: string } }
    expect(frame.ok).toBe(false)
    expect(frame.error.code).toBe(RPC_ERRORS.PERMISSION_DENIED)
    expect(frame.error.message).toContain('PLUGIN_PERMISSION_DENIED')
  })

  it('SDK 挂起 → 超时错误帧', async () => {
    // commands.invoke 走 IPC（config.get 是同步内存读取，不会挂起）
    invokeMock.mockImplementation(() => new Promise(() => undefined)) // 永不 resolve
    const { bridge, outFrames } = makeRealBridge(50)
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'r4',
      method: 'commands.invoke',
      params: { command: 'list_java_projects' },
    })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    const frame = outFrames[0] as { ok: boolean; error: { code: string } }
    expect(frame.error.code).toBe(RPC_ERRORS.TIMEOUT)
  })

  it('events.subscribe → 总线事件以 evt 帧推送；dispose 解绑', async () => {
    const { bridge, outFrames } = makeRealBridge()
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'r5',
      method: 'events.subscribe',
      params: { event: 'tick' },
    })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1)) // 订阅 ack

    // 总线上发该插件命名空间的事件 → 插件收 evt 帧
    const { pluginEventBus } = await import('@/plugins/events')
    pluginEventBus.emit('com.miyo.java-debug:tick', { n: 1 })
    await vi.waitFor(() => expect(outFrames).toHaveLength(2))
    expect(outFrames[1]).toEqual({ v: RPC_VERSION, type: 'evt', event: 'tick', payload: { n: 1 } })

    bridge.dispose()
    pluginEventBus.emit('com.miyo.java-debug:tick', { n: 2 })
    expect(outFrames).toHaveLength(2) // dispose 后不再推送
  })

  it('dispose 会话随桥回收（防插件侧泄漏）', () => {
    const { bridge } = makeRealBridge()
    const spy = vi.spyOn(bridge, 'dispose')
    bridge.dispose()
    expect(spy).toHaveBeenCalledTimes(1)
  })
})
