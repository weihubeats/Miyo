<script setup lang="ts">
/**
 * JavaSaveTestCaseDialog：把当前 Java 方法调用保存为集成测试场景步骤。
 *
 * 保存内容：java_invocation 步骤（java_project_id / method_id / arguments /
 * timeout_ms）——Runtime 连接状态与凭证不落步骤（重放时按项目配置现连）。
 * 目标场景：当前 Java 项目关联的 Miyo 项目下的场景列表（可选既有场景追加
 * 或新建场景）。断言 / 提取规则在集成测试里继续编辑。
 */
import { computed, reactive, ref, watch } from 'vue'
import Modal from '@/components/ui/Modal.vue'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useJavaDebugStore } from '@/domains/java-debug/stores/javaDebug'
import { useDashboardStore } from '@/domains/workspace/stores/dashboard'
import { useLocaleStore } from '@/stores/locale'
import type { Project, Scenario } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: [] }>()

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const store = useJavaDebugStore()
const dashboard = useDashboardStore()

const state = reactive({
  projectId: '',
  scenarioId: '', // '' = 新建
  newScenarioName: '',
  stepName: '',
  loading: false,
  saving: false,
})

const scenarios = ref<Scenario[]>([])

/** 候选 Miyo 项目（当前 Java 项目有链接时锁定）。 */
const candidateProjects = computed<Project[]>(() => dashboard.projects ?? [])
const linkedProjectId = computed(() => store.activeProject?.project_id ?? '')
const projectLocked = computed(() => !!linkedProjectId.value)

watch(
  () => props.open,
  async (open) => {
    if (!open) return
    if (!store.activeMethod) {
      toast.error(t('java.saveCaseTitle'))
      emit('close')
      return
    }
    state.scenarioId = ''
    state.stepName = ''
    state.newScenarioName = ''
    // dashboard 项目缓存为空时兜底拉取。
    if (dashboard.projects === null) {
      try {
        dashboard.setProjects(await api.getProjects())
      } catch {
        // 列表拉取失败：候选为空，界面提示先关联项目。
      }
    }
    state.projectId = linkedProjectId.value || dashboard.projects?.[0]?.id || ''
    if (state.projectId) void loadScenarios(state.projectId)
  },
)

watch(
  () => state.projectId,
  (next, prev) => {
    if (next && next !== prev) {
      scenarios.value = []
      state.scenarioId = ''
      void loadScenarios(next)
    }
  },
)

async function loadScenarios(projectId: string): Promise<void> {
  state.loading = true
  try {
    scenarios.value = await api.listScenarios(projectId)
  } catch {
    scenarios.value = []
  } finally {
    state.loading = false
  }
}

/** 默认步骤名：类简名#方法名。 */
const defaultStepName = computed(() => {
  const method = store.activeMethod
  if (!method) return ''
  return `${method.class_name.split('.').pop()}#${method.method_name}`
})

const canSave = computed(() => {
  if (!store.activeMethod) return false
  if (!state.projectId) return false
  // 既有场景必须选中；新建必须填名。
  if (state.scenarioId) return true
  return state.newScenarioName.trim() !== ''
})

async function save(): Promise<void> {
  const method = store.activeMethod
  if (!method || !canSave.value || state.saving) return
  state.saving = true
  try {
    // 场景：追加到既有场景，或新建。
    let scenario: Scenario | undefined
    if (state.scenarioId) {
      scenario = scenarios.value.find((s) => s.id === state.scenarioId)
    }
    const step = {
      endpoint_id: null,
      enabled: true,
      extract: [],
      assertions: [],
      name: state.stepName.trim() || defaultStepName.value,
      java_invocation: {
        java_project_id: store.activeProjectId ?? '',
        class_name: method.class_name,
        method_id: method.method_id,
        arguments: safeParseArgs(),
        timeout_ms: store.timeoutOverride ?? null,
      },
    }
    let saved: Scenario
    if (scenario) {
      saved = await api.saveScenario({
        ...scenario,
        steps: [...scenario.steps, step],
      })
    } else {
      saved = await api.saveScenario({
        id: crypto.randomUUID(),
        project_id: state.projectId,
        name: state.newScenarioName.trim(),
        description: '',
        category: '',
        sort_order: 0,
        steps: [step],
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      })
    }
    toast.success(t('java.saveCaseSaved', { name: saved.name }))
    emit('close')
  } catch (e) {
    toast.error(errorText(e))
  } finally {
    state.saving = false
  }
}

function safeParseArgs(): unknown {
  try {
    return JSON.parse(store.argsJson || '[]')
  } catch {
    return []
  }
}

function openScenes(): void {
  void import('@/router').then(({ default: router }) => router.push('/integration'))
}
</script>

<template>
  <Modal :open="props.open" :title="t('java.saveCaseTitle')" width="520px" @close="emit('close')">
    <div class="jsc">
      <label class="jsc-field">
        <span class="jsc-label">{{ t('java.saveCaseProject') }}</span>
        <select v-model="state.projectId" class="miyo-input" :disabled="projectLocked" data-testid="java-case-project">
          <option v-for="p in candidateProjects" :key="p.id" :value="p.id">{{ p.name }}</option>
        </select>
        <span class="jsc-hint">{{ t('java.saveCaseProjectHint') }}</span>
      </label>

      <template v-if="state.projectId">
        <label class="jsc-field">
          <span class="jsc-label">{{ t('java.saveCaseScene') }}</span>
          <span v-if="state.loading" class="jsc-hint">{{ t('java.saveCaseLoading') }}</span>
          <select v-else v-model="state.scenarioId" class="miyo-input" data-testid="java-case-scenario">
            <option value="">{{ t('java.saveCaseSceneNew') }}</option>
            <option v-for="s in scenarios" :key="s.id" :value="s.id">{{ s.name }}</option>
          </select>
          <span v-if="scenarios.length === 0 && !state.loading" class="jsc-hint">
            {{ t('java.saveCaseNoScenes') }}
          </span>
        </label>

        <label v-if="!state.scenarioId" class="jsc-field">
          <span class="jsc-label">{{ t('java.saveCaseSceneName') }}</span>
          <input
            v-model="state.newScenarioName"
            class="miyo-input"
            :placeholder="t('java.saveCaseSceneNamePlaceholder')"
            data-testid="java-case-scene-name"
          />
        </label>

        <label class="jsc-field">
          <span class="jsc-label">{{ t('java.saveCaseStepName') }}</span>
          <input
            v-model="state.stepName"
            class="miyo-input"
            :placeholder="defaultStepName"
            data-testid="java-case-step-name"
          />
        </label>

        <p class="jsc-hint">{{ t('java.saveCaseAssertHint') }}</p>
      </template>

      <div class="jsc-actions">
        <button type="button" class="miyo-btn" @click="openScenes">
          {{ t('java.saveCaseGoScenes') }}
        </button>
        <span class="jsc-spacer" />
        <button type="button" class="miyo-btn" @click="emit('close')">
          {{ t('common.cancel') }}
        </button>
        <button
          type="button"
          class="miyo-btn miyo-btn-primary"
          :disabled="!canSave || state.saving"
          data-testid="java-case-save"
          @click="save"
        >
          {{ t('common.save') }}
        </button>
      </div>
    </div>
  </Modal>
</template>

<style scoped>
.jsc {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.jsc-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.jsc-label {
  font-size: 12px;
  color: var(--text-secondary);
}
.jsc-hint {
  font-size: 11px;
  color: var(--text-tertiary, var(--text-secondary));
  line-height: 1.5;
}
.jsc-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
}
.jsc-spacer {
  flex: 1;
}
</style>
