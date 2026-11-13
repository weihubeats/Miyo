<script setup lang="ts">
/**
 * EnvWorkspace：环境管理统一工作区（左右双栏主体，唯一环境编辑实现）。
 *
 * 由 EnvironmentManager（弹窗壳）与 SettingsDialog 环境页签（内嵌）共用：
 * - 弹窗模式：父级提供 Modal 外壳；Modal 关闭即卸载内容，每次打开重新挂载，
 *   onMounted 即完成初始化（等价于旧的 watch(open) 语义）；
 * - 内嵌模式（embedded）：无外壳直接渲染，长驻设置页签内；项目切换经守卫重载，
 *   并向父级暴露 hasPending / guardAction / guardClose 供页签与弹窗关闭守卫复用。
 *
 * 左侧 Sidebar：「全局配置」组（全局变量 / 全局参数 / Vault Secrets 占位「规划中」）
 * +「环境」组（色点 + 名称 + Base URL + 当前徽标 + ⋯ 菜单：设为当前 / 重命名 /
 * 复制 / 删除），底部导入 / 导出（可选脱敏导出）。
 * 右侧详情：环境名称 + 生效状态 + Base URL（格式校验）+ 变量表
 * （变量名 | 远程值 | 本地覆盖值 | 类型 | 启用 | 操作，见 EnvVarTable）。
 *
 * 所有编辑作用于本地副本，保存时一次落库；未保存切换 / 关闭弹三选确认
 * （保存并继续 / 放弃修改 / 继续编辑）。detached 模式（管理非当前项目）数据
 * 自加载、保存不回写工作区列表。变量注入与解析链路不在本组件——只编辑数据。
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import {
  envBaseUrl,
  envColorClass,
  normalizeBaseUrl,
  resolveSensitive,
  validateBaseUrl,
} from '@/utils/environment'
import { deepClone } from '@/utils/clone'
import { rowKey } from '@/utils/rowKey'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Menu, { type MenuItem } from '@/components/ui/Menu.vue'
import Modal from '@/components/ui/Modal.vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import EnvVarTable from '@/domains/environment/components/env/EnvVarTable.vue'
import { errorText } from '@/utils/errorText'

import type {
  EnvExchangeFormat,
  Environment,
  EnvironmentVariable,
  GlobalParam,
  ImportedEnv,
} from '@/types/miyoApi.d'

const props = defineProps<{
  /** 内嵌模式：无 Modal 外壳（设置 → 环境页签直接渲染）。 */
  embedded?: boolean
  /** 独立管理模式：管理指定项目（非当前工作区项目）的环境——设置页跨项目。 */
  projectId?: string | null
  projectName?: string | null
  initialEnvId?: string | null
  createNew?: boolean
}>()

const emit = defineEmits<{ close: [] }>()

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t
const toast = useToast()
const api = useMiyoApi()

const envs = ref<Environment[]>([])
const selected = ref<Environment | null>(null)
const busy = ref(false)
const dirty = ref(false)
/** 新建环境是否仍未被编辑：仅创建本身不计入「未保存修改」，关闭时直接丢弃不弹确认。 */
let newEnvPristine = false

// ---------- 敏感值遮蔽总开关（肩窥保护，持久化；EnvVarTable 经 v-model 共享） ----------
const MASK_KEY = 'miyo.envmgr-hide-sensitive'
const hideSensitive = ref(tryReadHideSensitive())

function tryReadHideSensitive(): boolean {
  try {
    return localStorage.getItem(MASK_KEY) !== '0'
  } catch {
    return true
  }
}

function onHideSensitiveToggle(v: boolean): void {
  hideSensitive.value = v
  try {
    localStorage.setItem(MASK_KEY, v ? '1' : '0')
  } catch {
    // 存储不可用仅丢持久化
  }
}

// ---------- 模式与项目 ----------
/** 独立管理模式：目标项目不是当前工作区项目——数据自加载、保存不回写工作区列表。 */
const detached = computed(() => !!props.projectId && props.projectId !== store.project?.id)
/** 实际管理的项目 id：独立模式取 props，否则跟随工作区当前项目。 */
const effectiveProjectId = computed(() => (detached.value ? props.projectId : store.project?.id) ?? '')
/** 独立模式加载环境列表中。 */
const envsLoading = ref(false)

const activeEnvId = computed(() => (detached.value ? null : store.activeEnvId))

/** 右侧详情面板作用域：'env' = 环境详情；'global' = 全局变量；'params' = 全局参数。 */
const scope = ref<'env' | 'global' | 'params'>('env')
/** 全局变量本地副本（作用域 global 时编辑此表，保存时整体落库）。 */
const globalVars = ref<EnvironmentVariable[]>([])
const globalDirty = ref(false)
/** 全局参数本地副本（作用域 params 时编辑此表，保存时整体落库）。 */
const globalParams = ref<GlobalParam[]>([])
const paramsDirty = ref(false)

/** 全局组：全局变量 / 全局参数已启用；Vault Secrets 仍为占位（置灰，不伪装已实现）。 */
const globalItems = [
  { key: 'global_variables', labelKey: 'envmgr.globalVars', descKey: 'envmgr.globalVarsDesc', enabled: true },
  { key: 'global_params', labelKey: 'envmgr.globalParams', descKey: 'envmgr.globalParamsDesc', enabled: true },
  { key: 'vault_secrets', labelKey: 'envmgr.vaultSecrets', descKey: 'envmgr.vaultDesc', enabled: false },
]

/** 全局组条目 hover 标题：标签 + 描述（未启用追加「规划中」）。 */
function globalItemTitle(item: (typeof globalItems)[number]): string {
  return t(item.enabled ? 'envmgr.itemTitle' : 'envmgr.itemTitleSoon', {
    label: t(item.labelKey),
    desc: t(item.descKey),
  })
}

function select(env: Environment | null): void {
  commitRename()
  selected.value = env ? deepClone(env) : null
  dirty.value = false
  newEnvPristine = false
  scope.value = 'env'
}

function selectGlobal(): void {
  cancelRename()
  scope.value = 'global'
  globalVars.value = deepClone(store.globalVariables)
  globalDirty.value = false
  selected.value = null
}

function selectParams(): void {
  cancelRename()
  scope.value = 'params'
  globalParams.value = deepClone(store.globalParams)
  paramsDirty.value = false
  selected.value = null
}

/** 全局变量表任意变更（EnvVarTable 原地改写后通知）。 */
function onGlobalChange(): void {
  globalDirty.value = true
}

