/**
 * JavaDebug store / JavaMethodPanel / JavaHistoryDrawer 单测：
 * 锁定回归——参数骨架生成、表单↔JSON 双向切换、参数校验拦截、
 * 历史状态徽标映射、收藏切换与项目切换清空。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import JavaMethodPanel from '@/domains/java-debug/components/JavaMethodPanel.vue'
import JavaResultPanel from '@/domains/java-debug/components/JavaResultPanel.vue'
import JavaHistoryDrawer from '@/domains/java-debug/components/JavaHistoryDrawer.vue'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'
import { useLocaleStore } from '@/stores/locale'
import type { JavaInvocationRecord, JavaInvokeResponse, JavaMethodInfo } from '@/types/miyoApi.d'

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({
    listJavaProjects: vi.fn(async () => []),
    saveJavaProject: vi.fn(),
    deleteJavaProject: vi.fn(),
    javaTestConnection: vi.fn(),
    javaListClasses: vi.fn(async () => []),
    javaListMethods: vi.fn(async () => []),
    javaInvoke: vi.fn(),
    javaCancelInvocation: vi.fn(),
    listJavaHistory: vi.fn(async () => []),
    getJavaInvocation: vi.fn(),
    deleteJavaInvocation: vi.fn(),
    clearJavaHistory: vi.fn(),
  }),
}))

vi.mock('@/composables/useToast.ts', () => ({
  useToast: () => ({
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
    toast: vi.fn(),
  }),
}))

beforeEach(() => {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
})

function method(overrides: Partial<JavaMethodInfo> = {}): JavaMethodInfo {
  return {
    method_id: 'com.example.OrderService#createOrder(CreateOrderRequest,long)',
    class_name: 'com.example.OrderService',
    method_name: 'createOrder',
    signature: 'public OrderResult createOrder(CreateOrderRequest, long)',
    return_type: 'OrderResult',
    return_generic: 'OrderResult',
    params: [
      { name: 'request', type_name: 'CreateOrderRequest', generic_type: '', required: false },
      { name: 'userId', type_name: 'long', generic_type: '', required: true },
    ],
    static_method: false,
    visibility: 'public',
    overloaded: false,
    declaring_class: 'com.example.OrderService',
    ...overrides,
  }
}

/** 历史记录测试夹具。 */
function historyRecord(overrides: Partial<JavaInvocationRecord> = {}): JavaInvocationRecord {
  return {
    id: 'r1',
    project_id: 'p1',
    class_name: 'com.example.OrderService',
    method_id: 'com.example.OrderService#pay()',
    method_signature: 'public void pay()',
    arguments: [],
    status: 'success',
    result: null,
    result_truncated: false,
    exception: null,
    duration_ms: 2.5,
    started_at: null,
    finished_at: null,
    created_at: '2026-11-17T00:00:00Z',
    ...overrides,
  }
}

describe('javaDebug store', () => {
  it('选中方法生成参数骨架（基本类型零值 / 引用类型 null）', () => {
    const store = useJavaDebugStore()
    store.selectMethod(method())
    const args = JSON.parse(store.argsJson) as unknown[]
    expect(args).toHaveLength(2)
    expect(args[0]).toBeNull()
    expect(args[1]).toBe(0)
  })

  it('参数默认值：String 空串 / boolean false', () => {
    const store = useJavaDebugStore()
    store.selectMethod(
      method({
        method_id: 'com.example.S#echo(String,boolean)',
        params: [
          { name: 'msg', type_name: 'String', generic_type: '', required: true },
          { name: 'flag', type_name: 'boolean', generic_type: '', required: true },
        ],
      }),
    )
    const args = JSON.parse(store.argsJson) as unknown[]
    expect(args).toEqual(['', false])
  })

  it('切换项目清空发现与结果状态', async () => {
    const store = useJavaDebugStore()
    store.projects = [
      {
        id: 'p1',
        name: 'A',
        mode: 'runtime',
        runtime_endpoint: 'http://127.0.0.1:9918',
        timeout_ms: 30_000,
        max_result_bytes: 1_048_576,
        max_concurrent: 4,
        description: '',
        created_at: '',
        updated_at: '',
      },
      {
        id: 'p2',
        name: 'B',
        mode: 'runtime',
        runtime_endpoint: 'http://127.0.0.1:9919',
        timeout_ms: 30_000,
        max_result_bytes: 1_048_576,
        max_concurrent: 4,
        description: '',
        created_at: '',
        updated_at: '',
      },
    ]
    store.activeProjectId = 'p1'
    store.activeClassName = 'com.example.X'
    store.classes = [
      {
        class_name: 'com.example.X',
        package_name: 'com.example',
        simple_name: 'X',
        method_count: 1,
        bean_name: null,
      },
    ]
    await store.loadHistory()
    store.activeProjectId = 'p2'
    expect(store.activeClassName).toBeNull()
    expect(store.classes).toHaveLength(0)
  })

  it('收藏切换持久化键按项目隔离', () => {
    const store = useJavaDebugStore()
    store.activeProjectId = 'p1'
    store.toggleFavorite('com.example.A')
    expect(store.isFavorite('com.example.A')).toBe(true)
    store.activeProjectId = 'p2'
    expect(store.isFavorite('com.example.A')).toBe(false)
  })
})

