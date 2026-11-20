<script setup lang="ts">
/**
 * PluginsView：插件管理中心（/plugins，核心路由）。
 *
 * - 「已安装」：卡片列表（状态徽章 / 启停开关 / 更新 / 卸载），点卡片开详情抽屉；
 * - 「市场」：内置目录（编译期嵌入清单）+ 本地 .miyo 包安装（检视确认 → 安装）；
 * - 详情抽屉：元信息 / 权限管理（收权）/ 配置表单（configSchema）/ 运行指标 /
 *   日志（audit 默认置顶）/ 激活失败恢复。
 *
 * UI 与 Miyo 设计体系同源：CSS 变量 + miyo-* 通用类；文案全部走 i18n。
 */
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import PageHeader from '@/components/navigation/PageHeader.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import Modal from '@/components/ui/Modal.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Spinner from '@/components/ui/Spinner.vue'
import { useLocaleStore } from '@/stores/locale'
import { useToast } from '@/composables/useToast'
import { usePluginManager } from '@/plugins/manager'
import { highRiskPermissions, lookupPermission } from '@/plugins/permissions'
import { errorText } from '@/utils/errorText'
import type { PluginCatalogView } from '@/types/generated/PluginCatalogView'
import type { PluginManifest } from '@/types/generated/PluginManifest'
import type { PluginView } from '@/types/generated/PluginView'
import PluginDetailDrawer from '@/domains/plugins/components/PluginDetailDrawer.vue'

const router = useRouter()
const route = useRoute()
const t = useLocaleStore().t
const toast = useToast()
const manager = usePluginManager()

const IPC_PREFIX = 'plugin:miyo|'

// ---------- 页签 ----------
type CenterTab = 'installed' | 'market'
const tab = ref<CenterTab>('installed')
const tabItems = computed(() => [
  { value: 'installed', label: t('plugins.tabInstalled') },
  { value: 'market', label: t('plugins.tabMarket') },
])

/** 守卫重定向（停用插件路由访问被拦到这里）：提示 + 清参数。 */
onMounted(() => {
  void manager.refreshInstalled()
  void loadCatalog()
  void loadMarketIndex()
  if (typeof route.query.blocked === 'string') {
    toast.toast({ type: 'warning', title: t('plugins.routeBlocked', { id: route.query.blocked }) })
    void router.replace({ path: '/plugins' })
  }
})

// ---------- 市场目录 ----------
const catalog = ref<PluginCatalogView[]>([])
const catalogLoading = ref(false)

async function loadCatalog(): Promise<void> {
  catalogLoading.value = true
  try {
    catalog.value = await manager.loadCatalog()
  } finally {
    catalogLoading.value = false
  }
}

// ---------- 官方市场索引（静态；与内置目录合并展示） ----------

interface MarketEntryView {
  id: string
  name: string
  version: string
  authors: string[]
  description: string
  category: string
  permissions: string[]
  min_app_version: string
  sdk_version: number
  changelog: string
  download_url: string
}

const marketEntries = ref<MarketEntryView[]>([])
const marketQuery = ref('')
const marketCategory = ref('all')
const categories = computed(() => {
  const set = new Set<string>()
  for (const e of marketEntries.value) set.add(e.category)
  return ['all', ...[...set].sort()]
})

async function loadMarketIndex(): Promise<void> {
  try {
    marketEntries.value = await invoke<MarketEntryView[]>(`${IPC_PREFIX}plugin_market_index`)
  } catch {
    marketEntries.value = [] // 索引缺失不阻塞市场页（内置目录仍可用）
  }
}

/** 市场渲染模型：索引条目优先，命中内置目录时用目录卡片（可安装/更新）。 */
const marketItems = computed(() => {
  const q = marketQuery.value.trim().toLowerCase()
  return marketEntries.value
    .filter((e) => marketCategory.value === 'all' || e.category === marketCategory.value)
    .filter((e) => {
      if (!q) return true
      return (
        e.name.toLowerCase().includes(q) ||
        e.id.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q)
      )
    })
    .map((entry) => ({
      entry,
      catalogEntry: catalog.value.find((c) => c.manifest.id === entry.id) ?? null,
      installed: catalog.value.some((c) => c.manifest.id === entry.id),
      compatible:
        catalog.value.find((c) => c.manifest.id === entry.id)?.compatible ?? true,
    }))
})