function addGlobalParam(): void {
  globalParams.value.push({ key: '', value: '', enabled: true, location: 'header' })
  paramsDirty.value = true
}

/** 全局参数注入位置选项（随语言刷新的常量数组必须包 computed）。 */
const locationOptions = computed(() => [
  { value: 'header', label: t('envmgr.locHeader') },
  { value: 'query', label: t('envmgr.locQuery') },
])

function onParamLocation(p: GlobalParam, value: string | number): void {
  p.location = value as GlobalParam['location']
  onParamsChange()
}

function removeGlobalParam(index: number): void {
  globalParams.value.splice(index, 1)
  paramsDirty.value = true
}

function onParamsChange(): void {
  paramsDirty.value = true
}

// ---------- 未保存修改的关闭/切换确认 ----------
const confirmLeave = ref(false)
let pendingAction: (() => void) | null = null

function hasPending(): boolean {
  // 新建环境尚未被编辑时，创建本身不算修改：关闭/切换直接丢弃，不弹确认
  return (dirty.value && !newEnvPristine) || globalDirty.value || paramsDirty.value
}

function guardClose(): boolean {
  commitRename()
  if (!hasPending()) return true
  confirmLeave.value = true
  return false
}

/** 有未保存修改时把目标动作挂起，弹三选确认；无修改直接执行。 */
function guard(action: () => void): void {
  commitRename()
  if (!hasPending()) {
    action()
    return
  }
  pendingAction = action
  confirmLeave.value = true
}

function discardChanges(): void {
  if (busy.value) return
  const act = pendingAction
  pendingAction = null
  confirmLeave.value = false
  dirty.value = false
  newEnvPristine = false
  globalDirty.value = false
  paramsDirty.value = false
  cancelRename()
  act?.()
}

function keepEditing(): void {
  if (busy.value) return
  pendingAction = null
  confirmLeave.value = false
}

/** 确认弹窗自身被关闭（Esc / 遮罩 / ✕）= 继续编辑。 */
function onConfirmToggle(open: boolean): void {
  if (open) {
    confirmLeave.value = true
    return
  }
  keepEditing()
}

/** 保存并继续：落库成功才执行被拦截的切换/关闭动作，失败则留在确认弹窗。 */
async function saveAndProceed(): Promise<void> {
  if (busy.value) return
  const ok = await save()
  if (!ok) return
  const act = pendingAction
  pendingAction = null
  confirmLeave.value = false
  act?.()
}

// ---------- 加载与初始化 ----------
/** 恢复全局配置本地副本（打开 / 取消时重置）。 */
function initState(): void {
  globalVars.value = deepClone(store.globalVariables)
  globalDirty.value = false
  globalParams.value = deepClone(store.globalParams)
  paramsDirty.value = false
}

/**
 * 加载环境列表（stale-while-revalidate）：
 * - 刷新（keepStale）期间保留旧列表与选中项，响应到达后整表替换——不在请求前
 *   清空数据（先清空再请求会让列表闪成空白）；
 * - 首次加载 / 切项目（数据域变化）才清空并展示骨架；
 * - 非独立模式直接读工作区 store（同步，无加载态）。
 */
async function loadEnvs(opts: { keepStale?: boolean } = {}): Promise<void> {
  const keepStale = opts.keepStale === true && envs.value.length > 0
  const reselect = (list: Environment[]): void => {
    const current = selected.value ? list.find((e) => e.id === selected.value?.id) : undefined
    const preferred = props.initialEnvId
      ? list.find((e) => e.id === props.initialEnvId)
      : undefined
    const active = list.find((e) => e.id === (detached.value ? undefined : store.activeEnvId))
    select(current ?? preferred ?? active ?? list[0] ?? null)
  }
  if (detached.value) {
    if (!keepStale) {
      envs.value = []
      select(null)
    }
    envsLoading.value = true
    try {
      envs.value = await api.listEnvironments(effectiveProjectId.value)
      reselect(envs.value)
    } catch (err) {
      toast.error(t('envmgr.loadFail'), {
        message: errorText(err),
      })
    } finally {
      envsLoading.value = false
    }
  } else {
    // 环境表只做浅拷贝：左侧列表是纯读（点击 select 时才深克隆选中项）
    envs.value = [...store.environments]
    reselect(envs.value)
  }
}

onMounted(() => {
  initState()
  void loadEnvs().then(() => {
    // 「新建环境」快捷入口：打开即建一条待保存的新环境（createNew 由调用方在关闭时复位）
    if (props.createNew) addEnvironment()
  })
})

// 内嵌模式长驻：项目切换时经未保存守卫重载列表（弹窗模式每次打开重新挂载，无需此 watch）
watch(
  () => effectiveProjectId.value,
  (now, before) => {
    if (!props.embedded || now === before) return
    guard(() => void loadEnvs())
  },
)

/** 外部数据可能已变化（速览快编 / 顶栏切换），显式刷新——SWR：保留旧列表直至响应到达。 */
function refresh(): void {
  guard(() => {
    initState()
    void loadEnvs({ keepStale: true })
  })
}

function addEnvironment(): void {
  const now = new Date().toISOString()
  const env: Environment = {
    id: crypto.randomUUID(),
    project_id: effectiveProjectId.value,
    name: t('envmgr.newEnvName'),
    base_url: '',
    variables: [],
    created_at: now,
    updated_at: now,
  }
  envs.value.push(env)
  select(env)
  dirty.value = true
  newEnvPristine = true
  void beginRename(env, true)
}

// ---------- 左侧行内重命名 ----------
/** 当前处于行内编辑的环境 id（null = 无）。 */
const editingId = ref<string | null>(null)
const editValue = ref('')
const editInput = ref<HTMLInputElement | null>(null)

/** 函数 ref：忽略 unmount 的 null 回调，避免「切行编辑」时新旧元素 ref 乱序被清空。 */
function setEditInput(el: unknown): void {
  if (el) editInput.value = el as HTMLInputElement
}

/** 左侧行显示名：选中行实时跟随右侧编辑中的 selected.name，其余行走本地副本。 */
function rowName(env: Environment): string {
  return selected.value && selected.value.id === env.id ? selected.value.name : env.name
}

/** 进入行内编辑：selectAll=true（新建流程）全选默认名；否则光标置尾。 */
async function beginRename(env: Environment, selectAll = false): Promise<void> {
  if (editingId.value && editingId.value !== env.id) commitRename()
  editingId.value = env.id
  editValue.value = rowName(env)
  // 双拍 nextTick：首拍前弹窗的 autofocus 会把焦点抢到关闭按钮，第二拍才轮到输入框
  await nextTick()
  await nextTick()
  if (editingId.value !== env.id) return
  const el = editInput.value
  el?.focus()
  if (selectAll) el?.select()
  else el?.setSelectionRange(el.value.length, el.value.length)
}

