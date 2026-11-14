<script setup lang="ts">
/**
 * SettingsDialog：设置弹框（Linear / Raycast 风格暗黑双栏设置面板）。
 *
 * 左栏：轻量 Menu List 导航（扁平行高 + 左侧紫色指示条）；
 * 右栏：卡片化设置组，Tab 切换淡入；简单项改动即自动保存。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { relaunch } from '@tauri-apps/plugin-process'
import { copyText } from '@/utils/clipboard'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useThemeStore, type ThemeMode } from '@/stores/theme'
import { useLocaleStore, type LocaleMode } from '@/stores/locale'
import { useWorkspaceStore, FALLBACK_GLOBAL_TIMEOUT_MS } from '@/stores/workspace'
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
} from '@/composables/useShortcuts'
import {
  clearSkippedUpdateVersion,
  clearUpdateLatches,
  debugFailNextCheck,
  debugRunCheckNow,
  debugSimulateUpdate,
  debugUpdateState,
  hasPendingUpdate,
  pendingUpdateVersion,
  requestOpenAbout,
  requestOpenRelease,
  skippedUpdateVersion,
} from '@/domains/workspace/composables/useAutoUpdate'
import { version as appVersion } from '../../../../package.json'
import Modal from '@/components/ui/Modal.vue'
import Icon, { type IconName } from '@/components/ui/Icon.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import CustomNumberInput from '@/components/ui/CustomNumberInput.vue'
import SettingsMcpTab from '@/domains/agent/components/settings/SettingsMcpTab.vue'
import SettingsAiTab from '@/domains/agent/components/settings/SettingsAiTab.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EnvWorkspace from '@/domains/environment/components/env/EnvWorkspace.vue'
import { errorText } from '@/utils/errorText'

import type {
  LogFile,
  Project,
  ProjectStat,
  SeqCounter,
} from '@/types/miyoApi.d'

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
type TabId = 'general' | 'network' | 'ai' | 'mcp' | 'shortcuts' | 'sequences' | 'data' | 'environments' | 'logs'
interface TabDef {
  id: TabId
  label: string
  icon: IconName
}
const tabs = computed<TabDef[]>(() => [
  { id: 'general', label: t('settings.general'), icon: 'settings' },
  { id: 'network', label: t('settings.network'), icon: 'globe' },
  { id: 'ai', label: t('settings.ai.tab'), icon: 'zap' },
  { id: 'mcp', label: t('settings.mcp'), icon: 'plug' },
  { id: 'shortcuts', label: t('settings.shortcuts'), icon: 'keyboard' },
  { id: 'sequences', label: t('settings.sequences'), icon: 'list' },
  { id: 'data', label: t('settings.data'), icon: 'folder' },
  { id: 'environments', label: t('settings.environments'), icon: 'beaker' },
  { id: 'logs', label: t('settings.logs'), icon: 'file' },
])
const activeTab = ref<TabId>('general')

// ---------- 环境工作区（内嵌，常驻挂载） ----------
// 「环境」页签直接内嵌 EnvWorkspace 统一工作区（一层直达，无二级弹窗）；
// 跨项目下拉保留在页签头，非当前项目自动进入工作区的 detached 独立管理模式。
// 首次激活才挂载（懒加载），之后 v-show 切换——返回环境管理不重挂载、不闪、不丢状态。
const envWorkspace = ref<InstanceType<typeof EnvWorkspace> | null>(null)
const envTabVisited = ref(false)

const project = ref<Project | null>(null)
const projectStat = ref<ProjectStat | null>(null)
const busy = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)

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
}

// ---------- 页签切换 / 弹窗关闭守卫 ----------
// 环境工作区常驻挂载（首次激活后 v-show 切换，不随页签切换卸载）：切页签不丢
// 未保存修改、无重挂载闪烁；仅在关闭整个设置弹窗（组件卸载）时经三选确认拦截。

/** 切页签：直接切换（环境工作区 v-show 常驻，无需守卫）。 */
function onTabClick(id: TabId): void {
  if (id === 'environments') envTabVisited.value = true
  activeTab.value = id
}

/** 关闭设置弹窗守卫：环境工作区常驻挂载（切页签不卸载），只要存在未保存修改
 *  （无论当前在哪个页签）都拦截并弹三选确认，避免静默丢失编辑。 */
function guardSettingsClose(): boolean {
  const ws = envWorkspace.value
  if (!ws || !ws.hasPending()) return true
  ws.guardAction(() => emit('close'))
  return false
}

