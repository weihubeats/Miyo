<script setup lang="ts">
/**
 * ScenarioLibrary：集成测试的场景库（列表页）。
 *
 * - 数据走 integration store 缓存（list_scenario_summaries 单次 IPC）；
 * - 搜索（名称 / 描述）+ 分类 chip 过滤（全部 / 未分类 / 各分类）；
 * - 每行：最近执行状态点 + 名称 + 分类 + 步骤数 + 最近执行时间 + 结果摘要；
 * - 操作：运行（直接跑最近保存的配置）/ 编辑 / 复制 / 导出 / 删除；
 * - 新建（可选分类）+ 导入（openDialog 选文件 → readTextFile → 解析入库）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useTauriEvent } from '@/composables/useTauriEvent'
import { errorText } from '@/utils/errorText'
import { buildScenarioExport, parseScenarioImport } from '@/utils/scenarioIO'
import { formatDateTime } from '@/utils/dateTime'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import RunHistoryPanel from '@/domains/integration/components/integration/RunHistoryPanel.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Modal from '@/components/ui/Modal.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import { formatDurationShort } from '@/utils/format'
import type {
  MatrixRunOutcome,
  Scenario,
  ScenarioRunSummary,
  ScenarioStep,
} from '@/types/miyoApi.d'

const t = useLocaleStore().t
const store = useIntegrationStore()
const workspace = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const router = useRouter()

// ---------- 搜索 + 分类过滤 + 状态过滤 + 排序 ----------
// 视图状态持久化在 integration store（libraryView）：进编辑器再返回时恢复
// 搜索 / 筛选 / 排序 / 滚动位置（导航规范场景 A；store 跨路由存活，返回不重放请求）。
const view = store.libraryView
const search = computed({
  get: () => view.search,
  set: (v: string) => (view.search = v),
})
const categoryFilter = computed({
  get: () => view.categoryFilter,
  set: (v: string | null) => (view.categoryFilter = v),
})
/** 最近执行状态过滤：全部 | 通过 | 失败 | 未运行。 */
const statusFilter = computed({
  get: () => view.statusFilter,
  set: (v: 'all' | 'ok' | 'fail' | 'none') => (view.statusFilter = v),
})
/** 排序：默认（编排序）| 名称 | 最近执行时间倒序。 */
const sortBy = computed({
  get: () => view.sortBy,
  set: (v: 'default' | 'name' | 'recent') => (view.sortBy = v),
})

/** 全部分类（去重、保序）。 */
const categories = computed(() => {
  const set: string[] = []
  for (const s of store.libraryItems) {
    const c = s.category.trim()
    if (c && !set.includes(c)) set.push(c)
  }
  return set
})

const hasCategory = computed(() => store.libraryItems.some((s) => s.category.trim()))

/** 最近一次执行的结果判定（状态过滤用）。 */
function lastRunResult(item: (typeof store.libraryItems)[number]): 'ok' | 'fail' | 'cancel' | null {
  const r = item.last_run
  if (!r) return null
  if (r.cancelled) return 'cancel'
  return r.fail_count > 0 ? 'fail' : 'ok'
}

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  const list = store.libraryItems.filter((s) => {
    if (categoryFilter.value === '' && s.category.trim()) return false
    if (categoryFilter.value && s.category.trim() !== categoryFilter.value) return false
    if (statusFilter.value !== 'all') {
      const res = lastRunResult(s)
      if (statusFilter.value === 'ok' && res !== 'ok') return false
      if (statusFilter.value === 'fail' && res !== 'fail') return false
      if (statusFilter.value === 'none' && res !== null) return false
    }
    if (!q) return true
    return (
      s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q)
    )
  })
  if (sortBy.value === 'name') {
    return [...list].sort((a, b) => a.name.localeCompare(b.name))
  }
  if (sortBy.value === 'recent') {
    return [...list].sort((a, b) => {
      const ta = a.last_run ? Date.parse(a.last_run.finished_at) : 0
      const tb = b.last_run ? Date.parse(b.last_run.finished_at) : 0
      return tb - ta
    })
  }
  return list
})

