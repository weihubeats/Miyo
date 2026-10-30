<script setup lang="ts">
/**
 * SyncDialog：团队同步面板（项目快照经 git / S3 / db 远端推拉）。
 *
 * 交互：
 * - 远端列表 + 新增表单（首期仅 git，S3/db 后端未实现时置灰提示）；
 * - 推送：自动脱敏凭证，结果回显被过滤的项（让用户确知凭证未离开本机）；
 * - 拉取：先预览差异（新增/删除/修改的接口清单），确认后才覆盖，
 *   覆盖前自动留档本地快照并回显路径（可回滚）。
 */
import { computed, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import type { SyncPullPreview, SyncRemote } from '../types/miyoApi'
import Icon from './ui/Icon.vue'
import Modal from './ui/Modal.vue'
import Popconfirm from './ui/Popconfirm.vue'

const props = defineProps<{
  open: boolean
  /** 当前项目（推/拉的操作对象）。 */
  projectId: string | null
  projectName: string
  /** 项目所属团队（远端绑定维度；null 视为默认团队）。 */
  teamId: string | null
}>()
const emit = defineEmits<{
  'update:open': [open: boolean]
  /** 拉取覆盖成功后触发（父层需刷新工作区数据）。 */
  applied: []
}>()

/** 默认团队固定 UUID（与迁移 0015 一致）。 */
const DEFAULT_TEAM_ID = '00000000-0000-0000-0000-000000000001'
const effectiveTeamId = computed(() => props.teamId ?? DEFAULT_TEAM_ID)

const t = useLocaleStore().t
const api = useMiyoApi()
const toast = useToast()

const remotes = ref<SyncRemote[]>([])
const loading = ref(false)
const busy = ref(false)
const preview = ref<SyncPullPreview | null>(null)
const previewRemoteId = ref<string | null>(null)
const redacted = ref<string[]>([])

// 新增表单
const showForm = ref(false)
const formKind = ref<'git' | 's3' | 'db'>('git')
const formUrl = ref('')
const formBranch = ref('main')
const formPath = ref('')
const formBucket = ref('')
const formPrefix = ref('')
const formDsnRef = ref('')
const formTable = ref('')

/** S3 / DB 远端后端未实现：表单可选可填，保存禁用并提示开发中。 */
const kindComingSoon = computed(() => formKind.value !== 'git')
const KINDS = ['git', 's3', 'db'] as const

/**
 * git 仓库地址格式：SSH（git@host:path / ssh://）与 HTTP(S) 均可，
 * 另允许本地路径（file:// 与绝对路径）方便内网/单机场景。
 */
const GIT_URL_RE = /^(git@|ssh:\/\/|https?:\/\/|file:\/\/|\/)/

const urlValid = computed(() => GIT_URL_RE.test(formUrl.value.trim()))
const urlError = computed(() =>
  formUrl.value.trim() && !urlValid.value ? t('sync.urlInvalid') : null,
)

const activeRemote = computed(() => remotes.value[0] ?? null)

async function loadRemotes(): Promise<void> {
  loading.value = true
  try {
    remotes.value = await api.syncRemoteList(effectiveTeamId.value)
    // 快照目录按团队约定预填（仓库内按项目分文件存放）。
    if (!formPath.value) {
      formPath.value = 'projects'
    }
  } catch (err) {
    toast.error(t('sync.loadFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    loading.value = false
  }
}

// 弹窗由 v-if 控制：每次打开都是新挂载（open 初始即 true），
// 必须 immediate 否则 loadRemotes 永不触发。
watch(
  () => props.open,
  (open) => {
    if (!open) return
    preview.value = null
    redacted.value = []
    showForm.value = false
    void loadRemotes()
  },
  { immediate: true },
)

async function createRemote(): Promise<void> {
  if (kindComingSoon.value || !urlValid.value) return
  busy.value = true
  try {
    await api.syncRemoteCreate(effectiveTeamId.value, {
      kind: 'git',
      url: formUrl.value.trim(),
      branch: formBranch.value.trim() || 'main',
      path: formPath.value.trim() || 'projects',
    })
    showForm.value = false
    formUrl.value = ''
    toast.success(t('sync.remoteAdded'))
    await loadRemotes()
  } catch (err) {
    toast.error(t('sync.remoteAddFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busy.value = false
  }
}

async function removeRemote(remote: SyncRemote): Promise<void> {
  busy.value = true
  try {
    await api.syncRemoteDelete(remote.id)
    if (previewRemoteId.value === remote.id) preview.value = null
    toast.success(t('sync.remoteRemoved'))
    await loadRemotes()
  } catch (err) {
    toast.error(t('sync.remoteRemoveFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busy.value = false
  }
}

async function push(): Promise<void> {
  if (!activeRemote.value || !props.projectId) return
  busy.value = true
  redacted.value = []
  try {
    const result = await api.syncPush(activeRemote.value.id, props.projectId)
    redacted.value = result.redacted
    if (result.noop) {
      toast.info(t('sync.pushNoop'))
    } else {
      toast.success(t('sync.pushOk'), { message: result.revision.slice(0, 12) })
    }
    await loadRemotes()
  } catch (err) {
    toast.error(t('sync.pushFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busy.value = false
  }
}

async function loadPreview(): Promise<void> {
  if (!activeRemote.value || !props.projectId) return
  busy.value = true
  try {
    preview.value = await api.syncPullPreview(activeRemote.value.id, props.projectId)
    previewRemoteId.value = activeRemote.value.id
  } catch (err) {
    toast.error(t('sync.previewFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busy.value = false
  }
}

async function applyPull(): Promise<void> {
  if (!activeRemote.value || !props.projectId) return
  busy.value = true
  try {
    const result = await api.syncPullApply(activeRemote.value.id, props.projectId)
    toast.success(t('sync.pullOk'), {
      message: result.local_backup
        ? t('sync.pullBackupAt', { path: result.local_backup.split('/').pop() || '' })
        : undefined,
    })
    preview.value = null
    await loadRemotes()
    // 覆盖已改 DB：通知父层重载工作区，避免界面仍显示旧数据。
    emit('applied')
  } catch (err) {
    toast.error(t('sync.pullFail'), {
      message: err instanceof Error ? err.message : String(err),
    })
  } finally {
    busy.value = false
  }
}

const diffTotal = computed(() => {
  if (!preview.value) return 0
  const d = preview.value.diff
  return d.added.length + d.removed.length + d.modified.length
})

function shortRev(rev: string | null): string {
  return rev ? rev.slice(0, 12) : t('sync.never')
}
</script>

<template>
  <Modal
    :open="open"
    width="560px"
    :title="t('sync.title')"
    dialog-class="sync-dialog"
    @update:open="emit('update:open', $event)"
  >
    <p v-if="!projectId" class="sync-empty">{{ t('sync.noProject') }}</p>
    <template v-else>
      <p class="sync-sub">{{ t('sync.subtitle', { project: projectName }) }}</p>

    <!-- 远端列表 -->
    <div v-if="loading" class="sync-empty">{{ t('sync.loading') }}</div>
    <div v-else-if="remotes.length === 0" class="sync-empty">
      {{ t('sync.noRemote') }}
    </div>
    <ul v-else class="sync-list">
      <li v-for="remote in remotes" :key="remote.id" class="sync-item">
        <span class="sync-kind">{{ remote.kind }}</span>
        <div class="sync-info">
          <p class="sync-url">
            {{ remote.config.kind === 'git' ? remote.config.url : JSON.stringify(remote.config) }}
          </p>
          <p class="sync-meta">
            {{
              remote.config.kind === 'git'
                ? t('sync.branchPath', { branch: remote.config.branch, path: remote.config.path })
                : ''
            }}
          </p>
          <p class="sync-meta">{{ t('sync.lastRev') }}: {{ shortRev(remote.last_synced_revision) }}</p>
        </div>
        <Popconfirm
          :title="t('sync.removeConfirm')"
          :description="t('sync.removeConfirmDesc')"
          :confirm-text="t('sync.removeConfirmOk')"
          @confirm="removeRemote(remote)"
        >
          <button class="sync-del" type="button" :disabled="busy" :title="t('sync.remoteRemove')">
            <Icon name="trash" :size="14" />
          </button>
        </Popconfirm>
      </li>
    </ul>

    <!-- 新增表单 -->
    <div v-if="showForm" class="sync-form">
      <div class="sync-kinds" role="tablist">
        <button
          v-for="k in KINDS"
          :key="k"
          class="sync-kind-tab"
          :class="{ active: formKind === k }"
          type="button"
          @click="formKind = k"
        >
          {{ k }}
          <span v-if="k !== 'git'" class="sync-soon">{{ t('sync.comingSoon') }}</span>
        </button>
      </div>

      <!-- git -->
      <template v-if="formKind === 'git'">
        <label class="sync-field">
          <span>{{ t('sync.remoteUrl') }}</span>
          <input
            v-model="formUrl"
            :placeholder="t('sync.phUrl')"
            :class="{ invalid: !!urlError }"
          />
        </label>
        <p v-if="urlError" class="sync-hint error">{{ urlError }}</p>
        <p class="sync-hint">{{ t('sync.urlHint') }}</p>
        <div class="sync-row">
          <label class="sync-field">
            <span>{{ t('sync.branch') }}</span>
            <input v-model="formBranch" :placeholder="t('sync.phBranch')" />
          </label>
          <label class="sync-field">
            <span>{{ t('sync.path') }}</span>
            <input v-model="formPath" :placeholder="t('sync.phPath')" />
          </label>
        </div>
      </template>

      <!-- s3 -->
      <template v-else-if="formKind === 's3'">
        <label class="sync-field">
          <span>{{ t('sync.bucket') }}</span>
          <input v-model="formBucket" disabled :placeholder="t('sync.phBucket')" />
        </label>
        <label class="sync-field">
          <span>{{ t('sync.prefix') }}</span>
          <input v-model="formPrefix" disabled :placeholder="t('sync.phPrefix')" />
        </label>
      </template>

      <!-- db -->
      <template v-else>
        <label class="sync-field">
          <span>{{ t('sync.dsnRef') }}</span>
          <input v-model="formDsnRef" disabled :placeholder="t('sync.phDsnRef')" />
        </label>
        <label class="sync-field">
          <span>{{ t('sync.table') }}</span>
          <input v-model="formTable" disabled :placeholder="t('sync.phTable')" />
        </label>
      </template>

      <p v-if="kindComingSoon" class="sync-hint soon">{{ t('sync.comingSoonHint') }}</p>
    </div>

    <!-- 拉取预览 -->
    <div v-if="preview" class="sync-preview">
      <p class="sync-preview-title">
        {{ t('sync.previewTitle') }}
        <span class="sync-preview-rev">{{ preview.revision.slice(0, 12) }}</span>
      </p>
      <p v-if="diffTotal === 0" class="sync-hint">{{ t('sync.diffNone') }}</p>
      <ul v-else class="sync-diff">
        <li v-for="p in preview.diff.added" :key="`a-${p}`" class="diff-add">+ {{ p }}</li>
        <li v-for="p in preview.diff.modified" :key="`m-${p}`" class="diff-mod">~ {{ p }}</li>
        <li v-for="p in preview.diff.removed" :key="`r-${p}`" class="diff-del">- {{ p }}</li>
      </ul>
      <p class="sync-hint">{{ t('sync.overwriteHint') }}</p>
    </div>

    <!-- 脱敏回显 -->
      <div v-if="redacted.length" class="sync-redact">
        <p class="sync-redact-title">{{ t('sync.redactedTitle', { n: redacted.length }) }}</p>
        <ul>
          <li v-for="item in redacted" :key="item">{{ item }}</li>
        </ul>
      </div>
    </template>

    <template #footer>
      <div v-if="!projectId" class="sync-actions">
        <div class="sync-spacer" />
        <button class="sync-btn" type="button" @click="emit('update:open', false)">
          {{ t('common.close') }}
        </button>
      </div>
      <div v-else class="sync-actions">
        <button class="sync-btn ghost" type="button" :disabled="busy" @click="showForm = !showForm">
          <Icon name="plus" :size="14" /> {{ t('sync.addRemote') }}
        </button>
        <button v-if="showForm" class="sync-btn primary" type="button" :disabled="busy || kindComingSoon || !urlValid" @click="createRemote">
          {{ t('common.save') }}
        </button>
        <div class="sync-spacer" />
        <button class="sync-btn" type="button" :disabled="busy || !activeRemote" @click="loadPreview">
          <Icon name="download" :size="14" /> {{ t('sync.pull') }}
        </button>
        <button
          v-if="preview"
          class="sync-btn primary"
          type="button"
          :disabled="busy"
          @click="applyPull"
        >
          {{ t('sync.applyPull') }}
        </button>
        <button class="sync-btn primary" type="button" :disabled="busy || !activeRemote" @click="push">
          <Icon name="upload" :size="14" /> {{ t('sync.push') }}
        </button>
      </div>
    </template>
  </Modal>
</template>

<style scoped>
.sync-sub {
  margin: 0 0 12px;
  color: var(--text-2);
}
.sync-empty {
  padding: 16px;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-md);
  color: var(--text-3);
  text-align: center;
}
.sync-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.sync-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
}
.sync-kind {
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.sync-info {
  flex: 1;
  min-width: 0;
}
.sync-url {
  margin: 0;
  color: var(--text-1);
  font-size: 13px;
  word-break: break-all;
}
.sync-meta {
  margin: 2px 0 0;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}
.sync-del {
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  padding: 4px;
  border-radius: var(--radius-sm);
}
.sync-del:hover {
  color: var(--danger, #e5484d);
  background: var(--bg-elevated);
}
.sync-form {
  margin-top: 12px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.sync-kinds {
  display: flex;
  gap: 6px;
  margin-bottom: 10px;
}
.sync-kind-tab {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-size: 12px;
  text-transform: uppercase;
  cursor: pointer;
}
.sync-kind-tab.active {
  background: var(--bg-elevated);
  color: var(--text-1);
  border-color: var(--accent, #6e56cf);
}
.sync-soon {
  padding: 1px 5px;
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-3);
  font-size: var(--fs-xxs);
  text-transform: none;
}
.sync-hint.soon {
  color: var(--warning, #f5a623);
}
.sync-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}
.sync-field span {
  color: var(--text-3);
  font-size: var(--fs-xxs);
}
.sync-field input {
  padding: 6px 8px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-1);
  font-size: 13px;
}
.sync-field input.invalid {
  border-color: var(--danger, #e5484d);
}
.sync-hint.error {
  margin-top: 4px;
  color: var(--danger, #e5484d);
}
.sync-row {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
.sync-hint {
  margin: 8px 0 0;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}
.sync-preview,
.sync-redact {
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-hover);
}
.sync-preview-title,
.sync-redact-title {
  margin: 0 0 6px;
  color: var(--text-1);
  font-size: 12px;
  font-weight: 600;
}
.sync-preview-rev {
  margin-left: 6px;
  color: var(--text-3);
  font-family: var(--font-mono, monospace);
}
.sync-diff {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 160px;
  overflow-y: auto;
  font-family: var(--font-mono, monospace);
  font-size: 12px;
}
.diff-add {
  color: var(--success, #30a46c);
}
.diff-mod {
  color: var(--warning, #f5a623);
}
.diff-del {
  color: var(--danger, #e5484d);
}
.sync-redact ul {
  margin: 0;
  padding-left: 16px;
  color: var(--text-2);
  font-size: 12px;
}
.sync-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}
.sync-spacer {
  flex: 1;
}
.sync-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  color: var(--text-1);
  font-size: 13px;
  cursor: pointer;
}
.sync-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.sync-btn.primary {
  background: var(--accent, #6e56cf);
  border-color: transparent;
  color: #fff;
}
.sync-btn.ghost {
  background: transparent;
}
</style>