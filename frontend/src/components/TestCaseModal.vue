<script setup lang="ts">
/**
 * TestCaseModal：新建 / 编辑测试用例弹窗（暗黑风）。
 * - 用例名称（必填）
 * - 用例分组：正向 | 负向 | 边界值 | 安全性 | 其他（默认 正向）
 * - 文件夹：自由分组名（datalist 给现有分组建议，可输入新建；空 = 未分组）
 * 确认后 emit('submit', { name, category, folder })，由调用方决定新建 / 编辑语义。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { TEST_CASE_CATEGORIES, caseCategoryLabel } from '../utils/testCases'
import type { TestCaseCategory } from '../types/miyoApi'
import CustomSelect from './ui/CustomSelect.vue'
import Modal from './ui/Modal.vue'

const props = defineProps<{
  open: boolean
  title?: string
  /** 预填名称（编辑时传原名称）。 */
  name?: string
  category?: TestCaseCategory
  /** 预填文件夹（编辑时传原值）。 */
  folder?: string
  /** 现有文件夹清单（datalist 建议）。 */
  folders?: string[]
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  submit: [payload: { name: string; category: TestCaseCategory; folder: string }]
}>()

const nameInput = ref('')
const categorySel = ref<TestCaseCategory>('正向')
const folderInput = ref('')
const FOLDER_DATALIST_ID = `tcm-folders-${Math.random().toString(36).slice(2, 8)}`

const locale = useLocaleStore()
const t = locale.t

/** 下拉展示译文，value 保持存库中文原值。 */
const categoryOptions = computed(() =>
  TEST_CASE_CATEGORIES.map((c) => ({ value: c, label: caseCategoryLabel(c) })),
)

function onCategoryChange(value: string | number): void {
  categorySel.value = String(value) as TestCaseCategory
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      nameInput.value = props.name ?? ''
      categorySel.value = props.category ?? '正向'
      folderInput.value = props.folder ?? ''
    }
  },
)

function confirm(): void {
  const trimmed = nameInput.value.trim()
  if (!trimmed) return
  emit('submit', {
    name: trimmed,
    category: categorySel.value,
    folder: folderInput.value.trim(),
  })
  emit('update:open', false)
}
</script>

<template>
  <Modal
    :open="open"
    :title="title ?? t('testcase.title')"
    width="360px"
    @update:open="emit('update:open', $event)"
  >
    <div class="tcm">
      <label class="tcm-field">
        <span class="tcm-label">{{ t('testcase.name') }}</span>
        <input
          v-model="nameInput"
          class="tcm-input"
          v-focus-end
          type="text"
          :placeholder="t('testcase.namePh')"
          spellcheck="false"
          @keyup.enter="confirm"
        />
      </label>
      <label class="tcm-field">
        <span class="tcm-label">{{ t('testcase.category') }}</span>
        <CustomSelect
          :model-value="categorySel"
          :options="categoryOptions"
          @update:model-value="onCategoryChange"
        />
      </label>
      <label class="tcm-field">
        <span class="tcm-label">{{ t('testcase.folder') }}</span>
        <input
          v-model="folderInput"
          class="tcm-input"
          type="text"
          :list="FOLDER_DATALIST_ID"
          :placeholder="t('testcase.folderPh')"
          spellcheck="false"
          @keyup.enter="confirm"
        />
        <datalist :id="FOLDER_DATALIST_ID">
          <option v-for="f in props.folders ?? []" :key="f" :value="f" />
        </datalist>
      </label>
      <div class="tcm-actions">
        <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('update:open', false)">
          {{ t('common.cancel') }}
        </button>
        <button
          class="miyo-btn miyo-btn-sm miyo-btn-primary"
          type="button"
          :disabled="!nameInput.trim()"
          @click="confirm"
        >
          {{ t('common.confirm') }}
        </button>
      </div>
    </div>
  </Modal>
</template>

<style scoped>
.tcm {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 2px 0 4px;
}

.tcm-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tcm-label {
  font-size: 12px;
  color: var(--text-2);
}

.tcm-input {
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  font-family: inherit;
  font-size: 13px;
  color: var(--text-1);
  background: var(--bg-1);
  outline: none;
  transition: border-color var(--dur) var(--ease);
}
.tcm-input:focus {
  border-color: var(--accent);
}
.tcm-input::placeholder {
  color: var(--text-3);
}

.tcm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}
</style>