// ---------- 最近执行状态 ----------
type RunTone = 'ok' | 'fail' | 'cancel' | null

/** 环境徽章：last_run.environment_id → 环境名（已删环境回退短 id）。 */
const envNameById = computed(() => {
  const m = new Map<string, string>()
  for (const e of workspace.environments) m.set(e.id, e.name)
  return m
})

function envBadgeName(item: (typeof store.libraryItems)[number]): string | null {
  const id = item.last_run?.environment_id
  if (!id) return null
  return envNameById.value.get(id) ?? id.slice(0, 8)
}

/** 最近一次运行成功率：ok / (ok + fail + skip)；从未运行返回 null。 */
function successRate(item: (typeof store.libraryItems)[number]): number | null {
  const r = item.last_run
  if (!r) return null
  const total = r.ok_count + r.fail_count + r.skipped_count
  if (total <= 0) return null
  return Math.round((r.ok_count / total) * 100)
}

function runTone(item: (typeof store.libraryItems)[number]): RunTone {
  return lastRunResult(item)
}

function runSummaryText(r: ScenarioRunSummary | null | undefined): string {
  if (!r) return t('integration.libLastRunNone')
  const parts = [
    t('scenario.summaryOk', { n: r.ok_count }),
    r.fail_count > 0 ? t('scenario.summaryFail', { n: r.fail_count }) : null,
    r.skipped_count > 0 ? t('scenario.summarySkip', { n: r.skipped_count }) : null,
  ].filter(Boolean)
  return `${t('integration.libLastRun', { time: formatDateTime(r.finished_at) })} · ${parts.join(' / ')}`
}

// ---------- AI 创建入口（空态引导：AI 生成场景草稿后一键应用） ----------
function goAiCreate(): void {
  void router.push('/ai-test')
}

// ---------- 列表滚动位置记忆（随 store.libraryView 跨路由存活） ----------
const listEl = ref<HTMLElement | null>(null)

function saveScroll(): void {
  view.scrollTop = listEl.value?.scrollTop ?? 0
  view.projectIdAtScroll = store.projectId
}

/** 返回列表恢复滚动：同项目且有记忆位置时恢复（列表项来自缓存，一帧内可渲染）。 */
async function restoreScroll(): Promise<void> {
  const top = view.scrollTop
  if (top <= 0 || view.projectIdAtScroll !== store.projectId) return
  await nextTick()
  if (listEl.value) listEl.value.scrollTop = top
}

onMounted(() => {
  listEl.value?.addEventListener('scroll', saveScroll, { passive: true })
  void restoreScroll()
})

onBeforeUnmount(() => {
  listEl.value?.removeEventListener('scroll', saveScroll)
})

// ---------- 项目级执行历史面板 ----------
const historyPanelOpen = ref(false)

/** 面板里点开某条运行：直达对应场景的结果视图（编辑器恢复该次运行）。 */
function onHistoryOpenRun(payload: { scenarioId: string; runId: string }): void {
  store.openScenarioRun(payload.scenarioId, payload.runId)
}

// ---------- 新建 / 复制 ----------
async function onCreate(): Promise<void> {
  const now = new Date().toISOString()
  const scenario: Scenario = {
    id: crypto.randomUUID(),
    project_id: store.projectId ?? '',
    name: t('integration.newName'),
    description: '',
    category: categoryFilter.value || '',
    sort_order: store.libraryItems.length,
    steps: [],
    script: null,
    created_at: now,
    updated_at: now,
  }
  try {
    const saved = await api.saveScenario(scenario)
    await store.loadLibrary(true)
    store.openScenario(saved.id)
  } catch (err) {
    toast.error(t('integration.saveFail'), {
      message: errorText(err),
    })
  }
}

