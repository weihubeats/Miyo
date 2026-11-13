/**
 * aiTest store：聊天事件流（流式 / 工具卡 / 审批 / 产物）、产物合并、
 * 会话恢复。事件用模拟的 listen 回调驱动（bindEvents 幂等绑定）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

// listen 捕获回调，测试直接投递事件。
let chatListener: ((ev: { payload: unknown }) => void) | null = null
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event: string, cb: (ev: { payload: unknown }) => void) => {
    chatListener = cb
    return () => {}
  }),
}))

/** 稳定的 mock 实例（每次 useMiyoApi() 返回同一组 vi.fn，供断言取用）。 */
const apiFns = {
  aiGetSession: vi.fn(async () => []),
  aiListSessions: vi.fn(async () => []),
  aiChatSend: vi.fn(async () => {}),
  aiChatApprove: vi.fn(async () => {}),
  aiChatCancel: vi.fn(async () => {}),
  aiListScenarioDrafts: vi.fn(async () => []),
  aiSaveScenarioDraft: vi.fn(async () => 'draft-1'),
  aiApplyScenarioDraft: vi.fn(async () => {}),
  aiRevertScenarioDraft: vi.fn(async () => {}),
  saveScenario: vi.fn(async (s: unknown) => s),
  saveTestCase: vi.fn(async (c: unknown) => c),
  listScenarios: vi.fn(async () => []),
  aiAnalyzeDependencies: vi.fn(async () => ({ edges: [], nodes: [], unresolved: [], summary: '' })),
  aiAnalyzeCoverage: vi.fn(async () => ({ score: 80, dimensions: [], endpoints: [], summary: '' })),
  aiGenerateTestCaseDrafts: vi.fn(async () => []),
  aiProposeFix: vi.fn(async () => ({})),
}

vi.mock('@/composables/useMiyoApi.ts', () => ({
  useMiyoApi: () => ({ pending: { value: false }, ...apiFns }),
}))

vi.mock('@/stores/workspace.ts', async () => {
  const actual = await vi.importActual<typeof import('@/stores/workspace')>('@/stores/workspace')
  return actual
})

import { useAiTestStore, ARTIFACT_ORDER } from '@/domains/agent/stores/aiTest'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
function emit(payload: Record<string, unknown>): void {
  chatListener?.({ payload })
}

describe('aiTest store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    useLocaleStore().setMode('zh')
    chatListener = null
  })

  it('bindEvents 幂等：多次调用只注册一个监听', async () => {
    const store = useAiTestStore()
    await store.bindEvents()
    await store.bindEvents()
    const { listen } = await import('@tauri-apps/api/event')
    expect(listen).toHaveBeenCalledTimes(1)
  })

  it('流式事件：delta 累积、工具卡状态、done 后重拉消息并清空临时态', async () => {
    const store = useAiTestStore()
    const ws = useWorkspaceStore()
    ws.project = { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' }
    await store.bindEvents()

    await store.newSession()
    expect(store.activeSessionId).toBeTruthy()
    const sid = store.activeSessionId!

    // 模拟后端事件序列。
    emit({ sessionId: sid, type: 'delta', delta: '正在分析' })
    emit({ sessionId: sid, type: 'delta', delta: '接口…' })
    emit({ sessionId: sid, type: 'tool_start', callId: 'c1', name: 'analyze_api_dependencies', args: {} })
    emit({ sessionId: sid, type: 'tool_result', callId: 'c1', name: 'analyze_api_dependencies', ok: true, result: {} })
    emit({
      sessionId: sid,
      type: 'artifact',
      artifactType: 'dependency_report',
      data: { edges: [{ from: 'a', to: 'b', via: ['x'], confidence: 'high', evidence: 'e' }], nodes: [], unresolved: [], summary: 's' },
    })
    emit({ sessionId: sid, type: 'done', stopReason: 'completed' })

    expect(store.streamContent).toBe('')
    expect(store.toolCards).toHaveLength(0)
    expect(store.streaming).toBe(false)
    // done 触发 reload → aiGetSession 被调用。
    expect(apiFns.aiGetSession).toHaveBeenCalledWith(sid)
  })

  it('pending 事件 → 待确认卡；approve 调后端并清空', async () => {
    const store = useAiTestStore()
    const ws = useWorkspaceStore()
    ws.project = { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' }
    await store.bindEvents()
    await store.newSession()
    const sid = store.activeSessionId!

    emit({
      sessionId: sid,
      type: 'pending',
      callId: 'c9',
      name: 'run_scenario',
      args: { scenarioId: 'x' },
      risk: 'write',
    })
    expect(store.pendingApproval).toEqual({
      callId: 'c9',
      name: 'run_scenario',
      args: { scenarioId: 'x' },
      risk: 'write',
    })
    await store.approve(true)
    expect(store.pendingApproval).toBeNull()
    expect(apiFns.aiChatApprove).toHaveBeenCalledWith(sid, 'c9', true)
  })

  it('其他会话的事件不串扰', async () => {
    const store = useAiTestStore()
    const ws = useWorkspaceStore()
    ws.project = { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' }
    await store.bindEvents()
    await store.newSession()
    emit({ sessionId: 'other-session', type: 'delta', delta: '别家的字' })
    expect(store.streamContent).toBe('')
  })

  it('产物合并：持久化消息 + 临时产物按最新去重，顺序按 ARTIFACT_ORDER', async () => {
    const store = useAiTestStore()
    const ws = useWorkspaceStore()
    ws.project = { id: 'p-1', name: 'A', description: '', variables: {}, created_at: '', updated_at: '' }
    await store.bindEvents()
    await store.newSession()

    emit({
      sessionId: store.activeSessionId,
      type: 'artifact',
      artifactType: 'coverage_report',
      data: { score: 1 },
    })
    emit({
      sessionId: store.activeSessionId,
      type: 'artifact',
      artifactType: 'scenario_draft',
      data: { scenario: { steps: [] } },
    })
    // 同类再发一次：取最新（去重）。
    emit({
      sessionId: store.activeSessionId,
      type: 'artifact',
      artifactType: 'coverage_report',
      data: { score: 2 },
    })

    const types = store.artifacts.map((a) => a.type)
    expect(types).toEqual(ARTIFACT_ORDER.filter((k) => types.includes(k)))
    expect(types).toEqual(['scenario_draft', 'coverage_report'])
    const cov = store.artifacts.find((a) => a.type === 'coverage_report')!
    expect((cov.data as { score: number }).score).toBe(2)
    // 新产物自动激活右栏 Tab。
    expect(store.activeArtifactType).toBe('coverage_report')
    expect(store.artifactPanelOpen).toBe(true)
  })

  it('fix 流程：applyFix 留档后应用草稿', async () => {
    const store = useAiTestStore()
    store.runScenario = {
      id: 'sc-1',
      project_id: 'p-1',
      name: '下单',
      description: '',
      category: '',
      sort_order: 0,
      steps: [],
      created_at: '',
      updated_at: '',
    }
    const ok = await store.applyFix({
      run_id: 'r1',
      scenario_id: 'sc-1',
      analysis: { ok: false, total_steps: 2, failed: 1, root_cause_summary: '根因', rerun_advice: null, failed_steps: [], disclaimer: '' },
      changes: [],
      draft_steps: [],
      notes: null,
      disclaimer: '',
    } as never)
    expect(ok).toBe(true)
    expect(apiFns.aiSaveScenarioDraft).toHaveBeenCalled()
    expect(apiFns.aiApplyScenarioDraft).toHaveBeenCalledWith('draft-1')
  })
})
