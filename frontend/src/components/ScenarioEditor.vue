<script setup lang="ts">
/**
 * ScenarioEditor：场景编排视图（scenario:{id} 伪页签的内容区）。
 * - 步骤列表：引用接口（URL/头/体用接口当前配置）+ 步骤级提取/断言，上下移排序；
 * - 链路变量预览：汇总启用步骤将提取的变量，后续步骤用 {{变量}} 引用；
 * - 运行：串行执行（前步提取值经 runtime 渲染后续请求模板），时间线报告
 *   + 进度事件 + 可取消；失败即停开关；未保存改动在运行前自动保存。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useMiyoApi } from '../composables/useMiyoApi'
import { copyText } from '../utils/clipboard'
import { applyPathVars } from '../utils/pathVariables'
import { formatDurationShort } from '../utils/format'
import { generateScenarioScript, type ScriptStep } from '../utils/scenarioScript'
import CustomSelect from './ui/CustomSelect.vue'
import JsonCodeMirror from './JsonCodeMirror.vue'
import SegmentedControl, { type SegmentOption } from './ui/SegmentedControl.vue'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import type {
  Scenario,
  ScenarioAssertionRule,
  ScenarioExtractRule,
  ScenarioRunResult,
  ScenarioStep,
} from '../types/miyoApi'

const props = defineProps<{ scenarioId: string }>()
const store = useWorkspaceStore()
const t = useLocaleStore().t
const toast = useToast()
const api = useMiyoApi()

// ---------- 编辑态（本地副本 + 显式保存） ----------
/** 断言期望值编辑态统一为字符串，保存时按字面量智能转型。 */
type EditorAssertion = Omit<ScenarioAssertionRule, 'expected'> & { expected: string }
interface EditorStep {
  endpointId: string
  enabled: boolean
  extract: ScenarioExtractRule[]
  assertions: EditorAssertion[]
  open: boolean
}

const name = ref('')
const description = ref('')
const steps = ref<EditorStep[]>([])
const savedSnapshot = ref('')
const loading = ref(true)
const saving = ref(false)

const scenario = computed(() => store.scenarioIndex.get(props.scenarioId) ?? null)

function decodeStep(s: ScenarioStep): EditorStep {
  const extract = Array.isArray(s.extract) ? (s.extract as ScenarioExtractRule[]) : []
  const assertions = Array.isArray(s.assertions) ? (s.assertions as ScenarioAssertionRule[]) : []
  return {
    endpointId: s.endpoint_id,
    enabled: s.enabled,
    extract: extract.map((r) => ({ ...r })),
    assertions: assertions.map((r) => ({
      name: r.name ?? null,
      type: r.type,
      path: r.path ?? '',
      op: r.op ?? 'eq',
      expected: r.expected == null ? '' : String(r.expected),
    })),
    open: false,
  }
}

/** 归一化（去空行）后的步骤，用于脏比较与落库。 */
function normalizedSteps(): unknown[] {
  return steps.value.map((s) => ({
    endpoint_id: s.endpointId,
    enabled: s.enabled,
    extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
    assertions: s.assertions
      .filter((r) => r.type)
      .map((r) => ({
        type: r.type,
        path: r.path?.trim() || null,
        op: r.op || 'eq',
        expected: coerceExpected(r.expected),
      })),
  }))
}

/** 期望值按字面量转型：数字 / 布尔 / 其余保持字符串。 */
function coerceExpected(raw: string): unknown {
  const v = raw.trim()
  if (v === 'true') return true
  if (v === 'false') return false
  if (v !== '' && !Number.isNaN(Number(v))) return Number(v)
  return v
}

function snapshot(): string {
  return JSON.stringify({
    name: name.value.trim(),
    description: description.value.trim(),
    steps: normalizedSteps(),
    // 自定义脚本纳入脏检查；跟随生成版（未编辑）视为无改动。
    script: scriptCustomized.value ? scriptDraft.value : null,
  })
}

const dirty = computed(() => snapshot() !== savedSnapshot.value)

async function save(): Promise<boolean> {
  if (!scenario.value) return false
  saving.value = true
  try {
    const updated: Scenario = {
      ...scenario.value,
      name: name.value.trim() || t('scenario.untitled'),
      description: description.value.trim(),
      steps: normalizedSteps() as Scenario['steps'],
      script: scriptCustomized.value ? scriptDraft.value : null,
    }
    const saved = await store.upsertScenario(updated)
    if (!saved) return false
    savedSnapshot.value = snapshot()
    return true
  } finally {
    saving.value = false
  }
}

