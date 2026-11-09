<script setup lang="ts">
/**
 * TestCasesPanel：Apifox 风格测试用例管理面板。
 * - 顶部分类 Tab（全部 (N) / 正向 / 负向 / 边界值 / 安全性 / 其他），点击过滤；
 * - 右上角：+ 添加用例 / ▶ 全部运行；
 * - 数据表：# | 名称 | 分组 | 运行结果 | 操作（... 菜单：直接运行 / 编辑 / 克隆 / 删除）；
 * - 点击用例行 → 回填调试页（method / path / params / headers / body）并切回「调试」。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Sortable from 'sortablejs'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { CATEGORY_TONE, TEST_CASE_CATEGORIES, caseCategoryLabel, formatDuration, matchTestCaseKeyword, relativeRunTime, statusTextOf, statusToneOf } from '../utils/testCases'
import { methodTextTone } from '../utils/methodTone'
import type { Endpoint, HttpMethod, TestCase, TestCaseCategory } from '../types/miyoApi'
import CustomSelect from './ui/CustomSelect.vue'
import EmptyState from './ui/EmptyState.vue'
import EnvBadge from './ui/EnvBadge.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import Modal from './ui/Modal.vue'
import Popconfirm from './ui/Popconfirm.vue'
import Skeleton from './ui/Skeleton.vue'
import TestCaseDrawer from './TestCaseDrawer.vue'
import TestCaseModal from './TestCaseModal.vue'
import ExportSmokeDialog from './docs/ExportSmokeDialog.vue'
import { useToast } from '../composables/useToast'
import { useMiyoApi } from '../composables/useMiyoApi'

const props = defineProps<{ draft: Endpoint | null }>()

const store = useWorkspaceStore()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const api = useMiyoApi()

type FilterKey = '全部' | TestCaseCategory

const filter = ref<FilterKey>('全部')

/** 关键字搜索：名称 / 方法 / 路径 / Body 文本 / 参数与请求头键值（见 matchTestCaseKeyword）。 */
const keyword = ref('')

/** 当前接口的用例（后端按创建时间排序，克隆/追加在尾部）。 */
const cases = computed<TestCase[]>(() =>
  props.draft ? (store.testCases.get(props.draft.id) ?? []) : [],
)

/** 执行状态过滤（全部 / 通过 / 失败 / 未执行）。 */
type StatusFilter = 'all' | 'Success' | 'Failed' | 'Untested'
const statusFilter = ref<StatusFilter>('all')

/** 请求方法过滤（全部 / GET / POST / …；按当前用例里出现过的方法动态生成）。 */
const methodFilter = ref<'all' | HttpMethod>('all')
const methodOptions = computed<{ value: 'all' | HttpMethod; label: string }[]>(() => {
  const seen = new Set<HttpMethod>()
  for (const c of cases.value) seen.add(c.method)
  return [
    { value: 'all', label: t('cases.methodAll') },
    ...[...seen].map((m) => ({ value: m, label: m })),
  ]
})

const filtered = computed<TestCase[]>(() => {
  const byCat =
    filter.value === '全部' ? cases.value : cases.value.filter((c) => c.category === filter.value)
  const byStatus = statusFilter.value === 'all' ? byCat : byCat.filter((c) => effectiveStatus(c) === statusFilter.value)
  const byMethod = methodFilter.value === 'all' ? byStatus : byStatus.filter((c) => c.method === methodFilter.value)
  const q = keyword.value.trim()
  if (!q) return byMethod
  return byMethod.filter((c) => matchTestCaseKeyword(c, q))
})

/** 生效状态：会话内跑过以 caseRunMeta 摘要为准（和列表列同源），否则看落库值。 */
function effectiveStatus(c: TestCase): 'Success' | 'Failed' | 'Untested' {
  const meta = store.caseRunMeta.get(c.id)
  if (meta) return meta.status >= 200 && meta.status < 400 ? 'Success' : 'Failed'
  return c.last_run_status
}

