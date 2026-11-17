<script setup lang="ts">
/**
 * JavaMethodPanel：中间列——方法选择 + 参数编辑 + 执行。
 *
 * 方法列表（完整签名 + 参数类型提示 + 重载徽标）；参数双模式编辑：
 * 可视化表单（按形参类型生成输入框）/ JSON 原始编辑器（JsonCodeMirror），
 * 可互切（表单 → JSON 序列化、JSON → 表单解析，解析失败提示保留原文）。
 * 复杂类型参数（集合 / Map / DTO / 数组）在表单模式提供参数级 JSON 编辑
 * 弹层。执行前做基本校验（JSON 合法 + 必填基本类型非空）；方法名命中
 * 危险关键词（delete / clear / drop 等）时二次确认。
 */
import { computed, reactive, ref, watch } from 'vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import JsonCodeMirror from '@/components/ui/JsonCodeMirror.vue'
import Modal from '@/components/ui/Modal.vue'
import SegmentedControl, { type SegmentOption } from '@/components/ui/SegmentedControl.vue'
import Spinner from '@/components/ui/Spinner.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaMethodInfo, JavaParamInfo } from '@/types/miyoApi.d'

const props = defineProps<{
  className: string | null
  methods: JavaMethodInfo[]
  loading: boolean
  activeMethodId: string | null
  argsJson: string
  editMode: 'form' | 'json'
  invoking: boolean
  hasConnection: boolean
  hasTemplate: boolean
  /** attach 模式扩展：实例解析策略 / 风险分级（v1 runtime 模式不传）。 */
  attachMode?: boolean
  agentRisk?: string | null
  agentRiskReason?: string | null
  canInstantiate?: boolean
  isProxy?: boolean
}>()

const emit = defineEmits<{
  selectMethod: [method: JavaMethodInfo]
  'update:argsJson': [value: string]
  'update:editMode': [value: 'form' | 'json']
  'update:timeout': [value: number | null]
  'update:instanceKind': [value: 'auto' | 'static' | 'bean' | 'new']
  'update:newConsent': [value: boolean]
  run: []
  cancel: []
  saveTemplate: []
  useTemplate: []
}>()
const timeoutValue = ref<number | null>(null)

/** attach 实例解析选项（实例方法才有意义；static 方法固定 static）。 */
const instanceOptions = computed<SegmentOption[]>(() => [
  { value: 'auto', label: t('java2.instanceAuto') },
  { value: 'static', label: t('java2.instanceStatic') },
  { value: 'bean', label: t('java2.instanceBean') },
  { value: 'new', label: t('java2.instanceNew') },
])

// attach 实例策略：外部受控值（store） + new 前置确认
const instanceKindValue = defineModel<'auto' | 'static' | 'bean' | 'new'>('instanceKind', {
  default: 'auto',
})
const newConsent = defineModel<boolean>('newConsent', { default: false })

function onInstanceKindChange(value: string | number | null): void {
  const kind = String(value) as 'auto' | 'static' | 'bean' | 'new'
  instanceKindValue.value = kind
  if (kind !== 'new') {
    newConsent.value = false
  }
}

function onTimeoutChange(event: Event): void {
  const raw = (event.target as HTMLInputElement).value.trim()
  const parsed = raw === '' ? null : Number.parseInt(raw, 10)
  timeoutValue.value = parsed !== null && Number.isFinite(parsed) && parsed > 0 ? parsed : null
  emit('update:timeout', timeoutValue.value)
}

const locale = useLocaleStore()
const t = locale.t

const activeMethod = computed(
  () => props.methods.find((m) => m.method_id === props.activeMethodId) ?? null,
)

const modeOptions = computed<SegmentOption[]>(() => [
  { value: 'form', label: t('java.modeForm') },
  { value: 'json', label: t('java.modeJson') },
])

/** JSON 解析状态（模式切换 / 校验共用）。 */
const parseError = ref('')
const formValues = ref<unknown[]>([])

/** argsJson → 表单值（JSON 模式切回表单时同步）。 */
watch(
  () => [props.argsJson, props.editMode] as const,
  ([json, mode]) => {
    if (mode !== 'form') return
    try {
      const parsed = JSON.parse(json || '[]') as unknown[]
      if (Array.isArray(parsed)) {
        formValues.value = parsed
        parseError.value = ''
      }
    } catch (e) {
      parseError.value = String(e instanceof Error ? e.message : e)
    }
  },
  { immediate: true },
)

