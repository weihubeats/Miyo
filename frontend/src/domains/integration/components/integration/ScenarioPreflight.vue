<script setup lang="ts">
/**
 * ScenarioPreflight：运行前校验面板（可折叠，默认只显示汇总）。
 *
 * - 汇总：阻断数量 / 警告数量 / 校验状态；点击展开明细；
 * - 明细：每条问题可定位到步骤 / 页签；给出修复建议；
 * - 非阻断警告可「忽略」（仅本次编辑会话，不写库），忽略后不计入警告数；
 * - 阻断问题阻止执行；受控强制执行入口仍保留（产品既有能力）。
 */
import { computed, ref } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import type { ValidateIssue } from '@/utils/scenarioValidate'

const props = defineProps<{
  issues: ValidateIssue[]
  errorCount: number
  warnCount: number
  /** 已忽略问题的稳定键集合。 */
  ignored: Set<string>
  open: boolean
}>()

const emit = defineEmits<{
  (e: 'update:open', open: boolean): void
  (e: 'locate', issue: ValidateIssue): void
  (e: 'ignore', key: string): void
  (e: 'force-run'): void
}>()

const t = useLocaleStore().t

const showIgnored = ref(false)

function issueKey(issue: ValidateIssue): string {
  return [issue.severity, issue.stepIndex, issue.field, issue.message, issue.subject ?? ''].join('|')
}

const visibleIssues = computed(() =>
  props.issues.filter((is) => {
    const ignored = props.ignored.has(issueKey(is))
    if (ignored && !showIgnored.value) return false
    return true
  }),
)

const ignoredCount = computed(
  () => props.issues.filter((is) => props.ignored.has(issueKey(is))).length,
)

const activeWarnCount = computed(() =>
  props.issues.filter(
    (is) => is.severity === 'warning' && !props.ignored.has(issueKey(is)),
  ).length,
)

/** 修复建议（按问题所在区域给方向，不假装知道具体值）。 */
function hintOf(issue: ValidateIssue): string {
  switch (issue.field) {
    case 'request':
      return t('integration.validateHintRequest')
    case 'control':
      return t('integration.validateHintControl')
    case 'extract':
      return t('integration.validateHintExtract')
    case 'assert':
      return t('integration.validateHintAssert')
    case 'vars':
      return t('integration.validateHintVars')
    default:
      return t('integration.validateHintGeneral')
  }
}

function toggle(): void {
  emit('update:open', !props.open)
}

function ignore(issue: ValidateIssue): void {
  if (issue.severity !== 'warning') return
  emit('ignore', issueKey(issue))
}
</script>

