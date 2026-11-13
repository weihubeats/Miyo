/**
 * aiTest：AI 测试工作区状态（会话 / 聊天流 / 产物 / 运行链路）。
 *
 * - 聊天：`ai_chat_send` 启动后端 agent 循环，流式回传走 `miyo:ai-chat`
 *   事件（bindEvents 幂等绑定）；done / error 后重拉会话消息取权威状态；
 * - 副作用确认：`pending` 事件 → 确认卡 → aiChatApprove 放行 / 拒绝；
 * - 产物：artifact 事件 → 右栏 Tabs（场景草稿 / 用例草稿 / 修复 / 覆盖率 /
 *   依赖 / 失败分析），会话重载时从 artifact 消息恢复；
 * - 运行：复用 useScenarioRun（buildRunSteps 来自 utils/runStepBuild），
 *   场景保存后运行、失败后一键分析走同一会话。
 */
import { computed, ref, type Ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useWorkspaceStore } from '@/stores/workspace'
import { errorText } from '@/utils/errorText'
import type {
  AiArtifact,
  AiChatEvent,
  AiMessageView,
  AiSessionView,
  CoverageReport,
  DependencyReport,
  FixProposal,
  Scenario,
  ScenarioDraftResult,
  ScenarioDraftView,
  TestCaseDraft,
} from '@/types/miyoApi.d'

/** 流式中的临时条目（done 后由持久化消息替换）。 */
export interface StreamingToolCard {
  callId: string
  name: string
  args: unknown
  ok: boolean | null
  result: unknown
}

/** 产物 Tabs 顺序（渲染序）。 */
export const ARTIFACT_ORDER = [
  'scenario_draft',
  'test_case_drafts',
  'fix_proposal',
  'failure_analysis',
  'coverage_report',
  'dependency_report',
] as const

