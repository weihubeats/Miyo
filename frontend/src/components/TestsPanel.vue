<script setup lang="ts">
/**
 * TestsPanel：测试（断言）面板——请求 Tab 的 Tests 标签页。
 * 从 ToolsDrawer 提取为独立面板：JSON 断言配置 → 运行 → 逐条结果。
 */
import { ref, watch } from 'vue'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { formatDuration } from '../utils/format'
import { handleTextareaTab } from '../utils/textareaIndent'
import Icon from './ui/Icon.vue'
import type { Endpoint, EndpointResult } from '../types/miyoApi'

const props = defineProps<{
  draft: Endpoint | null
  url: string
}>()

const api = useMiyoApi()
const toast = useToast()
const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

const testsJson = ref('')
const testResult = ref<EndpointResult | null>(null)
const testing = ref(false)

/**
 * 仅在切换接口（draft.id）或 tests 引用变化时同步编辑框。
 * 原来 `deep: true` 监听整个草稿：任意嵌套键入都重置 testsJson，
 * 覆盖用户正在输入的内容 + 每键一次 stringify。
 */
function syncFromDraft(): void {
  const tests = (props.draft?.request as { tests?: unknown } | undefined)?.tests
  testsJson.value = tests ? JSON.stringify(tests, null, 2) : ''
}
watch(() => props.draft?.id, syncFromDraft, { immediate: true })
watch(
  () => (props.draft?.request as { tests?: unknown } | undefined)?.tests,
  syncFromDraft,
)

async function runTests(): Promise<void> {
  if (!props.draft) return
  try {
    ;(props.draft.request as { tests?: unknown }).tests = testsJson.value.trim()
      ? JSON.parse(testsJson.value)
      : null
  } catch {
    toast.error(t('tests.invalidJson'))
    return
  }
  testing.value = true
  try {
    testResult.value = await api.testEndpoint({
      endpoint: props.draft,
      url: props.url,
      environment_id: store.activeEnvId,
    })
  } catch (err) {
    toast.error(t('tests.runFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    testing.value = false
  }
}

/** 断言 JSON 编辑器 Tab 缩进而非跳出焦点。 */
function onCodeKeydown(e: KeyboardEvent): void {
  if (e.key === 'Tab') handleTextareaTab(e.target as HTMLTextAreaElement, e)
}
</script>

<template>
  <div class="panel">
    <div class="tp-head">
      <span class="tp-title">{{ t('tests.title') }}</span>
      <span class="tp-hint">{{ t('tests.hint') }}</span>
    </div>
    <textarea
      v-model="testsJson"
      class="miyo-input tp-input"
      spellcheck="false"
      :placeholder="t('tests.ph')"
      @keydown="onCodeKeydown"
    ></textarea>
    <div class="tp-run-row">
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="testing" @click="runTests">
        <Icon :name="testing ? 'refresh' : 'beaker'" :size="13" :stroke-width="testing ? 1.8 : 1.5" />
        {{ testing ? t('tests.testing') : t('tests.run') }}
      </button>
      <span v-if="testResult" class="tp-badge" :class="testResult.ok ? 'ok' : 'fail'">
        {{ testResult.ok ? t('tests.pass') : t('tests.fail') }} · {{ testResult.status ?? '-' }} ·
        {{ formatDuration(testResult.duration_ms) }}
      </span>
    </div>
    <ul v-if="testResult?.outcomes.length" class="tp-list">
      <li
        v-for="(o, i) in testResult.outcomes"
        :key="i"
        class="tp-outcome"
        :class="o.passed ? 'ok' : 'fail'"
      >
        <span class="tp-icon">
          <Icon :name="o.passed ? 'check' : 'x'" :size="13" :stroke-width="2" />
        </span>
        <span class="tp-text">{{ o.description }}</span>
        <span v-if="o.reason" class="tp-reason">{{ o.reason }}</span>
      </li>
    </ul>
    <p v-else-if="testResult && !testResult.ok" class="tp-hint-empty">
      {{ testResult.request_error ?? t('tests.noAssertions') }}
    </p>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tp-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.tp-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}

.tp-hint {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.tp-input {
  width: 100%;
  min-height: 110px;
  font-size: 12px;
  resize: vertical;
}

.tp-run-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.tp-badge {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  font-family: var(--font-mono);
}
.tp-badge.ok {
  color: var(--success);
  background: var(--success-tint);
}
.tp-badge.fail {
  color: var(--danger);
  background: var(--danger-tint);
}

.tp-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.tp-outcome {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 12px;
}
.tp-outcome.ok {
  border-color: var(--success-tint);
  background: var(--success-tint);
  color: var(--text-1);
}
.tp-outcome.fail {
  border-color: var(--danger-border);
  background: var(--danger-tint);
  color: var(--text-1);
}

.tp-icon {
  flex-shrink: 0;
  display: inline-flex;
}
.tp-outcome.ok .tp-icon {
  color: var(--success);
}
.tp-outcome.fail .tp-icon {
  color: var(--danger);
}

.tp-text {
  font-weight: 500;
}

.tp-reason {
  color: var(--text-2);
  word-break: break-all;
}

.tp-hint-empty {
  margin: 0;
  font-size: 12px;
  color: var(--text-2);
}
</style>
