<script setup lang="ts">
/**
 * ProjectDeleteModal：删除项目确认弹窗。
 *
 * - project 为 null 时关闭；
 * - 确认后调用后端删除并 emit('deleted', id)，由父级移出列表。
 */
import Modal from '../ui/Modal.vue'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { useLocaleStore } from '../../stores/locale'
import type { Project } from '../../types/miyoApi'
import { errorText } from '../../utils/errorText'

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const props = defineProps<{ project: Project | null }>()

const emit = defineEmits<{
  close: []
  deleted: [id: string]
}>()

async function confirmDelete(): Promise<void> {
  if (!props.project) return
  const target = props.project
  try {
    await api.deleteProject(target.id)
    emit('close')
    toast.success(t('workspace.projectDeleted'), { message: target.name })
    emit('deleted', target.id)
  } catch (e) {
    toast.error(t('workspace.projectDeleteFail'), { message: errorText(e), duration: 6000 })
  }
}
</script>

<template>
  <Modal :open="project !== null" :title="t('projectTabs.delete')" width="380px" @close="emit('close')">
    <p class="confirm-hint">
      {{ t('pdelete.confirm', { name: project?.name ?? '' }) }}
    </p>
    <template #footer>
      <button class="miyo-btn" type="button" @click="emit('close')">{{ t('common.cancel') }}</button>
      <button class="miyo-btn miyo-btn-danger-solid" type="button" :disabled="api.pending.value" @click="confirmDelete">
        {{ t('common.delete') }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.confirm-hint {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-2);
  line-height: 1.6;
  word-break: break-all;
}
</style>