async function copyDownloadUrl(url: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(url)
    toast.success(t('plugins.copiedToast'))
  } catch {
    toast.error(t('plugins.copyDownloadUrl'))
  }
}

// ---------- 安装 / 更新 / 卸载 ----------

/** 版本可比（market 条目已装且目录版本更新）。 */
function updateAvailable(entry: PluginCatalogView): boolean {
  if (!entry.installed || !entry.installed_version) return false
  return entry.installed_version !== entry.manifest.version
}

const installing = ref(false)
async function onInstall(entry: PluginCatalogView): Promise<void> {
  installing.value = true
  try {
    await manager.installBuiltin(entry.manifest.id)
    // 安装成功即启用（内置插件可信；权限已在卡片下方列明）。
    await manager.enable(entry.manifest.id)
    toast.success(t('plugins.installedToast', { name: displayNameOf(entry.manifest) }))
    await loadCatalog()
  } catch (e) {
    toast.error(t('plugins.installFailed'), { message: errorText(e) })
  } finally {
    installing.value = false
  }
}

async function onUpdate(entry: PluginCatalogView): Promise<void> {
  installing.value = true
  try {
    await manager.updateBuiltin(entry.manifest.id)
    toast.success(t('plugins.updatedToast', { name: displayNameOf(entry.manifest) }))
    await loadCatalog()
  } catch (e) {
    toast.error(t('plugins.updateFailed'), { message: errorText(e) })
  } finally {
    installing.value = false
  }
}

/** 待卸载插件（确认弹窗）。 */
const uninstallTarget = ref<PluginView | null>(null)
function askUninstall(view: PluginView): void {
  uninstallTarget.value = view
}
async function confirmUninstall(): Promise<void> {
  const target = uninstallTarget.value
  if (!target) return
  try {
    await manager.uninstall(target.id)
    toast.success(t('plugins.uninstalledToast', { name: displayNameOf(target.manifest) }))
    await loadCatalog()
  } catch (e) {
    toast.error(t('plugins.uninstallFailed'), { message: errorText(e) })
  } finally {
    uninstallTarget.value = null
  }
}

// ---------- 启停 ----------

const toggling = ref<string | null>(null)
async function onToggle(view: PluginView, enabled: boolean): Promise<void> {
  toggling.value = view.id
  try {
    if (enabled) await manager.enable(view.id)
    else await manager.disable(view.id)
    await manager.refreshInstalled()
    await loadCatalog()
  } catch (e) {
    toast.error(t(enabled ? 'plugins.enableFailed' : 'plugins.disableFailed'), {
      message: errorText(e),
    })
    await manager.refreshInstalled()
  } finally {
    toggling.value = null
  }
}

/** 激活失败恢复（error 态卡片按钮）。 */
async function onRecover(view: PluginView): Promise<void> {
  try {
    await manager.retryActivate(view.id)
    await manager.refreshInstalled()
    toast.success(t('plugins.recoveredToast'))
  } catch (e) {
    toast.error(t('plugins.recoverFailed'), { message: errorText(e) })
  }
}

// ---------- 本地包安装（检视确认流） ----------

const inspection = ref<{
  manifest: PluginManifest
  integrity: string
  signed: boolean
  origin: string
  path: string
} | null>(null)
const trustUnsigned = ref(false)
const inspecting = ref(false)
/** 检视用途：install（本地新装）/ update（本地包更新）/ remote（市场远程）。 */
const inspectMode = ref<'install' | 'update' | 'remote'>('install')
const remoteInspecting = ref<string | null>(null)
/** High 风险权限单独确认（用户勾选后安装 / 更新按钮才可用；每次检视重置）。 */
const highRiskConfirmed = ref(false)
/** 当前检视包声明的 High 风险权限（空 = 无需单独确认）。 */
const inspectionHighRisk = computed(() =>
  inspection.value ? highRiskPermissions(inspection.value.manifest.permissions) : [],
)

/** 市场远程安装：下载 → 检视缓存（后端）→ 复用确认弹窗 → 确认后从缓存安装。 */
async function onRemoteInstall(entry: {
  id: string
  name: string
  version: string
  download_url: string
  permissions: string[]
}): Promise<void> {
  remoteInspecting.value = entry.id
  try {
    const info = await invoke<{
      manifest: PluginManifest
      integrity: string
      signed: boolean
      origin: string
    }>(`${IPC_PREFIX}plugin_inspect_from_url`, { url: entry.download_url })
    inspection.value = { ...info, path: entry.download_url }
    trustUnsigned.value = false
    highRiskConfirmed.value = false
  } catch (e) {
    toast.error(t('plugins.downloadFailed'), { message: errorText(e) })
  } finally {
    remoteInspecting.value = null
  }
}

