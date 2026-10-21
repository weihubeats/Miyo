<script setup lang="ts">
/**
 * SkillEditorDialog：SKILL.md 编辑弹窗（新建 / 编辑共用）。
 *
 * - frontmatter 只暴露 name + description；未知键（allowed-tools 等）原样透传，
 *   保存时不会被丢弃；
 * - 正文用 CodeMirror（markdown 模式）编辑，复用 JsonCodeMirror 的封装；
 * - 编辑态把原文作为 `rawExpected` 回传，后端据此做并发保护。
 */
import { computed, ref, watch } from 'vue'
import Modal from '../ui/Modal.vue'
import Icon from '../ui/Icon.vue'
import JsonCodeMirror from '../JsonCodeMirror.vue'
import { useLocaleStore } from '../../stores/locale'
import type { SkillDoc } from '../../types/foxApi'

const props = withDefaults(
  defineProps<{
    open: boolean
    mode: 'create' | 'edit'
    doc: SkillDoc | null
    saving?: boolean
  }>(),
  { saving: false },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  save: [
    payload: {
      name: string
      description: string
      body: string
      extra: Record<string, unknown>
      rawExpected: string | null
    },
  ]
  delete: []
}>()

const t = useLocaleStore().t

const name = ref('')
const description = ref('')
const body = ref('')

/** 与后端 `validate_name` 保持一致的宽松规则（ASCII 字母数字 + . _ -）。 */
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/

watch(
  () => [props.open, props.doc] as const,
  ([open]) => {
    if (!open) return
    name.value = props.doc?.frontmatter.name ?? ''
    description.value = props.doc?.frontmatter.description ?? ''
    body.value = props.doc?.body ?? ''
  },
  { immediate: true },
)

const nameError = computed(() => {
  if (props.mode === 'edit') return ''
  const value = name.value.trim()
  if (!value) return t('skills.nameRequired')
  if (!NAME_RE.test(value) || value.endsWith('.') || value.includes('..')) {
    return t('skills.nameInvalid')
  }
  return ''
})

const extraKeys = computed(() => Object.keys(props.doc?.frontmatter.extra ?? {}))

const canSave = computed(
  () => !nameError.value && description.value.trim().length > 0 && !props.saving,
)

function submit(): void {
  if (!canSave.value) return
  emit('save', {
    name: name.value.trim(),
    description: description.value.trim(),
    body: body.value,
    extra: (props.doc?.frontmatter.extra ?? {}) as Record<string, unknown>,
    rawExpected: props.doc?.raw ?? null,
  })
}
</script>

<template>
  <Modal
    :open="open"
    :title="mode === 'create' ? t('skills.createTitle') : t('skills.editTitle')"
    width="760px"
    @update:open="emit('update:open', $event)"
  >
    <div class="editor">
      <p v-if="mode === 'edit' && doc" class="source-path" :title="doc.summary.dir">
        <span class="source-label">{{ t('skills.realDir') }}</span>
        <code>{{ doc.summary.dir }}</code>
      </p>

      <label class="field">
        <span class="label">{{ t('skills.nameLabel') }}</span>
        <input
          v-model="name"
          class="rf-input"
          type="text"
          :readonly="mode === 'edit'"
          :placeholder="t('skills.namePlaceholder')"
          spellcheck="false"
        />
        <span v-if="nameError" class="field-error">{{ nameError }}</span>
        <span v-else-if="mode === 'edit'" class="field-hint">{{ t('skills.nameLocked') }}</span>
      </label>

      <label class="field">
        <span class="label">{{ t('skills.descLabel') }}</span>
        <input
          v-model="description"
          class="rf-input"
          type="text"
          :placeholder="t('skills.descPlaceholder')"
        />
        <span v-if="!description.trim()" class="field-hint">{{ t('skills.descRequired') }}</span>
      </label>

      <div class="field body-field">
        <span class="label">{{ t('skills.bodyLabel') }}</span>
        <div class="body-editor">
          <JsonCodeMirror
            v-model="body"
            language="markdown"
            :placeholder-text="t('skills.bodyPlaceholder')"
            autofocus
          />
        </div>
      </div>

      <p v-if="extraKeys.length" class="extra-hint">
        {{ t('skills.extraPreserved', { keys: extraKeys.join(', ') }) }}
      </p>
    </div>

    <template #footer>
      <div class="footer">
        <button
          v-if="mode === 'edit'"
          class="rf-btn rf-btn-sm rf-btn-danger"
          type="button"
          :disabled="saving"
          @click="emit('delete')"
        >
          <Icon name="trash" :size="14" />
          {{ t('skills.delete') }}
        </button>
        <span class="spacer" />
        <button class="rf-btn rf-btn-sm" type="button" @click="emit('update:open', false)">
          {{ t('common.cancel') }}
        </button>
        <button
          class="rf-btn rf-btn-sm rf-btn-primary"
          type="button"
          :disabled="!canSave"
          @click="submit"
        >
          <Icon name="save" :size="14" />
          {{ saving ? t('common.saving') : t('common.save') }}
        </button>
      </div>
    </template>
  </Modal>
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 真实源目录：真源条目是链接时展示解析后的内容位置，避免以为编辑的是副本。 */
.source-path {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin: 0;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-2);
  min-width: 0;
}

.source-label {
  flex: 0 0 auto;
  font-size: 11px;
  color: var(--text-3);
}

.source-path code {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-2);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.label {
  font-size: 12px;
  color: var(--text-2);
}

.field-error {
  font-size: 11px;
  color: var(--danger);
}

.field-hint {
  font-size: 11px;
  color: var(--text-3);
}

.body-field {
  flex: 1;
}

.body-editor {
  height: 320px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.extra-hint {
  margin: 0;
  font-size: 11px;
  color: var(--text-3);
}

.footer {
  display: flex;
  align-items: center;
  gap: 8px;
}

.spacer {
  flex: 1;
}
</style>