// ---------- 步骤编辑 ----------
const endpointOptions = computed(() =>
  store.endpoints.map((e) => ({ value: e.id, label: `${e.method} ${e.name}` })),
)
const endpointMap = computed(() => new Map(store.endpoints.map((e) => [e.id, e] as const)))

const EXTRACT_FROM_OPTIONS = computed(() => [
  { value: 'body', label: t('scenario.fromBody') },
  { value: 'header', label: t('scenario.fromHeader') },
])
const ASSERT_TYPE_OPTIONS = computed(() => [
  { value: 'status', label: t('scenario.assertStatus') },
  { value: 'jsonpath', label: t('scenario.assertJsonpath') },
  { value: 'header', label: t('scenario.assertHeader') },
  { value: 'response_time_ms', label: t('scenario.assertTime') },
])
const ASSERT_OP_OPTIONS = computed(() =>
  (
    ['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'exists', 'not_exists', 'empty', 'not_empty'] as const
  ).map((op) => ({ value: op, label: t(`scenario.op_${op}`) })),
)
const NO_EXPECTED_OPS = new Set(['exists', 'not_exists', 'empty', 'not_empty'])

function addStep(): void {
  const first = store.endpoints[0]
  steps.value.push({
    endpointId: first?.id ?? '',
    enabled: true,
    extract: [],
    assertions: [],
    open: steps.value.length === 0,
  })
}
function removeStep(i: number): void {
  steps.value.splice(i, 1)
}
function moveStep(i: number, delta: number): void {
  const j = i + delta
  if (j < 0 || j >= steps.value.length) return
  const [row] = steps.value.splice(i, 1)
  steps.value.splice(j, 0, row)
}

/** 链路变量预览：启用步骤将提取的变量并集。 */
const producedVars = computed(() => {
  const out: string[] = []
  for (const s of steps.value) {
    if (!s.enabled) continue
    for (const r of s.extract) {
      const n = r.name.trim()
      if (n && !out.includes(n)) out.push(n)
    }
  }
  return out
})

function varTag(name: string): string {
  return `{{${name}}}`
}

// ---------- 运行状态 ----------
// 必须先于视图模式区声明：generatedScript computed 与其 watch 都读 stopOnFailure。
const stopOnFailure = ref(true)
const running = ref(false)
const runId = ref('')
const progress = ref({ done: 0, total: 0 })
const runResult = ref<ScenarioRunResult | null>(null)

// ---------- 视图模式：手动编排 ⇄ 脚本 ----------
/** 手动是唯一编辑真源；脚本视图由步骤实时生成的只读预览（可复制/进 CI）。 */
const viewMode = ref<'manual' | 'script'>('manual')
const VIEW_MODES = computed<SegmentOption[]>(() => [
  { value: 'manual', label: t('scenario.tabManual') },
  { value: 'script', label: t('scenario.tabScript') },
])

const scriptBase = computed(() => {
  const env = store.environments.find((e) => e.id === store.activeEnvId)
  const envBase = (env?.base_url ?? '').trim()
  if (envBase) return envBase
  return store.urlDomain === '{{base_url}}' ? store.sessionBaseUrl : store.urlDomain
})

/** 编辑态 → 生成器输入（启用步骤；Body 只展开 json / text，其余注明以 Miyo 运行为准）。 */
function toScriptSteps(): ScriptStep[] {
  const out: ScriptStep[] = []
  for (const s of steps.value) {
    if (!s.enabled) continue
    const ep = endpointMap.value.get(s.endpointId)
    if (!ep) continue
    const bodyKind =
      ep.request.body.mode === 'json'
        ? ('json' as const)
        : ep.request.body.mode === 'text'
          ? ('text' as const)
          : ('other' as const)
    out.push({
      name: ep.name,
      method: ep.method,
      url: urlTemplateFor(ep),
      headers: ep.request.headers
        .filter((h) => h.enabled)
        .filter((h) => h.key.trim())
        .map((h) => ({ key: h.key.trim(), value: h.value })),
      body:
        ep.request.body.mode === 'json' || ep.request.body.mode === 'text'
          ? ep.request.body.raw
          : null,
      bodyKind,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          type: r.type,
          path: r.path?.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected),
        })),
    })
  }
  return out
}