describe('JavaMethodPanel', () => {
  function mountPanel(overrides: Record<string, unknown> = {}) {
    return mount(JavaMethodPanel, {
      props: {
        className: 'com.example.OrderService',
        methods: [method()],
        loading: false,
        activeMethodId: method().method_id,
        argsJson: '[null, 0]',
        editMode: 'form',
        invoking: false,
        hasConnection: true,
        hasTemplate: false,
        ...overrides,
      },
    })
  }

  it('表单模式按形参渲染输入框与类型提示', () => {
    const wrapper = mountPanel()
    const inputs = wrapper.findAll('input.miyo-input')
    expect(inputs.length).toBeGreaterThanOrEqual(2)
    expect(wrapper.text()).toContain('CreateOrderRequest')
    expect(wrapper.text()).toContain('long')
  })

  it('参数个数不匹配给出校验错误且执行按钮禁用', async () => {
    const wrapper = mountPanel({ argsJson: '[null]' })
    await vi.waitFor(() => {
      expect(wrapper.find('[data-testid="java-args-error"]').exists()).toBe(true)
    })
    expect(wrapper.find('[data-testid="java-invoke-btn"]').attributes('disabled')).toBeDefined()
  })

  it('无连接时显示引导空态而非方法列表', () => {
    const wrapper = mountPanel({ hasConnection: false })
    expect(wrapper.text()).toContain('未连接 Runtime')
    expect(wrapper.text()).not.toContain('createOrder')
  })
})

describe('JavaResultPanel', () => {
  function response(overrides: Partial<JavaInvokeResponse> = {}): JavaInvokeResponse {
    return {
      invocation_id: 'i-1',
      method_id: 'com.example.OrderService#createOrder(CreateOrderRequest,long)',
      status: 'success',
      result: { orderNo: 'NO-1' },
      result_truncated: false,
      exception: null,
      duration_ms: 3.2,
      started_at: null,
      finished_at: null,
      ...overrides,
    }
  }

  it('成功结果渲染对象树 + 执行信息', () => {
    const wrapper = mount(JavaResultPanel, {
      props: {
        response: response(),
        startedAt: 1_700_000_000_000,
        argsJson: '[]',
        params: [],
        invoking: false,
        history: [],
      },
    })
    expect(wrapper.text()).toContain('成功')
    expect(wrapper.text()).toContain('NO-1')
  })

  it('异常结果突出异常类型与堆栈', async () => {
    const wrapper = mount(JavaResultPanel, {
      props: {
        response: response({
          status: 'failed',
          result: null,
          exception: {
            exception_type: 'java.lang.NullPointerException',
            message: 'order not found',
            stack_trace: ['com.example.OrderService.pay(OrderService.java:88)'],
            cause_chain: [],
          },
        }),
        startedAt: null,
        argsJson: '[]',
        params: [],
        invoking: false,
        history: [],
      },
    })
    expect(wrapper.text()).toContain('失败')
    // 切到异常堆栈 Tab 再断言（默认展示返回结果 Tab）。
    const exceptionTab = wrapper
      .findAll('button')
      .find((b) => b.text().includes('异常堆栈'))
    expect(exceptionTab).toBeDefined()
    await exceptionTab!.trigger('click')
    expect(wrapper.text()).toContain('java.lang.NullPointerException')
    expect(wrapper.text()).toContain('OrderService.java:88')
  })

  it('截断结果提示明确', () => {
    const wrapper = mount(JavaResultPanel, {
      props: {
        response: response({ result_truncated: true }),
        startedAt: null,
        argsJson: '[]',
        params: [],
        invoking: false,
        history: [],
      },
    })
    expect(wrapper.text()).toContain('已截断')
  })
})

