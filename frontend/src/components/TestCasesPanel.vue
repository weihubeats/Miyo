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

// ---------- 文件夹分组 ----------
/** 分组渲染模型：命名文件夹按组内最早创建时间排序（发现序），未分组殿后。 */
const groups = computed<{ name: string; cases: TestCase[]; start: number }[]>(() => {
  const named = new Map<string, TestCase[]>()
  const ungrouped: TestCase[] = []
  for (const c of filtered.value) {
    const f = c.folder ?? ''
    if (f) {
      const arr = named.get(f)
      if (arr) arr.push(c)
      else named.set(f, [c])
    } else {
      ungrouped.push(c)
    }
  }
  const namedArr = [...named.entries()].map(([name, cs]) => ({ name, cases: cs, start: 0 }))
  namedArr.sort((a, b) => {
    const ta = Math.min(...a.cases.map((c) => Number(new Date(c.created_at))))
    const tb = Math.min(...b.cases.map((c) => Number(new Date(c.created_at))))
    return ta - tb || a.name.localeCompare(b.name)
  })
  // 新建尚未有用例的分组（组件态）排在实组之后、未分组之前
  for (const name of pendingFolders.value) {
    if (!namedArr.some((g) => g.name === name)) namedArr.push({ name, cases: [], start: 0 })
  }
  let total = 0
  for (const g of namedArr) {
    g.start = total
    total += g.cases.length
  }
  if (ungrouped.length) namedArr.push({ name: '', cases: ungrouped, start: total })
  return namedArr
})

/** 折叠态（按文件夹名；切接口清空）。 */
const collapsedFolders = ref<Set<string>>(new Set())

function toggleGroup(name: string): void {
  const next = new Set(collapsedFolders.value)
  if (next.has(name)) next.delete(name)
  else next.add(name)
  collapsedFolders.value = next
}

/** 现有文件夹清单（新建 / 编辑弹窗的 datalist 建议）。 */
const folderNames = computed<string[]>(() => {
  const out: string[] = []
  for (const c of cases.value) {
    const f = c.folder ?? ''
    if (f && !out.includes(f)) out.push(f)
  }
  for (const f of pendingFolders.value) {
    if (!out.includes(f)) out.push(f)
  }
  return out
})

// 分组重命名（行内编辑）
const renamingFolder = ref<string | null>(null)
const renameText = ref('')

/** 进入行内重命名；name 允许空串（未分组）。 */
function startFolderRename(name: string): void {
  renamingFolder.value = name
  renameText.value = name
}

async function commitFolderRename(): Promise<void> {
  // renamingFolder 为 null = 未在编辑；空串是合法键（未分组）
  if (renamingFolder.value === null || !props.draft) return
  const oldName = renamingFolder.value
  const next = renameText.value.trim()
  renamingFolder.value = null
  if (!next || next === oldName) return
  if (oldName === '') {
    // 未分组 → 命名：整批未分组用例归入新名
    const ids = (store.testCases.get(props.draft.id) ?? [])
      .filter((c) => !(c.folder ?? ''))
      .map((c) => c.id)
    if (ids.length && (await store.setTestCaseFolder(props.draft.id, ids, next))) {
      toast.success(t('cases.folderRenamed'))
    }
    return
  }
  // 组件态空分组：直接改 Set，不走 IPC
  if (pendingFolders.value.has(oldName) && !groups.value.some((g) => g.name === oldName && g.cases.length)) {
    const nextSet = new Set(pendingFolders.value)
    nextSet.delete(oldName)
    nextSet.add(next)
    pendingFolders.value = nextSet
    return
  }
  const ok = await store.renameTestCaseFolder(props.draft.id, oldName, next)
  if (ok) {
    collapsedFolders.value = new Set(
      [...collapsedFolders.value].map((n) => (n === oldName ? next : n)),
    )
  }
}

// 分组解散（组内用例移至未分组）
const groupMenuEl = ref<InstanceType<typeof Menu> | null>(null)
const groupMenuTarget = ref<{ name: string; cases: TestCase[] } | null>(null)

