/**
 * useRequestUrl：地址栏状态机（自 EndpointEditor 抽出）。
 *
 * 职责（原 EndpointEditor「地址栏前缀/环境下拉」+「路径输入归一化/buildUrl」
 * 两大块，行为不变）：
 * - 展示前缀（唯一真实数据源）：环境 base_url 变量 > 会话 Base URL；chip 解析
 *   态（env / warn / session）与「未解析」判定；
 * - Base URL 下拉（Apifox 式前置 URL 选择器）：会话分组 / 环境行（未配 base
 *   回退会话前缀并给「未配置」微标记）/ 管理环境哨兵；
 * - 路径输入归一化：逐键原文存草稿（禁止打字中途被改写），提交时机
 *   （失焦 / 发送 / 保存 / 粘贴）才归一——完整 URL 拆 query 入参 + 前缀拆分、
 *   {{变量}} 引用成域名源、其余归一为 / 开头路径（gRPC 地址原样）；
 * - buildUrl：gRPC 直返 path；HTTP 路径变量先行代入（镜像 miyo-core
 *   replace_path_variables——后端直接用前端拼好的完整 URL）→ 环境拼接。
 *
 * 留在宿主：输入框 DOM（urlInputEl）、{{ 补全（useVarAutocomplete）、
 * commitPendingInput 组合（地址栏 + Body 防抖镜像一起刷）。
 */
import { computed, ref, type ComputedRef } from 'vue'
import { DEFAULT_SESSION_BASE_URL, type useWorkspaceStore } from '@/stores/workspace'
import type { useToast } from '@/composables/useToast'
import {
  envBaseUrl,
  environmentVariableMap,
  mergedVariableMap,
  resolveRequestUrl,
  resolveVariables,
  variableListToMap,
} from '@/utils/environment'
import { applyPathVars } from '@/utils/pathVariables'
import { envBadgeLabel as envBadgeLabelOf } from '@/utils/requestBar'
import type { Endpoint, Environment } from '@/types/miyoApi.d'

function isAbsolutePath(p: string): boolean {
  return p.startsWith('http://') || p.startsWith('https://')
}

export interface UseRequestUrlOptions {
  /** 当前草稿接口（读写 path / request.params）。 */
  draft: ComputedRef<Endpoint | null>
  store: ReturnType<typeof useWorkspaceStore>
  toast: ReturnType<typeof useToast>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
}