/** Esc：回滚不落名。 */
function cancelRename(): void {
  if (!editingId.value) return
  editingId.value = null
  editValue.value = ''
}

/**
 * 提交行内改名：空名回退原名；名字没变不置脏（新建未编辑关闭不弹确认）。
 * 只写 selected.name，不落 envs 条目——行显示走 rowName()（选中行读 selected.name），
 * 与右侧 .em-name 完全同源；切换/丢弃时 select() 重取 envs 即自然回滚。
 */
function commitRename(): void {
  const id = editingId.value
  if (!id) return
  editingId.value = null
  const name = editValue.value.trim()
  editValue.value = ''
  if (!selected.value || selected.value.id !== id) return
  const current = selected.value.name
  const finalName = name || current
  if (finalName === current) return
  selected.value.name = finalName
  markDirty()
}

function onRowClick(env: Environment): void {
  if (editingId.value === env.id) return
  if (editingId.value) commitRename()
  guard(() => select(env))
}

/** 双击行名进入行内重命名（非选中行先经未保存确认再选中）。 */
function onRowDblClick(env: Environment): void {
  if (editingId.value === env.id) return
  if (selected.value?.id === env.id) {
    void beginRename(env)
    return
  }
  guard(() => {
    select(env)
    void beginRename(env)
  })
}

// ---------- 环境操作菜单（行 ⋯ 与详情头 ⋯ 共用） ----------
const rowMenu = ref<InstanceType<typeof Menu> | null>(null)
const menuEnvId = ref<string | null>(null)

function envById(id: string | null): Environment | null {
  return envs.value.find((e) => e.id === id) ?? null
}

function envMenuItems(env: Environment): MenuItem[] {
  const items: MenuItem[] = []
  if (!detached.value && env.id !== activeEnvId.value) {
    items.push({ key: 'set-active', label: t('envmgr.setActive'), icon: 'check' })
  }
  if (selected.value?.id === env.id) {
    items.push({ key: 'rename', label: t('envmgr.rename'), icon: 'pencil' })
  }
  items.push({ key: 'duplicate', label: t('envmgr.duplicate'), icon: 'copy' })
  items.push({
    key: 'delete',
    label: t('common.delete'),
    icon: 'trash',
    danger: true,
    dividerBefore: true,
    // 删除当前激活环境：追加「需重新选择环境」警示
    confirm:
      env.id === activeEnvId.value
        ? `${t('envmgr.deleteConfirmTitle', { name: rowName(env) })} ${t('envmgr.deleteActiveWarn')}`
        : t('envmgr.deleteConfirmTitle', { name: rowName(env) }),
  })
  return items
}

function openRowMenu(env: Environment, event: MouseEvent): void {
  menuEnvId.value = env.id
  rowMenu.value?.openAt(event.currentTarget as HTMLElement, envMenuItems(env), 'right')
}

function openSelectedMenu(event: MouseEvent): void {
  if (!selected.value) return
  menuEnvId.value = selected.value.id
  rowMenu.value?.openAt(event.currentTarget as HTMLElement, envMenuItems(selected.value), 'left')
}

function onMenuSelect(item: MenuItem): void {
  const env = envById(menuEnvId.value)
  if (!env) return
  if (item.key === 'set-active') {
    void setActiveEnv(env)
    return
  }
  // 重命名 / 复制会切换或重建选中项：经未保存守卫
  guard(() => {
    if (item.key === 'rename') {
      if (selected.value?.id !== env.id) select(env)
      void beginRename(env)
    } else if (item.key === 'duplicate') {
      void duplicateEnv(env)
    }
  })
}

function onMenuConfirm(item: MenuItem): void {
  if (item.key !== 'delete') return
  const env = envById(menuEnvId.value)
  if (!env) return
  guard(() => void remove(env))
}

/** 设为当前环境（仅非 detached；后端校验归属）。 */
async function setActiveEnv(env: Environment): Promise<void> {
  try {
    await store.setEnvironment(env.id)
    toast.success(t('envmgr.setActiveDone', { name: env.name }))
  } catch (err) {
    toast.error(t('envmgr.setActiveFail'), {
      message: errorText(err),
    })
  }
}

/** 复制环境：全量拷贝为的新环境（名称加「副本」后缀，重名自动递增），落库并选中。 */
async function duplicateEnv(env: Environment): Promise<void> {
  const now = new Date().toISOString()
  const taken = new Set(envs.value.map((e) => e.name))
  let name = `${rowName(env)}${t('envmgr.dupSuffix')}`
  let n = 2
  while (taken.has(name)) {
    name = `${rowName(env)}${t('envmgr.dupSuffixN', { n })}`
    n += 1
  }
  const copy: Environment = {
    ...deepClone(env),
    id: crypto.randomUUID(),
    project_id: effectiveProjectId.value,
    name,
    created_at: now,
    updated_at: now,
  }
  try {
    const saved = detached.value
      ? await store.saveEnvironmentRemote(copy)
      : await store.updateEnvironment(copy, { silent: true })
    envs.value.push(saved)
    select(saved)
    toast.success(t('envmgr.duplicated', { name: saved.name }))
  } catch (err) {
    toast.error(t('envmgr.saveFail'), { message: errorText(err) })
  }
}

async function remove(env: Environment): Promise<void> {
  const persisted = detached.value || store.environments.some((e) => e.id === env.id)
  try {
    if (persisted && !detached.value) await store.deleteEnvironment(env.id, { silent: true })
    if (detached.value) await api.deleteEnvironment(env.id)
    envs.value = envs.value.filter((e) => e.id !== env.id)
    if (editingId.value === env.id) cancelRename()
    if (selected.value?.id === env.id) {
      const idx = envs.value.findIndex((e) => e.id === env.id)
      select(envs.value[Math.min(Math.max(idx, 0), Math.max(envs.value.length - 1, 0))] ?? null)
    }
    toast.success(t('envmgr.deleted', { name: env.name }))
  } catch (err) {
    toast.error(t('envmgr.deleteFail'), { message: errorText(err) })
  }
}

// ---------- 环境导入导出（Miyo 原生 JSON / Postman Environment） ----------
const exchanging = ref(false)
/** 导出时对敏感变量值脱敏（后端 redact；非敏感变量保持明文仍可回导）。 */
const exportRedact = ref(false)

