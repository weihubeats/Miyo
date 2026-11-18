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

  it('合法请求 → 成功响应帧（storage 透传 pluginId + sessionToken）', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'plugin:miyo|plugin_session_create') return Promise.resolve('tok-bridge')
      return Promise.resolve(['a', 'b'])
    })
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
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_data_keys', {
      id: 'com.miyo.java-debug',
      sessionToken: 'tok-bridge',
    })
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
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'plugin:miyo|plugin_session_create') return Promise.resolve('tok-t')
      return new Promise(() => undefined) // 永不 resolve
    })
    const { bridge, outFrames } = makeRealBridge(50)
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'r4',
      method: 'commands.invoke',
      params: { command: 'java_invoke' },
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

  // ---------------- RPC 加固（Phase 2 安全地基） ----------------

  it('重放防护：同一请求 ID 二次到达被丢弃', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'plugin:miyo|plugin_session_create') return Promise.resolve('tok-r')
      return Promise.resolve(null)
    })
    const { bridge, outFrames } = makeRealBridge()
    const frame = { v: RPC_VERSION, type: 'req', id: 'dup-1', method: 'config.get' }
    bridge.handleMessage(frame)
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    bridge.handleMessage(frame) // 重放
    bridge.handleMessage({ ...frame }) // 内容相同、对象不同——同 id
    await new Promise((r) => setTimeout(r, 20))
    expect(outFrames).toHaveLength(1) // 只有一次响应
  })

  it('帧体积上限：超限静默丢弃', () => {
    const { bridge, outFrames } = makeRealBridge()
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'big-1',
      method: 'config.set',
      params: { values: { blob: 'x'.repeat(1_100_000) } },
    })
    expect(outFrames).toHaveLength(0)
  })

  it('JSON 深度上限 → BAD_REQUEST', async () => {
    const { bridge, outFrames } = makeRealBridge()
    let deep: unknown = 'leaf'
    for (let i = 0; i < 20; i++) deep = { v: deep }
    bridge.handleMessage({
      v: RPC_VERSION,
      type: 'req',
      id: 'deep-1',
      method: 'config.set',
      params: { values: deep as Record<string, unknown> },
    })
    await vi.waitFor(() => expect(outFrames).toHaveLength(1))
    const frame = outFrames[0] as { ok: boolean; error: { code: string } }
    expect(frame.error.code).toBe(RPC_ERRORS.BAD_REQUEST)
  })

  it('并发上限：超出在途数的请求立即 RATE_LIMITED', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'plugin:miyo|plugin_session_create') return Promise.resolve('tok-c')
      return new Promise(() => undefined) // 挂起占满在途
    })
    const { bridge, outFrames } = makeRealBridge(5_000)
    for (let i = 0; i < 20; i++) {
      bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: `in-${i}`, method: 'config.get' })
    }
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'over', method: 'config.get' })
    await vi.waitFor(() =>
      expect(outFrames.some((f) => (f as { id: string }).id === 'over')).toBe(true),
    )
    const frame = outFrames.find((f) => (f as { id: string }).id === 'over') as {
      ok: boolean
      error: { code: string }
    }
    expect(frame.error.code).toBe(RPC_ERRORS.RATE_LIMITED)
  })

  it('订阅上限：超过 32 个订阅报错', async () => {
    const { bridge, outFrames } = makeRealBridge()
    // 逐条让出微任务（dispatch 是异步的；同步连发会先撞并发上限）
    for (let i = 0; i < 33; i++) {
      bridge.handleMessage({
        v: RPC_VERSION,
        type: 'req',
        id: `sub-${i}`,
        method: 'events.subscribe',
        params: { event: `e${i}` },
      })
      await new Promise((r) => setTimeout(r, 0))
    }
    await vi.waitFor(() => expect(outFrames.length).toBeGreaterThanOrEqual(33))
    const over = outFrames.find((f) => (f as { id: string }).id === "sub-32") as {
      ok: boolean
      error: { code: string; message: string }
    }
    expect(over.ok).toBe(false)
    expect(over.error.message).toContain('PLUGIN_SUBSCRIPTION_LIMIT')
  })

  it('nonce：携带不匹配 n 的帧被静默丢弃；v1 无 n 帧保持兼容', () => {
    const outFrames: unknown[] = []
    const session = createPluginSession(javaView(), hostBridge)
    const bridge = new HostRpcBridge({
      sourceWindow: fakeWindow,
      postMessage: (frame) => outFrames.push(frame),
      session,
      timeoutMs: 500,
      nonce: 'abc123',
    })
    // 带错 nonce → 丢弃
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'n-bad', method: 'config.get', n: 'wrong' })
    // 带对 nonce → 正常处理
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'n-ok', method: 'config.get', n: 'abc123' })
    // 不带 nonce（v1 插件）→ 兼容放行
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'n-legacy', method: 'config.get' })
    return vi.waitFor(() => {
      const ids = outFrames.map((f) => (f as { id: string }).id)
      expect(ids).toContain('n-ok')
      expect(ids).toContain('n-legacy')
      expect(ids).not.toContain('n-bad')
    })
  })

  it('熔断：错误风暴后进入打开期（RATE_LIMITED 快速失败）', async () => {
    invokeMock.mockRejectedValue({ code: 'INTERNAL', message: 'boom' })
    const { bridge, outFrames } = makeRealBridge()
    // 逐条等待分发落地（错误计入熔断窗口；同步连发会先撞并发上限）
    for (let i = 0; i < 50; i++) {
      bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: `err-${i}`, method: 'storage.keys' })
      await new Promise((r) => setTimeout(r, 0))
    }
    await vi.waitFor(() => expect(outFrames.length).toBeGreaterThanOrEqual(50))
    // 第 51 次请求：熔断打开 → RATE_LIMITED（不再触发 SDK / IPC）
    const before = invokeMock.mock.calls.length
    bridge.handleMessage({ v: RPC_VERSION, type: 'req', id: 'post-breaker', method: 'config.get' })
    await vi.waitFor(() =>
      expect(outFrames.some((f) => (f as { id: string }).id === 'post-breaker')).toBe(true),
    )
    const frame = outFrames.find((f) => (f as { id: string }).id === 'post-breaker') as {
      ok: boolean
      error: { code: string }
    }
    expect(frame.error.code).toBe(RPC_ERRORS.RATE_LIMITED)
    expect(invokeMock.mock.calls.length).toBe(before) // 熔断期不经 IPC
  })
})