export function useRequestUrl(options: UseRequestUrlOptions) {
  const { draft, store, toast, t } = options

  /** 地址栏展示前缀（唯一真实数据源）：环境 base_url 变量 > 会话 Base URL。 */
  const urlDomain = computed(() => (draft.value ? store.urlDomain : ''))

  /** 激活环境（chip 色点）。 */
  const activeEnv = computed(
    () => store.environments.find((e) => e.id === store.activeEnvId) ?? null,
  )
  const activeEnvName = computed(() => activeEnv.value?.name ?? '')

  /** 环境变量 + 项目变量 + 全局变量合并表（chips / 预览解析用；单源 utils/environment.mergedVariableMap）。 */
  const envVars = computed(() =>
    mergedVariableMap({
      globalVariables: store.globalVariables,
      projectVariables: store.project?.variables,
      env: activeEnv.value,
    }),
  )

  /** 地址栏前缀 chip 文案：环境 base_url 变量的「解析后」实际值或会话 Base URL。 */
  const resolvedDomain = computed(() => {
    const src = urlDomain.value
    if (!src) return ''
    return resolveVariables(src, envVars.value)
  })

  /** chip 变量引用未解析（环境未定义该变量）。 */
  const urlUnresolved = computed(
    () => urlDomain.value.startsWith('{{') && resolvedDomain.value === urlDomain.value,
  )

  /** 基础 URL 标签样式：环境变量已解析 → 主题色；未解析 → 警告；会话回退 → 中性。 */
  const chipClass = computed(() => {
    if (urlUnresolved.value) return 'warn'
    if (urlDomain.value.startsWith('{{')) return 'env'
    return 'session'
  })

  /** Base URL 下拉「管理环境…」→ 打开环境管理。 */
  const showEnvManager = ref(false)

  /** 地址栏 Base URL 下拉（Apifox 式：前置 URL 选择器）。 */
  const BASE_URL_MANAGE = '__manage__'
  /** 下拉里的「不使用环境」分组标题（不可选、不参与键盘导航；与「管理环境…」同为零值哨兵）。 */
  const BASE_URL_SESSION_HEADER = '__session__'
  const baseUrlMenuOpen = ref(false)

  const sessionBaseVars = computed(() => ({
    ...variableListToMap(store.globalVariables),
    ...(store.project?.variables ?? {}),
  }))

  function resolveEnvBaseUrl(env: Environment): string {
    const vars = {
      ...sessionBaseVars.value,
      ...environmentVariableMap(env),
    }
    const raw = envBaseUrl(env)
    return raw ? resolveVariables(raw, vars) : ''
  }

  /** 下拉选项：`''`=无环境（会话 Base URL），各环境=解析后前置 URL，末项=管理环境。 */
  const baseUrlOptions = computed(() => {
    const opts: { value: string; label: string; header?: boolean }[] = []
    const session = store.sessionBaseUrl
      ? resolveVariables(store.sessionBaseUrl, sessionBaseVars.value)
      : ''
    // 「无环境」不是环境，是会话前缀：单列一组并给标题，避免与环境行同构排列被读成
    // 「有个叫无环境的环境指向这个 IP」（实证困惑：导入 cURL 的 origin 预填在这里）。
    opts.push({ value: BASE_URL_SESSION_HEADER, label: t('editor.baseUrlGroupSession'), header: true })
    opts.push({ value: '', label: session || t('envbar.noEnv') })
    for (const env of store.environments) {
      // 环境未配 Base URL 时真实发送前缀回退会话 Base URL（同 store.urlDomain / 发送
      // 拼接口径），左列必须显示实际发送目标。兜底「无环境」会与真正的无环境选项撞名：
      // 未配 base 的环境逐行渲染成「无环境 | 环境名」，整列多个无环境（回归 bug）。
      opts.push({ value: env.id, label: resolveEnvBaseUrl(env) || session || t('envbar.noEnv') })
    }
    opts.push({ value: BASE_URL_MANAGE, label: t('envbar.manage') })
    return opts
  })

  /** 会话前缀是否已偏离内置默认（偏离才给「无环境」行重置入口）。 */
  const sessionBaseIsCustom = computed(() => store.sessionBaseUrl !== DEFAULT_SESSION_BASE_URL)

  /** 环境是否未配 Base URL：其下拉行左列是回退的会话前缀，给「未配置」微标记去掉歧义。 */
  function envBaseUrlUnset(id: string): boolean {
    const env = store.environments.find((e) => e.id === id)
    return !!env && !envBaseUrl(env)
  }

  /** 「无环境」行重置：会话前缀回内置默认（不动当前环境选择）。 */
  function onResetSessionBaseUrl(): void {
    store.resetSessionBaseUrl()
    toast.success(t('editor.baseUrlResetDone'))
  }

  /** 下拉行右侧标签：环境名 / 无环境。 */
  function baseUrlSideLabel(value: string): string {
    if (value === BASE_URL_MANAGE) return ''
    if (!value) return t('envbar.noEnv')
    return store.environments.find((e) => e.id === value)?.name ?? ''
  }

  function onBaseUrlChange(value: string | number): void {
    const v = String(value)
    if (v === BASE_URL_MANAGE) {
      showEnvManager.value = true
      return
    }
    void store.setEnvironment(v === '' ? null : v)
  }

  /** Base URL 紧凑标签：直接展示解析后的裸域名（无域名时退回环境名），一眼可见实际发送目标。 */
  const envBadgeLabel = computed(() =>
    envBadgeLabelOf({
      urlDomain: urlDomain.value,
      resolvedDomain: resolvedDomain.value,
      envName: activeEnvName.value,
    }),
  )

  /** 路径输入框 placeholder：gRPC 提示地址格式；HTTP 提示 Base URL 拼接。 */
  const urlPlaceholder = computed(() => {
    if (draft.value?.method === 'GRPC') return t('grpc.addressPh')
    if (!urlDomain.value) return t('editor.urlPhBare')
    return t('editor.urlPhJoin', { v: resolvedDomain.value || urlDomain.value })
  })

  /**
   * 地址栏输入处理。
   *
   * `normalize=false`（逐键，v-model setter）：原文存草稿，禁止任何改写。
   * 旧实现在这里就补 `/` 前缀 / 拆 URL——打一个 `h` 瞬间被改写成 `/h`，
   * 想输自定义域名时打到 `htt…` 已被加斜杠，绝对地址分支永远无法命中。
   *
   * `normalize=true`（提交时机：失焦 / 发送 / 保存 / 粘贴）：
   * 1. 完整 URL：query 并入参数；展示前缀为环境变量时 origin+path 整条存入
   *    path（发送走 isAbsolutePath 分支，不覆写共享环境），否则 origin 写会话 Base URL。
   * 2. `{{变量}}` 引用成为域名源。
   * 3. 其余归一为 `/` 开头的路径（gRPC 地址原样：host:port / dns:/// 加斜杠会破坏格式）。
   */
  function applyUrlInput(value: string, normalize: boolean): void {
    const d = draft.value
    if (!d) return

    if (!normalize) {
      d.path = value
      return
    }

    const v = value.trim()
    if (!v) return

    // 1) 完整 URL：query 并入参数；按展示前缀决定拆分方式。
    //    主机不含空白（`[^/\s]+`），余下部分先 trim —— 手工输入或旧版本残留的空白不该
    //    被固化进会话前缀（`http://h:1 `）或拼成 `/ /x` 这种路径。
    const abs = v.match(/^(?:https?|wss?):\/\/[^/\s]+/)
    if (abs) {
      let rest = v.slice(abs[0].length).trim() || '/'
      const qIdx = rest.indexOf('?')
      if (qIdx !== -1) {
        const qs = rest.slice(qIdx + 1)
        rest = rest.slice(0, qIdx) || '/'
        for (const [key, val] of new URLSearchParams(qs).entries()) {
          d.request.params.push({ key, value: val, enabled: true, description: '', field_type: 'string', required: true, example: '' })
        }
      }
      const envPrefixed = store.urlDomain.startsWith('{{')
      // 环境前缀生效时 origin 整条进 path（发送直达导入主机），不写会话前缀：
      // 该写入会被环境前缀遮蔽，事后切「无环境」冒出没设过的域名（全仓同规则）
      if (!envPrefixed) store.sessionBaseUrl = abs[0]
      const rel = rest.startsWith('/') ? rest : `/${rest}`
      d.path = envPrefixed ? `${abs[0]}${rel}` : rel
      return
    }

    // 2) 以 `{{变量}}` 开头：变量引用成为域名源。
    const varRef = v.match(/^\{\{[^{}]+\}\}/)
    if (varRef) {
      store.sessionBaseUrl = varRef[0]
      d.path = v.slice(varRef[0].length) || '/'
      return
    }

    // 3) 其余视为路径本身（gRPC 地址原样）。
    if (d.method === 'GRPC') {
      d.path = v
      return
    }
    d.path = v.startsWith('/') ? v : `/${v}`
  }

  /** 路径输入框（与 chip 组成完整请求地址）：逐键原文存储，归一化在提交时机执行。 */
  const urlPath = computed({
    get: () => draft.value?.path ?? '',
    set: (value: string) => applyUrlInput(value, false),
  })

  /** 提交地址栏输入并归一化（失焦 / 发送 / 保存前调用；重复执行幂等）。 */
  function commitUrlPath(): void {
    if (draft.value) applyUrlInput(draft.value.path, true)
  }

  /**
   * 请求地址（与 send / 代码生成 / 压测共用）；有环境时按环境 Base URL 拼接，变量由 resolveRequestUrl 解析。
   *
   * 路径变量代入见 `utils/pathVariables.applyPathVars`（镜像 miyo-core
   * `util::replace_path_variables`）：后端 execute_request 直接使用前端拼好的
   * 完整 URL，不消费 request.path_variables（该字段只被 OpenAPI 导出与文档预览
   * 读取），故代入必须在前端完成；未配置的行不替换，路径原样发送。
   */
  function buildUrl(): string {
    const d = draft.value
    if (!d) return ''
    // gRPC：path 即完整地址（host:port，可含 {{变量}}），不拼 Base URL
    if (d.method === 'GRPC') return d.path
    const path = applyPathVars(d.path, d.request.path_variables)
    if (isAbsolutePath(path)) return path
    if (activeEnv.value && envBaseUrl(activeEnv.value)) {
      return resolveRequestUrl(activeEnv.value, path, envVars.value).url
    }
    const rel = path.startsWith('/') ? path : `/${path}`
    return `${store.urlDomain}${rel}`
  }

  return {
    urlDomain,
    activeEnv,
    activeEnvName,
    envVars,
    resolvedDomain,
    urlUnresolved,
    chipClass,
    showEnvManager,
    BASE_URL_MANAGE,
    BASE_URL_SESSION_HEADER,
    baseUrlMenuOpen,
    sessionBaseVars,
    resolveEnvBaseUrl,
    baseUrlOptions,
    sessionBaseIsCustom,
    envBaseUrlUnset,
    onResetSessionBaseUrl,
    baseUrlSideLabel,
    onBaseUrlChange,
    envBadgeLabel,
    urlPlaceholder,
    applyUrlInput,
    urlPath,
    commitUrlPath,
    buildUrl,
  }
}

