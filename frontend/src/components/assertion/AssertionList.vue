<script setup lang="ts">
/**
 * AssertionList：断言规则行编辑器（场景步骤 / 测试用例共用）。
 *
 * 原实现内嵌在 WorkflowEditor（integration.tabAssert 段），随测试用例
 * 断言能力抽为共享组件：原地改 props.rules（与 ParamsPanel 同款
 * 「props 直改」先例），父级的步骤/用例字段即编辑态本身。
 *
 * type 与后端 AssertionSpec（miyo-test assert.rs）对齐——status /
 * jsonpath / header / body / response_time_ms / length（数组或文本长度，
 * 走数字比较）/ graphql_errors（body.errors 数组）/ javascript（JS 沙箱
 * 断言，expected 为代码，assert(value, msg?) 求值，2s 超时无网络）。
 */
import { computed } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import CustomSelect from '../ui/CustomSelect.vue'
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import type { ScenarioAssertionRule } from '../../types/miyoApi'

const rules = defineModel<ScenarioAssertionRule[]>('rules', { required: true })

defineProps<{
  /** 空态提示文案（缺省用集成测试的通用文案）。 */
  emptyHint?: string
}>()

const t = useLocaleStore().t

function addRule(): void {
  rules.value = [...rules.value, { name: '', type: 'status', path: '', op: 'eq', expected: '200' }]
}

function removeAt(j: number): void {
  rules.value = rules.value.filter((_, i) => i !== j)
}

/** 各断言类型的可用算子（status/数字类无 contains；body 只做包含判断）。 */
function opsFor(type: string): string[] {
  switch (type) {
    case 'status':
    case 'response_time_ms':
    case 'length':
      return ['eq', 'neq', 'gt', 'gte', 'lt', 'lte']
    case 'header':
      return ['eq', 'neq', 'contains', 'not_contains', 'exists', 'not_exists', 'matches', 'not_matches', 'empty', 'not_empty']
    case 'jsonpath':
      return ['eq', 'neq', 'contains', 'not_contains', 'gt', 'gte', 'lt', 'lte', 'exists', 'not_exists', 'matches', 'not_matches', 'empty', 'not_empty']
    case 'body':
      return ['contains', 'not_contains', 'exists', 'not_exists']
    case 'graphql_errors':
      return ['empty', 'not_empty', 'contains', 'not_contains']
    case 'javascript':
      return ['eq']
    default:
      return ['eq']
  }
}

const TYPE_OPTIONS = computed(() =>
  ASSERT_TYPES.map((v) => ({ value: v, label: t(`integration.assertType.${v}`) })),
)
function opOptions(type: string) {
  return opsFor(type).map((op) => ({ value: op, label: t(`integration.assertOp.${op}`) }))
}
function onTypeChange(a: ScenarioAssertionRule, type: string): void {
  a.type = type as ScenarioAssertionRule['type']
  const ops = opsFor(type)
  if (!ops.includes(a.op ?? 'eq')) a.op = (ops[0] ?? 'eq') as ScenarioAssertionRule['op']
  if (type === 'status' || type === 'response_time_ms' || type === 'body') a.path = ''
  if (type === 'javascript' && typeof a.expected !== 'string') a.expected = ''
}
/** 需要 path 目标的断言类型（length 的 path 可选：缺省取正文长度）。 */
function needsPath(type: string): boolean {
  return type === 'jsonpath' || type === 'header' || type === 'length'
}
/** 需要期望值的断言类型（exists/empty 类与 JS 脚本不需要）。 */
function needsExpected(a: ScenarioAssertionRule): boolean {
  if (a.type === 'javascript') return false
  return !['exists', 'not_exists', 'empty', 'not_empty'].includes(a.op ?? 'eq')
}

const ASSERT_TYPES = [
  'status',
  'jsonpath',
  'header',
  'body',
  'response_time_ms',
  'length',
  'graphql_errors',
  'javascript',
] as const
</script>

<template>
  <div class="asr-list">
    <div class="asr-head">
      <span class="asr-hint">{{ emptyHint ?? t('integration.assertEmpty') }}</span>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="addRule">
        <Icon name="plus" :size="12" /> {{ t('integration.assertAdd') }}
      </button>
    </div>
    <template v-for="(a, j) in rules" :key="j">
      <div v-if="a.type === 'javascript'" class="asr-row asr-script-row">
        <div class="asr-script-top">
          <input v-model="a.name" class="miyo-input asr-name" type="text" :placeholder="t('integration.assertNamePh')" spellcheck="false" />
          <CustomSelect
            class="asr-type"
            :model-value="a.type"
            :options="TYPE_OPTIONS"
            size="sm"
            :pop-min-width="150"
            @update:model-value="(v) => onTypeChange(a, String(v ?? 'status'))"
          />
          <IconButton name="trash" :size="13" :title="t('common.delete')" @click="removeAt(j)" />
        </div>
        <textarea
          :value="typeof a.expected === 'string' ? a.expected : ''"
          class="miyo-input asr-code"
          rows="3"
          spellcheck="false"
          :placeholder="t('integration.assertCodePh')"
          @input="a.expected = ($event.target as HTMLTextAreaElement).value"
        ></textarea>
      </div>
      <div v-else class="asr-row">
        <input v-model="a.name" class="miyo-input asr-name" type="text" :placeholder="t('integration.assertNamePh')" spellcheck="false" />
        <CustomSelect
          class="asr-type"
          :model-value="a.type"
          :options="TYPE_OPTIONS"
          size="sm"
          :pop-min-width="150"
          @update:model-value="(v) => onTypeChange(a, String(v ?? 'status'))"
        />
        <input
          v-if="needsPath(a.type)"
          v-model="a.path"
          class="miyo-input asr-path"
          type="text"
          spellcheck="false"
          :placeholder="a.type === 'length' ? t('integration.assertLengthPh') : t('integration.assertTargetPh')"
        />
        <CustomSelect
          class="asr-op"
          :model-value="a.op"
          :options="opOptions(a.type)"
          size="sm"
          :pop-min-width="130"
          @update:model-value="(v) => (a.op = String(v ?? 'eq') as ScenarioAssertionRule['op'])"
        />
        <input
          v-if="needsExpected(a)"
          v-model="a.expected"
          class="miyo-input asr-expected"
          type="text"
          spellcheck="false"
          :placeholder="t('integration.assertExpectedPh')"
        />
        <IconButton name="trash" :size="13" :title="t('common.delete')" @click="removeAt(j)" />
      </div>
    </template>
  </div>
</template>

<style scoped>
.asr-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.asr-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.asr-hint {
  flex: 1;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.5;
}

.asr-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.asr-script-row {
  flex-direction: column;
  align-items: stretch;
}

.asr-script-top {
  display: flex;
  align-items: center;
  gap: 6px;
}

.asr-code {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  line-height: 1.5;
  resize: vertical;
  min-height: 56px;
}

.asr-name {
  flex: none;
  width: 110px;
}

.asr-type {
  flex: none;
  width: 132px;
}

.asr-op {
  flex: none;
  width: 116px;
}

.asr-path {
  flex: 1;
  min-width: 0;
}

.asr-expected {
  flex: 1;
  min-width: 0;
}
</style>