const exportMenu = ref<InstanceType<typeof Menu> | null>(null)

/** 导出菜单项（Miyo JSON / Postman 为专有名词，两种语言显示原文）。 */
const EXPORT_FORMAT_ITEMS: MenuItem[] = [
  { key: 'miyo_json', label: 'Miyo JSON', icon: 'file' },
  { key: 'postman_json', label: 'Postman', icon: 'download' },
]

/** 导出选中环境：经目录选择框落盘；redact 开启时敏感值已由后端打码。 */
async function exportSelected(format: EnvExchangeFormat): Promise<void> {
  if (!selected.value || exchanging.value) return
  if (hasPending()) {
    toast.warning(t('envmgr.exportUnsavedWarn'))
  }
  exchanging.value = true
  try {
    const doc = await api.exportEnvironment(selected.value.id, format, exportRedact.value)
    const dir = await openDialog({ directory: true, title: t('envmgr.exportDirTitle') })
    if (!dir || Array.isArray(dir)) return
    const path = await join(dir, doc.suggested_name)
    await api.writeTextFile(path, doc.content)
    toast.success(t('envmgr.exportSuccess'), { message: doc.suggested_name })
    // 未脱敏导出的明文凭证提示
    if (doc.sensitive_count && !exportRedact.value) {
      toast.warning(t('envmgr.exportSensitive', { n: doc.sensitive_count }))
    }
  } catch (err) {
    toast.error(t('envmgr.exportFail'), { message: errorText(err) })
  } finally {
    exchanging.value = false
  }
}

/** 打开导出格式菜单（需先选中环境）。 */
function openExportMenu(event: MouseEvent): void {
  if (!selected.value || exchanging.value) return
  exportMenu.value?.openAt(event.currentTarget as HTMLElement, EXPORT_FORMAT_ITEMS, 'left')
}

function onExportSelect(item: MenuItem): void {
  void exportSelected(item.key as EnvExchangeFormat)
}

const importOpen = ref(false)
const importText = ref('')
const importPreview = ref<ImportedEnv | null>(null)
const importError = ref('')
const importing = ref(false)

function openImport(): void {
  importText.value = ''
  importPreview.value = null
  importError.value = ''
  importOpen.value = true
}

async function importFromFile(): Promise<void> {
  try {
    const file = await openDialog({ multiple: false, title: t('envmgr.importFileTitle') })
    if (!file || Array.isArray(file)) return
    importText.value = await api.readTextFile(file)
    await previewImport()
  } catch (err) {
    importError.value = errorText(err)
  }
}

async function previewImport(): Promise<void> {
  importError.value = ''
  importPreview.value = null
  if (!importText.value.trim()) {
    importError.value = t('envmgr.importEmptyError')
    return
  }
  importing.value = true
  try {
    importPreview.value = await api.importEnvironment(importText.value, effectiveProjectId.value)
  } catch (err) {
    importError.value = errorText(err)
  } finally {
    importing.value = false
  }
}

/** 确认导入：重名自动加「导入」后缀，以新环境落库并选中（归属当前管理的项目）。 */
async function confirmImport(): Promise<void> {
  const preview = importPreview.value
  if (!preview || importing.value) return
  importing.value = true
  try {
    const taken = new Set(envs.value.map((e) => e.name))
    let name = preview.name
    if (taken.has(name)) name = `${name}${t('envmgr.importDupSuffix')}`
    let n = 2
    while (taken.has(name)) {
      name = `${preview.name}${t('envmgr.importDupSuffixN', { n })}`
      n += 1
    }
    const now = new Date().toISOString()
    const env: Environment = {
      id: crypto.randomUUID(),
      project_id: effectiveProjectId.value,
      name,
      base_url: preview.base_url,
      variables: preview.variables,
      created_at: now,
      updated_at: now,
    }
    const saved = detached.value
      ? await store.saveEnvironmentRemote(env)
      : await store.updateEnvironment(env, { silent: true })
    envs.value.push(saved)
    select(saved)
    importOpen.value = false
    toast.success(t('envmgr.importSuccess', { name: saved.name }), {
      message: t('envmgr.importSummary', {
        vars: preview.variables.length,
        format: preview.format,
      }),
    })
  } catch (err) {
    importError.value = errorText(err)
  } finally {
    importing.value = false
  }
}

// ---------- 变量行增删 / 批量合并（EnvVarTable 事件 → 父级执行，按作用域写入） ----------
function newVarRow(): EnvironmentVariable {
  return { key: '', remote_value: '', local_value: '', enabled: true, description: null, }
}

/** 当前变量表归属：env 作用域写选中环境，global 作用域写全局变量副本。 */
function currentVarRows(): EnvironmentVariable[] {
  return scope.value === 'global' ? globalVars.value : (selected.value?.variables ?? [])
}

function markVarScopeDirty(): void {
  if (scope.value === 'global') globalDirty.value = true
  else markDirty()
}

function onVarAdd(): void {
  currentVarRows().push(newVarRow())
  markVarScopeDirty()
}

function onVarRemove(index: number): void {
  currentVarRows().splice(index, 1)
  markVarScopeDirty()
}

/** 批量导入合并：命中已有 key 就地覆盖远程值，其余追加；与本组件置脏口径一致。 */
function onVarApplyBatch(rows: { key: string; remote_value: string }[]): void {
  const target = currentVarRows()
  let n = 0
  for (const r of rows) {
    const existing = target.find((v) => v.key.trim() === r.key)
    if (existing) existing.remote_value = r.remote_value
    else target.push({ ...newVarRow(), key: r.key, remote_value: r.remote_value })
    n += 1
  }
  markVarScopeDirty()
  toast.success(t('envmgr.batchImportSuccess', { n }))
}

// ---------- 编辑标记与保存 ----------
/** 实际编辑标记：清除「新建未编辑」状态，此后关闭/切换恢复未保存确认流程。 */
function markDirty(): void {
  dirty.value = true
  newEnvPristine = false
}

function onAnyChange(): void {
  markDirty()
}

/** 已启用变量数（详情区第二层计数）。 */
function enabledCount(): number {
  return selected.value?.variables.filter((v) => v.enabled).length ?? 0
}

/** 敏感变量数（显式标注或键名启发式，与表格掩码口径一致）。 */
function sensitiveCount(): number {
  return selected.value?.variables.filter((v) => resolveSensitive(v)).length ?? 0
}

/** Base URL 格式校验（实时）：空值合法，其余须 http(s):// 或 {{变量}} 开头。 */
const baseUrlError = computed(() => validateBaseUrl(selected.value?.base_url ?? ''))

