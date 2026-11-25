<script setup lang="ts">
/**
 * JavaDebugView：Java 方法实时调试工作台（顶级路由 /java-debug）。
 *
 * 2.0 双模式（默认 attach，localStorage 记忆）：
 * - attach：本地 JVM 自动发现 → 用户确认 → 动态注入 Agent → 类 / Bean /
 *   方法发现 → 受控调用（零源码侵入；左栏 进程/Bean 页签 + 中列 类/方法 +
 *   右列 结果，复用 v1 的类浏览器 / 方法面板 / 结果面板 / 历史抽屉）；
 * - runtime：v1 SDK 嵌入端点工作台原样保留（旧项目兼容连接方式）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import PageHeader from '@/components/navigation/PageHeader.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SplitPanes from '@/components/ui/SplitPanes.vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import { useToast } from '@/composables/useToast'
import { useGoBack } from '@/composables/useGoBack'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import JavaAttachConfirmDialog from '@/domains/java-debug/components/JavaAttachConfirmDialog.vue'
import JavaBeansPanel from '@/domains/java-debug/components/JavaBeansPanel.vue'
import JavaClassBrowser from '@/domains/java-debug/components/JavaClassBrowser.vue'
import JavaHistoryDrawer from '@/domains/java-debug/components/JavaHistoryDrawer.vue'
import JavaMethodPanel from '@/domains/java-debug/components/JavaMethodPanel.vue'
import JavaOnboarding from '@/domains/java-debug/components/JavaOnboarding.vue'
import JavaProcessPanel from '@/domains/java-debug/components/JavaProcessPanel.vue'
import JavaProjectDialog from '@/domains/java-debug/components/JavaProjectDialog.vue'
import JavaResultPanel from '@/domains/java-debug/components/JavaResultPanel.vue'
import JavaSaveTestCaseDialog from '@/domains/java-debug/components/JavaSaveTestCaseDialog.vue'
import JavaSetupGuide from '@/domains/java-debug/components/JavaSetupGuide.vue'
import { useJavaAttachStore } from '@/domains/java-debug/stores/javaAttach'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'
import { errorText } from '@/utils/errorText'

type ViewMode = 'attach' | 'runtime'
const MODE_KEY = 'miyo.java.mode'

/** 统一返回策略（第四阶段）：有会话历史原路返回，深链直开回仪表板兜底。 */
const goBackFrom = useGoBack()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const workspace = useWorkspaceStore()

const attachStore = useJavaAttachStore()
const runtimeStore = useJavaDebugStore()

// ---------- 模式切换（attach 默认；runtime 兼容 v1 项目） ----------
const viewMode = ref<ViewMode>(loadMode())

// ---------- 分栏宽度：按模式持久化（miyo.java-split.<mode>，刷新恢复） ----------
const splitState = ref<Record<string, number>>(loadSplits())

const attachOuter = computed({
  get: () => splitFor('attachOuter', 300),
  set: (v) => setSplit('attachOuter', v),
})
const attachInner = computed({
  get: () => splitFor('attachInner', 260),
  set: (v) => setSplit('attachInner', v),
})
const runtimeOuter = computed({
  get: () => splitFor('runtimeOuter', 280),
  set: (v) => setSplit('runtimeOuter', v),
})
const runtimeInner = computed({
  get: () => splitFor('runtimeInner', 460),
  set: (v) => setSplit('runtimeInner', v),
})

function loadSplits(): Record<string, number> {
  try {
    const raw = localStorage.getItem('miyo.java-split')
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, unknown>
    const out: Record<string, number> = {}
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof v === 'number' && Number.isFinite(v)) out[k] = v
    }
    return out
  } catch {
    return {}
  }
}

function splitFor(key: string, fallback: number): number {
  const v = splitState.value[key]
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback
}

function setSplit(key: string, v: number): void {
  splitState.value = { ...splitState.value, [key]: v }
  try {
    localStorage.setItem('miyo.java-split', JSON.stringify(splitState.value))
  } catch {
    // 写失败静默
  }
}

function loadMode(): ViewMode {
  try {
    const raw = localStorage.getItem(MODE_KEY)
    return raw === 'runtime' ? 'runtime' : 'attach'
  } catch {
    return 'attach'
  }
}

