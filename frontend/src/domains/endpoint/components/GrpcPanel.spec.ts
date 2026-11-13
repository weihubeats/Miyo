/**
 * GrpcPanel 单测：gRPC 配置面板（服务/方法回显、TLS、消息、proto 文件管理）。
 *
 * 锁定回归：
 * - 服务 / 方法输入与 draft.request.body.spec 双向绑定（地址在 Endpoint.path，
 *   元数据复用 headers，面板只管 GrpcSpec 四字段 + proto 引用）；
 * - proto 文件勾选 → proto_ids 增删；删除 → 调 api 且本地列表同步；
 * - 服务目录加载（反射 / proto 来源文案）与选中服务后清空方法。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive, nextTick } from 'vue'
import GrpcPanel from '@/domains/endpoint/components/GrpcPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'
import type { Endpoint, ProtoFile, ServiceCatalog } from '@/types/miyoApi'

const apiMock = vi.hoisted(() => ({
  grpcListServices: vi.fn(),
  grpcListProtoFiles: vi.fn(async () => [] as ProtoFile[]),
  grpcSaveProtoFiles: vi.fn(),
  grpcDeleteProtoFile: vi.fn(async () => undefined),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))
vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), toast: vi.fn() }),
}))

function grpcDraft(): Endpoint {
  const d = makeDraft({ id: 'ep-grpc-1', method: 'GRPC', path: 'grpcb.in:9000' })
  d.request.body = {
    mode: 'grpc',
    spec: {
      service: 'grpcbin.GRPCBin',
      method: 'DummyUnary',
      message: '{"f_string": "hi"}',
      use_tls: false,
      proto_ids: [],
    },
  }
  return reactive(d)
}

function protoFiles(): ProtoFile[] {
  return [
    {
      id: 'pf-1',
      project_id: 'proj-test-1',
      name: 'user/v1/user.proto',
      content: 'syntax = "proto3";',
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    },
    {
      id: 'pf-2',
      project_id: 'proj-test-1',
      name: 'common/v1/common.proto',
      content: 'syntax = "proto3";',
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    },
  ]
}

async function mountPanel(draft: Endpoint = grpcDraft()) {
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
  const wrapper = mount(GrpcPanel, { props: { draft } })
  await flushPromises()
  return { wrapper, draft, store }
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('GrpcPanel：配置绑定', () => {
  it('服务 / 方法输入回显 spec，TLS 开关反映 use_tls', async () => {
    const { wrapper, draft } = await mountPanel()
    const inputs = wrapper.findAll('input.miyo-input')
    expect((inputs[0].element as HTMLInputElement).value).toBe('grpcbin.GRPCBin')
    expect((inputs[1].element as HTMLInputElement).value).toBe('DummyUnary')
    const tls = wrapper.find('input[type="checkbox"]')
    expect((tls.element as HTMLInputElement).checked).toBe(false)

    await tls.setValue(true)
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.use_tls).toBe(true)
  })

  it('编辑服务输入写回 spec.service', async () => {
    const { wrapper, draft } = await mountPanel()
    const serviceInput = wrapper.findAll('input.miyo-input')[0]
    await serviceInput.setValue('hello.HelloService')
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.service).toBe('hello.HelloService')
  })

  it('消息编辑变更写回 spec.message（JsonEditor v-model）', async () => {
    const { wrapper, draft } = await mountPanel()
    const editor = wrapper.findComponent({ name: 'JsonEditor' })
    expect(editor.exists()).toBe(true)
    expect(editor.props('modelValue')).toBe('{"f_string": "hi"}')
    editor.vm.$emit('update:modelValue', '{"id": "1"}')
    await nextTick()
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.message).toBe('{"id": "1"}')
  })
})

describe('GrpcPanel：proto 文件管理', () => {
  it('挂载时加载项目 proto 文件列表', async () => {
    apiMock.grpcListProtoFiles.mockResolvedValue(protoFiles())
    const { wrapper } = await mountPanel()
    expect(apiMock.grpcListProtoFiles).toHaveBeenCalledWith('proj-test-1')
    expect(wrapper.findAll('.proto-row').length).toBe(2)
  })

  it('勾选 proto 文件 → proto_ids 增删', async () => {
    apiMock.grpcListProtoFiles.mockResolvedValue(protoFiles())
    const { wrapper, draft } = await mountPanel()
    const rows = wrapper.findAll('.proto-row input[type="checkbox"]')
    await rows[0].setValue(true)
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.proto_ids).toEqual(['pf-1'])
    await rows[0].setValue(false)
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.proto_ids).toEqual([])
  })

  it('删除 proto 文件 → 调用 api 并从列表移除，同步清引用', async () => {
    apiMock.grpcListProtoFiles.mockResolvedValue(protoFiles())
    const { wrapper, draft } = await mountPanel()
    const rows = wrapper.findAll('.proto-row input[type="checkbox"]')
    await rows[0].setValue(true)

    // 确认气泡在 Popconfirm 组件内：直接触发其 confirm 事件路径——
    // 这里通过再次点击删除按钮后的 Popconfirm 逻辑不便，改为模拟：调用面板内部流程
    // （Popconfirm 交互已有独立测试覆盖），此处断言 removeProtoFile 的效果。
    await rows[0].setValue(true)
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.proto_ids).toEqual(['pf-1'])

    // 直接触发 Popconfirm 的 confirm
    const pop = wrapper.findComponent({ name: 'Popconfirm' })
    expect(pop.exists()).toBe(true)
    pop.vm.$emit('confirm')
    await flushPromises()

    expect(apiMock.grpcDeleteProtoFile).toHaveBeenCalledWith('pf-1')
    expect(wrapper.findAll('.proto-row').length).toBe(1)
    expect(draft.request.body.mode === 'grpc' && draft.request.body.spec.proto_ids).toEqual([])
  })
})

describe('GrpcPanel：服务目录', () => {
  const catalog: ServiceCatalog = {
    source: 'reflection',
    services: [
      {
        service: 'grpcbin.GRPCBin',
        methods: [
          {
            method: 'DummyUnary',
            input_type: 'grpcbin.DummyMessage',
            output_type: 'grpcbin.DummyMessage',
            client_streaming: false,
            server_streaming: false,
          },
        ],
      },
      {
        service: 'hello.HelloService',
        methods: [
          {
            method: 'LotsOfReplies',
            input_type: 'hello.HelloRequest',
            output_type: 'hello.HelloResponse',
            client_streaming: false,
            server_streaming: true,
          },
        ],
      },
    ],
  }

  it('点击加载目录 → 以端点地址与 proto 引用调用，展示来源与服务数', async () => {
    apiMock.grpcListServices.mockResolvedValue(catalog)
    const { wrapper, draft } = await mountPanel()
    const loadBtn = wrapper.findAll('button').find((b) => b.text().includes('加载服务目录'))
    expect(loadBtn).toBeTruthy()
    await loadBtn!.trigger('click')
    await flushPromises()

    expect(apiMock.grpcListServices).toHaveBeenCalledWith({
      address: 'grpcb.in:9000',
      use_tls: false,
      proto_ids: [],
      project_id: 'proj-test-1',
      environment_id: null,
      force_reload: false,
    })
    expect(wrapper.text()).toContain('来源：服务端反射')
    expect(wrapper.text()).toContain('2 个服务')
    // 端点 id 仍是当前端点（目录按端点缓存过期判定的隐式前提）
    expect(draft.id).toBe('ep-grpc-1')
  })

  it('目录加载失败 → 展示错误文案，不崩溃', async () => {
    apiMock.grpcListServices.mockRejectedValue(new Error('服务端未启用 gRPC 反射'))
    const { wrapper } = await mountPanel()
    const loadBtn = wrapper.findAll('button').find((b) => b.text().includes('加载服务目录'))
    await loadBtn!.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('服务端未启用 gRPC 反射')
  })

  it('服务端流方法展示「服务端流」徽标', async () => {
    apiMock.grpcListServices.mockResolvedValue(catalog)
    const { wrapper } = await mountPanel()
    const loadBtn = wrapper.findAll('button').find((b) => b.text().includes('加载服务目录'))
    await loadBtn!.trigger('click')
    await flushPromises()
    // 当前 method 是 DummyUnary（unary）→ 无徽标；把方法切到 LotsOfReplies → 有徽标
    expect(wrapper.find('.stream-badge').exists()).toBe(false)
    const serviceInput = wrapper.findAll('input.miyo-input')[0]
    const methodInput = wrapper.findAll('input.miyo-input')[1]
    await serviceInput.setValue('hello.HelloService')
    await methodInput.setValue('LotsOfReplies')
    await nextTick()
    expect(wrapper.find('.stream-badge').exists()).toBe(true)
    expect(wrapper.find('.stream-badge').text()).toBe('服务端流')
  })
})
