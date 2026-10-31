<script setup lang="ts">
/**
 * EnvironmentManager：环境管理（大弹窗双栏）。
 *
 * 左侧 Sidebar：
 * - 「全局组」：全局变量 / 全局参数 / Vault Secrets（占位，后续版本启用）；
 * - 「环境组」：当前项目的环境列表（色点 + 名称 + Base URL），底部「+ 新建环境」。
 *
 * 右侧详情主面板：
 * - Header：环境名称编辑 + 摘要；
 * - Base URL：环境前置基址（可含 {{变量}}）；
 * - 环境变量表：变量名 | 远程值 | 本地值 | 启用 | 操作 —— 本地值优先覆盖远程值；
 * - 底部统一「保存 / 取消」变更控制；未保存时切换/关闭弹居中三选确认
 *   （保存并继续 / 放弃修改 / 继续编辑），「保存并继续」为默认键。
 *
 * 所有编辑作用于本地副本，保存时一次落库（store.updateEnvironment upsert）。
 */
import { computed, nextTick, ref, watch } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { envBaseUrl, envColorClass, normalizeBaseUrl } from '../utils/environment'
import { deepClone } from '../utils/clone'
import { rowKey } from '../utils/rowKey'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Menu, { type MenuItem } from './ui/Menu.vue'
import Modal from './ui/Modal.vue'
import Popconfirm from './ui/Popconfirm.vue'
import type {
  EnvExchangeFormat,
  Environment,
  EnvironmentVariable,
  GlobalParam,
  ImportedEnv,
} from '../types/miyoApi'

const props = defineProps<{
  open: boolean
  initialEnvId?: string | null
  createNew?: boolean
  /** 独立管理模式：管理指定项目（非当前工作区项目）的环境——首页设置进入。 */
  projectId?: string | null
  projectName?: string | null
}>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

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

/** 独立管理模式：目标项目不是当前工作区项目——数据自加载、保存不回写工作区列表。 */
const detached = computed(() => !!props.projectId && props.projectId !== store.project?.id)
/** 实际管理的项目 id：独立模式取 props，否则跟随工作区当前项目。 */
const effectiveProjectId = computed(() => (detached.value ? props.projectId : store.project?.id) ?? '')
/** 独立模式加载环境列表中。 */
const envsLoading = ref(false)

const activeEnvId = computed(() => (detached.value ? null : store.activeEnvId))

/** 弹窗标题：独立模式追加项目名，明确当前管理的是谁的环境。 */
const dialogTitle = computed(() =>
  detached.value && props.projectName
    ? `${t('settings.environments')} · ${props.projectName}`
    : t('settings.environments'),
)

/** 右侧详情面板作用域：'env' = 环境详情；'global' = 全局变量；'params' = 全局参数。 */
const scope = ref<'env' | 'global' | 'params'>('env')
/** 全局变量本地副本（作用域 global 时编辑此表，保存时整体落库）。 */
const globalVars = ref<EnvironmentVariable[]>([])
const globalDirty = ref(false)
/** 全局参数本地副本（作用域 params 时编辑此表，保存时整体落库）。 */
const globalParams = ref<GlobalParam[]>([])
const paramsDirty = ref(false)

/** 全局组：全局变量 / 全局参数已启用；Vault Secrets 仍为占位（置灰）。 */
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

function addGlobalVariable(): void {
  globalVars.value.push({
    key: '',
    remote_value: '',
    local_value: '',
    enabled: true,
    description: null,
  })
  globalDirty.value = true
}

function removeGlobalVariable(index: number): void {
  globalVars.value.splice(index, 1)
  globalDirty.value = true
}

function onGlobalChange(): void {
  globalDirty.value = true
}

