<script setup lang="ts">
/**
 * ScenarioPanel：侧边栏「场景」页签——项目级集成链路管理区（三栏工作区左栏）。
 *
 * 行为：
 * - 点击行打开场景编排页（scenario:{id} 伪页签）；选中行有明确高亮态；
 * - 搜索过滤（按场景名，输入即时过滤）；
 * - + 新建（默认名落库后打开）；行内复制（步骤 / 脚本深拷贝为新场景）；
 * - 行内删除（确认后连编排页签一起关闭）；
 * - 每行展示步骤数与最近一次执行状态点（绿 / 红 / 黄；无运行为灰），
 *   状态取自运行历史（listScenarioRuns 每场景取最新 1 条，面板级缓存）。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import Popconfirm from './ui/Popconfirm.vue'
import type { Scenario, ScenarioRunSummary, ScenarioStep } from '../types/miyoApi'

const store = useWorkspaceStore()
const t = useLocaleStore().t
const api = useMiyoApi()
const toast = useToast()

async function onCreate(): Promise<void> {
  const created = await store.createScenario()
  if (created) store.openScenario(created.id)
}

// ---------- 搜索过滤 ----------
const search = ref('')

const filteredScenarios = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return store.scenarios
  return store.scenarios.filter((s) => s.name.toLowerCase().includes(q))
})

// ---------- 最近执行状态（面板级缓存；场景增删后刷新） ----------
const lastRuns = ref(new Map<string, ScenarioRunSummary>())

async function refreshLastRuns(): Promise<void> {
  const ids = store.scenarios.map((s) => s.id)
  const entries = await Promise.all(
    ids.map(async (id): Promise<readonly [string, ScenarioRunSummary | null]> => {
      try {
        const runs = await api.listScenarioRuns(id, 1)
        return [id, runs[0] ?? null] as const
      } catch {
        return [id, null] as const
      }
    }),
  )
  const m = new Map<string, ScenarioRunSummary>()
  for (const [id, run] of entries) {
    if (run) m.set(id, run)
  }
  lastRuns.value = m
}

watch(
  () => store.scenarios.map((s) => s.id).join(','),
  () => {
    void refreshLastRuns()
  },
  { immediate: true },
)

/** 最近一次执行的状态点：取消 = 黄；有失败 = 红；全过 = 绿；无运行 = null（灰点）。 */
function runTone(id: string): 'ok' | 'fail' | 'cancel' | null {
  const r = lastRuns.value.get(id)
  if (!r) return null
  if (r.cancelled) return 'cancel'
  return r.fail_count > 0 ? 'fail' : 'ok'
}

function runToneTitle(id: string): string {
  const tone = runTone(id)
  if (tone === 'ok') return t('scenario.lastRunOk')
  if (tone === 'fail') return t('scenario.lastRunFail')
  if (tone === 'cancel') return t('scenario.lastRunCancelled')
  return t('scenario.lastRunNone')
}

// ---------- 复制场景（步骤 / 脚本深拷贝，落库为新场景后打开） ----------
async function onDuplicate(s: Scenario): Promise<void> {
  const now = new Date().toISOString()
  const copy: Scenario = {
    ...s,
    id: crypto.randomUUID(),
    name: t('scenario.copyName', { name: s.name }),
    sort_order: store.scenarios.length,
    steps: JSON.parse(JSON.stringify(s.steps)) as ScenarioStep[],
    script: s.script ?? null,
    created_at: now,
    updated_at: now,
  }
  const saved = await store.upsertScenario(copy)
  if (saved) {
    toast.success(t('scenario.duplicated', { name: saved.name }))
    store.openScenario(saved.id)
  }
}
</script>