const generatedScript = computed(() =>
  generateScenarioScript(name.value.trim() || t('scenario.untitled'), toScriptSteps(), {
    base: scriptBase.value,
    stopOnFailure: stopOnFailure.value,
  }),
)

/** 脚本编辑态：未自定义时实时跟随生成版；一旦编辑即以脚本为准（可重置）。 */
const scriptDraft = ref('')
const scriptCustomized = ref(false)

// 手动步骤变化时，未自定义的脚本跟随重新生成。
watch(generatedScript, (g) => {
  if (!scriptCustomized.value) scriptDraft.value = g
})

function onScriptInput(v: string): void {
  scriptDraft.value = v
  // 手改回与生成版一致时自动解除自定义标记。
  scriptCustomized.value = v !== generatedScript.value
}

function resetScript(): void {
  scriptCustomized.value = false
  scriptDraft.value = generatedScript.value
}

async function copyScript(): Promise<void> {
  try {
    await copyText(scriptDraft.value)
    toast.success(t('scenario.scriptCopied'))
  } catch {
    toast.error(t('scenario.scriptCopyFail'))
  }
}

// ---------- 运行 ----------

/** 接口当前配置 → URL 模板（路径变量先行代入；{{var}} 保留给执行期渲染）。 */
function urlTemplateFor(ep: NonNullable<ReturnType<typeof endpointMap.value.get>>): string {
  const domain = store.urlDomain
  const isAbs = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(ep.path)
  const path = ep.path.startsWith('/') ? ep.path : `/${ep.path}`
  return isAbs
    ? applyPathVars(ep.path, ep.request.path_variables)
    : `${domain}${applyPathVars(path, ep.request.path_variables)}`
}

/** 步骤 → 运行入参：URL 保留 {{}} 模板（执行期用 runtime 二次渲染）。 */
function buildRunSteps() {
  const out: {
    endpoint: NonNullable<ReturnType<typeof endpointMap.value.get>>
    url: string
    enabled: boolean
    extract: unknown
    assertions: unknown
  }[] = []
  for (const s of steps.value) {
    if (!s.enabled) continue
    const ep = endpointMap.value.get(s.endpointId)
    if (!ep) continue
    out.push({
      endpoint: ep,
      url: urlTemplateFor(ep),
      enabled: true,
      extract: s.extract.filter((r) => r.name.trim() && r.path.trim()),
      assertions: s.assertions
        .filter((r) => r.type)
        .map((r) => ({
          type: r.type,
          path: r.path?.trim() || null,
          op: r.op || 'eq',
          expected: coerceExpected(r.expected),
        })),
    })
  }
  return out
}