<template>
  <div class="wf-validate" :class="{ collapsed: !open }">
    <button class="wf-validate-head" type="button" @click="toggle">
      <Icon name="chevron-right" :size="12" class="wf-validate-caret" />
      <span class="wf-validate-title">{{ t('integration.validateTitle') }}</span>
      <span v-if="errorCount" class="wf-validate-tag error">
        {{ t('integration.validateErrors', { n: errorCount }) }}
      </span>
      <span v-if="activeWarnCount" class="wf-validate-tag warn">
        {{ t('integration.validateWarnings', { n: activeWarnCount }) }}
      </span>
      <span v-if="!errorCount && !activeWarnCount" class="wf-validate-tag ok">
        {{ t('integration.validateOk') }}
      </span>
      <span v-if="ignoredCount" class="wf-validate-ignored">
        {{ t('integration.validateIgnoredCount', { n: ignoredCount }) }}
      </span>
      <div class="wf-validate-spacer"></div>
      <span v-if="!open" class="wf-validate-hint">{{ t('integration.validateExpand') }}</span>
    </button>

    <template v-if="open">
      <div v-if="ignoredCount" class="wf-validate-ignored-row">
        <label class="wf-validate-showignored">
          <input v-model="showIgnored" type="checkbox">
          <span>{{ t('integration.validateShowIgnored') }}</span>
        </label>
      </div>

      <p v-if="!visibleIssues.length" class="wf-validate-empty">
        {{ t('integration.validateOk') }}
      </p>
      <ul v-else class="wf-validate-list">
        <li
          v-for="(is, k) in visibleIssues"
          :key="`${issueKey(is)}-${k}`"
          class="wf-validate-item"
          :class="[is.severity, { ignored: ignored.has(issueKey(is)) }]"
          role="button"
          tabindex="0"
          @click="emit('locate', is)"
          @keydown.enter="emit('locate', is)"
        >
          <span class="wf-validate-sev">{{ is.severity === 'error' ? '!' : '·' }}</span>
          <span class="wf-validate-where">
            {{ is.stepIndex >= 0 ? t('integration.validateStepAt', { n: is.stepIndex + 1 }) : t('integration.validateScene') }}
          </span>
          <span class="wf-validate-msg">
            <span>{{ t(is.message, is.detail ?? {}) }}</span>
            <span class="wf-validate-hintline">
              {{ t('integration.validateHintLabel') }}：{{ hintOf(is) }}
            </span>
          </span>
          <span class="wf-validate-go">{{ t('integration.validateGoto') }}</span>
          <IconButton
            v-if="is.severity === 'warning' && !ignored.has(issueKey(is))"
            name="eye-off"
            :size="12"
            :title="t('integration.validateIgnore')"
            @click.stop="ignore(is)"
          />
        </li>
      </ul>

      <div v-if="errorCount" class="wf-validate-foot">
        <span class="wf-validate-foot-hint">{{ t('integration.validateForceHint') }}</span>
        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('force-run')">
          {{ t('integration.validateForceRun') }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.wf-validate {
  flex: none;
  margin: 0 0 6px;
  border-radius: var(--radius-md);
  background: var(--bg-panel);
}

.wf-validate-head {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xs);
  cursor: pointer;
  text-align: left;
}

.wf-validate-caret {
  flex: none;
  transition: transform 0.15s ease;
}

.wf-validate:not(.collapsed) .wf-validate-caret {
  transform: rotate(90deg);
}

.wf-validate-title {
  flex: none;
  font-weight: 600;
}

.wf-validate-tag {
  flex: none;
  padding: 0 6px;
  border-radius: var(--radius-pill);
  font-size: var(--fs-2xxs);
  line-height: 16px;
}

.wf-validate-tag.error {
  background: var(--danger-tint);
  color: var(--danger);
}

.wf-validate-tag.warn {
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-validate-tag.ok {
  background: var(--success-tint);
  color: var(--success);
}

.wf-validate-ignored {
  flex: none;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.wf-validate-spacer {
  flex: 1;
}

.wf-validate-hint {
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.wf-validate-ignored-row {
  padding: 0 10px 4px;
}

.wf-validate-showignored {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-validate-list {
  margin: 0;
  padding: 2px 6px 6px;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.wf-validate-empty {
  margin: 0;
  padding: 6px 12px 10px;
  font-size: var(--fs-xxs);
  color: var(--success);
}

.wf-validate-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xxs);
  color: var(--text-2);
  cursor: pointer;
}

.wf-validate-item:hover {
  background: var(--bg-hover);
}

.wf-validate-item.ignored {
  opacity: 0.55;
}

.wf-validate-sev {
  flex: none;
  width: 14px;
  text-align: center;
  font-weight: 700;
  color: var(--text-3);
}

.wf-validate-item.error .wf-validate-sev {
  color: var(--danger);
}

.wf-validate-item.warning .wf-validate-sev {
  color: var(--warning);
}

.wf-validate-where {
  flex: none;
  color: var(--text-3);
}

.wf-validate-msg {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.wf-validate-hintline {
  color: var(--text-3);
  font-size: var(--fs-2xxs);
}

.wf-validate-go {
  flex: none;
  color: var(--accent);
  font-size: var(--fs-2xxs);
}

.wf-validate-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px 8px;
  border-top: 1px solid var(--border);
}

.wf-validate-foot-hint {
  flex: 1;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

@media (prefers-reduced-motion: reduce) {
  .wf-validate-caret {
    transition: none;
  }
}
</style>