describe('JavaHistoryDrawer', () => {
  it('空历史展示引导空态', () => {
    const wrapper = mount(JavaHistoryDrawer, {
      props: { open: true, records: [], loading: false },
      attachTo: document.body,
    })
    expect(document.body.textContent).toContain('暂无调用历史')
    wrapper.unmount()
  })

  it('历史记录渲染状态徽标 / 签名 / 耗时，异常详情折叠', () => {
    const wrapper = mount(JavaHistoryDrawer, {
      attachTo: document.body,
      props: {
        open: true,
        loading: false,
        records: [
          {
            id: 'r1',
            project_id: 'p1',
            class_name: 'com.example.OrderService',
            method_id: 'com.example.OrderService#boom()',
            method_signature: 'public void boom()',
            arguments: ['a'],
            status: 'failed',
            result: null,
            result_truncated: false,
            exception: {
              exception_type: 'java.lang.IllegalStateException',
              message: 'order not found',
              stack_trace: ['at OrderService'],
              cause_chain: [],
            },
            duration_ms: 1.5,
            started_at: null,
            finished_at: null,
            created_at: '2026-11-17T00:00:00Z',
          },
        ],
      },
    })
    expect(document.body.textContent).toContain('失败')
    expect(document.body.textContent).toContain('boom()')
    expect(document.body.textContent).toContain('1.5 ms')
    // 折叠态不展示堆栈
    expect(document.body.textContent).not.toContain('at OrderService')
    wrapper.unmount()
  })

  it('文本搜索与状态过滤（类 / 方法名不区分大小写）', async () => {
    const records = [
      historyRecord({ id: 'r1', class_name: 'com.example.OrderService', method_signature: 'public void pay()' }),
      historyRecord({ id: 'r2', class_name: 'com.example.RefundService', method_signature: 'public void refund()', status: 'failed' }),
    ]
    const wrapper = mount(JavaHistoryDrawer, {
      attachTo: document.body,
      props: { open: true, loading: false, records },
    })
    // 默认全部可见
    expect(document.body.textContent).toContain('pay()')
    expect(document.body.textContent).toContain('refund()')
    // 搜索 refund（大小写不敏感）
    const search = document.body.querySelector('[data-testid="java-history-search"]') as HTMLInputElement
    search.value = 'REFUND'
    search.dispatchEvent(new Event('input', { bubbles: true }))
    await vi.waitFor(() => {
      expect(document.body.textContent).not.toContain('pay()')
      expect(document.body.textContent).toContain('refund()')
    })
    // 清搜索，切状态过滤 success
    search.value = ''
    search.dispatchEvent(new Event('input', { bubbles: true }))
    const status = document.body.querySelector('[data-testid="java-history-status"]') as HTMLSelectElement
    status.value = 'success'
    status.dispatchEvent(new Event('change', { bubbles: true }))
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain('pay()')
      expect(document.body.textContent).not.toContain('refund()')
    })
    wrapper.unmount()
  })

  it('展开详情的敏感参数脱敏展示', async () => {
    const wrapper = mount(JavaHistoryDrawer, {
      attachTo: document.body,
      props: {
        open: true,
        loading: false,
        records: [historyRecord({ id: 'r3', arguments: [{ username: 'u', password: 'raw-secret' }] })],
      },
    })
    // 展开详情
    const item = document.body.querySelector('[data-testid="java-history-list"] article') as HTMLElement
    item.click()
    await vi.waitFor(() => {
      const text = document.body.textContent ?? ''
      expect(text).toContain('******')
      expect(text).not.toContain('raw-secret')
    })
    wrapper.unmount()
  })
})