async function run(): Promise<void> {
  if (!scenario.value || running.value) return
  // 未保存先落库（后端按提交内容执行，避免「跑的不是看到的」）。
  if (dirty.value && !(await save())) return
  const built = buildRunSteps()
  if (!built.length) {
    toast.warning(t('scenario.noEnabledSteps'))
    return
  }
  runResult.value = null
  progress.value = { done: 0, total: built.length }
  running.value = true
  runId.value = crypto.randomUUID()
  try {
    const result = await api.runScenario({
      steps: built,
      environmentId: store.activeEnvId,
      stopOnFailure: stopOnFailure.value,
      runId: runId.value,
    })
    runResult.value = result
  } catch (err) {
    toast.error(t('scenario.runFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    running.value = false
  }
}

async function stop(): Promise<void> {
  if (!runId.value) return
  try {
    await api.cancelScenarioRun(runId.value)
  } catch {
    // 已结束的运行取消会失败，忽略即可
  }
}

// 场景切换加载：放在全部状态声明之后——immediate 回调在 setup 期间同步
// 执行，若提前会踩到下方 runResult 等的暂时性死区（WebKit：Cannot access
// uninitialized variable，场景页签一打开即崩）。
watch(
  () => props.scenarioId,
  () => {
    loading.value = true
    const s = scenario.value
    name.value = s?.name ?? ''
    description.value = s?.description ?? ''
    steps.value = (s?.steps ?? []).map(decodeStep)
    scriptDraft.value = s?.script ?? generatedScript.value
    scriptCustomized.value = !!s?.script
    savedSnapshot.value = snapshot()
    runResult.value = null
    loading.value = false
  },
  { immediate: true },
)

let unlistenProgress: UnlistenFn | null = null
onMounted(async () => {
  unlistenProgress = await listen<{
    runId: string
    done: number
    total: number
  }>('miyo:scenario-progress', (e) => {
    if (e.payload.runId !== runId.value) return
    progress.value = { done: e.payload.done, total: e.payload.total }
  })
})
onBeforeUnmount(() => {
  unlistenProgress?.()
  unlistenProgress = null
})
</script>

<template>
  <div v-if="!scenario && !loading" class="se-missing">
    <EmptyState icon="beaker" :title="t('scenario.missing')" compact />
  </div>
  <div v-else class="scenario-editor">
    <div class="se-head">
      <input v-model="name" class="se-name" :placeholder="t('scenario.namePh')" spellcheck="false" />
      <span v-if="dirty" class="se-dirty" :title="t('scenario.unsaved')"></span>
      <div class="se-actions">
        <SegmentedControl
          v-model="viewMode"
          :options="VIEW_MODES"
          size="sm"
          :aria-label="t('scenario.viewMode')"
        />
        <button
          class="miyo-btn miyo-btn-sm"
          type="button"
          :disabled="saving || !dirty"
          @click="save"
        >
          <Icon name="save" :size="13" /> {{ t('common.save') }}
        </button>
        <button
          v-if="viewMode === 'manual' && !running"
          class="miyo-btn miyo-btn-sm se-run"
          type="button"
          @click="run"
        >
          <Icon name="play" :size="13" /> {{ t('scenario.run') }}
        </button>
        <button
          v-else-if="viewMode === 'manual'"
          class="miyo-btn miyo-btn-sm"
          type="button"
          @click="stop"
        >
          <Icon name="stop" :size="13" /> {{ t('scenario.stop') }}
        </button>
      </div>
    </div>
    <input v-model="description" class="se-desc" :placeholder="t('scenario.descPh')" spellcheck="false" />
    <div class="se-subrow">
      <p class="se-hint">{{ t('scenario.flowHint') }}</p>
      <label class="se-sof" :title="t('scenario.stopOnFailureHint')">
        <input v-model="stopOnFailure" type="checkbox" class="se-sof-check" />
        {{ t('scenario.stopOnFailure') }}
      </label>
    </div>

    <div v-if="producedVars.length" class="se-vars">
      <span class="se-vars-label">{{ t('scenario.linkVars') }}</span>
      <code v-for="v in producedVars" :key="v" class="se-var-chip">{{ varTag(v) }}</code>
    </div>

    <template v-if="viewMode === 'manual'">
    <div class="se-steps">
      <div
        v-for="(s, i) in steps"
        :key="i"
        class="se-step"
        :class="{ off: !s.enabled }"
      >
        <div class="se-step-head">
          <span class="se-step-idx">{{ i + 1 }}</span>
          <input
            v-model="s.enabled"
            type="checkbox"
            class="se-step-check"
            :title="t('scenario.stepEnable')"
          />
          <CustomSelect
            v-model="s.endpointId"
            :options="endpointOptions"
            size="sm"
            :placeholder="t('scenario.pickEndpoint')"
            class="se-step-ep"
          />
          <span v-if="s.endpointId && !endpointMap.has(s.endpointId)" class="se-step-missing">
            {{ t('scenario.endpointMissing') }}
          </span>
          <div class="se-step-tools">
            <IconButton
              name="chevron-up"
              :size="13"
              :title="t('scenario.moveUp')"
              :disabled="i === 0"
              @click="moveStep(i, -1)"
            />
            <IconButton
              name="chevron-down"
              :size="13"
              :title="t('scenario.moveDown')"
              :disabled="i === steps.length - 1"
              @click="moveStep(i, 1)"
            />
            <IconButton
              :name="s.open ? 'chevrons-down-up' : 'chevrons-up-down'"
              :size="13"
              :title="t('scenario.stepDetail')"
              @click="s.open = !s.open"
            />
            <IconButton
              name="trash"
              :size="13"
              tone="danger"
              :title="t('scenario.removeStep')"
              @click="removeStep(i)"
            />
          </div>
        </div>

        <div v-if="s.open" class="se-step-body">
          <div class="se-rule-group">
            <div class="se-rule-title">
              {{ t('scenario.extractSection') }}
              <button class="se-rule-add" type="button" @click="s.extract.push({ name: '', from: 'body', path: '' })">
                <Icon name="plus" :size="12" /> {{ t('scenario.addExtract') }}
              </button>
            </div>
            <div v-for="(r, j) in s.extract" :key="j" class="se-rule-row">
              <input
                v-model="r.name"
                class="miyo-input miyo-input-sm se-r-name"
                :placeholder="t('scenario.extractNamePh')"
                spellcheck="false"
              />
              <CustomSelect v-model="r.from" :options="EXTRACT_FROM_OPTIONS" size="sm" class="se-r-from" />
              <input
                v-model="r.path"
                class="miyo-input miyo-input-sm se-r-path"
                :placeholder="r.from === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')"
                spellcheck="false"
              />
              <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="s.extract.splice(j, 1)" />
            </div>
          </div>

          <div class="se-rule-group">
            <div class="se-rule-title">
              {{ t('scenario.assertSection') }}
              <button
                class="se-rule-add"
                type="button"
                @click="s.assertions.push({ name: null, type: 'status', path: '', op: 'eq', expected: '' })"
              >
                <Icon name="plus" :size="12" /> {{ t('scenario.addAssert') }}
              </button>
            </div>
            <div v-for="(a, j) in s.assertions" :key="j" class="se-rule-row">
              <CustomSelect v-model="a.type" :options="ASSERT_TYPE_OPTIONS" size="sm" class="se-a-type" />
              <input
                v-if="a.type === 'jsonpath' || a.type === 'header'"
                v-model="a.path"
                class="miyo-input miyo-input-sm se-a-path"
                :placeholder="a.type === 'header' ? t('scenario.headerPathPh') : t('scenario.extractPathPh')"
                spellcheck="false"
              />
              <CustomSelect v-model="a.op" :options="ASSERT_OP_OPTIONS" size="sm" class="se-a-op" />
              <input
                v-if="!NO_EXPECTED_OPS.has(a.op ?? 'eq')"
                v-model="a.expected"
                class="miyo-input miyo-input-sm se-a-exp"
                :placeholder="t('scenario.expectedPh')"
                spellcheck="false"
              />
              <IconButton name="minus" :size="13" tone="danger" :title="t('common.delete')" @click="s.assertions.splice(j, 1)" />
            </div>
          </div>
        </div>
      </div>

      <button class="se-add-step" type="button" @click="addStep">
        <Icon name="plus" :size="13" /> {{ t('scenario.addStep') }}
      </button>
    </div>

    <!-- 运行报告（时间线） -->
    <div v-if="running || runResult" class="se-report">
      <div class="se-report-head">
        <span class="se-report-title">{{ t('scenario.resultTitle') }}</span>
        <span v-if="running" class="se-report-meta">
          {{ t('scenario.progress', { done: progress.done, total: progress.total }) }}
        </span>
        <span v-else-if="runResult?.stopped_on_failure" class="se-report-warn">
          {{ t('scenario.stoppedAt') }}
        </span>
        <span v-else-if="runResult?.cancelled" class="se-report-warn">
          {{ t('scenario.cancelled') }}
        </span>
      </div>
      <div
        v-for="(r, i) in runResult?.results ?? []"
        :key="i"
        class="se-rrow"
        :class="{ fail: !r.ok }"
      >
        <div class="se-rrow-head">
          <span class="se-rdot" aria-hidden="true"></span>
          <span class="se-rname">{{ i + 1 }}. {{ r.endpoint_name }}</span>
          <span v-if="r.status != null" class="se-rstatus">{{ r.status }}</span>
          <span v-if="r.duration_ms != null" class="se-rtime">{{ formatDurationShort(r.duration_ms) }}</span>
        </div>
        <p v-if="r.request_error" class="se-rerr">{{ r.request_error }}</p>
        <ul v-if="r.outcomes.length" class="se-routcomes">
          <li v-for="(o, j) in r.outcomes" :key="j" :class="{ fail: !o.passed }">
            {{ o.description }}<span v-if="o.reason"> — {{ o.reason }}</span>
          </li>
        </ul>
        <div v-if="r.extracted && Object.keys(r.extracted).length" class="se-rext">
          <span class="se-rext-label">{{ t('scenario.extracted') }}</span>
          <code v-for="(v, k) in r.extracted" :key="k" class="se-var-chip">
            {{ varTag(k) }} = {{ v }}
          </code>
        </div>
      </div>
    </div>
    </template>

    <!-- 脚本视图：可编辑 + 语法高亮；未编辑时实时跟随手动步骤生成 -->
    <template v-else>
      <div class="se-script-bar">
        <span class="se-script-hint">
          {{ scriptCustomized ? t('scenario.scriptCustomized') : t('scenario.scriptHint') }}
        </span>
        <div class="se-script-actions">
          <button
            v-if="scriptCustomized"
            class="miyo-btn miyo-btn-sm"
            type="button"
            @click="resetScript"
          >
            <Icon name="refresh" :size="13" /> {{ t('scenario.scriptReset') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="copyScript">
            <Icon name="copy" :size="13" /> {{ t('common.copyAction') }}
          </button>
        </div>
      </div>
      <JsonCodeMirror
        :model-value="scriptDraft"
        language="javascript"
        class="se-script"
        @update:model-value="onScriptInput"
      />
    </template>
  </div>
</template>

<style scoped>
.scenario-editor {
  height: 100%;
  min-height: 0;
  overflow: auto;
  padding: 14px 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.se-missing {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.se-script-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.se-script-hint {
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}

.se-script {
  flex: 1;
  min-height: 0;
}

.se-script-actions {
  display: flex;
  gap: 8px;
  flex: none;
}

.se-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.se-name {
  flex: 1;
  min-width: 0;
  border: none;
  border-bottom: 1px dashed transparent;
  border-radius: 0;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-sm);
  font-weight: 600;
  outline: none;
  padding: 1px 2px;
}

.se-name:hover {
  border-bottom-color: var(--text-3);
}

.se-dirty {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--warning);
}

.se-actions {
  display: flex;
  gap: 8px;
}

.se-run {
  background: var(--accent, #8b5cf6);
  color: #fff;
}

.se-desc {
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  outline: none;
  padding: 0;
}

.se-desc:focus {
  border-bottom: 1px solid var(--border);
}

.se-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  line-height: 1.6;
  color: var(--text-3);
}

.se-subrow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.se-sof {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-2);
  cursor: pointer;
  white-space: nowrap;
}

.se-vars {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.se-vars-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-var-chip {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  background: var(--bg-code);
  border: 1px solid var(--border);
  color: var(--tok-key);
}

.se-steps {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.se-step {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-card);
  overflow: hidden;
}

.se-step.off {
  opacity: 0.55;
}

.se-step-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
}

.se-step-idx {
  flex: none;
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--bg-hover);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-weight: 600;
}

.se-step-check {
  flex: none;
}

.se-step-ep {
  flex: 1;
  min-width: 0;
}

.se-step-missing {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--danger);
}

.se-step-tools {
  display: flex;
  gap: 2px;
}

.se-step-body {
  border-top: 1px solid var(--border);
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.se-rule-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.se-rule-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-2);
}