/** 任一作用域存在未保存修改（保存条徽标 / 保存键启用）。 */
const anyDirty = computed(() => dirty.value || globalDirty.value || paramsDirty.value)

/** 落库当前作用域的修改；返回是否保存成功（供「保存并继续」判断是否执行后续动作）。 */
async function save(): Promise<boolean> {
  if (busy.value) return false
  commitRename()
  if (scope.value === 'params') {
    busy.value = true
    try {
      const normalized: GlobalParam[] = globalParams.value
        .filter((p) => p.key.trim() !== '')
        .map((p) => ({ ...p, key: p.key.trim() }))
      await store.saveGlobalParams(normalized)
      globalParams.value = deepClone(store.globalParams)
      paramsDirty.value = false
      confirmLeave.value = false
      toast.success(t('envmgr.globalParamsSaved'))
      return true
    } catch (err) {
      toast.error(t('envmgr.globalParamsSaveFail'), { message: errorText(err) })
      return false
    } finally {
      busy.value = false
    }
  }
  if (scope.value === 'global') {
    busy.value = true
    try {
      const normalized: EnvironmentVariable[] = globalVars.value
        .filter((v) => v.key.trim() !== '')
        .map((v) => ({ ...v, key: v.key.trim() }))
      await store.saveGlobalVariables(normalized)
      globalVars.value = deepClone(store.globalVariables)
      globalDirty.value = false
      confirmLeave.value = false
      toast.success(t('envmgr.globalVarsSaved'))
      return true
    } catch (err) {
      toast.error(t('envmgr.globalVarsSaveFail'), { message: errorText(err) })
      return false
    } finally {
      busy.value = false
    }
  }
  if (!selected.value) return false
  const name = selected.value.name.trim()
  if (!name) {
    toast.warning(t('envmgr.nameRequired'))
    return false
  }
  const env = selected.value
  const normalizedBase = normalizeBaseUrl(env.base_url)
  const normalizedVariables: EnvironmentVariable[] = env.variables
    .filter((v) => v.key.trim() !== '')
    .map((v) => ({ ...v, key: v.key.trim() }))
  busy.value = true
  try {
    const payload = {
      ...env,
      name,
      base_url: normalizedBase,
      variables: normalizedVariables,
    }
    let saved: Environment
    if (detached.value) {
      // 独立模式：直连 IPC 保存并回显校验，不回写工作区项目列表
      saved = await store.saveEnvironmentRemote(payload)
      envs.value = envs.value.map((e) => (e.id === saved.id ? deepClone(saved) : e))
    } else {
      saved = await store.updateEnvironment(payload, { silent: true })
      envs.value = [...store.environments]
    }
    selected.value = deepClone(saved)
    dirty.value = false
    newEnvPristine = false
    confirmLeave.value = false
    toast.success(t('envmgr.saved', { name: saved.name }))
    return true
  } catch (err) {
    toast.error(t('envmgr.saveFail'), { message: errorText(err) })
    return false
  } finally {
    busy.value = false
  }
}

/** 取消（弹窗模式专属）：重置本地副本并请求外壳关闭。 */
function cancel(): void {
  guard(() => {
    initState()
    emit('close')
  })
}

defineExpose({
  /** 是否存在未保存修改（设置页页签 / 关闭守卫用）。 */
  hasPending,
  /** 关闭守卫：有未保存修改时弹三选确认并返回 false。 */
  guardClose,
  /** 有未保存修改时挂起动作弹三选确认，否则直接执行。 */
  guardAction: guard,
  /** 显式刷新（左栏刷新按钮 / 父级协作）。 */
  refresh,
})
</script>

