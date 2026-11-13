/**
 * EndpointEditor 地址栏输入单测：
 *
 * 回归 bug——旧 urlPath setter 逐键归一化，输入 `h` 瞬间被改写成 `/h`，
 * 想输自定义域名 `http://…` 打到一半已被加斜杠，绝对地址分支永远无法命中。
 * 修复后：逐键原文存储，归一化只在提交时机（失焦 / 发送 / 保存 / 粘贴）执行。
 */
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import EndpointEditor from '@/domains/endpoint/components/EndpointEditor.vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { makeDraft } from '@/testUtils/draftFixture'
import { collectErrors, stubScrollIntoView } from '@/testUtils/componentTest'
import type { Environment, Endpoint } from '@/types/miyoApi'

/** miyoApi 全量 mock（口径同 EndpointEditor.response.spec：挂载期懒加载接口全数兜底）。 */
const apiMock = vi.hoisted(() => ({
  executeRequest: vi.fn(async () => {
    throw new Error('not used')
  }),
  listExamples: vi.fn(async () => []),
  listRequestExamples: vi.fn(async () => []),
  listTestCases: vi.fn(async () => []),
  listHistories: vi.fn(async () => []),
  listEnvironments: vi.fn(async () => []),
  getActiveEnvironment: vi.fn(async () => null),
  getGlobalVariables: vi.fn(async () => []),
  getGlobalParams: vi.fn(async () => []),
  saveRequestExample: vi.fn(),
  saveExample: vi.fn(),
  parseCurlCommand: vi.fn(async () => ({
    url: 'https://p.example.com/a',
    method: 'GET',
    headers: [],
    body: null,
    auth: { type: 'none' },
  })),
}))

vi.mock('@/composables/useMiyoApi.ts', () => ({ useMiyoApi: () => apiMock }))

const endpoint: Endpoint = makeDraft({ id: 'ep-url', name: '地址栏', path: '/' })

async function mountEditor() {
  const store = useWorkspaceStore()
  // 文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
  store.project = {
    id: 'proj-url-1',
    name: 'P',
    description: '',
    variables: {},
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
  }
  store.endpoints = [endpoint]
  const wrapper = mount(EndpointEditor, { shallow: true })
  await nextTick()
  store.openEndpoint(endpoint)
  await nextTick()
  return { wrapper, store }
}

/** 注入「有 base_url 的激活环境」（口径同 workspace.spec）。 */
function activateEnvWithBase(store: ReturnType<typeof useWorkspaceStore>): Environment {
  const now = new Date().toISOString()
  const env: Environment = {
    id: 'env-1',
    project_id: 'proj-url-1',
    name: '测试',
    base_url: 'https://env.example.com',
    variables: [],
    created_at: now,
    updated_at: now,
  }
  store.environments.push(env)
  store.activeEnvId = 'env-1'
  return env
}

/** 构造环境（baseUrl 留空 = 未配前置 URL，下拉行左列回退会话前缀）。 */
function makeEnv(id: string, name: string, baseUrl = ''): Environment {
  const now = '2026-01-01T00:00:00.000Z'
  return {
    id,
    project_id: 'proj-url-1',
    name,
    base_url: baseUrl,
    variables: [],
    created_at: now,
    updated_at: now,
  }
}

