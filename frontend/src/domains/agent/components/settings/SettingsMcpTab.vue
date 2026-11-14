<script setup lang="ts">
/**
 * SettingsMcpTab：MCP 服务页签（自 SettingsDialog 拆出，独立加载）。
 *
 * 自包含：启停开关 / 监听端口 / 运行状态 / 工具权限（read·write 分组逐个
 * 启停）。挂载时自行 loadMcpStatus（父级不再预取）；启停与端口保存后回读
 * 状态，失败路径同样回读（状态与后端事实对齐）。
 */
import { computed, onMounted, ref } from 'vue'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import type { McpPermissionStatus, McpToolStatus } from '@/types/miyoApi.d'
import CustomNumberInput from '@/components/ui/CustomNumberInput.vue'
import Modal from '@/components/ui/Modal.vue'
import Icon from '@/components/ui/Icon.vue'
import { errorText } from '@/utils/errorText'

const api = useMiyoApi()
const toast = useToast()
const t = useLocaleStore().t

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

/**
 * 启停开关：首次启用先展示权限说明（用户确认后才真正开启——MCP 会把
 * 项目数据暴露给 AI，高风险权限默认关闭）；先应用（启动失败不落盘）再刷新状态。
 */
const permExplainOpen = ref(false)
async function toggleMcp(): Promise<void> {
  const next = !mcpEnabled.value
  if (next) {
    // 首次启用 → 权限说明确认（确认后真正调用 setMcpEnabled）。
    permExplainOpen.value = true
    return
  }
  await applyMcpEnabled(false)
}

/** 用户确认权限说明后真正启用。 */
async function confirmPermExplain(): Promise<void> {
  permExplainOpen.value = false
  await applyMcpEnabled(true)
}

async function applyMcpEnabled(next: boolean): Promise<void> {
  mcpBusy.value = true
  try {
    await api.setMcpEnabled(next)
    mcpEnabled.value = next
    await loadMcpStatus()
    toast.success(next ? t('settings.mcpEnabledToast') : t('settings.mcpDisabledToast'))
  } catch (err) {
    toast.error(t('settings.saveFail'), {
      message: errorText(err),
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
      message: errorText(err),
    })
    await loadMcpStatus()
  } finally {
    mcpBusy.value = false
  }
}

// ---------- MCP 权限等级（read/write/execute 默认启用；高风险需确认） ----------
const mcpPerms = ref<McpPermissionStatus[]>([])
const mcpPermBusy = ref(false)
/** 待确认启用的高风险等级（确认对话框数据）。 */
const pendingHighRisk = ref<McpPermissionStatus | null>(null)
/** 对话框开关的 computed 包装（关闭 = 放弃启用）。 */
const pendingHighRiskConfirm = computed({
  get: () => pendingHighRisk.value !== null,
  set: (v: boolean) => {
    if (!v) pendingHighRisk.value = null
  },
})
const pendingHighRiskLabel = computed(() =>
  pendingHighRisk.value ? permDesc(pendingHighRisk.value.level) : '',
)

/** 等级说明走 i18n（后端 description 仅兜底）。 */
const permDesc = (level: string): string => {
  const key = `settings.mcpPermLevel.${level}`
  const translated = t(key)
  return translated === key ? (mcpPerms.value.find((x) => x.level === level)?.description ?? '') : translated
}

async function loadMcpPermissions(): Promise<void> {
  try {
    mcpPerms.value = await api.getMcpPermissions()
  } catch {
    mcpPerms.value = []
  }
}

async function toggleMcpPerm(perm: McpPermissionStatus): Promise<void> {
  const next = !perm.enabled
  if (next && perm.highRisk) {
    // 高风险等级：必须用户在对话框中明确确认才启用。
    pendingHighRisk.value = perm
    return
  }
  const enabled = mcpPerms.value.filter((x) => x.enabled).map((x) => x.level)
  await savePermissions(
    next ? Array.from(new Set([...enabled, perm.level])) : enabled.filter((l) => l !== perm.level),
  )
}

/** 用户确认启用高风险等级。 */
async function confirmHighRisk(): Promise<void> {
  const perm = pendingHighRisk.value
  pendingHighRisk.value = null
  if (!perm) return
  await savePermissions(
    Array.from(new Set([...mcpPerms.value.filter((x) => x.enabled).map((x) => x.level), perm.level])),
    true,
  )
}

