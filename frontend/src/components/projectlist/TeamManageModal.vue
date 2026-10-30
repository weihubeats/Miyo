<script setup lang="ts">
/**
 * TeamManageModal：团队管理弹窗（现阶段团队仅是项目容器）。
 *
 * - 列出全部团队（默认团队恒在首位，不可删除/重命名提示）；
 * - 行内新建 / 重命名 / 删除（有下属项目时后端拒绝，错误回显到该行）；
 * - 任何变更后 emit('changed')，父级刷新团队与项目列表。
 */
import { ref, watch } from 'vue'
import Modal from '../ui/Modal.vue'
import Icon from '../ui/Icon.vue'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { useLocaleStore } from '../../stores/locale'
import type { Team } from '../../types/miyoApi'

const NAME_MAX = 30
/** 默认团队固定 UUID（迁移 0015），不可删除/改名。 */
const DEFAULT_TEAM_ID = '00000000-0000-0000-0000-000000000001'

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const open = defineModel<boolean>('open', { default: false })
const emit = defineEmits<{ changed: []; 'update:open': [open: boolean] }>()

const teams = ref<Team[]>([])
const newTeamName = ref('')
/** 每行的行内编辑态：teamId → 新名称 */
const renaming = ref<Record<string, string>>({})
/** 每行错误回显：teamId → 错误信息 */
const rowErrors = ref<Record<string, string>>({})
const busy = ref(false)

async function load(): Promise<void> {
  try {
    teams.value = await api.listTeams()
  } catch {
    teams.value = []
  }
}

watch(open, (v) => {
  if (v) {
    newTeamName.value = ''
    renaming.value = {}
    rowErrors.value = {}
    void load()
  }
})

function isDefault(team: Team): boolean {
  return team.id === DEFAULT_TEAM_ID
}

async function createTeam(): Promise<void> {
  const name = newTeamName.value.trim()
  if (!name) return
  if (name.length > NAME_MAX) {
    toast.error(t('team.nameTooLong', { n: NAME_MAX }))
    return
  }
  busy.value = true
  try {
    await api.createTeam(name)
    newTeamName.value = ''
    await load()
    emit('changed')
  } catch (e) {
    toast.error(t('team.createFail'), { message: e instanceof Error ? e.message : String(e), duration: 5000 })
  } finally {
    busy.value = false
  }
}

function startRename(team: Team): void {
  renaming.value = { ...renaming.value, [team.id]: team.name }
}

function cancelRename(teamId: string): void {
  const next = { ...renaming.value }
  delete next[teamId]
  renaming.value = next
}

async function confirmRename(team: Team): Promise<void> {
  const name = (renaming.value[team.id] ?? '').trim()
  if (!name || name === team.name) {
    delete renaming.value[team.id]
    return
  }
  busy.value = true
  try {
    await api.updateTeam({ ...team, name })
    delete renaming.value[team.id]
    await load()
    emit('changed')
  } catch (e) {
    rowErrors.value = { ...rowErrors.value, [team.id]: e instanceof Error ? e.message : String(e) }
  } finally {
    busy.value = false
  }
}

async function removeTeam(team: Team): Promise<void> {
  busy.value = true
  try {
    await api.deleteTeam(team.id)
    rowErrors.value = { ...rowErrors.value }
    delete rowErrors.value[team.id]
    await load()
    emit('changed')
  } catch (e) {
    rowErrors.value = { ...rowErrors.value, [team.id]: e instanceof Error ? e.message : String(e) }
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <Modal
    :open="open"
    width="480px"
    :title="t('team.title')"
    dialog-class="team-dialog"
    @update:open="emit('update:open', $event)"
  >
    <p class="team-sub">{{ t('team.subtitle') }}</p>

    <ul class="team-list">
      <li v-for="team in teams" :key="team.id" class="team-row">
        <span class="team-row-icon"><Icon name="users" :size="14" /></span>
        <template v-if="renaming[team.id] !== undefined">
          <input
            v-model="renaming[team.id]"
            class="miyo-input team-rename-input"
            :maxlength="NAME_MAX"
            @keyup.enter="confirmRename(team)"
            @keyup.escape="cancelRename(team.id)"
          />
          <button class="team-btn" type="button" :disabled="busy" @click="confirmRename(team)">
            <Icon name="check" :size="14" />
          </button>
          <button class="team-btn" type="button" :disabled="busy" @click="cancelRename(team.id)">
            <Icon name="x" :size="14" />
          </button>
        </template>
        <template v-else>
          <span class="team-row-name">{{ team.name }}</span>
          <span v-if="isDefault(team)" class="team-row-tag">{{ t('team.defaultTag') }}</span>
          <div class="team-row-spacer" />
          <button
            v-if="!isDefault(team)"
            class="team-btn"
            type="button"
            :disabled="busy"
            :title="t('team.rename')"
            @click="startRename(team)"
          >
            <Icon name="pencil" :size="13" />
          </button>
          <button
            v-if="!isDefault(team)"
            class="team-btn danger"
            type="button"
            :disabled="busy"
            :title="t('team.delete')"
            @click="removeTeam(team)"
          >
            <Icon name="trash" :size="13" />
          </button>
        </template>
        <p v-if="rowErrors[team.id]" class="team-row-error">{{ rowErrors[team.id] }}</p>
      </li>
    </ul>

    <div class="team-create">
      <input
        v-model="newTeamName"
        class="miyo-input"
        :placeholder="t('team.newPh')"
        :maxlength="NAME_MAX"
        @keyup.enter="createTeam"
      />
      <button class="miyo-btn miyo-btn-primary" type="button" :disabled="busy || !newTeamName.trim()" @click="createTeam">
        <Icon name="plus" :size="14" /> {{ t('team.create') }}
      </button>
    </div>

    <template #footer>
      <div class="team-actions">
        <div class="team-row-spacer" />
        <button class="miyo-btn" type="button" @click="emit('update:open', false)">
          {{ t('common.close') }}
        </button>
      </div>
    </template>
  </Modal>
</template>

<style scoped>
.team-sub {
  margin: 0 0 12px;
  font-size: 12.5px;
  color: var(--text-3);
}
.team-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.team-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.team-row-icon {
  display: inline-flex;
  color: var(--text-3);
}
.team-row-name {
  font-size: 13px;
  color: var(--text-1);
}
.team-row-tag {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 1px 6px;
}
.team-row-spacer {
  flex: 1;
}
.team-row-error {
  flex-basis: 100%;
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--danger, #e5484d);
}
.team-rename-input {
  flex: 1;
}
.team-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
}
.team-btn:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.team-btn.danger:hover {
  color: var(--danger, #e5484d);
}
.team-create {
  display: flex;
  gap: 8px;
  margin-top: 14px;
}
.team-create .miyo-input {
  flex: 1;
}
.team-actions {
  display: flex;
  gap: 8px;
}
</style>