watch(viewMode, (mode) => {
  try {
    localStorage.setItem(MODE_KEY, mode)
  } catch {
    // localStorage 不可用：会话内存即可
  }
  if (mode === 'attach') {
    runtimeStore.dispose()
    void attachStore.loadProcesses()
    attachStore.setAutoRefresh(attachStore.autoRefresh)
  } else {
    attachStore.dispose()
    void runtimeStore.loadProjects()
  }
})

// ---------------------------------------------------------------------------
// attach 工作台
// ---------------------------------------------------------------------------
const leftTab = ref<'processes' | 'beans'>('processes')
const attachDialogPid = ref<number | null>(null)
const historyOpenAttach = ref(false)

const attachTarget = computed(
  () => attachStore.processes.find((p) => p.pid === attachDialogPid.value) ?? null,
)

const attachedPids = computed(() => [...attachStore.sessions.keys()])

onMounted(async () => {
  if (viewMode.value === 'attach') {
    await attachStore.loadProcesses()
    attachStore.setAutoRefresh(true)
    // 组件自动获取：agent / launcher jar 缺失即后台下载（sha256 校验 + 原子落盘）
    void attachStore.ensureComponents()
  } else {
    await runtimeStore.loadProjects()
    if (runtimeStore.projects.length > 0 && runtimeStore.connection.state !== 'ok') {
      const ok = await runtimeStore.connect()
      if (ok) await runtimeStore.loadClasses()
    }
  }
})

onBeforeUnmount(() => {
  attachStore.setAutoRefresh(false)
})

function requestAttach(pid: number): void {
  attachDialogPid.value = pid
}

async function confirmAttach(): Promise<void> {
  const pid = attachDialogPid.value
  if (pid == null) return
  const ok = await attachStore.attach(pid)
  attachDialogPid.value = null
  if (ok) {
    toast.success(t('java2.attachedToast', { pid }))
    leftTab.value = 'processes'
  } else {
    toast.error(t('java2.attachFailedToast'))
  }
}

async function detachActive(): Promise<void> {
  const pid = attachStore.activePid
  if (pid == null) return
  await attachStore.detach(pid)
  toast.success(t('java2.detachedToast'))
}

async function unloadActive(): Promise<void> {
  const pid = attachStore.activePid
  if (pid == null) return
  try {
    await attachStore.unloadAgent(pid)
    toast.success(t('java2.unloadedToast'))
  } catch (e) {
    toast.error(errorText(e))
  }
}

async function manualProbe(pid: number): Promise<void> {
  const info = await attachStore.probeManualPid(pid)
  if (info && info.attachable) {
    requestAttach(pid)
  } else {
    toast.error(info?.attach_block_reason ?? t('java2.manualPidNotFound'))
  }
}

/** Bean 点击 → 选中其实现类的类浏览（方法发现走 actual_type）。 */
function selectBean(bean: import('@/types/miyoApi').JavaBeanInfo): void {
  const className = bean.actual_type ?? bean.definition_type
  if (className) {
    void attachStore.selectClass(className)
    leftTab.value = 'processes'
  }
}

async function attachRun(): Promise<void> {
  try {
    const response = await attachStore.invoke()
    if (!response) return
    if (response.status === 'success') {
      toast.success(t('java.invokeSuccessToast'), {
        message: t('java.invokeSuccessMeta', {
          duration: String(response.duration_ms ?? 0),
          instance: response.instance_kind ?? '',
        }),
      })
    } else if (response.status === 'timeout') {
      toast.warning(t('java.invokeTimeoutToast'))
    } else {
      toast.warning(t('java.invokeFailedToast'))
    }
  } catch (e) {
    // 链路级失败（权限 / 未连接 / 参数非法）必须弹出来：
    // 原先 attachRun 不 catch，rejection 只在 console 里，界面毫无反馈。
    toast.error(t('java.invokeErrorToast'), { message: errorText(e), duration: 6000 })
  }
}

async function attachCancel(): Promise<void> {
  try {
    await attachStore.cancelLast()
    toast.success(t('java.cancelSent'))
  } catch (e) {
    toast.error(errorText(e))
  }
}

/** 类搜索 → 服务端过滤（Agent classes.list search）+ 本地浏览器过滤叠加。 */
function onAttachClassSearch(term: string): void {
  attachStore.classSearch = term
  void attachStore.loadClasses()
}

function onAttachPackageFilter(value: string): void {
  attachStore.packageFilter = value
  void attachStore.loadClasses()
}

