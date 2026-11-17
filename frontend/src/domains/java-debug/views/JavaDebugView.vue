<script setup lang="ts">
/**
 * JavaDebugView：Java 方法实时调试工作台（顶级路由 /java-debug）。
 *
 * 布局：顶栏（项目选择 / Runtime 连接状态 / 连接 / Runtime 信息 / 历史 /
 * 保存用例）+ SplitPanes 三列（类浏览器 / 方法与参数 / 执行结果）+
 * 历史抽屉 + 项目弹窗 + SDK 教程 + 接入向导。无项目时整页展示接入向导
 * （JavaOnboarding）；连接期间 30s 心跳探活（store 内实现）。
 */
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SplitPanes from '@/components/ui/SplitPanes.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import JavaClassBrowser from '@/domains/java-debug/components/JavaClassBrowser.vue'
import JavaHistoryDrawer from '@/domains/java-debug/components/JavaHistoryDrawer.vue'
import JavaMethodPanel from '@/domains/java-debug/components/JavaMethodPanel.vue'
import JavaOnboarding from '@/domains/java-debug/components/JavaOnboarding.vue'
import JavaProjectDialog from '@/domains/java-debug/components/JavaProjectDialog.vue'
import JavaResultPanel from '@/domains/java-debug/components/JavaResultPanel.vue'
import JavaSaveTestCaseDialog from '@/domains/java-debug/components/JavaSaveTestCaseDialog.vue'
import JavaSetupGuide from '@/domains/java-debug/components/JavaSetupGuide.vue'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'
import { errorText } from '@/utils/errorText'

const router = useRouter()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const store = useJavaDebugStore()
const workspace = useWorkspaceStore()

const projectDialogOpen = ref(false)
const editingProject = ref<import('@/types/miyoApi.d').JavaDebugProject | null>(null)
const projectPreset = ref<{ name?: string; endpoint?: string } | null>(null)
const historyOpen = ref(false)
const connecting = ref(false)
const guideOpen = ref(false)
const saveCaseOpen = ref(false)

onMounted(async () => {
  await store.loadProjects()
  // 有项目且未连接：静默自动连接（失败不弹错，交给顶栏状态）。
  if (store.projects.length > 0 && store.connection.state !== 'ok') {
    const ok = await store.connect()
    if (ok) await store.loadClasses()
  }
})

/** 无任何项目 = 首次进入，整页展示接入向导。 */
const needsOnboarding = computed(() => store.projects.length === 0)

/** 返回来源页（与 RealtimeView 同款回退逻辑）。 */
function goBack(): void {
  if (window.history.state?.back) router.back()
  else void router.push('/projects')
}

const projectOptions = computed(() =>
  store.projects.map((p) => ({ value: p.id, label: p.name })),
)

const connectionTone = computed<StatusTone>(() => {
  switch (store.connection.state) {
    case 'ok':
      return 'ok'
    case 'fail':
      return 'fail'
    default:
      return 'wait'
  }
})

const connectionLabel = computed(() => {
  switch (store.connection.state) {
    case 'ok':
      return t('java.connOk')
    case 'fail':
      return t('java.connFail')
    default:
      return t('java.connIdle')
  }
})

const connected = computed(() => store.connection.state === 'ok')

/** 连接后的 Runtime 信息 chips（握手 info 快照）。 */
const runtimeInfo = computed(() => store.connection.info)

/** 关联环境的展示（来自 workspace store 的环境列表）。 */
const linkedEnvName = computed(() => {
  const envId = store.activeProject?.environment_id
  if (!envId) return null
  return workspace.environments.find((e) => e.id === envId)?.name ?? null
})

async function connect(): Promise<void> {
  if (!store.activeProject) return
  connecting.value = true
  const ok = await store.connect().finally(() => {
    connecting.value = false
  })
  if (ok) {
    toast.success(t('java.connectedToast', { app: store.activeProject.name }))
    await store.loadClasses()
  } else {
    toast.error(store.connection.message || t('java.connFail'))
  }
}

function disconnect(): void {
  store.disconnect()
}

function openNewProject(): void {
  editingProject.value = null
  projectPreset.value = null
  projectDialogOpen.value = true
}

function openEditProject(): void {
  if (!store.activeProject) return
  editingProject.value = store.activeProject
  projectPreset.value = null
  projectDialogOpen.value = true
}

/** 接入向导：本地候选 → 预填注册弹窗（应用名 / 端点）。 */
function useCandidate(runtime: import('@/types/miyoApi.d').JavaDiscoveredRuntime): void {
  editingProject.value = null
  projectPreset.value = {
    endpoint: runtime.endpoint,
    name: runtime.app_name || undefined,
  }
  projectDialogOpen.value = true
}