async function pickAndInspect(mode: 'install' | 'update' = 'install'): Promise<void> {
  inspectMode.value = mode
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: 'Miyo Plugin', extensions: ['miyo', 'json'] }],
  })
  if (typeof picked !== 'string') return
  inspecting.value = true
  try {
    const info = await invoke<{
      manifest: PluginManifest
      integrity: string
      signed: boolean
      origin: string
    }>(`${IPC_PREFIX}plugin_inspect_package`, { path: picked })
    inspection.value = { ...info, path: picked }
    trustUnsigned.value = false
    highRiskConfirmed.value = false
  } catch (e) {
    toast.error(t('plugins.inspectFailed'), { message: errorText(e) })
  } finally {
    inspecting.value = false
  }
}

const installFileBusy = ref(false)
async function confirmInstallFile(): Promise<void> {
  const pkg = inspection.value
  if (!pkg) return
  if (!pkg.signed && !trustUnsigned.value) return
  // High 风险权限单独确认（PERMISSIONS.md 口径：确认页明示 + High 单独勾选）
  if (inspectionHighRisk.value.length > 0 && !highRiskConfirmed.value) return
  installFileBusy.value = true
  try {
    if (inspectMode.value === 'remote') {
      await invoke(`${IPC_PREFIX}plugin_install_from_cached`, {
        url: pkg.path,
        trustUnsigned: pkg.signed ? undefined : true,
      })
      toast.success(t('plugins.installedToast', { name: displayNameOf(pkg.manifest) }))
    } else if (inspectMode.value === 'update') {
      await invoke(`${IPC_PREFIX}plugin_update_from_path`, {
        path: pkg.path,
        trustUnsigned: pkg.signed ? undefined : true,
      })
      toast.success(t('plugins.updatedToast', { name: displayNameOf(pkg.manifest) }))
    } else {
      await invoke(`${IPC_PREFIX}plugin_install_from_path`, {
        path: pkg.path,
        trustUnsigned: pkg.signed ? undefined : true,
      })
      toast.success(t('plugins.installedToast', { name: displayNameOf(pkg.manifest) }))
    }
    inspection.value = null
    await manager.refreshInstalled()
    tab.value = 'installed'
  } catch (e) {
    toast.error(
      t(
        inspectMode.value === 'update'
          ? 'plugins.updateFailed'
          : inspectMode.value === 'remote'
            ? 'plugins.installFailed'
            : 'plugins.installFailed',
      ),
      { message: errorText(e) },
    )
  } finally {
    installFileBusy.value = false
  }
}

// ---------- 本地包回滚（确认流） ----------

const rollbackTarget = ref<PluginView | null>(null)
async function confirmRollback(): Promise<void> {
  const target = rollbackTarget.value
  if (!target) return
  try {
    const done = await invoke<{ version: string }>(`${IPC_PREFIX}plugin_rollback`, { id: target.id })
    toast.success(t('plugins.rollbackDoneToast', { version: done.version }))
    await manager.refreshInstalled()
    await loadCatalog()
  } catch (e) {
    toast.error(t('plugins.rollbackFailed'), { message: errorText(e) })
  } finally {
    rollbackTarget.value = null
  }
}

// ---------- 展示辅助 ----------

function displayNameOf(manifest: PluginManifest): string {
  return manifest.name_key ? t(manifest.name_key) : manifest.name
}
function descriptionOf(manifest: PluginManifest): string {
  return manifest.description_key ? t(manifest.description_key) : manifest.description
}
function stateTone(view: PluginView): StatusTone {
  if (view.state === 'error') return 'fail'
  return view.enabled ? 'ok' : 'wait'
}
function stateLabel(view: PluginView): string {
  if (view.state === 'error') return t('plugins.stateError')
  return view.enabled ? t('plugins.stateActive') : t('plugins.stateInstalled')
}
function kindLabel(view: PluginView): string {
  return view.kind === 'ui' ? t('plugins.kindUi') : t('plugins.kindSidecar')
}
function sourceLabel(view: PluginView): string {
  return view.source === 'builtin' ? t('plugins.sourceBuiltin') : t('plugins.sourceLocal')
}
function iconOf(manifest: PluginManifest): string | null {
  if (!manifest.icon) return null
  return manifest.icon
}
// ---------- 详情抽屉 ----------
const detailOpen = ref(false)
const detailId = ref<string | null>(null)
function openDetail(view: PluginView): void {
  detailId.value = view.id
  detailOpen.value = true
}
function closeDetail(): void {
  detailOpen.value = false
  detailId.value = null
}
</script>