<template>
  <div class="em" :class="{ 'em-embedded': embedded }">
    <div class="em-body">
      <!-- ============ 左侧 Sidebar ============ -->
      <aside class="em-side">
        <div class="em-group">
          <div class="em-group-title">{{ t('envmgr.groupGlobal') }}</div>
          <div
            v-for="item in globalItems"
            :key="item.key"
            class="em-global-row"
            :class="{
              active:
                (item.key === 'global_variables' && scope === 'global') ||
                (item.key === 'global_params' && scope === 'params'),
              disabled: !item.enabled,
            }"
            :title="globalItemTitle(item)"
            @click="
              item.enabled &&
                (item.key === 'global_params' ? guard(selectParams) : guard(selectGlobal))
            "
          >
            <span class="edot ed-global"></span>
            <span class="em-global-name">{{ t(item.labelKey) }}</span>
            <span v-if="!item.enabled" class="em-global-soon">{{ t('envmgr.soon') }}</span>
            <span v-else-if="item.key === 'global_variables'" class="em-global-count">
              {{ store.globalVariables.filter((v) => v.enabled).length }}
            </span>
            <span v-else-if="item.key === 'global_params'" class="em-global-count">
              {{ store.globalParams.filter((p) => p.enabled).length }}
            </span>
          </div>
        </div>

        <div class="em-group em-group-envs">
          <!-- 组头：标题 + 数量 + 刷新 / 新建（固定在环境列表旁） -->
          <div class="em-group-head">
            <div class="em-group-title">{{ t('envmgr.groupEnvs') }}</div>
            <span class="em-group-count">{{ envs.length }}</span>
            <span class="em-group-head-spacer"></span>
            <IconButton name="refresh" :size="12" :title="t('common.refresh')" @click="refresh" />
            <IconButton name="plus" :size="12" tone="accent" class="em-add" :title="t('envmgr.addEnv')" @click="addEnvironment" />
          </div>
          <div class="em-list-body">
            <!-- 首次加载 / 切项目：骨架占位；SWR 刷新期间保留旧列表不清空 -->
            <div v-if="envsLoading && !envs.length" class="em-side-skeleton">
              <Skeleton v-for="i in 3" :key="i" height="22px" />
            </div>
            <div
              v-for="env in envs"
              :key="env.id"
              class="em-row"
              :class="{ active: env.id === activeEnvId, sel: env.id === selected?.id }"
              @click="onRowClick(env)"
              @dblclick="onRowDblClick(env)"
            >
              <span
                class="edot"
                :class="env.id === activeEnvId ? 'ed-active' : `ed-${envColorClass(rowName(env))}`"
              ></span>
              <input
                v-if="editingId === env.id"
                :ref="setEditInput"
                v-model="editValue"
                class="miyo-input miyo-input-sm em-row-input"
                :placeholder="t('envmgr.namePh')"
                spellcheck="false"
                @click.stop
                @keydown.enter.prevent="commitRename"
                @keydown.esc.stop.prevent="cancelRename"
                @blur="commitRename"
              />
              <template v-else>
                <span class="em-row-name" v-tooltip-overflow="rowName(env)">{{ rowName(env) }}</span>
                <span v-if="envBaseUrl(env)" class="em-row-url">{{ envBaseUrl(env) }}</span>
                <span v-if="env.id === activeEnvId" class="em-row-active">{{ t('envmgr.current') }}</span>
                <IconButton
                  name="more-horizontal"
                  :size="12"
                  class="em-row-more"
                  :title="t('envmgr.more')"
                  @click.stop="openRowMenu(env, $event)"
                />
              </template>
            </div>
          </div>
          <!-- 导入 / 导出 + 脱敏选项：固定左侧底部 -->
          <div class="em-exchange-row">
            <button
              class="miyo-btn miyo-btn-sm"
              type="button"
              :disabled="!selected || exchanging"
              :title="t('envmgr.exportTitle')"
              @click="openExportMenu"
            >
              <Icon name="upload" :size="13" /> {{ t('envmgr.export') }}
            </button>
            <button
              class="miyo-btn miyo-btn-sm"
              type="button"
              :disabled="exchanging"
              :title="t('envmgr.importTitle')"
              @click="openImport"
            >
              <Icon name="download" :size="13" /> {{ t('envmgr.import') }}
            </button>
          </div>
          <label class="em-redact-row" :title="t('envmgr.exportRedactTip')">
            <input v-model="exportRedact" type="checkbox" />
            {{ t('envmgr.exportRedact') }}
          </label>
        </div>
      </aside>

      <!-- 导出格式菜单（Miyo JSON / Postman） -->
      <Menu ref="exportMenu" @select="onExportSelect" />
      <!-- 环境操作菜单（行 ⋯ / 详情头 ⋯） -->
      <Menu ref="rowMenu" @select="onMenuSelect" @confirm="onMenuConfirm" />

      <!-- ============ 右侧详情 ============ -->
      <section class="em-editor">
        <template v-if="scope === 'env' && selected">
          <!-- 第一层：名称 + 选中态 + 激活态 + 更多操作（选中 = 正在编辑；激活 = 请求执行使用） -->
          <div class="em-editor-head">
            <input
              v-model="selected.name"
              class="miyo-input em-name"
              :placeholder="t('envmgr.namePh')"
              spellcheck="false"
              @input="onAnyChange"
            />
            <span class="em-pill em-pill-editing">{{ t('envmgr.editing') }}</span>
            <span
              v-if="!detached"
              class="em-pill em-pill-active"
              :class="{ on: selected.id === activeEnvId }"
            >
              {{ selected.id === activeEnvId ? t('envmgr.statusActive') : t('envmgr.statusInactive') }}
            </span>
            <IconButton
              name="more-horizontal"
              :size="14"
              :title="t('envmgr.more')"
              @click="openSelectedMenu($event)"
            />
          </div>
          <!-- 第二层：变量计数（总数 / 已启用 / 敏感） -->
          <p class="em-editor-sub">
            {{
              t('envmgr.detailMeta', {
                total: selected.variables.length,
                enabled: enabledCount(),
                sensitive: sensitiveCount(),
              })
            }}
          </p>

          <!-- Base URL：标签与说明分行（说明完整展示不截断） -->
          <div class="em-section">
            <div class="em-section-head">
              <span class="em-section-title">{{ t('envmgr.sectionBaseUrl') }}</span>
            </div>
            <p class="em-section-desc">{{ t('envmgr.baseUrlDesc') }}</p>
            <div class="em-base-box">
              <input
                v-model="selected.base_url"
                class="miyo-input miyo-input-sm em-base-input"
                :class="{ invalid: baseUrlError }"
                :placeholder="t('envmgr.baseUrlPh')"
                spellcheck="false"
                @input="onAnyChange"
              />
              <p v-if="baseUrlError" class="em-base-error">{{ t(baseUrlError) }}</p>
            </div>
          </div>

          <!-- 环境变量表（6 列：名 / 远程 / 本地覆盖 / 类型 / 启用 / 操作） -->
          <div class="em-section em-section-vars">
            <div class="em-section-head">
              <span class="em-section-title">{{ t('envmgr.sectionVars') }}</span>
              <span class="em-section-hint">{{ t('envmgr.varsHint') }}</span>
            </div>
            <EnvVarTable
              :rows="selected.variables"
              :hide-sensitive="hideSensitive"
              @update:hide-sensitive="onHideSensitiveToggle"
              @change="onAnyChange"
              @add="onVarAdd"
              @remove="onVarRemove"
              @apply-batch="onVarApplyBatch"
            />
          </div>
        </template>

        <!-- 全局变量详情 -->
        <template v-else-if="scope === 'global'">
          <div class="em-editor-head">
            <span class="em-editor-title">{{ t('envmgr.globalVars') }}</span>
            <span class="em-editor-meta">
              {{ t('envmgr.globalVarsMeta', { n: globalVars.filter((v) => v.enabled).length }) }}
            </span>
          </div>
          <div class="em-section em-section-vars">
            <div class="em-section-head">
              <span class="em-section-title">{{ t('envmgr.varsTitle') }}</span>
              <span class="em-section-hint">{{ t('envmgr.globalVarsHint') }}</span>
            </div>
            <EnvVarTable
              :rows="globalVars"
              :hide-sensitive="hideSensitive"
              :key-placeholder="t('envmgr.globalVarKeyPh')"
              @update:hide-sensitive="onHideSensitiveToggle"
              @change="onGlobalChange"
              @add="onVarAdd"
              @remove="onVarRemove"
              @apply-batch="onVarApplyBatch"
            />
          </div>
        </template>

        <!-- 全局参数详情 -->
        <template v-else-if="scope === 'params'">
          <div class="em-editor-head">
            <span class="em-editor-title">{{ t('envmgr.globalParams') }}</span>
            <span class="em-editor-meta">
              {{ t('envmgr.globalParamsMeta', { n: globalParams.filter((p) => p.enabled).length }) }}
            </span>
          </div>
          <div class="em-section em-section-vars">
            <div class="em-section-head">
              <span class="em-section-title">{{ t('envmgr.paramsTitle') }}</span>
              <span class="em-section-hint">{{ t('envmgr.globalParamsHint') }}</span>
            </div>
            <div class="em-table">
              <div class="em-th em-th-param">
                <span class="em-col-key">{{ t('envmgr.colParamKey') }}</span>
                <span class="em-col-remote">{{ t('envmgr.colValue') }}</span>
                <span class="em-col-loc">{{ t('envmgr.colLocation') }}</span>
                <span class="em-col-enabled">{{ t('envmgr.colEnabled') }}</span>
                <span class="em-col-op"></span>
              </div>
              <div
                v-for="(p, i) in globalParams"
                :key="rowKey(p)"
                class="em-tr em-tr-param"
                :class="{ off: !p.enabled }"
              >
                <input
                  v-model="p.key"
                  class="miyo-input miyo-input-sm em-col-key"
                  :placeholder="t('envmgr.paramKeyPh')"
                  spellcheck="false"
                  @input="onParamsChange"
                />
                <input
                  v-model="p.value"
                  class="miyo-input miyo-input-sm em-col-remote"
                  :placeholder="t('envmgr.paramValuePh')"
                  spellcheck="false"
                  @input="onParamsChange"
                />
                <CustomSelect
                  :model-value="p.location"
                  :options="locationOptions"
                  size="sm"
                  class="em-col-loc"
                  @update:model-value="onParamLocation(p, $event)"
                />
                <input
                  v-model="p.enabled"
                  type="checkbox"
                  class="em-col-enabled"
                  :checked="p.enabled"
                  :aria-label="t('envmgr.colEnabled')"
                  @change="onParamsChange"
                />
                <IconButton
                  name="trash"
                  :size="13"
                  tone="danger"
                  :title="t('envmgr.deleteParam')"
                  class="em-col-op"
                  @click="removeGlobalParam(i)"
                />
              </div>
              <button class="miyo-btn miyo-btn-sm em-add-var" type="button" @click="addGlobalParam">
                <Icon name="plus" :size="13" /> {{ t('envmgr.addParam') }}
              </button>
            </div>
          </div>
        </template>
        <p v-else-if="envsLoading && !envs.length" class="em-empty">
          {{ t('common.loading') }}
        </p>
        <p v-else class="em-empty">
          {{ t('envmgr.empty') }}
        </p>
      </section>
    </div>

    <!-- 保存条：未保存徽标 + 保存（无修改禁用）；内嵌模式无取消 -->
    <div class="em-savebar">
      <span v-if="hasPending()" class="em-unsaved">
        <span class="em-unsaved-dot"></span>
        {{ t('envmgr.unsavedBadge') }}
      </span>
      <span class="em-savebar-spacer"></span>
      <button v-if="!embedded" class="miyo-btn" type="button" @click="cancel">
        {{ t('common.cancel') }}
      </button>
      <button
        class="miyo-btn miyo-btn-primary"
        type="button"
        :disabled="busy || !anyDirty"
        @click="save"
      >
        <Icon v-if="busy" name="refresh" :size="13" class="em-spin" />
        {{ busy ? t('envmgr.saving') : t('common.save') }}
      </button>
    </div>
  </div>

  <!-- 环境导入 -->
  <Modal
    v-if="importOpen"
    :open="importOpen"
    :title="t('envmgr.importModalTitle')"
    width="560px"
    @update:open="importOpen = $event"
    @close="importOpen = false"
  >
    <p class="em-import-hint">{{ t('envmgr.importHint') }}</p>
    <textarea
      v-model="importText"
      class="miyo-input em-import-input"
      spellcheck="false"
      placeholder='{"name": "prod", "values": [{"key": "base_url", "value": "https://…"}]}'
    ></textarea>
    <div class="em-import-actions">
      <button class="miyo-btn miyo-btn-sm" type="button" @click="importFromFile">
        <Icon name="folder" :size="13" /> {{ t('envmgr.importFromFile') }}
      </button>
      <button
        class="miyo-btn miyo-btn-sm miyo-btn-primary"
        type="button"
        :disabled="importing || !importText.trim()"
        @click="previewImport"
      >
        {{ importing ? t('envmgr.parsing') : t('envmgr.parsePreview') }}
      </button>
    </div>
    <p v-if="importError" class="em-import-error">{{ importError }}</p>
    <div v-if="importPreview" class="em-import-preview">
      <div class="em-import-row">
        <span class="em-import-label">{{ t('envmgr.previewName') }}</span>
        <span>{{ importPreview.name }}</span>
      </div>
      <div class="em-import-row">
        <span class="em-import-label">{{ t('envmgr.previewFormat') }}</span>
        <span>{{ importPreview.format === 'postman' ? 'Postman' : 'Miyo' }}</span>
      </div>
      <div class="em-import-row">
        <span class="em-import-label">{{ t('envmgr.previewContent') }}</span>
        <span>{{ t('envmgr.previewSummary', { vars: importPreview.variables.length }) }}</span>
      </div>
    </div>
    <template #footer>
      <button class="miyo-btn" type="button" @click="importOpen = false">{{ t('common.cancel') }}</button>
      <button
        class="miyo-btn miyo-btn-primary"
        type="button"
        :disabled="!importPreview || importing"
        @click="confirmImport"
      >
        {{ importing ? t('envmgr.importing') : t('envmgr.importAsNew') }}
      </button>
    </template>
  </Modal>

  <!-- 未保存修改确认：居中三选弹窗。closable=false 去掉 ✕，autofocus 落在首个按钮
       「保存并继续」上（Enter 直接触发）；Esc 关闭本弹窗 = 继续编辑。
       声明在最后，与其他弹窗嵌套时保持顶层。 -->
  <Modal
    :open="confirmLeave"
    :title="t('envmgr.unsavedTitle')"
    width="380px"
    :closable="false"
    @update:open="onConfirmToggle"
  >
    <div class="em-confirm">
      <p class="em-confirm-text">{{ t('envmgr.unsavedWarning') }}</p>
      <div class="em-confirm-actions">
        <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="saveAndProceed">
          {{ t('envmgr.saveAndContinue') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="discardChanges">
          {{ t('envmgr.discard') }}
        </button>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="keepEditing">
          {{ t('envmgr.keepEditing') }}
        </button>
      </div>
    </div>
  </Modal>
</template>

<style scoped>
.em {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: min(640px, 76vh);
}

/* 内嵌模式（设置 → 环境页签）：填满父容器 */
.em-embedded {
  height: 100%;
  min-height: 0;
}

/* ---- 未保存修改确认弹窗 ---- */
.em-confirm {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.em-confirm-text {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-2);
}

.em-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

/* ---- 双栏主区 ---- */
.em-body {
  display: flex;
  gap: 12px;
  min-height: 0;
  flex: 1;
}

/* ============ 左侧 Sidebar ============ */
.em-side {
  width: 264px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  padding: 10px;
  overflow: hidden;
}

.em-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.em-group-envs {
  flex: 1;
  min-height: 0;
}

.em-group-title {
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  padding: 4px 6px;
}

/* 环境组头：标题 + 数量 + 刷新 / 新建 */
.em-group-head {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 2px 2px 0;
}

.em-group-head .em-group-title {
  padding: 0 2px;
}

.em-group-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.em-group-head-spacer {
  flex: 1;
}

.em-global-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius);
  opacity: 0.6;
}