async function onProjectSaved(project: import('@/types/miyoApi.d').JavaDebugProject): Promise<void> {
  await store.loadProjects()
  store.activeProjectId = project.id
  // 保存即尝试连接（新注册项目一步到位进入工作台）。
  const ok = await store.connect()
  if (ok) await store.loadClasses()
}

function editSettings(): void {
  openEditProject()
}

async function run(): Promise<void> {
  const response = await store.invoke()
  if (response && response.status !== 'success') {
    // 业务失败不打断（结果面板突出异常），仅提示已返回。
    toast.warning(t('java.invokeFailedToast'))
  }
}

async function cancelRun(): Promise<void> {
  try {
    await store.cancelLast()
    toast.success(t('java.cancelSent'))
  } catch (e) {
    toast.error(errorText(e))
  }
}

async function refreshHistory(): Promise<void> {
  await store.loadHistory()
}

/** 从历史重跑：回填参数并关抽屉。 */
function rerun(record: import('@/types/miyoApi.d').JavaInvocationRecord): void {
  store.applyFromRecord(record)
  historyOpen.value = false
}

/** 结果面板历史 Tab 的快速重跑（不关抽屉，只回填参数）。 */
function rerunFromPanel(record: import('@/types/miyoApi.d').JavaInvocationRecord): void {
  store.applyFromRecord(record)
  void store.selectClass(record.class_name).then(() => {
    const found = store.methods.find((m) => m.method_id === record.method_id)
    if (found) store.selectMethod(found)
    store.applyFromRecord(record)
  })
}

function pickRecent(methodId: string): void {
  const method = store.methods.find((m) => m.method_id === methodId)
  if (method) {
    store.selectMethod(method)
    return
  }
  // 方法不在当前类列表：先选其所属类再选方法。
  const className = methodId.split('#')[0] ?? ''
  void store.selectClass(className).then(() => {
    const found = store.methods.find((m) => m.method_id === methodId)
    if (found) store.selectMethod(found)
  })
}

/** 保存当前调用配置（参数快照）为请求示例风格的本地草稿 —— MVP 落点：
 * 参数回填历史即可复用，这里提供「复制为 JSON」快捷动作。 */
async function copyArgs(): Promise<void> {
  try {
    await navigator.clipboard.writeText(store.argsJson)
    toast.success(t('java.copied'))
  } catch {
    toast.error(t('java.copyFailed'))
  }
}

/** 当前参数存为模板（下次选该方法自动带出）。 */
function onSaveTemplate(): void {
  store.saveArgsTemplate()
  toast.success(t('java.argsTemplateSaved'))
}

/** 应用已存模板（重新回填参数）。 */
function onUseTemplate(): void {
  if (store.argsTemplate != null) store.argsJson = store.argsTemplate
}
</script>

