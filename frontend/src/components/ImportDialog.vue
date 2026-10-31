<script setup lang="ts">
/**
 * ImportDialog：OpenAPI/Swagger/Postman 文档导入。
 * 粘贴文本或选择文件 → 后端解析预览 → 确认后落库。
 *
 * - mode="workspace"（默认）：导入到当前激活项目（工作区内使用）；
 * - mode="new-project"：先创建新项目并激活，再写入接口（仪表板「导入项目」）。
 */
import { computed, onMounted, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import Modal from './ui/Modal.vue'
import Icon from './ui/Icon.vue'
import type { ImportedEndpoint, ImportFormat, Project } from '../types/miyoApi'

const props = withDefaults(
  defineProps<{
    /** workspace：写入当前激活项目；new-project：创建新项目承接导入。 */
    mode?: 'workspace' | 'new-project'
    /** 预填文档文本（拖拽导入：Dropzone 读入文件后带内容打开本弹窗）。 */
    initialText?: string
  }>(),
  { mode: 'workspace' },
)

const emit = defineEmits<{ close: []; imported: [project: Project] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const text = ref('')
const busy = ref(false)
const result = ref<{ format: ImportFormat; endpoints: ImportedEndpoint[] } | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)

/** new-project 模式下的目标项目名。 */
const projectName = ref('')

/**
 * 冲突处理策略（默认「跳过」= 最安全，杜绝未经确认覆盖现有数据）：
 * 冲突判定 = 同 method + path（与 store.importEndpoints 比对口径一致）。
 * 仅 workspace 模式有「现有数据」可比；new-project 起始为空项目，无冲突。
 */
const conflictStrategy = ref<'skip' | 'overwrite' | 'duplicate'>('skip')
const conflicts = computed(() => {
  if (!result.value || props.mode !== 'workspace') return []
  const byKey = new Map(store.endpoints.map((e) => [`${e.method} ${e.path}`, e] as const))
  return result.value.endpoints.filter((it) => byKey.has(`${it.method} ${it.path}`))
})

/** 格式展示名：品牌名保持原文，带说明的走字典（computed 随语言切换）。 */
const FORMAT_LABEL = computed<Record<ImportFormat, string>>(() => ({
  openapi30: 'OpenAPI 3.0',
  openapi31: t('importdlg.formatOpenapi31'),
  swagger20: 'Swagger 2.0',
  postman21: t('importdlg.formatPostman'),
  unknown: t('importdlg.formatUnknown'),
}))

async function pickFile(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  text.value = await file.text()
  parse()
}

async function parse(): Promise<void> {
  if (!text.value.trim()) return
  busy.value = true
  try {
    result.value = await api.importDocument(text.value)
    toast.success(
      t('importdlg.detected', {
        format: FORMAT_LABEL.value[result.value.format],
        n: result.value.endpoints.length,
      }),
    )
  } catch (err) {
    result.value = null
    toast.error(t('importdlg.parseFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

// 拖拽导入入口：带预填内容打开时立即解析
onMounted(() => {
  if (props.initialText) {
    text.value = props.initialText
    void parse()
  }
})

async function confirm(): Promise<void> {
  if (!result.value) return
  busy.value = true
  try {
    if (props.mode === 'new-project') {
      // 先建项目并激活（switchProject 会加载其环境/接口缓存），再写入接口
      const now = new Date().toISOString()
      const name =
        projectName.value.trim() ||
        t('importdlg.defaultProjectName', { v: new Date().toLocaleDateString() })
      const project = await api.saveProject({
        id: crypto.randomUUID(),
        name,
        description: t('importdlg.importedFrom', { v: FORMAT_LABEL.value[result.value.format] }),
        variables: {},
        created_at: now,
        updated_at: now,
      })
      await store.switchProject(project.id)
      const summary = await store.importEndpoints(result.value.endpoints, conflictStrategy.value)
      toast.success(t('importdlg.createdProject', { name: project.name, n: summary.endpoints }))
      emit('imported', project)
    } else {
      const summary = await store.importEndpoints(result.value.endpoints, conflictStrategy.value)
      toast.success(t('importdlg.imported', { n: summary.endpoints, m: summary.examples }))
      if (summary.skipped || summary.overwritten) {
        toast.info(t('importdlg.conflicts', { s: summary.skipped, o: summary.overwritten }))
      }
    }
    emit('close')
  } catch (err) {
    toast.error(t('importdlg.importFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    busy.value = false
  }
}

/** 确认按钮可用性：workspace 模式需激活项目；两种模式都要求已解析出接口。 */
const canConfirm = computed(
  () => !!result.value && !busy.value && (props.mode === 'new-project' || !!store.project),
)
</script>

<template>
  <Modal :open="true" :title="mode === 'new-project' ? t('importdlg.titleNewProject') : t('workspace.importDocFile')" width="560px" @close="emit('close')">
    <p class="import-hint">
      {{
        mode === 'new-project'
          ? t('importdlg.hintNewProject')
          : t('importdlg.hint')
      }}
    </p>

    <label v-if="mode === 'new-project'" class="import-name-row">
      <span class="import-name-label">{{ t('importdlg.nameLabel') }}</span>
      <input
        v-model="projectName"
        class="miyo-input"
        :placeholder="t('importdlg.namePh')"
        spellcheck="false"
      />
    </label>

    <textarea
      v-model="text"
      class="miyo-input import-text"
      spellcheck="false"
      :placeholder="t('importdlg.textPh')"
    ></textarea>
    <div class="import-tools">
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="busy" @click="fileInput?.click()">
        {{ t('importdlg.pickFile') }}
      </button>
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="busy || !text.trim()" @click="parse">
        {{ busy ? t('importdlg.parsing') : t('importdlg.parse') }}
      </button>
      <input ref="fileInput" type="file" accept=".json,.yaml,.yml,application/json" class="import-file" @change="pickFile" />
    </div>

    <div v-if="result" class="import-preview">
      <p class="import-hint">
        {{
          t('importdlg.preview', {
            format: FORMAT_LABEL[result.format],
            n: result.endpoints.length,
            m: result.endpoints.filter((e) => e.folder_hint).length,
          })
        }}
      </p>
      <div v-if="conflicts.length" class="import-conflict">
        <p class="import-conflict-warn">
          <Icon name="alert-triangle" :size="12" />
          {{ t('importdlg.conflictWarn', { n: conflicts.length }) }}
        </p>
        <div class="import-strategies" role="radiogroup" :aria-label="t('importdlg.strategyLabel')">
          <label class="import-strategy">
            <input v-model="conflictStrategy" type="radio" value="skip" />
            {{ t('importdlg.strategySkip') }}
          </label>
          <label class="import-strategy">
            <input v-model="conflictStrategy" type="radio" value="overwrite" />
            {{ t('importdlg.strategyOverwrite') }}
          </label>
          <label class="import-strategy">
            <input v-model="conflictStrategy" type="radio" value="duplicate" />
            {{ t('importdlg.strategyDuplicate') }}
          </label>
        </div>
      </div>
      <ul class="import-list">
        <li v-for="(ep, i) in result.endpoints.slice(0, 12)" :key="i" class="import-row">
          <span class="import-method">{{ ep.method }}</span>
          <span class="import-path">{{ ep.path }}</span>
        </li>
        <li v-if="result.endpoints.length > 12" class="import-hint">{{ t('importdlg.more', { n: result.endpoints.length - 12 }) }}</li>
      </ul>
    </div>

    <template #footer>
      <button
        v-if="result"
        class="miyo-btn miyo-btn-primary miyo-btn-sm"
        type="button"
        :disabled="!canConfirm"
        @click="confirm"
      >
        {{ mode === 'new-project' ? t('importdlg.createAndImport') : t('importdlg.confirmImport') }}
      </button>
      <button class="miyo-btn miyo-btn-sm" type="button" @click="emit('close')">{{ t('common.cancel') }}</button>
    </template>
  </Modal>
</template>

<style scoped>
.import-hint {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-2);
}

.import-name-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
}
.import-name-label {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--text-3);
}
.import-name-row .miyo-input {
  flex: 1;
  height: 30px;
}

.import-text {
  width: 100%;
  min-height: 140px;
  margin-top: 10px;
  font-family: var(--font-mono);
  font-size: 12px;
  resize: vertical;
}

.import-tools {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 8px;
}

.import-preview {
  margin-top: 12px;
}

.import-conflict {
  margin-top: 10px;
  padding: 8px 10px;
  border: 1px solid var(--warning);
  border-radius: var(--radius-md);
  background: var(--warning-tint);
}

.import-conflict-warn {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 12.5px;
  color: var(--warning);
}

.import-strategies {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  margin-top: 7px;
}

.import-strategy {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--text-2);
  cursor: pointer;
}

.import-list {
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.import-row {
  display: flex;
  gap: 10px;
  font-size: 12.5px;
}

.import-method {
  width: 56px;
  flex-shrink: 0;
  font-weight: 700;
  color: var(--text-2);
}

.import-path {
  font-family: var(--font-mono);
  color: var(--text-1);
  word-break: break-all;
}

.import-file {
  display: none;
}
</style>