describe('EndpointEditor：地址栏逐键不改写，提交才归一化', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubScrollIntoView()
  })

  it('输入 h 不再被改写成 /h；失焦提交才补斜杠（回归：逐键归一化）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    expect((input.element as HTMLInputElement).value).toBe('/')

    // 清空（旧实现 setter 忽略空串，输入框清不掉；新实现允许）。
    await input.setValue('')
    expect(store.activeEndpoint!.path).toBe('')

    // 逐键输入：显示与模型都保持原文，不出现 `/h`。
    await input.setValue('h')
    expect((input.element as HTMLInputElement).value).toBe('h')
    expect(store.activeEndpoint!.path).toBe('h')

    // 失焦提交 → 归一为 /h。
    await input.trigger('blur')
    await nextTick()
    expect(store.activeEndpoint!.path).toBe('/h')
    expect((input.element as HTMLInputElement).value).toBe('/h')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('环境激活时逐键输完整自定义域名：中间态不拆不加斜杠，失焦整条存 path 且环境不被写', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const env = activateEnvWithBase(store)
    const sessionBefore = store.sessionBaseUrl
    const input = wrapper.find('.url-input')

    // 逐键中间态：路径原样，不被加 `/`、不被提前拆 origin。
    for (const partial of [
      'h',
      'ht',
      'http://a',
      'http://api.custom.com',
      'http://api.custom.com/v1/users',
    ]) {
      await input.setValue(partial)
      expect(store.activeEndpoint!.path).toBe(partial)
    }

    // 失焦提交：环境前缀分支 → origin+path 整条存入 path（发送直达自定义域名）。
    await input.trigger('blur')
    await nextTick()
    expect(store.activeEndpoint!.path).toBe('http://api.custom.com/v1/users')
    // 共享环境的 base_url 不被导入污染。
    expect(env.base_url).toBe('https://env.example.com')
    expect(store.activeEnvId).toBe('env-1')
    // 会话前缀同样不写（环境前缀生效时写入被遮蔽，事后切「无环境」会冒出陌生域名）
    expect(store.sessionBaseUrl).toBe(sessionBefore)

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('无环境输完整域名：失焦拆分——origin 进前缀、path 留相对、query 并入参数', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')

    await input.setValue('https://api.custom.com/v1/users?page=2')
    // 提交前：原文存储。
    expect(store.activeEndpoint!.path).toBe('https://api.custom.com/v1/users?page=2')

    await input.trigger('blur')
    await nextTick()
    expect(store.activeEndpoint!.path).toBe('/v1/users')
    expect(store.sessionBaseUrl).toBe('https://api.custom.com')
    expect(store.activeEndpoint!.request.params).toEqual(
      expect.arrayContaining([expect.objectContaining({ key: 'page', value: '2' })]),
    )

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴完整 URL：无需失焦立即拆分，query 并入参数（保持旧粘贴体验）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', {
      value: { getData: () => 'https://p.example.com/a?k=1' },
    })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(store.activeEndpoint!.path).toBe('/a')
    expect(store.sessionBaseUrl).toBe('https://p.example.com')
    expect(store.activeEndpoint!.request.params).toEqual(
      expect.arrayContaining([expect.objectContaining({ key: 'k', value: '1' })]),
    )

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴干净路径文本：拦截原生粘贴，改走 insertText 插入（根治 WKWebView smart paste 补空格）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa/bb/'
    await nextTick()
    const el = input.element as HTMLInputElement
    el.setSelectionRange(el.value.length, el.value.length)

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => '/cc' } })
    input.element.dispatchEvent(ev)
    await nextTick()

    // 回归 bug：放行原生粘贴时 WKWebView smart paste 会在插入点邻接文字补分隔空格
    // （`/aa/bb/` + 粘 `/cc` → `/aa/bb/ /cc`），且补的空格不在 clipboardData 里
    // 拦不到——只能拦截行为，改走同屏 insertText（原生事务，撤销栈完好）。
    expect(ev.defaultPrevented).toBe(true)
    expect(el.value).toBe('/aa/bb//cc')
    expect(store.activeEndpoint!.path).toBe('/aa/bb//cc')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('纯空白粘贴：丢弃（地址栏不该收空白）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa'
    await nextTick()

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => '   ' } })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect(store.activeEndpoint!.path).toBe('/aa')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('字段自身带不可见尾空格时粘贴：先排掉残留再插（回归：从地址栏复制再粘回来多一个空格）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    // 旧版本粘出来的残留：值末尾一个空格，界面上看着就是 `/routes`
    store.activeEndpoint!.path = '/routes '
    await nextTick()

    const el = input.element as HTMLInputElement
    el.setSelectionRange(el.value.length, el.value.length)
    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => 'routes' } })
    el.dispatchEvent(ev)
    await nextTick()

    // 关键：结果里不能出现 `/routes routes`（空白只能是字段残留，粘贴时排掉）
    expect(ev.defaultPrevented).toBe(true)
    expect(el.value).toBe('/routesroutes')
    expect(store.activeEndpoint!.path).toBe('/routesroutes')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴「完整地址 + 空格 + 路径」：拼回一条 URL，不丢主机（回归：前缀带不可见空白）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = ''
    await nextTick()

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', {
      value: { getData: () => 'http://p.example.com:8093 /routes' },
    })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect(store.activeEndpoint!.path).toBe('/routes')
    expect(store.sessionBaseUrl).toBe('http://p.example.com:8093')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴「路径 + 名称」文档片段：只取路径段（回归：/routes routes）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = ''
    await nextTick()

    // 接口文档整段复制：路径 + 名称（换行还会被单行 input 折成空格 → `/routes routes`）
    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => 'routes routes' } })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect((input.element as HTMLInputElement).value).toBe('routes')
    // 失焦定稿补斜杠，落到用户期望的 `/routes`
    input.element.dispatchEvent(new Event('blur'))
    await nextTick()
    expect(store.activeEndpoint!.path).toBe('/routes')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴「方法 + 路径」文档片段：取含 / 的那一段（回归：POST /routes）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa/bb/'
    await nextTick()

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => 'POST /routes' } })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect((input.element as HTMLInputElement).value).toBe('/routes')
    expect(store.activeEndpoint!.path).toBe('/routes')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴带换行/尾部空白的路径：整条替换，不留多余空白', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa/bb/'
    await nextTick()

    // 从文档复制一行路径，末尾带换行（单行 input 里会折成空格）
    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', {
      value: { getData: () => '/api/v1/pay/payInOrder/createOrder\n' },
    })
    input.element.dispatchEvent(ev)
    await nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect((input.element as HTMLInputElement).value).toBe('/api/v1/pay/payInOrder/createOrder')
    expect(store.activeEndpoint!.path).toBe('/api/v1/pay/payInOrder/createOrder')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴 cURL 命令：仍拦截走导入（分流边界不回归）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa/bb/'
    await nextTick()

    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', {
      value: { getData: () => 'curl https://p.example.com/a -X GET' },
    })
    input.element.dispatchEvent(ev)

    expect(ev.defaultPrevented).toBe(true)
    // cURL 语义是「这就是请求」，整条覆盖并按导入规则落 path / 会话前缀
    await vi.waitFor(() => expect(store.activeEndpoint!.path).toBe('/a'))
    expect(store.sessionBaseUrl).toBe('https://p.example.com')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('粘贴多行 cURL：换行折叠后仍走导入，不被当成路径塞进地址栏', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditor()
    const input = wrapper.find('.url-input')
    store.activeEndpoint!.path = '/aa/bb/'
    await nextTick()

    // 从终端整段复制的形态：续行反斜杠 + 换行 + 缩进
    const multi = 'curl \\\n  -X POST https://p.example.com/pay \\\n  -H "Content-Type: application/json" \\\n  -d \'{"a":1}\''
    const ev = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(ev, 'clipboardData', { value: { getData: () => multi } })
    input.element.dispatchEvent(ev)

    expect(ev.defaultPrevented).toBe(true)
    // spec 里 api.parseCurlCommand 是桩：只要走到导入分支即说明分流正确，
    // 真正的命令解析由 Rust parse_curl 负责（utils/url.spec 已覆盖判定本身）。
    await vi.waitFor(() => expect(store.activeEndpoint!.path).toBe('/a'))
    expect(store.sessionBaseUrl).toBe('https://p.example.com')
    // 回归：按原文判定时 isCurlCommand 为假 → pickPathToken 取到的是一段碎片
    expect(store.activeEndpoint!.path).not.toContain('curl')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('gRPC 地址逐键输入：失焦不加 / 前缀（host:port 原样保留）', async () => {
    const errors = collectErrors()
    const store = useWorkspaceStore()
    useLocaleStore().setMode('zh')
    store.project = {
      id: 'proj-url-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    const grpcEp = makeDraft({ id: 'ep-grpc-url', name: 'gRPC', method: 'GRPC', path: '' })
    store.endpoints = [grpcEp]
    const wrapper = mount(EndpointEditor, { shallow: true })
    await nextTick()
    store.openEndpoint(grpcEp)
    await nextTick()

    const input = wrapper.find('.url-input')
    await input.setValue('localhost:5051')
    expect(store.activeEndpoint!.path).toBe('localhost:5051')
    await input.trigger('blur')
    await nextTick()
    expect(store.activeEndpoint!.path).toBe('localhost:5051')

    wrapper.unmount()
    errors.restore()
    expect(errors.errors).toEqual([])
  })
})