.se-rule-add {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
}

.se-rule-add:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}

.se-rule-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.se-r-name {
  flex: 0 0 130px;
}

.se-r-from {
  flex: 0 0 90px;
}

.se-r-path,
.se-a-path,
.se-a-exp {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

.se-a-type {
  flex: 0 0 120px;
}

.se-a-op {
  flex: 0 0 100px;
}

.se-add-step {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px dashed var(--border);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  padding: 6px 12px;
  border-radius: var(--radius-md);
  cursor: pointer;
}

.se-add-step:hover {
  color: var(--text-1);
  border-color: var(--text-3);
}

/* ---- 运行报告 ---- */
.se-report {
  border-top: 1px solid var(--border);
  padding-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.se-report-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.se-report-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-2);
}

.se-report-meta,
.se-report-warn {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-report-warn {
  color: var(--danger);
}

.se-rrow {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.se-rrow.fail {
  border-color: var(--danger-border, var(--danger));
}

.se-rrow-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.se-rdot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
}

.se-rrow.fail .se-rdot {
  background: var(--danger);
}

.se-rname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
  color: var(--text-1);
}

.se-rstatus {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-2);
}

.se-rtime {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.se-rerr {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--danger);
  white-space: pre-wrap;
  word-break: break-all;
}

.se-routcomes {
  margin: 0;
  padding: 0 0 0 16px;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.se-routcomes li {
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-2);
  word-break: break-all;
}

.se-routcomes li.fail {
  color: var(--danger);
}

.se-rext {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.se-rext-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
