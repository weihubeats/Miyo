<script setup lang="ts">
/**
 * ProjectCreateModal：新建 API 项目弹窗。
 *
 * - 名称必填（不超过 50 字符）、描述可选（不超过 200 字符）；
 * - 创建成功后 emit('created', project)，由父级并入列表。
 */
import { computed, ref, watch } from 'vue'
import Modal from '@/components/ui/Modal.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import type { SelectOption } from '@/components/ui/CustomSelect.vue'
import { useMiyoApi } from '@/composables/useMiyoApi'
import { useToast } from '@/composables/useToast'
import { useLocaleStore } from '@/stores/locale'
import type { Project, Team } from '@/types/miyoApi.d'
import { errorText } from '@/utils/errorText'

const NAME_MAX = 50
const DESC_MAX = 200
/** 行内新建团队哨兵值（CustomSelect 选项）。 */
const NEW_TEAM = '__new__'

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const open = defineModel<boolean>('open', { default: false })

const props = defineProps<{
  /** 预选团队（团队组头部「＋ 新建」快捷新建时传入）。 */
  presetTeamId?: string | null
}>()

const newName = ref('')
const newDesc = ref('')
const createError = ref<string | null>(null)
const teams = ref<Team[]>([])
const selectedTeamId = ref<string | null>(null)
/** 行内新建团队输入（选择"新建团队…"时展开）。 */
const newTeamName = ref('')

/** 团队选项：团队列表（含默认团队）+ 行内新建。 */
const teamOptions = computed<SelectOption[]>(() => [
  ...teams.value.map((team) => ({ value: team.id, label: team.name })),
  { value: NEW_TEAM, label: t('pcreate.teamNew') },
])

async function loadTeams(): Promise<void> {
  try {
    teams.value = await api.listTeams()
  } catch {
    teams.value = []
  }
}

// 每次打开时重置表单（预选团队优先，否则默认团队；避免 null 与列表里的「默认团队」重复出现）
watch(open, (v) => {
  if (v) {
    newName.value = ''
    newDesc.value = ''
    createError.value = null
    newTeamName.value = ''
    void loadTeams().then(() => {
      const fallback =
        (props.presetTeamId && teams.value.find((team) => team.id === props.presetTeamId)) ||
        teams.value.find((team) => team.id === '00000000-0000-0000-0000-000000000001') ||
        teams.value[0]
      selectedTeamId.value = fallback?.id ?? null
    })
  }
})

const emit = defineEmits<{ created: [project: Project] }>()

async function confirmCreate(): Promise<void> {
  const name = newName.value.trim()
  if (!name) {
    createError.value = t('workspace.projectNameRequired')
    return
  }
  if (name.length > NAME_MAX) {
    createError.value = t('pcreate.nameTooLong', { n: NAME_MAX })
    return
  }
  const now = new Date().toISOString()
  try {
    // 选了"新建团队"：先建团队再建项目。
    let teamId = selectedTeamId.value
    if (teamId === NEW_TEAM) {
      const tn = newTeamName.value.trim()
      if (!tn) {
        createError.value = t('pcreate.teamNameRequired')
        return
      }
      const team = await api.createTeam(tn)
      teamId = team.id
    }
    const project = await api.saveProject({
      id: crypto.randomUUID(),
      name,
      description: newDesc.value.trim(),
      variables: {},
      team_id: teamId,
      created_at: now,
      updated_at: now,
    })
    open.value = false
    toast.success(t('workspace.projectCreated'), { message: name })
    emit('created', project)
  } catch (e) {
    toast.error(t('workspace.projectCreateFail'), { message: errorText(e), duration: 6000 })
  }
}
</script>

<template>
  <Modal v-model:open="open" :title="t('projectlist.newProject')" width="420px" @close="open = false">
    <div class="form-field">
      <label class="form-label" for="new-project-name">{{ t('workspace.projectName') }}</label>
      <input
        id="new-project-name"
        v-model="newName"
        class="miyo-input"
        :class="{ 'miyo-input-error': createError }"
        :placeholder="t('workspace.projectNamePh')"
        maxlength="60"
        spellcheck="false"
        @input="createError = null"
        @keyup.enter="confirmCreate"
      />
      <p v-if="createError" class="miyo-field-error" role="alert">{{ createError }}</p>
    </div>
    <div class="form-field">
      <label class="form-label">{{ t('pcreate.team') }}</label>
      <CustomSelect
        v-model="selectedTeamId"
        :options="teamOptions"
        pop-auto-width
      />
      <input
        v-if="selectedTeamId === NEW_TEAM"
        v-model="newTeamName"
        class="miyo-input"
        :placeholder="t('pcreate.teamNamePh')"
        maxlength="30"
        @keyup.enter="confirmCreate"
      />
    </div>
    <div class="form-field">
      <label class="form-label" for="new-project-desc">{{ t('workspace.projectDesc') }}</label>
      <textarea
        id="new-project-desc"
        v-model="newDesc"
        class="miyo-textarea"
        :maxlength="DESC_MAX"
        :placeholder="t('workspace.projectDescPh')"
        rows="3"
      ></textarea>
    </div>
    <template #footer>
      <button class="miyo-btn" type="button" @click="open = false">{{ t('common.cancel') }}</button>
      <button class="miyo-btn miyo-btn-primary" type="button" :disabled="api.pending.value" @click="confirmCreate">
        {{ t('workspace.create') }}
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
