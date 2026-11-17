<script setup lang="ts">
/**
 * JavaProjectDialog：Java 调试项目注册 / 编辑弹窗。
 *
 * 字段：名称、Runtime 地址、token（可空）、关联 Miyo 项目 / 环境（可选）、
 * 超时 / 返回值上限 / 并发上限。保存前形状校验在前端做一层（scheme / 非空），
 * 后端 save_java_project 是权威校验。
 *
 * token 三态语义（与后端对齐）：编辑时留空 = 保持现有 token（payload 缺省）；
 * 点「清除已存 token」= 显式清空（payload 空串）；输入新值 = 更新。
 * 「测试连接」走 java_probe_endpoint（地址 + token 直连握手，不落库）。
 */
import { computed, reactive, ref, watch } from 'vue'
import Modal from '@/components/ui/Modal.vue'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import type { Environment } from '@/types/miyoApi.d'
import { useDashboardStore } from '@/domains/workspace/stores/dashboard'
import type { JavaDebugProject } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const props = defineProps<{
  /** 传项目 = 编辑；null = 新建。 */
  editing: JavaDebugProject | null
  open: boolean
  /** 新建预填（来自接入向导的本地候选：端点 / 应用名）。 */
  preset?: { name?: string; endpoint?: string } | null
}>()

const emit = defineEmits<{
  close: []
  saved: [project: JavaDebugProject]
}>()

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t
const dashboard = useDashboardStore()

const saving = ref(false)
const testing = ref(false)
/** 编辑态是否点了「清除已存 token」（payload 用空串显式清除）。 */
const tokenCleared = ref(false)
/** token 留空时是否代表「保持现有」（仅编辑态）。 */
const keepExistingToken = computed(() => !!props.editing && form.authToken.trim() === '' && !tokenCleared.value)

const form = reactive({
  name: '',
  runtimeEndpoint: 'http://127.0.0.1:9918',
  authToken: '',
  projectId: '',
  environmentId: '',
  timeoutMs: 30_000,
  maxResultBytes: 1_048_576,
  maxConcurrent: 4,
  description: '',
})

watch(
  () => [props.open, props.editing, props.preset] as const,
  ([open, editing, preset]) => {
    if (!open) return
    // 关联项目下拉的数据源：dashboard 缓存为空（未走过 /projects）时兜底拉取。
    if (dashboard.projects === null) {
      void api.getProjects().then((list) => dashboard.setProjects(list)).catch(() => {})
    }
    form.name = editing?.name ?? preset?.name ?? ''
    form.runtimeEndpoint = editing?.runtime_endpoint ?? preset?.endpoint ?? 'http://127.0.0.1:9918'
    // 后端列表 / 保存返回已脱敏（auth_token 恒 null）：编辑态 token 一律从空开始。
    form.authToken = ''
    tokenCleared.value = false
    form.projectId = editing?.project_id ?? ''
    form.environmentId = editing?.environment_id ?? ''
    form.timeoutMs = editing?.timeout_ms ?? 30_000
    form.maxResultBytes = editing?.max_result_bytes ?? 1_048_576
    form.maxConcurrent = editing?.max_concurrent ?? 4
    form.description = editing?.description ?? ''
  },
  { immediate: true },
)

const nameInvalid = computed(() => form.name.trim() === '')
const endpointInvalid = computed(() => {
  const lower = form.runtimeEndpoint.trim().toLowerCase()
  return !lower.startsWith('http://') && !lower.startsWith('https://')
})

/** 关联项目的环境列表（选中项目后按需拉取）。 */
const relatedEnvironments = ref<Environment[]>([])
watch(
  () => form.projectId,
  async (next) => {
    relatedEnvironments.value = []
    form.environmentId = ''
    if (!next) return
    try {
      relatedEnvironments.value = await api.listEnvironments(next)
    } catch {
      // 拉取失败不阻塞表单（环境仅是可选关联）。
    }
  },
)

