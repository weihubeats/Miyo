<script setup lang="ts">
/**
 * ExportSmokeDialog：冒烟测试文档导出弹窗（测试用例面板右上角入口）。
 *
 * - 范围：仅当前接口 / 整个项目（已废弃接口后端统一排除）；
 * - 格式：Markdown（.md），内容由后端 `export_smoke_docs` 生成；
 * - 流程：export_smoke_docs 生成内容 → @tauri-apps/plugin-dialog 目录选择框（NSOpenPanel）
 *   （选目录后拼接默认文件名 smoke-{项目}-{日期}.md）→ save_text_file 落盘
 *   → 成功 Toast 附「打开文件位置」（opener.revealItemInDir）。
 */
import { computed, ref } from 'vue'
import { join } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import Modal from '../ui/Modal.vue'
import Icon from '../ui/Icon.vue'
import { useWorkspaceStore } from '../../stores/workspace'
import { useLocaleStore } from '../../stores/locale'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import type { Endpoint, ExportedDoc } from '../../types/miyoApi'

const props = defineProps<{
  /** 仅导出当前接口时使用。 */
  draft: Endpoint | null
}>()

const emit = defineEmits<{ close: [] }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

/** 前端内存态运行元信息（caseId → 状态码 / 耗时），导出结果时一并传递。 */
function runResultsPayload(): Record<string, { status: number; durationMs: number }> {
  const out: Record<string, { status: number; durationMs: number }> = {}
  for (const [caseId, meta] of store.caseRunMeta) {
    out[caseId] = meta
  }
  return out
}

// ---------- 范围 ----------

type Scope = 'current' | 'project'

const scope = ref<Scope>('current')

const canExportProject = computed(() => !!store.project)
const projectName = computed(() => store.project?.name ?? '')

// ---------- 导出流程 ----------

const exporting = ref(false)
/** 是否导出用例最近一次运行结果（通过 / 失败 / 未测试）。 */
const includeResults = ref(false)
/** 导出成功后的落盘路径（供「打开文件位置」）。 */
const savedPath = ref<string | null>(null)

async function startExport(): Promise<void> {
  const projectId = store.project?.id
  if (!projectId) return
  if (scope.value === 'current' && !props.draft) return

  exporting.value = true
  savedPath.value = null
  try {
    // 1) 后端生成内容 + 建议文件名
    const doc: ExportedDoc = await api.exportSmokeDocs({
      projectId,
      endpointId: scope.value === 'current' ? props.draft!.id : null,
      includeResults: includeResults.value,
      ...(includeResults.value ? { runResults: runResultsPayload() } : {}),
    })

    // 2) 目录选择框（NSOpenPanel 目录树可正常展开下级）→ 拼接默认文件名落盘。
    const dir = await open({
      directory: true,
      title: t('exportdocs.saveDirTitle'),
    })
    if (!dir) return // 用户取消

    const path = await join(dir, doc.suggested_name)

    // 3) 写入磁盘
    await api.writeTextFile(path, doc.content)
    savedPath.value = path

    toast.success(t('exportsmoke.exported'), {
      message: path.split('/').pop() || path,
      action: {
        label: t('exportdocs.reveal'),
        run: () => {
          void revealItemInDir(path).catch(() => toast.error(t('exportdocs.revealFail')))
        },
      },
    })
    emit('close')
  } catch (err) {
    toast.error(t('workspace.exportFail'), { message: err instanceof Error ? err.message : String(err) })
  } finally {
    exporting.value = false
  }
}
</script>

<template>
  <Modal :open="true" :title="t('exportsmoke.title')" width="560px" @close="emit('close')">
    <!-- 范围 -->
    <div class="sec">
      <p class="sec-label">{{ t('exportdocs.scope') }}</p>
      <div class="scope-grid">
        <button
          type="button"
          class="scope-card"
          :class="{ active: scope === 'current' }"
          @click="scope = 'current'"
        >
          <span class="scope-title">{{ t('exportdocs.scopeCurrent') }}</span>
          <span class="scope-desc">{{ draft?.name || t('exportdocs.noEndpoint') }}</span>
        </button>
        <button
          type="button"
          class="scope-card"
          :class="{ active: scope === 'project', disabled: !canExportProject }"
          :disabled="!canExportProject"
          @click="canExportProject && (scope = 'project')"
        >
          <span class="scope-title">{{ t('exportdocs.scopeProject') }}</span>
          <span class="scope-desc">
            {{ canExportProject ? t('exportsmoke.scopeProjectDesc', { name: projectName }) : t('exportdocs.noProject') }}
          </span>
        </button>
      </div>
    </div>

    <!-- 格式说明 -->
    <div class="sec">
      <p class="sec-label">{{ t('exportdocs.format') }}</p>
      <div class="fmt-card">
        <Icon name="file" :size="15" class="fmt-icon" />
        <span class="fmt-body">
          <span class="fmt-title">Markdown (.md)</span>
          <span class="fmt-desc">
            {{ t('exportsmoke.fmtDesc') }}
          </span>
        </span>
      </div>
    </div>

    <!-- 运行结果选项 -->
    <label class="opt-row" for="smoke-include-results">
      <input
        id="smoke-include-results"
        v-model="includeResults"
        type="checkbox"
        class="opt-check"
      />
      <span class="opt-body">
        <span class="opt-title">{{ t('exportsmoke.includeResults') }}</span>
        <span class="opt-desc">
          {{ t('exportsmoke.includeResultsDesc') }}
        </span>
      </span>
    </label>

    <template #footer>
      <button type="button" class="miyo-btn" @click="emit('close')">{{ t('common.cancel') }}</button>
      <button type="button" class="miyo-btn miyo-btn-primary" :disabled="exporting" @click="startExport">
        <Icon name="download" :size="13" />
        {{ exporting ? t('exportdocs.generating') : t('exportdocs.start') }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.sec {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.sec + .sec {
  margin-top: 16px;
}

.sec-label {
  margin: 0;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

/* ---- 范围双卡 ---- */
.scope-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.scope-card {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-panel);
  color: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.scope-card:hover:not(.disabled) {
  border-color: var(--border-strong);
}
.scope-card.active {
  border-color: var(--accent);
  background: var(--accent-tint);
}
.scope-card.disabled {
  opacity: 0.45;
  cursor: default;
}

.scope-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-1);
}
.scope-desc {
  font-size: var(--fs-xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 格式说明 ---- */
.fmt-card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-panel);
}
.fmt-icon {
  flex-shrink: 0;
  color: var(--accent);
}
.fmt-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.fmt-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-1);
}
.fmt-desc {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

/* ---- 运行结果选项 ---- */
.opt-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-panel);
  cursor: pointer;
  transition: border-color var(--dur) var(--ease);
}
.opt-row:hover {
  border-color: var(--border-strong);
}
.opt-row:has(.opt-check:checked) {
  border-color: var(--accent);
  background: var(--accent-tint);
}

.opt-check {
  margin-top: 2px;
  accent-color: var(--accent);
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}

.opt-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.opt-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-1);
}
.opt-desc {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
