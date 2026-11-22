/**
 * PluginSDK 单测：权限门禁（env / notify / events / commands 白名单）、
 * 存储作用域（pluginId 随调用透传，后端强制隔离）、事件解绑（dispose）、
 * 插件间事件命名空间隔离。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { createPluginSession, type PluginHostBridge } from '@/plugins/sdk'
import { pluginEventBus } from '@/plugins/events'
import { useLocaleStore } from '@/stores/locale'
import { javaView } from '@/testUtils/pluginFixture'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    toast: vi.fn(),
    dismiss: vi.fn(),
  }),
}))

const hostBridge: PluginHostBridge = {
  registerNav: vi.fn(),
  unregisterNav: vi.fn(),
  registerRoute: vi.fn(),
  unregisterRoute: vi.fn(),
  registerCommand: vi.fn(),
  unregisterCommand: vi.fn(),
}

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockReset()
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

describe('PluginSDK 权限门禁', () => {
  it('commands.invoke：白名单外命令拒绝', async () => {
    const { context } = createPluginSession(javaView(), hostBridge)
    await expect(context.commands.invoke('set_data_dir', {})).rejects.toThrow('PLUGIN_COMMAND_DENIED')
  })

  it('commands.invoke：白名单内但权限未批准 → 拒绝', async () => {
    // java.debug 声明了但用户未批准
    const view = javaView({ view: { granted_permissions: [] } })
    const { context } = createPluginSession(view, hostBridge)
    await expect(context.commands.invoke('java_invoke', {})).rejects.toThrow('PLUGIN_PERMISSION_DENIED')
  })

  it('commands.invoke：java.debug 聚合权限展开 → java.method.invoke 放行且带 IPC 前缀', async () => {
    // java.debug（存量行）等价全部 java.* 子权限：java_invoke 需要 java.method.invoke
    invokeMock.mockResolvedValue({ status: 'success' })
    const { context } = createPluginSession(javaView(), hostBridge)
    const res = await context.commands.invoke('java_invoke', { id: 'x' })
    expect(res).toEqual({ status: 'success' })
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|java_invoke', { id: 'x' })
  })

  it('commands.invoke：显式子权限不足 → 拒绝（数据管理命令不再白名单）', async () => {
    // list_java_projects 是数据管理命令：SDK 白名单不登记（宿主 useMiyoApi 专用）
    invokeMock.mockResolvedValue([])
    const { context } = createPluginSession(javaView(), hostBridge)
    await expect(context.commands.invoke('list_java_projects', {})).rejects.toThrow(
      'PLUGIN_COMMAND_DENIED',
    )
  })

  it('env：std.env.read 未批准 → 抛权限错误', async () => {
    const view = javaView({ view: { granted_permissions: [] } })
    const { context } = createPluginSession(view, hostBridge)
    await expect(context.env.getVariableMap()).rejects.toThrow('PLUGIN_PERMISSION_DENIED')
  })

  it('env：已批准 → 返回变量合并表', async () => {
    const { context } = createPluginSession(javaView(), hostBridge)
    // workspace store 在无 IPC 的 jsdom 下以空态降级（合并结果为空表）
    const vars = await context.env.getVariableMap()
    expect(vars).toBeTypeOf('object')
  })

  it('notify：std.notify 未批准 → 抛权限错误', () => {
    const view = javaView({ view: { granted_permissions: [] } })
    const { context } = createPluginSession(view, hostBridge)
    expect(() => context.notify.info('hi')).toThrow('PLUGIN_PERMISSION_DENIED')
  })

  it('notify：已批准 → 走 Toast', () => {
    const { context } = createPluginSession(javaView(), hostBridge)
    expect(() => context.notify.info('hi')).not.toThrow()
  })

  it('events：std.events 未批准 → 抛权限错误', () => {
    const view = javaView({ view: { granted_permissions: [] } })
    const { context } = createPluginSession(view, hostBridge)
    expect(() => context.events.on('x', () => undefined)).toThrow('PLUGIN_PERMISSION_DENIED')
  })
})

describe('PluginSDK 存储与事件', () => {
  /** 会话凭证 mock：plugin_session_create 返回固定 token，其余按 args.key 返回。 */
  function mockSessionToken(): string {
    const token = 'tok-1234'
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'plugin:miyo|plugin_session_create') return Promise.resolve(token)
      if (cmd === 'plugin:miyo|plugin_data_get') return Promise.resolve({ a: 1 })
      return Promise.resolve(null)
    })
    return token
  }

  it('storage：调用携带 pluginId + sessionToken（后端按 id 隔离 + 会话校验）', async () => {
    const token = mockSessionToken()
    const { context } = createPluginSession(javaView(), hostBridge)
    await context.storage.set('favorite', ['x'])
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_data_set', {
      id: 'com.miyo.java-debug',
      key: 'favorite',
      value: ['x'],
      sessionToken: token,
    })
    const got = await context.storage.get('favorite')
    expect(got).toEqual({ a: 1 })
    expect(invokeMock).toHaveBeenLastCalledWith('plugin:miyo|plugin_data_get', {
      id: 'com.miyo.java-debug',
      key: 'favorite',
      sessionToken: token,
    })
  })

  it('storage：会话失效（FORBIDDEN）→ 重建会话重试一次（撤销即时生效）', async () => {
    const dataGetTokens: string[] = []
    let tokenCounter = 0
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "plugin:miyo|plugin_session_create") return Promise.resolve(`tok-${++tokenCounter}`)
      if (cmd === "plugin:miyo|plugin_data_get") {
        dataGetTokens.push(args?.sessionToken as string)
        return Promise.reject({ code: 'FORBIDDEN', message: 'session expired' })
      }
      return Promise.resolve(null)
    })
    const { context } = createPluginSession(javaView(), hostBridge)
    // activate 预取 + 本次 get 各自走「失败 → 重建会话 → 重试」路径，
    // 最终 FORBIDDEN 如实上抛；重试一律使用新签发的 token。
    await expect(context.storage.get('k')).rejects.toMatchObject({ code: 'FORBIDDEN' })
    await vi.waitFor(() => expect(dataGetTokens.length).toBeGreaterThanOrEqual(4))
    expect(new Set(dataGetTokens).size).toBeGreaterThanOrEqual(2)
  })

  it('config：set 合并写入 __config 键', async () => {
    const token = mockSessionToken()
    const { context } = createPluginSession(javaView(), hostBridge)
    await context.config.set({ audit_log: false })
    await context.config.set({ extra: 1 })
    expect(invokeMock).toHaveBeenLastCalledWith('plugin:miyo|plugin_data_set', {
      id: 'com.miyo.java-debug',
      key: '__config',
      value: { audit_log: false, extra: 1 },
      sessionToken: token,
    })
    expect(context.config.get()).toEqual({ audit_log: false, extra: 1 })
  })

  it('events：插件命名空间隔离 + dispose 统一解绑', () => {
    const a = createPluginSession(javaView(), hostBridge)
    const b = createPluginSession(javaView(), hostBridge)

    const seenA: unknown[] = []
    a.context.events.on('ping', (p) => seenA.push(p))
    const seenBus = vi.fn()
    pluginEventBus.on('com.miyo.java-debug:ping', seenBus)

    // A emit 只进 A 的 handler（同 id 命名空间）与总线
    a.context.events.emit('ping', 1)
    expect(seenA).toEqual([1])
    expect(seenBus).toHaveBeenCalledWith(1)

    // dispose 后 A 的 handler 解绑（不再收到新事件）；总线级测试监听不受影响
    a.dispose()
    a.context.events.emit('ping', 2)
    expect(seenA).toEqual([1])
    expect(seenBus).toHaveBeenCalledTimes(2)
    expect(pluginEventBus.size).toBe(1) // 仅剩总线级测试监听
    pluginEventBus.emit('com.miyo.java-debug:ping', 3)
    expect(seenBus).toHaveBeenCalledTimes(3)
    b.dispose()
  })

  it('logger：audit/info 落后端（fire-and-forget，携带 pluginId + sessionToken）', async () => {
    const token = mockSessionToken()
    const { context } = createPluginSession(javaView(), hostBridge)
    context.logger.audit('invoke demo#method status=ok')
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_record_log', {
        id: 'com.miyo.java-debug',
        level: 'audit',
        message: 'invoke demo#method status=ok',
        sessionToken: token,
      })
    })
  })
})