.em-global-row.active {
  background: var(--accent-tint);
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  opacity: 1;
  cursor: pointer;
}

.em-global-row:not(.disabled) {
  opacity: 1;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    opacity var(--dur) var(--ease);
}

.em-global-row:not(.disabled):hover {
  background: var(--bg-hover);
}

.em-global-name {
  font-size: 12.5px;
  color: var(--text-2);
}

.em-global-count {
  margin-left: auto;
  font-size: var(--fs-xxs);
  color: var(--accent);
  background: var(--accent-tint);
  border-radius: 999px;
  padding: 0 6px;
  line-height: 1.6;
}

.em-global-soon {
  margin-left: auto;
  font-size: var(--fs-xxs);
  letter-spacing: 0.04em;
  color: var(--text-3);
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 0 6px;
  line-height: 1.5;
}

.em-list-body {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 加载骨架 */
.em-side-skeleton {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px 4px;
}

.em-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius);
  border: 1px solid transparent;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}

.em-row:hover {
  background: var(--bg-hover);
}

.em-row.sel {
  background: var(--bg-active);
}

/* 当前激活环境：accent 底 + 绿点（见 .ed-active）+ 当前徽标 */
.em-row.active {
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}

.em-row-name {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-1);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 行内重命名输入框：占满行剩余宽度 */
.em-row-input {
  flex: 1;
  min-width: 0;
}

