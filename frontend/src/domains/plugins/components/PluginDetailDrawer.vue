<script setup lang="ts">
/**
 * PluginDetailDrawer：插件详情抽屉（插件管理中心）。
 *
 * 页签：概览（元信息 / 运行指标 / nav 声明）/ 权限（批准快照 + 收权）/
 * 配置（configSchema 驱动表单 → 插件私有存储 __config）/ 日志（级别过滤，
 * audit 置顶展示）。
 *
 * 数据源：manager.entries（视图 + 指标）+ 后端 plugin_logs / plugin_data_* IPC。
 */
import { computed, ref, watch } from 'vue'
import Drawer from '@/components/ui/Drawer.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import Tabs, { type TabItem } from '@/components/ui/Tabs.vue'
import Icon from '@/components/ui/Icon.vue'
import Spinner from '@/components/ui/Spinner.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { usePluginManager } from '@/plugins/manager'
import { lookupPermission } from '@/plugins/permissions'
import { errorText } from '@/utils/errorText'
import { invoke } from '@tauri-apps/api/core'
import type { PluginManifest } from '@/types/generated/PluginManifest'
import type { PluginNavItem } from '@/types/generated/PluginNavItem'
import type { PluginView } from '@/types/generated/PluginView'
import PluginConfigForm from '@/domains/plugins/components/PluginConfigForm.vue'

const props = defineProps<{
  open: boolean
  pluginId: string | null
}>()

const emit = defineEmits<{
  close: []
  /** 权限 / 配置变更（父级刷新列表与目录）。 */
  changed: []
}>()

const t = useLocaleStore().t
const toast = useToast()
const manager = usePluginManager()

const IPC_PREFIX = 'plugin:miyo|'

const view = computed<PluginView | null>(() =>
  props.pluginId ? (manager.entries.get(props.pluginId)?.view ?? null) : null,
)
const entry = computed(() => (props.pluginId ? manager.entries.get(props.pluginId) : undefined))

// ---------- 页签 ----------
const tab = ref('overview')
const tabItems = computed<TabItem[]>(() => [
  { key: 'overview', label: t('plugins.detail.overview') },
  { key: 'permissions', label: t('plugins.detail.permissions') },
  { key: 'config', label: t('plugins.detail.config') },
  { key: 'logs', label: t('plugins.detail.logs') },
])
watch(
  () => props.open,
  (open) => {
    if (open) tab.value = 'overview'
  },
)

// ---------- 完整性 / Sidecar 状态 / 版本历史 ----------

const verifying = ref(false)
async function onVerifyIntegrity(): Promise<void> {
  const v = view.value
  if (!v) return
  verifying.value = true
  try {
    const ok = await invoke<boolean>(`${IPC_PREFIX}plugin_verify_integrity`, { id: v.id })
    if (ok) toast.success(t('plugins.integrityOkToast'))
    await manager.refreshInstalled()
    emit('changed')
  } catch (e) {
    toast.error(t('plugins.integrityFailedToast'), { message: errorText(e) })
  } finally {
    verifying.value = false
  }
}

interface SidecarStatusView {
  plugin_id: string
  pid: number
  sidecar_version: string
  restarts_last_hour: number
}
const sidecarStatus = ref<SidecarStatusView | null>(null)

interface PluginVersionRow {
  version: string
  created_at: string
  signed: boolean
}
const versionHistory = ref<PluginVersionRow[]>([])

async function loadRuntimeInfo(): Promise<void> {
  const v = view.value
  if (!v) return
  // sidecar 状态（ui-sidecar 形态才查）
  if (v.kind === 'ui-sidecar') {
    try {
      const all = await invoke<SidecarStatusView[]>(`${IPC_PREFIX}plugin_sidecar_status`)
      sidecarStatus.value = all.find((s) => s.plugin_id === v.id) ?? null
    } catch {
      sidecarStatus.value = null
    }
  } else {
    sidecarStatus.value = null
  }
  // 版本历史（本地包才有回滚语义）
  if (v.source !== 'builtin') {
    try {
      versionHistory.value = await invoke<PluginVersionRow[]>(`${IPC_PREFIX}plugin_versions`, {
        id: v.id,
      })
    } catch {
      versionHistory.value = []
    }
  } else {
    versionHistory.value = []
  }
}