function addGlobalParam(): void {
  globalParams.value.push({ key: '', value: '', enabled: true, location: 'header' })
  paramsDirty.value = true
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

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) {
      cancelRename()
      return
    }
    globalVars.value = deepClone(store.globalVariables)
    globalDirty.value = false
    globalParams.value = deepClone(store.globalParams)
    paramsDirty.value = false

    if (detached.value) {
      // 独立模式：目标项目的环境列表走 IPC 自加载，不读工作区 store（那是当前项目的列表）
      envsLoading.value = true
      envs.value = []
      select(null)
      try {
        const list = await api.listEnvironments(effectiveProjectId.value)
        if (!props.open) return // 加载期间已被关闭
        envs.value = list
      } catch (err) {
        if (!props.open) return
        toast.error(t('envmgr.loadFail'), {
          message: err instanceof Error ? err.message : String(err),
        })
      } finally {
        envsLoading.value = false
      }
      const preferred = props.initialEnvId
        ? envs.value.find((e) => e.id === props.initialEnvId)
        : undefined
      select(preferred ?? envs.value[0] ?? null)
    } else {
      // 环境表只做浅拷贝：左侧列表是纯读（渲染 + 点击 select 时才深克隆选中项），
      // 深克隆整表会把每个环境的变量/模块数组都复制一遍，纯属白费。
      envs.value = [...store.environments]
      const preferred = props.initialEnvId
        ? store.environments.find((e) => e.id === props.initialEnvId)
        : undefined
      const active = store.environments.find((e) => e.id === store.activeEnvId)
      select(preferred ?? active ?? store.environments[0] ?? null)
    }
    // 「新建环境」快捷入口：打开即建一条待保存的新环境（createNew 由调用方在关闭时复位）
    if (props.createNew) addEnvironment()
  },
)

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

/** 进入行内编辑：createAll=true（新建流程）全选默认名，直接输入即覆盖；否则光标置尾。 */
async function beginRename(env: Environment, selectAll = false): Promise<void> {
  if (editingId.value && editingId.value !== env.id) commitRename()
  editingId.value = env.id
  editValue.value = rowName(env)
  // 双拍 nextTick：首拍前 Modal 的 autofocus 会把焦点抢到关闭按钮，第二拍才轮到输入框
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
 * 与右侧 .em-name 完全同源；切换/丢弃时 select() 重取 envs 即自然回滚，
 * 也避免浅拷贝数组被原地改写穿 store。
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

// ---------- 环境导入导出（Miyo 原生 JSON / Postman Environment） ----------
// 导出格式收敛进「导出」按钮的下拉菜单（两种格式都完整展示，不再用角落里被挤到截断的下拉）；
// 导入无需选格式：import_environment 自动识别 Miyo / Postman。
const exchanging = ref(false)

/** 导出菜单项（Miyo JSON / Postman 为专有名词，两种语言显示原文）。 */
const EXPORT_FORMAT_ITEMS: MenuItem[] = [
  { key: 'miyo_json', label: 'Miyo JSON', icon: 'file' },
  { key: 'postman_json', label: 'Postman', icon: 'download' },
]

const exportMenu = ref<InstanceType<typeof Menu> | null>(null)

/** 导出选中环境：经目录选择框落盘（变量以明文落盘，与备份 JSON 口径一致）。 */
async function exportSelected(format: EnvExchangeFormat): Promise<void> {
  if (!selected.value || exchanging.value) return
  if (dirty.value) {
    toast.warning(t('envmgr.exportUnsavedWarn'))
  }
  exchanging.value = true
  try {
    const doc = await api.exportEnvironment(selected.value.id, format)
    const dir = await openDialog({ directory: true, title: t('envmgr.exportDirTitle') })
    if (!dir || Array.isArray(dir)) return
    const path = await join(dir, doc.suggested_name)
    await api.writeTextFile(path, doc.content)
    toast.success(t('envmgr.exportSuccess'), { message: doc.suggested_name })
    // 明文凭证提示：导出需可回导不做脱敏，但提醒用户这份文件的敏感度。
    if (doc.sensitive_count) {
      toast.warning(t('envmgr.exportSensitive', { n: doc.sensitive_count }))
    }
  } catch (err) {
    toast.error(t('envmgr.exportFail'), { message: err instanceof Error ? err.message : String(err) })
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
    importError.value = err instanceof Error ? err.message : String(err)
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
    importPreview.value = await api.importEnvironment(importText.value, store.project?.id ?? '')
  } catch (err) {
    importError.value = err instanceof Error ? err.message : String(err)
  } finally {
    importing.value = false
  }
}

/** 确认导入：重名自动加「导入」后缀，以新环境落库并选中。 */
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
      project_id: store.project?.id ?? '',
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
    importError.value = err instanceof Error ? err.message : String(err)
  } finally {
    importing.value = false
  }
}

// ---------- 编辑标记 ----------
/** 实际编辑标记：清除「新建未编辑」状态，此后关闭/切换恢复未保存确认流程。 */
function markDirty(): void {
  dirty.value = true
  newEnvPristine = false
}