async function onDuplicate(s: Scenario): Promise<void> {
  const now = new Date().toISOString()
  const copy: Scenario = {
    ...s,
    id: crypto.randomUUID(),
    name: t('scenario.copyName', { name: s.name }),
    sort_order: store.libraryItems.length,
    steps: JSON.parse(JSON.stringify(s.steps)) as ScenarioStep[],
    script: s.script ?? null,
    created_at: now,
    updated_at: now,
  }
  try {
    const saved = await api.saveScenario(copy)
    toast.success(t('integration.duplicated', { name: saved.name }))
    await store.loadLibrary(true)
  } catch (err) {
    toast.error(t('integration.saveFail'), {
      message: errorText(err),
    })
  }
}

async function onDelete(id: string): Promise<void> {
  try {
    await api.deleteScenario(id)
    store.forgetScenario(id)
  } catch (err) {
    toast.error(t('integration.saveFail'), {
      message: errorText(err),
    })
  }
}

// ---------- 数据驱动 / 环境矩阵运行（引擎升级：每组数据独立执行） ----------

/** 矩阵实时进度（groupId → {done,total,label,ok}；卡片角标渲染）。 */
const matrixProgress = ref<{
  groupId: string
  done: number
  total: number
  label: string
  ok: boolean
} | null>(null)
/** 矩阵结果弹窗（运行完成后展示逐行对比 + 打开行详情）。 */
const matrixResult = ref<MatrixRunOutcome | null>(null)
const matrixResultOpen = ref(false)
const runningMatrixScenarioId = ref<string | null>(null)
useTauriEvent<{
  groupId: string
  index: number
  total: number
  label: string
  ok: boolean
  runId: string
}>('miyo:matrix-progress', (p) => {
  matrixProgress.value = {
    groupId: p.groupId,
    done: p.index,
    total: p.total,
    label: p.label,
    ok: p.ok,
  }
})

/** 矩阵行物化：数据集行（inline/json/csv 解析为变量行）或环境行。 */
async function buildMatrixRows(
  s: Scenario,
  mode: 'data' | 'environment',
): Promise<{ key: string; label: string; environmentId?: string | null; vars?: Record<string, string> | null }[]> {
  if (mode === 'data') {
    const rows: { key: string; label: string; vars: Record<string, string> }[] = []
    for (const ds of s.data_sets ?? []) {
      const parsed: Record<string, string>[] = []
      if (ds.source.kind === 'inline') {
        for (const r of ds.source.rows) {
          parsed.push(
            Object.fromEntries(
              Object.entries(r).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)]),
            ),
          )
        }
      } else if (ds.source.kind === 'json') {
        const arr = JSON.parse(ds.source.text) as Record<string, unknown>[]
        for (const r of arr) {
          parsed.push(
            Object.fromEntries(
              Object.entries(r).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)]),
            ),
          )
        }
      } else if (ds.source.kind === 'csv') {
        const lines = ds.source.text.split('\n').filter((l) => l.trim())
        if (lines.length < 2) continue
        const header = lines[0]!.split(',').map((h) => h.trim())
        for (const line of lines.slice(1)) {
          const cells = line.split(',')
          parsed.push(Object.fromEntries(header.map((h, i) => [h, (cells[i] ?? '').trim()])))
        }
      }
      if (!parsed.length) {
        toast.warning(t('integration.matrixEmptySet', { name: ds.name }))
        continue
      }
      parsed.forEach((vars, i) => {
        rows.push({
          key: `${ds.id}-${i}`,
          label: parsed.length > 1 ? `${ds.name}#${i + 1}` : ds.name,
          vars,
        })
      })
    }
    return rows
  }
  // 环境矩阵：每环境一行（变量取该环境合并结果——后端按行独立解析）。
  const envs = await api.listEnvironments(s.project_id)
  return envs.map((e) => ({ key: e.id, label: e.name, environmentId: e.id }))
}