function onAttachIncludeDependencies(value: boolean): void {
  attachStore.includeDependencies = value
  void attachStore.loadClasses()
}

function onAttachIncludeJdk(value: boolean): void {
  attachStore.includeJdk = value
  void attachStore.loadClasses()
}

/** 方法点击：v1 形状方法 + 原始 agent 方法（风险 / param_types）一起入 store。 */
function agentMethodOf(method: import('@/types/miyoApi').JavaMethodInfo):
  | import('@/types/miyoApi').JavaAgentMethodInfo
  | undefined {
  return attachStore.agentMethods.find((m) => m.method_id === method.method_id)
}

function rerunAttach(record: import('@/types/miyoApi').JavaInvocationRecord): void {
  const method = attachStore.methods.find((m) => m.method_id === record.method_id)
  if (method) {
    attachStore.selectMethod(method)
  }
  attachStore.argsJson = JSON.stringify(record.arguments ?? [], null, 2)
  attachStore.editMode = 'json'
  attachStore.lastResponse = null
  historyOpenAttach.value = false
}

const attachSessionTone = computed<StatusTone>(() => {
  if (attachStore.attached) return 'ok'
  if (attachStore.attachingPid != null) return 'wait'
  return 'wait'
})

const sessionLabel = computed(() => {
  if (attachStore.attached) return t('java2.sessionAttached')
  if (attachStore.attachingPid != null) return t('java2.attaching')
  return t('java2.sessionIdle')
})

// ---------------------------------------------------------------------------
// runtime 工作台（v1 原样保留；逻辑与既有版本一致）
// ---------------------------------------------------------------------------
const projectDialogOpen = ref(false)
const editingProject = ref<import('@/types/miyoApi').JavaDebugProject | null>(null)
const projectPreset = ref<{ name?: string; endpoint?: string } | null>(null)
const historyOpen = ref(false)
const connecting = ref(false)
const guideOpen = ref(false)
const saveCaseOpen = ref(false)

const runtimeConnected = computed(() => runtimeStore.connection.state === 'ok')
const needsOnboarding = computed(
  () => viewMode.value === 'runtime' && runtimeStore.projects.length === 0,
)

const projectOptions = computed(() =>
  runtimeStore.projects.map((p) => ({ value: p.id, label: p.name })),
)

const connectionTone = computed<StatusTone>(() => {
  switch (runtimeStore.connection.state) {
    case 'ok':
      return 'ok'
    case 'fail':
      return 'fail'
    default:
      return 'wait'
  }
})

const connectionLabel = computed(() => {
  switch (runtimeStore.connection.state) {
    case 'ok':
      return t('java.connOk')
    case 'fail':
      return t('java.connFail')
    default:
      return t('java.connIdle')
  }
})

const runtimeInfo = computed(() => runtimeStore.connection.info)

const linkedEnvName = computed(() => {
  const envId = runtimeStore.activeProject?.environment_id
  if (!envId) return null
  return workspace.environments.find((e) => e.id === envId)?.name ?? null
})

async function connect(): Promise<void> {
  if (!runtimeStore.activeProject) return
  connecting.value = true
  const ok = await runtimeStore.connect().finally(() => {
    connecting.value = false
  })
  if (ok) {
    toast.success(t('java.connectedToast', { app: runtimeStore.activeProject.name }))
    await runtimeStore.loadClasses()
  } else {
    toast.error(runtimeStore.connection.message || t('java.connFail'))
  }
}

function disconnect(): void {
  runtimeStore.disconnect()
}

function goBack(): void {
  goBackFrom()
}

function openNewProject(): void {
  editingProject.value = null
  projectPreset.value = null
  projectDialogOpen.value = true
}

function openEditProject(): void {
  if (!runtimeStore.activeProject) return
  editingProject.value = runtimeStore.activeProject
  projectDialogOpen.value = true
}

function useCandidate(runtime: import('@/types/miyoApi').JavaDiscoveredRuntime): void {
  editingProject.value = null
  projectPreset.value = {
    endpoint: runtime.endpoint,
    name: runtime.app_name || undefined,
  }
  projectDialogOpen.value = true
}

async function onProjectSaved(project: import('@/types/miyoApi').JavaDebugProject): Promise<void> {
  await runtimeStore.loadProjects()
  runtimeStore.activeProjectId = project.id
  const ok = await runtimeStore.connect()
  if (ok) await runtimeStore.loadClasses()
}