// ---------- 环境变量 ----------
function addVariable(): void {
  const env = selected.value
  if (!env) return
  env.variables.push({
    key: '',
    remote_value: '',
    local_value: '',
    enabled: true,
    description: null,
  })
  markDirty()
}

function removeVariable(index: number): void {
  const env = selected.value
  if (!env) return
  env.variables.splice(index, 1)
  markDirty()
}

function onAnyChange(): void {
  markDirty()
}

function variablesCount(): number {
  return selected.value?.variables.filter((v) => v.enabled).length ?? 0
}

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
      toast.error(t('envmgr.globalParamsSaveFail'), { message: err instanceof Error ? err.message : String(err) })
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
      toast.error(t('envmgr.globalVarsSaveFail'), { message: err instanceof Error ? err.message : String(err) })
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
    toast.error(t('envmgr.saveFail'), { message: err instanceof Error ? err.message : String(err) })
    return false
  } finally {
    busy.value = false
  }
}

function cancel(): void {
  guard(() => {
    envs.value = [...store.environments]
    const active = store.environments.find((e) => e.id === store.activeEnvId)
    select(active ?? store.environments[0] ?? null)
    globalVars.value = deepClone(store.globalVariables)
    globalDirty.value = false
    globalParams.value = deepClone(store.globalParams)
    paramsDirty.value = false
    emit('update:open', false)
  })
}

