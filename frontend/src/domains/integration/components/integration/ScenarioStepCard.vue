<script setup lang="ts">
/**
 * ScenarioStepCard：工作流步骤卡片（第三层步骤列表的单行）。
 *
 * 展示：序号 · 方法/类型 · 名称 · 路径 · 执行状态 · 断言数 · 提取数 · 耗时。
 * 状态用颜色区分（未执行 / 执行中 / 成功 / 失败 / 跳过 / 取消 / 阻断），
 * 失败只做左缘强调 + 淡底，不用大面积红框。
 *
 * 纯展示 + 事件转发：排序由宿主的 SortableJS 负责（拖拽手柄 `.wf-drag`），
 * 编辑态数据归 WorkflowEditor。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import { methodBadgeTone } from '@/utils/methodTone'
import { formatDurationShort } from '@/utils/format'
import Icon from '@/components/ui/Icon.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'
import type { StepTone } from '@/domains/integration/composables/useScenarioRun'

const props = defineProps<{
  index: number
  method: string
  title: string
  path: string
  tone: StepTone | null
  toneText: string
  duration: number | null
  httpStatus: number | null
  write: boolean
  enabled: boolean
  selected: boolean
  checked: boolean
  hasError: boolean
  issueCount: number
  collapsed: boolean
  elapsed: string
  loopLabel: string
  conditionOn: boolean
  parallel: boolean
  waitMs: number | null
  extractCount: number
  assertCount: number
  canViewResult: boolean
}>()

const emit = defineEmits<{
  (e: 'select'): void
  (e: 'toggle-enabled'): void
  (e: 'duplicate'): void
  (e: 'menu', event: MouseEvent): void
  (e: 'remove'): void
  (e: 'toggle-collapse'): void
  (e: 'toggle-check', checked: boolean): void
  (e: 'view-result'): void
}>()

const t = useLocaleStore().t

const durationText = computed(() =>
  props.duration != null ? formatDurationShort(props.duration) : '',
)
</script>

<template>
  <div
    class="wf-step"
    :class="{
      selected,
      disabled: !enabled,
      invalid: hasError,
      failed: tone === 'fail',
      batch: parallel,
      collapsed,
    }"
    role="button"
    tabindex="0"
    @click="emit('select')"
    @keydown.enter="emit('select')"
  >
    <input
      class="wf-step-check"
      type="checkbox"
      :checked="checked"
      :aria-label="t('integration.batchSelected', { n: index + 1 })"
      @click.stop
      @change="emit('toggle-check', ($event.target as HTMLInputElement).checked)"
    >
    <span class="wf-drag" :title="t('integration.addStep')" aria-hidden="true">⋮⋮</span>
    <span class="wf-step-index num">{{ index + 1 }}</span>
    <span class="wf-step-method" :class="methodBadgeTone(method)">{{ method }}</span>
    <div class="wf-step-main">
      <span class="wf-step-name" :title="title">{{ title }}</span>
      <span v-if="!collapsed" class="wf-step-path" :title="path">{{ path }}</span>
    </div>
    <span v-if="write" class="wf-step-side" :title="t('integration.sideEffectWarn')">
      {{ t('integration.sideEffectWarn') }}
    </span>
    <span v-if="httpStatus != null" class="wf-step-http num">{{ httpStatus }}</span>
    <span v-if="tone" class="wf-step-tone" :class="`tone-${tone}`">{{ toneText }}</span>
    <span v-if="tone === 'run' && elapsed" class="wf-step-elapsed num">{{ elapsed }}</span>
    <span v-if="durationText" class="wf-step-duration num">{{ durationText }}</span>
    <span v-if="!collapsed" class="wf-step-badges">
      <span v-if="loopLabel" class="wf-badge wf-badge-accent" :title="loopLabel">{{ loopLabel }}</span>
      <span v-if="conditionOn" class="wf-badge" :title="t('integration.conditionBadge')">
        {{ t('integration.conditionBadge') }}
      </span>
      <span v-if="parallel" class="wf-badge" :title="t('integration.parallelBadge')">
        {{ t('integration.parallelBadge') }}
      </span>
      <span v-if="(waitMs ?? 0) > 0" class="wf-badge" :title="t('integration.waitBadge', { n: waitMs ?? 0 })">
        {{ t('integration.waitBadge', { n: waitMs ?? 0 }) }}
      </span>
      <span v-if="extractCount" class="wf-badge">{{ t('scenario.stepExtractBadge', { n: extractCount }) }}</span>
      <span v-if="assertCount" class="wf-badge">{{ t('scenario.stepAssertBadge', { n: assertCount }) }}</span>
    </span>
    <span class="wf-step-actions" @click.stop>
      <button
        class="wf-act"
        type="button"
        :title="collapsed ? t('integration.stepExpand') : t('integration.stepCollapse')"
        @click="emit('toggle-collapse')"
      >
        <Icon :name="collapsed ? 'chevron-right' : 'chevron-down'" :size="12" />
      </button>
      <button
        v-if="canViewResult"
        class="wf-act"
        type="button"
        :title="t('integration.stepViewResult')"
        @click="emit('view-result')"
      >
        <Icon name="list" :size="12" />
      </button>
      <button
        class="wf-act"
        type="button"
        :class="{ off: !enabled }"
        :title="t('integration.stepEnabledHint')"
        @click="emit('toggle-enabled')"
      >
        <Icon :name="enabled ? 'eye' : 'eye-off'" :size="12" />
      </button>
      <button class="wf-act" type="button" :title="t('integration.stepDuplicate')" @click="emit('duplicate')">
        <Icon name="copy" :size="12" />
      </button>
      <button class="wf-act" type="button" :title="t('workspace.moreTools')" @click="emit('menu', $event)">
        <Icon name="more-horizontal" :size="12" />
      </button>
      <Popconfirm
        :title="t('integration.stepDelete')"
        :confirm-text="t('common.delete')"
        @confirm="emit('remove')"
      >
        <button class="wf-act" type="button" :title="t('integration.stepDelete')" @click.stop>
          <Icon name="trash" :size="12" />
        </button>
      </Popconfirm>
    </span>
    <span
      v-if="issueCount"
      class="wf-step-issue"
      :class="{ err: hasError }"
      :title="t('integration.validateTitle')"
    >
      {{ issueCount }}
    </span>
  </div>
</template>

<style scoped>
.wf-step {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  cursor: pointer;
  min-width: 0;
  position: relative;
}

.wf-step:hover {
  border-color: var(--border-strong);
}

.wf-step.selected {
  border-color: var(--accent);
  background: var(--accent-tint);
}

.wf-step.disabled {
  opacity: var(--opacity-disabled);
}

.wf-step.batch {
  box-shadow: inset 2px 0 0 var(--accent);
}

.wf-step.invalid {
  border-color: var(--danger);
}

/* 失败态：红左缘 + 淡底，避免大面积红框淹没列表 */
.wf-step.failed {
  border-color: var(--danger-border);
  background: var(--danger-tint);
  box-shadow: inset 3px 0 0 var(--danger);
}

