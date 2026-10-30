<script setup lang="ts">
/**
 * TestCasesPanel：Apifox 风格测试用例管理面板。
 * - 顶部分类 Tab（全部 (N) / 正向 / 负向 / 边界值 / 安全性 / 其他），点击过滤；
 * - 右上角：+ 添加用例 / ▶ 全部运行；
 * - 数据表：# | 名称 | 分组 | 运行结果 | 操作（... 菜单：直接运行 / 编辑 / 克隆 / 删除）；
 * - 点击用例行 → 回填调试页（method / path / params / headers / body）并切回「调试」。
 */
import { computed, ref, watch } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { CATEGORY_TONE, TEST_CASE_CATEGORIES, caseCategoryLabel, formatDuration, matchTestCaseKeyword, statusTextOf, statusToneOf } from '../utils/testCases'
import { methodTextTone } from '../utils/methodTone'
import type { Endpoint, TestCase, TestCaseCategory } from '../types/miyoApi'
import EmptyState from './ui/EmptyState.vue'
import EnvBadge from './ui/EnvBadge.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import TestCaseDrawer from './TestCaseDrawer.vue'
import TestCaseModal from './TestCaseModal.vue'
import ExportSmokeDialog from './docs/ExportSmokeDialog.vue'
import { useToast } from '../composables/useToast'

const props = defineProps<{ draft: Endpoint | null }>()

const store = useWorkspaceStore()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

type FilterKey = '全部' | TestCaseCategory

const filter = ref<FilterKey>('全部')

/** 关键字搜索：名称 / 方法 / 路径 / Body 文本 / 参数与请求头键值（见 matchTestCaseKeyword）。 */
const keyword = ref('')

/** 当前接口的用例（后端按创建时间排序，克隆/追加在尾部）。 */
const cases = computed<TestCase[]>(() =>
  props.draft ? (store.testCases.get(props.draft.id) ?? []) : [],
)

const filtered = computed<TestCase[]>(() => {
  const byCat =
    filter.value === '全部' ? cases.value : cases.value.filter((c) => c.category === filter.value)
  const q = keyword.value.trim()
  if (!q) return byCat
  return byCat.filter((c) => matchTestCaseKeyword(c, q))
})

/** 空态标题：无用例 → 空提示；有搜索词无命中 → 无匹配；仅分类无命中 → 分组空提示。 */
const emptyTitle = computed(() => {
  if (!cases.value.length) return t('cases.empty')
  if (keyword.value.trim()) return t('cases.noMatchQ', { q: keyword.value.trim() })
  return filter.value === '全部'
    ? t('cases.empty')
    : t('cases.emptyInGroup', { v: caseCategoryLabel(filter.value) })
})

/** 空态描述：搜索无命中时给清空指引，其余走新建指引。 */
const emptyDescription = computed(() =>
  cases.value.length && keyword.value.trim() ? t('cases.noMatchHint') : t('cases.emptyHint'),
)

/** 分类计数（全部 (N) + 各分类）：单遍聚合（原来每分类 filter 一遍，O(5N)）。 */
const counts = computed<Record<FilterKey, number>>(() => {
  const all: Record<FilterKey, number> = {
    全部: cases.value.length,
    正向: 0,
    负向: 0,
    边界值: 0,
    安全性: 0,
    其他: 0,
  }
  for (const c of cases.value) {
    if (c.category in all) all[c.category as TestCaseCategory] += 1
    else all['其他'] += 1
  }
  return all
})
// ---------- 新建 / 编辑 ----------
const modalOpen = ref(false)
const editingCase = ref<TestCase | null>(null)

function openCreate(): void {
  editingCase.value = null
  modalOpen.value = true
}

async function onModalSubmit(payload: { name: string; category: TestCaseCategory }): Promise<void> {
  const d = props.draft
  if (!d) return
  if (editingCase.value) {
    const ok = await store.renameTestCase(d.id, editingCase.value.id, payload.name, payload.category)
    if (ok) toast.success(t('cases.updated'))
  } else {
    await store.saveTestCase(d.id, payload.name, payload.category, d.request, d.path, d.method)
  }
}