/** 表单值 → argsJson（输入即回写）。 */
function onFormInput(): void {
  try {
    emit('update:argsJson', JSON.stringify(formValues.value, null, 2))
    parseError.value = ''
  } catch {
    // formValues 恒为数组，不触发。
  }
}

function setMode(mode: string): void {
  const next = mode as 'form' | 'json'
  if (next === 'json' && props.editMode === 'form') {
    emit('update:argsJson', JSON.stringify(formValues.value, null, 2))
  }
  emit('update:editMode', next)
}

const parsedArgs = computed<unknown[]>(() => {
  try {
    const value = JSON.parse(props.argsJson || '[]') as unknown
    return Array.isArray(value) ? value : []
  } catch {
    return []
  }
})

/** 参数形状校验：个数 + JSON 合法（类型细节由 Runtime 终检并回传错误）。 */
const validationError = computed(() => {
  if (parseError.value) return t('java.argsJsonInvalid', { message: parseError.value })
  try {
    JSON.parse(props.argsJson || '[]')
  } catch (e) {
    return t('java.argsJsonInvalid', { message: String(e instanceof Error ? e.message : e) })
  }
  if (activeMethod.value && parsedArgs.value.length !== activeMethod.value.params.length) {
    return t('java.argsCountMismatch', {
      expected: activeMethod.value.params.length,
      actual: parsedArgs.value.length,
    })
  }
  return ''
})

const runnable = computed(
  () =>
    !!activeMethod.value &&
    !props.invoking &&
    !validationError.value &&
    props.hasConnection,
)

/** 表单字段占位（按类型提示）。 */
function fieldPlaceholder(typeName: string, genericType: string): string {
  if (genericType.startsWith('List<') || genericType.endsWith('[]')) return '[ ... ]'
  if (typeName === 'boolean' || typeName === 'Boolean') return 'true / false'
  if (
    ['long', 'Long', 'int', 'Integer', 'double', 'Double', 'float', 'Float', 'BigDecimal'].includes(
      typeName,
    )
  ) {
    return '0'
  }
  return typeName
}

function inputType(typeName: string): string {
  if (typeName === 'boolean' || typeName === 'Boolean') return 'checkbox'
  if (['long', 'Long', 'int', 'Integer', 'double', 'Double', 'float', 'Float'].includes(typeName)) {
    return 'number'
  }
  return 'text'
}

/** checkbox / number 的值绑定归一。 */
function coerce(index: number, event: Event, typeName: string): void {
  const target = event.target as HTMLInputElement
  if (typeName === 'boolean' || typeName === 'Boolean') {
    formValues.value[index] = target.checked
  } else if (
    ['long', 'Long', 'int', 'Integer', 'short', 'Short', 'byte', 'Byte'].includes(typeName)
  ) {
    formValues.value[index] = target.value === '' ? 0 : Number.parseInt(target.value, 10)
  } else if (['double', 'Double', 'float', 'Float'].includes(typeName)) {
    formValues.value[index] = target.value === '' ? 0 : Number.parseFloat(target.value)
  } else {
    formValues.value[index] = target.value
  }
  onFormInput()
}

// ---------- 复杂类型参数：参数级 JSON 编辑弹层 ----------
/** 简单类型白名单（之外一律视为复杂类型，用 JSON 编辑）。 */
const SIMPLE_TYPES = new Set([
  'String', 'char', 'Character', 'boolean', 'Boolean', 'int', 'Integer', 'short', 'Short',
  'byte', 'Byte', 'long', 'Long', 'float', 'Float', 'double', 'Double', 'BigDecimal', 'BigInteger',
])

function isComplexParam(p: JavaParamInfo): boolean {
  return !SIMPLE_TYPES.has(p.type_name)
}

/** 弹层状态：目标参数下标 + 草稿文本。 */
const jsonEdit = reactive<{ open: boolean; index: number; draft: string; error: string }>({
  open: false,
  index: -1,
  draft: '',
  error: '',
})

function openJsonEditor(index: number): void {
  const current = formValues.value[index]
  jsonEdit.index = index
  jsonEdit.draft = JSON.stringify(current ?? null, null, 2)
  jsonEdit.error = ''
  jsonEdit.open = true
}