// 切换关联项目时环境联动清空（跨项目环境无意义）。
watch(
  () => form.projectId,
  (next, prev) => {
    if (next !== prev) form.environmentId = ''
  },
)

async function save(): Promise<void> {
  if (nameInvalid.value || endpointInvalid.value) return
  saving.value = true
  try {
    const now = new Date().toISOString()
    // token 三态：显式清除 = 空串；留空编辑 = 缺省（后端保留原值）；新值 = 更新。
    const tokenPayload: string | undefined = tokenCleared.value
      ? ''
      : form.authToken.trim() || undefined
    const project: JavaDebugProject = {
      id: props.editing?.id ?? crypto.randomUUID(),
      project_id: form.projectId || null,
      name: form.name.trim(),
      runtime_endpoint: form.runtimeEndpoint.trim(),
      environment_id: form.environmentId || null,
      auth_token: tokenPayload ?? null,
      timeout_ms: Math.max(1_000, Math.min(300_000, form.timeoutMs)),
      max_result_bytes: Math.max(1_024, form.maxResultBytes),
      max_concurrent: Math.max(1, Math.min(64, form.maxConcurrent)),
      description: form.description.trim(),
      last_connected_at: props.editing?.last_connected_at ?? null,
      last_info: props.editing?.last_info ?? null,
      created_at: props.editing?.created_at ?? now,
      updated_at: now,
    }
    const saved = await api.saveJavaProject(project)
    toast.success(t('java.projectSaved', { name: saved.name }))
    emit('saved', saved)
    emit('close')
  } catch (e) {
    toast.error(t('java.projectSaveFailed', { message: errorText(e) }))
  } finally {
    saving.value = false
  }
}

/** 直连探测（不落库）：地址 + 当前输入的 token 握手。 */
async function testConnection(): Promise<void> {
  if (endpointInvalid.value) return
  testing.value = true
  try {
    const info = await api.javaProbeEndpoint(
      form.runtimeEndpoint.trim(),
      form.authToken.trim() || undefined,
    )
    toast.success(
      t('java.testOk', { app: info.app_name || form.name, version: info.java_version }),
    )
  } catch (e) {
    toast.error(t('java.testFailed', { message: errorText(e) }))
  } finally {
    testing.value = false
  }
}
</script>