<template>
  <div class="scenario-panel">
    <div class="sp-toolbar">
      <div class="sp-search">
        <Icon name="search" :size="12" class="sp-search-icon" />
        <input
          v-model="search"
          class="sp-search-input"
          type="text"
          :placeholder="t('scenario.panelSearchPh')"
          spellcheck="false"
        />
        <button
          v-if="search"
          class="sp-search-clear"
          type="button"
          :aria-label="t('common.clearSearch')"
          @click="search = ''"
        >
          <Icon name="x" :size="11" />
        </button>
      </div>
      <button
        class="tool-add"
        type="button"
        :aria-label="t('scenario.new')"
        :title="t('scenario.new')"
        @click="onCreate"
      >
        <Icon name="plus" :size="14" />
      </button>
    </div>

    <div v-if="store.scenarios.length" class="sp-list">
      <div
        v-for="s in filteredScenarios"
        :key="s.id"
        class="sp-row"
        role="button"
        tabindex="0"
        :class="{ active: store.activeTabId === `scenario:${s.id}` }"
        @click="store.openScenario(s.id)"
        @keydown.enter="store.openScenario(s.id)"
      >
        <span class="sp-run-dot" :class="runTone(s.id)" :title="runToneTitle(s.id)" aria-hidden="true"></span>
        <span class="sp-name" :title="s.name">{{ s.name }}</span>
        <span class="sp-steps">{{ t('scenario.stepCount', { n: s.steps.length }) }}</span>
        <span class="sp-actions">
          <button
            class="sp-act"
            type="button"
            :aria-label="t('scenario.duplicate')"
            :title="t('scenario.duplicate')"
            @click.stop="onDuplicate(s)"
          >
            <Icon name="copy" :size="12" />
          </button>
          <Popconfirm
            :title="t('scenario.confirmDelete', { name: s.name })"
            :confirm-text="t('common.delete')"
            @confirm="store.removeScenario(s.id)"
          >
            <button
              class="sp-act"
              type="button"
              :aria-label="t('scenario.delete')"
              :title="t('scenario.delete')"
              @click.stop
            >
              <Icon name="trash" :size="12" />
            </button>
          </Popconfirm>
        </span>
      </div>
      <p v-if="!filteredScenarios.length" class="sp-no-match">{{ t('common.noResults') }}</p>
    </div>

    <EmptyState
      v-else
      icon="beaker"
      :title="t('scenario.emptyTitle')"
      :description="t('scenario.emptyDesc')"
      compact
    >
      <button class="miyo-btn miyo-btn-sm" type="button" @click="onCreate">
        <Icon name="plus" :size="13" /> {{ t('scenario.emptyAction') }}
      </button>
    </EmptyState>
  </div>
</template>

<style scoped>
.scenario-panel {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

/* ---- 工具栏：搜索 + 新建 ---- */
.sp-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px 8px;
}

.sp-search {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 5px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--sidebar-border, var(--border));
  border-radius: var(--radius-md);
  background: var(--sidebar-search-bg, var(--bg-code));
}

.sp-search:focus-within {
  border-color: var(--accent);
}

.sp-search-icon {
  flex: none;
  color: var(--text-3);
  opacity: 0.7;
}

.sp-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.sp-search-input::placeholder {
  color: var(--text-3);
}

.sp-search-clear {
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

.sp-search-clear:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.sp-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 0 6px 8px;
}

.sp-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-1);
  min-width: 0;
  border: 1px solid transparent;
}

.sp-row:hover {
  background: var(--bg-hover);
}

/* 选中态：背景 + 边框明确标识（打开中的场景） */
.sp-row.active {
  background: var(--bg-active, var(--bg-hover));
  border-color: var(--border-strong);
}

/* ---- 最近执行状态点 ---- */
.sp-run-dot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-3);
  opacity: 0.5;
}

.sp-run-dot.ok {
  background: var(--success);
  opacity: 1;
}

.sp-run-dot.fail {
  background: var(--danger);
  opacity: 1;
}

.sp-run-dot.cancel {
  background: var(--warning);
  opacity: 1;
}

.sp-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
}

.sp-steps {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

/* ---- 行内操作（复制 / 删除，hover 显示） ---- */
.sp-actions {
  flex: none;
  display: none;
  align-items: center;
  gap: 2px;
}

.sp-row:hover .sp-actions {
  display: inline-flex;
}

.sp-act {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: var(--radius-sm);
}

.sp-act:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.sp-row:hover .sp-act[aria-label]:hover {
  color: var(--text-1);
}

.sp-no-match {
  margin: 0;
  padding: 10px 8px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