/** 矩阵运行：步骤 = 场景主步骤物化（引用式 → 内联请求），生命周期随场景。 */
async function runMatrix(s: Scenario, mode: 'data' | 'environment'): Promise<void> {
  runningMatrixScenarioId.value = s.id
  matrixScenarioId.value = s.id
  try {
    const rows = await buildMatrixRows(s, mode)
    if (!rows.length) {
      toast.warning(
        mode === 'data' ? t('integration.matrixNoData') : t('integration.matrixNoEnv'),
      )
      return
    }
    const steps = (s.steps ?? []).filter((st) => st.enabled).map((st) => {
      if (st.request) {
        return {
          endpoint: {
            id: st.request.url,
            project_id: '',
            folder_id: null,
            name: st.name || `step`,
            method: st.request.method,
            path: st.request.url,
            description: '',
            status: 'developing' as const,
            sort_order: 0,
            request: st.request.spec,
            created_at: '',
            updated_at: '',
          },
          url: st.request.url,
          enabled: true,
          extract: st.extract ?? [],
          assertions: st.assertions ?? [],
          condition: st.condition ?? null,
          loop: st.loop ?? null,
          wait_ms: st.wait_ms ?? null,
          parallel: st.parallel ?? false,
          retry: st.retry ?? null,
        }
      }
      const ep = workspace.endpoints.find((e) => e.id === st.endpoint_id)
      if (!ep) return null
      return {
        endpoint: ep,
        url: `{{base_url}}${ep.path.startsWith('/') ? ep.path : `/${ep.path}`}`,
        enabled: true,
        extract: st.extract ?? [],
        assertions: st.assertions ?? [],
        condition: st.condition ?? null,
        loop: st.loop ?? null,
        wait_ms: st.wait_ms ?? null,
        parallel: st.parallel ?? false,
        retry: st.retry ?? null,
      }
    }).filter((x): x is NonNullable<typeof x> => x !== null)
    if (!steps.length) {
      toast.warning(t('scenario.noEnabledSteps'))
      return
    }
    const outcome = await api.runScenarioMatrix({
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      steps: steps as any,
      setup: [],
      teardown: [],
      teardownEnabled: s.teardown_enabled ?? true,
      rows,
      stopOnFailure: false,
      groupId: crypto.randomUUID(),
      scenarioId: s.id,
    })
    const failed = outcome.total - outcome.okRows
    matrixProgress.value = null
    runningMatrixScenarioId.value = null
    matrixResult.value = outcome
    matrixResultOpen.value = true
    toast.success(
      t('integration.matrixDone', {
        total: outcome.total,
        ok: outcome.okRows,
        fail: failed,
      }),
      { duration: 8000 },
    )
    await store.loadLibrary(true)
  } catch (err) {
    toast.error(t('integration.matrixFail'), {
      message: errorText(err),
    })
  }
}

/** 矩阵结果弹窗：打开某行运行详情（历史直达）。 */
function openMatrixRow(i: number): void {
  const row = matrixResult.value?.rows[i]
  if (!row) return
  const sc = sId()
  if (sc) store.openScenarioRun(sc, row.runId)
  matrixResultOpen.value = false
}

/** 矩阵运行的场景 id（runMatrix 时记录）。 */
function sId(): string | null {
  return matrixScenarioId.value
}
const matrixScenarioId = ref<string | null>(null)

// ---------- 导出 / 导入 ----------
async function onExport(s: Scenario): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, title: t('integration.export') })
    if (!dir || Array.isArray(dir)) return
    const base = s.name.replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await api.joinPath(dir, `${base}.miyo-scenario.json`)
    const { json, redactedFields } = buildScenarioExport(
      s,
      (id) => workspace.endpoints.find((e) => e.id === id),
    )
    await api.writeTextFile(path, json)
    // 导出前预览（脱敏提示）：告知哪些敏感值被替换为占位符，需接手方回填。
    if (redactedFields.length) {
      toast.info(t('integration.exportRedacted'), {
        message: redactedFields.slice(0, 10).join('；'),
        duration: 8000,
      })
    }
    toast.success(t('integration.exported', { path }))
  } catch (err) {
    toast.error(t('integration.exportFail'), {
      message: errorText(err),
    })
  }
}

