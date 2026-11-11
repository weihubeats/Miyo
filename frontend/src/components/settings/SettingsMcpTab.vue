<script setup lang="ts">
/**
 * SettingsMcpTab：MCP 服务页签（自 SettingsDialog 拆出，独立加载）。
 *
 * 自包含：启停开关 / 监听端口 / 运行状态 / 工具权限（read·write 分组逐个
 * 启停）。挂载时自行 loadMcpStatus（父级不再预取）；启停与端口保存后回读
 * 状态，失败路径同样回读（状态与后端事实对齐）。
 */
import { computed, onMounted, ref } from 'vue'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { useLocaleStore } from '../../stores/locale'
import type { McpToolStatus } from '../../types/miyoApi'
import CustomNumberInput from '../ui/CustomNumberInput.vue'

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

onMounted(() => {
  void loadMcpStatus()
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
</template>