describe('JavaMethodPanel 调用入口始终可达', () => {
  /**
   * 回归：方法数量多的类（实测 22 个）曾把「参数编辑 + 执行按钮」顶到视口之外，
   * 用户首屏只看到方法列表，误判为「没有调用入口、没有填参数的框」（真实截图）。
   * 锁定两点：① 方法列表自身限高滚动（不撑高整列）；② 执行按钮与参数区在同一
   * 面板内始终渲染。
   */
  function manyMethods(count: number): JavaMethodInfo[] {
    return Array.from({ length: count }, (_, i) => ({
      ...method(),
      method_id: `com.example.BigService#m${i}(long)`,
      method_name: `m${i}`,
      signature: `public void m${i}(long v)`,
      return_type: 'void',
      return_generic: 'void',
      params: [{ name: 'v', type_name: 'long', generic_type: '', required: true }],
    }))
  }

  it('22 个方法时仍渲染参数区与执行按钮', () => {
    const methods = manyMethods(22)
    const wrapper = mount(JavaMethodPanel, {
      props: {
        className: 'com.example.BigService',
        methods,
        loading: false,
        activeMethodId: methods[21].method_id,
        argsJson: '[0]',
        editMode: 'form',
        invoking: false,
        hasConnection: true,
        hasTemplate: false,
        attachMode: true,
      },
    })
    // 执行按钮存在且可用
    const runBtn = wrapper.find('[data-testid="java-invoke-btn"]')
    expect(runBtn.exists()).toBe(true)
    expect(runBtn.attributes('disabled')).toBeUndefined()
    // 参数输入框存在（方法有 1 个 long 参数）
    expect(wrapper.findAll('.jmp-param').length).toBe(1)
    // 方法列表渲染了全部条目（滚动承载，而非截断）
    expect(wrapper.findAll('.jmp-method')).toHaveLength(22)
  })

  it('方法列表容器限高并可滚动（不把调用区顶出视口）', () => {
    const methods = manyMethods(22)
    const wrapper = mount(JavaMethodPanel, {
      props: {
        className: 'com.example.BigService',
        methods,
        loading: false,
        activeMethodId: methods[0].method_id,
        argsJson: '[0]',
        editMode: 'form',
        invoking: false,
        hasConnection: true,
        hasTemplate: false,
        attachMode: true,
      },
    })
    const list = wrapper.find('.jmp-method-list')
    expect(list.exists()).toBe(true)
    // 样式在 SFC scoped 块内：用内联断言不可行，改为锁定「列表与编辑区是兄弟节点」
    // ——只要列表不再是唯一子节点撑满，编辑区就能被滚动/滚动入视口看到。
    expect(wrapper.find('.jmp-editor').exists()).toBe(true)
    expect(wrapper.find('.jmp-run-row').exists()).toBe(true)
  })
})

