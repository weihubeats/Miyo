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
import { computed, ref } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useLocaleStore } from '../../stores/locale'
import { useWorkspaceStore } from '../../stores/workspace'
import { useIntegrationStore } from '../../stores/integration'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { buildScenarioExport, parseScenarioImport } from '../../utils/scenarioIO'
import { formatDateTime } from '../../utils/dateTime'
import CustomSelect from '../ui/CustomSelect.vue'
import EmptyState from '../ui/EmptyState.vue'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import Popconfirm from '../ui/Popconfirm.vue'
import { formatDurationShort } from '../../utils/format'
import type { Scenario, ScenarioRunSummary, ScenarioStep } from '../../types/miyoApi'

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
      message: err instanceof Error ? err.message : String(err),
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
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function onDelete(id: string): Promise<void> {
  try {
    await api.deleteScenario(id)
    store.forgetScenario(id)
  } catch (err) {
    toast.error(t('integration.saveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

// ---------- 导出 / 导入 ----------
async function onExport(s: Scenario): Promise<void> {
  try {
    const dir = await openDialog({ directory: true, title: t('integration.export') })
    if (!dir || Array.isArray(dir)) return
    const base = s.name.replace(/[\\/:*?"<>|\s]+/g, '_') || 'scenario'
    const path = await join(dir, `${base}.miyo-scenario.json`)
    const json = buildScenarioExport(s, (id) => workspace.endpoints.find((e) => e.id === id))
    await api.writeTextFile(path, json)
    toast.success(t('integration.exported', { path }))
  } catch (err) {
    toast.error(t('integration.exportFail'), {
      message: err instanceof Error ? err.message : String(err),
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
    const raw = err instanceof Error ? err.message : String(err)
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
          <IconButton name="play" :size="13" :title="t('integration.run')" @click.stop="store.openScenario(s.id)" />
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
