<script setup lang="ts">
/**
 * SyncImportDialog：从团队同步远端导入「本地没有」的项目（首页入口）。
 *
 * 交互：
 * - 打开即拉取该团队远端清单（无远端 → 提示先在项目内配置）；
 * - 远端项目列表：名称 / 接口数 / 远端更新时间；本地已有同 id 项目置灰提示；
 * - 导入成功 emit 'imported'，父层刷新项目列表。
 */
import { ref, watch } from 'vue'
import { useLocaleStore } from '../../stores/locale'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import type { RemoteProjectEntry } from '../../types/miyoApi'
import Icon from '../ui/Icon.vue'
import Modal from '../ui/Modal.vue'

const props = defineProps<{
  open: boolean
  /** 团队 id（远端绑定维度）。 */
  teamId: string
  teamName: string
}>()
const emit = defineEmits<{
  'update:open': [open: boolean]
  /** 有项目导入成功（父层刷新列表）。 */
  imported: []
}>()

const t = useLocaleStore().t
const api = useMiyoApi()
const toast = useToast()

const remotesOk = ref(false)
const entries = ref<RemoteProjectEntry[]>([])
const loading = ref(false)
const busyId = ref<string | null>(null)

// 弹窗由 v-if 控制：挂载时 open 已为 true，watch 必须 immediate。
watch(
  () => props.open,
  (open) => {
    if (!open) return
    void load()
  },
  { immediate: true },
)

async function load(): Promise<void> {
  loading.value = true
  remotesOk.value = false
  entries.value = []
  try {
    const remotes = await api.syncRemoteList(props.teamId)
    const remote = remotes[0]
    if (!remote) return
    remotesOk.value = true
    entries.value = await api.syncRemoteListProjects(remote.id)
  } catch (err) {
    toast.error(t('simport.loadFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    loading.value = false
  }
}

async function importProject(entry: RemoteProjectEntry): Promise<void> {
  busyId.value = entry.project_id
  try {
    const remotes = await api.syncRemoteList(props.teamId)
    const remote = remotes[0]
    if (!remote) return
    const result = await api.syncImportProject(remote.id, entry.project_id)
    toast.success(t('simport.importOk', { name: result.name }), {
      message: t('simport.importOkDetail', { n: result.endpoints }),
    })
    emit('imported')
    await load()
  } catch (err) {
    toast.error(t('simport.importFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busyId.value = null
  }
}

function shortTime(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso.slice(0, 16)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
</script>

<template>
  <Modal
    :open="open"
    :title="t('simport.title', { team: teamName })"
    width="560px"
    @update:open="emit('update:open', $event)"
  >
    <p class="simport-sub">{{ t('simport.subtitle') }}</p>

    <p v-if="loading" class="simport-hint">{{ t('common.loading') }}</p>

    <template v-else>
      <!-- 无远端 -->
      <p v-if="!remotesOk" class="simport-hint">{{ t('simport.noRemote') }}</p>

      <!-- 有远端但无快照 -->
      <p v-else-if="!entries.length" class="simport-hint">{{ t('simport.empty') }}</p>

      <ul v-else class="simport-list">
        <li v-for="e in entries" :key="e.project_id" class="simport-item">
          <span class="simport-icon"><Icon name="folder" :size="14" /></span>
          <div class="simport-meta">
            <span class="simport-name" :title="e.name">{{ e.name }}</span>
            <span class="simport-sub2">
              {{ t('simport.endpoints', { n: e.endpoint_count }) }} ·
              {{ t('simport.updatedAt', { time: shortTime(e.updated_at) }) }}
            </span>
          </div>
          <span v-if="e.exists_locally" class="simport-exists">{{ t('simport.exists') }}</span>
          <button
            v-else
            class="simport-import"
            type="button"
            :disabled="busyId === e.project_id"
            @click="importProject(e)"
          >
            <Icon name="download" :size="12" />
            {{ busyId === e.project_id ? t('common.loading') : t('simport.importBtn') }}
          </button>
        </li>
      </ul>
    </template>
  </Modal>
</template>

<style scoped>
.simport-sub {
  margin: 0 0 12px;
  color: var(--text-2);
  font-size: 12px;
  line-height: 1.6;
}
.simport-hint {
  margin: 0;
  padding: 24px 0;
  color: var(--text-3);
  font-size: 12px;
  text-align: center;
}
.simport-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 360px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.simport-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
}
.simport-icon {
  display: inline-flex;
  color: var(--text-3);
  flex-shrink: 0;
}
.simport-meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}
.simport-name {
  color: var(--text-1);
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.simport-sub2 {
  color: var(--text-3);
  font-size: var(--fs-xxs);
}
.simport-exists {
  color: var(--text-3);
  font-size: var(--fs-xxs);
  flex-shrink: 0;
}
.simport-import {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  padding: 4px 10px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-1);
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}
.simport-import:hover:not(:disabled) {
  background: var(--bg-hover);
  border-color: var(--accent);
  color: var(--accent);
}
.simport-import:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
