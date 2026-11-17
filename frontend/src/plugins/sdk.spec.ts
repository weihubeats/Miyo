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

  it('commands.invoke：java.debug 已批准 → 放行且带 IPC 前缀', async () => {
    invokeMock.mockResolvedValue([])
    const { context } = createPluginSession(javaView(), hostBridge)
    const projects = await context.commands.invoke('list_java_projects', { projectId: null })
    expect(projects).toEqual([])
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|list_java_projects', { projectId: null })
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
  it('storage：调用携带 pluginId（后端按 id 隔离；__config 键保留给配置）', async () => {
    // 会话创建即预取 __config（消费一次 get）——统一 resolve，避免 Once 被预取吃掉
    invokeMock.mockResolvedValue({ a: 1 })
    const { context } = createPluginSession(javaView(), hostBridge)
    await context.storage.set('favorite', ['x'])
    expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_data_set', {
      id: 'com.miyo.java-debug',
      key: 'favorite',
      value: ['x'],
    })
    const got = await context.storage.get('favorite')
    expect(got).toEqual({ a: 1 })
    expect(invokeMock).toHaveBeenLastCalledWith('plugin:miyo|plugin_data_get', {
      id: 'com.miyo.java-debug',
      key: 'favorite',
    })
  })

  it('config：set 合并写入 __config 键', async () => {
    const { context } = createPluginSession(javaView(), hostBridge)
    await context.config.set({ audit_log: false })
    await context.config.set({ extra: 1 })
    expect(invokeMock).toHaveBeenLastCalledWith('plugin:miyo|plugin_data_set', {
      id: 'com.miyo.java-debug',
      key: '__config',
      value: { audit_log: false, extra: 1 },
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

  it('logger：audit/info 落后端（fire-and-forget，携带 pluginId）', async () => {
    invokeMock.mockResolvedValue(null)
    const { context } = createPluginSession(javaView(), hostBridge)
    context.logger.audit('invoke demo#method status=ok')
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('plugin:miyo|plugin_record_log', {
        id: 'com.miyo.java-debug',
        level: 'audit',
        message: 'invoke demo#method status=ok',
      })
    })
  })
})