describe('地址栏前置 URL 下拉（环境选项标签）', () => {
  /** 全量挂载（非 shallow）：CustomSelect 弹层要真实组件渲染，口径同 requestSettings spec。 */
  async function mountEditorFull() {
    const store = useWorkspaceStore()
    useLocaleStore().setMode('zh')
    store.project = {
      id: 'proj-url-1',
      name: 'P',
      description: '',
      variables: {},
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
    }
    endpoint.path = '/' // 同文件前序用例会改写共享 fixture，先复位（相对路径才渲染前置 URL 下拉）
    store.endpoints = [endpoint]
    const wrapper = mount(EndpointEditor, {
      global: { directives: { 'tooltip-overflow': {}, 'focus-end': {} } },
    })
    await nextTick()
    store.openEndpoint(endpoint)
    await nextTick()
    await nextTick()
    return { wrapper, store }
  }

  it('环境未配 Base URL：左列显示会话 Base URL 而非「无环境」（回归：整列多个无环境）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditorFull()
    store.sessionBaseUrl = 'http://localhost'
    const now = '2026-01-01T00:00:00.000Z'
    const mkEnv = (id: string, name: string): Environment => ({
      id,
      project_id: 'proj-url-1',
      name,
      base_url: '',
      variables: [],
      created_at: now,
      updated_at: now,
    })
    store.environments.push(mkEnv('env-dev', '开发环境'), mkEnv('env-test', '测试环境'))
    store.activeEnvId = 'env-test'
    await nextTick()

    // 打开前置 URL 下拉（CustomSelect 弹层挂 body，查询口径同 EnvironmentBar spec）
    await wrapper.find('.base-url-select .cs-trigger').trigger('click')
    await nextTick()
    await nextTick()
    const pop = document.querySelector('.base-url-pop')
    expect(pop).toBeTruthy()

    const urls = Array.from(pop!.querySelectorAll('.base-url-opt-url')).map((el) =>
      el.textContent?.trim(),
    )
    const names = Array.from(pop!.querySelectorAll('.base-url-opt-name')).map((el) =>
      el.textContent?.trim(),
    )
    // 左列 = 各选项真实发送前缀：无环境与未配 base 的环境都回退会话 Base URL
    // （store.urlDomain / 发送拼接同口径），不再逐行兜底成「无环境」撞名。
    expect(urls).toEqual(['http://localhost', 'http://localhost', 'http://localhost'])
    // 右列 = 选项语义名：无环境 / 各环境名。
    expect(names).toEqual(['无环境', '开发环境', '测试环境'])
    // 末项「管理环境…」在场（单列，不参与左/右双列）。
    expect(pop!.querySelector('.base-url-manage')?.textContent?.trim()).toBe('管理环境…')

    wrapper.unmount()
    document.querySelector('.base-url-pop')?.remove()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('「无环境」独立成组：标题在场、未配 base 的环境标「未配置」、默认值不给重置入口', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditorFull()
    store.sessionBaseUrl = 'http://localhost'
    store.environments = [makeEnv('env-dev', '开发环境'), makeEnv('env-test', '测试环境', 'https://t.example.com')]
    await nextTick()

    await wrapper.find('.base-url-select .cs-trigger').trigger('click')
    await nextTick()
    await nextTick()
    const pop = document.querySelector('.base-url-pop')
    expect(pop).toBeTruthy()

    // 会话前缀单列一组：不再与环境行排成同构列表（否则被读成「无环境环境指向某 IP」）
    expect(pop!.querySelector('.cs-opt-header')?.textContent?.trim()).toBe('不使用环境 · 会话前缀')
    // 未配 base 的环境行给微标记（左列是回退的会话前缀，非该环境自身配置）；配了的没有
    const hints = Array.from(pop!.querySelectorAll('.base-url-opt-hint')).map((el) =>
      el.textContent?.trim(),
    )
    expect(hints).toEqual(['未配置'])
    // 会话前缀仍是内置默认：不该出现重置入口
    expect(pop!.querySelector('.base-url-reset')).toBeNull()

    wrapper.unmount()
    document.querySelector('.base-url-pop')?.remove()
    errors.restore()
    expect(errors.errors).toEqual([])
  })

  it('会话前缀偏离默认：给重置入口，点击回内置默认且不切环境（可逆，不再只增不减）', async () => {
    const errors = collectErrors()
    const { wrapper, store } = await mountEditorFull()
    store.environments = [makeEnv('env-dev', '开发环境', 'https://dev.example.com')]
    store.activeEnvId = 'env-dev'
    // 复现报告场景：导入/粘贴过的 origin 预填在会话前缀里，切到「无环境」才看得见
    store.sessionBaseUrl = 'http://172.31.50.103:8092'
    await nextTick()

    await wrapper.find('.base-url-select .cs-trigger').trigger('click')
    await nextTick()
    await nextTick()
    const pop = document.querySelector('.base-url-pop')!
    const btn = pop.querySelector('.base-url-reset') as HTMLElement | null
    expect(btn).toBeTruthy()
    expect(btn!.getAttribute('title')).toBe('重置为默认前缀')

    btn!.click()
    await nextTick()
    expect(store.sessionBaseUrl).toBe('http://localhost')
    // 重置只动会话前缀，不切换环境（点的是行尾按钮，不是选中该行）
    expect(store.activeEnvId).toBe('env-dev')
    // 同口径持久化（watch 按项目回写，重启不再回落到该 origin）
    expect(localStorage.getItem('miyo.base-url.proj-url-1')).toBe('http://localhost')

    wrapper.unmount()
    document.querySelector('.base-url-pop')?.remove()
    errors.restore()
    expect(errors.errors).toEqual([])
  })
})