async function onImport(): Promise<void> {
  try {
    const file = await openDialog({
      multiple: false,
      title: t('integration.importFileTitle'),
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!file || Array.isArray(file)) return
    const text = await api.readTextFile(file)
    const parsed = parseScenarioImport(text)
    const now = new Date().toISOString()
    const scenario: Scenario = {
      id: crypto.randomUUID(),
      project_id: store.projectId ?? '',
      name: parsed.name,
      description: parsed.description,
      category: parsed.category,
      sort_order: store.libraryItems.length,
      steps: parsed.steps,
      script: parsed.script,
      created_at: now,
      updated_at: now,
    }
    const saved = await api.saveScenario(scenario)
    toast.success(t('integration.imported', { name: saved.name }))
    await store.loadLibrary(true)
  } catch (err) {
    const raw = errorText(err)
    toast.error(raw === 'invalid-json' || raw === 'invalid-shape' ? t('integration.importInvalid') : t('integration.importFail'), {
      message: raw,
    })
  }
}
</script>

<template>
  <div ref="listEl" class="lib-root">
    <div class="lib-toolbar">
      <SearchInput v-model="search" class="lib-search" :placeholder="t('integration.libSearchPh')" />
      <IconButton name="upload" :size="14" :title="t('integration.libImport')" @click="onImport" />
      <IconButton name="history" :size="14" :title="t('integration.histProjectTitle')" @click="historyPanelOpen = true" />
      <CustomSelect
        class="lib-sort"
        :model-value="String(sortBy)"
        :options="[
          { value: 'default', label: t('integration.libSortDefault') },
          { value: 'name', label: t('integration.libSortName') },
          { value: 'recent', label: t('integration.libSortRecent') },
        ]"
        size="sm"
        :pop-min-width="130"
        @update:model-value="(v) => (sortBy = String(v ?? 'default') as 'default' | 'name' | 'recent')"
      />
      <CustomSelect
        class="lib-status-filter"
        :model-value="String(statusFilter)"
        :options="[
          { value: 'all', label: t('integration.libStatusAll') },
          { value: 'ok', label: t('integration.libStatusOk') },
          { value: 'fail', label: t('integration.libStatusFail') },
          { value: 'none', label: t('integration.libStatusNone') },
        ]"
        size="sm"
        :pop-min-width="110"
        @update:model-value="(v) => (statusFilter = String(v ?? 'all') as 'all' | 'ok' | 'fail' | 'none')"
      />
      <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="onCreate">
        <Icon name="plus" :size="13" /> {{ t('integration.libNew') }}
      </button>
    </div>

    <!-- 分类过滤 chips -->
    <div v-if="hasCategory" class="lib-cats">
      <button
        class="lib-cat"
        :class="{ active: categoryFilter === null }"
        type="button"
        @click="categoryFilter = null"
      >
        {{ t('integration.libAllCategories') }}
      </button>
      <button
        v-for="c in categories"
        :key="c"
        class="lib-cat"
        :class="{ active: categoryFilter === c }"
        type="button"
        @click="categoryFilter = categoryFilter === c ? null : c"
      >
        {{ c }}
      </button>
      <button
        class="lib-cat"
        :class="{ active: categoryFilter === '' }"
        type="button"
        @click="categoryFilter = categoryFilter === '' ? null : ''"
      >
        {{ t('integration.libUncategorized') }}
      </button>
    </div>

    <div v-if="store.libraryItems.length" class="lib-list">
      <div
        v-for="s in filtered"
        :key="s.id"
        class="lib-row"
        role="button"
        tabindex="0"
        @click="store.openScenario(s.id)"
        @keydown.enter="store.openScenario(s.id)"
      >
        <span class="lib-run-dot" :class="runTone(s)" :title="runSummaryText(s.last_run)" aria-hidden="true"></span>
        <div class="lib-main">
          <span class="lib-name" :title="s.name">{{ s.name }}</span>
          <span v-if="s.category" class="lib-cat-tag" :title="s.category">{{ s.category }}</span>
        </div>
        <span class="lib-meta">{{ t('integration.libSteps', { n: s.steps.length }) }}</span>
        <span v-if="successRate(s) !== null" class="lib-rate" :class="{ 'lib-rate-bad': (successRate(s) ?? 100) < 100 }" :title="runSummaryText(s.last_run)">
          {{ successRate(s) }}%
        </span>
        <span v-else class="lib-meta">—</span>
        <span v-if="s.last_run" class="lib-meta num" :title="runSummaryText(s.last_run)">
          {{ formatDurationShort(s.last_run.duration_ms ?? 0) }}
        </span>
        <span v-else class="lib-meta">—</span>
        <span v-if="envBadgeName(s)" class="lib-env" :title="t('integration.libEnvBadge', { name: envBadgeName(s) ?? '' })">
          {{ envBadgeName(s) }}
        </span>
        <span v-else class="lib-meta">—</span>
        <span class="lib-meta lib-time" :title="runSummaryText(s.last_run)">
          {{ s.last_run ? formatDateTime(s.last_run.finished_at) : t('integration.libLastRunNone') }}
        </span>
        <span class="lib-actions">
          <IconButton
            name="history"
            :size="13"
            :title="t('integration.libHistory')"
            :disabled="!s.last_run"
            @click.stop="s.last_run && store.openScenarioRun(s.id, s.last_run.id)"
          />
          <span
            v-if="matrixProgress && runningMatrixScenarioId === s.id"
            class="num text-xxs font-medium text-accent"
            :title="t('integration.matrixRunning', { done: matrixProgress.done, total: matrixProgress.total, label: matrixProgress.label })"
          >
            {{ matrixProgress.done }}/{{ matrixProgress.total }}
          </span>
          <IconButton name="play" :size="13" :title="t('integration.run')" @click.stop="store.openScenario(s.id)" />
          <IconButton
            name="list"
            :size="13"
            :title="t('integration.matrixDataRun')"
            :disabled="!(s.data_sets?.length ?? 0)"
            @click.stop="runMatrix(s, 'data')"
          />
          <IconButton
            name="globe"
            :size="13"
            :title="t('integration.matrixEnvRun')"
            @click.stop="runMatrix(s, 'environment')"
          />
          <IconButton name="copy" :size="13" :title="t('integration.duplicate')" @click.stop="onDuplicate(s)" />
          <IconButton name="download" :size="13" :title="t('integration.export')" @click.stop="onExport(s)" />
          <Popconfirm
            :title="t('integration.confirmDelete', { name: s.name })"
            :confirm-text="t('common.delete')"
            @confirm="onDelete(s.id)"
          >
            <IconButton name="trash" :size="13" :title="t('scenario.delete')" @click.stop />
          </Popconfirm>
        </span>
      </div>
      <p v-if="!filtered.length" class="lib-no-match">{{ t('common.noResults') }}</p>
    </div>

    <EmptyState v-else-if="!store.libraryLoading" icon="beaker" :title="t('integration.libEmptyTitle')" :description="t('integration.libEmptyDesc')">
      <div class="lib-empty-actions">
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="onCreate">
          <Icon name="plus" :size="13" /> {{ t('integration.libNew') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="goAiCreate">
          <Icon name="zap" :size="13" /> {{ t('integration.libEmptyAi') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="onImport">
          <Icon name="upload" :size="13" /> {{ t('integration.libImport') }}
        </button>
      </div>
      <p class="lib-empty-hint">{{ t('integration.libEmptyFromEndpoint') }}</p>
    </EmptyState>

  <!-- 项目级执行历史：全部场景的最近运行流（筛选 / 直达详情 / 删除） -->
  <RunHistoryPanel
    v-model:open="historyPanelOpen"
    :project-id="store.projectId"
    @open-run="onHistoryOpenRun"
  />
  </div>
<!-- 矩阵结果：逐行对比（行点击 → 打开该行运行详情） -->
<Modal v-model:open="matrixResultOpen" :title="t('integration.matrixResultTitle')" width="520px">
  <div class="flex flex-col gap-1.5">
    <div
      v-for="(row, i) in matrixResult?.rows ?? []"
      :key="row.key"
      class="mx-row"
    >
      <button
        type="button"
        class="min-w-0 flex-1 text-left"
        @click="openMatrixRow(i)"
      >
        <div class="flex items-center gap-2">
          <span
            class="mx-dot"
            :class="row.ok ? 'mx-dot-ok' : 'mx-dot-fail'"
            aria-hidden="true"
          ></span>
          <span class="text-xs font-medium text-ink1">{{ row.label }}</span>
        </div>
        <p class="mt-0.5 text-xxs text-ink3">
          {{ t('scenario.assertPassed', { n: row.okCount, m: row.okCount + row.failCount + row.skippedCount }) }}
          <template v-if="row.durationMs != null"> · {{ Math.round(row.durationMs) }}ms</template>
        </p>
      </button>
      <code class="font-mono text-xxs text-ink3">{{ row.runId.slice(0, 8) }}</code>
    </div>
  </div>
</Modal>
</template>

<style scoped>
/* 矩阵结果行卡（跟随主题 hairline token，禁止调色板字面量） */
.mx-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 12px;
}
.mx-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.mx-dot-ok {
  background: var(--success);
}
.mx-dot-fail {
  background: var(--danger);
}

.lib-root {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 16px 20px;
  gap: 12px;
  overflow: auto;
}

.lib-toolbar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
}

/* 搜索框：统一 SearchInput，仅保留布局占位 */
.lib-search {
  flex: 1;
  max-width: 420px;
}

.lib-cats {
  flex: none;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.lib-cat {
  height: 24px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-family: inherit;
  cursor: pointer;
}

.lib-cat:hover {
  border-color: var(--border-strong);
  color: var(--text-1);
}

.lib-cat.active {
  background: var(--accent-tint);
  border-color: var(--accent);
  color: var(--accent);
}

.lib-list {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.lib-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-panel);
  cursor: pointer;
  min-width: 0;
  /* 长列表渲染优化：屏外行跳过布局/绘制（场景数无上限，列表平铺渲染）。
     行高估算用于滚动条占位；分组嵌套结构不适合虚拟滚动（见 useVirtualList）。 */
  content-visibility: auto;
  contain-intrinsic-size: auto 46px;
}

.lib-row:hover {
  border-color: var(--border-strong);
  background: var(--bg-card);
}

.lib-run-dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.5;
}

.lib-run-dot.ok {
  background: var(--success);
  opacity: 1;
}

.lib-run-dot.fail {
  background: var(--danger);
  opacity: 1;
}

.lib-run-dot.cancel {
  background: var(--warning);
  opacity: 1;
}

.lib-main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.lib-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  font-weight: 500;
}

.lib-row:hover .lib-name {
  color: var(--accent);
}

.lib-cat-tag {
  flex: none;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  background: var(--accent-tint);
  color: var(--accent);
}

.lib-meta {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono);
}

.lib-time {
  width: 150px;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lib-sort {
  flex: none;
  width: 130px;
}

.lib-status-filter {
  flex: none;
  width: 110px;
}

.lib-rate {
  flex: none;
  width: 44px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--success);
}

.lib-rate-bad {
  color: var(--danger);
}

.lib-env {
  flex: none;
  max-width: 110px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border);
  color: var(--text-2);
  font-family: var(--font-mono);
}

.lib-fail {
  color: var(--danger);
}

.lib-actions {
  flex: none;
  display: none;
  align-items: center;
  gap: 2px;
}

.lib-row:hover .lib-actions {
  display: inline-flex;
}

.lib-no-match {
  margin: 0;
  padding: 16px 8px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  text-align: center;
}

.lib-empty-actions {
  display: flex;
  gap: 8px;
}

.lib-empty-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
</style>
