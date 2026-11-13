<script setup lang="ts">
/**
 * CodePanel：生成代码面板（请求 Tab 的 Code 标签页）。
 * 从 ToolsDrawer 提取为独立面板：选择语言 → 生成 → 复制，输出为只读代码预览。
 * 入参为 codegen 原始输入（method/url/headers/body/auth），不耦合 Endpoint——
 * 调试页与测试用例抽屉（用例快照还原 BodySpec）均可复用。
 */
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import CustomSelect from './ui/CustomSelect.vue'
import Icon from './ui/Icon.vue'
import type { AuthSpec, BodySpec, CodeLang, HttpMethod, KeyValue } from '../types/miyoApi'
import { errorText } from '../utils/errorText'

const props = defineProps<{
  method: HttpMethod
  url: string
  headers: KeyValue[]
  body: BodySpec
  auth: AuthSpec
  /** 打开即按当前语言自动生成，切语言时自动重新生成（导出弹窗用）。 */
  autoGenerate?: boolean
}>()

/** 卸载后丢弃异步结果：防止关闭弹窗后仍在写状态 / 弹 Toast。 */
let disposed = false
onBeforeUnmount(() => {
  disposed = true
})

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const CODE_LANGS: Array<{ value: CodeLang; label: string }> = [
  { value: 'curl', label: 'cURL' },
  { value: 'python', label: 'Python (requests)' },
  { value: 'js', label: 'JavaScript (fetch)' },
  { value: 'go', label: 'Go (net/http)' },
  { value: 'java', label: 'Java (OkHttp)' },
  { value: 'php', label: 'PHP (cURL)' },
  { value: 'rust', label: 'Rust (reqwest)' },
]

const codeLang = ref<CodeLang>('curl')
const generatedCode = ref<string | null>(null)
const generating = ref(false)

async function generateCode(): Promise<void> {
  generating.value = true
  try {
    const code = await api.codegenRender({
      lang: codeLang.value,
      method: props.method,
      url: props.url,
      headers: props.headers,
      body: props.body,
      auth: props.auth,
    })
    if (disposed) return
    generatedCode.value = code
  } catch (err) {
    if (disposed) return
    toast.error(t('codegen.genFail'), { message: errorText(err) })
  } finally {
    if (!disposed) generating.value = false
  }
}

async function copyCode(): Promise<void> {
  if (!generatedCode.value) return
  try {
    await navigator.clipboard.writeText(generatedCode.value)
    toast.success(t('codegen.copiedClipboard'))
  } catch {
    toast.error(t('response.copyFail'))
  }
}

onMounted(() => {
  if (props.autoGenerate) void generateCode()
})

watch(codeLang, () => {
  if (props.autoGenerate) void generateCode()
})
</script>

<template>
  <div class="panel">
    <div class="cp-row">
      <CustomSelect
        :model-value="codeLang"
        :options="CODE_LANGS"
        size="sm"
        class="cp-lang-select"
        @update:model-value="codeLang = $event as CodeLang"
      />
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="generating" @click="generateCode">
        <Icon name="code" :size="13" />
        {{ generating ? t('codegen.generating') : t('codegen.generate') }}
      </button>
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="!generatedCode" @click="copyCode">
        <Icon name="copy" :size="13" /> {{ t('common.copy') }}
      </button>
    </div>
    <pre v-if="generatedCode" class="cp-preview">{{ generatedCode }}</pre>
    <p v-else class="cp-empty">{{ t('codegen.emptyHint') }}</p>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.cp-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.cp-lang-select {
  width: 200px;
}

.cp-preview {
  margin: 0;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 420px;
  overflow-y: auto;
}

.cp-empty {
  margin: 0;
  padding: 14px 16px;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius);
  font-size: 12px;
  color: var(--text-3);
}
</style>
