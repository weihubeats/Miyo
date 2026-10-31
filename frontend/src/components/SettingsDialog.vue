<script setup lang="ts">
/**
 * SettingsDialog：设置弹框（Linear / Raycast 风格暗黑双栏设置面板）。
 *
 * 左栏：轻量 Menu List 导航（扁平行高 + 左侧紫色指示条）；
 * 右栏：卡片化设置组，Tab 切换淡入；简单项改动即自动保存。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { relaunch } from '@tauri-apps/plugin-process'
import { copyText } from '../utils/clipboard'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { lazyComponent } from '../composables/lazyComponent'
import { useThemeStore, type ThemeMode } from '../stores/theme'
import { useLocaleStore, type LocaleMode } from '../stores/locale'
import { useWorkspaceStore, FALLBACK_GLOBAL_TIMEOUT_MS } from '../stores/workspace'
import {
  SHORTCUT_DEFAULTS,
  bindingLabel,
  defaultBindingOf,
  findBindingConflict,
  isShortcutCustomized,
  resetAllShortcutBindings,
  resetShortcutBinding,
  setShortcutBinding,
  shortcutBindingsTick,
  type ShortcutBinding,
} from '../composables/useShortcuts'
import {
  clearSkippedUpdateVersion,
  clearUpdateLatches,
  debugFailNextCheck,
  debugRunCheckNow,
  debugSimulateUpdate,
  debugUpdateState,
  hasPendingUpdate,
  pendingUpdateVersion,
  requestOpenRelease,
  skippedUpdateVersion,
} from '../composables/useAutoUpdate'
import Modal from './ui/Modal.vue'
import Icon, { type IconName } from './ui/Icon.vue'
import Popconfirm from './ui/Popconfirm.vue'
import CustomNumberInput from './ui/CustomNumberInput.vue'
import CustomSelect from './ui/CustomSelect.vue'
import { envBaseUrl } from '../utils/environment'
import type {
  Environment,
  LogFile,
  McpToolStatus,
  Project,
  ProjectStat,
  SeqCounter,
} from '../types/miyoApi'

// 环境管理是独立大弹窗：设置面板只在点「编辑 / 新建环境」时才需要它的 chunk。
const EnvironmentManager = lazyComponent(() => import('./EnvironmentManager.vue'))

const emit = defineEmits<{ close: []; restored: [projectId: string] }>()

const api = useMiyoApi()
const toast = useToast()
const theme = useThemeStore()
const locale = useLocaleStore()
const t = locale.t

const THEME_OPTIONS = computed<{ value: ThemeMode; label: string; icon: IconName }[]>(() => [
  { value: 'system', label: t('settings.themeSystem'), icon: 'monitor' },
  { value: 'dark', label: t('settings.themeDark'), icon: 'moon' },
  { value: 'light', label: t('settings.themeLight'), icon: 'sun' },
])

const LANG_OPTIONS = computed<{ value: LocaleMode; label: string }[]>(() => [
  { value: 'system', label: t('settings.languageSystem') },
  { value: 'zh', label: '简体中文' },
  { value: 'en', label: 'English' },
])

// ---------- 分类导航 ----------
type TabId = 'general' | 'network' | 'mcp' | 'shortcuts' | 'sequences' | 'data' | 'environments' | 'logs'
interface TabDef {
  id: TabId
  label: string
  icon: IconName
}
const tabs = computed<TabDef[]>(() => [
  { id: 'general', label: t('settings.general'), icon: 'settings' },
  { id: 'network', label: t('settings.network'), icon: 'globe' },
  { id: 'mcp', label: t('settings.mcp'), icon: 'plug' },
  { id: 'shortcuts', label: t('settings.shortcuts'), icon: 'keyboard' },
  { id: 'sequences', label: t('settings.sequences'), icon: 'list' },
  { id: 'data', label: t('settings.data'), icon: 'folder' },
  { id: 'environments', label: t('settings.environments'), icon: 'beaker' },
  { id: 'logs', label: t('settings.logs'), icon: 'file' },
])
const activeTab = ref<TabId>('general')

const showManager = ref(false)
/** 「编辑」某环境时，传给 EnvironmentManager 让它初始聚焦该环境。 */
const managerEnvId = ref<string | null>(null)

const project = ref<Project | null>(null)
const projectStat = ref<ProjectStat | null>(null)
const busy = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)

// ---------- 环境概览 ----------
const environments = ref<Environment[]>([])
const activeEnvId = ref<string | null>(null)
const envLoading = ref(false)
/** 环境页签可选项目列表 + 当前选中项（默认后端激活项目；首页可切换管理任意项目）。 */
const envProjects = ref<Project[]>([])
const envProjectId = ref('')

async function loadEnvProjects(): Promise<void> {
  try {
    envProjects.value = await api.getProjects()
  } catch {
    envProjects.value = []
  }
  // 默认选中后端激活项目；无效则回退第一个
  if (!envProjects.value.some((p) => p.id === envProjectId.value)) {
    envProjectId.value = project.value?.id ?? envProjects.value[0]?.id ?? ''
  }
}

const envProjectName = computed(
  () => envProjects.value.find((p) => p.id === envProjectId.value)?.name ?? '',
)

const envProjectOptions = computed(() =>
  envProjects.value.map((p) => ({ value: p.id, label: p.name })),
)

function onEnvProjectChange(value: string | number): void {
  envProjectId.value = String(value)
  void loadEnvironments()
}

async function loadEnvironments(): Promise<void> {
  envLoading.value = true
  try {
    const [envs, active] = await Promise.all([
      api.listEnvironments(envProjectId.value),
      api.getActiveEnvironment(),
    ])
    environments.value = envs
    activeEnvId.value = active?.id ?? null
  } catch {
    environments.value = []
    activeEnvId.value = null
  } finally {
    envLoading.value = false
  }
}

/** 打开环境管理弹窗；envId 为空则聚焦当前激活（或第一个）环境。 */
function openEnvironmentManager(envId: string | null = null): void {
  managerEnvId.value = envId
  showManager.value = true
}

/** 环境概览辅助：环境声明的 Base URL。 */
function envBase(env: Environment): string {
  return envBaseUrl(env)
}

/** 环境概览辅助：启用中的变量数量。 */
function envVarCount(env: Environment): number {
  return env.variables.filter((v) => v.enabled).length
}

// 管理弹窗关闭后刷新概览（可能新增/删除/改激活环境）
watch(showManager, (open) => {
  if (!open) void loadEnvironments()
})

onMounted(async () => {
  try {
    project.value = (await api.getActiveProject()) ?? null
  } catch {
    project.value = null
  }
  if (project.value) {
    try {
      const stats = await api.listProjectStats()
      projectStat.value = stats.find((s) => s.project_id === project.value?.id) ?? null
    } catch {
      projectStat.value = null
    }
  }
  try {
    proxyUrl.value = (await api.getHttpProxy()) ?? ''
    proxyEnabled.value = !!proxyUrl.value
  } catch {
    proxyUrl.value = ''
    proxyEnabled.value = false
  }
  try {
    mcpTools.value = await api.getMcpTools()
  } catch {
    mcpTools.value = []
  }
  try {
    const tls = await api.getTlsSettings()
    tlsIdentityPath.value = tls.identityPath ?? ''
    tlsAcceptInvalid.value = !!tls.acceptInvalidCerts
  } catch {
    tlsIdentityPath.value = ''
    tlsAcceptInvalid.value = false
  }
  try {
    const ms = await api.getHttpTimeoutMs()
    timeoutSec.value = ms != null ? Math.round(ms / 1000) : DEFAULT_TIMEOUT_SEC
  } catch {
    timeoutSec.value = DEFAULT_TIMEOUT_SEC
  }
  await loadEnvProjects()
  await loadEnvironments()
  await loadCounters()
  reloadSkipped()
  await loadDataDirs()
})

// ---------- 通用设置：请求超时（自动保存） ----------
/** 全局超时兜底值（秒）：与后端执行链路的内置默认同源（workspace 常量镜像 miyo-http）。 */
const DEFAULT_TIMEOUT_SEC = FALLBACK_GLOBAL_TIMEOUT_MS / 1000
const timeoutSec = ref(DEFAULT_TIMEOUT_SEC)