.em-row-url {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  max-width: 70px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex-shrink: 1;
}

.em-row-active {
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--success);
  background: var(--success-tint);
  border-radius: 999px;
  padding: 0 6px;
  flex-shrink: 0;
}

/* 行 ⋯ 菜单按钮：hover / 选中行浮现 */
.em-row-more {
  margin-left: auto;
  opacity: 0;
  width: 22px;
  height: 22px;
  flex-shrink: 0;
}

.em-row:hover .em-row-more,
.em-row.sel .em-row-more {
  opacity: 1;
}

/* 导入导出行 */
.em-exchange-row {
  display: flex;
  gap: 6px;
  margin-top: 6px;
}

.em-exchange-row .miyo-btn {
  flex: 1;
  min-width: 0;
  justify-content: center;
}

/* 脱敏导出选项 */
.em-redact-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  padding: 0 2px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  cursor: pointer;
  user-select: none;
}

.em-redact-row input {
  accent-color: var(--accent);
  margin: 0;
}

/* 环境导入弹窗 */
.em-import-hint {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--text-2);
}
.em-import-input {
  width: 100%;
  min-height: 110px;
  font-family: var(--font-mono);
  font-size: 12px;
  resize: vertical;
}
.em-import-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
.em-import-error {
  margin: 8px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: var(--danger-tint);
  color: var(--danger);
  font-size: 12px;
}
.em-import-preview {
  margin-top: 10px;
  padding: 10px 12px;
  border-radius: var(--radius);
  background: var(--bg-card);
  border: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.em-import-row {
  display: flex;
  gap: 10px;
  font-size: 12.5px;
}
.em-import-label {
  width: 44px;
  flex-shrink: 0;
  color: var(--text-3);
}

/* ============ 右侧详情 ============ */
/* 结构：头部 / Base URL 固定，变量表区域 flex 拉伸 + 内部滚动（消灭弹窗底部空白） */
.em-editor {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow: hidden;
}

.em-editor-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* 第二层：变量计数行（总数 / 已启用 / 敏感） */
.em-editor-sub {
  margin: -6px 2px 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* 状态 pill：编辑中（选中态，中性描边）/ 当前生效（激活态，绿） */
.em-pill {
  font-size: var(--fs-xxs);
  line-height: 1.6;
  padding: 0 8px;
  border-radius: 999px;
  flex-shrink: 0;
  white-space: nowrap;
}

.em-pill-editing {
  border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
  color: var(--accent);
  background: var(--accent-tint);
}

.em-pill-active {
  border: 1px solid var(--border);
  color: var(--text-3);
}

.em-pill-active.on {
  color: var(--success);
  background: var(--success-tint);
  border-color: transparent;
}

.em-name {
  flex: 1;
  height: var(--h-md);
  min-width: 0;
}

.em-editor-title {
  flex: 1;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-1);
}

.em-editor-meta {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  flex-shrink: 0;
  white-space: nowrap;
}

/* 说明行：完整展示允许换行，不截断 */
.em-section-desc {
  margin: -2px 2px 0;
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}

.em-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.em-section-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 0 2px;
}

.em-section-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-1);
}

.em-section-hint {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 变量表区域：占满剩余高度（表格内部滚动） */
.em-section-vars {
  flex: 1;
  min-height: 0;
}

/* Base URL：单输入 + 校验错误行 */
.em-base-box {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.em-base-input {
  width: 100%;
  font-family: var(--font-mono);
}

.em-base-input.invalid {
  border-color: var(--danger);
}

.em-base-error {
  margin: 0 2px;
  font-size: var(--fs-xxs);
  color: var(--danger);
}

.em-empty {
  margin: auto;
  font-size: 12.5px;
  color: var(--text-3);
  text-align: center;
}

/* ---- 保存条 ---- */
.em-savebar {
  display: flex;
  align-items: center;
  gap: 8px;
  border-top: 1px solid var(--border);
  padding-top: 10px;
  flex-shrink: 0;
}

.em-savebar-spacer {
  flex: 1;
}

/* 未保存徽标：amber 圆点 + 文案 */
.em-unsaved {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--warning);
  background: var(--warning-tint);
  border-radius: 999px;
  padding: 2px 8px;
}

.em-unsaved-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
}

/* ---- 环境色点 ---- */
.edot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-3);
}
.ed-dev {
  background: var(--success);
}
.ed-test {
  background: var(--info);
}
.ed-staging {
  background: var(--warning);
}
.ed-prod {
  background: var(--orange);
}
.ed-global {
  background: var(--accent);
}
/* 当前激活环境：强制绿色（后定义覆盖同名启发式色）+ 微光环 */
.edot.ed-active {
  background: var(--success);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 22%, transparent);
}
</style>
