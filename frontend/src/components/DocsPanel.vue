<script setup lang="ts">
/**
 * DocsPanel：文档预览（API 文档级渲染当前接口定义）。
 *
 * - 双栏布局：左栏（60%）基本信息 / 认证 / 请求头 / 请求与响应 Schema；
 *   右栏（40%）响应示例（CodeMirror 只读高亮）+ 多语言代码生成；
 * - 头部 Endpoint 模块：Method 胶囊 + 等宽 Path + 复制 Path / 跳转调试；
 * - 请求 / 响应参数用树状表格（SchemaTreeTable）展示，
 *   Schema 从请求 Body 与 2xx 响应示例的 JSON 样本推断（不改动草稿）。
 */
import { computed, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { useToast } from '../composables/useToast'
import { copyText } from '../utils/clipboard'
import { inferSchema } from '../utils/schemaInfer'
import type { SchemaRow } from '../utils/schemaInfer'
import { bodyTypeLabel, bodyTypeOf } from '../utils/testCases'
import CodeSnippetsPanel from './docs/CodeSnippetsPanel.vue'
import ExportDocsDialog from './docs/ExportDocsDialog.vue'
import ResponseExamplesPanel from './docs/ResponseExamplesPanel.vue'
import SchemaTreeTable from './docs/SchemaTreeTable.vue'
import Icon from './ui/Icon.vue'
import type { AuthSpec, Endpoint, EndpointStatus } from '../types/miyoApi'

const props = withDefaults(defineProps<{ draft: Endpoint | null; url?: string }>(), { url: '' })

const store = useWorkspaceStore()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const spec = computed(() => props.draft?.request ?? null)

/** 过滤启用项（KeyValue / MultipartField 通用：有 enabled + key 即可）。 */
const enabled = <T extends { enabled: boolean; key: string }>(list: T[]): T[] =>
  list.filter((kv) => kv.enabled && kv.key.trim())

const enabledParams = computed(() => enabled(spec.value?.params ?? []))
const enabledHeaders = computed(() => enabled(spec.value?.headers ?? []))
const pathVariables = computed(() => enabled(spec.value?.path_variables ?? []))

/** 接口状态标签（与 DesignPanel 一致，复用 design.status* 键）。 */
const STATUS_KEYS: Record<EndpointStatus, string> = {
  designing: 'design.statusDesigning',
  developing: 'design.statusDeveloping',
  testing: 'design.statusTesting',
  released: 'design.statusReleased',
  deprecated: 'design.statusDeprecated',
}

// ---------- 认证 ----------

const authView = computed(() => {
  const a: AuthSpec | undefined = spec.value?.auth
  if (!a || a.type === 'none') return { label: t('body.none'), detail: t('docs.authNoneDetail') }
  switch (a.type) {
    case 'bearer':
      return { label: 'Bearer Token', detail: `Authorization: Bearer ${a.token || t('docs.unfilled')}` }
    case 'basic':
      return { label: 'Basic Auth', detail: `${a.username || t('docs.unfilledUsername')} : ${'•'.repeat(8)}` }
    case 'apikey':
      return {
        label: t('docs.authApiKey', { v: a.in === 'header' ? t('docs.authInHeader') : t('docs.authInQuery') }),
        detail: `${a.key || t('docs.unfilledKey')} = ${a.value || t('docs.unfilled')}`,
      }
    case 'oauth2':
      return { label: 'OAuth2', detail: `client_id: ${a.client_id || t('docs.unfilled')}` }
    case 'digest':
      return { label: t('docs.authDigest'), detail: `${a.username || t('docs.unfilledUsername')} : ${'•'.repeat(8)}` }
    case 'hawk':
      return { label: t('docs.authHawk'), detail: t('docs.authHawkDetail', { id: a.key_id || t('docs.unfilled') }) }
    case 'awsv4':
      return {
        label: 'AWS Signature V4',
        detail: `${a.access_key || t('docs.unfilledAk')} / ${a.region || t('docs.unfilledRegion')} / ${a.service || t('docs.unfilledService')}`,
      }
    case 'hmac':
      return {
        label: 'HMAC (AK-SK)',
        detail: t('docs.authHmacDetail', { ak: a.access_key || t('docs.unfilledAk') }),
      }
    default:
      return { label: t('body.none'), detail: '' }
  }
})

// ---------- Body ----------

/** JSON 文本 → Schema 行（解析失败返回 null，由模板回退为源码展示）。 */
function jsonSchemaOf(raw: string): SchemaRow[] | null {
  const text = raw.trim()
  if (!text) return null
  try {
    return inferSchema(JSON.parse(text))
  } catch {
    return null
  }
}

/** 表单字段展示行（urlencoded / multipart 归一化）。 */
interface FormRow {
  key: string
  value: string
  kind: string
  description: string
}

/** 请求 Body 展示模型：树状 Schema / KV 字段表 / 源码回退。 */
const bodyView = computed<
  | { kind: 'schema'; label: string; schema: SchemaRow[] }
  | { kind: 'form'; label: string; fields: FormRow[]; showKind: boolean }
  | { kind: 'raw'; label: string; content: string }
  | null
>(() => {
  const s = spec.value
  if (!s) return null
  const label = bodyTypeLabel(bodyTypeOf(s.body))
  switch (s.body.mode) {
    case 'json': {
      const schema = jsonSchemaOf(s.body.raw)
      if (schema) return { kind: 'schema', label, schema }
      return { kind: 'raw', label, content: s.body.raw || t('docs.bodyEmpty') }
    }
    case 'urlencoded':
      return {
        kind: 'form',
        label,
        showKind: false,
        fields: enabled(s.body.fields).map<FormRow>((f) => ({
          key: f.key,
          value: f.value,
          kind: t('docs.fieldText'),
          description: f.description,
        })),
      }
    case 'multipart':
      return {
        kind: 'form',
        label,
        showKind: true,
        fields: enabled(s.body.fields).map<FormRow>((f) => ({
          key: f.key,
          value: f.value_type === 'file_path' ? `@${f.value}` : f.value,
          kind: f.value_type === 'file_path' ? t('docs.fieldFile') : t('docs.fieldText'),
          description: '',
        })),
      }
    case 'graphql':
      return { kind: 'raw', label, content: s.body.spec.query || t('docs.bodyEmpty') }
    case 'text':
      return { kind: 'raw', label, content: s.body.raw || t('docs.bodyEmpty') }
    case 'binary':
      return { kind: 'raw', label, content: s.body.path || t('docs.noFileSelected') }
    case 'none':
      return null
  }
  // switch 已按 BodySpec 联合类型穷尽；此行仅为满足 lint 的显式返回。
  return null
})

// ---------- 响应示例与响应 Schema ----------

const examples = computed(() => store.examples.get(props.draft?.id ?? '') ?? [])

/** 响应 Schema：取首个可解析为 JSON 的 2xx 示例推断。 */
const responseSchema = computed(() => {
  const ok = examples.value.find((e) => e.status >= 200 && e.status < 300)
  if (!ok) return null
  return jsonSchemaOf(ok.body)
})

// ---------- 头部快捷操作 ----------

async function copyPath(): Promise<void> {
  const path = props.draft?.path
  if (!path) return
  const ok = await copyText(path)
  if (ok) {
    toast.success(t('docs.pathCopied'))
  } else {
    toast.error(t('response.copyFail'))
  }
}

function jumpToDebug(): void {
  store.setActiveView('debug')
}

// ---------- 文档导出 ----------

const showExport = ref(false)
</script>

<template>
  <div v-if="draft" class="docs">
    <!-- ---- 头部 Endpoint 模块 ---- -->
    <header class="doc-head doc-card">
      <div class="head-row">
        <span class="method-pill" :class="`m-${draft.method.toLowerCase()}`">{{ draft.method }}</span>
        <code class="head-path" v-tooltip-overflow>{{ draft.path }}</code>
        <div class="head-actions">
          <button class="head-export" type="button" @click="showExport = true">
            <Icon name="download" :size="13" /> {{ t('docs.exportDocs') }}
          </button>
          <button class="miyo-btn miyo-btn-sm" type="button" @click="copyPath">
            <Icon name="copy" :size="12" /> {{ t('docs.copyPath') }}
          </button>
          <button class="miyo-btn miyo-btn-sm miyo-btn-primary" type="button" @click="jumpToDebug">
            <Icon name="send" :size="12" /> {{ t('docs.jumpToDebug') }}
          </button>
        </div>
      </div>
      <div class="head-sub">
        <h3 class="head-name" v-tooltip-overflow>{{ draft.name || t('default.endpointName') }}</h3>
        <span class="head-status" :class="`s-${draft.status}`">
          {{ t(STATUS_KEYS[draft.status]) }}
        </span>
      </div>
      <p v-if="draft.description" class="head-desc">{{ draft.description }}</p>
    </header>

    <!-- ---- 双栏：左 60% 文档主体 / 右 40% 响应示例 + 代码生成 ---- -->
    <div class="doc-grid">
      <div class="doc-left">
        <!-- 基本信息 -->
        <section class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.secOverview') }}</h4>
          <dl class="meta-grid">
            <div class="meta-item">
              <dt>{{ t('design.endpointName') }}</dt>
              <dd>{{ draft.name || '—' }}</dd>
            </div>
            <div class="meta-item">
              <dt>{{ t('docs.currentStatus') }}</dt>
              <dd class="status-val">
                <span class="status-dot" :class="`d-${draft.status}`"></span>
                {{ t(STATUS_KEYS[draft.status]) }}
              </dd>
            </div>
            <div class="meta-item">
              <dt>{{ t('docs.timeout') }}</dt>
              <dd>{{ spec?.timeout_ms ?? '-' }} ms</dd>
            </div>
            <div class="meta-item">
              <dt>{{ t('docs.updatedAt') }}</dt>
              <dd>{{ draft.updated_at.slice(0, 16).replace('T', ' ') }}</dd>
            </div>
          </dl>
          <div v-if="pathVariables.length" class="path-vars">
            <span class="path-vars-label">{{ t('docs.pathVars') }}</span>
            <code v-for="pv in pathVariables" :key="pv.key" class="path-var">
              {{ pv.key }}={{ pv.value }}
            </code>
          </div>
        </section>

        <!-- 认证方式 -->
        <section class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.secAuth') }}</h4>
          <div class="auth-row">
            <span class="auth-badge">{{ authView.label }}</span>
            <code v-if="authView.detail" class="auth-detail">{{ authView.detail }}</code>
          </div>
        </section>

        <!-- Query 参数 -->
        <section v-if="enabledParams.length" class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.queryParams', { n: enabledParams.length }) }}</h4>
          <table class="kv-table">
            <thead>
              <tr><th>{{ t('docs.colKey') }}</th><th>{{ t('docs.colValue') }}</th><th>{{ t('docs.colDesc') }}</th></tr>
            </thead>
            <tbody>
              <tr v-for="p in enabledParams" :key="p.key">
                <td><code class="kv-key">{{ p.key }}</code></td>
                <td><code class="kv-val">{{ p.value || '—' }}</code></td>
                <td class="kv-desc">{{ p.description || '—' }}</td>
              </tr>
            </tbody>
          </table>
        </section>

        <!-- 请求头 -->
        <section v-if="enabledHeaders.length" class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.headers', { n: enabledHeaders.length }) }}</h4>
          <table class="kv-table">
            <thead>
              <tr><th>{{ t('docs.colKey') }}</th><th>{{ t('docs.colValue') }}</th><th>{{ t('docs.colDesc') }}</th></tr>
            </thead>
            <tbody>
              <tr v-for="h in enabledHeaders" :key="h.key">
                <td><code class="kv-key">{{ h.key }}</code></td>
                <td><code class="kv-val">{{ h.value || '—' }}</code></td>
                <td class="kv-desc">{{ h.description || '—' }}</td>
              </tr>
            </tbody>
          </table>
        </section>

        <!-- 请求 Body（Schema 树 / 表单字段表 / 源码） -->
        <section v-if="bodyView" class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.reqBody', { v: bodyView.label }) }}</h4>
          <SchemaTreeTable v-if="bodyView.kind === 'schema'" :rows="bodyView.schema" />
          <table v-else-if="bodyView.kind === 'form'" class="kv-table">
            <thead>
              <tr>
                <th>{{ t('docs.colKey') }}</th>
                <th>{{ t('docs.colValue') }}</th>
                <th v-if="bodyView.showKind">{{ t('body.colType') }}</th>
                <th>{{ t('docs.colDesc') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="f in bodyView.fields" :key="f.key">
                <td><code class="kv-key">{{ f.key }}</code></td>
                <td><code class="kv-val">{{ f.value || '—' }}</code></td>
                <td v-if="bodyView.showKind">{{ f.kind }}</td>
                <td class="kv-desc">{{ f.description || '—' }}</td>
              </tr>
            </tbody>
          </table>
          <pre v-else class="raw-body">{{ bodyView.content }}</pre>
        </section>

        <!-- 响应 Body Schema -->
        <section class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.respBodySchema') }}</h4>
          <SchemaTreeTable v-if="responseSchema" :rows="responseSchema" />
          <p v-else class="schema-empty">
            {{ t('docs.respSchemaEmpty') }}
          </p>
        </section>
      </div>

      <!-- 右栏：响应示例 置顶 + 代码生成 -->
      <div class="doc-right">
        <ResponseExamplesPanel :examples="examples" :draft="draft" />
        <CodeSnippetsPanel v-if="url" :draft="draft" :url="url" />
        <section v-else class="doc-card sec">
          <h4 class="doc-sec-title">{{ t('docs.secCode') }}</h4>
          <p class="schema-empty">{{ t('docs.codeNeedUrl') }}</p>
        </section>
      </div>
    </div>

    <!-- 文档导出弹窗 -->
    <ExportDocsDialog v-if="showExport" :draft="draft" @close="showExport = false" />
  </div>
</template>

<style scoped>
.docs {
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 1180px;
  padding: 4px 0 16px;
}

/* ---- 头部 Endpoint 模块 ---- */

.head-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.method-pill {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: 12.5px;
  font-weight: 700;
  padding: 3px 10px;
  border-radius: 6px;
  border: 1px solid color-mix(in srgb, var(--m) 22%, transparent);
  background: color-mix(in srgb, var(--m) 10%, transparent);
  color: var(--m);
}
.m-get,
.m-head {
  --m: var(--get);
}
.m-post {
  --m: var(--post);
}
.m-put {
  --m: var(--put);
}
.m-patch {
  --m: var(--patch);
}
.m-delete {
  --m: var(--delete);
}
.m-options {
  --m: var(--text-2);
}

.head-path {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.head-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

/* 导出文档入口：中性暗底按钮（区别于主操作「跳转调试」） */
.head-export {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-card);
  color: var(--text-1);
  font-family: inherit;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.head-export:hover {
  background: var(--bg-hover);
}

.head-sub {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
}

.head-name {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.head-status {
  flex-shrink: 0;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: var(--fs-xxs);
  color: var(--info);
  background: var(--info-tint);
  border: 1px solid var(--info-border);
}
.head-status.s-released {
  color: var(--success);
  background: var(--success-tint);
  border-color: color-mix(in srgb, var(--success) 22%, transparent);
}
.head-status.s-deprecated {
  color: var(--danger);
  background: var(--danger-tint);
  border-color: var(--danger-border);
}

.head-desc {
  margin: 6px 0 0;
  font-size: 12.5px;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
}

/* ---- 双栏 ---- */

.doc-grid {
  display: grid;
  grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
  gap: 16px;
  align-items: start;
}

/* 窄窗回退单栏（右栏内容移到主体之后） */
@media (max-width: 1080px) {
  .doc-grid {
    grid-template-columns: 1fr;
  }
}

.doc-left {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.doc-right {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
  position: sticky;
  top: 0;
}

.sec {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* ---- 基本信息（2x2 Key-Value 极简网格） ---- */

.meta-grid {
  margin: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px 24px;
}

.meta-item dt {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.meta-item dd {
  margin: 3px 0 0;
  font-size: 12px;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status-val {
  display: flex;
  align-items: center;
  gap: 6px;
}

/* 状态 Dot：随生命周期着色 */
.status-dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
}
.d-designing {
  background: var(--info);
}
.d-developing {
  background: var(--warning);
}
.d-testing {
  background: var(--patch);
}
.d-released {
  background: var(--success);
}
.d-deprecated {
  background: var(--danger);
}

.path-vars {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.path-vars-label {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.path-var {
  padding: 1px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text-2);
}

/* ---- 认证 ---- */

.auth-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.auth-badge {
  padding: 2px 10px;
  border-radius: 999px;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-tint);
  border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
}

.auth-detail {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- KV 表（Query / Headers / 表单 Body 共用） ---- */

.kv-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
}

.kv-table th {
  padding: 5px 10px;
  text-align: left;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.4px;
  border-bottom: 1px solid var(--border);
  white-space: nowrap;
}

.kv-table td {
  padding: 5px 10px;
  border-bottom: 1px solid var(--border);
  color: var(--text-2);
  vertical-align: middle;
}

.kv-table tbody tr:last-child td {
  border-bottom: none;
}

.kv-table tbody tr:hover td {
  background: var(--bg-hover);
}

.kv-key {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-1);
}

.kv-val {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-2);
  word-break: break-all;
}

.kv-desc {
  color: var(--text-3);
}

/* ---- 源码回退与空态 ---- */

.raw-body {
  margin: 0;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-1);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 320px;
  overflow: auto;
}

.schema-empty {
  margin: 0;
  padding: 4px 0;
  font-size: 12.5px;
  color: var(--text-3);
}
</style>