function openGroupMenu(event: MouseEvent, g: { name: string; cases: TestCase[] }): void {
  groupMenuTarget.value = { name: g.name, cases: g.cases }
  const items: MenuItem[] = [
    { key: 'add-case', label: t('cases.addCase'), icon: 'plus' },
    // 未分组的「重命名」= 命名该分组（整批用例归入新名）
    { key: 'rename', label: t('cases.renameFolder'), icon: 'pencil', dividerBefore: true },
  ]
  if (g.name) {
    items.push({
      key: 'dissolve',
      label: t('cases.dissolveFolder'),
      icon: 'minus',
      danger: true,
      confirm: t('cases.dissolveConfirm', { name: g.name, n: g.cases.length }),
    })
  }
  groupMenuEl.value?.openAt(event.currentTarget as HTMLElement, items, 'left')
}

async function onGroupMenuSelect(item: MenuItem): Promise<void> {
  const g = groupMenuTarget.value
  const d = props.draft
  if (!g || !d) return
  if (item.key === 'rename') startFolderRename(g.name)
  if (item.key === 'add-case') openCreate(g.name)
  if (item.key === 'dissolve') {
    if (pendingFolders.value.has(g.name) && !g.cases.length) {
      const next = new Set(pendingFolders.value)
      next.delete(g.name)
      pendingFolders.value = next
    } else if (await store.dissolveTestCaseFolder(d.id, g.name)) {
      pendingFolders.value = new Set([...pendingFolders.value].filter((n) => n !== g.name))
      toast.success(t('cases.folderDissolved'))
    }
  }
}

async function onGroupMenuConfirm(item: MenuItem): Promise<void> {
  const g = groupMenuTarget.value
  const d = props.draft
  if (!g || !d || item.key !== 'dissolve') return
  if (pendingFolders.value.has(g.name) && !g.cases.length) {
    const next = new Set(pendingFolders.value)
    next.delete(g.name)
    pendingFolders.value = next
  } else if (await store.dissolveTestCaseFolder(d.id, g.name)) {
    pendingFolders.value = new Set([...pendingFolders.value].filter((n) => n !== g.name))
    toast.success(t('cases.folderDissolved'))
  }
}

// 分组运行（复用集合运行链路，folder 过滤；可独立取消）
const groupRun = ref<{ name: string; runId: string; done: number; total: number } | null>(null)