async function remove(env: Environment): Promise<void> {
  const persisted = store.environments.some((e) => e.id === env.id)
  try {
    if (persisted) await store.deleteEnvironment(env.id, { silent: true })
    const idx = envs.value.findIndex((e) => e.id === env.id)
    envs.value = envs.value.filter((e) => e.id !== env.id)
    if (editingId.value === env.id) cancelRename()
    if (selected.value?.id === env.id) {
      select(envs.value[Math.min(idx, Math.max(envs.value.length - 1, 0))] ?? null)
    }
    toast.success(t('envmgr.deleted', { name: env.name }))
  } catch (err) {
    toast.error(t('envmgr.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}
</script>

<template>
  <Modal
    :open="open"
    :title="dialogTitle"
    width="min(1120px, 94vw)"
    :guard-close="guardClose"
    @update:open="emit('update:open', $event)"
  >
    <div class="em">
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
            <div class="em-group-title">{{ t('envmgr.groupEnvs') }}</div>
            <div class="em-list-body">
              <div v-if="envsLoading" class="em-side-hint">{{ t('common.loading') }}</div>
              <div
                v-for="env in envs"
                :key="env.id"
                class="em-row"
                :class="{ active: env.id === activeEnvId, sel: env.id === selected?.id }"
                @click="onRowClick(env)"
                @dblclick="onRowDblClick(env)"
              >
                <span class="edot" :class="`ed-${envColorClass(rowName(env))}`"></span>
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
                  <Popconfirm
                    :title="t('envmgr.deleteConfirmTitle', { name: env.name })"
                    :description="t('confirm.undone')"
                    :confirm-text="t('common.delete')"
                    @confirm="remove(env)"
                  >
                    <IconButton name="trash" :size="12" tone="danger" class="em-row-del" :title="t('common.delete')" />
                  </Popconfirm>
                </template>
              </div>
            </div>
            <button class="miyo-btn miyo-btn-sm em-add" type="button" @click="addEnvironment">
              <Icon name="plus" :size="13" /> {{ t('envmgr.addEnv') }}
            </button>
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
          </div>
        </aside>

        <!-- 导出格式菜单（Miyo JSON / Postman） -->
        <Menu ref="exportMenu" @select="onExportSelect" />

        <!-- ============ 右侧详情 ============ -->
        <section class="em-editor">
          <template v-if="scope === 'env' && selected">
            <div class="em-editor-head">
              <input
                v-model="selected.name"
                class="miyo-input em-name"
                :placeholder="t('envmgr.namePh')"
                spellcheck="false"
                @input="onAnyChange"
              />
               <span class="em-editor-meta">
                 {{ t('envmgr.editorMeta', { vars: variablesCount() }) }}
               </span>
            </div>

            <!-- 前置 URL 配置表 -->
            <div class="em-section">
              <div class="em-section-head">
                <span class="em-section-title">{{ t('envmgr.sectionBaseUrl') }}</span>
                <span class="em-section-hint">
                  {{ t('envmgr.baseUrlHint') }}
                </span>
              </div>
              <div class="em-table">
                <div class="em-tr em-tr-base">
                  <input
                    v-model="selected.base_url"
                    class="miyo-input miyo-input-sm em-base-input"
                    :placeholder="t('envmgr.baseUrlPh')"
                    spellcheck="false"
                    @input="onAnyChange"
                  />
                </div>
              </div>
            </div>

            <!-- 环境变量表 -->
            <div class="em-section">
              <div class="em-section-head">
                <span class="em-section-title">{{ t('envmgr.sectionVars') }}</span>
                <span class="em-section-hint">{{ t('envmgr.varsHint') }}</span>
              </div>
              <div class="em-table">
                <div class="em-th em-th-var">
                  <span class="em-col-key">{{ t('envmgr.colKey') }}</span>
                  <span class="em-col-remote">{{ t('envmgr.colRemote') }}</span>
                  <span class="em-col-local">{{ t('envmgr.colLocal') }}</span>
                  <span class="em-col-enabled">{{ t('envmgr.colEnabled') }}</span>
                  <span class="em-col-op"></span>
                </div>
                <div
                  v-for="(v, i) in selected.variables"
                  :key="rowKey(v)"
                  class="em-tr em-tr-var"
                  :class="{ off: !v.enabled }"
                >
                  <input
                    v-model="v.key"
                    class="miyo-input miyo-input-sm em-col-key"
                    :placeholder="t('envmgr.varKeyPh')"
                    spellcheck="false"
                    @input="onAnyChange"
                  />
                  <input
                    v-model="v.remote_value"
                    class="miyo-input miyo-input-sm em-col-remote"
                    :placeholder="t('envmgr.varRemotePh')"
                    spellcheck="false"
                    @input="onAnyChange"
                  />
                  <input
                    v-model="v.local_value"
                    class="miyo-input miyo-input-sm em-col-local"
                    :placeholder="t('envmgr.varLocalPh')"
                    spellcheck="false"
                    @input="onAnyChange"
                  />
                  <input
                    v-model="v.enabled"
                    type="checkbox"
                    class="em-col-enabled"
                    :checked="v.enabled"
                    :aria-label="t('envmgr.colEnabled')"
                    @change="onAnyChange"
                  />
                  <IconButton
                    name="trash"
                    :size="13"
                    tone="danger"
                    :title="t('envmgr.deleteVar')"
                    class="em-col-op"
                    @click="removeVariable(i)"
                  />
                </div>
                <button class="miyo-btn miyo-btn-sm em-add-var" type="button" @click="addVariable">
                  <Icon name="plus" :size="13" /> {{ t('envmgr.addVar') }}
                </button>
              </div>
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
            <div class="em-section">
              <div class="em-section-head">
                <span class="em-section-title">{{ t('envmgr.varsTitle') }}</span>
                <span class="em-section-hint">{{ t('envmgr.globalVarsHint') }}</span>
              </div>
              <div class="em-table">
                <div class="em-th em-th-var">
                  <span class="em-col-key">{{ t('envmgr.colKey') }}</span>
                  <span class="em-col-remote">{{ t('envmgr.colRemote') }}</span>
                  <span class="em-col-local">{{ t('envmgr.colLocal') }}</span>
                  <span class="em-col-enabled">{{ t('envmgr.colEnabled') }}</span>
                  <span class="em-col-op"></span>
                </div>
                <div
                  v-for="(v, i) in globalVars"
                  :key="rowKey(v)"
                  class="em-tr em-tr-var"
                  :class="{ off: !v.enabled }"
                >
                  <input
                    v-model="v.key"
                    class="miyo-input miyo-input-sm em-col-key"
                    :placeholder="t('envmgr.globalVarKeyPh')"
                    spellcheck="false"
                    @input="onGlobalChange"
                  />
                  <input
                    v-model="v.remote_value"
                    class="miyo-input miyo-input-sm em-col-remote"
                    :placeholder="t('envmgr.varRemotePh')"
                    spellcheck="false"
                    @input="onGlobalChange"
                  />
                  <input
                    v-model="v.local_value"
                    class="miyo-input miyo-input-sm em-col-local"
                    :placeholder="t('envmgr.varLocalPh')"
                    spellcheck="false"
                    @input="onGlobalChange"
                  />
                  <input
                    v-model="v.enabled"
                    type="checkbox"
                    class="em-col-enabled"
                    :checked="v.enabled"
                    :aria-label="t('envmgr.colEnabled')"
                    @change="onGlobalChange"
                  />
                  <IconButton
                    name="trash"
                    :size="13"
                    tone="danger"
                    :title="t('envmgr.deleteVar')"
                    class="em-col-op"
                    @click="removeGlobalVariable(i)"
                  />
                </div>
                <button
                  class="miyo-btn miyo-btn-sm em-add-var"
                  type="button"
                  @click="addGlobalVariable"
                >
                  <Icon name="plus" :size="13" /> {{ t('envmgr.addVar') }}
                </button>
              </div>
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
            <div class="em-section">
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
                  <select
                    v-model="p.location"
                    class="miyo-input miyo-input-sm em-col-loc"
                    @change="onParamsChange"
                  >
                    <option value="header">{{ t('envmgr.locHeader') }}</option>
                    <option value="query">{{ t('envmgr.locQuery') }}</option>
                  </select>
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
          <p v-else class="em-empty">
            {{ t('envmgr.empty') }}
          </p>
        </section>
      </div>
    </div>

    <template #footer>
      <button class="miyo-btn" type="button" @click="cancel">{{ t('common.cancel') }}</button>
      <button
        class="miyo-btn miyo-btn-primary"
        type="button"
        :disabled="busy || (!dirty && !globalDirty && !paramsDirty)"
        @click="save"
      >
        {{ busy ? t('envmgr.saving') : t('common.save') }}
      </button>
    </template>
  </Modal>

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
  gap: 14px;
  min-height: 60vh;
  max-height: 70vh;
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
  gap: 14px;
  min-height: 0;
  flex: 1;
}

/* ============ 左侧 Sidebar ============ */
.em-side {
  width: 240px;
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

.em-global-name {
  font-size: 12.5px;
  color: var(--text-2);
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

/* 独立模式加载提示 */
.em-side-hint {
  padding: 10px 6px;
  font-size: 12px;
  color: var(--text-3);
  text-align: center;
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

/* 行内重命名输入框：占满行剩余宽度，替换名称/url/徽标/删除的位置 */
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
  color: var(--accent);
  background: var(--accent-tint);
  border-radius: 999px;
  padding: 0 6px;
  flex-shrink: 0;
}

.em-row-del {
  margin-left: auto;
  opacity: 0;
  width: 22px;
  height: 22px;
}

.em-row:hover .em-row-del,
.em-row.sel .em-row-del {
  opacity: 1;
}

.em-add {
  margin-top: 4px;
  width: 100%;
  border-style: dashed;
  color: var(--text-2);
}

/* 导入导出行：格式选择 + 导入/导出按钮 */
.em-exchange-row {
  display: flex;
  gap: 6px;
  margin-top: 6px;
}

/* 导出 / 导入等宽铺满，替代原先被挤到截断的格式下拉 */
.em-exchange-row .miyo-btn {
  flex: 1;
  min-width: 0;
  justify-content: center;
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
.em-editor {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  overflow-y: auto;
}

.em-editor-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.em-name {
  flex: 1;
  height: var(--h-md);
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
}

.em-table {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  padding: 6px;
}

.em-th {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 8px 6px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.em-tr {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
}

.em-tr.off {
  opacity: 0.45;
}

/* Base URL 行（单输入） */
.em-tr-base {
  padding: 2px 0;
}

.em-base-input {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

/* 变量表列 */
.em-th-var {
  margin-bottom: 4px;
}

.em-th-param {
  margin-bottom: 4px;
}

.em-col-loc {
  width: 96px;
  flex-shrink: 0;
  font-size: 12px;
}

.em-col-key {
  width: 22%;
  min-width: 0;
}

.em-col-remote {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

.em-col-local {
  width: 24%;
  min-width: 0;
  font-family: var(--font-mono);
}

.em-col-enabled {
  width: 42px;
  flex-shrink: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.em-col-op {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
}

.em-add-var {
  margin-top: 4px;
  border-style: dashed;
  color: var(--text-2);
}

.em-empty {
  margin: auto;
  font-size: 12.5px;
  color: var(--text-3);
  text-align: center;
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
</style>
