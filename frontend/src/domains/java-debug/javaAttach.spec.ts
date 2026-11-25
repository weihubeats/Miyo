/**
 * javaAttach store spec：Agent 数据归一（方法 / 类 → v1 形状）、attach 会话
 * 状态机（attach → detach → 重连）、收藏 / 实例策略。IPC 全 mock（不触后端）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useJavaAttachStore, normalizeClass, normalizeMethod } from './stores/javaAttach'
import { useLocaleStore } from '@/stores/locale'
import type { JavaMethodInfo } from '@/types/miyoApi.d'

// IPC mock（useMiyoApi 在 store setup 中取用）
const mockApi = {
  javaDiscoverProcesses: vi.fn(),
  javaAttachProcess: vi.fn(),
  javaDetachProcess: vi.fn(),
  javaUnloadAgent: vi.fn(),
  javaAttachSessions: vi.fn(),
  javaAgentClasses: vi.fn(),
  javaAgentMethods: vi.fn(),
  javaAgentBeans: vi.fn(),
  javaAgentSpringStatus: vi.fn(),
  javaAgentInvoke: vi.fn(),
  javaAgentCancel: vi.fn(),
  javaProbePid: vi.fn(),
  listJavaProjects: vi.fn(),
  listJavaHistory: vi.fn(),
}

vi.mock('@/composables/useMiyoApi', () => ({
  useMiyoApi: () => mockApi,
}))

const snapshot = {
  pid: 4242,
  vm_start_ms: 1_000,
  agent_version: '2.0.0',
  protocol: 2,
  handshake: {
    protocol: 2,
    agent_version: '2.0.0',
    pid: 4242,
    vm_start_ms: 1_000,
    java_version: '21.0.6',
    java_vm_version: '21.0.6',
    jvm_vendor: 'Azul',
    java_home: '/jdk21',
    max_concurrent: 4,
    max_timeout_ms: 300_000,
    uptime_ms: 5,
  },
  launcher_java_home: '/jdk21',
  attached_at: '2026-11-18T00:00:00Z',
  reattached: false,
}

const agentMethod = {
  method_id: 'com.example.C#add(long,long)',
  class_name: 'com.example.C',
  declaring_class: 'com.example.C',
  method_name: 'add',
  signature: 'static long add(long, long)',
  params: [
    { name: 'a', type: 'long', generic: '', primitive: true },
    { name: 'b', type: 'long', generic: '', primitive: true },
  ],
  param_types: ['long', 'long'],
  return_type: 'long',
  return_generic: '',
  static: true,
  visibility: 'public',
  synthetic: false,
  bridge: false,
  varargs: false,
  overloaded: false,
  risk: 'safe',
  risk_reason: null,
}

describe('normalize helpers', () => {
  it('agent 方法 → v1 形状（JavaMethodPanel 复用）', () => {
    const normalized = normalizeMethod(agentMethod as never)
    expect(normalized.method_id).toBe('com.example.C#add(long,long)')
    expect(normalized.static_method).toBe(true)
    expect(normalized.params[0]).toEqual({
      name: 'a',
      type_name: 'long',
      generic_type: '',
      required: true,
    })
    expect(normalized.overloaded).toBe(false)
  })

  it('agent 类行 → v1 形状（method_count 未知为 -1）', () => {
    const normalized = normalizeClass({
      name: 'com.example.PlainApp',
      package_name: 'com.example',
      simple_name: 'PlainApp',
      jdk_internal: false,
      array: false,
      synthetic: false,
      loader: { id: '1', name: 'AppClassLoader', identity: 1 },
      category: 'application',
      role: 'other',
      bean_name: 'plainApp',
      source: {
        kind: 'app_classes_dir',
        location: 'file:/app/target/classes/',
        label: 'classes',
        evidence: '目录型 classes 输出',
      },
    })
    expect(normalized.class_name).toBe('com.example.PlainApp')
    expect(normalized.method_count).toBe(-1)
    expect(normalized.bean_name).toBe('plainApp')
    // 分类信息透传（分组树 + 证据 tooltip）
    expect(normalized.category).toBe('application')
    expect(normalized.role).toBe('other')
    expect(normalized.category_evidence).toBe('目录型 classes 输出')
  })
})

describe('javaAttach store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    vi.clearAllMocks()
    mockApi.javaDiscoverProcesses.mockResolvedValue({
      processes: [],
      toolchain_count: 2,
      launcher_ready: true,
      agent_ready: true,
      scan_ms: 5,
      hints: [],
    })
    mockApi.javaAgentClasses.mockResolvedValue({
      total: 1,
      returned: 1,
      offset: 0,
      items: [
        { name: 'com.example.PlainApp', package_name: 'com.example', simple_name: 'PlainApp' },
      ],
    })
    mockApi.javaAgentMethods.mockResolvedValue({
      class_name: 'com.example.C',
      methods: [agentMethod],
      can_instantiate: true,
      proxy: false,
    })
    mockApi.listJavaProjects.mockResolvedValue([
      {
        id: 'proj-1',
        name: 'PID 4242',
        mode: 'attach',
        attach_pid: 4242,
        runtime_endpoint: 'attach://4242',
        timeout_ms: 30000,
        max_result_bytes: 1048576,
        max_concurrent: 4,
        description: '',
        created_at: '',
        updated_at: '',
      },
    ])
    mockApi.listJavaHistory.mockResolvedValue([])
  })

  it('attach 成功：会话入表 + 类清单加载 + 历史项目解析', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    const store = useJavaAttachStore()
    const ok = await store.attach(4242)
    expect(ok).toBe(true)
    expect(store.attached).toBe(true)
    expect(store.activePid).toBe(4242)
    expect(store.classes).toHaveLength(1)
    expect(store.classes[0].class_name).toBe('com.example.PlainApp')
    await store.loadHistory()
    expect(store.historyProjectId).toBe('proj-1')
  })

  it('attach 后方法选择：v1 形状 + 原始 agent 方法并存', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    const store = useJavaAttachStore()
    await store.attach(4242)
    await store.selectClass('com.example.C')
    expect(store.methods[0].method_id).toBe('com.example.C#add(long,long)')
    store.selectMethod(store.methods[0])
    expect(store.activeAgentMethod?.risk).toBe('safe')
    // 实例策略默认 auto → 静态方法归 static
    await store.invoke()
    expect(mockApi.javaAgentInvoke).toHaveBeenCalled()
    const call = mockApi.javaAgentInvoke.mock.calls[0]
    expect(call[1].instance.kind).toBe('static')
    expect(call[1].paramTypes).toEqual(['long', 'long'])
  })

  it('detach：会话移除 + 发现状态清空', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    mockApi.javaDetachProcess.mockResolvedValue(undefined)
    const store = useJavaAttachStore()
    await store.attach(4242)
    expect(store.attached).toBe(true)
    await store.detach(4242)
    expect(store.attached).toBe(false)
    expect(store.classes).toHaveLength(0)
    expect(store.activePid).toBeNull()
  })

  it('invoke 业务异常不算链路错误：响应原样保留', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    mockApi.javaAgentInvoke.mockResolvedValue({
      invocation_id: 'i-1',
      method_id: 'com.example.C#add(long,long)',
      status: 'failed',
      result: null,
      result_truncated: false,
      exception: {
        exception_type: 'java.lang.ArithmeticException',
        message: '除数不能为 0',
        stack_trace: ['com.example.C.add(C.java:9)'],
        cause_chain: [],
      },
      duration_ms: 1.2,
    })
    const store = useJavaAttachStore()
    await store.attach(4242)
    await store.selectClass('com.example.C')
    store.selectMethod(store.methods[0])
    const resp = await store.invoke()
    expect(resp?.status).toBe('failed')
    expect(resp?.exception?.exception_type).toBe('java.lang.ArithmeticException')
    expect(store.lastResponse?.terminated).toBeUndefined()
  })

  it('断开后会话不复活：扫描结果的滞后 attached 不得重建会话', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    const store = useJavaAttachStore()
    await store.attach(4242)
    expect(store.sessions.has(4242)).toBe(true)

    // 断开：后端会话移除，本地也移除
    mockApi.javaDetachProcess.mockResolvedValue(undefined)
    mockApi.javaAttachSessions.mockResolvedValue([])
    await store.detach(4242)
    expect(store.sessions.has(4242)).toBe(false)

    // 下一轮轮询：进程扫描快照仍是断开前的旧值（attached=true）
    mockApi.javaDiscoverProcesses.mockResolvedValue({
      processes: [
        {
          pid: 4242,
          main_class: 'com.example.App',
          display_name: 'com.example.App',
          command_line: 'java com.example.App',
          java_executable: '/jdk/bin/java',
          java_home: '/jdk',
          java_version: '17.0.16',
          jvm_vendor: 'Azul',
          kind: 'jdk',
          started_at: '',
          elapsed: '01:00',
          uid: '501',
          attachable: true,
          attach_block_reason: null,
          attached: true,
          source: 'os_scan',
        },
      ],
      toolchain_count: 1,
      launcher_ready: true,
      agent_ready: true,
      scan_ms: 3,
      hints: [],
    })
    await store.loadProcesses()
    // 会话表仍是空的（旧实现会在这里把 4242 塞回来，界面「已连接」复活）
    expect(store.sessions.has(4242)).toBe(false)
  })

  it('参数骨架：集合 / Map 给可执行默认，DTO 用上次参数回填', async () => {
    localStorage.clear()
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    const store = useJavaAttachStore()
    await store.attach(4242)

    const bare: JavaMethodInfo = {
      method_id: 'com.example.C#build(java.util.List,java.util.Map)',
      class_name: 'com.example.C',
      method_name: 'build',
      signature: 'void build(List, Map)',
      return_type: 'void',
      return_generic: 'void',
      static_method: false,
      visibility: 'public',
      overloaded: false,
      declaring_class: 'com.example.C',
      params: [
        { name: 'items', type_name: 'java.util.List', generic_type: 'java.util.List<java.lang.String>', required: true },
        { name: 'extra', type_name: 'java.util.Map', generic_type: 'java.util.Map<java.lang.String,java.lang.Object>', required: false },
      ],
    }
    // activeMethod 由 methods 表反查：先把方法放进列表
    ;(store as unknown as { methods: unknown[] }).methods = [bare]
    store.selectMethod(bare)
    expect(JSON.parse(store.argsJson)).toEqual([[], {}])

    // 填好参数后调用：调用成功即记住（免去重跑重填）
    store.argsJson = JSON.stringify([['BTC'], { k: 1 }])
    mockApi.javaAgentInvoke.mockResolvedValue({
      invocation_id: 'inv-1',
      method_id: bare.method_id,
      status: 'success',
      duration_ms: 1,
      result: null,
      instance_kind: 'static',
      terminated: true,
    })
    await store.invoke()
    store.selectMethod(null)
    store.selectMethod(bare)
    expect(JSON.parse(store.argsJson)).toEqual([['BTC'], { k: 1 }])

    // 恢复默认：清空记忆后回到骨架
    store.clearRememberedArgs()
    store.selectMethod(bare)
    expect(JSON.parse(store.argsJson)).toEqual([[], {}])
    localStorage.clear()
  })

  it('dispose：停止轮询 + 清空状态（插件停用回收路径）', async () => {
    mockApi.javaAttachProcess.mockResolvedValue(snapshot)
    mockApi.javaAgentSpringStatus.mockResolvedValue({
      spring_present: false,
      contexts: [],
      capture_state: 'disabled',
    })
    const store = useJavaAttachStore()
    store.setAutoRefresh(true)
    await store.attach(4242)
    store.dispose()
    expect(store.autoRefresh).toBe(false)
    expect(store.classes).toHaveLength(0)
  })
})