async function savePermissions(levels: string[], acknowledgeHighRisk = false): Promise<void> {
  mcpPermBusy.value = true
  try {
    await api.setMcpPermissions({ levels, acknowledgeHighRisk })
    await loadMcpPermissions()
  } catch (err) {
    toast.error(t('settings.mcpPermSaveFail'), {
      message: errorText(err),
    })
    await loadMcpPermissions()
  } finally {
    mcpPermBusy.value = false
  }
}

// ---------- 自定义敏感字段规则（脱敏单源） ----------
const sensitivePatterns = ref<string[]>([])
const newPattern = ref('')
const patternsBusy = ref(false)

async function loadSensitivePatterns(): Promise<void> {
  try {
    sensitivePatterns.value = await api.getSensitivePatterns()
  } catch {
    sensitivePatterns.value = []
  }
}

async function addPattern(): Promise<void> {
  const v = newPattern.value.trim()
  if (!v || sensitivePatterns.value.includes(v)) {
    newPattern.value = ''
    return
  }
  patternsBusy.value = true
  try {
    await api.setSensitivePatterns([...sensitivePatterns.value, v])
    sensitivePatterns.value = await api.getSensitivePatterns()
    newPattern.value = ''
  } catch (err) {
    toast.error(t('settings.patternSaveFail'), {
      message: errorText(err),
    })
  } finally {
    patternsBusy.value = false
  }
}

async function removePattern(pattern: string): Promise<void> {
  patternsBusy.value = true
  try {
    await api.setSensitivePatterns(sensitivePatterns.value.filter((p) => p !== pattern))
    sensitivePatterns.value = await api.getSensitivePatterns()
  } catch (err) {
    toast.error(t('settings.patternSaveFail'), {
      message: errorText(err),
    })
  } finally {
    patternsBusy.value = false
  }
}

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
    toast.error(t('settings.mcpToolSaveFail'), { message: errorText(err) })
  } finally {
    mcpToolBusy.value = null
  }
}

onMounted(() => {
  void loadMcpStatus()
  void loadMcpPermissions()
  void loadSensitivePatterns()
  // 工具清单随页签自行加载（原为父级弹窗打开时预取；拆出后首访本页签时拉取）
  void api
    .getMcpTools()
    .then((v) => {
      mcpTools.value = v
    })
    .catch(() => {
      mcpTools.value = []
    })
})

</script>

<template>
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

      <!-- 权限等级：read/write/execute 默认启用；高风险（真实网络请求 / 删除）需明确确认 -->
      <div class="mt-5 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
        <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.mcpPermTitle') }}</div>
        <p class="mt-0.5 mb-4 text-xs text-zinc-600 dark:text-zinc-400">
          {{ t('settings.mcpPermDesc') }}
        </p>
        <div class="flex flex-col gap-2">
          <div
            v-for="perm in mcpPerms"
            :key="perm.level"
            class="flex items-center justify-between gap-4 rounded-lg border px-3.5 py-2.5"
            :class="perm.highRisk && perm.enabled
              ? 'border-amber-300 bg-amber-50/60 dark:border-amber-500/30 dark:bg-amber-500/[0.06]'
              : 'border-zinc-200/70 dark:border-white/[0.06]'"
          >
            <div class="min-w-0">
              <div class="flex items-center gap-2">
                <span class="text-xs font-medium text-zinc-900 dark:text-zinc-100">{{ permDesc(perm.level) }}</span>
                <code class="font-mono text-xxs text-zinc-500 dark:text-zinc-400">{{ perm.level }}</code>
                <span
                  v-if="perm.highRisk"
                  class="rounded px-1.5 py-0.5 text-xxs font-medium bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-400"
                >{{ t('settings.mcpPermHighRisk') }}</span>
              </div>
              <p class="mt-0.5 text-xxs text-zinc-600 dark:text-zinc-400">
                {{ t('settings.mcpPermToolCount', { n: perm.toolCount }) }}
              </p>
            </div>
            <button
              type="button"
              role="switch"
              :aria-checked="perm.enabled"
              :aria-label="permDesc(perm.level)"
              :disabled="mcpPermBusy"
              class="relative h-[22px] w-[40px] shrink-0 rounded-full transition-colors duration-150"
              :class="perm.enabled ? 'bg-purple-500' : 'border border-zinc-300 bg-zinc-200 dark:border-white/10 dark:bg-white/10'"
              @click="toggleMcpPerm(perm)"
            >
              <span
                class="absolute top-1/2 h-[16px] w-[16px] -translate-y-1/2 rounded-full bg-white shadow transition-all duration-150"
                :class="perm.enabled ? 'left-[21px]' : 'left-[2px]'"
              />
            </button>
          </div>
        </div>
      </div>

      <!-- 自定义敏感字段规则：脱敏单源的自定义词根（MCP 返回 / 导出全生效） -->
      <div class="mt-5 border-t border-zinc-200/70 pt-5 dark:border-white/[0.06]">
        <div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{{ t('settings.patternTitle') }}</div>
        <p class="mt-0.5 mb-3 text-xs text-zinc-600 dark:text-zinc-400">{{ t('settings.patternDesc') }}</p>
        <div class="flex items-center gap-2">
          <input
            v-model="newPattern"
            class="miyo-input flex-1"
            type="text"
            spellcheck="false"
            :placeholder="t('settings.patternPh')"
            :disabled="patternsBusy"
            @keydown.enter="addPattern"
          />
          <button
            type="button"
            class="miyo-btn miyo-btn-sm shrink-0"
            :disabled="patternsBusy || !newPattern.trim()"
            @click="addPattern"
          >
            {{ t('common.add') }}
          </button>
        </div>
        <div v-if="sensitivePatterns.length" class="mt-3 flex flex-wrap gap-1.5">
          <span
            v-for="p in sensitivePatterns"
            :key="p"
            class="inline-flex items-center gap-1 rounded-full bg-zinc-100 px-2.5 py-1 font-mono text-xxs text-zinc-700 dark:bg-white/[0.06] dark:text-zinc-300"
          >
            {{ p }}
            <button
              type="button"
              class="text-zinc-400 hover:text-red-500"
              :aria-label="t('common.delete')"
              :disabled="patternsBusy"
              @click="removePattern(p)"
            >
              ×
            </button>
          </span>
        </div>
        <p v-else class="mt-3 text-xxs text-zinc-500 dark:text-zinc-400">{{ t('settings.patternEmpty') }}</p>
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

