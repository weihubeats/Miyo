/**
 * GrpcResponsePanel 单测：unary 响应与服务端流时间线。
 *
 * 锁定回归：
 * - unary：grpc-status 徽标语义（0=OK 绿，非零红 + 状态名 + grpc-message），
 *   响应消息 JSON 进 JsonTree，元数据表渲染（tonic 0.14 trailers 已并入 headers）；
 * - 服务端流：消息时间线（序号/耗时/预览）、运行中展示停止按钮并调用关闭、
 *   结束事件展示 grpc-status 与元数据。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick, reactive } from 'vue'
import GrpcResponsePanel from '@/domains/endpoint/components/GrpcResponsePanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import type { GrpcResponse } from '@/types/miyoApi'
import type { GrpcStreamState } from '@/stores/workspace'

const apiMock = vi.hoisted(() => ({
  grpcStreamClose: vi.fn(async () => true),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))

function unaryResponse(overrides: Partial<GrpcResponse> = {}): GrpcResponse {
  return {
    message_json: '{"id":"u-1","name":"小奏"}',
    metadata: [
      ['content-type', 'application/grpc'],
      ['grpc-status', '0'],
    ],
    grpc_status: 0,
    grpc_message: '',
    duration_ms: 12.5,
    size_bytes: 26,
    ...overrides,
  }
}

function streamState(overrides: Partial<GrpcStreamState> = {}): GrpcStreamState {
  return {
    streamId: 'st-1',
    endpointId: 'ep-1',
    service: 'hello.HelloService',
    method: 'LotsOfReplies',
    messages: [
      { kind: 'message', sequence: 1, message_json: '{"greeting":"hello 1"}', elapsed_ms: 100, size_bytes: 20 },
      { kind: 'message', sequence: 2, message_json: '{"greeting":"hello 2"}', elapsed_ms: 150, size_bytes: 20 },
    ],
    status: 'running',
    end: null,
    error: null,
    ...overrides,
  }
}

function mountPanel(props: { response: GrpcResponse | null; stream: GrpcStreamState | null }) {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  return mount(GrpcResponsePanel, { props: { ...props, method: 'GRPC' } })
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('GrpcResponsePanel：unary', () => {
  it('grpc-status 0 → OK 徽标（成功色），消息与元数据渲染', async () => {
    const wrapper = mountPanel({ response: unaryResponse(), stream: null })
    const pill = wrapper.find('.status-pill')
    expect(pill.classes()).toContain('ok')
    expect(pill.text()).toContain('OK')
    // 消息 JSON 解析进 JsonTree（键值可见）
    expect(wrapper.text()).toContain('小奏')
    // 元数据表
    const rows = wrapper.findAll('.meta-row')
    expect(rows.length).toBe(2)
    expect(wrapper.text()).toContain('content-type')
  })

  it('grpc-status 非零 → 失败徽标 + 状态名 + grpc-message 文案', async () => {
    const wrapper = mountPanel({
      response: unaryResponse({
        grpc_status: 12,
        grpc_message: 'unknown service demo.Demo',
        metadata: [['grpc-status', '12']],
      }),
      stream: null,
    })
    const pill = wrapper.find('.status-pill')
    expect(pill.classes()).not.toContain('ok')
    expect(pill.text()).toContain('UNIMPLEMENTED')
    expect(wrapper.text()).toContain('unknown service demo.Demo')
  })
})

describe('GrpcResponsePanel：服务端流', () => {
  it('运行中：展示接收中徽标、消息时间线、停止按钮', async () => {
    const wrapper = mountPanel({ response: null, stream: streamState() })
    expect(wrapper.text()).toContain('接收中')
    const items = wrapper.findAll('.stream-item')
    expect(items.length).toBe(2)
    expect(items[0].text()).toContain('#1')
    expect(items[0].text()).toContain('hello 1')
    // 停止按钮存在且调用关闭 + 清理 store 运行态
    const store = useWorkspaceStore()
    store.grpcStreams.set('st-1', streamState())
    await wrapper.find('button.miyo-btn-danger').trigger('click')
    await flushPromises()
    expect(apiMock.grpcStreamClose).toHaveBeenCalledWith('st-1')
    expect(store.grpcStreams.has('st-1')).toBe(false)
  })

  it('正常结束：grpc-status 徽标 + 元数据表；无停止按钮', async () => {
    const wrapper = mountPanel({
      response: null,
      stream: streamState({
        status: 'done',
        end: {
          kind: 'end',
          cancelled: false,
          grpc_status: 0,
          grpc_message: '',
          metadata: [['grpc-status', '0'], ['grpc-message', '']],
        },
      }),
    })
    const pill = wrapper.find('.status-pill')
    expect(pill.classes()).toContain('ok')
    expect(pill.text()).toContain('OK')
    expect(wrapper.find('button.miyo-btn-danger').exists()).toBe(false)
    expect(wrapper.findAll('.meta-row').length).toBe(2)
  })

  it('失败：展示 Failed 错误文案', async () => {
    const wrapper = mountPanel({
      response: null,
      stream: streamState({ status: 'failed', error: 'gRPC 状态 UNAVAILABLE（14）' }),
    })
    expect(wrapper.text()).toContain('gRPC 状态 UNAVAILABLE（14）')
  })

  it('新消息到达时跟随最新一条（未手动 pin）', async () => {
    // 与真实链路一致：store 的 ref(Map) 会对流状态做深响应代理
    const st = reactive(streamState())
    const wrapper = mountPanel({ response: null, stream: st })
    expect(wrapper.text()).toContain('消息 #2 详情')
    // 追加第三条：详情标题跟随到 #3
    st.messages.push({ kind: 'message', sequence: 3, message_json: '{"greeting":"hello 3"}', elapsed_ms: 200, size_bytes: 20 })
    await nextTick()
    expect(wrapper.text()).toContain('消息 #3 详情')
  })
})