// ---------- 行操作 ----------
const menuEl = ref<InstanceType<typeof Menu> | null>(null)
const menuTarget = ref<TestCase | null>(null)

/**
 * 行选中态：点击行体高亮，用于在「全部」视图里定位当前正在看/操作的用例。
 * 只存 id（用例被删/切接口后自然失效）；切换接口（draft.id 变化）时清空。
 */
const selectedId = ref<string | null>(null)

function selectRow(c: TestCase): void {
  selectedId.value = c.id
}

function openRowMenu(event: MouseEvent, c: TestCase): void {
  selectRow(c)
  menuTarget.value = c
  const items: MenuItem[] = [
    { key: 'run', label: t('cases.menuRun'), icon: 'play' },
    { key: 'edit', label: t('cases.menuEdit'), icon: 'pencil' },
    { key: 'open-debug', label: t('cases.menuOpenDebug'), icon: 'layout-grid' },
    { key: 'clone', label: t('cases.menuClone'), icon: 'copy', dividerBefore: true },
    { key: 'delete', label: t('common.delete'), icon: 'trash', danger: true, confirm: t('cases.deleteConfirm', { name: c.name }) },
  ]
  menuEl.value?.openAt(event.currentTarget as HTMLElement, items, 'left')
}

async function onMenuSelect(item: MenuItem): Promise<void> {
  const c = menuTarget.value
  const d = props.draft
  if (!c || !d) return
  switch (item.key) {
    case 'run':
      await runOne(c)
      break
    case 'edit':
      openDrawer(c)
      break
    case 'open-debug':
      store.openTestCaseInDebug(d.id, c)
      break
    case 'clone':
      await store.cloneTestCase(d.id, c)
      break
  }
}

async function onMenuConfirm(item: MenuItem): Promise<void> {
  const c = menuTarget.value
  const d = props.draft
  if (!c || !d || item.key !== 'delete') return
  await store.removeTestCase(d.id, c.id)
}

// ---------- 导出冒烟文档 ----------
const exportSmokeOpen = ref(false)

// ---------- 详情 / 编辑抽屉 ----------
const drawerOpen = ref(false)
const drawerCase = ref<TestCase | null>(null)

function openDrawer(c: TestCase): void {
  drawerCase.value = c
  drawerOpen.value = true
}

/** 抽屉「立即运行」：原地执行（不切 Tab），结果留在抽屉内。 */
async function runDrawerPayload(payload: {
  method: string
  urlPath: string
  pathVariables: TestCase['path_variables']
  params: TestCase['params']
  headers: TestCase['headers']
  bodyType: string
  bodyContent: string
}): Promise<import('../types/miyoApi').ExecuteResponse | null> {
  const d = props.draft
  if (!d || !drawerCase.value) return null
  const snapshot: TestCase = {
    ...drawerCase.value,
    method: payload.method as TestCase['method'],
    url_path: payload.urlPath,
    path_variables: payload.pathVariables,
    params: payload.params,
    headers: payload.headers,
    body_type: payload.bodyType,
    body_content: payload.bodyContent,
  }
  return store.runTestCase(d.id, snapshot, store.activeEnvId)
}

/** 抽屉「保存修改」：名称/分组 + 请求内容一起落库。 */
async function saveDrawerPayload(payload: {
  name: string
  category: TestCaseCategory
  method: string
  urlPath: string
  pathVariables: TestCase['path_variables']
  params: TestCase['params']
  headers: TestCase['headers']
  bodyType: string
  bodyContent: string
}): Promise<void> {
  const d = props.draft
  if (!d || !drawerCase.value) return
  const c = drawerCase.value
  await store.updateTestCaseContent(d.id, c.id, {
    method: payload.method as TestCase['method'],
    urlPath: payload.urlPath,
    pathVariables: payload.pathVariables,
    params: payload.params,
    headers: payload.headers,
    bodyType: payload.bodyType,
    bodyContent: payload.bodyContent,
  })
  const ok = await store.renameTestCase(d.id, c.id, payload.name, payload.category)
  if (ok) toast.success(t('cases.saved'))
}

// ---------- 运行 ----------
const runningIds = ref<Set<string>>(new Set())
const runningAll = ref(false)