onMounted(async () => {
  // 各项互不依赖，并发加载（打开耗时 ≈ 最慢一项而非逐项之和）。
  // project 先行：loadEnvProjects 的默认选中与项目统计都依赖它；
  // MCP 状态/工具清单由 MCP 页签自行加载（首访挂载时）。
  project.value = await api.getActiveProject().catch(() => null)
  if (project.value) {
    void api
      .listProjectStats()
      .then((stats) => {
        projectStat.value = stats.find((s) => s.project_id === project.value?.id) ?? null
      })
      .catch(() => {
        projectStat.value = null
      })
  }
  await Promise.all([
    api
      .getHttpProxy()
      .then((v) => {
        proxyUrl.value = v ?? ''
        proxyEnabled.value = !!proxyUrl.value
      })
      .catch(() => {
        proxyUrl.value = ''
        proxyEnabled.value = false
      }),
    api
      .getTlsSettings()
      .then((tls) => {
        tlsIdentityPath.value = tls.identityPath ?? ''
        tlsAcceptInvalid.value = !!tls.acceptInvalidCerts
      })
      .catch(() => {
        tlsIdentityPath.value = ''
        tlsAcceptInvalid.value = false
      }),
    api
      .getHttpTimeoutMs()
      .then((ms) => {
        timeoutSec.value = ms != null ? Math.round(ms / 1000) : DEFAULT_TIMEOUT_SEC
      })
      .catch(() => {
        timeoutSec.value = DEFAULT_TIMEOUT_SEC
      }),
    loadEnvProjects(),
    loadCounters(),
    loadDataDirs(),
  ])
  reloadSkipped()
})

// ---------- 通用设置：请求超时（自动保存） ----------
/** 全局超时兜底值（秒）：与后端执行链路的内置默认同源（workspace 常量镜像 miyo-http）。 */
const DEFAULT_TIMEOUT_SEC = FALLBACK_GLOBAL_TIMEOUT_MS / 1000
const timeoutSec = ref(DEFAULT_TIMEOUT_SEC)

/** 编辑中实时回显（与其余 CustomNumberInput 用法一致；曾单向绑定导致手输被丢弃，#134）。 */
function onTimeoutModelUpdate(v: string | number): void {
  if (v === '') return
  const n = typeof v === 'number' ? v : Number(v)
  if (Number.isFinite(n)) timeoutSec.value = n
}

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
    toast.error(t('settings.saveFail'), { message: errorText(err) })
  }
}