async function saveTimeout(sec: number): Promise<void> {
  const v = Math.round(Number(sec))
  if (!Number.isFinite(v) || v < 1 || v > 3600) {
    toast.error(t('settings.timeoutRange'))
    return
  }
  try {
    await api.setHttpTimeoutMs(v * 1000)
    timeoutSec.value = v
    // 编辑器占位符实时展示新的全局超时（workspace store 缓存该值）
    void useWorkspaceStore().refreshGlobalTimeoutMs()
    toast.success(t('settings.timeoutSaved', { v }))
  } catch (err) {
    toast.error(t('settings.saveFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

// ---------- 网络与代理 ----------
const proxyUrl = ref('')
const proxyEnabled = ref(false)
const proxyBusy = ref(false)
const proxyTesting = ref(false)
const proxyTest = ref<{ ok: boolean; message: string } | null>(null)

// ---------- MCP 工具权限 ----------
const mcpTools = ref<McpToolStatus[]>([])
const mcpToolBusy = ref<string | null>(null)

const mcpReadTools = computed(() => mcpTools.value.filter((x) => x.kind === 'read'))
const mcpWriteTools = computed(() => mcpTools.value.filter((x) => x.kind === 'write'))

async function toggleMcpTool(tool: McpToolStatus): Promise<void> {
  mcpToolBusy.value = tool.name
  try {
    await api.setMcpToolEnabled({ name: tool.name, enabled: !tool.enabled })
    tool.enabled = !tool.enabled
  } catch (err) {
    toast.error(t('settings.mcpToolSaveFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    mcpToolBusy.value = null
  }
}

// ---------- TLS：客户端证书（mTLS）与自签放行 ----------
const tlsIdentityPath = ref('')
const tlsAcceptInvalid = ref(false)
const tlsBusy = ref(false)

/** 保存 TLS 设置（证书路径变更 / 自签开关切换共用；后端立即重建共享客户端）。 */
async function saveTls(): Promise<void> {
  tlsBusy.value = true
  try {
    await api.setTlsSettings({
      identityPath: tlsIdentityPath.value.trim() || null,
      acceptInvalidCerts: tlsAcceptInvalid.value,
    })
    toast.success(t('settings.tlsSaved'))
  } catch (err) {
    toast.error(t('settings.tlsSaveFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    tlsBusy.value = false
  }
}

/** 自签放行开关：翻转后立即保存（行为变化即时生效）。 */
async function toggleTlsInsecure(): Promise<void> {
  tlsAcceptInvalid.value = !tlsAcceptInvalid.value
  await saveTls()
}

async function applyProxy(url: string | null): Promise<boolean> {
  proxyBusy.value = true
  try {
    await api.setHttpProxy(url)
    return true
  } catch (err) {
    toast.error(t('settings.proxySaveFail'), { message: err instanceof Error ? err.message : String(err) })
    return false
  } finally {
    proxyBusy.value = false
  }
}

/** 启用开关：关闭 → 直连；开启 → 沿用已有地址（为空则不落「开」态，聚焦输入框待配置）。 */
async function toggleProxy(): Promise<void> {
  if (proxyEnabled.value) {
    proxyEnabled.value = false
    if (await applyProxy(null)) toast.success(t('settings.proxyDirect'))
    proxyTest.value = null
  } else {
    if (!proxyUrl.value.trim()) {
      // 空地址不落「开」态：开关显示开启而后端仍直连，是状态与事实相反的误导。
      // 展开输入框并提示，填好失焦（saveProxyUrl）校验通过后由用户再开启。
      toast.warning(t('settings.proxyEmptyHint'), { duration: 6000 })
      return
    }
    proxyEnabled.value = true
    if (await applyProxy(proxyUrl.value.trim())) toast.success(t('settings.proxyEnabledToast'))
  }
}

/** URL 失焦自动保存。 */
async function saveProxyUrl(): Promise<void> {
  const u = proxyUrl.value.trim()
  if (!proxyEnabled.value) return
  if (u && !/^(https?|socks5?):\/\//i.test(u)) {
    toast.error(t('settings.proxyUrlInvalid'))
    return
  }
  if (await applyProxy(u || null)) toast.success(u ? t('settings.proxySaved') : t('settings.proxyDirect'))
}

/** 测试连通性：先落地当前输入，再经共享客户端（含代理）请求目标。 */
async function testProxy(): Promise<void> {
  if (proxyEnabled.value && proxyUrl.value.trim()) {
    await applyProxy(proxyUrl.value.trim())
  }
  proxyTesting.value = true
  proxyTest.value = null
  try {
    const r = await api.testHttpProxy()
    proxyTest.value = { ok: r.ok, message: r.message }
    if (r.ok) {
      toast.success(t('settings.proxyOk'))
    } else {
      toast.error(r.message)
    }
  } catch (err) {
    proxyTest.value = { ok: false, message: err instanceof Error ? err.message : String(err) }
    toast.error(t('settings.proxyTestFail'), { message: proxyTest.value.message })
  } finally {
    proxyTesting.value = false
  }
}

// ---------- MCP 服务（Agent 控制面启停） ----------
const mcpEnabled = ref(true)
const mcpRunning = ref(false)
const mcpAddress = ref<string | null>(null)
const mcpPort = ref(4110)
/** 已持久化的端口：change 重复触发（失焦同值）时跳过保存。 */
const mcpPortSaved = ref(4110)
const mcpBusy = ref(false)

async function loadMcpStatus(): Promise<void> {
  try {
    mcpEnabled.value = await api.getMcpEnabled()
  } catch {
    mcpEnabled.value = true
  }
  try {
    mcpPort.value = await api.getMcpPort()
    mcpPortSaved.value = mcpPort.value
  } catch {
    mcpPort.value = 4110
    mcpPortSaved.value = 4110
  }
  try {
    const s = await api.agentStatus()
    mcpRunning.value = s.running
    mcpAddress.value = s.address
  } catch {
    mcpRunning.value = false
    mcpAddress.value = null
  }
}

/** 启停开关：先应用（启动失败不落盘）再刷新状态。 */
async function toggleMcp(): Promise<void> {
  const next = !mcpEnabled.value
  mcpBusy.value = true
  try {
    await api.setMcpEnabled(next)
    mcpEnabled.value = next
    await loadMcpStatus()
    toast.success(next ? t('settings.mcpEnabledToast') : t('settings.mcpDisabledToast'))
  } catch (err) {
    toast.error(t('settings.saveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
    await loadMcpStatus()
  } finally {
    mcpBusy.value = false
  }
}

/** 修改监听起始端口：校验 → 保存（运行中后端自动重启）→ 刷新状态。 */
async function saveMcpPort(port: number): Promise<void> {
  const v = Math.round(Number(port))
  if (!Number.isFinite(v) || v < 1 || v > 65535) {
    toast.error(t('settings.mcpPortInvalid'))
    mcpPort.value = mcpPortSaved.value
    return
  }
  if (v === mcpPortSaved.value) return
  mcpBusy.value = true
  try {
    await api.setMcpPort(v)
    await loadMcpStatus()
    toast.success(t('settings.mcpPortSaved'))
  } catch (err) {
    toast.error(t('settings.saveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
    await loadMcpStatus()
  } finally {
    mcpBusy.value = false
  }
}

// ---------- 自增序列 ----------
const counters = ref<SeqCounter[]>([])
const newSeqKey = ref('')
const newSeqValue = ref<number>(1)

async function loadCounters(): Promise<void> {
  try {
    counters.value = await api.listSeqCounters()
  } catch {
    counters.value = []
  }
}

async function setCounter(key: string, value: number, message: string): Promise<void> {
  try {
    await api.setSeqCounter(key, Math.max(1, Math.round(value)))
    toast.success(message)
  } catch (err) {
    toast.error(t('settings.saveFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    await loadCounters()
  }
}

function saveSeq(c: SeqCounter): void {
  void setCounter(c.key, c.value, c.key ? t('settings.seqSaved', { key: c.key }) : t('settings.seqGlobalSaved'))
}

function resetSeq(c: SeqCounter): void {
  void setCounter(c.key, 1, c.key ? t('settings.seqReset', { key: c.key }) : t('settings.seqGlobalReset'))
}

async function deleteSeq(c: SeqCounter): Promise<void> {
  try {
    await api.deleteSeqCounter(c.key)
    toast.success(c.key ? t('settings.seqDeleted', { key: c.key }) : t('settings.seqGlobalDeleted'))
  } catch (err) {
    toast.error(t('settings.deleteFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    await loadCounters()
  }
}

async function addSeq(): Promise<void> {
  const key = newSeqKey.value.trim()
  const value = Math.round(Number(newSeqValue.value))
  if (!Number.isFinite(value) || value < 1) {
    toast.error(t('settings.seqStartInvalid'))
    return
  }
  try {
    await api.setSeqCounter(key, value)
    toast.success(key ? t('settings.seqAdded', { key, value }) : t('settings.seqGlobalAdded'))
    newSeqKey.value = ''
    newSeqValue.value = 1
  } catch (err) {
    toast.error(t('settings.addFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    await loadCounters()
  }
}

// ---------- 数据目录（miyo.db / master.key 所在；变更重启后生效） ----------
const dataDir = ref('')
const defaultDataDir = ref('')
const needRestart = ref(false)

async function loadDataDirs(): Promise<void> {
  try {
    dataDir.value = await api.getDataDir()
  } catch {
    dataDir.value = ''
  }
  try {
    defaultDataDir.value = await api.getDefaultDataDir()
  } catch {
    defaultDataDir.value = ''
  }
}

const isDefaultDir = computed(
  () => !!dataDir.value && !!defaultDataDir.value && dataDir.value === defaultDataDir.value,
)

async function pickDataDir(): Promise<void> {
  const picked = await open({ directory: true, title: t('settings.dataDirTitle') })
  if (typeof picked !== 'string' || !picked) return // 用户取消
  try {
    await api.setDataDir(picked)
    await loadDataDirs()
    needRestart.value = true
    toast.success(t('settings.dataDirSaved'), { message: t('settings.dataDirRestartHint') })
  } catch (err) {
    toast.error(t('settings.dataDirSaveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function resetDataDirToDefault(): Promise<void> {
  try {
    await api.resetDataDir()
    await loadDataDirs()
    needRestart.value = true
    toast.success(t('settings.dataDirResetDone'), { message: t('settings.dataDirRestartHint') })
  } catch (err) {
    toast.error(t('settings.dataDirSaveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

async function revealDataDir(): Promise<void> {
  if (!dataDir.value) return
  try {
    await revealItemInDir(dataDir.value)
  } catch {
    toast.error(t('settings.revealFail'))
  }
}

const copiedDataDir = ref(false)
let copiedDataDirTimer: ReturnType<typeof setTimeout> | undefined

/** 点击路径复制完整目录（ overlong 路径全宽展示 + 可选中，复制给反馈图标）。 */
async function copyDataDir(): Promise<void> {
  if (!dataDir.value) return
  const ok = await copyText(dataDir.value)
  if (!ok) {
    toast.error(t('response.copyFail'))
    return
  }
  copiedDataDir.value = true
  if (copiedDataDirTimer) clearTimeout(copiedDataDirTimer)
  copiedDataDirTimer = setTimeout(() => {
    copiedDataDir.value = false
  }, 1500)
  toast.success(t('settings.dataDirCopied'))
}

async function restartNow(): Promise<void> {
  try {
    await relaunch()
  } catch (err) {
    toast.error(t('settings.restartFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  }
}

// ---------- 数据与备份 ----------
async function exportBackup(): Promise<void> {  if (!project.value) return
  busy.value = true
  try {
    const doc = await api.backupExport(project.value.id)
    const text = doc.content
    const stamp = new Date().toISOString().slice(0, 10)
    const filename = `${project.value.name}-${t('settings.backupFileName')}-${stamp}.json`

    // Tauri 环境：目录选择框选目标文件夹，再拼接默认文件名经 save_text_file 落盘。
    if ('__TAURI_INTERNALS__' in window) {
      const dir = await open({
        directory: true,
        title: t('settings.backupDirTitle'),
      })
      if (!dir) return // 用户取消
      const path = await join(dir, filename)
      await api.writeTextFile(path, text)
      toast.success(t('settings.backupExported'), {
        message: path.split('/').pop() || path,
        action: {
          label: t('settings.revealLabel'),
          run: () => {
            void revealItemInDir(path).catch(() => toast.error(t('settings.revealFail')))
          },
        },
      })
      // 明文凭据提示：备份必须可恢复，不做替换，但要让用户知道这份文件里有几把钥匙。
      if (doc.sensitive_count) {
        toast.warning(t('settings.backupSensitive', { n: doc.sensitive_count }))
      }
      return
    }

    // 浏览器预览兜底：Blob 下载
    const blob = new Blob([text], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
    toast.success(t('settings.backupExportedPlain'))
  } catch (err) {
    toast.error(t('settings.exportFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

async function onImportFile(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  busy.value = true
  try {
    const text = await file.text()
    const summary = await api.backupRestore(text)
    const extras: string[] = []
    const sep = locale.resolved === 'zh' ? '、' : ', '
    if (summary.settings_applied?.length) extras.push(t('settings.importSettingsApplied', { v: summary.settings_applied.join(sep) }))
    if (summary.settings_skipped?.length) extras.push(t('settings.importSettingsKept', { v: summary.settings_skipped.join(sep) }))
    const mergedVars = summary.global_variables_merged ?? 0
    const mergedParams = summary.global_params_merged ?? 0
    if (mergedVars + mergedParams > 0) extras.push(t('settings.importVarsMerged', { n: mergedVars + mergedParams }))
    toast.success(t('settings.importRestored', { name: summary.name, endpoints: summary.endpoints, envs: summary.environments }), {
      message: extras.join(locale.resolved === 'zh' ? '；' : '; ') || undefined,
    })
    // 恢复落库为「全新项目」：通知宿主刷新列表/工作区，否则需重启才能看到
    emit('restored', summary.id)
    emit('close')
  } catch (err) {
    toast.error(t('settings.importFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

// ---------- 日志查看 ----------
const logFiles = ref<LogFile[]>([])
const logSelected = ref<string>('')
/** 下拉选项：文件名 + 大小（与原原生 option 文案一致）。 */
const logFileOptions = computed(() =>
  logFiles.value.map((f) => ({
    value: f.name,
    label: `${f.name}${t('settings.logSize', { v: (f.size_bytes / 1024).toFixed(1) })}`,
  })),
)
const logContent = ref('')
const logLoading = ref(false)
/** 保留天数：已持久化值（change 同值跳过保存）。 */
const logRetention = ref(14)
const logRetentionSaved = ref(14)
const logRetentionBusy = ref(false)

async function loadLogRetention(): Promise<void> {
  try {
    logRetention.value = await api.getLogRetentionDays()
    logRetentionSaved.value = logRetention.value
  } catch {
    logRetention.value = 14
    logRetentionSaved.value = 14
  }
}

/** 修改保留天数：校验 → 保存（后端立即清理）→ 刷新列表。 */
async function saveLogRetention(days: number): Promise<void> {
  const v = Math.round(Number(days))
  if (!Number.isFinite(v) || v < 1 || v > 365) {
    toast.error(t('settings.logRetentionInvalid'))
    logRetention.value = logRetentionSaved.value
    return
  }
  if (v === logRetentionSaved.value) return
  logRetentionBusy.value = true
  try {
    await api.setLogRetentionDays(v)
    logRetentionSaved.value = v
    await loadLogFiles()
    if (logSelected.value && !logFiles.value.some((f) => f.name === logSelected.value)) {
      logSelected.value = logFiles.value[0]?.name ?? ''
    }
    await loadLogTail()
    toast.success(t('settings.logRetentionSaved'))
  } catch (err) {
    toast.error(t('settings.saveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
    await loadLogRetention()
  } finally {
    logRetentionBusy.value = false
  }
}

async function loadLogFiles(): Promise<void> {
  try {
    logFiles.value = (await api.logFiles()) ?? []
    if (!logSelected.value && logFiles.value.length) {
      logSelected.value = logFiles.value[0].name
    }
  } catch {
    logFiles.value = []
  }
}

async function loadLogTail(): Promise<void> {
  if (!logSelected.value) {
    logContent.value = ''
    return
  }
  logLoading.value = true
  try {
    logContent.value = await api.logTail(logSelected.value, 300)
  } catch (err) {
    toast.error(t('settings.logReadFail'), { message: err instanceof Error ? err.message : String(err) })
    logContent.value = ''
  } finally {
    logLoading.value = false
  }
}

async function openLogDir(): Promise<void> {
  try {
    const dir = await api.logDirPath()
    await revealItemInDir(dir)
  } catch {
    toast.error(t('settings.logDirFail'))
  }
}

watch(activeTab, (tab) => {
  if (tab === 'logs') {
    void loadLogRetention().then(() => void loadLogFiles().then(() => void loadLogTail()))
  }
  if (tab === 'general') {
    reloadSkipped()
  }
  if (tab === 'mcp') {
    void loadMcpStatus()
  }
})

// ---------- 快捷键自定义 ----------
/** 快捷键行（生效键位 + 是否改过，随覆盖变更自动刷新）。 */
const shortcutRows = computed(() => {
  // 订阅覆盖变更，改动即重算展示
  // eslint-disable-next-line @typescript-eslint/no-unused-expressions
  shortcutBindingsTick.value
  return SHORTCUT_DEFAULTS.map((d) => ({
    ...d,
    effective: defaultBindingOf(d.id) ?? d.binding,
    customized: isShortcutCustomized(d.id),
  }))
})

const shortcutGroups = computed(() => {
  const order: string[] = []
  const map = new Map<string, typeof shortcutRows.value>()
  for (const row of shortcutRows.value) {
    const list = map.get(row.group)
    if (list) list.push(row)
    else {
      map.set(row.group, [row])
      order.push(row.group)
    }
  }
  return order.map((group) => ({ group, items: map.get(group)! }))
})

const customizedCount = computed(() => shortcutRows.value.filter((r) => r.customized).length)

/** 正在录制的项 id（null = 未录制）。 */
const recordingId = ref<string | null>(null)

function startRecording(id: string): void {
  recordingId.value = id
}

/** 录制中按 Esc 取消。 */
function cancelRecording(): void {
  recordingId.value = null
}

const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta'])

/**
 * 录制捕获（window 捕获阶段拦截，不触发全局快捷键）：
 * 纯修饰键忽略；要求至少按住 ⌘/Ctrl（防裸字母劫持输入）；Esc 取消。
 */
function onRecordKeydown(e: KeyboardEvent): void {
  const id = recordingId.value
  if (!id) return
  e.preventDefault()
  e.stopPropagation()
  if (e.key === 'Escape') {
    cancelRecording()
    return
  }
  if (MODIFIER_KEYS.has(e.key)) return
  if (!e.ctrlKey && !e.metaKey) {
    toast.warning(t('settings.scNeedMod'), { message: t('settings.scNeedModHint') })
    return
  }
  const binding: ShortcutBinding = {
    mod: 'ctrl',
    shift: e.shiftKey,
    alt: e.altKey,
    key: e.key,
  }
  const conflict = findBindingConflict(id, binding)
  if (conflict) {
    toast.error(t('settings.scConflict', { name: t(conflict.description) }), { message: t('settings.scConflictHint') })
    return
  }
  setShortcutBinding(id, binding)
  recordingId.value = null
  const def = SHORTCUT_DEFAULTS.find((d) => d.id === id)
  toast.success(t('settings.scUpdated', { name: def ? t(def.description) : id }), { message: bindingLabel(binding) })
}

watch(recordingId, (id) => {
  if (id) window.addEventListener('keydown', onRecordKeydown, true)
  else window.removeEventListener('keydown', onRecordKeydown, true)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onRecordKeydown, true)
  if (copiedDataDirTimer) clearTimeout(copiedDataDirTimer)
})

function resetOneShortcut(id: string): void {
  resetShortcutBinding(id)
  const def = SHORTCUT_DEFAULTS.find((d) => d.id === id)
  toast.success(t('settings.scReset', { name: def ? t(def.description) : id }), {
    message: def ? bindingLabel(def.binding) : undefined,
  })
}

function resetAllShortcuts(): void {
  resetAllShortcutBindings()
  toast.success(t('settings.scResetAll'))
}

// ---------- 软件更新：跳过的版本 ----------
const skippedVersion = ref<string | null>(null)

function reloadSkipped(): void {
  skippedVersion.value = skippedUpdateVersion()
}

function unskipVersion(): void {
  clearSkippedUpdateVersion()
  skippedVersion.value = null
  // 取消即“让我再看到它”：清提醒锁存并立即真查一次，结果落定后再反馈+刷新
  //（远端仍有该版本则 toast + 小红点马上回来；没有则明确告知已是最新）
  clearUpdateLatches()
  toast.success(t('settings.unskipped'), { message: t('settings.unskippedHint') })
  void debugRunCheckNow().then((r) => {
    refreshDebugState()
    if (r.status === 'none') toast.info(t('settingsdbg.upToDate'))
    else if (r.status === 'failed') toast.error(t('settingsdbg.checkFailed'), { message: r.message })
    else if (r.status === 'no-instance') toast.info(t('settingsdbg.noInstance'))
  })
}

// ---------- 软件更新：待安装版本 ----------
/** 有暂存更新时展示版本号（小红点点进来后落到此处一键安装）。 */
const pendingVersion = computed(() => (hasPendingUpdate.value ? pendingUpdateVersion() : null))

function openUpdateDetail(): void {
  emit('close')
  // 详情走独立 ReleaseNotesModal（不把用户丢进关于弹窗 / 设置长列表）
  requestOpenRelease()
}

// ---------- 更新调试（仅开发版可见）：免改版号验证更新链路 ----------
const showUpdateDebug = import.meta.env.DEV
const debugVersion = ref('9.9.9')
const debugStateText = ref('')

function refreshDebugState(): void {
  const s = debugUpdateState()
  const fmtTime = (ts: number): string => (ts ? new Date(ts).toLocaleString() : t('settingsdbg.never'))
  const orNone = (v: string | null): string => v || t('settingsdbg.none')
  debugStateText.value = t('settingsdbg.stateLine', {
    last: fmtTime(s.lastCheck),
    notified: orNone(s.notifiedVersion),
    at: s.notifiedAt ? new Date(s.notifiedAt).toLocaleString() : t('settingsdbg.none'),
    skipped: orNone(s.skipped),
    pending: orNone(s.pending),
  })
}
refreshDebugState()

function debugSimulate(): void {
  const v = debugVersion.value.trim() || '9.9.9'
  if (!debugSimulateUpdate(v)) {
    toast.info(t('settingsdbg.noInstance'))
    return
  }
  refreshDebugState()
  // 点击点显式反馈：模拟已生效（顶部 Tag / 发现新版本卡片 / 更新 toast 已就位）
  toast.success(t('settingsdbg.simulated', { v }))
}

function debugFail(): void {
  debugFailNextCheck()
  toast.success(t('settingsdbg.applied'))
  refreshDebugState()
}

function debugCheckNow(): void {
  // 结果落定后再反馈+刷新：无新版/失败不再静默
  void debugRunCheckNow().then((r) => {
    refreshDebugState()
    if (r.status === 'none') toast.info(t('settingsdbg.upToDate'))
    else if (r.status === 'failed') toast.error(t('settingsdbg.checkFailed'), { message: r.message })
    else if (r.status === 'no-instance') toast.info(t('settingsdbg.noInstance'))
  })
}

function debugClear(): void {
  clearUpdateLatches()
  refreshDebugState()
  toast.success(t('settingsdbg.applied'))
}

// ---------- 通用派生 ----------
const sequencesCount = computed(() => counters.value.length)
const projectSummary = computed(() => {
  if (!project.value) return t('settings.noProjectSummary')
  const eps = projectStat.value?.endpoint_count ?? '—'
  return t('settings.projectSummary', { name: project.value.name, n: eps })
})
</script>

<template>
  <Modal :open="true" :title="t('settings.title')" width="880px" dialog-class="sd-dialog" @close="emit('close')">
    <div class="flex h-[min(520px,70vh)]">
      <!-- 左：极简 List 导航（.m-body 已贴边，内距由本栏自持；分隔线与标题栏 hairline 同源） -->
      <aside class="flex w-52 shrink-0 flex-col gap-1 border-r border-line p-3">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          class="relative flex h-9 w-full items-center gap-2.5 rounded-xl border-none px-3 text-left text-sm transition-all duration-150"
          :class="
            activeTab === tab.id
              ? 'bg-purple-50 font-semibold text-purple-700 dark:bg-purple-500/15 dark:text-purple-300'
              : 'bg-transparent text-zinc-600 hover:bg-slate-100/80 hover:text-slate-700 dark:text-zinc-400 dark:hover:bg-white/[0.05] dark:hover:text-zinc-200'
          "
          @click="activeTab = tab.id"
        >
          <!-- 选中时的左侧高亮细条 -->
          <span
            v-if="activeTab === tab.id"
            class="su-tab-marker absolute left-1 top-1/2 h-4 w-0.5 -translate-y-1/2 rounded-full"
          />
          <Icon
            :name="tab.icon"
            :size="15"
            :class="activeTab === tab.id ? 'text-purple-700 dark:text-purple-300' : 'text-zinc-500 dark:text-zinc-400'"
          />
          <span class="flex-1 truncate">{{ tab.label }}</span>
          <span
            v-if="tab.id === 'sequences' && sequencesCount"
            class="rounded-full px-1.5 py-px text-xxs leading-4"
            :class="
              activeTab === 'sequences'
                ? 'bg-purple-500/15 text-purple-700 dark:bg-purple-400/20 dark:text-purple-200'
                : 'bg-zinc-200/70 text-zinc-600 dark:bg-white/[0.08] dark:text-zinc-400'
            "
          >
            {{ sequencesCount }}
          </span>
        </button>
      </aside>

      <!-- 右：内容区（Tab 切换淡入） -->
      <div class="flex-1 min-w-0 overflow-y-auto p-6">
        <Transition name="pane" mode="out-in">
          <div :key="activeTab" class="space-y-4">
            <!-- 通用设置 -->
            <section v-if="activeTab === 'general'">
              <header>
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.general') }}</h2>
                <p class="mt-1 mb-5 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.generalDesc') }}</p>
              </header>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center justify-between gap-4">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.timeout') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.timeoutDesc') }}
                    </p>
                  </div>
                  <div class="relative shrink-0">
                    <CustomNumberInput
                      :model-value="timeoutSec"
                      :min="1"
                      :max="3600"
                      :step="10"
                      size="md"
                      tone="inset"
                      class="w-24"
                      @change="saveTimeout"
                    />
                    <span
                      class="pointer-events-none absolute right-[28px] top-1/2 -translate-y-1/2 text-xs text-zinc-500"
                    >
                      {{ t('settings.timeoutUnit') }}
                    </span>
                  </div>
                </div>
              </div>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center justify-between gap-4">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.theme') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.themeDesc') }}</p>
                  </div>
                  <div
                    class="flex shrink-0 items-center gap-1 rounded-lg border border-zinc-300/50 bg-zinc-200/60 p-1 dark:border-white/10 dark:bg-black/30"
                    role="radiogroup"
                    :aria-label="t('settings.theme')"
                  >
                    <button
                      v-for="opt in THEME_OPTIONS"
                      :key="opt.value"
                      type="button"
                      role="radio"
                      :aria-checked="theme.mode === opt.value"
                      class="flex h-8 items-center gap-1.5 rounded-md px-3 text-xs transition-all duration-150"
                      :class="
                        theme.mode === opt.value
                          ? 'su-chip-active font-medium text-white shadow-sm'
                          : 'text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-200'
                      "
                      @click="theme.setMode(opt.value)"
                    >
                      <Icon :name="opt.icon" :size="13" aria-hidden="true" />
                      {{ opt.label }}
                    </button>
                  </div>
                </div>
                <div class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <div class="flex items-center justify-between gap-4 pt-5">
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.language') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.languageDesc') }}</p>
                    </div>
                    <div
                      class="flex shrink-0 items-center gap-1 rounded-lg border border-zinc-300/50 bg-zinc-200/60 p-1 dark:border-white/10 dark:bg-black/30"
                      role="radiogroup"
                      :aria-label="t('settings.language')"
                    >
                      <button
                        v-for="opt in LANG_OPTIONS"
                        :key="opt.value"
                        type="button"
                        role="radio"
                        :aria-checked="locale.mode === opt.value"
                        class="flex h-8 items-center gap-1.5 rounded-md px-3 text-xs transition-all duration-150"
                        :class="
                          locale.mode === opt.value
                            ? 'su-chip-active font-medium text-white shadow-sm'
                            : 'text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-200'
                        "
                        @click="locale.setMode(opt.value)"
                      >
                        {{ opt.label }}
                      </button>
                    </div>
                  </div>
                </div>
                <div class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <button
                    type="button"
                    class="flex w-full items-center justify-between gap-4 pt-5 text-left"
                    @click="activeTab = 'shortcuts'"
                  >
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.shortcuts') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.shortcutsJump') }}</p>
                    </div>
                    <span
                      class="shrink-0 rounded-full border border-zinc-200 bg-zinc-100 px-2 py-0.5 text-xxs text-zinc-500 dark:border-white/[0.05] dark:bg-white/[0.03] dark:text-zinc-400"
                    >
                      {{ customizedCount ? t('settings.customizedCount', { n: customizedCount }) : t('settings.customizableCount', { n: SHORTCUT_DEFAULTS.length }) }}
                    </span>
                  </button>
                </div>
                <!-- 发现新版本：高亮卡片提权（渐变底 + 紫色版本号 + 主紫行动按钮） -->
                <div v-if="pendingVersion" class="su-update-card mt-5 rounded-xl border border-purple-500/30 p-4">
                  <div class="flex items-center justify-between gap-4">
                    <div class="max-w-md">
                      <div class="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
                        {{ t('about.newVersion') }}
                        <span class="font-mono font-bold text-purple-600 dark:text-purple-400">v{{ pendingVersion }}</span>
                      </div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.updateAvailableDesc') }}</p>
                    </div>
                    <button
                      class="miyo-btn miyo-btn-primary miyo-btn-sm shrink-0"
                      type="button"
                      @click="openUpdateDetail"
                    >
                      {{ t('app.viewDetails') }}
                    </button>
                  </div>
                </div>
                <div v-if="skippedVersion" class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <div class="flex items-center justify-between gap-4 pt-5">
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">
                        {{ t('settings.skippedVersion', { v: skippedVersion }) }}
                      </div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.skippedVersionDesc') }}</p>
                    </div>
                    <button class="miyo-btn miyo-btn-sm shrink-0" type="button" @click="unskipVersion">
                      {{ t('settings.unskip') }}
                    </button>
                  </div>
                </div>
                <div v-if="showUpdateDebug" class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <div class="pt-5">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settingsdbg.title') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settingsdbg.desc') }}</p>
                    <div class="mt-2 flex flex-wrap items-center gap-2">
                      <input
                        v-model="debugVersion"
                        class="miyo-input miyo-input-sm w-36"
                        spellcheck="false"
                        :placeholder="t('settingsdbg.versionPh')"
                      />
                      <button class="miyo-btn miyo-btn-sm" type="button" @click="debugSimulate">
                        {{ t('settingsdbg.simulate') }}
                      </button>
                      <button class="miyo-btn miyo-btn-sm" type="button" @click="debugFail">
                        {{ t('settingsdbg.failNext') }}
                      </button>
                      <button class="miyo-btn miyo-btn-sm" type="button" @click="debugCheckNow">
                        {{ t('settingsdbg.checkNow') }}
                      </button>
                      <button class="miyo-btn miyo-btn-sm" type="button" @click="debugClear">
                        {{ t('settingsdbg.clear') }}
                      </button>
                    </div>
                    <p class="mt-1.5 font-mono text-xxs text-zinc-500 dark:text-zinc-400">{{ debugStateText }}</p>
                  </div>
                </div>
              </div>
            </section>

            <!-- 网络与代理 -->
            <section v-if="activeTab === 'network'">
              <header>
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.network') }}</h2>
                <p class="mt-1 mb-5 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.networkDesc') }}</p>
              </header>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center justify-between gap-4">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.proxyEnable') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.proxyEnableDesc') }}
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    :aria-checked="proxyEnabled"
                    class="relative h-[22px] w-[40px] shrink-0 rounded-full transition-colors duration-150"
                    :class="proxyEnabled ? 'bg-purple-500' : 'border border-zinc-300 bg-zinc-200 dark:border-white/10 dark:bg-white/10'"
                    @click="toggleProxy"
                  >
                    <span
                      class="absolute top-1/2 h-[16px] w-[16px] -translate-y-1/2 rounded-full bg-white shadow transition-all duration-150"
                      :class="proxyEnabled ? 'left-[21px]' : 'left-[2px]'"
                    />
                  </button>
                </div>

                <div v-if="proxyEnabled" class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <div class="flex items-center justify-between gap-4 pt-5">
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.proxyUrlLabel') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                        {{ t('settings.proxyUrlDescEg') }} <code class="font-mono text-xxs">http://127.0.0.1:7890</code> {{ t('settings.proxyUrlDescOr') }}
                        <code class="font-mono text-xxs">socks5://host:1080</code>{{ t('settings.proxyUrlDescBlur') }}
                      </p>
                    </div>
                    <input
                      v-model="proxyUrl"
                      class="miyo-input h-8 w-72 shrink-0 font-mono text-xs"
                      type="text"
                      placeholder="http://127.0.0.1:7890"
                      spellcheck="false"
                      @change="saveProxyUrl"
                    />
                  </div>

                  <div class="flex items-center justify-between gap-4 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.proxyTestTitle') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                        {{ proxyTest ? proxyTest.message : t('settings.proxyTestDesc') }}
                      </p>
                    </div>
                    <button
                      class="miyo-btn shrink-0"
                      type="button"
                      :disabled="proxyBusy || proxyTesting"
                      @click="testProxy"
                    >
                      <Icon name="zap" :size="13" />
                      {{ proxyTesting ? t('settings.proxyTesting') : t('settings.proxyTest') }}
                    </button>
                  </div>
                </div>
              </div>

              <!-- TLS：客户端证书（mTLS）与自签放行 -->
              <div class="mt-5 rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center justify-between gap-4">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.tlsIdentity') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.tlsIdentityDesc') }}
                    </p>
                  </div>
                  <input
                    v-model="tlsIdentityPath"
                    class="miyo-input h-8 w-72 shrink-0 font-mono text-xs"
                    type="text"
                    placeholder="/path/to/client.pem"
                    spellcheck="false"
                    @change="saveTls"
                  />
                </div>
                <div class="mt-5 flex items-center justify-between gap-4 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.tlsInsecure') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.tlsInsecureDesc') }}
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    :aria-checked="tlsAcceptInvalid"
                    :disabled="tlsBusy"
                    class="relative h-[22px] w-[40px] shrink-0 rounded-full transition-colors duration-150"
                    :class="tlsAcceptInvalid ? 'bg-purple-500' : 'border border-zinc-300 bg-zinc-200 dark:border-white/10 dark:bg-white/10'"
                    @click="toggleTlsInsecure"
                  >
                    <span
                      class="absolute top-1/2 h-[16px] w-[16px] -translate-y-1/2 rounded-full bg-white shadow transition-all duration-150"
                      :class="tlsAcceptInvalid ? 'left-[21px]' : 'left-[2px]'"
                    />
                  </button>
                </div>
              </div>
            </section>

            <!-- MCP 服务 -->
            <section v-if="activeTab === 'mcp'">
              <header>
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcp') }}</h2>
                <p class="mt-1 mb-5 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.mcpDesc') }}</p>
              </header>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center justify-between gap-4">
                  <div class="max-w-md">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcpEnable') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.mcpEnableDesc') }}
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    :aria-checked="mcpEnabled"
                    :disabled="mcpBusy"
                    class="relative h-[22px] w-[40px] shrink-0 rounded-full transition-colors duration-150"
                    :class="mcpEnabled ? 'bg-purple-500' : 'border border-zinc-300 bg-zinc-200 dark:border-white/10 dark:bg-white/10'"
                    @click="toggleMcp"
                  >
                    <span
                      class="absolute top-1/2 h-[16px] w-[16px] -translate-y-1/2 rounded-full bg-white shadow transition-all duration-150"
                      :class="mcpEnabled ? 'left-[21px]' : 'left-[2px]'"
                    />
                  </button>
                </div>

                <div class="mt-5 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
                  <div class="flex items-center justify-between gap-4">
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcpPort') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                        {{ t('settings.mcpPortDesc') }}
                      </p>
                    </div>
                    <div class="w-32 shrink-0">
                      <CustomNumberInput
                        :model-value="mcpPort"
                        :min="1"
                        :max="65535"
                        size="md"
                        tone="inset"
                        :disabled="mcpBusy"
                        @change="saveMcpPort"
                      />
                    </div>
                  </div>

                  <!-- 工具权限：逐个停用 MCP 工具，停用后 AI 不可见且控制面拒绝调用 -->
                  <div class="mt-5 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcpToolsTitle') }}</div>
                    <p class="mt-0.5 mb-4 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.mcpToolsDesc') }}
                    </p>
                    <div
                      v-for="group in [
                        { key: 'read', label: t('settings.mcpGroupRead'), tools: mcpReadTools },
                        { key: 'write', label: t('settings.mcpGroupWrite'), tools: mcpWriteTools },
                      ]"
                      :key="group.key"
                      class="mb-4 last:mb-0"
                    >
                      <div class="mb-2 text-xxs font-medium uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
                        {{ group.label }}
                      </div>
                      <div class="flex flex-col gap-2">
                        <div
                          v-for="tool in group.tools"
                          :key="tool.name"
                          class="flex items-center justify-between gap-4 rounded-lg border border-zinc-200/70 px-3.5 py-2.5 dark:border-white/[0.06]"
                        >
                          <div class="min-w-0">
                            <div class="text-xs font-medium text-zinc-900 dark:text-zinc-100">
                              {{ t(`settings.mcpTool.${tool.name}`) }}
                            </div>
                            <code class="font-mono text-xxs text-zinc-500 dark:text-zinc-400">{{ tool.name }}</code>
                          </div>
                          <button
                            type="button"
                            role="switch"
                            :aria-checked="tool.enabled"
                            :aria-label="t(`settings.mcpTool.${tool.name}`)"
                            :disabled="mcpToolBusy === tool.name"
                            class="relative h-[22px] w-[40px] shrink-0 rounded-full transition-colors duration-150"
                            :class="tool.enabled ? 'bg-purple-500' : 'border border-zinc-300 bg-zinc-200 dark:border-white/10 dark:bg-white/10'"
                            @click="toggleMcpTool(tool)"
                          >
                            <span
                              class="absolute top-1/2 h-[16px] w-[16px] -translate-y-1/2 rounded-full bg-white shadow transition-all duration-150"
                              :class="tool.enabled ? 'left-[21px]' : 'left-[2px]'"
                            />
                          </button>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>

                <div class="mt-5 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
                  <div class="flex items-center justify-between gap-4">
                    <div class="min-w-0 max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcpStatus') }}</div>
                      <p class="mt-0.5 truncate text-xs text-zinc-600 dark:text-zinc-400">
                        <span
                          class="mr-1.5 inline-block h-2 w-2 rounded-full align-middle"
                          :class="mcpRunning ? 'bg-emerald-500' : 'bg-zinc-400 dark:bg-zinc-500'"
                          aria-hidden="true"
                        ></span>
                        {{ mcpRunning ? t('settings.mcpRunning') : t('settings.mcpStopped') }}
                        <template v-if="mcpRunning && mcpAddress">
                          · {{ t('settings.mcpAddress') }}
                          <code class="font-mono text-xxs">{{ mcpAddress }}</code>
                        </template>
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <!-- 快捷键 -->
            <section v-if="activeTab === 'shortcuts'">
              <header class="mb-6 flex items-start justify-between gap-4">
                <div>
                  <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.shortcuts') }}</h2>
                  <p class="mt-1 text-xs text-zinc-600 dark:text-zinc-400">
                    {{ t('settings.shortcutsDesc') }}
                  </p>
                </div>
                <button
                  class="miyo-btn miyo-btn-sm shrink-0"
                  type="button"
                  :disabled="!customizedCount"
                  @click="resetAllShortcuts"
                >
                  <Icon name="refresh" :size="12" />
                  {{ t('settings.shortcutsResetAll') }}
                </button>
              </header>

              <div v-for="g in shortcutGroups" :key="g.group" class="mb-4">
                <div class="mb-1.5 px-1 text-xxs font-medium uppercase tracking-wider text-zinc-500 dark:text-zinc-400">
                  {{ t(g.group) }}
                </div>
                <div class="overflow-hidden rounded-xl border border-zinc-200/70 bg-zinc-50/80 dark:border-white/[0.06] dark:bg-zinc-900/40">
                  <div
                    v-for="row in g.items"
                    :key="row.id"
                    class="flex items-center justify-between gap-3 border-b border-zinc-200/60 px-4 py-2.5 last:border-b-0 dark:border-white/[0.05]"
                  >
                    <div class="flex min-w-0 items-center gap-2">
                      <span class="truncate text-xs text-zinc-900 dark:text-zinc-200">{{ t(row.description) }}</span>
                      <span
                        v-if="row.customized"
                        class="shrink-0 rounded-full bg-purple-100 px-1.5 py-px text-xxs font-medium text-purple-700 dark:bg-purple-500/15 dark:text-purple-300"
                      >
                        {{ t('settings.customized') }}
                      </span>
                    </div>
                    <div class="flex shrink-0 items-center gap-1.5">
                      <button
                        type="button"
                        class="sc-key-btn"
                        :class="{ recording: recordingId === row.id }"
                        :title="recordingId === row.id ? t('settings.recordingHint') : t('settings.rerecordHint')"
                        @click="recordingId === row.id ? cancelRecording() : startRecording(row.id)"
                      >
                        {{ recordingId === row.id ? t('settings.pressKeys') : bindingLabel(row.effective) }}
                      </button>
                      <button
                        v-if="row.customized"
                        type="button"
                        class="sc-reset-btn"
                        :title="t('settings.resetKeyHint')"
                        :aria-label="t('settings.resetKeyHint')"
                        @click="resetOneShortcut(row.id)"
                      >
                        <Icon name="refresh" :size="12" />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <!-- 自增序列 -->
            <section v-if="activeTab === 'sequences'">
              <header class="mb-6">
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.sequencesTitle') }}</h2>
                <p class="mt-1 text-xs text-zinc-600 dark:text-zinc-400">
                  {{ t('settings.seqDescA') }} <code class="rounded bg-zinc-100 px-1.5 py-0.5 font-mono text-xxs text-purple-600 dark:bg-white/5 dark:text-purple-300">&#123;&#123;$seq:key&#125;&#125;</code> {{ t('settings.seqDescB') }}
                </p>
              </header>

              <div class="space-y-4">
                <!-- 新增自增序列卡片 -->
                <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-4 dark:border-white/[0.06] dark:bg-zinc-900/40">
                  <div class="mb-3">
                    <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.seqAddTitle') }}</div>
                    <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">
                      {{ t('settings.seqGlobalHint') }} <code class="font-mono text-xxs text-zinc-700 dark:text-zinc-300">&#123;&#123;$seq&#125;&#125;</code>
                    </p>
                  </div>
                  <div class="flex items-center gap-2">
                    <input
                      v-model="newSeqKey"
                      type="text"
                      class="miyo-input flex-1 font-mono text-xs"
                      :placeholder="t('settings.seqKeyPh')"
                      spellcheck="false"
                      @keydown.enter="addSeq"
                    />
                    <div class="w-28 shrink-0">
                      <CustomNumberInput
                        :model-value="newSeqValue"
                        :min="1"
                        size="md"
                        :placeholder="t('settings.seqStartPh')"
                        @change="(v) => (newSeqValue = v)"
                      />
                    </div>
                    <button
                      class="miyo-btn miyo-btn-primary shrink-0"
                      type="button"
                      @click="addSeq"
                    >
                      <Icon name="plus" :size="13" />
                      <span>{{ t('settings.seqAdd') }}</span>
                    </button>
                  </div>
                </div>

                <!-- 序列清单列表卡片 -->
                <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-1 dark:border-white/[0.06] dark:bg-zinc-900/40">
                  <div class="flex items-center justify-between px-4 py-3">
                    <div class="text-xs font-medium uppercase tracking-wider text-zinc-500 dark:text-zinc-400">{{ t('settings.seqListTitle') }}</div>
                    <span class="text-xxs text-zinc-500">{{ t('settings.seqAutoSave') }}</span>
                  </div>

                  <div v-if="counters.length" class="mx-2 mb-2 overflow-hidden rounded-lg border border-zinc-200/70 bg-white dark:border-white/[0.06] dark:bg-black/20">
                    <table class="w-full border-collapse text-left text-xs">
                      <thead>
                        <tr class="border-b border-zinc-200/70 bg-zinc-50/60 text-xxs text-zinc-500 dark:border-white/[0.06] dark:bg-white/[0.02] dark:text-zinc-400">
                          <th class="px-3.5 py-2.5 font-medium">{{ t('settings.seqColKey') }}</th>
                          <th class="w-36 px-3.5 py-2.5 font-medium">{{ t('settings.seqColNext') }}</th>
                          <th class="w-24 px-3.5 py-2.5 text-right font-medium">{{ t('settings.seqColOps') }}</th>
                        </tr>
                      </thead>
                      <tbody class="divide-y divide-zinc-200/70 dark:divide-white/[0.06]">
                        <tr
                          v-for="c in counters"
                          :key="c.key || '__global__'"
                          class="transition-colors hover:bg-zinc-50/50 dark:hover:bg-white/[0.02]"
                        >
                          <td class="px-3.5 py-2.5">
                            <div class="flex items-center gap-2 font-mono">
                              <span
                                v-if="!c.key"
                                class="inline-flex items-center rounded bg-purple-100 px-1.5 py-0.5 text-xxs font-medium text-purple-700 dark:bg-purple-500/15 dark:text-purple-300"
                              >
                                {{ t('envmgr.groupGlobal') }}
                              </span>
                              <span class="text-zinc-900 dark:text-zinc-200">
                                <template v-if="c.key">&#123;&#123;$seq:{{ c.key }}&#125;&#125;</template>
                                <template v-else>&#123;&#123;$seq&#125;&#125;</template>
                              </span>
                            </div>
                          </td>
                          <td class="px-3.5 py-2.5">
                            <CustomNumberInput
                              :model-value="c.value"
                              :min="1"
                              size="sm"
                              class="w-24"
                              @change="(v) => { c.value = v; saveSeq(c) }"
                            />
                          </td>
                          <td class="px-3.5 py-2.5 text-right">
                            <div class="flex items-center justify-end gap-1">
                              <button
                                class="miyo-btn miyo-btn-sm miyo-btn-ghost"
                                type="button"
                                :title="t('settings.seqResetTitle')"
                                :aria-label="t('settings.seqResetTitle')"
                                @click="resetSeq(c)"
                              >
                                <Icon name="refresh" :size="12" />
                              </button>
                              <Popconfirm
                                :title="t('settings.seqDeleteTitle')"
                                :confirm-text="t('common.delete')"
                                @confirm="deleteSeq(c)"
                              >
                                <button
                                  class="miyo-btn miyo-btn-sm miyo-btn-ghost text-zinc-500 hover:text-red-500 dark:text-zinc-400 dark:hover:text-red-400"
                                  type="button"
                                  :aria-label="t('settings.seqDeleteTitle')"
                                >
                                  <Icon name="trash" :size="12" />
                                </button>
                              </Popconfirm>
                            </div>
                          </td>
                        </tr>
                      </tbody>
                    </table>
                  </div>

                  <!-- 优雅 Empty State -->
                  <div
                    v-else
                    class="flex flex-col items-center justify-center py-10 text-center"
                  >
                    <div class="mb-2 flex h-9 w-9 items-center justify-center rounded-full bg-zinc-200/50 text-zinc-500 dark:bg-white/5 dark:text-zinc-500">
                      <Icon name="list" :size="16" />
                    </div>
                    <p class="text-xs text-zinc-700 dark:text-zinc-400">{{ t('settings.seqEmpty') }}</p>
                    <p class="mt-0.5 text-xxs text-zinc-500">{{ t('settings.seqEmptyHint') }}</p>
                  </div>
                </div>
              </div>
            </section>

            <!-- 数据与备份 -->
            <section v-if="activeTab === 'data'">
              <header>
                <h2 class="text-base font-bold text-ink1">{{ t('settings.data') }}</h2>
                <p class="my-2.5 text-xs leading-relaxed text-ink3">{{ t('settings.dataDesc') }}</p>
              </header>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="flex items-center gap-3">
                  <div
                    class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-purple-100 text-purple-700 dark:bg-purple-500/15 dark:text-purple-400"
                  >
                    <Icon name="package" :size="18" />
                  </div>
                  <div class="min-w-0">
                    <div class="truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">
                      {{ project ? project.name : t('settings.noProject') }}
                    </div>
                    <p class="mt-0.5 truncate text-xs text-zinc-600 dark:text-zinc-400">{{ projectSummary }}</p>
                  </div>
                </div>

                <div class="mt-4 flex items-center gap-2 border-t border-zinc-200/70 pt-4 dark:border-white/[0.06]">
                  <button
                    class="miyo-btn"
                    type="button"
                    :disabled="!project || busy"
                    @click="exportBackup"
                  >
                    <Icon name="download" :size="13" />
                    {{ busy ? t('workspace.processing') : t('settings.exportBackup') }}
                  </button>
                  <button class="miyo-btn" type="button" :disabled="busy" @click="fileInput?.click()">
                    <Icon name="upload" :size="13" />
                    {{ t('settings.importRestore') }}
                  </button>
                  <input
                    ref="fileInput"
                    type="file"
                    accept=".json,application/json"
                    class="hidden"
                    @change="onImportFile"
                  />
                </div>
              </div>

              <div class="rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-5 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.dataDir') }}</div>
                <!-- 路径井与次级按钮组同行等高（--h-md = 32px）：路径超长单行截断，完整值走 title + 复制按钮 -->
                <div class="datadir-row">
                  <div class="datadir-path" :title="dataDir || undefined">
                    <span class="datadir-path-text">{{ dataDir || '…' }}</span>
                    <button
                      class="datadir-copy-btn"
                      :class="{ copied: copiedDataDir }"
                      type="button"
                      :title="copiedDataDir ? t('settings.dataDirCopied') : t('settings.dataDirCopy')"
                      :aria-label="copiedDataDir ? t('settings.dataDirCopied') : t('settings.dataDirCopy')"
                      :disabled="!dataDir"
                      @click="copyDataDir"
                    >
                      <Icon :name="copiedDataDir ? 'check' : 'copy'" :size="13" />
                    </button>
                  </div>
                  <div class="datadir-actions">
                    <button class="miyo-btn" type="button" @click="pickDataDir">
                      {{ t('settings.dataDirChange') }}
                    </button>
                    <button
                      class="miyo-btn"
                      type="button"
                      :disabled="isDefaultDir"
                      @click="resetDataDirToDefault"
                    >
                      {{ t('settings.dataDirReset') }}
                    </button>
                    <button
                      class="miyo-btn"
                      type="button"
                      :disabled="!dataDir"
                      @click="revealDataDir"
                    >
                      {{ t('settings.dataDirReveal') }}
                    </button>
                  </div>
                </div>
                <p class="mt-2 text-xxs leading-relaxed text-ink3">{{ t('settings.dataDirDesc') }}</p>
                <p v-if="needRestart" class="mt-3 text-xs text-zinc-600 dark:text-zinc-400">
                  {{ t('settings.dataDirRestartNow') }}
                  <button class="miyo-btn miyo-btn-sm ml-2" type="button" @click="restartNow">
                    {{ t('settings.restartNow') }}
                  </button>
                </p>
              </div>
            </section>

            <!-- 环境管理 -->
            <section v-if="activeTab === 'environments'">
              <header>
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.environments') }}</h2>
                <p class="mt-1 mb-4 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.environmentsDesc') }}</p>
                <div class="mb-4 flex items-center gap-2">
                  <span class="shrink-0 text-xs text-zinc-500 dark:text-zinc-400">{{ t('settings.envProject') }}</span>
                  <CustomSelect
                    :model-value="envProjectId"
                    :options="envProjectOptions"
                    size="sm"
                    class="w-64"
                    @update:model-value="onEnvProjectChange"
                  />
                </div>
              </header>

              <div class="space-y-2">
                <div
                  v-if="!envProjectId && !envLoading"
                  class="rounded-lg border border-zinc-200/70 bg-zinc-50/80 p-6 text-center text-xs text-zinc-600 dark:border-white/[0.06] dark:bg-zinc-900/40 dark:text-zinc-500"
                >
                  {{ t('settings.envNoProject') }}
                </div>
                <div
                  v-for="env in environments"
                  :key="env.id"
                  class="flex items-center justify-between gap-3 rounded-lg border border-zinc-200/70 bg-zinc-50/80 p-3.5 dark:border-white/[0.06] dark:bg-zinc-900/40"
                >
                  <div class="flex min-w-0 items-center gap-2.5">
                    <span
                      class="sd-dot"
                      :class="env.id === activeEnvId ? 'sd-dot-active' : ''"
                      aria-hidden="true"
                    ></span>
                    <div class="min-w-0">
                      <div class="flex items-center gap-1.5">
                        <span class="truncate text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ env.name }}</span>
                        <span
                          v-if="env.id === activeEnvId"
                          class="rounded-full bg-emerald-100 px-1.5 py-px text-xxs font-medium text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-400"
                        >
                          {{ t('envmgr.current') }}
                        </span>
                      </div>
                      <div class="truncate font-mono text-xxs text-zinc-600 dark:text-zinc-500">
                        {{ envBase(env) || t('settings.envNoBase') }}
                      </div>
                    </div>
                  </div>
                  <div class="flex shrink-0 items-center gap-2">
                    <span
                      class="rounded-full bg-zinc-200/70 px-2 py-0.5 text-xxs text-zinc-600 dark:bg-white/[0.04] dark:text-zinc-400"
                    >
                      {{ t('settings.envVarCount', { n: envVarCount(env) }) }}
                    </span>
                    <button
                      type="button"
                      class="rounded-md px-2 py-1 text-xxs text-zinc-500 transition-colors duration-150 hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-white/[0.06] dark:hover:text-zinc-200"
                      @click="openEnvironmentManager(env.id)"
                    >
                      {{ t('common.edit') }}
                    </button>
                  </div>
                </div>

                <div
                  v-if="envProjectId && !environments.length && !envLoading"
                  class="rounded-lg border border-zinc-200/70 bg-zinc-50/80 p-6 text-center text-xs text-zinc-600 dark:border-white/[0.06] dark:bg-zinc-900/40 dark:text-zinc-500"
                >
                  {{ t('settings.envEmpty') }}
                </div>
              </div>

              <div class="mt-4 border-t border-zinc-200/70 pt-4 dark:border-white/[0.06]">
                <button
                  class="miyo-btn w-full"
                  type="button"
                  :disabled="!envProjectId"
                  @click="openEnvironmentManager()"
                >
                  <Icon name="settings" :size="13" />
                  {{ t('settings.envOpenManager') }}
                </button>
              </div>
            </section>

            <!-- 日志 -->
            <section v-if="activeTab === 'logs'">
              <header>
                <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.logs') }}</h2>
                <p class="mt-1 mb-5 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.logsDesc') }}</p>
              </header>

              <div class="mb-4 flex items-center justify-between gap-4 rounded-xl border border-zinc-200/70 bg-zinc-50/80 p-4 dark:border-white/[0.06] dark:bg-zinc-900/40">
                <div class="max-w-md">
                  <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.logRetention') }}</div>
                  <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.logRetentionDesc') }}</p>
                </div>
                <div class="relative w-28 shrink-0">
                  <CustomNumberInput
                    :model-value="logRetention"
                    :min="1"
                    :max="365"
                    size="md"
                    tone="inset"
                    :disabled="logRetentionBusy"
                    @change="saveLogRetention"
                  />
                  <span
                    class="pointer-events-none absolute right-[28px] top-1/2 -translate-y-1/2 text-xs text-zinc-500"
                  >
                    {{ t('settings.logRetentionUnit') }}
                  </span>
                </div>
              </div>

              <div class="mb-3 flex items-center gap-2">
                <CustomSelect
                  v-model="logSelected"
                  :options="logFileOptions"
                  size="sm"
                  class="max-w-60 flex-1"
                  @update:model-value="loadLogTail"
                />
                <button class="miyo-btn miyo-btn-sm" type="button" :disabled="logLoading" @click="loadLogTail">
                  <Icon name="refresh" :size="13" /> {{ logLoading ? t('common.loading') : t('common.refresh') }}
                </button>
                <button class="miyo-btn miyo-btn-sm" type="button" @click="openLogDir">
                  <Icon name="folder" :size="13" /> {{ t('settings.logsOpenDir') }}
                </button>
              </div>
              <pre v-if="logContent" class="log-view">{{ logContent }}</pre>
              <p v-else class="text-xs text-zinc-500">{{ t('settings.logsEmpty') }}</p>
            </section>
          </div>
        </Transition>
      </div>
    </div>

    <EnvironmentManager
      v-model:open="showManager"
      :initial-env-id="managerEnvId"
      :project-id="envProjectId || null"
      :project-name="envProjectName || null"
    />
  </Modal>
</template>

<style scoped>
/* 选中态紫色填充（Tab 高亮条 / 主题·语言 radio chip）：走 --primary 令牌
   （浅色降刺眼 violet-600，深色 violet-500），替代字面 bg-purple-600 */
.su-tab-marker,
.su-chip-active {
  background: var(--primary);
}

.pane-enter-active,
.pane-leave-active {
  transition:
    opacity 160ms var(--ease),
    transform 160ms var(--ease);
}
.pane-enter-from {
  opacity: 0;
  transform: translateY(4px);
}
.pane-leave-to {
  opacity: 0;
  transform: translateY(-2px);
}

/* 环境概览状态点：默认灰，激活绿 + 柔光 */
.sd-dot {
  width: 8px;
  height: 8px;
  border-radius: 9999px;
  background: var(--text-3);
  flex-shrink: 0;
}
.sd-dot-active {
  background: var(--success);
  box-shadow: 0 0 0 3px var(--success-tint);
}

/* 数据目录：路径井 + 次级按钮组同行。两者等高（--h-md = 32px，与面板内
   「导出备份」等按钮同档），路径单行截断（完整值另给 title / 复制按钮），
   颜色全走主题变量——字面 slate-100/200/700 在深色下失真。 */
.datadir-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
}
.datadir-path {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  height: var(--h-md);
  padding: 0 3px 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-1);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  line-height: 1;
  color: var(--text-2);
  cursor: text;
}
.datadir-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.datadir-path-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}
.datadir-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.datadir-copy-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-1);
}
.datadir-copy-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.datadir-copy-btn:disabled {
  opacity: 0.4;
  cursor: default;
}
.datadir-copy-btn.copied {
  color: var(--success);
}
.datadir-copy-btn.copied:hover {
  color: var(--success);
}
.log-view {
  margin: 0;
  max-height: 320px;
  overflow: auto;
  padding: 10px 12px;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  background: var(--bg-card);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-2);
}

/* 快捷键录制按钮：kbd 风格，录制态主题色呼吸 */
.sc-key-btn {
  min-width: 110px;
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-bottom-width: 2px;
  border-radius: 6px;
  background: var(--bg-hover);
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  white-space: nowrap;
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.sc-key-btn:hover {
  border-color: var(--accent);
}
.sc-key-btn.recording {
  border-color: var(--accent);
  color: var(--accent);
  animation: sc-key-pulse 1.2s ease-in-out infinite;
}
@keyframes sc-key-pulse {
  0%, 100% { box-shadow: 0 0 0 0 var(--accent-tint); }
  50% { box-shadow: 0 0 0 5px transparent; }
}
.sc-reset-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.sc-reset-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
@media (prefers-reduced-motion: reduce) {
  .sc-key-btn.recording {
    animation: none;
  }
}
</style>

<style>
/* 设置弹窗容器：覆写 Modal 的 CSS 变量。
   深色 = 深邃高级暗色底 + 极细白描边 + 柔和深影；浅色 = macOS 原生白色面板质感。 */
.sd-dialog {
  /* 表面比通用 elevated 深一档 + 更重投影；双主题值走 --dialog-* 别名 */
  --bg-elevated: var(--dialog-bg);
  --border-strong: var(--dialog-border-strong);
  --radius-lg: 16px;
  --shadow-lg: var(--dialog-shadow);
}

/* 弹窗骨架：主体贴边（侧栏分隔线与选中底直达面板边缘，不再被 m-body 内距
   切出一圈留白），标题栏加 hairline 与内容分层。
   ⚠ 选择器必须带 .m-dialog 提权：单写 .sd-dialog .m-body 与 Modal 自身的
   `.m-body[data-v-*]` 同特异度（0,2,0），胜负取决于打包后的样式表顺序（仓内实证坑）。 */
.m-dialog.sd-dialog .m-head {
  padding: 13px 16px 12px;
  border-bottom: 1px solid var(--border);
}
.m-dialog.sd-dialog .m-body {
  padding: 0;
}

/* 通用设置「发现新版本」高亮卡片：深色 = 紫→锌渐变，浅色 = 浅紫→浅灰渐变
   （深色系半透渐变在浅底上会压暗文字），双主题值走 --update-card-bg。 */
.su-update-card {
  background: var(--update-card-bg);
}
</style>