async function runOne(c: TestCase): Promise<void> {
  const d = props.draft
  if (!d || runningIds.value.has(c.id)) return
  runningIds.value = new Set(runningIds.value).add(c.id)
  try {
    const res = await store.runTestCase(d.id, c, store.activeEnvId)
    if (res) {
      const ok = res.status >= 200 && res.status < 400
      if (ok) {
        toast.success(t('cases.passed', { name: c.name }), { message: `HTTP ${res.status} · ${res.duration_ms}ms` })
      } else {
        toast.error(t('cases.failed', { name: c.name }), { message: `HTTP ${res.status} · ${res.duration_ms}ms` })
      }
    }
  } catch (err) {
    toast.error(t('cases.runFail', { name: c.name }), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    const next = new Set(runningIds.value)
    next.delete(c.id)
    runningIds.value = next
  }
}

/** 在途集合运行标识（取消按钮持有）。 */
const activeCollectionRunId = ref<string | null>(null)
/** 集合运行进度（done/total，已完成数由后端事件推进，此处取整）。 */
const collectionProgress = ref<{ done: number; total: number } | null>(null)

async function runAll(): Promise<void> {
  const d = props.draft
  if (!d || runningAll.value) return
  runningAll.value = true
  collectionProgress.value = { done: 0, total: store.testCases.get(d.id)?.length ?? 0 }
  const runId = crypto.randomUUID()
  activeCollectionRunId.value = runId
  try {
    const r = await store.runAllTestCases(d.id, {
      runId,
      onProgress: (done, total) => {
        collectionProgress.value = { done, total }
      },
    })
    if (r.cancelled) {
      toast.info(t('cases.runCancelled', { ok: r.success, total: r.total }))
    } else {
      toast[r.success === r.total ? 'success' : 'info'](
        t('cases.runAllDone', { ok: r.success, total: r.total }),
      )
    }
  } catch (err) {
    toast.error(t('cases.runAllFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    if (activeCollectionRunId.value === runId) activeCollectionRunId.value = null
    collectionProgress.value = null
    runningAll.value = false
  }
}

/** 取消在途集合运行：后端停止领新任务，已完成项保留。 */
function cancelRunAll(): void {
  if (!activeCollectionRunId.value) return
  void store.cancelAllTestCases(activeCollectionRunId.value)
  toast.info(t('cases.cancelling'))
}

const STATUS_TONE: Record<TestCase['last_run_status'], string> = {
  Success: 'var(--ok)',
  Failed: 'var(--danger)',
  Untested: 'var(--text-3)',
}

/** 最近一次运行元信息（状态码 / 耗时），运行结果列富化展示。 */
function runMetaOf(c: TestCase): { status: number; durationMs: number } | undefined {
  return store.caseRunMeta.get(c.id)
}

watch(
  () => props.draft?.id,
  () => {
    filter.value = '全部'
    keyword.value = ''
    selectedId.value = null
  },
)
</script>

<template>
  <div class="tcp">
    <!-- 顶部：分类筛选 + 操作区 -->
    <div class="tcp-bar">
      <div class="tcp-filters">
        <button
          v-for="key in (['全部', ...TEST_CASE_CATEGORIES] as FilterKey[])"
          :key="key"
          class="tcp-filter"
          :class="{ active: filter === key }"
          type="button"
          @click="filter = key"
        >
          {{ key === '全部' ? t('cases.catAll') : caseCategoryLabel(key) }}
          <span class="tcp-count">{{ counts[key] }}</span>
        </button>
      </div>
      <div class="tcp-search">
        <Icon name="search" :size="12" class="tcp-search-icon" />
        <input
          v-model="keyword"
          class="tcp-search-input"
          type="text"
          :placeholder="t('cases.searchPh')"
          spellcheck="false"
        />
        <button
          v-if="keyword"
          class="tcp-search-clear"
          type="button"
          :title="t('cases.clearSearch')"
          :aria-label="t('cases.clearSearch')"
          @click="keyword = ''"
        >
          <Icon name="x" :size="12" />
        </button>
      </div>
      <div class="tcp-actions">
        <!-- 运行环境上下文：全部运行 / 行内运行均走 store.activeEnvId，此处同源可见 -->
        <EnvBadge />
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="!cases.length"
          :title="t('cases.exportSmokeHint')"
          @click="exportSmokeOpen = true"
        >
          <Icon name="download" :size="13" /> {{ t('cases.exportSmoke') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="openCreate">
          <Icon name="plus" :size="13" /> {{ t('cases.addCase') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!cases.length || runningAll"
          @click="runAll"
        >
          <Icon name="play" :size="13" />
          {{
            runningAll && collectionProgress
              ? t('cases.runningProgress', { done: collectionProgress.done, total: collectionProgress.total })
              : runningAll
                ? t('cases.running')
                : t('cases.runAll')
          }}
        </button>
        <button
          v-if="runningAll && activeCollectionRunId"
          class="miyo-btn miyo-btn-sm miyo-btn-danger"
          type="button"
          @click="cancelRunAll"
        >
          <Icon name="x" :size="13" /> {{ t('common.cancel') }}
        </button>
      </div>
    </div>

    <!-- 用例表格 -->
    <div v-if="filtered.length" class="tcp-table">
      <div class="tcp-row tcp-head">
        <span class="tcp-col-idx">#</span>
        <span class="tcp-col-name">{{ t('cases.colName') }}</span>
        <span class="tcp-col-cat">{{ t('cases.colCategory') }}</span>
        <span class="tcp-col-status">{{ t('cases.colResult') }}</span>
        <span class="tcp-col-ops">{{ t('cases.colOps') }}</span>
      </div>
      <div
        v-for="(c, i) in filtered"
        :key="c.id"
        class="tcp-row tcp-body-row"
        :class="{ 'is-selected': selectedId === c.id }"
        @click="selectRow(c)"
      >
        <span class="tcp-col-idx">{{ i + 1 }}</span>
        <button class="tcp-col-name tcp-name-btn" type="button" :title="t('cases.rowTitle', { name: c.name })" @click="openDrawer(c)">
          <span class="tcp-method" :class="methodTextTone(c.method)">{{ c.method }}</span>
          <span class="tcp-name-text">{{ c.name }}</span>
        </button>
        <span class="tcp-col-cat">
          <span class="tcp-cat-dot" :style="{ background: CATEGORY_TONE[c.category] }"></span>
          {{ caseCategoryLabel(c.category) }}
        </span>
        <span class="tcp-col-status">
          <span v-if="runningIds.has(c.id)" class="tcp-status tcp-status-running">
            <span class="tcp-spinner"></span>
            {{ t('cases.running') }}
          </span>
          <template v-else-if="runMetaOf(c)">
            <span
              class="tcp-status tcp-status-meta"
              :class="`tone-${statusToneOf(runMetaOf(c)!.status)}`"
            >
              <span class="tcp-dot"></span>
              {{ runMetaOf(c)!.status }} {{ statusTextOf(runMetaOf(c)!.status) }}
              <span class="tcp-dur">({{ formatDuration(runMetaOf(c)!.durationMs) }})</span>
            </span>
          </template>
          <span
            v-else
            class="tcp-status"
            :class="`tcp-status-${c.last_run_status.toLowerCase()}`"
            :style="{ color: STATUS_TONE[c.last_run_status] }"
          >
            {{ c.last_run_status === 'Untested' ? '—' : c.last_run_status }}
          </span>
        </span>
        <span class="tcp-col-ops">
          <IconButton
            name="more-horizontal"
            :size="14"
            :title="t('common.moreActions')"
            @click="openRowMenu($event, c)"
          />
        </span>
      </div>
    </div>
    <EmptyState
      v-else
      icon="list"
      compact
      :title="emptyTitle"
      :description="emptyDescription"
    />

    <TestCaseModal
      :open="modalOpen"
      :title="editingCase ? t('cases.editTitle') : t('testcase.title')"
      :name="editingCase?.name ?? ''"
      :category="editingCase?.category ?? '正向'"
      @update:open="modalOpen = $event"
      @submit="onModalSubmit"
    />

    <TestCaseDrawer
      :open="drawerOpen"
      :endpoint-id="draft?.id ?? ''"
      :test-case="drawerCase"
      :resolve-url="store.resolveTestCaseUrl"
      :on-run="runDrawerPayload"
      :on-save="saveDrawerPayload"
      @update:open="drawerOpen = $event"
    />

    <ExportSmokeDialog
      v-if="exportSmokeOpen"
      :draft="draft"
      @close="exportSmokeOpen = false"
    />

    <Menu ref="menuEl" @select="onMenuSelect" @confirm="onMenuConfirm" />
  </div>
</template>

<style scoped>
.tcp {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.tcp-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.tcp-filters {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
}

.tcp-filter {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: none;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-2);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.tcp-filter:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.tcp-filter.active {
  color: var(--accent);
  border-color: var(--accent);
  background: var(--accent-tint, rgba(168, 85, 247, 0.12));
}

.tcp-count {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.tcp-filter.active .tcp-count {
  color: var(--accent);
}

.tcp-search {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 140px;
  max-width: 280px;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-1);
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.tcp-search:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint, rgba(168, 85, 247, 0.18));
}
.tcp-search-icon {
  flex-shrink: 0;
  color: var(--text-3);
}
.tcp-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  padding: 0;
  font-family: inherit;
  font-size: 12px;
  color: var(--text-1);
}
.tcp-search-input::placeholder {
  color: var(--text-3);
}
.tcp-search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: none;
  color: var(--text-3);
  cursor: pointer;
}
.tcp-search-clear:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.tcp-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tcp-table {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.tcp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border: none;
  background: none;
  font-family: inherit;
  text-align: left;
}

.tcp-head {
  height: 30px;
  background: var(--bg-2);
  font-size: var(--fs-xs);
  color: var(--text-3);
  /* 表头不随内容滚动折断：固定列宽与数据行同源，避免「名称」被挤掉 */
  white-space: nowrap;
}

.tcp-body-row {
  min-height: 38px;
  border-top: 1px solid var(--border);
  transition: background var(--dur) var(--ease);
}
.tcp-body-row:hover {
  background: var(--bg-hover);
}

/* 选中行：accent 淡底 + 左侧色条 + 加粗，浅色下也能一眼分辨（不再只有 hover） */
.tcp-body-row.is-selected {
  position: relative;
  background: var(--accent-tint);
}
.tcp-body-row.is-selected::before {
  content: '';
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: var(--accent);
}

.tcp-col-idx {
  width: 34px;
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.tcp-col-name {
  flex: 1 1 auto;
  /* 名称列是唯一弹性列：给下限，否则窄容器下会被固定列挤成 0 宽（文本竖排/重叠） */
  min-width: 120px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.tcp-name-btn {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  border: none;
  background: none;
  padding: 0;
  font-family: inherit;
  cursor: pointer;
  text-align: left;
  transition: opacity var(--dur) var(--ease);
}
.tcp-name-btn:hover {
  opacity: 0.8;
}

.tcp-method {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  padding: 1px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, currentColor 22%, transparent);
  background: color-mix(in srgb, currentColor 12%, transparent);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  letter-spacing: 0.3px;
}

.tcp-name-text {
  min-width: 0;
  font-size: 12.5px;
  color: var(--text-1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tcp-col-cat {
  width: 96px;
  flex-shrink: 1;
  min-width: 64px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-2);
}

.tcp-cat-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.tcp-col-status {
  width: 148px;
  flex-shrink: 1;
  min-width: 96px;
  font-size: 12px;
  font-family: var(--font-mono);
}

.tcp-status-meta {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
}
.tcp-status-meta.tone-ok {
  color: var(--ok);
}
.tcp-status-meta.tone-warn {
  color: var(--warning);
}
.tcp-status-meta.tone-err {
  color: var(--danger);
}
.tcp-status-meta.tone-info {
  color: var(--miyo-info, #38bdf8);
}

.tcp-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
  box-shadow: 0 0 6px currentColor;
}

.tcp-dur {
  color: var(--text-3);
}

.tcp-col-ops {
  width: 32px;
  flex-shrink: 0;
  display: flex;
  justify-content: flex-end;
}

.tcp-spinner {
  width: 11px;
  height: 11px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: tcp-spin 0.7s linear infinite;
}

@keyframes tcp-spin {
  to {
    transform: rotate(360deg);
  }
}

.tcp-status-running {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
</style>