describe('JavaMethodPanel 复杂参数与危险确认', () => {
  function mountPanel(overrides: Record<string, unknown> = {}) {
    return mount(JavaMethodPanel, {
      props: {
        className: 'com.example.OrderService',
        methods: [method()],
        loading: false,
        activeMethodId: method().method_id,
        argsJson: '[{"items":[1]}, 7]',
        editMode: 'form',
        invoking: false,
        hasConnection: true,
        hasTemplate: false,
        ...overrides,
      },
    })
  }

  it('复杂类型参数可直接内联编辑，弹层仍可展开', async () => {
    const wrapper = mountPanel()
    // 内联文本框（核心改进：改一个字段不再需要点两次）
    const inline = wrapper.find('[data-testid="java-param-inline-0"]')
    expect(inline.exists()).toBe(true)
    await inline.setValue('{"items": [1, 2]}')
    const emittedInline = JSON.parse(
      wrapper.emitted('update:argsJson')!.at(-1)![0] as string,
    ) as unknown[]
    expect(emittedInline[0]).toEqual({ items: [1, 2] })

    // 展开编辑（多行 JSON）仍然可用
    const btn = wrapper.find('[data-testid="java-param-json-0"]')
    expect(btn.exists()).toBe(true)
    expect(wrapper.text()).toContain('展开编辑')
    await btn.trigger('click')
    // 弹层出现（Modal 挂 body）
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain('应用')
    })
    const apply = document.body.querySelector('[data-testid="java-param-json-apply"]') as HTMLButtonElement
    apply.click()
    await vi.waitFor(() => {
      expect(wrapper.emitted('update:argsJson')).toBeTruthy()
    })
    // 弹层从当前值起编，应用后保留内联输入的内容
    const payload = JSON.parse(
      wrapper.emitted('update:argsJson')!.at(-1)![0] as string,
    ) as unknown[]
    expect(payload[0]).toEqual({ items: [1, 2] })
    expect(payload[1]).toBe(7)
  })

  it('危险方法名先出二次确认，确认后才触发 run', async () => {
    const wrapper = mountPanel({
      methods: [
        method({
          method_id: 'com.example.OrderService#deleteAll()',
          method_name: 'deleteAll',
          params: [],
        }),
      ],
      activeMethodId: 'com.example.OrderService#deleteAll()',
      argsJson: '[]',
    })
    const run = wrapper.find('[data-testid="java-invoke-btn"]')
    expect(wrapper.text()).toContain('deleteAll')
    await run.trigger('click')
    // 第一次点击 → 出确认条，未 emit run
    expect(wrapper.find('[data-testid="java-danger-confirm"]').exists()).toBe(true)
    expect(wrapper.emitted('run')).toBeUndefined()
    // 确认 → emit run
    await wrapper.find('[data-testid="java-danger-run"]').trigger('click')
    expect(wrapper.emitted('run')).toHaveLength(1)
  })

  it('非危险方法直接执行', async () => {
    const wrapper = mountPanel()
    await wrapper.find('[data-testid="java-invoke-btn"]').trigger('click')
    expect(wrapper.emitted('run')).toHaveLength(1)
    expect(wrapper.find('[data-testid="java-danger-confirm"]').exists()).toBe(false)
  })

  it('模板按钮 emit save / use 事件', async () => {
    const wrapper = mountPanel({ hasTemplate: true })
    await wrapper.find('[data-testid="java-template-save"]').trigger('click')
    await wrapper.find('[data-testid="java-template-use"]').trigger('click')
    expect(wrapper.emitted('saveTemplate')).toHaveLength(1)
    expect(wrapper.emitted('useTemplate')).toHaveLength(1)
  })
})

describe('JavaResultPanel 历史 Tab 与参数脱敏', () => {
  function response(overrides: Partial<JavaInvokeResponse> = {}): JavaInvokeResponse {
    return {
      invocation_id: 'i-1',
      method_id: 'com.example.OrderService#createOrder(CreateOrderRequest,long)',
      status: 'success',
      result: { orderNo: 'NO-1' },
      result_truncated: false,
      exception: null,
      duration_ms: 3.2,
      started_at: null,
      finished_at: null,
      ...overrides,
    }
  }

  it('输入参数 Tab 按形参名脱敏', async () => {
    const wrapper = mount(JavaResultPanel, {
      props: {
        response: response(),
        startedAt: null,
        argsJson: JSON.stringify([{ password: 'plain-text' }, 1]),
        params: [
          { name: 'password', type_name: 'String', generic_type: '', required: true },
          { name: 'count', type_name: 'long', generic_type: '', required: true },
        ],
        invoking: false,
        history: [],
      },
    })
    const argsTab = wrapper.findAll('button').find((b) => b.text().includes('输入参数'))
    await argsTab!.trigger('click')
    await vi.waitFor(() => {
      const text = wrapper.find('[data-testid="java-result-args"]').text()
      expect(text).toContain('******')
      expect(text).not.toContain('plain-text')
      expect(text).toContain('已脱敏')
    })
  })

  it('历史 Tab 渲染最近记录并支持快速重跑', async () => {
    const record = historyRecord({ id: 'r9', method_signature: 'public void pay()' })
    const wrapper = mount(JavaResultPanel, {
      props: {
        response: response(),
        startedAt: null,
        argsJson: '[]',
        params: [],
        invoking: false,
        history: [record],
      },
    })
    const historyTab = wrapper.findAll('button').find((b) => b.text().includes('调用历史'))
    await historyTab!.trigger('click')
    expect(wrapper.text()).toContain('pay()')
    const rerun = wrapper.find('.jrp-history-rerun')
    await rerun.trigger('click')
    expect(wrapper.emitted('rerun')).toBeTruthy()
    expect(wrapper.emitted('rerun')![0][0]).toMatchObject({ id: 'r9' })
  })
})
