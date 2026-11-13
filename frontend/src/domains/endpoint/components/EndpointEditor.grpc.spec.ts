/**
 * EndpointEditor gRPC 分支单测：方法切 GRPC 后的页签/发送分流/响应区。
 *
 * 锁定回归：
 * - GRPC 方法：配置页签只剩 gRPC + 元数据（无 Body/Params/Code），默认激活 gRPC；
 * - 切到 GRPC：request.body 自动初始化为 grpc 配置（applyMethodDefaults）；
 * - 发送分流：GRPC 走 grpcInvoke（unary 结果进 GrpcResponsePanel，不再走 executeRequest）；
 * - 服务端流：返回 stream_id 后响应区展示流时间线。
 */
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint, GrpcInvokeResult } from '@/types/miyoApi'

const apiMock = vi.hoisted(() => ({
  grpcInvoke: vi.fn(),
  grpcListServices: vi.fn(async () => ({ services: [], source: 'reflection' })),
  grpcListProtoFiles: vi.fn(async () => []),
  listExamples: vi.fn(async () => []),
  listRequestExamples: vi.fn(async () => []),
  listTestCases: vi.fn(async () => []),
  listRequestHistories: vi.fn(async () => []),
  listEnvironments: vi.fn(async () => []),
  getActiveEnvironment: vi.fn(async () => null),
  getGlobalVariables: vi.fn(async () => []),
  getGlobalParams: vi.fn(async () => []),
  executeRequest: vi.fn(),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

async function mountEditor(prepare?: (ep: Endpoint) => void) {
  setActivePinia(createPinia())
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  const store = useWorkspaceStore()
  store.project = {
    id: 'proj-test-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  const ep = makeDraft({ id: 'ep-grpc-1', name: 'gRPC 调试', method: 'GET', path: '/x' })
  prepare?.(ep)
  store.endpoints = [ep]
  const wrapper = mount(EndpointEditor, {
    global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
  })
  await nextTick()
  store.openEndpoint(ep)
  await nextTick()
  await nextTick()
  return { wrapper, store, ep }
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('EndpointEditor：GRPC 方法分支', () => {
  it('切到 GRPC：body 初始化 grpc 配置，页签只剩 gRPC + 元数据，默认激活 gRPC', async () => {
    const { wrapper, ep, store } = await mountEditor()
    // 编辑器操作的是草稿（openEndpoint 克隆），模拟用户在方法下拉切到 GRPC
    const draft = store.draftOf(ep.id)
    expect(draft).toBeTruthy()
    draft!.method = 'GRPC'
    await nextTick()
    await nextTick()

    expect(draft!.request.body.mode).toBe('grpc')
    const tabs = wrapper.findAll('.tabs .tab .tab-label').map((t) => t.text())
    expect(tabs).toEqual(['gRPC', 'Headers'])
    // 激活页签为 gRPC（GrpcPanel 已渲染：出现服务输入框）
    expect(wrapper.findAll('input.miyo-input').length).toBeGreaterThan(0)
    // 代码生成入口隐藏
    expect(wrapper.findComponent({ name: 'CodeExportMenu' }).exists()).toBe(false)
  })

  it('GRPC unary 发送：走 grpcInvoke，响应区展示 grpc-status 与消息', async () => {
    const result: GrpcInvokeResult = {
      kind: 'unary',
      response: {
        message_json: '{"fString":"Miyo"}',
        metadata: [['grpc-status', '0']],
        grpc_status: 0,
        grpc_message: '',
        duration_ms: 25.5,
        size_bytes: 21,
      },
    }
    apiMock.grpcInvoke.mockResolvedValue(result)
    const { wrapper } = await mountEditor((ep) => {
      ep.method = 'GRPC'
      ep.path = 'grpcb.in:9000'
      ep.request.body = {
        mode: 'grpc',
        spec: {
          service: 'grpcbin.GRPCBin',
          method: 'DummyUnary',
          message: '{}',
          use_tls: false,
          proto_ids: [],
        },
      }
    })

    const sendBtn = wrapper.find('.bar-send')
    await sendBtn.trigger('click')
    await flushPromises()
    await nextTick()

    expect(apiMock.grpcInvoke).toHaveBeenCalledTimes(1)
    const args = apiMock.grpcInvoke.mock.calls[0][0]
    expect(args.address).toBe('grpcb.in:9000')
    expect(args.service).toBe('grpcbin.GRPCBin')
    expect(args.method).toBe('DummyUnary')
    expect(args.environment_id).toBe(null)
    // unary 响应入响应区：grpc-status 徽标 + 消息内容
    expect(wrapper.text()).toContain('OK')
    expect(wrapper.text()).toContain('Miyo')
    // 不应走 HTTP 执行链路
    expect(apiMock.executeRequest).not.toHaveBeenCalled()
  })

  it('GRPC 服务端流发送：注册流状态，响应区展示流时间线', async () => {
    const result: GrpcInvokeResult = { kind: 'stream', stream_id: 'st-9' }
    apiMock.grpcInvoke.mockResolvedValue(result)
    const { wrapper, store } = await mountEditor((ep) => {
      ep.method = 'GRPC'
      ep.path = 'grpcb.in:9000'
      ep.request.body = {
        mode: 'grpc',
        spec: {
          service: 'hello.HelloService',
          method: 'LotsOfReplies',
          message: '{"greeting":"hi"}',
          use_tls: false,
          proto_ids: [],
        },
      }
    })

    await wrapper.find('.bar-send').trigger('click')
    await flushPromises()
    await nextTick()

    expect(apiMock.grpcInvoke).toHaveBeenCalledTimes(1)
    // store 已注册流运行态（事件通道按 stream_id 追加消息）
    const st = store.grpcStreamOf('ep-grpc-1')
    expect(st?.streamId).toBe('st-9')
    expect(st?.status).toBe('running')
    // 响应区展示运行中的流
    expect(wrapper.text()).toContain('接收中')
    expect(wrapper.text()).toContain('hello.HelloService/LotsOfReplies')
  })
})
