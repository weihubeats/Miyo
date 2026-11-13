<script setup lang="ts">
/**
 * ExtractList：变量提取规则行编辑器（场景步骤 / 测试用例共用）。
 *
 * 原实现内嵌在 WorkflowEditor（integration.tabExtract 段），随测试用例
 * 断言/提取能力一起抽为共享组件：原地改 props.rules（与 ParamsPanel 同款
 * 「props 直改」先例），不复制数组——父级的步骤/用例字段即编辑态本身。
 *
 * from 取值与后端 ExtractSpec 对齐：body（JSONPath）/ header（头名）/
 * cookie（Cookie 名）/ body_regex（正则，取首个捕获组）——后两者由
 * miyo-test extract.rs 解析。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import type { ScenarioExtractRule } from '@/types/miyoApi.d'

const rules = defineModel<ScenarioExtractRule[]>('rules', { required: true })

defineProps<{
  /** 空态提示文案（缺省用集成测试的通用文案）。 */
  emptyHint?: string
}>()

const t = useLocaleStore().t

function addRule(): void {
  rules.value = [...rules.value, { name: '', from: 'body', path: '' }]
}

function pathPlaceholder(from: ScenarioExtractRule['from']): string {
  switch (from) {
    case 'header':
      return t('integration.extractHeaderNamePh')
    case 'cookie':
      return t('integration.extractCookieNamePh')
    case 'body_regex':
      return t('integration.extractRegexPh')
    default:
      return t('integration.extractPathPh')
  }
}

/** 随语言刷新的下拉选项（AGENTS.md：常量数组必须包 computed）。 */
function removeAt(j: number): void {
  rules.value = rules.value.filter((_, i) => i !== j)
}

const FROM_OPTIONS = computed(() => [
  { value: 'body', label: t('integration.extractFromBody') },
  { value: 'header', label: t('integration.extractFromHeader') },
  { value: 'cookie', label: t('integration.extractFromCookie') },
  { value: 'body_regex', label: t('integration.extractFromRegex') },
])
</script>

<template>
  <div class="xtr-list">
    <div class="xtr-head">
      <span class="xtr-hint">{{ emptyHint ?? t('integration.extractEmpty') }}</span>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="addRule">
        <Icon name="plus" :size="12" /> {{ t('integration.extractAdd') }}
      </button>
    </div>
    <div v-for="(r, j) in rules" :key="j" class="xtr-row">
      <input v-model="r.name" class="miyo-input xtr-name" type="text" :placeholder="t('integration.extractNamePh')" spellcheck="false" />
      <CustomSelect
        class="xtr-from"
        :model-value="r.from"
        :options="FROM_OPTIONS"
        size="sm"
        :pop-min-width="130"
        @update:model-value="(v) => (r.from = (v === 'header' || v === 'cookie' || v === 'body_regex' ? v : 'body') as ScenarioExtractRule['from'])"
      />
      <input
        v-model="r.path"
        class="miyo-input xtr-path"
        type="text"
        spellcheck="false"
        :placeholder="pathPlaceholder(r.from)"
      />
      <IconButton name="trash" :size="13" :title="t('common.delete')" @click="removeAt(j)" />
    </div>
  </div>
</template>

<style scoped>
.xtr-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.xtr-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.xtr-hint {
  flex: 1;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  line-height: 1.5;
}

.xtr-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.xtr-name {
  flex: none;
  width: 110px;
}

.xtr-from {
  flex: none;
  width: 110px;
}

.xtr-path {
  flex: 1;
  min-width: 0;
}
</style>
