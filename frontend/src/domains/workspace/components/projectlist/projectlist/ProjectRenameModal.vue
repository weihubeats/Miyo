<script setup lang="ts">
/**
 * ProjectRenameModal：重命名项目弹窗。
 *
 * - project 为 null 时关闭；名称必填（不超过 50 字符）；
 * - 保存成功后 emit('saved', project)，由父级更新列表。
 */
import { ref, watch } from 'vue'
import Modal from '@/components/ui/Modal.vue'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import type { Project } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const NAME_MAX = 50

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const props = defineProps<{ project: Project | null }>()

const emit = defineEmits<{
  close: []
  saved: [project: Project]
}>()

const renameName = ref('')
const renameError = ref<string | null>(null)

// 切换目标项目时重置表单
watch(
  () => props.project,
  (p) => {
    if (p) {
      renameName.value = p.name
      renameError.value = null
    }
  },
)

async function confirmRename(): Promise<void> {
  const name = renameName.value.trim()
  if (!name) {
    renameError.value = t('workspace.projectNameRequired')
    return
  }
  if (name.length > NAME_MAX) {
    renameError.value = t('pcreate.nameTooLong', { n: NAME_MAX })
    return
  }
  if (!props.project) return
  try {
    const saved = await api.saveProject({ ...props.project, name, updated_at: new Date().toISOString() })
    emit('close')
    toast.success(t('workspace.projectRenamed'))
    emit('saved', saved)
  } catch (e) {
    toast.error(t('workspace.projectRenameFail'), { message: errorText(e), duration: 6000 })
  }
}
</script>

<template>
  <Modal :open="project !== null" :title="t('workspace.renameProject')" width="420px" @close="emit('close')">
    <div class="form-field">
      <label class="form-label" for="rename-project">{{ t('workspace.projectName') }}</label>
      <input
        id="rename-project"
        v-model="renameName"
        class="miyo-input"
        v-focus-end
        :class="{ 'miyo-input-error': renameError }"
        :placeholder="t('workspace.projectName')"
        maxlength="60"
        spellcheck="false"
        @input="renameError = null"
        @keyup.enter="confirmRename"
      />
      <p v-if="renameError" class="miyo-field-error" role="alert">{{ renameError }}</p>
    </div>
    <template #footer>
      <button class="miyo-btn" type="button" @click="emit('close')">{{ t('common.cancel') }}</button>
      <button class="miyo-btn miyo-btn-primary" type="button" :disabled="api.pending.value" @click="confirmRename">
        {{ t('common.save') }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.form-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
}

.form-label {
  font-size: 12px;
  color: var(--text-2);
}

.miyo-input-error {
  border-color: var(--danger) !important;
}
.miyo-input-error:focus {
  border-color: var(--danger) !important;
  box-shadow: 0 0 0 2px var(--danger-tint) !important;
}
</style>