async function onRollback(): Promise<void> {
  const v = view.value
  if (!v) return
  try {
    const done = await invoke<{ version: string }>(`${IPC_PREFIX}plugin_rollback`, { id: v.id })
    toast.success(t('plugins.rollbackDoneToast', { version: done.version }))
    await manager.refreshInstalled()
    await loadRuntimeInfo()
    emit('changed')
  } catch (e) {
    toast.error(t('plugins.rollbackFailed'), { message: errorText(e) })
  }
}

// ---------- 权限 ----------

/** 已批准集合（本地编辑态；保存即落库快照）。 */
const grantedDraft = ref<string[]>([])
watch(
  () => view.value?.granted_permissions,
  (granted) => {
    grantedDraft.value = [...(granted ?? [])]
  },
  { immediate: true },
)

const permSaving = ref(false)
async function savePermissions(): Promise<void> {
  const v = view.value
  if (!v) return
  permSaving.value = true
  try {
    await invoke(`${IPC_PREFIX}plugin_set_permissions`, {
      id: v.id,
      grantedPermissions: grantedDraft.value,
    })
    // 后端已吊销该插件全部会话（撤销即时生效）——本地凭证同步作废
    sessionToken.value = null
    configPluginId.value = null
    toast.success(t('plugins.permSavedToast'))
    await manager.refreshInstalled()
    emit('changed')
  } catch (e) {
    toast.error(t('plugins.permSaveFailed'), { message: errorText(e) })
  } finally {
    permSaving.value = false
  }
}

function riskTone(risk: string | undefined): StatusTone {
  if (risk === 'high') return 'fail'
  if (risk === 'medium') return 'warn'
  return 'info'
}
function permLabel(p: string): string {
  return t(lookupPermission(p)?.labelKey ?? 'plugins.permUnknown')
}

// ---------- 配置 ----------

const configForm = ref<Record<string, unknown>>({})
const configSaving = ref(false)
const configLoaded = ref(false)

/**
 * 会话凭证：plugin_data_* 现为 std 族后端强制命令（token ↔ plugin_id 绑定）。
 * 宿主管理界面与插件 SDK 同一模型——打开抽屉时按需换取，插件切换 / 停用后
 * 重新签发（旧 token 随服务端吊销失效）。
 */
const sessionToken = ref<string | null>(null)

async function ensureSession(id: string): Promise<string> {
  if (sessionToken.value !== null && configPluginId.value === id) {
    return sessionToken.value
  }
  const token = await invoke<string>(`${IPC_PREFIX}plugin_session_create`, { id })
  sessionToken.value = token
  configPluginId.value = id
  return token
}

const configPluginId = ref<string | null>(null)

async function loadConfig(): Promise<void> {
  const v = view.value
  if (!v) return
  try {
    const raw = await invoke<unknown | null>(`${IPC_PREFIX}plugin_data_get`, {
      id: v.id,
      key: '__config',
      sessionToken: await ensureSession(v.id),
    })
    configForm.value = (raw as Record<string, unknown>) ?? {}
    configLoaded.value = true
  } catch {
    // 停用 / 会话失效：配置表单保持空态（详情页仍可查看元信息与权限）
    sessionToken.value = null
    configPluginId.value = null
    configForm.value = {}
    configLoaded.value = true
  }
}

async function saveConfig(): Promise<void> {
  const v = view.value
  if (!v) return
  configSaving.value = true
  try {
    await invoke(`${IPC_PREFIX}plugin_data_set`, {
      id: v.id,
      key: '__config',
      value: configForm.value,
      sessionToken: await ensureSession(v.id),
    })
    toast.success(t('plugins.configSavedToast'))
  } catch (e) {
    toast.error(t('plugins.configSaveFailed'), { message: errorText(e) })
  } finally {
    configSaving.value = false
  }
}

// ---------- 日志 ----------

interface LogRow {
  id: number
  plugin_id: string
  level: string
  message: string
  created_at: string
}

const logs = ref<LogRow[]>([])
const logsLoading = ref(false)
const logsLevel = ref<string | null>(null)
const levelOptions = computed(() => [
  { value: '', label: t('plugins.logs.all') },
  { value: 'audit', label: 'audit' },
  { value: 'info', label: 'info' },
  { value: 'warn', label: 'warn' },
  { value: 'error', label: 'error' },
])