export const useAiTestStore = defineStore('aiTest', () => {
  const t = useLocaleStore().t
  const toast = useToast()
  const api = useMiyoApi()
  const workspace = useWorkspaceStore()

  // ---------- 上下文 ----------
  /** 项目选择器数据（进视图加载一次）。 */
  const projects = ref<import('@/types/miyoApi.d').Project[]>([])
  const projectId = computed(() => workspace.project?.id ?? null)
  const endpoints = computed(() => workspace.endpoints)

  /** 初始化：项目清单 + 默认选中（跟随工作区激活项目，无则选第一个）。 */
  async function initContext(): Promise<void> {
    try {
      projects.value = await api.getProjects()
    } catch {
      projects.value = []
    }
    if (!projectId.value && projects.value.length > 0) {
      await workspace.switchProject(projects.value[0]!.id)
    }
  }

  // ---------- 会话 ----------
  const sessions = ref<AiSessionView[]>([])
  const activeSessionId = ref<string | null>(null)
  const messages = ref<AiMessageView[]>([])
  const sessionsLoading = ref(false)

  // ---------- 流式状态（done 后清空，权威状态来自 messages） ----------
  const streaming = ref(false)
  const streamContent = ref('')
  const toolCards = ref<StreamingToolCard[]>([])
  const pendingApproval = ref<{
    callId: string
    name: string
    args: unknown
    risk: 'high' | 'network' | 'write'
  } | null>(null)
  const ephemeralArtifacts = ref<AiArtifact[]>([])

  // ---------- 产物（右栏） ----------
  /** 最新一份各类产物（type → artifact）。 */
  const artifacts = computed<AiArtifact[]>(() => {
    const merged = new Map<string, AiArtifact>()
    for (const m of messages.value) {
      if (m.role === 'artifact' && m.artifact) merged.set(m.artifact.type, m.artifact)
    }
    for (const a of ephemeralArtifacts.value) merged.set(a.type, a)
    return ARTIFACT_ORDER.filter((k) => merged.has(k)).map(
      (k) => merged.get(k)!,
    )
  })
  const activeArtifactType = ref<string | null>(null)
  const artifactPanelOpen = ref(true)
  watch(artifacts, (list) => {
    if (!activeArtifactType.value && list.length > 0) {
      activeArtifactType.value = list[0]!.type
      artifactPanelOpen.value = true
    }
  })

  // ---------- 场景清单（左栏业务流程） ----------
  const scenarios = ref<Scenario[]>([])
  const scenariosLoading = ref(false)
  async function loadScenarios(): Promise<void> {
    if (!projectId.value) return
    scenariosLoading.value = true
    try {
      scenarios.value = await api.listScenarios(projectId.value)
    } catch (e) {
      toast.error(errorText(e))
    } finally {
      scenariosLoading.value = false
    }
  }

  /** 接口行点击：让 AI 为该接口生成用例草稿。 */
  async function askCases(endpointId: string, name: string): Promise<void> {
    await send(t('ai.askCasesPrompt', { name, id: endpointId }))
  }

  // ---------- 直连分析（左栏） ----------
  const dependencyReport = ref<DependencyReport | null>(null)
  const coverageReport = ref<CoverageReport | null>(null)
  const analyzing = ref(false)

  // ---------- 运行链路（RunDock 组件内复用 useScenarioRun；store 只持有当前场景） ----------
  const runScenario = ref<Scenario | null>(null) as Ref<Scenario | null> // `as Ref`：绕开 UnwrapRef TS2589（运行时行为不变）
  /** 运行令牌：RunDock 监听自增后自动运行（避免跨组件直调）。 */
  const runToken = ref(0)

  // ---------- 会话管理 ----------
  async function loadSessions(): Promise<void> {
    if (!projectId.value) return
    sessionsLoading.value = true
    try {
      sessions.value = await api.aiListSessions(projectId.value)
    } finally {
      sessionsLoading.value = false
    }
  }

  async function newSession(): Promise<string> {
    const id = crypto.randomUUID()
    activeSessionId.value = id
    messages.value = []
    ephemeralArtifacts.value = []
    await loadSessions()
    return id
  }

  async function selectSession(id: string): Promise<void> {
    activeSessionId.value = id
    await reloadMessages()
    // 恢复该会话最近的场景运行上下文（若有）。
    runScenario.value = null
  }

  async function reloadMessages(): Promise<void> {
    if (!activeSessionId.value) return
    try {
      messages.value = await api.aiGetSession(activeSessionId.value)
      ephemeralArtifacts.value = []
    } catch (e) {
      toast.error(errorText(e))
    }
  }

  async function deleteSession(id: string): Promise<void> {
    await api.aiDeleteSession(id)
    if (activeSessionId.value === id) {
      activeSessionId.value = null
      messages.value = []
      ephemeralArtifacts.value = []
      pendingApproval.value = null
      toolCards.value = []
      streaming.value = false
    }
    await loadSessions()
  }

  // ---------- 聊天 ----------
  async function send(text: string): Promise<void> {
    const trimmed = text.trim()
    if (!trimmed || streaming.value) return
    const sid = activeSessionId.value ?? (await newSession())
    if (!projectId.value) {
      toast.error(t('ai.noProject'))
      return
    }
    streaming.value = true
    streamContent.value = ''
    toolCards.value = []
    // 本地立即回显用户消息（done 后以服务端为准）。
    messages.value = [
      ...messages.value,
      {
        id: `local-${Date.now()}`,
        seq: messages.value.length + 1,
        role: 'user',
        content: trimmed,
        tool_calls: null,
        tool_call_id: null,
        artifact: null,
        created_at: new Date().toISOString(),
      },
    ]
    try {
      await api.aiChatSend({
        sessionId: sid,
        message: trimmed,
        projectId: projectId.value,
        locale: useLocaleStore().mode,
      })
    } catch (e) {
      streaming.value = false
      toast.error(errorText(e))
    }
  }

  async function approve(approved: boolean): Promise<void> {
    if (!activeSessionId.value || !pendingApproval.value) return
    const { callId } = pendingApproval.value
    pendingApproval.value = null
    try {
      await api.aiChatApprove(activeSessionId.value, callId, approved)
    } catch (e) {
      toast.error(errorText(e))
    }
  }

  async function cancelChat(): Promise<void> {
    if (!activeSessionId.value) return
    try {
      await api.aiChatCancel(activeSessionId.value)
    } catch (e) {
      toast.error(errorText(e))
    }
  }

  /** 一键失败分析：以 runId 注入分析请求（走同一 agent 循环）。 */
  async function analyzeRun(runId: string): Promise<void> {
    await send(t('ai.analyzeRunPrompt', { runId }))
  }

  // ---------- 事件绑定（幂等） ----------
  let unlisten: UnlistenFn | null = null
  async function bindEvents(): Promise<void> {
    if (unlisten) return
    unlisten = await listen<AiChatEvent>('miyo:ai-chat', (ev) => {
      const p = ev.payload
      if (p.sessionId !== activeSessionId.value) return
      switch (p.type) {
        case 'delta': {
          streamContent.value += p.delta ?? ''
          break
        }
        case 'tool_start': {
          toolCards.value = [
            ...toolCards.value,
            {
              callId: p.callId ?? '',
              name: p.name ?? '',
              args: p.args,
              ok: null,
              result: undefined,
            },
          ]
          break
        }
        case 'tool_result': {
          toolCards.value = toolCards.value.map((c) =>
            c.callId === p.callId ? { ...c, ok: p.ok ?? false, result: p.result } : c,
          )
          break
        }
        case 'pending': {
          pendingApproval.value = {
            callId: p.callId ?? '',
            name: p.name ?? '',
            args: p.args,
            risk: p.risk ?? 'write',
          }
          break
        }
        case 'artifact': {
          if (p.artifactType) {
            ephemeralArtifacts.value = [
              ...ephemeralArtifacts.value.filter((a) => a.type !== p.artifactType),
              { type: p.artifactType, data: p.data },
            ]
            activeArtifactType.value = p.artifactType
            artifactPanelOpen.value = true
          }
          break
        }
        case 'done':
        case 'error':
        case 'cancelled': {
          streaming.value = false
          streamContent.value = ''
          toolCards.value = []
          pendingApproval.value = null
          void reloadMessages()
          if (p.type === 'error') {
            toast.error(p.message ?? 'AI loop error')
          }
          break
        }
      }
    })
  }

  // ---------- 直连分析 ----------
  async function loadDependencies(): Promise<void> {
    if (!projectId.value) return
    analyzing.value = true
    try {
      dependencyReport.value = await api.aiAnalyzeDependencies(projectId.value)
      activeArtifactType.value = 'dependency_report'
      artifactPanelOpen.value = true
    } catch (e) {
      toast.error(errorText(e))
    } finally {
      analyzing.value = false
    }
  }

  async function loadCoverage(): Promise<void> {
    if (!projectId.value) return
    analyzing.value = true
    try {
      coverageReport.value = await api.aiAnalyzeCoverage(projectId.value)
      activeArtifactType.value = 'coverage_report'
      artifactPanelOpen.value = true
    } catch (e) {
      toast.error(errorText(e))
    } finally {
      analyzing.value = false
    }
  }

  // ---------- 闭环动作：保存 / 用例 / 修复 ----------
  /** 保存场景草稿（预览确认后）：走 save_scenario，之后可直接运行。 */
  async function saveScenarioFromDraft(draft: ScenarioDraftResult): Promise<Scenario | null> {
    try {
      await api.saveScenario(draft.scenario)
      toast.success(t('ai.scenarioSaved', { name: draft.scenario.name }))
      runScenario.value = draft.scenario
      // 会话留痕：告知 AI 已保存（用户可继续让它调整）。
      await send(t('ai.savedNotice', { name: draft.scenario.name }))
      return draft.scenario
    } catch (e) {
      toast.error(errorText(e))
      return null
    }
  }

  /** 草稿 DTO 行 → 完整 KeyValue（设计元数据补齐 Rust 侧同款默认）。 */
  function toFullKv(kv: {
    key: string
    value: string
    enabled: boolean
    description?: string
  }): import('@/types/miyoApi.d').KeyValue {
    return {
      key: kv.key,
      value: kv.value,
      enabled: kv.enabled,
      description: kv.description ?? '',
      field_type: 'string',
      required: true,
      example: '',
    }
  }

  /** 保存勾选的用例草稿到接口（save_test_case upsert）。 */
  async function saveTestCaseDrafts(endpointId: string, drafts: TestCaseDraft[]): Promise<number> {
    let saved = 0
    for (const d of drafts) {
      try {
        await api.saveTestCase({
          id: crypto.randomUUID(),
          request_id: endpointId,
          name: d.name,
          category: (['正向', '负向', '边界值', '安全性', '其他'].includes(d.category)
            ? d.category
            : '其他') as import('@/types/miyoApi.d').TestCaseCategory,
          folder: t('ai.caseFolder'),
          method: d.method as import('@/types/miyoApi.d').HttpMethod,
          url_path: d.url_path,
          // 草稿 DTO（KeyValueDto）缺设计元数据字段，落库前补齐 Rust 侧默认值
          path_variables: d.path_variables.map(toFullKv),
          params: d.params.map(toFullKv),
          headers: d.headers.map(toFullKv),
          body_type: d.body_type,
          body_content: d.body_content,
          assertions: (Array.isArray(d.assertions) ? d.assertions : []) as import('@/types/miyoApi.d').ScenarioAssertionRule[],
          extractors: [],
          sort_order: 0,
          last_run_status: 'Untested',
          created_at: new Date().toISOString(),
        })
        saved += 1
      } catch (e) {
        toast.error(`${d.name}: ${errorText(e)}`)
      }
    }
    if (saved > 0) toast.success(t('ai.casesSaved', { n: saved }))
    return saved
  }

  /** 应用修复草稿：留档 → 回写场景。 */
  async function applyFix(proposal: FixProposal): Promise<boolean> {
    if (!runScenario.value || !proposal.draft_steps) return false
    try {
      const draftScenario: Scenario = {
        ...runScenario.value,
        steps: proposal.draft_steps,
        updated_at: new Date().toISOString(),
      }
      const draftId = await api.aiSaveScenarioDraft({
        scenarioId: runScenario.value.id,
        base: runScenario.value,
        draft: draftScenario,
        summary: proposal.analysis.root_cause_summary ?? '',
        source: 'ai_fix',
      })
      await api.aiApplyScenarioDraft(draftId)
      runScenario.value = draftScenario
      toast.success(t('ai.fixApplied'))
      return true
    } catch (e) {
      toast.error(errorText(e))
      return false
    }
  }

  /** 撤销：回写最近一份未应用 / 已应用草稿的 base。 */
  async function revertFix(scenarioId: string): Promise<boolean> {
    try {
      const drafts = await api.aiListScenarioDrafts(scenarioId, 1)
      if (drafts.length === 0) {
        toast.error(t('ai.noDraftToRevert'))
        return false
      }
      await api.aiRevertScenarioDraft(drafts[0]!.id)
      runScenario.value = drafts[0]!.base
      toast.success(t('ai.fixReverted'))
      return true
    } catch (e) {
      toast.error(errorText(e))
      return false
    }
  }

  async function listDrafts(scenarioId: string): Promise<ScenarioDraftView[]> {
    return api.aiListScenarioDrafts(scenarioId, 20)
  }

  // ---------- 运行入口 ----------
  function runSavedScenario(scenario: Scenario): void {
    runScenario.value = scenario
    runToken.value += 1
  }

  return {
    // 上下文
    projects,
    projectId,
    endpoints,
    workspace,
    initContext,
    // 会话
    sessions,
    activeSessionId,
    messages,
    sessionsLoading,
    loadSessions,
    newSession,
    selectSession,
    deleteSession,
    reloadMessages,
    // 聊天
    streaming,
    streamContent,
    toolCards,
    pendingApproval,
    send,
    approve,
    cancelChat,
    analyzeRun,
    bindEvents,
    // 产物
    artifacts,
    activeArtifactType,
    artifactPanelOpen,
    // 场景
    scenarios,
    scenariosLoading,
    loadScenarios,
    askCases,
    // 分析
    dependencyReport,
    coverageReport,
    analyzing,
    loadDependencies,
    loadCoverage,
    // 运行
    runScenario,
    runToken,
    runSavedScenario,
    // 闭环
    saveScenarioFromDraft,
    saveTestCaseDrafts,
    applyFix,
    revertFix,
    listDrafts,
  }
})