<template>
  <div class="pg-root">
    <PageHeader :title="t('plugins.title')">
      <SegmentedControl v-model="tab" :options="tabItems" size="sm" />
      <template #actions>
        <IconButton
          name="upload"
          :size="15"
          :title="t('plugins.installFromFile')"
          @click="pickAndInspect('install')"
        />
      </template>
    </PageHeader>

    <main class="pg-body">
      <!-- 已安装 -->
      <section v-if="tab === 'installed'" class="pg-list" aria-label="installed">
        <EmptyState
          v-if="manager.installedList.length === 0"
          icon="package"
          :title="t('plugins.emptyInstalled')"
          :description="t('plugins.emptyInstalledHint')"
        >
          <button class="miyo-btn miyo-btn-sm" type="button" @click="tab = 'market'">
            {{ t('plugins.browseMarket') }}
          </button>
        </EmptyState>
        <article
          v-for="e in manager.installedList"
          :key="e.view.id"
          class="pg-card"
          :class="{ 'pg-card-error': e.view.state === 'error' }"
        >
          <button class="pg-card-main" type="button" @click="openDetail(e.view)">
            <img v-if="iconOf(e.view.manifest)?.startsWith('data:image/')" class="pg-icon" :src="iconOf(e.view.manifest)!" alt="" />
            <Icon v-else-if="iconOf(e.view.manifest)" :name="(iconOf(e.view.manifest) as never)" :size="18" />
            <div class="pg-card-text">
              <div class="pg-card-title">
                <span class="pg-name">{{ displayNameOf(e.view.manifest) }}</span>
                <span class="pg-ver">v{{ e.view.version }}</span>
                <span class="pg-src">{{ sourceLabel(e.view) }}</span>
                <StatusBadge :tone="stateTone(e.view)" :label="stateLabel(e.view)" />
              </div>
              <p class="pg-desc">{{ descriptionOf(e.view.manifest) }}</p>
              <div class="pg-meta">
                <span>{{ kindLabel(e.view) }}</span>
                <span>·</span>
                <span>{{ e.view.manifest.authors.join(', ') || '—' }}</span>
                <span>·</span>
                <span>{{ e.view.granted_permissions.length }} {{ t('plugins.permCount') }}</span>
                <template v-if="e.lastActivateMs !== null">
                  <span>·</span>
                  <span>{{ t('plugins.activateMs', { ms: e.lastActivateMs }) }}</span>
                </template>
              </div>
              <p v-if="e.view.state === 'error' && e.view.error" class="pg-err" role="alert">
                {{ e.view.error.message }}
              </p>
            </div>
          </button>
          <div class="pg-card-actions" @click.stop>
            <button
              v-if="e.view.state === 'error'"
              class="miyo-btn miyo-btn-sm"
              type="button"
              @click="onRecover(e.view)"
            >
              {{ t('plugins.recover') }}
            </button>
            <button
              v-if="e.view.source !== 'builtin'"
              class="miyo-btn miyo-btn-sm"
              type="button"
              :title="t('plugins.updateFromPath')"
              @click="pickAndInspect('update')"
            >
              {{ t('plugins.updateFromPath') }}
            </button>
            <label class="pg-switch" :title="t(e.view.enabled ? 'plugins.disableAction' : 'plugins.enableAction')">
              <input
                type="checkbox"
                role="switch"
                :checked="e.view.enabled"
                :disabled="toggling === e.view.id"
                @change="onToggle(e.view, ($event.target as HTMLInputElement).checked)"
              />
              <span class="pg-switch-track" aria-hidden="true"></span>
            </label>
            <IconButton
              v-if="e.view.source !== 'builtin'"
              name="history"
              :size="15"
              :title="t('plugins.rollback')"
              @click="rollbackTarget = e.view"
            />
            <IconButton name="trash" :size="15" tone="danger" :title="t('plugins.uninstallAction')" @click="askUninstall(e.view)" />
          </div>
        </article>
      </section>

      <!-- 市场 -->
      <section v-else class="pg-list" aria-label="market">
        <div class="pg-market-tools">
          <SearchInput
            v-model="marketQuery"
            size="md"
            class="pg-market-search"
            :placeholder="t('plugins.marketSearch')"
          />
          <div class="pg-market-cats">
            <button
              v-for="c in categories"
              :key="c"
              class="pg-cat-chip"
              :class="{ active: marketCategory === c }"
              type="button"
              @click="marketCategory = c"
            >
              {{ c === 'all' ? t('plugins.marketAllCategories') : t(`plugins.category${c.charAt(0).toUpperCase() + c.slice(1)}`) }}
            </button>
          </div>
        </div>
        <div v-if="catalogLoading" class="pg-loading"><Spinner /></div>
        <EmptyState
          v-else-if="marketItems.length === 0"
          icon="package"
          :title="t('plugins.emptyMarket')"
          :description="t('plugins.emptyMarketHint')"
        />
        <article v-for="item in marketItems" v-else :key="item.entry.id" class="pg-card">
          <div class="pg-card-main pg-card-static">
            <div class="pg-card-text">
              <div class="pg-card-title">
                <span class="pg-name">{{ item.entry.name }}</span>
                <span class="pg-ver">v{{ item.entry.version }}</span>
                <span class="pg-src">{{ t(`plugins.category${item.entry.category.charAt(0).toUpperCase() + item.entry.category.slice(1)}`) }}</span>
                <StatusBadge
                  v-if="item.installed"
                  tone="ok"
                  :label="t('plugins.upToDate')"
                />
                <StatusBadge
                  v-else-if="!item.compatible"
                  tone="fail"
                  :label="t('plugins.incompatible', { v: item.entry.min_app_version })"
                />
              </div>
              <p class="pg-desc">{{ item.entry.description }}</p>
              <!-- 权限声明：安装前明示（最小知情） -->
              <div class="pg-perms-row">
                <span
                  v-for="p in item.entry.permissions"
                  :key="p"
                  class="pg-perm-chip"
                  :class="`risk-${lookupPermission(p)?.risk ?? 'low'}`"
                  :title="t(lookupPermission(p)?.labelKey ?? 'plugins.permUnknown')"
                >
                  {{ t(lookupPermission(p)?.labelKey ?? 'plugins.permUnknown') }}
                </span>
              </div>
            </div>
          </div>
          <div class="pg-card-actions">
            <template v-if="item.catalogEntry">
              <template v-if="!item.catalogEntry.installed">
                <button
                  class="miyo-btn miyo-btn-sm miyo-btn-primary"
                  type="button"
                  :disabled="installing || !item.compatible"
                  @click="onInstall(item.catalogEntry)"
                >
                  {{ t('plugins.installAction') }}
                </button>
              </template>
              <template v-else>
                <button
                  v-if="updateAvailable(item.catalogEntry)"
                  class="miyo-btn miyo-btn-sm miyo-btn-primary"
                  type="button"
                  :disabled="installing"
                  @click="onUpdate(item.catalogEntry)"
                >
                  {{ t('plugins.updateAction', { v: item.catalogEntry.manifest.version }) }}
                </button>
                <StatusBadge v-else tone="ok" :label="t('plugins.upToDate')" />
              </template>
            </template>
            <template v-else>
              <button
                class="miyo-btn miyo-btn-sm miyo-btn-primary"
                type="button"
                :disabled="remoteInspecting !== null"
                @click="onRemoteInstall(item.entry)"
              >
                {{
                  remoteInspecting === item.entry.id
                    ? t('plugins.remoteDownloading')
                    : t('plugins.remoteInstall')
                }}
              </button>
              <IconButton
                name="copy"
                :size="15"
                :title="t('plugins.copyDownloadUrl')"
                @click="copyDownloadUrl(item.entry.download_url)"
              />
            </template>
          </div>
        </article>
        <p class="pg-market-guide">{{ t('plugins.marketSourceGuide') }}</p>
      </section>
    </main>

    <!-- 详情抽屉 -->
    <PluginDetailDrawer
      :open="detailOpen"
      :plugin-id="detailId"
      @close="closeDetail"
      @changed="() => { void manager.refreshInstalled(); void loadCatalog() }"
    />

    <!-- 卸载确认 -->
    <Modal :open="!!uninstallTarget" :title="t('plugins.uninstallTitle')" width="440px" @update:open="(v: boolean) => { if (!v) uninstallTarget = null }">
      <p class="pg-confirm-text">
        {{ t('plugins.uninstallConfirm', { name: uninstallTarget ? displayNameOf(uninstallTarget.manifest) : '' }) }}
      </p>
      <p class="pg-confirm-hint">{{ t('plugins.uninstallHint') }}</p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="uninstallTarget = null">
          {{ t('common.cancel') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="confirmUninstall">
          {{ t('plugins.uninstallAction') }}
        </button>
      </template>
    </Modal>

    <!-- 回滚确认 -->
    <Modal
      :open="!!rollbackTarget"
      :title="t('plugins.rollbackConfirmTitle')"
      width="440px"
      @update:open="(v: boolean) => { if (!v) rollbackTarget = null }"
    >
      <p class="pg-confirm-text">
        {{
          t('plugins.rollbackConfirmBody', {
            version: rollbackTarget ? `v${rollbackTarget.version}` : '',
            at: rollbackTarget ? rollbackTarget.updated_at : '',
          })
        }}
      </p>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="rollbackTarget = null">
          {{ t('common.cancel') }}
        </button>
        <button class="miyo-btn miyo-btn-sm miyo-btn-danger" type="button" @click="confirmRollback">
          {{ t('plugins.rollback') }}
        </button>
      </template>
    </Modal>

    <!-- 本地包安装确认 -->
    <Modal
      :open="!!inspection"
      :title="inspectMode === 'update' ? t('plugins.updateFromPath') : t('plugins.inspectTitle')"
      width="520px"
      @update:open="(v: boolean) => { if (!v) inspection = null }"
    >
      <template v-if="inspection">
        <div class="pg-inspect-head">
          <strong>{{ displayNameOf(inspection.manifest) }}</strong>
          <span class="pg-ver">v{{ inspection.manifest.version }}</span>
          <span class="pg-src">{{
            inspection.origin === 'package'
              ? t('plugins.originPackage')
              : inspection.origin === 'remote'
                ? t('plugins.originRemote')
                : t('plugins.originDirectory')
          }}</span>
          <StatusBadge :tone="inspection.signed ? 'ok' : 'warn'" :label="inspection.signed ? t('plugins.signed') : t('plugins.unsigned')" />
        </div>
        <p class="pg-desc">{{ descriptionOf(inspection.manifest) }}</p>
        <div class="pg-inspect-section">{{ t('plugins.permSection') }}</div>
        <ul class="pg-perm-list">
          <li v-for="p in inspection.manifest.permissions" :key="p" class="pg-perm-item">
            <span class="pg-perm-chip" :class="`risk-${lookupPermission(p)?.risk ?? 'low'}`">
              {{ t(lookupPermission(p)?.labelKey ?? 'plugins.permUnknown') }}
            </span>
            <span class="pg-perm-id">{{ p }}</span>
          </li>
        </ul>
        <!-- High 风险权限单独确认（文档承诺项）：存在 High 声明时必须显式勾选，
             安装 / 更新按钮才可用——与未签名信任勾选相互独立。 -->
        <label v-if="inspectionHighRisk.length > 0" class="pg-trust pg-trust-high">
          <input v-model="highRiskConfirmed" type="checkbox" />
          <span>{{ t('plugins.confirmHighRisk', { count: inspectionHighRisk.length }) }}</span>
        </label>
        <label v-if="!inspection.signed" class="pg-trust">
          <input v-model="trustUnsigned" type="checkbox" />
          <span>{{ t('plugins.trustUnsigned') }}</span>
        </label>
        <p class="pg-integrity" :title="inspection.integrity">
          {{ t('plugins.integrityLabel') }}: {{ inspection.integrity.slice(0, 16) }}…
        </p>
      </template>
      <template #footer>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="inspection = null">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="
            installFileBusy ||
            (!inspection?.signed && !trustUnsigned) ||
            (inspectionHighRisk.length > 0 && !highRiskConfirmed)
          "
          @click="confirmInstallFile"
        >
          {{ t(inspectMode === 'update' ? 'plugins.updateFromPath' : 'plugins.installConfirm') }}
        </button>
      </template>
    </Modal>
  </div>
</template>

<style scoped>
.pg-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
  background: var(--bg-app);
  color: var(--text-1);
}

.pg-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 20px 24px 40px;
}