/** 统计条：总用例 / 通过 / 失败 / 未执行 / 通过率（当前接口全量口径，随过滤 Tab 无关）。 */
const stats = computed(() => {
  let passed = 0
  let failed = 0
  let untested = 0
  for (const c of cases.value) {
    const st = effectiveStatus(c)
    if (st === 'Success') passed += 1
    else if (st === 'Failed') failed += 1
    else untested += 1
  }
  const total = cases.value.length
  const executed = passed + failed
  return {
    total,
    passed,
    failed,
    untested,
    rate: executed ? Math.round((passed / executed) * 100) : null,
  }
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
    { key: 'add-to-scenario', label: t('cases.addToScenario'), icon: 'package', dividerBefore: true },
    { key: 'clone', label: t('cases.menuClone'), icon: 'copy' },
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
    case 'add-to-scenario':
      scenarioTarget.value = c
      scenarioDialogOpen.value = true
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

// ---------- 添加到集成测试场景（统一执行引擎：用例 → 场景步骤） ----------
const scenarioDialogOpen = ref(false)
const scenarioTarget = ref<TestCase | null>(null)
const scenarios = ref<import('../types/miyoApi').Scenario[]>([])
const scenarioPick = ref<string>('__new__')
const scenarioNewName = ref('')
const scenarioBusy = ref(false)

watch(scenarioDialogOpen, async (open) => {
  if (!open) return
  scenarioPick.value = '__new__'
  scenarioNewName.value = ''
  try {
    scenarios.value = props.draft?.project_id
      ? ((await api.listScenarios(props.draft.project_id)) ?? [])
      : []
  } catch {
    scenarios.value = []
  }
})

/** 用例快照 → 场景内联步骤：请求整体内联，断言 / 提取挂步骤级（同一引擎求值）。 */
function caseToScenarioStep(c: TestCase): import('../types/miyoApi').ScenarioStep {
  const { spec } = store.buildCaseRequest(c)
  const isAbs = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(c.url_path)
  const path = c.url_path.startsWith('/') ? c.url_path : `/${c.url_path}`
  return {
    endpoint_id: null,
    enabled: true,
    name: c.name,
    extract: Array.isArray(c.extractors) ? c.extractors : [],
    assertions: Array.isArray(c.assertions) ? c.assertions : [],
    request: {
      method: c.method,
      url: isAbs ? c.url_path : `{{base_url}}${path}`,
      spec,
    },
  }
}

async function confirmAddToScenario(): Promise<void> {
  const d = props.draft
  const c = scenarioTarget.value
  if (!d || !c || scenarioBusy.value) return
  scenarioBusy.value = true
  try {
    const step = caseToScenarioStep(c)
    const now = new Date().toISOString()
    if (scenarioPick.value === '__new__') {
      const name = scenarioNewName.value.trim()
      if (!name) {
        toast.warning(t('cases.scenarioNameRequired'))
        return
      }
      const scenario: import('../types/miyoApi').Scenario = {
        id: crypto.randomUUID(),
        project_id: d.project_id,
        name,
        description: '',
        category: '',
        sort_order: 0,
        steps: [step],
        created_at: now,
        updated_at: now,
      }
      await api.saveScenario(scenario)
    } else {
      const target = scenarios.value.find((x) => x.id === scenarioPick.value)
      if (!target) return
      target.steps.push(step)
      target.updated_at = now
      await api.saveScenario(target)
    }
    toast.success(t('cases.scenarioAdded', { name: c.name }))
    scenarioDialogOpen.value = false
  } catch (err) {
    toast.error(t('ws.caseUpdateFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    scenarioBusy.value = false
  }
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

/** 抽屉载荷（完整：请求快照 + 断言提取 + 执行控制）。 */
interface DrawerPayload {
  name: string
  category: TestCaseCategory
  folder: string
  method: string
  urlPath: string
  pathVariables: TestCase['path_variables']
  params: TestCase['params']
  headers: TestCase['headers']
  bodyType: string
  bodyContent: string
  assertions: TestCase['assertions']
  extractors: TestCase['extractors']
  auth: TestCase['auth']
  timeoutMs: number | null
  disableCookies: boolean
}

/** 抽屉「立即运行」：原地执行（不切 Tab），结果留在抽屉内。 */
async function runDrawerPayload(payload: DrawerPayload): Promise<import('../types/miyoApi').EndpointResult | null> {
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
    assertions: payload.assertions ?? [],
    extractors: payload.extractors ?? [],
    auth: payload.auth ?? null,
    timeout_ms: payload.timeoutMs,
    disable_cookies: payload.disableCookies,
  }
  return store.runTestCase(d.id, snapshot, store.activeEnvId)
}

/** 抽屉「保存修改」：名称/分组/文件夹 + 请求内容 + 断言提取 + 执行控制一起落库。 */
async function saveDrawerPayload(payload: DrawerPayload): Promise<void> {
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
    assertions: payload.assertions ?? [],
    extractors: payload.extractors ?? [],
    auth: payload.auth ?? null,
    timeoutMs: payload.timeoutMs,
    disableCookies: payload.disableCookies,
  })
  const metaChanged =
    payload.name !== c.name || payload.category !== c.category || (payload.folder ?? '') !== (c.folder ?? '')
  if (metaChanged) {
    await store.renameTestCase(d.id, c.id, payload.name, payload.category, payload.folder)
  }
  // 同步父级持有的抽屉用例引用（保存后 lastResult / 标题即时刷新）。
  const list = store.testCases.get(d.id)
  const target = list?.find((x) => x.id === c.id)
  if (target) {
    drawerCase.value = target
  }
  toast.success(t('cases.saved'))
}

// ---------- 批量选择 / 批量操作 ----------
/** 勾选的用例 id（勾选即进入批量模式，操作条出现）。 */
const checkedIds = ref<Set<string>>(new Set())

function toggleCheck(id: string): void {
  const next = new Set(checkedIds.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  checkedIds.value = next
}

function isGroupAllChecked(g: { cases: TestCase[] }): boolean {
  return g.cases.length > 0 && g.cases.every((c) => checkedIds.value.has(c.id))
}

/** 全选 / 取消一组（组头复选框）。 */
function toggleGroupCheck(g: { cases: TestCase[] }): void {
  const next = new Set(checkedIds.value)
  const all = isGroupAllChecked(g)
  for (const c of g.cases) {
    if (all) next.delete(c.id)
    else next.add(c.id)
  }
  checkedIds.value = next
}

function clearChecks(): void {
  checkedIds.value = new Set()
}

const checkedCases = computed<TestCase[]>(() =>
  cases.value.filter((c) => checkedIds.value.has(c.id)),
)

/** 批量删除（Popconfirm 二次确认后逐条删，复用现有删除链路）。 */
const batchDeleting = ref(false)
async function batchDelete(): Promise<void> {
  const d = props.draft
  if (!d || !checkedCases.value.length || batchDeleting.value) return
  batchDeleting.value = true
  try {
    for (const c of checkedCases.value) {
      await store.removeTestCase(d.id, c.id)
    }
    toast.success(t('cases.batchDeleted', { n: checkedCases.value.length }))
    clearChecks()
  } finally {
    batchDeleting.value = false
  }
}

/** 批量运行：勾选集合并入集合链路（复用 runAllTestCases 的 ids 过滤）。 */
const batchRunning = ref(false)
async function batchRun(): Promise<void> {
  const d = props.draft
  if (!d || !checkedCases.value.length || batchRunning.value || runningAll.value) return
  batchRunning.value = true
  const runId = crypto.randomUUID()
  activeCollectionRunId.value = runId
  collectionProgress.value = { done: 0, total: checkedCases.value.length }
  try {
    const r = await store.runAllTestCases(d.id, {
      runId,
      ids: checkedCases.value.map((c) => c.id),
      onProgress: (done, total) => {
        collectionProgress.value = { done, total }
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
    if (activeCollectionRunId.value === runId) activeCollectionRunId.value = null
    collectionProgress.value = null
    batchRunning.value = false
  }
}

/** 批量移动分组（下拉选择目标组；空 = 未分组）。 */
async function batchMove(folder: string): Promise<void> {
  const d = props.draft
  if (!d || !checkedCases.value.length) return
  const ok = await store.setTestCaseFolder(d.id, checkedCases.value.map((c) => c.id), folder)
  if (ok) toast.success(t('cases.batchMoved', { n: checkedCases.value.length }))
}

/** 批量操作条的目标分组候选（现有组 + 未分组）。 */
const batchMoveTargets = computed<{ value: string; label: string }[]>(() => [
  ...folderNames.value.map((f) => ({ value: f, label: f })),
  { value: '', label: t('cases.ungrouped') },
])

// ---------- 拖拽排序 / 跨组移动（SortableJS，组内换位 + 跨组改 folder） ----------
const groupEls = new Map<string, HTMLElement>()
const sortables: Sortable[] = []

function setGroupEl(name: string, el: unknown): void {
  if (el instanceof HTMLElement) groupEls.set(name, el)
  else groupEls.delete(name)
}

function initSortables(): void {
  disposeSortables()
  for (const [name, el] of groupEls) {
    sortables.push(
      Sortable.create(el, {
        group: 'tcp-cases',
        animation: 150,
        easing: 'cubic-bezier(0.2, 0, 0, 1)',
        filter: 'button, input, label',
        preventOnFilter: false,
        onEnd: (evt) => {
          void onCaseDrop(name, evt)
        },
      }),
    )
  }
}

function disposeSortables(): void {
  for (const s of sortables) s.destroy()
  sortables.length = 0
}

/** 落点处理：组内换位 → 全量重排序；跨组 → 改 folder + 重排序。 */
async function onCaseDrop(
  fromName: string,
  evt: Sortable.SortableEvent,
): Promise<void> {
  const d = props.draft
  if (!d) return
  const toName = (evt.to as HTMLElement).dataset.groupName ?? ''
  const draggedId = (evt.item as HTMLElement).dataset.caseId
  if (!draggedId) return
  // DOM 新序 → 该组 id 序（只包含可见行；过滤态下拖拽只影响可见子集）。
  const domIds = [...(evt.to as HTMLElement).querySelectorAll<HTMLElement>('[data-case-id]')]
    .map((el) => el.dataset.caseId)
    .filter((id): id is string => !!id)
  if (toName !== fromName) {
    const ok = await store.setTestCaseFolder(d.id, [draggedId], toName)
    if (!ok) return
  }
  // 全量稳定序：把可见子集的 DOM 序合并回完整列表（未见的用例保持相对位置）。
  const full = store.testCases.get(d.id) ?? []
  const subset = new Set(domIds)
  const ordered: string[] = []
  let k = 0
  for (const c of full) {
    if (subset.has(c.id)) {
      ordered.push(domIds[k++] ?? c.id)
    } else {
      ordered.push(c.id)
    }
  }
  await store.reorderTestCases(d.id, ordered)
}

onMounted(initSortables)
onBeforeUnmount(disposeSortables)
// 分组结构变化（新建/重命名/过滤切换）后重建 Sortable 实例。
watch(groups, () => initSortables(), { flush: 'post' })

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
      // 引擎判定：有断言看 ok；无断言维持 2xx-3xx。提示带断言通过数。
      const ok = effectiveStatus(c)
      const n = res.outcomes.length
      const detail = [
        res.status != null ? `HTTP ${res.status}` : null,
        res.duration_ms != null ? formatDuration(res.duration_ms) : null,
        n ? t('cases.assertSummary', { ok: res.outcomes.filter((o) => o.passed).length, n }) : null,
      ]
        .filter(Boolean)
        .join(' · ')
      if (ok === 'Success') {
        toast.success(t('cases.passed', { name: c.name }), { message: detail })
      } else {
        toast.error(t('cases.failed', { name: c.name }), { message: detail })
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

/** 最近一次运行元信息（状态码 / 耗时 / 结果），运行结果列富化展示。 */
function runMetaOf(c: TestCase): { status: number; durationMs: number; result?: import('../types/miyoApi').EndpointResult } | undefined {
  return store.caseRunMeta.get(c.id)
}

/** 最近一次断言摘要（落库值：ok/fail；会话内以 result 为准）。 */
function assertSummaryOf(c: TestCase): { ok: number; fail: number } | null {
  const r = runMetaOf(c)?.result
  if (r) return { ok: r.outcomes.filter((o) => o.passed).length, fail: r.outcomes.filter((o) => !o.passed).length }
  if (c.last_run_ok == null && c.last_run_fail == null) return null
  return { ok: c.last_run_ok ?? 0, fail: c.last_run_fail ?? 0 }
}

/** 列表加载态（骨架）与错误态（重试）。 */
const loadState = computed<'loading' | 'ready' | 'error'>(() =>
  props.draft ? (store.testCaseLoadState.get(props.draft.id) ?? 'ready') : 'ready',
)

watch(
  () => props.draft?.id,
  () => {
    filter.value = '全部'
    keyword.value = ''
    statusFilter.value = 'all'
    methodFilter.value = 'all'
    selectedId.value = null
    checkedIds.value = new Set()
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
          @click="checkedIds.size ? clearChecks() : checkedIds = new Set(cases.map((c) => c.id))"
        >
          <Icon name="list" :size="13" /> {{ checkedIds.size ? t('cases.batchCancel') : t('cases.batchSelect') }}
        </button>
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

    <!-- 统计条：总用例 / 通过 / 失败 / 未执行 / 通过率 -->
    <div v-if="cases.length" class="tcp-stats">
      <span class="tcp-stat">
        <span class="tcp-stat-num">{{ stats.total }}</span> {{ t('cases.statTotal') }}
      </span>
      <span class="tcp-stat tone-pass">
        <span class="tcp-stat-num">{{ stats.passed }}</span> {{ t('cases.statPassed') }}
      </span>
      <span class="tcp-stat tone-fail">
        <span class="tcp-stat-num">{{ stats.failed }}</span> {{ t('cases.statFailed') }}
      </span>
      <span class="tcp-stat">
        <span class="tcp-stat-num">{{ stats.untested }}</span> {{ t('cases.statUntested') }}
      </span>
      <span v-if="stats.rate !== null" class="tcp-stat tcp-stat-rate">
        {{ t('cases.statRate') }} <span class="tcp-stat-num">{{ stats.rate }}%</span>
      </span>
    </div>

    <!-- 二级过滤：执行状态 + 请求方法（与分类 Tab / 关键字搜索叠加） -->
    <div v-if="cases.length" class="tcp-subfilters">
      <CustomSelect
        class="tcp-subfilter"
        :model-value="statusFilter"
        :options="[
          { value: 'all', label: t('cases.statusAll') },
          { value: 'Success', label: t('cases.statusPassed') },
          { value: 'Failed', label: t('cases.statusFailed') },
          { value: 'Untested', label: t('cases.statusUntested') },
        ]"
        size="sm"
        :pop-min-width="110"
        @update:model-value="(v) => (statusFilter = String(v ?? 'all') as StatusFilter)"
      />
      <CustomSelect
        class="tcp-subfilter"
        :model-value="methodFilter"
        :options="methodOptions"
        size="sm"
        :pop-min-width="110"
        @update:model-value="(v) => (methodFilter = String(v ?? 'all') as 'all' | HttpMethod)"
      />
    </div>

    <!-- 批量操作条 -->
    <div v-if="checkedIds.size" class="tcp-batchbar">
      <span class="tcp-batchbar-n">{{ t('cases.batchSelected', { n: checkedIds.size }) }}</span>
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="batchRunning" @click="batchRun">
        <Icon name="play" :size="12" /> {{ t('cases.batchRun') }}
      </button>
      <CustomSelect
        class="tcp-batchbar-move"
        :model-value="'__noop__'"
        :options="[{ value: '__noop__', label: t('cases.batchMove') }, ...batchMoveTargets]"
        size="sm"
        :pop-min-width="140"
        @update:model-value="(v) => { if (v !== '__noop__') { void batchMove(String(v ?? '')) } }"
      />
      <Popconfirm :title="t('cases.batchDeleteConfirm', { n: checkedIds.size })" @confirm="batchDelete">
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" :disabled="batchDeleting">
          <Icon name="trash" :size="12" /> {{ t('common.delete') }}
        </button>
      </Popconfirm>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="clearChecks">
        {{ t('cases.batchCancel') }}
      </button>
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

    <!-- 加载骨架（首屏拉取用例期间） -->
    <div v-if="loadState === 'loading'" class="sk-cases" aria-busy="true" :aria-label="t('common.loading')">
      <Skeleton v-for="i in 5" :key="i" width="86%" height="14px" />
    </div>
    <!-- 加载失败：错误条 + 重试 -->
    <div v-else-if="loadState === 'error'" class="miyo-inline-error" role="alert">
      <span class="miyo-inline-error-text">{{ t('cases.loadFail') }}</span>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="store.loadTestCases(props.draft!.id)">
        {{ t('common.retry') }}
      </button>
    </div>
    <!-- 用例表格（按文件夹分组：命名组按发现序，未分组殿后；可折叠；组内拖拽排序 / 跨组拖动） -->
    <div v-else-if="filtered.length || pendingFolders.size" class="tcp-table">
      <div class="tcp-row tcp-head">
        <span v-if="checkedIds.size" class="tcp-col-check"></span>
        <span class="tcp-col-idx">#</span>
        <span class="tcp-col-name">{{ t('cases.colName') }}</span>
        <span class="tcp-col-last">{{ t('cases.colLastRun') }}</span>
        <span class="tcp-col-status">{{ t('cases.colResult') }}</span>
        <span class="tcp-col-ops">{{ t('cases.colOps') }}</span>
      </div>
      <template v-for="g in groups" :key="g.name || '__ungrouped__'">
        <!-- 分组头：折叠 + 名称（行内重命名）+ 计数 + 分组运行 + 操作菜单 -->
        <div class="tcp-group-head" :class="{ 'is-collapsed': collapsedFolders.has(g.name) }">
          <label v-if="checkedIds.size && g.cases.length" class="tcp-group-check" :title="t('cases.batchSelect')">
            <input
              type="checkbox"
              :checked="isGroupAllChecked(g)"
              @click.prevent
              @change="toggleGroupCheck(g)"
            />
          </label>
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
          <!-- Sortable 容器：组内换位 + 跨组拖动（data-group-name 标识落点组） -->
          <div
            class="tcp-group-body"
            :data-group-name="g.name"
            :ref="(el) => setGroupEl(g.name, el)"
          >
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
              :data-case-id="c.id"
              @click="selectRow(c)"
            >
              <label v-if="checkedIds.size" class="tcp-col-check" @click.stop>
                <input
                  type="checkbox"
                  :checked="checkedIds.has(c.id)"
                  @change="toggleCheck(c.id)"
                />
              </label>
              <span class="tcp-col-idx">{{ g.start + j + 1 }}</span>
              <button class="tcp-col-name tcp-name-btn" type="button" @click="openDrawer(c)">
                <span class="tcp-name-line">
                  <span class="tcp-method" :class="methodTextTone(c.method)">{{ c.method }}</span>
                  <span class="tcp-name-text">{{ c.name }}</span>
                  <span class="tcp-cat-tag" :style="{ color: CATEGORY_TONE[c.category] }">
                    {{ caseCategoryLabel(c.category) }}
                  </span>
                </span>
                <span class="tcp-path-line" :title="c.url_path">{{ c.url_path }}</span>
              </button>
              <span class="tcp-col-last">
                <template v-if="c.last_run_at || runMetaOf(c)">
                  <span class="tcp-last-time">{{ relativeRunTime(c.last_run_at) }}</span>
                  <span v-if="runMetaOf(c)" class="tcp-last-dur">{{ formatDuration(runMetaOf(c)!.durationMs) }}</span>
                  <span
                    v-else-if="assertSummaryOf(c)"
                    class="tcp-last-assert"
                    :class="{ 'has-fail': assertSummaryOf(c)!.fail }"
                  >
                    {{ assertSummaryOf(c)!.ok }}/{{ assertSummaryOf(c)!.ok + assertSummaryOf(c)!.fail }}
                  </span>
                </template>
                <span v-else class="tcp-last-none">—</span>
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
          </div><!-- /tcp-group-body -->
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

    <!-- 添加到集成测试场景：选已有场景或新建（步骤内联用例快照，统一引擎） -->
    <Modal
      :open="scenarioDialogOpen"
      :title="t('cases.addToScenarioTitle', { name: scenarioTarget?.name ?? '' })"
      width="440px"
      @update:open="scenarioDialogOpen = $event"
    >
      <div class="tcp-scn-list">
        <label class="tcp-scn-item">
          <input v-model="scenarioPick" type="radio" value="__new__" />
          <input
            v-model="scenarioNewName"
            class="miyo-input tcp-scn-newname"
            type="text"
            :placeholder="t('cases.scenarioNewPh')"
            spellcheck="false"
            maxlength="60"
            @click="scenarioPick = '__new__'"
          />
        </label>
        <label v-for="sc in scenarios" :key="sc.id" class="tcp-scn-item">
          <input v-model="scenarioPick" type="radio" :value="sc.id" />
          <span class="tcp-scn-name">{{ sc.name }}</span>
          <span class="tcp-scn-n">{{ sc.steps.length }}</span>
        </label>
        <p v-if="!scenarios.length" class="tcp-scn-empty">{{ t('cases.scenarioEmpty') }}</p>
      </div>
      <template #footer>
        <button class="miyo-btn" type="button" @click="scenarioDialogOpen = false">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-primary"
          type="button"
          :disabled="scenarioBusy || (scenarioPick === '__new__' && !scenarioNewName.trim())"
          @click="confirmAddToScenario"
        >
          {{ t('common.confirm') }}
        </button>
      </template>
    </Modal>

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
      :endpoint="draft"
      :test-case="drawerCase"
      :last-result="drawerCase ? (store.caseRunMeta.get(drawerCase.id)?.result ?? null) : null"
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

/* ---- 统计条：总 / 通过 / 失败 / 未执行 / 通过率 ---- */
.tcp-stats {
  display: flex;
  align-items: center;
  gap: 14px;
  margin: 8px 12px 0;
  padding: 6px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-2);
  font-size: var(--fs-xs);
  color: var(--text-2);
  flex-wrap: wrap;
}
.tcp-stat-num {
  font-weight: 600;
  color: var(--text-1);
  font-family: var(--font-mono);
}
.tcp-stat.tone-pass .tcp-stat-num { color: var(--success); }
.tcp-stat.tone-fail .tcp-stat-num { color: var(--danger); }
.tcp-stat-rate {
  margin-left: auto;
  color: var(--accent);
}

/* ---- 二级过滤：状态 / 方法 ---- */
.tcp-subfilters {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 12px 0;
}
.tcp-subfilter {
  width: 130px;
}

/* ---- 批量操作条 ---- */
.tcp-batchbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 12px 0;
  padding: 6px 10px;
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  border-radius: var(--radius-md);
  background: var(--accent-tint);
}
.tcp-batchbar-n {
  font-size: var(--fs-xs);
  color: var(--text-1);
  font-weight: 600;
  margin-right: auto;
}
.tcp-batchbar-move {
  width: 150px;
}

/* ---- 组头复选 / 行复选 ---- */
.tcp-group-check,
.tcp-col-check {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}
.tcp-col-check {
  width: 18px;
}
.tcp-group-check input,
.tcp-col-check input {
  accent-color: var(--accent);
}

/* ---- 组体（Sortable 容器） ---- */
.tcp-group-body {
  display: flex;
  flex-direction: column;
}
/* 拖拽落点高亮（Sortable chosen/ghost） */
.tcp-group-body .sortable-ghost {
  opacity: 0.4;
}
.tcp-group-body .sortable-chosen {
  box-shadow: 0 4px 14px rgb(0 0 0 / 0.3);
}

/* ---- 「最近执行」列：相对时间 + 耗时 / 断言摘要 ---- */
.tcp-col-last {
  width: 130px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-3);
  white-space: nowrap;
}
.tcp-last-time {
  font-family: var(--font-mono);
}
.tcp-last-dur,
.tcp-last-assert {
  font-family: var(--font-mono);
  color: var(--text-2);
}
.tcp-last-assert.has-fail {
  color: var(--danger);
}
.tcp-last-none {
  color: var(--text-3);
  opacity: 0.6;
}

/* ---- 名称两行密度：第一行 方法徽章 + 名称 + 分类 tag，第二行路径 ---- */
.tcp-name-btn {
  flex-direction: column;
  align-items: stretch;
  gap: 1px;
  padding: 4px 0;
}
.tcp-name-line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.tcp-cat-tag {
  flex-shrink: 0;
  font-size: var(--fs-xxs);
  opacity: 0.85;
}
.tcp-path-line {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ---- 添加到场景对话框 ---- */
.tcp-scn-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 300px;
  overflow-y: auto;
}
.tcp-scn-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  cursor: pointer;
  transition: border-color var(--dur) var(--ease);
}
.tcp-scn-item:hover {
  border-color: var(--border-strong);
}
.tcp-scn-item input[type='radio'] {
  accent-color: var(--accent);
  flex-shrink: 0;
}
.tcp-scn-newname {
  flex: 1;
  height: 26px;
  font-size: var(--fs-xs);
}
.tcp-scn-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
}
.tcp-scn-n {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.tcp-scn-empty {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

/* ---- 加载骨架 ---- */
.sk-cases {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin: 12px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
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