const jsonEditParamTitle = computed(() => {
  const p = activeMethod.value?.params[jsonEdit.index]
  if (!p) return t('java.paramEditJson')
  return t('java.paramJsonTitle', { name: p.name, type: p.generic_type || p.type_name })
})

function applyJsonEdit(): void {
  try {
    const parsed = JSON.parse(jsonEdit.draft) as unknown
    formValues.value[jsonEdit.index] = parsed
    onFormInput()
    jsonEdit.open = false
  } catch (e) {
    jsonEdit.error = t('java.paramJsonInvalid', {
      message: String(e instanceof Error ? e.message : e),
    })
  }
}

/** 复杂参数摘要（单行预览，点开弹层编辑完整 JSON）。 */
function complexPreview(value: unknown): string {
  const text = JSON.stringify(value ?? null)
  return text.length > 48 ? `${text.slice(0, 48)}…` : text
}

// ---------- 危险方法名二次确认 ----------
const DANGEROUS_NAME = /\b(delete|remove|clear|drop|truncate|shutdown|stop|kill|reset|purge|expire|evict|cancel)\w*/i

function dangerousKeyword(method: JavaMethodInfo): string | null {
  const m = DANGEROUS_NAME.exec(method.method_name)
  return m ? m[0] : null
}

const dangerousConfirm = ref(false)
const dangerKeyword = computed(() =>
  activeMethod.value ? dangerousKeyword(activeMethod.value) : null,
)

function onRunClick(): void {
  if (dangerKeyword.value && !dangerousConfirm.value) {
    dangerousConfirm.value = true
    return
  }
  dangerousConfirm.value = false
  emit('run')
}
</script>

