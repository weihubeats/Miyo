/**
 * useCollectionRun：测试用例集合运行状态机（自 TestCasesPanel 抽出）。
 *
 * 收敛三处几乎相同的样板（全量运行 / 批量勾选运行 / 分组运行）：
 * 统一持有 runId、进度（collectionProgress）、取消标识与汇总 toast 口径，
 * 三个入口各缩到一行调用——改汇总提示文案不再需要同步三处。
 *
 * 三个入口的守卫差异保留原语义：
 * - 全量：runningAll 互斥；
 * - 批量：batchRunning + runningAll + 勾选非空；
 * - 分组：runningAll + 同组互斥（进度按组名比对，防串组）。
 */
import { ref } from 'vue'
import type { useWorkspaceStore } from '../stores/workspace'
import type { useToast } from './useToast'

export interface UseCollectionRunOptions {
  store: ReturnType<typeof useWorkspaceStore>
  toast: ReturnType<typeof useToast>
  /** 文案函数（组件 setup 内的 locale.t）。 */
  t: (key: string, params?: Record<string, string | number>) => string
}

export function useCollectionRun({ store, toast, t }: UseCollectionRunOptions) {
  /** 在途集合运行标识（取消按钮持有）。 */
  const activeCollectionRunId = ref<string | null>(null)
  /** 集合运行进度（done/total，已完成数由后端事件推进）。 */
  const collectionProgress = ref<{ done: number; total: number } | null>(null)
  /** 全量运行中（列表按钮禁用 + 各入口互斥）。 */
  const runningAll = ref(false)
  /** 批量运行中（批量按钮禁用）。 */
  const batchRunning = ref(false)
  /** 分组运行态（组头内联进度 + 独立取消；按组名比对防串）。 */
  const groupRun = ref<{ name: string; runId: string; done: number; total: number } | null>(null)

  /** 汇总提示：全部成功 success，否则 info（cancelled 单独文案）。 */
  function summaryToast(r: { success: number; total: number; cancelled?: boolean }): void {
    if (r.cancelled) {
      toast.info(t('cases.runCancelled', { ok: r.success, total: r.total }))
      return
    }
    toast[r.success === r.total ? 'success' : 'info'](
      t('cases.runAllDone', { ok: r.success, total: r.total }),
    )
  }

  function failToast(err: unknown): void {
    toast.error(t('cases.runAllFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }

  /**
   * 集合运行核心：领 runId → 置进度 → store.runAllTestCases → 汇总 toast →
   * 清理（仅当 runId 未被更新覆盖，防并发入口互踩）。
   */
  async function runCases(
    endpointId: string,
    opts: { ids?: string[]; folder?: string; initialTotal: number; showCancelled: boolean },
  ): Promise<void> {
    const runId = crypto.randomUUID()
    activeCollectionRunId.value = runId
    collectionProgress.value = { done: 0, total: opts.initialTotal }
    try {
      const r = await store.runAllTestCases(endpointId, {
        runId,
        ...(opts.ids ? { ids: opts.ids } : {}),
        ...(opts.folder ? { folder: opts.folder } : {}),
        onProgress: (done, total) => {
          collectionProgress.value = { done, total }
        },
      })
      if (opts.showCancelled || !r.cancelled) summaryToast(r)
    } catch (err) {
      failToast(err)
    } finally {
      if (activeCollectionRunId.value === runId) activeCollectionRunId.value = null
      collectionProgress.value = null
    }
  }

  /** 全量运行（入口守卫：runningAll 互斥）。 */
  async function runAll(endpointId: string): Promise<void> {
    if (runningAll.value) return
    runningAll.value = true
    try {
      await runCases(endpointId, {
        initialTotal: store.testCases.get(endpointId)?.length ?? 0,
        showCancelled: true,
      })
    } finally {
      runningAll.value = false
    }
  }

  /** 批量运行勾选集（守卫：勾选非空 + batchRunning + runningAll 互斥）。 */
  async function runBatch(endpointId: string, ids: string[]): Promise<void> {
    if (!ids.length || batchRunning.value || runningAll.value) return
    batchRunning.value = true
    try {
      await runCases(endpointId, { ids, initialTotal: ids.length, showCancelled: false })
    } finally {
      batchRunning.value = false
    }
  }

  /** 分组运行（守卫：runningAll + 同组互斥；进度按组名比对防串）。 */
  async function runGroup(endpointId: string, name: string): Promise<void> {
    if (runningAll.value || groupRun.value) return
    groupRun.value = { name, runId: crypto.randomUUID(), done: 0, total: 0 }
    try {
      const r = await store.runAllTestCases(endpointId, {
        runId: groupRun.value.runId,
        folder: name,
        onProgress: (done, total) => {
          if (groupRun.value?.name === name) {
            groupRun.value = { ...groupRun.value, done, total }
          }
        },
      })
      if (r.cancelled) {
        toast.info(t('cases.runCancelled', { ok: r.success, total: r.total }))
      } else {
        summaryToast(r)
      }
    } catch (err) {
      failToast(err)
    } finally {
      if (groupRun.value?.name === name) groupRun.value = null
    }
  }

  /** 取消在途集合运行：后端停止领新任务，已完成项保留。 */
  function cancelActive(): void {
    const rid = activeCollectionRunId.value
    if (rid) {
      void store.cancelAllTestCases(rid)
      toast.info(t('cases.cancelling'))
      return
    }
    if (groupRun.value) {
      void store.cancelAllTestCases(groupRun.value.runId)
      toast.info(t('cases.cancelling'))
    }
  }

  /** 取消分组运行（组头独立取消入口）。 */
  function cancelGroupRun(): void {
    if (!groupRun.value) return
    void store.cancelAllTestCases(groupRun.value.runId)
    toast.info(t('cases.cancelling'))
  }

  return {
    activeCollectionRunId,
    collectionProgress,
    runningAll,
    batchRunning,
    groupRun,
    runAll,
    runBatch,
    runGroup,
    cancelActive,
    cancelGroupRun,
  }
}
