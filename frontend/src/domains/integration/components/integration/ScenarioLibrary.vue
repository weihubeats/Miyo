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
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import { buildScenarioExport, parseScenarioImport } from '@/utils/scenarioIO'
import { formatDateTime } from '@/utils/dateTime'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Modal from '@/components/ui/Modal.vue'
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

// ---------- 搜索 + 分类过滤 + 排序 ----------
const search = ref('')
const categoryFilter = ref<string | null>(null)
/** 排序：默认（编排序）| 名称 | 最近执行时间倒序。 */
const sortBy = ref<'default' | 'name' | 'recent'>('default')

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

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  const list = store.libraryItems.filter((s) => {
    if (categoryFilter.value === '' && s.category.trim()) return false
    if (categoryFilter.value && s.category.trim() !== categoryFilter.value) return false
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

function runTone(item: (typeof store.libraryItems)[number]): RunTone {
  const r = item.last_run
  if (!r) return null
  if (r.cancelled) return 'cancel'
  return r.fail_count > 0 ? 'fail' : 'ok'
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
let unlistenMatrix: UnlistenFn | null = null

onMounted(async () => {
  unlistenMatrix = await tauriListen<{
    groupId: string
    index: number
    total: number
    label: string
    ok: boolean
    runId: string
  }>('miyo:matrix-progress', (e) => {
    const p = e.payload
    matrixProgress.value = {
      groupId: p.groupId,
      done: p.index,
      total: p.total,
      label: p.label,
      ok: p.ok,
    }
  })
})
onUnmounted(() => {
  unlistenMatrix?.()
  unlistenMatrix = null
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
    const path = await join(dir, `${base}.miyo-scenario.json`)
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
  <div class="lib-root">
    <div class="lib-toolbar">
      <div class="lib-search">
        <Icon name="search" :size="12" class="lib-search-icon" />
        <input
          v-model="search"
          class="lib-search-input"
          type="text"
          :placeholder="t('integration.libSearchPh')"
          spellcheck="false"
        />
        <button v-if="search" class="lib-search-clear" type="button" :aria-label="t('common.clearSearch')" @click="search = ''">
          <Icon name="x" :size="11" />
        </button>
      </div>
      <IconButton name="upload" :size="14" :title="t('integration.libImport')" @click="onImport" />
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
        <span v-if="s.last_run" class="lib-meta num" :title="runSummaryText(s.last_run)">
          <span :class="{ 'lib-fail': s.last_run.fail_count > 0 }">✓{{ s.last_run.ok_count }}</span>
          <span v-if="s.last_run.fail_count > 0" class="lib-fail"> ✗{{ s.last_run.fail_count }}</span>
          · {{ formatDurationShort(s.last_run.duration_ms ?? 0) }}
        </span>
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
            class="num text-xxs font-medium text-purple-600 dark:text-purple-400"
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
        <button class="miyo-btn miyo-btn-sm" type="button" @click="onImport">
          <Icon name="upload" :size="13" /> {{ t('integration.libImport') }}
        </button>
      </div>
    </EmptyState>
  </div>
<!-- 矩阵结果：逐行对比（行点击 → 打开该行运行详情） -->
<Modal v-model:open="matrixResultOpen" :title="t('integration.matrixResultTitle')" width="520px">
  <div class="flex flex-col gap-1.5">
    <div
      v-for="(row, i) in matrixResult?.rows ?? []"
      :key="row.key"
      class="flex items-center justify-between gap-3 rounded-lg border border-zinc-200/70 px-3 py-2 dark:border-white/[0.06]"
    >
      <button
        type="button"
        class="min-w-0 flex-1 text-left"
        @click="openMatrixRow(i)"
      >
        <div class="flex items-center gap-2">
          <span
            class="inline-block h-2 w-2 rounded-full"
            :class="row.ok ? 'bg-emerald-500' : 'bg-red-500'"
            aria-hidden="true"
          ></span>
          <span class="text-xs font-medium text-zinc-900 dark:text-zinc-100">{{ row.label }}</span>
        </div>
        <p class="mt-0.5 text-xxs text-zinc-500 dark:text-zinc-400">
          {{ t('scenario.assertPassed', { n: row.okCount, m: row.okCount + row.failCount + row.skippedCount }) }}
          <template v-if="row.durationMs != null"> · {{ Math.round(row.durationMs) }}ms</template>
        </p>
      </button>
      <code class="font-mono text-xxs text-zinc-400">{{ row.runId.slice(0, 8) }}</code>
    </div>
  </div>
</Modal>
</template>

<style scoped>
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

.lib-search {
  flex: 1;
  max-width: 420px;
  display: flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}

.lib-search:focus-within {
  border-color: var(--accent);
}

.lib-search-icon {
  flex: none;
  color: var(--text-3);
  opacity: 0.7;
}

.lib-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.lib-search-input::placeholder {
  color: var(--text-3);
}

.lib-search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-3);
  cursor: pointer;
  padding: 0;
}

.lib-search-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
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
  border-radius: 999px;
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
  font-size: 10px;
  padding: 1px 8px;
  border-radius: 999px;
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
</style>