<template>
  <section class="jmp">
    <EmptyState
      v-if="!hasConnection"
      icon="plug"
      :title="t('java.connectFirstTitle')"
      :description="t('java.connectFirstDesc')"
    />
    <EmptyState
      v-else-if="!className"
      icon="package"
      :title="t('java.pickClassTitle')"
      :description="t('java.pickClassDesc')"
    />
    <EmptyState
      v-else-if="loading"
      icon="clock"
      :title="t('java.loadingMethods')"
      compact
    />

    <template v-else>
      <div class="jmp-methods">
        <h3 class="jmp-section-title">{{ t('java.methodsTitle', { count: methods.length }) }}</h3>
        <ul class="jmp-method-list">
          <li v-for="m in methods" :key="m.method_id">
            <button
              type="button"
              class="jmp-method"
              :class="{ active: m.method_id === activeMethodId }"
              @click="emit('selectMethod', m)"
            >
              <span class="jmp-method-head">
                <span class="jmp-method-name">{{ m.method_name }}</span>
                <span v-if="m.overloaded" class="jmp-badge">{{ t('java.overloadedBadge') }}</span>
                <span v-if="m.static_method" class="jmp-badge jmp-badge-static">{{ t('java.staticBadge') }}</span>
                <span
                  v-if="attachMode && m.visibility === 'private'"
                  class="jmp-badge"
                >private</span>
                <span class="jmp-method-ret">{{ m.return_generic || m.return_type }}</span>
              </span>
              <span class="jmp-method-sig">{{ m.signature }}</span>
            </button>
          </li>
        </ul>
      </div>

      <template v-if="activeMethod">
        <div class="jmp-editor">
          <div class="jmp-editor-head">
            <span class="jmp-section-title">{{ t('java.argsTitle') }}</span>
            <span class="jmp-head-actions">
              <button
                type="button"
                class="miyo-btn jmp-mini-btn"
                :title="t('java.argsTemplateSave')"
                data-testid="java-template-save"
                @click="emit('saveTemplate')"
              >
                {{ t('java.argsTemplateSave') }}
              </button>
              <button
                type="button"
                class="miyo-btn jmp-mini-btn"
                :title="t('java.argsTemplateUse')"
                :disabled="!hasTemplate"
                data-testid="java-template-use"
                @click="emit('useTemplate')"
              >
                {{ t('java.argsTemplateUse') }}
              </button>
              <SegmentedControl
                :model-value="editMode"
                :options="modeOptions"
                size="sm"
                @update:model-value="setMode"
              />
            </span>
          </div>
          <div class="jmp-sig">{{ activeMethod.signature }}</div>

          <!-- 可视化参数表单 -->
          <div v-if="editMode === 'form'" class="jmp-form">
            <div v-if="activeMethod.params.length === 0" class="jmp-noargs">
              {{ t('java.noParams') }}
            </div>
            <div
              v-for="(p, i) in activeMethod.params"
              :key="p.name + i"
              class="jmp-param"
            >
              <span class="jmp-param-name">
                {{ p.name }}
                <span class="jmp-param-type">{{ p.generic_type || p.type_name }}</span>
                <span v-if="p.required" class="jmp-param-req">{{ t('java.requiredBadge') }}</span>
              </span>
              <!-- 复杂类型：JSON 摘要 + 弹层编辑 -->
              <div v-if="isComplexParam(p)" class="jmp-param-complex">
                <code class="jmp-complex-preview">{{ complexPreview(formValues[i]) }}</code>
                <button
                  type="button"
                  class="miyo-btn jmp-mini-btn"
                  :data-testid="`java-param-json-${i}`"
                  @click="openJsonEditor(i)"
                >
                  {{ t('java.paramEditJson') }}
                </button>
              </div>
              <label v-else class="jmp-param-control">
                <input
                  v-if="inputType(p.type_name) !== 'checkbox'"
                  class="miyo-input"
                  :type="inputType(p.type_name)"
                  :value="typeof formValues[i] === 'string' || typeof formValues[i] === 'number'
                    ? String(formValues[i])
                    : formValues[i] === true ? 'true' : formValues[i] === false ? 'false'
                    : formValues[i] == null ? '' : JSON.stringify(formValues[i])"
                  :placeholder="fieldPlaceholder(p.type_name, p.generic_type)"
                  :aria-label="p.name"
                  @input="coerce(i, $event, p.type_name)"
                />
                <span v-else class="jmp-param-bool">
                  <input
                    type="checkbox"
                    :checked="formValues[i] === true"
                    :aria-label="p.name"
                    @change="coerce(i, $event, p.type_name)"
                  />
                </span>
              </label>
            </div>
          </div>

          <!-- JSON 原始编辑器 -->
          <div v-else class="jmp-json">
            <JsonCodeMirror
              :model-value="argsJson"
              language="json"
              :placeholder-text="t('java.argsJsonPlaceholder')"
              @update:model-value="emit('update:argsJson', $event)"
            />
          </div>

          <p v-if="validationError" class="jmp-error" data-testid="java-args-error">
            {{ validationError }}
          </p>

          <!-- attach 模式：实例解析策略（static 方法固定直调；实例方法可选
               auto / bean / new——new 需显式确认，构造器会执行） -->
          <div v-if="attachMode && activeMethod && !activeMethod.static_method" class="jmp-instance">
            <div class="jmp-instance-row">
              <span class="jmp-instance-label">{{ t('java2.instanceKindLabel') }}</span>
              <SegmentedControl
                :model-value="instanceKindValue"
                :options="instanceOptions"
                size="sm"
                data-testid="java-instance-kind"
                @update:model-value="onInstanceKindChange"
              />
            </div>
            <p v-if="instanceKindValue === 'new'" class="jmp-instance-warn">
              {{ t('java2.instanceNewWarn') }}
            </p>
            <label v-if="instanceKindValue === 'new'" class="jmp-instance-consent">
              <input
                type="checkbox"
                :checked="newConsent"
                data-testid="java-new-consent"
                @change="emit('update:newConsent', ($event.target as HTMLInputElement).checked)"
              />
              <span>{{ t('java2.instanceNewConsent') }}</span>
            </label>
          </div>

          <!-- 危险方法二次确认 -->
          <div v-if="dangerousConfirm && dangerKeyword" class="jmp-danger" data-testid="java-danger-confirm">
            <p class="jmp-danger-text">
              {{ t('java.dangerConfirmBody', { keyword: dangerKeyword, method: activeMethod.method_id }) }}
            </p>
            <div class="jmp-danger-actions">
              <button type="button" class="miyo-btn" @click="dangerousConfirm = false">
                {{ t('common.cancel') }}
              </button>
              <button
                type="button"
                class="miyo-btn miyo-btn-danger"
                data-testid="java-danger-run"
                @click="onRunClick"
              >
                {{ t('java.dangerConfirmRun') }}
              </button>
            </div>
          </div>

          <div class="jmp-run-row">
            <label class="jmp-timeout">
              <span>{{ t('java.timeoutOverride') }}</span>
              <input
                class="miyo-input"
                type="number"
                min="1000"
                step="1000"
                :placeholder="t('java.timeoutDefault')"
                :value="timeoutValue ?? ''"
                @change="onTimeoutChange($event)"
              />
            </label>
            <button
              type="button"
              class="miyo-btn miyo-btn-primary"
              :disabled="!runnable || dangerousConfirm"
              data-testid="java-invoke-btn"
              @click="onRunClick"
            >
              <Spinner v-if="invoking" :size="13" />
              <Icon v-else name="play" :size="13" />
              {{ invoking ? t('java.running') : t('java.run') }}
            </button>
            <button
              v-if="invoking"
              type="button"
              class="miyo-btn"
              @click="emit('cancel')"
            >
              {{ t('java.cancelRun') }}
            </button>
          </div>
        </div>
      </template>
    </template>

    <!-- 参数级 JSON 编辑弹层 -->
    <Modal
      :open="jsonEdit.open"
      :title="jsonEditParamTitle"
      width="560px"
      @close="jsonEdit.open = false"
    >
      <div class="jmp-json-modal">
        <JsonCodeMirror
          :model-value="jsonEdit.draft"
          language="json"
          @update:model-value="jsonEdit.draft = $event"
        />
        <p v-if="jsonEdit.error" class="jmp-error">{{ jsonEdit.error }}</p>
        <div class="jmp-json-actions">
          <button type="button" class="miyo-btn" @click="jsonEdit.open = false">
            {{ t('common.cancel') }}
          </button>
          <button
            type="button"
            class="miyo-btn miyo-btn-primary"
            data-testid="java-param-json-apply"
            @click="applyJsonEdit"
          >
            {{ t('java.paramApply') }}
          </button>
        </div>
      </div>
    </Modal>
  </section>