async function loadLogs(): Promise<void> {
  const v = view.value
  if (!v) return
  logsLoading.value = true
  try {
    logs.value = await invoke<LogRow[]>(`${IPC_PREFIX}plugin_logs`, {
      id: v.id,
      limit: 200,
      level: logsLevel.value || undefined,
    })
  } catch {
    logs.value = []
  } finally {
    logsLoading.value = false
  }
}

async function clearLogs(): Promise<void> {
  const v = view.value
  if (!v) return
  try {
    await invoke(`${IPC_PREFIX}plugin_clear_logs`, { id: v.id })
    await loadLogs()
  } catch (e) {
    toast.error(t('plugins.logsClearFailed'), { message: errorText(e) })
  }
}

function logTone(level: string): StatusTone {
  if (level === 'error') return 'fail'
  if (level === 'warn') return 'warn'
  if (level === 'audit') return 'run'
  return 'info'
}

// ---------- 打开时加载 ----------

watch(
  () => [props.open, props.pluginId] as const,
  ([open]) => {
    if (open) {
      void loadConfig()
      void loadLogs()
      void loadRuntimeInfo()
    } else {
      // 关闭抽屉即释放本地凭证引用（服务端会话按 TTL / 吊销策略回收）
      sessionToken.value = null
      configPluginId.value = null
    }
  },
  { immediate: true },
)

// ---------- 展示辅助 ----------

function displayName(manifest: PluginManifest): string {
  return manifest.name_key ? t(manifest.name_key) : manifest.name
}
function description(manifest: PluginManifest): string {
  return manifest.description_key ? t(manifest.description_key) : manifest.description
}
function stateTone(v: PluginView): StatusTone {
  if (v.state === 'error') return 'fail'
  return v.enabled ? 'ok' : 'wait'
}
function stateLabel(v: PluginView): string {
  if (v.state === 'error') return t('plugins.stateError')
  return v.enabled ? t('plugins.stateActive') : t('plugins.stateInstalled')
}
function runtimeLabel(): string {
  const rt = entry.value?.runtime ?? 'inactive'
  return t(`plugins.runtime.${rt}`)
}
function navRouteOf(item: PluginNavItem): string {
  return item.route
}
</script>