async function runGroup(name: string): Promise<void> {
  const d = props.draft
  if (!d || runningAll.value || groupRun.value) return
  groupRun.value = { name, runId: crypto.randomUUID(), done: 0, total: 0 }
  try {
    const r = await store.runAllTestCases(d.id, {
      runId: groupRun.value.runId,
      folder: name,
      onProgress: (done, total) => {
        if (groupRun.value?.name === name) groupRun.value = { ...groupRun.value, done, total }
      },
    })
    toast[r.success === r.total ? 'success' : 'info'](
      t('cases.runAllDone', { ok: r.success, total: r.total }),
    )
  } catch (err) {
    toast.error(t('cases.runAllFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    if (groupRun.value?.name === name) groupRun.value = null
  }
}

function cancelGroupRun(): void {
  if (!groupRun.value) return
  void store.cancelAllTestCases(groupRun.value.runId)
  toast.info(t('cases.cancelling'))
}

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
/** 新建弹窗预填的文件夹（分组头「+」/ 工具栏新建文件夹带入）。 */
const modalFolder = ref('')

function openCreate(folder = ''): void {
  editingCase.value = null
  modalFolder.value = folder
  modalOpen.value = true
}

// ---------- 新建文件夹 ----------
/** 行内新建：Enter 即建（空分组先挂在组件态，加入用例后由用例数据承载）。 */
const creatingFolder = ref(false)
const newFolderName = ref('')

function startCreateFolder(): void {
  creatingFolder.value = true
  newFolderName.value = ''
}

function cancelCreateFolder(): void {
  creatingFolder.value = false
  newFolderName.value = ''
}

/** 焦点移出新建行（点击行外区域）才自动提交；行内按钮点击不触发。 */
function onCreateRowFocusout(event: FocusEvent): void {
  const row = event.currentTarget as HTMLElement
  if (row.contains(event.relatedTarget as Node)) return
  commitCreateFolder()
}

function commitCreateFolder(): void {
  const name = newFolderName.value.trim()
  creatingFolder.value = false
  if (!name) return
  // 重名 → 直接展开该组并弹「添加用例」（预填此分组）
  openCreate(name)
  if (!folderNames.value.includes(name) && !pendingFolders.value.has(name)) {
    pendingFolders.value = new Set(pendingFolders.value).add(name)
  }
}

/** 尚无用例支撑的新建分组（组件态；加入用例后转由用例数据承载）。 */
const pendingFolders = ref<Set<string>>(new Set())

async function onModalSubmit(payload: {
  name: string
  category: TestCaseCategory
  folder: string
}): Promise<void> {
  const d = props.draft
  if (!d) return
  if (editingCase.value) {
    const ok = await store.renameTestCase(
      d.id,
      editingCase.value.id,
      payload.name,
      payload.category,
      payload.folder,
    )
    if (ok) toast.success(t('cases.updated'))
  } else {
    await store.saveTestCase(
      d.id,
      payload.name,
      payload.category,
      d.request,
      d.path,
      d.method,
      payload.folder,
    )
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
  const ok = await store.renameTestCase(d.id, c.id, payload.name, payload.category, c.folder ?? '')
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
    collapsedFolders.value = new Set()
    renamingFolder.value = null
    groupRun.value = null
    pendingFolders.value = new Set()
    creatingFolder.value = false
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
        <button class="miyo-btn miyo-btn-sm" type="button" @click="startCreateFolder">
          <Icon name="folder-plus" :size="13" /> {{ t('cases.newFolder') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="openCreate()">
          <Icon name="plus" :size="13" /> {{ t('cases.addCase') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!cases.length || runningAll || !!groupRun"
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

    <!-- 行内新建文件夹：Enter / ✓ 确认，Esc / ✕ 取消（空态同样可用） -->
    <div v-if="creatingFolder" class="tcp-create-row" @focusout="onCreateRowFocusout">
      <Icon name="folder-plus" :size="13" class="tcp-create-icon" />
      <input
        v-model="newFolderName"
        v-focus-end
        class="tcp-create-input"
        type="text"
        :placeholder="t('cases.newFolderPh')"
        spellcheck="false"
        @keyup.enter="commitCreateFolder"
        @keydown.esc.prevent.stop="cancelCreateFolder"
      />
      <IconButton
        name="check"
        :size="14"
        class="tcp-create-confirm"
        :title="t('common.confirm')"
        @click="commitCreateFolder"
      />
      <IconButton
        name="x"
        :size="14"
        :title="t('common.cancel')"
        @click="cancelCreateFolder"
      />
    </div>

    <!-- 用例表格（按文件夹分组：命名组按发现序，未分组殿后；可折叠） -->
    <div v-if="filtered.length || pendingFolders.size" class="tcp-table">
      <div class="tcp-row tcp-head">
        <span class="tcp-col-idx">#</span>
        <span class="tcp-col-name">{{ t('cases.colName') }}</span>
        <span class="tcp-col-cat">{{ t('cases.colCategory') }}</span>
        <span class="tcp-col-status">{{ t('cases.colResult') }}</span>
        <span class="tcp-col-ops">{{ t('cases.colOps') }}</span>
      </div>
      <template v-for="g in groups" :key="g.name || '__ungrouped__'">
        <!-- 分组头：折叠 + 名称（行内重命名）+ 计数 + 分组运行 + 操作菜单 -->
        <div class="tcp-group-head" :class="{ 'is-collapsed': collapsedFolders.has(g.name) }">
          <button
            class="tcp-group-toggle"
            type="button"
            :aria-expanded="!collapsedFolders.has(g.name)"
            @click="toggleGroup(g.name)"
          >
            <Icon
              name="chevron-right"
              :size="13"
              class="tcp-group-chevron"
              :class="{ open: !collapsedFolders.has(g.name) }"
            />
            <Icon :name="g.name ? 'folder-open' : 'list'" :size="13" class="tcp-group-icon" />
            <input
              v-if="renamingFolder === g.name"
              v-model="renameText"
              v-focus-end
              class="tcp-group-rename"
              type="text"
              :placeholder="g.name ? undefined : t('cases.newFolderPh')"
              spellcheck="false"
              @keyup.enter="commitFolderRename"
              @blur="commitFolderRename"
              @click.stop
              @keydown.esc.stop="renamingFolder = null"
            />
            <span
              v-else
              class="tcp-group-name"
              :title="g.name ? `${g.name}（${t('cases.renameHint')}）` : t('cases.renameFolder')"
              @dblclick.stop="startFolderRename(g.name)"
            >
              {{ g.name ? g.name : t('cases.ungrouped') }}
            </span>
            <span class="tcp-group-count">{{ g.cases.length }}</span>
          </button>
          <div class="tcp-group-actions">
            <template v-if="groupRun?.name === g.name">
              <span class="tcp-group-progress">
                {{ t('cases.runningProgress', { done: groupRun.done, total: groupRun.total }) }}
              </span>
              <IconButton name="x" :size="13" :title="t('common.cancel')" @click="cancelGroupRun" />
            </template>
            <template v-else>
              <IconButton
                name="plus"
                :size="13"
                :title="t('cases.addCaseTo', { name: g.name || t('cases.ungrouped') })"
                @click="openCreate(g.name)"
              />
              <IconButton
                name="play"
                :size="13"
                :title="t('cases.runGroup')"
                :disabled="runningAll || !!groupRun || !g.cases.length"
                @click="runGroup(g.name)"
              />
              <IconButton
                name="more-horizontal"
                :size="13"
                :title="t('common.moreActions')"
                @click="openGroupMenu($event, g)"
              />
            </template>
          </div>
        </div>
        <template v-if="!collapsedFolders.has(g.name)">
          <div v-if="!g.cases.length" class="tcp-group-empty">
            {{ t('cases.groupEmpty') }}
            <button class="tcp-group-empty-add" type="button" @click="openCreate(g.name)">
              <Icon name="plus" :size="11" /> {{ t('cases.addCase') }}
            </button>
          </div>
          <div
            v-for="(c, j) in g.cases"
            :key="c.id"
            class="tcp-row tcp-body-row"
            :class="{ 'is-selected': selectedId === c.id }"
            @click="selectRow(c)"
          >
            <span class="tcp-col-idx">{{ g.start + j + 1 }}</span>
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
        </template>
      </template>
    </div>
    <EmptyState
      v-else-if="!pendingFolders.size"
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
      :folder="editingCase ? editingCase.folder ?? '' : modalFolder"
      :folders="folderNames"
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
    <Menu ref="groupMenuEl" @select="onGroupMenuSelect" @confirm="onGroupMenuConfirm" />
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

/* ---- 文件夹分组 ---- */
.tcp-group-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-top: 1px solid var(--border);
  background: var(--bg-2);
  min-width: 0;
}

.tcp-group-head:first-of-type {
  border-top: none;
}

.tcp-group-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
  border: none;
  background: none;
  color: var(--text-1);
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
  text-align: left;
}

.tcp-group-toggle:hover {
  background: var(--bg-hover);
}

.tcp-group-chevron {
  flex: none;
  color: var(--text-3);
  transition: transform var(--dur) var(--ease);
}

.tcp-group-chevron.open {
  transform: rotate(90deg);
}

.tcp-group-icon {
  flex: none;
  color: var(--text-3);
}

.tcp-group-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tcp-group-rename {
  min-width: 0;
  flex: 1;
  max-width: 280px;
  padding: 2px 6px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-sm);
  background: var(--bg-1);
  color: var(--text-1);
  font-size: var(--fs-xs);
  outline: none;
}

.tcp-group-count {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  background: var(--bg-hover);
  border-radius: 999px;
  padding: 1px 7px;
}

.tcp-group-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

.tcp-group-progress {
  font-size: var(--fs-xxs);
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}


/* ---- 行内新建文件夹：与表格行同语的紧凑卡片，不做全宽横幅 ---- */
.tcp-create-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: fit-content;
  min-width: 340px;
  max-width: 60%;
  padding: 6px 8px;
  margin-bottom: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
}

.tcp-create-icon {
  flex: none;
  color: var(--accent);
}

.tcp-create-input {
  flex: 1;
  min-width: 160px;
  max-width: 260px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-1);
  color: var(--text-1);
  font-size: var(--fs-xs);
  outline: none;
}

.tcp-create-input:focus {
  border-color: var(--accent);
}

/* 确认为主操作：主题色淡底与取消形成主次 */
.tcp-create-confirm {
  color: var(--accent);
  background: var(--accent-tint);
}

.tcp-create-confirm:hover {
  background: var(--accent-tint);
  filter: brightness(1.15);
}

/* ---- 空分组引导行 ---- */
.tcp-group-empty {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px 8px 34px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.tcp-group-empty-add {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: none;
  color: var(--accent);
  font-size: var(--fs-xs);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
}

.tcp-group-empty-add:hover {
  background: var(--bg-hover);
}

</style>