.pg-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-width: 860px;
  margin: 0 auto;
}

.pg-market-tools {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 8px;
}
/* 搜索框：统一 SearchInput，仅保留通栏占位 */
.pg-market-search {
  width: 100%;
}
.pg-market-cats {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}
.pg-cat-chip {
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-2);
  border-radius: var(--radius-md);
  padding: 3px 10px;
  font-size: 12px;
  cursor: pointer;
}
.pg-cat-chip.active {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent, #fff);
}
.pg-market-guide {
  font-size: 12px;
  color: var(--text-3);
  text-align: center;
  padding: 8px 0 16px;
}
.pg-loading {
  display: flex;
  justify-content: center;
  padding: 40px 0;
}

/* 卡片：与项目卡同族视觉（elevated 面板 + 悬停提升） */
.pg-card {
  display: flex;
  align-items: stretch;
  gap: 4px;
  padding: 14px 16px;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
}
.pg-card:hover {
  border-color: var(--border-strong);
  box-shadow: var(--shadow);
}
.pg-card-error {
  border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
}

.pg-card-main {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: 12px;
  padding: 0;
  border: none;
  background: none;
  font-family: inherit;
  color: inherit;
  text-align: left;
  cursor: pointer;
}
.pg-card-static {
  cursor: default;
}