function editSettings(): void {
  openEditProject()
}

async function run(): Promise<void> {
  const response = await runtimeStore.invoke()
  if (response && response.status !== 'success') {
    toast.warning(t('java.invokeFailedToast'))
  }
}

async function cancelRun(): Promise<void> {
  try {
    await runtimeStore.cancelLast()
    toast.success(t('java.cancelSent'))
  } catch (e) {
    toast.error(errorText(e))
  }
}

async function refreshHistory(): Promise<void> {
  await runtimeStore.loadHistory()
}

function rerun(record: import('@/types/miyoApi').JavaInvocationRecord): void {
  runtimeStore.applyFromRecord(record)
  historyOpen.value = false
}

function rerunFromPanel(record: import('@/types/miyoApi').JavaInvocationRecord): void {
  runtimeStore.applyFromRecord(record)
  void runtimeStore.selectClass(record.class_name).then(() => {
    const found = runtimeStore.methods.find((m) => m.method_id === record.method_id)
    if (found) runtimeStore.selectMethod(found)
    runtimeStore.applyFromRecord(record)
  })
}

function pickRecent(methodId: string): void {
  const method = runtimeStore.methods.find((m) => m.method_id === methodId)
  if (method) {
    runtimeStore.selectMethod(method)
    return
  }
  const className = methodId.split('#')[0] ?? ''
  void runtimeStore.selectClass(className).then(() => {
    const found = runtimeStore.methods.find((m) => m.method_id === methodId)
    if (found) runtimeStore.selectMethod(found)
  })
}

async function copyArgs(): Promise<void> {
  try {
    await navigator.clipboard.writeText(runtimeStore.argsJson)
    toast.success(t('java.copied'))
  } catch {
    toast.error(t('java.copyFailed'))
  }
}

function onSaveTemplate(): void {
  runtimeStore.saveArgsTemplate()
  toast.success(t('java.argsTemplateSaved'))
}

function onUseTemplate(): void {
  if (runtimeStore.argsTemplate != null) runtimeStore.argsJson = runtimeStore.argsTemplate
}

const modeOptions = computed(() => [
  { value: 'attach', label: t('java2.modeAttach') },
  { value: 'runtime', label: t('java2.modeRuntime') },
])
</script>

