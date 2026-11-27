<script setup lang="ts">
/**
 * BatchRunDialog：批量执行配置（套件全量 / 勾选场景 / 文件夹三个入口共用）。
 *
 * 只负责收集运行配置并交给 testSuites store；真正的调度在 Rust 批量引擎。
 * 点击开始后立即关闭弹窗（执行在途，进度由 BatchReportPanel 展示），
 * 启动失败才提示错误——避免弹窗被长任务占住。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { useWorkspaceStore } from '@/stores/workspace'
import { useIntegrationStore } from '@/domains/integration/stores/integration'
import {
  DEFAULT_BATCH_OPTIONS,
  useTestSuitesStore,
  type BatchRunOptions,
} from '@/domains/integration/stores/testSuites'
import { useToast } from '@/composables/useToast'
import { errorText } from '@/utils/errorText'
import Modal from '@/components/ui/Modal.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import CustomNumberInput from '@/components/ui/CustomNumberInput.vue'
import Switch from '@/components/ui/Switch.vue'

const t = useLocaleStore().t
const workspace = useWorkspaceStore()
const integration = useIntegrationStore()
const store = useTestSuitesStore()
const toast = useToast()

const draft = ref<BatchRunOptions>({ ...DEFAULT_BATCH_OPTIONS })

watch(
  () => store.runDialogOpen,
  (open) => {
    if (open) draft.value = { ...store.runOptions }
  },
)

const modeOptions = computed(() => [
  { value: 'serial', label: t('testSuite.modeSerial') },
  { value: 'parallel', label: t('testSuite.modeParallel') },
])

const policyOptions = computed(() => [
  { value: 'continue', label: t('testSuite.policyContinue') },
  { value: 'stop', label: t('testSuite.policyStop') },
  { value: 'retry', label: t('testSuite.policyRetry') },
  { value: 'skip_dependents', label: t('testSuite.policySkipDependents') },
])

/** 目标摘要文案（按预设类型）。 */
const targetLabel = computed(() => {
  const preset = store.runPreset
  if (!preset) return ''
  if (preset.suite_id) {
    const suite = store.suites.find((s) => s.id === preset.suite_id)
    return t('testSuite.runTargetSuite', { name: suite?.name ?? '' })
  }
  if (preset.folder_id) {
    const folder = integration.folders.find((f) => f.id === preset.folder_id)
    return t('testSuite.runTargetFolder', { name: folder?.name ?? '' })
  }
  return t('testSuite.runTargetScenarios', { n: preset.scenario_ids?.length ?? 0 })
})

/** 执行环境：跟随工作区当前激活环境（切换环境 = 新建批次，不污染历史）。 */
const environmentLabel = computed(() => {
  const env = workspace.environments.find((e) => e.id === workspace.activeEnvId)
  return env?.name ?? t('testSuite.envActive')
})

function close(): void {
  store.closeRunDialog()
}

function start(): void {
  const preset = store.runPreset
  if (!preset) return
  void store
    .startBatch(preset, draft.value)
    .catch((err) => toast.error(t('testSuite.startFail'), { message: errorText(err) }))
  close()
}
</script>

<template>
  <Modal
    :open="store.runDialogOpen"
    :title="t('testSuite.runTitle')"
    width="500px"
    @update:open="(v: boolean) => !v && close()"
  >
    <div class="bsd-body">
      <div class="bsd-target">
        <span class="bsd-target-label">{{ targetLabel }}</span>
        <span class="bsd-target-env">{{ t('testSuite.environment') }}：{{ environmentLabel }}</span>
      </div>

      <div class="bsd-row">
        <span class="bsd-label">{{ t('testSuite.mode') }}</span>
        <SegmentedControl
          :model-value="draft.mode"
          :options="modeOptions"
          size="sm"
          @update:model-value="(v: string) => (draft.mode = v as BatchRunOptions['mode'])"
        />
      </div>

      <div v-if="draft.mode === 'parallel'" class="bsd-row">
        <span class="bsd-label">{{ t('testSuite.concurrency') }}</span>
        <CustomNumberInput
          :model-value="draft.concurrency"
          :min="1"
          :max="50"
          size="sm"
          @update:model-value="(v: string | number) => (draft.concurrency = Number(v) || 1)"
        />
      </div>

      <div class="bsd-row">
        <span class="bsd-label">{{ t('testSuite.failurePolicy') }}</span>
        <CustomSelect
          :model-value="draft.failure_policy"
          :options="policyOptions"
          size="sm"
          @update:model-value="
            (v: string | number) => (draft.failure_policy = String(v) as BatchRunOptions['failure_policy'])
          "
        />
      </div>

      <div v-if="draft.failure_policy === 'retry'" class="bsd-row">
        <span class="bsd-label">{{ t('testSuite.retryCount') }}</span>
        <CustomNumberInput
          :model-value="draft.retry_count"
          :min="0"
          :max="10"
          size="sm"
          @update:model-value="(v: string | number) => (draft.retry_count = Number(v) || 0)"
        />
      </div>

      <div class="bsd-row">
        <span class="bsd-label">{{ t('testSuite.skipDisabled') }}</span>
        <Switch
          :model-value="draft.skip_disabled"
          size="sm"
          :label="t('testSuite.skipDisabled')"
          @update:model-value="(v: boolean) => (draft.skip_disabled = v)"
        />
      </div>

      <p class="bsd-tip">{{ t('testSuite.runTip') }}</p>
    </div>

    <template #footer>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="close">
        {{ t('testSuite.cancelAction') }}
      </button>
      <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="start">
        {{ t('testSuite.start') }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.bsd-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.bsd-target {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
}

.bsd-target-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}

.bsd-target-env {
  font-size: 12px;
  color: var(--text-3);
}

.bsd-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.bsd-label {
  font-size: 12px;
  color: var(--text-3);
}

.bsd-tip {
  margin: 0;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-3);
}
</style>