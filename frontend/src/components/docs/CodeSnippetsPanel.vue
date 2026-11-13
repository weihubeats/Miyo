<script setup lang="ts">
/**
 * CodeSnippetsPanel：文档预览右栏的「多语言代码生成」面板。
 *
 * - Tab 切换语言：cURL / JavaScript / Java / Go / Rust；
 * - 打开即生成，切换语言 / 接口 / URL 变化时自动重新生成；
 * - 右上角一键复制（Tauri 原生剪贴板降级链）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../ui/Icon.vue'
import Tabs from '../ui/Tabs.vue'
import type { TabItem } from '../ui/Tabs.vue'
import { useLocaleStore } from '../../stores/locale'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { copyText } from '../../utils/clipboard'
import { dedupeJsonKeys } from '../../utils/jsonFormat'
import { highlightCode } from '../../utils/highlight'
import type { BodySpec, CodeLang, Endpoint } from '../../types/miyoApi'
import { errorText } from '../../utils/errorText'

const props = defineProps<{
  draft: Endpoint
  url: string
}>()

/** 卸载后丢弃异步结果：防止切换视图后仍在写状态 / 弹 Toast。 */
let disposed = false
onBeforeUnmount(() => {
  disposed = true
})

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

/** 文档页代码语言选单（对应后端 Lang）。 */
const LANG_TABS: Array<{ value: CodeLang; label: string }> = [
  { value: 'curl', label: 'cURL' },
  { value: 'js', label: 'JavaScript' },
  { value: 'java', label: 'Java' },
  { value: 'go', label: 'Go' },
  { value: 'rust', label: 'Rust' },
]

const langTabs: TabItem[] = LANG_TABS.map((l) => ({ key: l.value, label: l.label }))

const lang = ref<CodeLang>('curl')
const code = ref('')
const generating = ref(false)

/** 按当前语言着色后的 HTML（内容已经 escapeHtml，v-html 安全）。 */
const codeHtml = computed(() => (code.value ? highlightCode(lang.value, code.value) : ''))

/**
 * 生成用 Body：JSON 模式先折叠重复键（历史数据可能出现同一字段多次，
 * 导致 cURL / JS / Java 代码里 "body": … 重复 N 次），其余模式原样。
 */
function bodyForCodegen(): BodySpec {
  const body = props.draft.request.body
  if (body.mode !== 'json') return body
  const raw = dedupeJsonKeys(body.raw)
  return raw === null || raw === body.raw ? body : { ...body, raw }
}

async function generate(): Promise<void> {
  generating.value = true
  try {
    const out = await api.codegenRender({
      lang: lang.value,
      method: props.draft.method,
      url: props.url,
      headers: props.draft.request.headers,
      body: bodyForCodegen(),
      auth: props.draft.request.auth,
    })
    if (disposed) return
    code.value = out
  } catch (err) {
    if (disposed) return
    code.value = ''
    toast.error(t('codegen.genFail'), { message: errorText(err) })
  } finally {
    if (!disposed) generating.value = false
  }
}

async function copyCode(): Promise<void> {
  if (!code.value) return
  const ok = await copyText(code.value)
  if (ok) {
    toast.success(t('codegen.copiedClipboard'))
  } else {
    toast.error(t('response.copyFail'))
  }
}

onMounted(() => {
  void generate()
})

watch(
  () => [lang.value, props.draft.id, props.url, props.draft.request.body] as const,
  () => {
    if (!disposed) void generate()
  },
)
</script>

<template>
  <section class="csp doc-card">
    <header class="csp-head">
      <h4 class="doc-sec-title">{{ t('docs.secCode') }}</h4>
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="!code" @click="copyCode">
        <Icon name="copy" :size="12" /> {{ t('snippets.copyCode') }}
      </button>
    </header>
    <Tabs v-model="lang" :tabs="langTabs" size="sm" class="csp-tabs" />
    <div class="csp-body">
      <pre v-if="generating" class="csp-hint">{{ t('codegen.generating') }}</pre>
      <pre v-else-if="code" class="csp-code" v-html="codeHtml"></pre>
      <p v-else class="csp-hint">{{ t('snippets.genEmpty') }}</p>
    </div>
  </section>
</template>

<style scoped>
.csp {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.csp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.csp-body {
  /* 代码井：底随主题（--bg-code），描边走 --border（原 #0a0a0a 写死深底浅色下黑洞） */
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-code);
  max-height: 320px;
  overflow: auto;
}

.csp-code,
.csp-hint {
  margin: 0;
  padding: 12px;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
}

.csp-hint {
  color: var(--text-3);
}

/* ---- 语法高亮配色（v-html 内容无 scoped 属性，需 :deep） ---- */
.csp-code :deep(.hl-c) {
  color: var(--text-3);
  font-style: italic;
}
.csp-code :deep(.hl-s) {
  color: var(--success);
}
.csp-code :deep(.hl-k) {
  color: var(--accent);
  font-weight: 600;
}
.csp-code :deep(.hl-v) {
  color: var(--warning);
}
.csp-code :deep(.hl-n) {
  /* 紫档随主题（--patch 深紫 / 浅紫），原 #c084fc 浅底下近不可读 */
  color: var(--patch);
}
.csp-code :deep(.hl-p) {
  color: var(--text-2);
}
.csp-code :deep(.hl-b) {
  color: var(--danger);
}
</style>
