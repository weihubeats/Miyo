<script setup lang="ts">
/**
 * CodeImportDialog：多语言代码导入弹窗。
 * 粘贴 cURL / Java / Python / JavaScript / Go 客户端代码 → 解析预览 → 导入为新草稿。
 * cURL 走后端解析器（parse_curl_command），其余语言走前端启发式解析
 * （utils/codeImport.ts）；语言默认自动检测，可手动指定。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useWorkspaceStore } from '../stores/workspace'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import Modal from './ui/Modal.vue'
import CustomSelect from './ui/CustomSelect.vue'
import { SNIPPET_LANGS, detectLang, parseCodeSnippet } from '../utils/codeImport'
import type { SnippetLang } from '../utils/codeImport'
import type { CurlParsed } from '../types/miyoApi'
import { errorText } from '../utils/errorText'

const props = defineProps<{ folderId: string | null }>()
const emit = defineEmits<{ close: [] }>()

const api = useMiyoApi()
const store = useWorkspaceStore()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const command = ref('')
/** auto = 自动检测。 */
const lang = ref<string>('auto')
const parsing = ref(false)
const error = ref<string | null>(null)
const parsed = ref<CurlParsed | null>(null)
/** 最近一次解析使用的语言（预览展示用）。 */
const resolvedLang = ref('')

/** 「自动检测」文案走字典；语言名（cURL / Java…）为品牌名保持原文。 */
const placeholder = computed(() => t('codedlg.placeholder'))
const langOptions = computed(() =>
  SNIPPET_LANGS.map((l) => (l.value === 'auto' ? { ...l, label: t('codeimport.langAuto') } : l)),
)

/** 归一化 shell 续行（反斜杠换行 → 空格）。 */
function normalize(cmd: string): string {
  return cmd.replace(/\\[ \t]*\r?\n/g, ' ')
}

async function parse(): Promise<void> {
  const trimmed = command.value.trim()
  if (!trimmed) return
  const target: SnippetLang | null = lang.value === 'auto' ? detectLang(trimmed) : (lang.value as SnippetLang)
  if (!target) {
    error.value = t('codedlg.langUndetected')
    parsed.value = null
    return
  }
  parsing.value = true
  error.value = null
  try {
    if (target === 'curl') {
      parsed.value = await api.parseCurlCommand(normalize(trimmed))
    } else {
      parsed.value = parseCodeSnippet(target, trimmed)
    }
    resolvedLang.value = target
  } catch (err) {
    error.value = errorText(err)
    parsed.value = null
  } finally {
    parsing.value = false
  }
}

/** 输入即自动解析（防抖 400ms）。 */
let parseTimer: number | undefined
watch(command, () => {
  window.clearTimeout(parseTimer)
  if (!command.value.trim()) {
    parsed.value = null
    error.value = null
    return
  }
  parseTimer = window.setTimeout(() => void parse(), 400)
})

/** 手动切换语言时立即重解析。 */
watch(lang, () => {
  if (command.value.trim()) void parse()
})

onBeforeUnmount(() => {
  window.clearTimeout(parseTimer)
})

/** Body 预览：raw 仅在部分模式下存在，统一收敛为文本。 */
const bodyPreview = computed(() => {
  const body = parsed.value?.body
  if (!body) return null
  if ('raw' in body) return `${body.mode}: ${body.raw}`
  if ('fields' in body) return `${body.mode}: ${t('codedlg.fieldsCount', { n: body.fields.length })}`
  if ('path' in body) return `binary: ${body.path}`
  return body.mode
})

const LANG_LABELS: Record<string, string> = Object.fromEntries(
  SNIPPET_LANGS.map((l) => [l.value, l.label]),
)

function importToEditor(): void {
  if (!parsed.value) return
  store.openCurlDraft(parsed.value, props.folderId)
  toast.success(t('curldlg.imported'))
  emit('close')
}
</script>

<template>
  <Modal :open="true" :title="t('codedlg.title')" width="620px" @close="emit('close')">
    <p class="modal-hint">
      {{ t('codedlg.hint') }}
    </p>
    <div class="lang-row">
      <span class="lang-label">{{ t('codedlg.lang') }}</span>
      <CustomSelect
        v-model="lang"
        :options="langOptions"
        size="sm"
        class="lang-select"
      />
    </div>
    <textarea
      v-model="command"
      class="miyo-input code-input"
      spellcheck="false"
      :placeholder="placeholder"
    ></textarea>
    <div class="modal-actions">
      <button class="miyo-btn" type="button" :disabled="parsing || !command.trim()" @click="parse">
        {{ parsing ? t('importdlg.parsing') : t('codedlg.reparse') }}
      </button>
    </div>

    <p v-if="error" class="import-error">{{ error }}</p>

    <div v-if="parsed" class="preview">
      <div class="preview-row">
        <span class="preview-method">{{ parsed.method }}</span>
        <span class="preview-url">{{ parsed.url }}</span>
        <span class="preview-lang">{{ LANG_LABELS[resolvedLang] ?? resolvedLang }}</span>
      </div>
      <div class="preview-row">
        <span class="preview-label">{{ t('curldlg.headers') }}</span>
        <span>{{ t('curldlg.count', { n: parsed.headers.length }) }}</span>
      </div>
      <div class="preview-row" v-if="parsed.body">
        <span class="preview-label">{{ t('curldlg.body') }}</span>
        <pre class="preview-body">{{ bodyPreview }}</pre>
      </div>
      <div class="preview-row" v-if="parsed.auth.type !== 'none'">
        <span class="preview-label">{{ t('curldlg.auth') }}</span>
        <span>{{ parsed.auth.type }}</span>
      </div>
    </div>

    <template #footer>
      <button class="miyo-btn" type="button" @click="emit('close')">{{ t('common.cancel') }}</button>
      <button
        class="miyo-btn miyo-btn-primary"
        type="button"
        :disabled="!parsed"
        @click="importToEditor"
      >
        {{ t('curldlg.importBtn') }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.modal-hint {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--text-2);
}

.lang-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}

.lang-label {
  font-size: 12px;
  color: var(--text-3);
}

.lang-select {
  width: 240px;
}

.code-input {
  width: 100%;
  min-height: 120px;
  margin-top: 10px;
  font-family: var(--font-mono);
  font-size: 12px;
  resize: vertical;
}

.modal-actions {
  display: flex;
  gap: 8px;
  margin-top: 10px;
}

.import-error {
  margin: 10px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: var(--danger-tint);
  border: 1px solid var(--danger-tint);
  color: var(--danger);
  font-size: 12px;
}

.preview {
  margin-top: 14px;
  padding: 12px;
  border-radius: var(--radius);
  background: var(--bg-card);
  border: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.preview-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12.5px;
  min-width: 0;
}

.preview-method {
  flex-shrink: 0;
  font-weight: 700;
  font-size: var(--fs-xxs);
  padding: 2px 7px;
  border-radius: 4px;
  background: var(--info-tint);
  color: var(--info);
}

.preview-url {
  font-family: var(--font-mono);
  word-break: break-all;
  min-width: 0;
}

.preview-lang {
  flex-shrink: 0;
  margin-left: auto;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.preview-label {
  color: var(--text-3);
  width: 56px;
  flex-shrink: 0;
}

.preview-body {
  margin: 0;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
}
</style>