<template>
  <div class="jdv">
    <PageHeader :title="t('java.viewTitle')" @back="goBack">
      <!-- 连接模式切换：attach 默认推荐（零源码侵入）；runtime 兼容 v1 SDK 项目 -->
      <div class="jdv-mode" data-testid="java-mode-switch">
        <button
          v-for="opt in modeOptions"
          :key="opt.value"
          type="button"
          class="jdv-mode-btn"
          :class="{ active: viewMode === opt.value }"
          :data-testid="`java-mode-${opt.value}`"
          @click="viewMode = opt.value as ViewMode"
        >
          {{ opt.label }}
        </button>
      </div>

      <!-- attach 顶栏：会话状态 + 目标信息 + 断开 / 卸载 -->
      <div v-if="viewMode === 'attach'" class="jdv-top-center">
        <StatusBadge :tone="attachSessionTone" :label="sessionLabel" :dot="attachStore.attached" />
        <template v-if="attachStore.activeSession">
          <span class="jdv-chip" :title="attachStore.activeSession.handshake.java_home">
            PID {{ attachStore.activePid }}
          </span>
          <span class="jdv-chip">
            {{ t('java.chipJava') }} {{ attachStore.activeSession.handshake.java_version }}
          </span>
          <span class="jdv-chip" :title="`agent ${attachStore.activeSession.agent_version} · protocol v${attachStore.activeSession.protocol}`">
            agent {{ attachStore.activeSession.agent_version }}
          </span>
          <button type="button" class="miyo-btn miyo-btn-sm" @click="detachActive">
            {{ t('java2.detach') }}
          </button>
          <button type="button" class="miyo-btn miyo-btn-sm" @click="unloadActive">
            {{ t('java2.unload') }}
          </button>
        </template>
      </div>

      <!-- runtime 顶栏（v1 原样） -->
      <div v-else class="jdv-top-center">
        <CustomSelect
          :model-value="runtimeStore.activeProjectId ?? ''"
          :options="projectOptions"
          size="sm"
          :placeholder="t('java.projectSelect')"
          data-testid="java-project-select"
          @update:model-value="(v: string | number | null) => (runtimeStore.activeProjectId = v ? String(v) : null)"
        />
        <IconButton name="plus" :title="t('java.projectNew')" @click="openNewProject" />

        <StatusBadge :tone="connectionTone" :label="connectionLabel" :dot="runtimeStore.connection.state === 'ok'" />

        <template v-if="runtimeConnected && runtimeInfo">
          <span class="jdv-chip" :title="runtimeInfo.jvm_version">
            {{ t('java.chipApp') }}: {{ runtimeInfo.app_name || runtimeStore.activeProject?.name }}
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
          v-if="!runtimeConnected"
          type="button"
          class="miyo-btn miyo-btn-primary"
          :disabled="!runtimeStore.activeProject || connecting"
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
          v-if="runtimeStore.connection.state === 'fail'"
          class="jdv-conn-error"
          :title="runtimeStore.connection.message"
        >{{ runtimeStore.connection.message }}</span>
      </div>

      <template #actions>
        <IconButton
          v-if="viewMode === 'runtime'"
          name="code"
          :title="t('java.setupGuide')"
          @click="guideOpen = true"
        />
        <IconButton
          v-if="viewMode === 'runtime'"
          name="save"
          :title="t('java.saveCase')"
          :disabled="!runtimeStore.activeMethod"
          @click="saveCaseOpen = true"
        />
        <IconButton
          v-if="viewMode === 'runtime'"
          name="pencil"
          :title="t('java.projectEdit')"
          :disabled="!runtimeStore.activeProject"
          @click="editSettings"
        />
        <IconButton
          v-if="viewMode === 'runtime'"
          name="copy"
          :title="t('java.copyArgs')"
          :disabled="!runtimeStore.activeMethod"
          @click="copyArgs"
        />
        <IconButton
          name="history"
          :title="t('java.historyTitle')"
          :disabled="viewMode === 'runtime' ? !runtimeStore.activeProject : !attachStore.historyProjectId"
          @click="viewMode === 'attach' ? (historyOpenAttach = true) : (historyOpen = true)"
        />
      </template>
    </PageHeader>

    <!-- runtime 首次进入：整页接入向导（v1 流程） -->
    <JavaOnboarding
      v-if="needsOnboarding"
      @new-remote="openNewProject"
      @open-guide="guideOpen = true"
      @use-candidate="useCandidate"
    />

    <!-- attach 工作台：进程/Bean | 类浏览器 | 方法+参数 | 结果 -->
    <div v-else-if="viewMode === 'attach'" class="jdv-body" data-testid="java-attach-workbench">
      <SplitPanes
        v-model:width="attachOuter"
        :min="240"
        :max="440"
        :stack-below="720"
      >
        <template #first>
          <div class="jdv-left">
            <div class="jdv-left-tabs">
              <button
                type="button"
                class="jdv-left-tab"
                :class="{ active: leftTab === 'processes' }"
                data-testid="java-tab-processes"
                @click="leftTab = 'processes'"
              >
                {{ t('java2.tabProcesses') }}
              </button>
              <button
                type="button"
                class="jdv-left-tab"
                :class="{ active: leftTab === 'beans' }"
                data-testid="java-tab-beans"
                @click="leftTab = 'beans'"
              >
                {{ t('java2.tabBeans') }}
              </button>
            </div>
            <JavaProcessPanel
              v-if="leftTab === 'processes'"
              :summary="attachStore.summary"
              :processes="attachStore.filteredProcesses"
              :loading="attachStore.scanning"
              :search="attachStore.processSearch"
              :active-pid="attachStore.activePid"
              :attached-pids="attachedPids"
              :auto-refresh="attachStore.autoRefresh"
              :components-downloading="attachStore.componentsDownloading"
              @update:search="attachStore.processSearch = $event"
              @update:auto-refresh="attachStore.setAutoRefresh($event)"
              @refresh="attachStore.loadProcesses()"
              @select="attachStore.activePid = $event"
              @download-components="attachStore.downloadComponents()"
              @attach="requestAttach"
              @manual-pid="manualProbe"
            />
            <JavaBeansPanel
              v-else
              :status="attachStore.springStatus"
              :beans="attachStore.beans"
              :total="attachStore.beanTotal"
              :loading="attachStore.beansLoading"
              :search="attachStore.beanSearch"
              :active-bean-name="null"
              @update:search="attachStore.beanSearch = $event"
              @select="selectBean"
              @reload="attachStore.loadBeans()"
            />
          </div>
        </template>
        <template #second>
          <SplitPanes
            v-model:width="attachInner"
            :min="200"
            :max="400"
          >
            <template #first>
              <JavaClassBrowser
                :classes="attachStore.classes"
                :loading="attachStore.classesLoading"
                :connected="attachStore.attached"
                :active-class-name="attachStore.activeClassName"
                :search="attachStore.classSearch"
                :recent="[]"
                :is-favorite="attachStore.isFavorite"
                :categorized="true"
                :stats="attachStore.classStats"
                :has-more="attachStore.classHasMore"
                :loading-more="attachStore.classesLoadingMore"
                :selected-categories="attachStore.categoryFilter"
                :package-filter="attachStore.packageFilter"
                :include-dependencies="attachStore.includeDependencies"
                :include-jdk="attachStore.includeJdk"
                :business-packages="attachStore.businessPackages"
                @update:search="onAttachClassSearch"
                @select="attachStore.selectClass($event)"
                @toggle-favorite="attachStore.toggleFavorite($event)"
                @update:package-filter="onAttachPackageFilter"
                @toggle-category="attachStore.toggleCategory($event)"
                @update:include-dependencies="onAttachIncludeDependencies"
                @update:include-jdk="onAttachIncludeJdk"
                @load-more="attachStore.loadMoreClasses()"
                @save-business-packages="attachStore.saveBusinessPackages($event)"
                @restore-defaults="attachStore.restoreClassFilterDefaults()"
              />
            </template>
            <template #second>
              <JavaMethodPanel
                :class-name="attachStore.activeClassName"
                :methods="attachStore.methods"
                :loading="attachStore.methodsLoading"
                :active-method-id="attachStore.activeMethodId"
                :args-json="attachStore.argsJson"
                :edit-mode="attachStore.editMode"
                :invoking="attachStore.invoking"
                :has-connection="attachStore.attached"
                :has-template="false"
                attach-mode
                :can-instantiate="true"
                v-model:instance-kind="attachStore.instanceKind"
                v-model:new-consent="attachStore.newConsent"
                @select-method="attachStore.selectMethod($event, agentMethodOf($event))"
                @update:args-json="attachStore.argsJson = $event"
                @update:edit-mode="attachStore.editMode = $event"
                @update:timeout="attachStore.timeoutOverride = $event"
                @run="attachRun"
                @cancel="attachCancel"
              />
            </template>
          </SplitPanes>
          <!--
            结果区（attach 分支原先缺失）：点击「执行」后 only 有中列的 Spinner
            闪一下，返回值 / 异常 / 耗时没有任何落点，用户视角等于「点了没反应」
            而方法确实执行了（真实反馈）。与 runtime 分支保持同一组件。
          -->
          <div class="jdv-attach-result">
            <JavaResultPanel
              :response="attachStore.lastResponse"
              :started-at="attachStore.lastStartedAt"
              :args-json="attachStore.argsJson"
              :params="attachStore.activeMethod?.params ?? []"
              :invoking="attachStore.invoking"
              :history="attachStore.history.slice(0, 10)"
              @rerun="rerunFromPanel"
            />
          </div>
        </template>
      </SplitPanes>
    </div>

    <!-- runtime 工作台（v1 原样） -->
    <div v-else class="jdv-body">
      <SplitPanes
        v-model:width="runtimeOuter"
        :min="200"
        :max="420"
        :stack-below="720"
      >
        <template #first>
          <JavaClassBrowser
              :classes="runtimeStore.classes"
              :loading="runtimeStore.classesLoading"
              :connected="runtimeConnected"
              :active-class-name="runtimeStore.activeClassName"
              :search="runtimeStore.classSearch"
              :recent="runtimeStore.recentMethods.filter((r) => r.projectId === runtimeStore.activeProjectId)"
              :is-favorite="runtimeStore.isFavorite"
              @update:search="runtimeStore.classSearch = $event"
              @select="runtimeStore.selectClass($event)"
              @toggle-favorite="runtimeStore.toggleFavorite($event)"
              @pick-recent="pickRecent"
            />
        </template>
        <template #second>
          <SplitPanes
            v-model:width="runtimeInner"
            :min="320"
            :max="720"
            width-on="second"
          >
            <template #first>
              <JavaMethodPanel
                :class-name="runtimeStore.activeClassName"
                :methods="runtimeStore.methods"
                :loading="runtimeStore.methodsLoading"
                :active-method-id="runtimeStore.activeMethodId"
                :args-json="runtimeStore.argsJson"
                :edit-mode="runtimeStore.editMode"
                :invoking="runtimeStore.invoking"
                :has-connection="runtimeConnected"
                :has-template="!!runtimeStore.argsTemplate"
                @select-method="runtimeStore.selectMethod($event)"
                @update:args-json="runtimeStore.argsJson = $event"
                @update:edit-mode="runtimeStore.editMode = $event"
                @update:timeout="runtimeStore.timeoutOverride = $event"
                @save-template="onSaveTemplate"
                @use-template="onUseTemplate"
                @run="run"
                @cancel="cancelRun"
              />
            </template>
            <template #second>
              <JavaResultPanel
                :response="runtimeStore.lastResponse"
                :started-at="runtimeStore.lastStartedAt"
                :args-json="runtimeStore.argsJson"
                :params="runtimeStore.activeMethod?.params ?? []"
                :invoking="runtimeStore.invoking"
                :history="runtimeStore.history.slice(0, 10)"
                @rerun="rerunFromPanel"
              />
            </template>
          </SplitPanes>
        </template>
      </SplitPanes>
    </div>

    <!-- 历史抽屉（双模式共用组件；数据源按模式切换） -->
    <JavaHistoryDrawer
      v-if="viewMode === 'runtime'"
      :open="historyOpen"
      :records="runtimeStore.history"
      :loading="runtimeStore.historyLoading"
      @close="historyOpen = false"
      @rerun="rerun"
      @changed="refreshHistory"
    />
    <JavaHistoryDrawer
      v-else
      :open="historyOpenAttach"
      :records="attachStore.history"
      :loading="attachStore.historyLoading"
      @close="historyOpenAttach = false"
      @rerun="rerunAttach"
      @changed="attachStore.loadHistory()"
    />

    <JavaAttachConfirmDialog
      :open="attachDialogPid != null"
      :target="attachTarget"
      :busy="attachStore.attachingPid != null"
      @close="attachDialogPid = null"
      @confirm="confirmAttach"
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
.jdv-mode {
  display: flex;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.jdv-mode-btn {
  font-size: var(--fs-xxs);
  padding: 3px 10px;
  color: var(--text-2);
  background: transparent;
  border: none;
  cursor: pointer;
}
.jdv-mode-btn.active {
  background: var(--bg-hover);
  color: var(--text-1);
  font-weight: 600;
}
.jdv-top-center {
  display: flex;
  align-items: center;
  gap: 8px;
  /* 内容自适应 + 可收缩（溢出经 chips ellipsis 兜底）；剩余空间由
     PageHeader 的 topbar spacer 吸收，把操作区顶到最右 */
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
}
.jdv-env {
  font-size: var(--fs-xxs);
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-2);
  white-space: nowrap;
}
.jdv-chip {
  font-size: var(--fs-xxs);
  padding: 2px 8px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-2);
  white-space: nowrap;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
}
.jdv-conn-error {
  font-size: var(--fs-xxs);
  color: var(--danger, #ef4444);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 320px;
}
.jdv-body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.jdv-left {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.jdv-left-tabs {
  display: flex;
  gap: 2px;
  padding: 8px 10px 0;
  flex-shrink: 0;
}
.jdv-left-tab {
  font-size: var(--fs-xs);
  padding: 4px 12px;
  border: 1px solid transparent;
  border-bottom: none;
  border-radius: var(--radius-md) var(--radius-md) 0 0;
  color: var(--text-2);
  background: transparent;
  cursor: pointer;
}
.jdv-left-tab.active {
  color: var(--text-1);
  font-weight: 600;
  border-color: var(--border);
  background: var(--bg-hover);
}
/* attach 工作台：左列（SplitPanes first）+ 右侧弹性区（中列 + 结果） */
.jdv-body > :deep(.split-panes) {
  flex: 1;
  min-width: 0;
}
/* attach 中列右侧的结果区：与 runtime 分支同宽高，独立滚动 */
.jdv-attach-result {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  height: 100%;
  border-left: 1px solid var(--border);
  overflow: hidden;
}
</style>