<!-- 首次启用：权限说明（确认后才真正开启 MCP） -->
<Modal v-model:open="permExplainOpen" :title="t('settings.mcpPermFirstTitle')" width="480px">
  <div class="flex flex-col gap-3 text-xs leading-relaxed text-zinc-700 dark:text-zinc-300">
    <p>{{ t('settings.mcpPermFirstBody') }}</p>
    <ul class="flex flex-col gap-1.5 rounded-lg bg-zinc-50 p-3 dark:bg-zinc-900/60">
      <li v-for="perm in mcpPerms" :key="perm.level" class="flex items-start gap-1.5">
        <Icon
          name="dot"
          class="mt-1 h-3 w-3 shrink-0"
          :class="perm.highRisk ? 'text-amber-500' : 'text-purple-500'"
        />
        <span>
          <span class="font-medium">{{ permDesc(perm.level) }}</span>
          <span class="text-zinc-500 dark:text-zinc-400">
            — {{ perm.highRisk ? t('settings.mcpPermFirstHighRisk') : t('settings.mcpPermFirstDefault') }}
          </span>
        </span>
      </li>
    </ul>
  </div>
  <template #footer>
    <button
      type="button"
      class="miyo-btn"
      @click="permExplainOpen = false"
    >{{ t('common.cancel') }}</button>
    <button
      type="button"
      class="miyo-btn miyo-btn-primary"
      @click="confirmPermExplain"
    >{{ t('settings.mcpPermFirstConfirm') }}</button>
  </template>
</Modal>

<!-- 高风险等级启用确认 -->
<Modal v-model:open="pendingHighRiskConfirm" :title="t('settings.mcpPermRiskTitle')" width="440px">
  <div class="flex flex-col gap-3 text-xs leading-relaxed text-zinc-700 dark:text-zinc-300">
    <p>{{ t('settings.mcpPermRiskBody', { level: pendingHighRiskLabel }) }}</p>
    <p class="rounded-lg bg-amber-50 p-3 text-amber-700 dark:bg-amber-500/[0.08] dark:text-amber-400">
      {{ t('settings.mcpPermRiskWarn') }}
    </p>
  </div>
  <template #footer>
    <button
      type="button"
      class="miyo-btn"
      @click="pendingHighRiskConfirm = false"
    >{{ t('common.cancel') }}</button>
    <button
      type="button"
      class="rounded-lg bg-amber-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-amber-600"
      @click="confirmHighRisk"
    >{{ t('settings.mcpPermRiskConfirm') }}</button>
  </template>
</Modal>
</template>