</template>

<style scoped>
.jmp {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
  min-height: 0;
  overflow-y: auto;
  padding: 10px 12px;
}
.jmp-section-title {
  margin: 0;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-tertiary, var(--text-secondary));
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.jmp-method-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.jmp-method {
  display: flex;
  flex-direction: column;
  gap: 3px;
  width: 100%;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.jmp-method:hover {
  background: var(--bg-hover);
}
.jmp-method.active {
  border-color: var(--primary);
}
.jmp-method-head {
  display: flex;
  align-items: center;
  gap: 6px;
}
.jmp-method-name {
  font-size: 12.5px;
  font-weight: 600;
  font-family: var(--font-mono, monospace);
}
.jmp-method-ret {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-tertiary, var(--text-secondary));
  font-family: var(--font-mono, monospace);
}
.jmp-method-sig {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jmp-badge {
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}
.jmp-badge-static {
  font-family: var(--font-mono, monospace);
}
.jmp-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 10px;
}
.jmp-editor-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.jmp-sig {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono, monospace);
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
  padding: 5px 8px;
  overflow-x: auto;
  white-space: nowrap;
}
.jmp-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.jmp-noargs {
  font-size: 12px;
  color: var(--text-tertiary, var(--text-secondary));
  padding: 8px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  text-align: center;
}
.jmp-param {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.jmp-param-control {
  display: flex;
  flex-direction: column;
}
.jmp-head-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.jmp-mini-btn {
  font-size: 11px;
  padding: 3px 8px;
}
.jmp-param-complex {
  display: flex;
  align-items: center;
  gap: 6px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  padding: 5px 8px;
  background: var(--bg-elevated);
}
.jmp-complex-preview {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jmp-danger {
  border: 1px solid var(--danger, #ef4444);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  background: var(--bg-elevated);
}
.jmp-danger-text {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--danger, #ef4444);
  line-height: 1.55;
  word-break: break-all;
}
.jmp-danger-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.jmp-json-modal {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.jmp-json-modal .miyo-input,
.jmp-json-actions {
  margin-top: 2px;
}
.jmp-json-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.jmp-param-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary);
  font-family: var(--font-mono, monospace);
}
.jmp-param-type {
  color: var(--text-tertiary, var(--text-secondary));
  font-size: 11px;
}
.jmp-param-req {
  font-size: 10px;
  color: var(--primary);
}
.jmp-json {
  min-height: 140px;
}
.jmp-error {
  margin: 0;
  font-size: 12px;
  color: var(--danger, #ef4444);
}
.jmp-run-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.jmp-timeout {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary);
}
.jmp-timeout .miyo-input {
  width: 110px;
}
.jmp-run-row .miyo-btn-primary {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
</style>