// ---------- 网络与代理 ----------
const proxyUrl = ref('')
const proxyEnabled = ref(false)
const proxyBusy = ref(false)
const proxyTesting = ref(false)
const proxyTest = ref<{ ok: boolean; message: string } | null>(null)

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
    toast.error(t('settings.tlsSaveFail'), { message: errorText(err) })
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
    toast.error(t('settings.proxySaveFail'), { message: errorText(err) })
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
    proxyTest.value = { ok: false, message: errorText(err) }
    toast.error(t('settings.proxyTestFail'), { message: proxyTest.value.message })
  } finally {
    proxyTesting.value = false
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
    toast.error(t('settings.saveFail'), { message: errorText(err) })
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
    toast.error(t('settings.deleteFail'), { message: errorText(err) })
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
    toast.error(t('settings.addFail'), { message: errorText(err) })
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
      message: errorText(err),
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
      message: errorText(err),
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
      message: errorText(err),
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
      const path = await api.joinPath(dir, filename)
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
    toast.error(t('settings.exportFail'), { message: errorText(err) })
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
    const scenarioCount = summary.scenarios ?? 0
    if (scenarioCount > 0) extras.push(t('settings.importScenarios', { n: scenarioCount }))
    toast.success(t('settings.importRestored', { name: summary.name, endpoints: summary.endpoints, envs: summary.environments }), {
      message: extras.join(locale.resolved === 'zh' ? '；' : '; ') || undefined,
    })
    // 恢复落库为「全新项目」：通知宿主刷新列表/工作区，否则需重启才能看到
    emit('restored', summary.id)
    emit('close')
  } catch (err) {
    toast.error(t('settings.importFail'), { message: errorText(err) })
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
      message: errorText(err),
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
    toast.error(t('settings.logReadFail'), { message: errorText(err) })
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

/** 关于入口：关设置 → 打开关于弹窗（版本号 / 检查更新 / 下载安装都在其中）。 */
function openAbout(): void {
  emit('close')
  requestOpenAbout()
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
  <Modal
    :open="true"
    :title="t('settings.title')"
    width="min(1200px, 94vw)"
    dialog-class="sd-dialog"
    :guard-close="guardSettingsClose"
    @close="emit('close')"
  >
    <!-- 弹窗主体恒定高度：切页签不改变弹窗尺寸（高度跳变是闪烁感来源之一） -->
    <div class="flex h-[min(680px,80vh)]">
      <!-- 左：极简 List 导航（.m-body 已贴边，内距由本栏自持；分隔线与标题栏 hairline 同源）。
           窄屏优先收缩导航（min(220px, 18vw)），保住右侧变量表格不被挤压 -->
      <aside class="flex w-[min(220px,18vw)] shrink-0 flex-col gap-1 border-r border-line p-3">
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
          @click="onTabClick(tab.id)"
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

      <!-- 右：内容区。环境工作区是绝对定位覆盖层（首访挂载后 v-show 常驻，切换不重挂载）；
           其余页签走 keyed 淡入淡出。容器类恒定，切换只更新内容不抖布局。 -->
      <div class="relative flex-1 min-w-0 overflow-y-auto p-6">
        <!-- 环境工作区（覆盖层） -->
        <Transition name="pane">
          <section
            v-if="envTabVisited"
            v-show="activeTab === 'environments'"
            class="absolute inset-0 flex flex-col p-6"
          >
            <header class="shrink-0">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.environments') }}</h2>
                  <p class="mt-1 text-xs text-zinc-600 dark:text-zinc-500">{{ t('settings.environmentsDesc') }}</p>
                </div>
                <div class="flex shrink-0 items-center gap-2 pt-0.5">
                  <span class="text-xs text-zinc-500 dark:text-zinc-400">{{ t('settings.envProject') }}</span>
                  <CustomSelect
                    :model-value="envProjectId"
                    :options="envProjectOptions"
                    size="sm"
                    class="w-56"
                    @update:model-value="onEnvProjectChange"
                  />
                </div>
              </div>
            </header>

            <div
              v-if="!envProjectId"
              class="mt-4 flex flex-1 items-center justify-center rounded-lg border border-zinc-200/70 bg-zinc-50/80 text-center text-xs text-zinc-600 dark:border-white/[0.06] dark:bg-zinc-900/40 dark:text-zinc-500"
            >
              {{ t('settings.envNoProject') }}
            </div>
            <div v-else class="mt-3 min-h-0 flex-1">
              <EnvWorkspace
                ref="envWorkspace"
                embedded
                :project-id="envProjectId || null"
                :project-name="envProjectName || null"
              />
            </div>
          </section>
        </Transition>

        <!-- 其余页签内容（keyed 淡入淡出） -->
        <Transition name="pane" mode="out-in">
          <div v-if="activeTab !== 'environments'" :key="activeTab" class="space-y-4">
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
                      @update:model-value="onTimeoutModelUpdate"
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
                <!-- 关于 Miyo：常驻入口（右侧展示当前版本号，点击打开关于弹窗——检查更新 / 下载安装） -->
                <div class="mt-5 border-t border-zinc-200/70 dark:border-white/[0.06]">
                  <button
                    type="button"
                    class="flex w-full items-center justify-between gap-4 pt-5 text-left"
                    @click="openAbout"
                  >
                    <div class="max-w-md">
                      <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.about') }}</div>
                      <p class="mt-0.5 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.aboutDesc') }}</p>
                    </div>
                    <span class="shrink-0 font-mono text-xs text-zinc-500 dark:text-zinc-400">v{{ appVersion }}</span>
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
            <SettingsAiTab v-if="activeTab === 'ai'" />
            <SettingsMcpTab v-if="activeTab === 'mcp'" />

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

            <!-- 环境页签已迁出：见上方绝对定位覆盖层（常驻挂载，v-show 切换） -->

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
  </Modal>
</template>

<style scoped>
/* 选中态紫色填充（Tab 高亮条 / 主题·语言 radio chip）：走 --primary 令牌
   （浅色降刺眼 violet-600，深色 violet-500），替代字面 bg-purple-600 */
.su-tab-marker,
.su-chip-active {
  background: var(--primary);
}

/* 页签切换 / 环境覆盖层淡入：120~160ms 规范区间，轻微透明度 + 位移 */
.pane-enter-active,
.pane-leave-active {
  transition:
    opacity 140ms var(--ease),
    transform 140ms var(--ease);
}
.pane-enter-from {
  opacity: 0;
  transform: translateY(4px);
}
.pane-leave-to {
  opacity: 0;
  transform: translateY(-2px);
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
  border-radius: var(--radius);
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
  height: var(--h-xs);
  border: none;
  border-radius: var(--radius);
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