.wf-step.failed.selected {
  border-color: var(--danger);
  background: var(--danger-tint);
}

.wf-step.collapsed .wf-step-name {
  font-weight: 600;
}

.wf-step-check {
  flex: none;
  width: 13px;
  height: 13px;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.wf-drag {
  flex: none;
  cursor: grab;
  color: var(--text-3);
  font-size: var(--fs-2xxs);
  letter-spacing: -2px;
  user-select: none;
}

.wf-step-index {
  flex: none;
  width: 18px;
  text-align: center;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.wf-step-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
}

.wf-step-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.wf-step-name {
  font-size: var(--fs-xs);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-step-path {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-step-side {
  flex: none;
  font-size: var(--fs-2xxs);
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-http {
  flex: none;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.wf-step-duration {
  flex: none;
  font-size: var(--fs-2xxs);
  color: var(--text-3);
}

.wf-step-tone {
  flex: none;
  font-size: var(--fs-2xxs);
  padding: 1px 8px;
  border-radius: var(--radius-pill);
}

.wf-step-tone.tone-ok {
  background: var(--success-tint);
  color: var(--success);
}

.wf-step-tone.tone-fail {
  background: var(--danger-tint);
  color: var(--danger);
}

.wf-step-tone.tone-skip {
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-step-tone.tone-blocked {
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-tone.tone-run {
  background: var(--accent-tint);
  color: var(--accent);
}

.wf-step-tone.tone-wait {
  background: var(--bg-hover);
  color: var(--text-3);
}

.wf-step-tone.tone-cancel {
  background: var(--warning-tint);
  color: var(--warning);
}

.wf-step-badges {
  flex: none;
  display: inline-flex;
  gap: 4px;
}

.wf-badge {
  font-size: var(--fs-2xxs);
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-3);
  font-family: var(--font-mono);
}

.wf-badge-accent {
  background: var(--accent-tint);
  color: var(--accent);
}

.wf-step-elapsed {
  flex: none;
  font-family: var(--font-mono);
  font-size: var(--fs-2xxs);
  color: var(--accent);
  background: var(--accent-tint);
  border-radius: var(--radius-sm);
  padding: 0 5px;
  line-height: 15px;
}

.wf-step-actions {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 1px;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.wf-step:hover .wf-step-actions,
.wf-step:focus-within .wf-step-actions {
  opacity: 1;
}

.wf-act {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}

.wf-act:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}

.wf-act.off {
  color: var(--warning);
}

.wf-step-issue {
  flex: none;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-md);
  background: var(--bg-hover);
  color: var(--text-3);
  font-size: var(--fs-xxs);
  line-height: 16px;
  text-align: center;
}

.wf-step-issue.err {
  background: var(--danger);
  color: var(--text-on-accent);
}

@media (prefers-reduced-motion: reduce) {
  .wf-step-actions {
    transition: none;
  }
}
</style>