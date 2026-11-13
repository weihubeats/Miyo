<script setup lang="ts">
/**
 * RunDock：AI 工作区的场景执行坞（复用 useScenarioRun 全链路）。
 *
 * store 置 runScenario + 自增 runToken → 本组件 watch 后自动运行；
 * 运行控制条 + RunResultView（逐步请求 / 响应 / 变量 / 断言）+ 失败一键分析。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useAiTestStore } from '@/domains/agent/stores/aiTest'
import { useScenarioRun } from '@/domains/integration/composables/useScenarioRun'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { buildRunStepsFromSteps } from '@/utils/runStepBuild'
import { computeVarChain, type VarChainEntry, type VarScanStep } from '@/utils/scenarioVars'
import { mergedVariableMap } from '@/utils/environment'
import RunResultView from '@/domains/integration/components/integration/RunResultView.vue'
import IconButton from '@/components/ui/IconButton.vue'
import type { Scenario } from '@/types/miyoApi.d'

const props = defineProps<{
  scenario: Scenario
  /** 运行令牌：store 自增后自动运行（避免跨组件直调）。 */
  runToken: number
}>()

const t = useLocaleStore().t
const ai = useAiTestStore()
const api = useMiyoApi()
const toast = useToast()
const workspace = ai.workspace

const runner = useScenarioRun({
  store: workspace,
  api,
  toast,
  t,
  scenario: computed(() => ({ id: props.scenario.id, name: props.scenario.name })),
  scenarioId: props.scenario.id,
  buildRunSteps: () => buildRunStepsFromSteps(props.scenario.steps ?? [], workspace.endpoints),
  dirty: computed(() => false),
  save: async () => true,
  openMenuAt: () => {},
  lifecycleSetup: () => buildRunStepsFromSteps(props.scenario.setup ?? [], workspace.endpoints),
  lifecycleTeardown: () => buildRunStepsFromSteps(props.scenario.teardown ?? [], workspace.endpoints),
  lifecycleTeardownEnabled: () => props.scenario.teardown_enabled ?? true,
})

watch(
  () => props.runToken,
  (n) => {
    if (n > 0) void runner.run()
  },
  { immediate: true },
)

/** 修复入口：存在失败步且场景未在跑时出现。 */
const hasFailure = computed(() => (runner.runResult.value?.results ?? []).some((r) => {
  const status = (r as { step_status?: string }).step_status
  return r.ok === false || status === 'failed' || status === 'blocked'
}))
const analyzing = ref(false)
async function analyzeFailure(): Promise<void> {
  if (!runner.runId.value) return
  analyzing.value = true
  try {
    await ai.analyzeRun(runner.runId.value)
  } finally {
    analyzing.value = false
  }
}

/** 模板用解构（setup 顶层 ref 自动解包；script 内仍走 runner.*）。 */
const { running, runId, progress, runResult, currentRunAt, currentRunStartedAt, progressPercent, fullBodyLoader } =
  runner
const runnerRun = () => runner.run()
const runnerStop = () => runner.stop()

/** RunResultView 的变量链（步骤真实配置 → computeVarChain，与编辑器同源）。 */
const varChain = computed<VarChainEntry[]>(() => {
  const scan: VarScanStep[] = (props.scenario.steps ?? []).map((s, i) => {
    const texts: string[] = []
    if (s.request) {
      texts.push(s.request.url, JSON.stringify(s.request.spec ?? {}))
    } else if (s.endpoint_id) {
      const ep = workspace.endpoints.find((e) => e.id === s.endpoint_id)
      if (ep) texts.push(ep.path, JSON.stringify(ep.request))
    }
    if (s.overrides) texts.push(JSON.stringify(s.overrides))
    if (s.condition) texts.push(JSON.stringify(s.condition))
    const extracts = (Array.isArray(s.extract) ? s.extract : [])
      .map((r) => r as { name?: string; path?: string; from?: string })
      .filter((r) => r.name?.trim() && r.path?.trim())
      .map((r) => ({
        name: r.name!.trim(),
        from: (r.from ?? 'body') as 'body' | 'header' | 'cookie' | 'body_regex',
        path: r.path!.trim(),
      }))
    return {
      index: i,
      name: s.name || `#${i + 1}`,
      extracts,
      referenceTexts: texts,
    }
  })
  return computeVarChain(
    scan,
    Object.keys(
      mergedVariableMap({
        globalVariables: workspace.globalVariables,
        projectVariables: workspace.project?.variables ?? null,
        env: workspace.environments.find((e) => e.id === workspace.activeEnvId),
      }),
    ),
  )
})
</script>

<template>
  <section class="run-dock">
    <header class="dock-head">
      <h3 class="dock-title">{{ t('ai.runDock.title') }} · {{ scenario.name }}</h3>
      <div class="dock-actions">
        <IconButton
          v-if="!running"
          name="play"
          :title="t('ai.runScenario')"
          @click="runnerRun()"
        />
        <IconButton
          v-else
          name="stop"
          :title="t('ai.stop')"
          tone="danger"
          @click="runnerStop()"
        />
        <button
          v-if="hasFailure && !running"
          class="miyo-btn miyo-btn-sm"
          :disabled="analyzing"
          @click="analyzeFailure"
        >
          {{ t('ai.runDock.analyzeFail') }}
        </button>
      </div>
    </header>
    <div v-if="running" class="dock-progress">
      <div class="miyo-progress-track">
        <div class="miyo-progress-fill" :style="{ width: `${progressPercent}%` }" />
      </div>
      <span class="dock-progress-text">{{ progress.done }} / {{ progress.total }}</span>
    </div>
    <RunResultView
      :scenario-name="scenario.name"
      :run-id="runId"
      :running="running"
      :started-at="currentRunStartedAt"
      :finished-at="currentRunAt"
      :result="runResult"
      :var-chain="varChain"
      :full-body-loader="fullBodyLoader"
      @rerun="runnerRun()"
    />
  </section>
</template>

<style scoped>
.run-dock {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}
.dock-head {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
  padding: var(--sp-2) 0;
}
.dock-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dock-actions {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex: none;
}
.dock-progress {
  flex: none;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding-bottom: var(--sp-2);
}
.miyo-progress-track {
  flex: 1;
  height: 4px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  overflow: hidden;
}
.miyo-progress-fill {
  height: 100%;
  background: var(--grad-primary);
  transition: width 0.2s ease;
}
.dock-progress-text {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
</style>