<template>
  <div class="jdv">
    <header class="jdv-top">
      <div class="jdv-top-left">
        <!-- 返回主页：带文案的清晰入口（原 icon-only + 反向 chevron 不可辨识，
             且 common.back 为字典缺失的悬空键，tooltip 显示裸键名） -->
        <button class="miyo-btn miyo-btn-sm" type="button" :title="t('app.backHome')" @click="goBack">
          <Icon name="chevron-left" :size="13" />
          {{ t('app.backHome') }}
        </button>
        <Icon name="package" :size="16" />
        <h1 class="jdv-title">{{ t('java.viewTitle') }}</h1>
      </div>

      <div class="jdv-top-center">
        <CustomSelect
          :model-value="store.activeProjectId ?? ''"
          :options="projectOptions"
          size="sm"
          :placeholder="t('java.projectSelect')"
          data-testid="java-project-select"
          @update:model-value="(v: string | number | null) => (store.activeProjectId = v ? String(v) : null)"
        />
        <IconButton name="plus" :title="t('java.projectNew')" @click="openNewProject" />

        <StatusBadge :tone="connectionTone" :label="connectionLabel" :dot="store.connection.state === 'ok'" />

        <!-- Runtime 信息 chips（握手 info 快照；连接后展示） -->
        <template v-if="connected && runtimeInfo">
          <span class="jdv-chip" :title="runtimeInfo.jvm_version">
            {{ t('java.chipApp') }}: {{ runtimeInfo.app_name || store.activeProject?.name }}
          </span>
          <span class="jdv-chip" :title="runtimeInfo.jvm_version">
            {{ t('java.chipJava') }} {{ runtimeInfo.java_version }}
          </span>
          <span class="jdv-chip">
            {{ t('java.chipWhitelist', { classes: runtimeInfo.class_count, methods: runtimeInfo.method_count }) }}
          </span>
          <span class="jdv-chip" :title="`SDK ${runtimeInfo.runtime_version} · protocol v${runtimeInfo.protocol_version}`">
            SDK {{ runtimeInfo.runtime_version }}
          </span>
        </template>

        <span v-if="linkedEnvName" class="jdv-env">{{ linkedEnvName }}</span>

        <button
          v-if="!connected"
          type="button"
          class="miyo-btn miyo-btn-primary"
          :disabled="!store.activeProject || connecting"
          data-testid="java-connect-btn"
          @click="connect"
        >
          <Icon name="plug" :size="12" />
          {{ connecting ? t('java.connecting') : t('java.connect') }}
        </button>
        <button v-else type="button" class="miyo-btn" @click="disconnect">
          {{ t('java.disconnect') }}
        </button>
        <span
          v-if="store.connection.state === 'fail'"
          class="jdv-conn-error"
          :title="store.connection.message"
        >{{ store.connection.message }}</span>
      </div>

      <div class="jdv-top-right">
        <IconButton name="code" :title="t('java.setupGuide')" @click="guideOpen = true" />
        <IconButton name="save" :title="t('java.saveCase')" :disabled="!store.activeMethod" @click="saveCaseOpen = true" />
        <IconButton name="pencil" :title="t('java.projectEdit')" :disabled="!store.activeProject" @click="editSettings" />
        <IconButton name="copy" :title="t('java.copyArgs')" :disabled="!store.activeMethod" @click="copyArgs" />
        <IconButton name="history" :title="t('java.historyTitle')" :disabled="!store.activeProject" @click="historyOpen = true" />
      </div>
    </header>

    <!-- 首次进入：整页接入向导 -->
    <JavaOnboarding
      v-if="needsOnboarding"
      @new-remote="openNewProject"
      @open-guide="guideOpen = true"
      @use-candidate="useCandidate"
    />

    <div v-else class="jdv-body">
      <SplitPanes :width="280" :min="200" :max="420">
        <template #first>
          <JavaClassBrowser
              :classes="store.classes"
              :loading="store.classesLoading"
              :connected="connected"
              :active-class-name="store.activeClassName"
              :search="store.classSearch"
              :recent="store.recentMethods.filter((r) => r.projectId === store.activeProjectId)"
              :is-favorite="store.isFavorite"
              @update:search="store.classSearch = $event"
              @select="store.selectClass($event)"
              @toggle-favorite="store.toggleFavorite($event)"
              @pick-recent="pickRecent"
            />
        </template>
        <template #second>
          <SplitPanes :width="460" :min="320" :max="720" width-on="second">
            <template #first>
              <JavaMethodPanel
                :class-name="store.activeClassName"
                :methods="store.methods"
                :loading="store.methodsLoading"
                :active-method-id="store.activeMethodId"
                :args-json="store.argsJson"
                :edit-mode="store.editMode"
                :invoking="store.invoking"
                :has-connection="connected"
                :has-template="!!store.argsTemplate"
                @select-method="store.selectMethod($event)"
                @update:args-json="store.argsJson = $event"
                @update:edit-mode="store.editMode = $event"
                @update:timeout="store.timeoutOverride = $event"
                @save-template="onSaveTemplate"
                @use-template="onUseTemplate"
                @run="run"
                @cancel="cancelRun"
              />
            </template>
            <template #second>
              <JavaResultPanel
                :response="store.lastResponse"
                :started-at="store.lastStartedAt"
                :args-json="store.argsJson"
                :params="store.activeMethod?.params ?? []"
                :invoking="store.invoking"
                :history="store.history.slice(0, 10)"
                @rerun="rerunFromPanel"
              />
            </template>
          </SplitPanes>
        </template>
      </SplitPanes>
    </div>

    <JavaHistoryDrawer
      :open="historyOpen"
      :records="store.history"
      :loading="store.historyLoading"
      @close="historyOpen = false"
      @rerun="rerun"
      @changed="refreshHistory"
    />

    <JavaProjectDialog
      :open="projectDialogOpen"
      :editing="editingProject"
      :preset="projectPreset"
      @close="projectDialogOpen = false"
      @saved="onProjectSaved"
    />

    <JavaSetupGuide :open="guideOpen" @close="guideOpen = false" />

    <JavaSaveTestCaseDialog
      :open="saveCaseOpen"
      @close="saveCaseOpen = false"
    />
  </div>
</template>

<style scoped>
.jdv {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
}
.jdv-top {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.jdv-top-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.jdv-title {
  margin: 0;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
}
.jdv-top-center {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
}
.jdv-env {
  font-size: 11.5px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  white-space: nowrap;
}
.jdv-chip {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  white-space: nowrap;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
}
.jdv-conn-error {
  font-size: 11px;
  color: var(--danger, #ef4444);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 320px;
}
.jdv-top-right {
  display: inline-flex;
  gap: 4px;
}
.jdv-body {
  flex: 1;
  min-height: 0;
}
</style>