.pg-icon {
  width: 34px;
  height: 34px;
  flex: none;
  border-radius: var(--radius-md);
  object-fit: contain;
  background: var(--bg-hover);
}

.pg-card-text {
  flex: 1;
  min-width: 0;
}

.pg-card-title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.pg-name {
  font-size: var(--fs-md);
  font-weight: 600;
}
.pg-ver {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  font-family: var(--font-mono);
}
.pg-src {
  font-size: var(--fs-2xxs);
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border);
  color: var(--text-3);
  background: var(--bg-hover);
}

.pg-desc {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.pg-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
}

.pg-err {
  margin: 6px 0 0;
  font-size: var(--fs-xs);
  color: var(--danger);
}

.pg-card-actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
}

/* 启停开关（极简轨道式） */
.pg-switch {
  position: relative;
  display: inline-flex;
  cursor: pointer;
}
.pg-switch input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: pointer;
}
.pg-switch-track {
  width: 34px;
  height: 20px;
  border-radius: var(--radius-pill);
  background: var(--bg-active);
  border: 1px solid var(--border);
  position: relative;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
}
.pg-switch-track::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--text-3);
  transition: transform var(--dur) var(--ease), background var(--dur) var(--ease);
}
.pg-switch input:checked + .pg-switch-track {
  background: var(--accent);
  border-color: var(--accent);
}
.pg-switch input:checked + .pg-switch-track::after {
  transform: translateX(14px);
  background: #fff;
}
.pg-switch input:focus-visible + .pg-switch-track {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

/* 权限 chip（风险着色） */
.pg-perms-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.pg-perm-chip {
  font-size: var(--fs-2xxs);
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border);
  background: var(--bg-hover);
  color: var(--text-2);
}
.pg-perm-chip.risk-medium {
  border-color: color-mix(in srgb, var(--warning) 55%, transparent);
  color: var(--warning);
  background: var(--warning-tint);
}
.pg-perm-chip.risk-high {
  border-color: color-mix(in srgb, var(--danger) 55%, transparent);
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
}

/* 卸载 / 安装确认弹层 */
.pg-confirm-text {
  margin: 0 0 8px;
  font-size: var(--fs-sm);
  color: var(--text-1);
}
.pg-confirm-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.pg-inspect-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 6px;
}
.pg-inspect-section {
  margin: 14px 0 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.pg-perm-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.pg-perm-item {
  display: flex;
  align-items: center;
  gap: 10px;
}
.pg-perm-id {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  font-family: var(--font-mono);
}
.pg-trust {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
}
/* High 风险确认行：危险色强调（与权限 chip 的 risk-high 同色系）。 */
.pg-trust-high {
  color: var(--danger);
}
.pg-integrity {
  margin: 10px 0 0;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  font-family: var(--font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