<template>
  <Drawer
    :open="open"
    :title="view ? displayName(view.manifest) : t('plugins.detailTitle')"
    width="520px"
    @update:open="(v: boolean) => { if (!v) emit('close') }"
  >
    <template v-if="view">
      <div class="pd-head">
        <StatusBadge :tone="stateTone(view)" :label="stateLabel(view)" />
        <span class="pd-ver">v{{ view.version }}</span>
        <span class="pd-runtime">{{ t('plugins.runtimeLabel') }}: {{ runtimeLabel() }}</span>
      </div>
      <p class="pd-desc">{{ description(view.manifest) }}</p>

      <Tabs v-model="tab" :tabs="tabItems" size="sm" class="pd-tabs" />

      <!-- 概览 -->
      <section v-if="tab === 'overview'" class="pd-section">
        <dl class="pd-kv">
          <dt>{{ t('plugins.detail.id') }}</dt>
          <dd class="pd-mono">{{ view.id }}</dd>
          <dt>{{ t('plugins.detail.authors') }}</dt>
          <dd>{{ view.manifest.authors.join(', ') || '—' }}</dd>
          <dt>{{ t('plugins.detail.kind') }}</dt>
          <dd>{{ view.kind === 'ui' ? t('plugins.kindUi') : t('plugins.kindSidecar') }}</dd>
          <dt>{{ t('plugins.detail.source') }}</dt>
          <dd>{{ view.source === 'builtin' ? t('plugins.sourceBuiltin') : t('plugins.sourceLocal') }}</dd>
          <dt>{{ t('plugins.detail.minApp') }}</dt>
          <dd class="pd-mono">≥ {{ view.manifest.min_app_version }}</dd>
          <dt>{{ t('plugins.detail.sdk') }}</dt>
          <dd class="pd-mono">v{{ view.manifest.sdk_version }}</dd>
          <dt>{{ t('plugins.detail.signed') }}</dt>
          <dd>
            <StatusBadge
              :tone="view.signed ? 'ok' : 'warn'"
              :label="view.signed ? t('plugins.signed') : t('plugins.unsigned')"
            />
          </dd>
          <dt>{{ t('plugins.filesVerified') }}</dt>
          <dd>
            <StatusBadge
              :tone="view.files_verified ? 'ok' : view.last_verified_at ? 'fail' : 'wait'"
              :label="
                view.files_verified
                  ? t('plugins.filesVerifiedOk')
                  : view.last_verified_at
                    ? t('plugins.filesVerifiedFail')
                    : t('plugins.filesNeverVerified')
              "
            />
            <span v-if="view.last_verified_at" class="pd-verified-at">
              {{ t('plugins.lastVerifiedAt') }}: {{ view.last_verified_at.slice(0, 19).replace('T', ' ') }}
            </span>
          </dd>
        </dl>
        <button
          v-if="view.install_path"
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="verifying"
          @click="onVerifyIntegrity"
        >
          {{ verifying ? t('common.loading') : t('plugins.verifyIntegrity') }}
        </button>
        <!-- Sidecar 运行状态（ui-sidecar 形态） -->
        <template v-if="view.kind === 'ui-sidecar'">
          <div class="pd-sub">{{ t('plugins.sidecarSection') }}</div>
          <dl v-if="sidecarStatus" class="pd-kv">
            <dt>{{ t('plugins.sidecarPid') }}</dt>
            <dd class="pd-mono">{{ sidecarStatus.pid }}</dd>
            <dt>{{ t('plugins.sidecarVersion') }}</dt>
            <dd class="pd-mono">{{ sidecarStatus.sidecar_version || '—' }}</dd>
            <dt>{{ t('plugins.sidecarRestarts') }}</dt>
            <dd class="pd-mono">{{ sidecarStatus.restarts_last_hour }}</dd>
          </dl>
          <p v-else class="pd-sidecar-off">{{ t('plugins.sidecarNotRunning') }}</p>
        </template>
        <!-- 版本历史 / 回滚 -->
        <template v-if="view.source !== 'builtin'">
          <div class="pd-sub">{{ t('plugins.historySection') }}</div>
          <ul v-if="versionHistory.length" class="pd-nav-list">
            <li v-for="h in versionHistory" :key="h.version + h.created_at" class="pd-nav-item">
              <span class="pd-mono">v{{ h.version }}</span>
              <span class="pd-verified-at">{{ h.created_at.slice(0, 19).replace('T', ' ') }}</span>
              <button class="miyo-btn miyo-btn-sm" type="button" @click="onRollback">
                {{ t('plugins.rollback') }}
              </button>
            </li>
          </ul>
          <p v-else class="pd-sidecar-off">{{ t('plugins.noHistory') }}</p>
        </template>
        <template v-if="entry?.lastActivateMs !== null">
          <div class="pd-sub">{{ t('plugins.metricsSection') }}</div>
          <dl class="pd-kv">
            <dt>{{ t('plugins.metrics.activateMs') }}</dt>
            <dd class="pd-mono">{{ entry?.lastActivateMs }} ms</dd>
            <dt>{{ t('plugins.metrics.count') }}</dt>
            <dd class="pd-mono">{{ entry?.activateCount }}</dd>
          </dl>
        </template>
        <template v-if="view.manifest.nav?.length">
          <div class="pd-sub">{{ t('plugins.navSection') }}</div>
          <ul class="pd-nav-list">
            <li v-for="item in view.manifest.nav" :key="item.id" class="pd-nav-item">
              <Icon :size="14" :name="(item.icon as never) ?? 'package'" />
              <span>{{ item.label_key ? t(item.label_key) : item.label }}</span>
              <span class="pd-mono pd-nav-route">{{ navRouteOf(item) }}</span>
            </li>
          </ul>
        </template>
        <template v-if="view.manifest.runtime_deps?.length">
          <div class="pd-sub">{{ t('plugins.runtimeDepsSection') }}</div>
          <ul class="pd-nav-list">
            <li v-for="dep in view.manifest.runtime_deps" :key="dep.kind" class="pd-nav-item">
              <span>{{ dep.kind }}</span>
              <span v-if="dep.min_version" class="pd-mono">≥ {{ dep.min_version }}</span>
              <span v-if="dep.description" class="pd-dep-desc">{{ dep.description }}</span>
            </li>
          </ul>
        </template>
        <p v-if="view.state === 'error' && view.error" class="pd-err" role="alert">
          {{ t('plugins.detailErrorHint') }}: {{ view.error.message }}
        </p>
      </section>

      <!-- 权限 -->
      <section v-else-if="tab === 'permissions'" class="pd-section">
        <p class="pd-hint">{{ t('plugins.permHint') }}</p>
        <ul class="pd-perm-list">
          <li v-for="p in view.manifest.permissions" :key="p" class="pd-perm-row">
            <label class="pd-perm-check">
              <input v-model="grantedDraft" type="checkbox" :value="p" />
              <StatusBadge :tone="riskTone(lookupPermission(p)?.risk)" :label="permLabel(p)" />
            </label>
            <span class="pd-mono pd-perm-id">{{ p }}</span>
          </li>
        </ul>
        <div class="pd-actions">
          <button
            class="miyo-btn miyo-btn-sm miyo-btn-primary"
            type="button"
            :disabled="permSaving"
            @click="savePermissions"
          >
            {{ t('plugins.permSave') }}
          </button>
        </div>
      </section>

      <!-- 配置 -->
      <section v-else-if="tab === 'config'" class="pd-section">
        <template v-if="view.manifest.config_schema">
          <PluginConfigForm
            v-model="configForm"
            :schema="view.manifest.config_schema"
            :disabled="configSaving"
          />
          <div class="pd-actions">
            <button
              class="miyo-btn miyo-btn-sm miyo-btn-primary"
              type="button"
              :disabled="configSaving"
              @click="saveConfig"
            >
              {{ t('plugins.configSave') }}
            </button>
          </div>
        </template>
        <EmptyState
          v-else
          icon="settings"
          :title="t('plugins.noConfig')"
          compact
        />
      </section>

      <!-- 日志 -->
      <section v-else class="pd-section">
        <div class="pd-logs-toolbar">
          <select v-model="logsLevel" class="pd-level" @change="loadLogs">
            <option v-for="opt in levelOptions" :key="opt.value" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="loadLogs">
            {{ t('plugins.logs.refresh') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="clearLogs">
            {{ t('plugins.logs.clear') }}
          </button>
        </div>
        <div v-if="logsLoading" class="pd-logs-loading"><Spinner /></div>
        <div v-else-if="logs.length === 0" class="pd-logs-empty">{{ t('plugins.logs.empty') }}</div>
        <ul v-else class="pd-log-list">
          <li v-for="row in logs" :key="row.id" class="pd-log-row">
            <StatusBadge :tone="logTone(row.level)" :label="row.level" />
            <span class="pd-log-msg">{{ row.message }}</span>
            <span class="pd-log-time pd-mono">{{ row.created_at.slice(11, 19) }}</span>
          </li>
        </ul>
      </section>
    </template>
  </Drawer>
</template>

<style scoped>
.pd-head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.pd-ver {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.pd-runtime {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.pd-desc {
  margin: 8px 0 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
}
.pd-tabs {
  margin-top: 14px;
}

.pd-section {
  margin-top: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.pd-kv {
  display: grid;
  grid-template-columns: 110px 1fr;
  gap: 6px 12px;
  margin: 0;
}
.pd-kv dt {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.pd-kv dd {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-1);
  overflow-wrap: anywhere;
}
.pd-mono {
  font-family: var(--font-mono);
}

.pd-sub {
  margin-top: 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.pd-nav-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.pd-nav-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.pd-nav-route {
  color: var(--text-3);
  font-size: var(--fs-2xs);
}
.pd-dep-desc {
  color: var(--text-3);
  font-size: var(--fs-2xs);
}

.pd-err {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--danger);
}
.pd-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.pd-perm-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.pd-perm-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.pd-perm-check {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}
.pd-perm-id {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}

.pd-actions {
  display: flex;
  justify-content: flex-end;
}

.pd-logs-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
}
.pd-level {
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-1);
  font-size: var(--fs-xs);
  font-family: inherit;
}
.pd-logs-loading {
  display: flex;
  justify-content: center;
  padding: 24px 0;
}
.pd-logs-empty {
  padding: 20px 0;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.pd-log-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 420px;
  overflow-y: auto;
}
.pd-log-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: var(--fs-xs);
}
.pd-log-msg {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--text-1);
}
.pd-log-time {
  flex: none;
  color: var(--text-3);
  font-size: var(--fs-2xs);
}

.pd-verified-at {
  font-size: 11px;
  color: var(--text-3);
  margin-left: 6px;
}
.pd-sidecar-off {
  font-size: 12px;
  color: var(--text-3);
  padding: 4px 0;
}
</style>