<template>
  <Modal :open="open" :title="editing ? t('java.projectEditTitle') : t('java.projectNewTitle')" width="560px" @close="emit('close')">
    <form class="jp-form" @submit.prevent="save">
      <label class="jp-field">
        <span class="jp-label">{{ t('java.projectName') }}</span>
        <input
          v-model="form.name"
          class="miyo-input"
          :placeholder="t('java.projectNamePlaceholder')"
          data-testid="java-project-name"
        />
      </label>
      <label class="jp-field">
        <span class="jp-label">{{ t('java.runtimeEndpoint') }}</span>
        <input
          v-model="form.runtimeEndpoint"
          class="miyo-input"
          :placeholder="t('java.runtimeEndpointPlaceholder')"
          :invalid="endpointInvalid || undefined"
          data-testid="java-project-endpoint"
        />
        <span v-if="endpointInvalid" class="jp-hint jp-hint-err">{{ t('java.runtimeEndpointInvalid') }}</span>
        <span class="jp-hint">{{ t('java.endpointHint') }}</span>
      </label>
      <label class="jp-field">
        <span class="jp-label">{{ t('java.authToken') }}</span>
        <div class="jp-token-row">
          <input
            v-model="form.authToken"
            class="miyo-input"
            type="password"
            :placeholder="editing ? t('java.authTokenKeepHint') : t('java.authTokenPlaceholder')"
            autocomplete="off"
            data-testid="java-project-token"
            @input="tokenCleared = false"
          />
          <button
            v-if="editing && !tokenCleared"
            type="button"
            class="miyo-btn jp-token-clear"
            :title="t('java.authTokenClear')"
            data-testid="java-token-clear"
            @click="tokenCleared = true; form.authToken = ''"
          >
            {{ t('java.authTokenClear') }}
          </button>
        </div>
        <span v-if="tokenCleared" class="jp-hint jp-hint-warn">{{ t('java.authTokenCleared') }}</span>
        <span v-else class="jp-hint">{{ keepExistingToken ? t('java.authTokenKeepHint') : t('java.authTokenHint') }}</span>
      </label>
      <div class="jp-row">
        <label class="jp-field">
          <span class="jp-label">{{ t('java.linkedProject') }}</span>
          <select v-model="form.projectId" class="miyo-input" data-testid="java-project-linked">
            <option value="">{{ t('java.linkedNone') }}</option>
            <option v-for="p in dashboard.projects ?? []" :key="p.id" :value="p.id">{{ p.name }}</option>
          </select>
        </label>
        <label class="jp-field">
          <span class="jp-label">{{ t('java.linkedEnvironment') }}</span>
          <select v-model="form.environmentId" class="miyo-input" :disabled="!form.projectId">
            <option value="">{{ t('java.linkedNone') }}</option>
            <option v-for="e in relatedEnvironments" :key="e.id" :value="e.id">{{ e.name }}</option>
          </select>
        </label>
      </div>
      <div class="jp-row">
        <label class="jp-field">
          <span class="jp-label">{{ t('java.timeoutMs') }}</span>
          <input v-model.number="form.timeoutMs" class="miyo-input" type="number" min="1000" max="300000" step="1000" />
        </label>
        <label class="jp-field">
          <span class="jp-label">{{ t('java.maxResultKb') }}</span>
          <input v-model.number="form.maxResultBytes" class="miyo-input" type="number" min="1024" step="1024" />
        </label>
        <label class="jp-field">
          <span class="jp-label">{{ t('java.maxConcurrent') }}</span>
          <input v-model.number="form.maxConcurrent" class="miyo-input" type="number" min="1" max="64" />
        </label>
      </div>
      <label class="jp-field">
        <span class="jp-label">{{ t('java.projectDescription') }}</span>
        <input v-model="form.description" class="miyo-input" :placeholder="t('java.projectDescriptionPlaceholder')" />
      </label>
      <p class="jp-hint">{{ t('java.securityNotice') }}</p>
      <div class="jp-actions">
        <button
          type="button"
          class="miyo-btn"
          :disabled="testing || endpointInvalid"
          data-testid="java-project-test"
          @click="testConnection"
        >
          {{ testing ? t('java.testing') : t('java.testConnection') }}
        </button>
        <button type="button" class="miyo-btn" @click="emit('close')">{{ t('common.cancel') }}</button>
        <button
          type="submit"
          class="miyo-btn miyo-btn-primary"
          :disabled="saving || nameInvalid || endpointInvalid"
          data-testid="java-project-save"
        >
          {{ t('common.save') }}
        </button>
      </div>
    </form>
  </Modal>
</template>

<style scoped>
.jp-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.jp-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.jp-label {
  font-size: 12px;
  color: var(--text-secondary);
}
.jp-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.jp-row .jp-field:has(input[type='number']) {
  min-width: 0;
}
.jp-hint {
  font-size: 11px;
  color: var(--text-tertiary, var(--text-secondary));
}
.jp-hint-err {
  color: var(--danger, #ef4444);
}
.jp-hint-warn {
  color: var(--warning, #f59e0b);
}
.jp-token-row {
  display: flex;
  gap: 8px;
  align-items: center;
}
.jp-token-row .miyo-input {
  flex: 1;
  min-width: 0;
}
.jp-token-clear {
  flex-shrink: 0;
  font-size: 11px;
  padding: 4px 9px;
}
.jp-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}
.jp-actions .miyo-btn:first-child {
  margin-right: auto;
}
</style>
