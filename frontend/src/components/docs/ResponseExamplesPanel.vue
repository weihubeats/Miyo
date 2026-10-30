<script setup lang="ts">
/**
 * ResponseExamplesPanel：文档预览右栏的「响应示例」面板。
 *
 * - 按状态码分组 Tab（2xx 成功在前，4xx/5xx 错误在后；首个为 Default）；
 * - 同状态码多示例时提供示例切换列表；
 * - Body 用 CodeMirror 只读模式渲染（JSON 语法高亮 + 行号 + 折叠）；
 * - 右上角一键复制当前示例 Body；
 * - 空态提供「手动添加示例」与「从 Mock 快速填充」（请求 Body Schema 映射，
 *   字段唯一）两个快捷入口。
 */
import { computed, ref, watch } from 'vue'
import JsonCodeMirror from '../JsonCodeMirror.vue'
import EmptyState from '../ui/EmptyState.vue'
import Icon from '../ui/Icon.vue'
import Tabs from '../ui/Tabs.vue'
import type { TabItem } from '../ui/Tabs.vue'
import { useWorkspaceStore } from '../../stores/workspace'
import { useLocaleStore } from '../../stores/locale'
import { useMiyoApi } from '../../composables/useMiyoApi'
import { useToast } from '../../composables/useToast'
import { copyText } from '../../utils/clipboard'
import { prettyJson } from '../../utils/jsonFormat'
import { statusTextOf } from '../../utils/testCases'
import { inferSchema, mockJsonFromSchema } from '../../utils/schemaInfer'
import type { Endpoint, ResponseExample } from '../../types/miyoApi'

const props = defineProps<{ examples: ResponseExample[]; draft?: Endpoint | null }>()

const store = useWorkspaceStore()
const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

/** 状态码分组：2xx 在前、3xx 次之、4xx/5xx 在后，各自升序；首个标记 Default。 */
const statusTabs = computed<TabItem[]>(() => {
  const statuses = Array.from(new Set(props.examples.map((e) => e.status))).sort((a, b) => {
    const rank = (s: number): number => (s < 400 ? 0 : 1)
    return rank(a) - rank(b) || a - b
  })
  return statuses.map((s, i) => ({
    key: String(s),
    label: `${s} ${statusTextOf(s)}${i === 0 ? ` ${t('respex.defaultTag')}` : ''}`,
  }))
})

const activeStatus = ref('')

watch(
  statusTabs,
  (tabs) => {
    if (!tabs.some((t) => t.key === activeStatus.value)) {
      activeStatus.value = tabs[0]?.key ?? ''
    }
  },
  { immediate: true },
)

/** 当前状态码下的示例列表。 */
const statusExamples = computed(() =>
  props.examples.filter((e) => String(e.status) === activeStatus.value),
)

const activeExample = ref<ResponseExample | null>(null)

watch(
  statusExamples,
  (list) => {
    if (!list.some((e) => e.id === activeExample.value?.id)) {
      activeExample.value = list[0] ?? null
    }
  },
  { immediate: true },
)

/** 展示用 Body：JSON 美化（无损），解析失败回退原文。 */
const displayBody = computed(() => {
  const body = activeExample.value?.body ?? ''
  if (!body.trim()) return ''
  try {
    return prettyJson(body)
  } catch {
    return body
  }
})

async function copyBody(): Promise<void> {
  const body = activeExample.value?.body ?? ''
  if (!body) return
  const ok = await copyText(body)
  if (ok) {
    toast.success(t('respex.copied'))
  } else {
    toast.error(t('response.copyFail'))
  }
}

// ---------- 空态快捷入口 ----------

/** 落库并写入 store 缓存（与 workspace.saveAsExample 同一缓存约定）。 */
async function persistExample(example: ResponseExample): Promise<void> {
  const saved = await api.saveExample(example)
  const endpointId = example.endpoint_id
  const list = store.examples.get(endpointId) ?? []
  list.unshift(saved)
  store.examples.set(endpointId, list)
  toast.success(t('respex.exampleSaved', { name: saved.name }))
}

function newExample(name: string, status: number, body: string): ResponseExample {
  const now = new Date().toISOString()
  return {
    id: crypto.randomUUID(),
    endpoint_id: props.draft?.id ?? '',
    name,
    status,
    headers: {},
    body,
    content_type: 'application/json',
    created_at: now,
    updated_at: now,
  }
}

/** 空示例（200 / 空 Body），创建后可切到调试页或直接编辑。 */
async function addManual(): Promise<void> {
  if (!props.draft) return
  try {
    await persistExample(newExample(t('respex.manualName'), 200, ''))
  } catch (err) {
    toast.error(t('respex.addFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

/**
 * 从 Mock 快速填充：请求 Body JSON → Schema → Mock JSON 映射
 * （mockJsonFromSchema 保证每个字段唯一），生成 200 示例。
 */
async function fillFromMock(): Promise<void> {
  if (!props.draft) return
  const body = props.draft.request.body
  let mockBody: string | null = null
  if (body.mode === 'json') {
    try {
      const mock = mockJsonFromSchema(inferSchema(JSON.parse(body.raw)))
      mockBody = mock ? JSON.stringify(mock, null, 2) : null
    } catch {
      mockBody = null
    }
  }
  if (mockBody === null) {
    toast.warning(t('respex.noMockSchema'))
    return
  }
  try {
    await persistExample(newExample(t('respex.mockName'), 200, mockBody))
  } catch (err) {
    toast.error(t('respex.fillFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}
</script>

<template>
  <section class="rep doc-card">
    <header class="rep-head">
      <h4 class="doc-sec-title">{{ t('respex.title') }}</h4>
      <span v-if="examples.length" class="rep-count">{{ t('respex.count', { n: examples.length }) }}</span>
      <button
        v-if="activeExample"
        class="miyo-btn miyo-btn-sm"
        type="button"
        @click="copyBody"
      >
        <Icon name="copy" :size="12" /> {{ t('common.copy') }}
      </button>
    </header>

    <template v-if="examples.length">
      <Tabs v-model="activeStatus" :tabs="statusTabs" size="sm" class="rep-tabs" />
      <div v-if="statusExamples.length > 1" class="rep-picker">
        <button
          v-for="ex in statusExamples"
          :key="ex.id"
          type="button"
          class="rep-picker-item"
          :class="{ active: activeExample?.id === ex.id }"
          @click="activeExample = ex"
        >
          {{ ex.name || t('default.exampleName') }}
        </button>
      </div>
      <div class="rep-meta" v-if="activeExample">
        <span class="rep-status" :class="{ err: activeExample.status >= 400 }">
          {{ activeExample.status }} {{ statusTextOf(activeExample.status) }}
        </span>
        <span v-if="activeExample.content_type" class="rep-ctype">{{ activeExample.content_type }}</span>
        <span class="rep-time">{{ activeExample.updated_at.slice(0, 16).replace('T', ' ') }}</span>
      </div>
      <div class="rep-body">
        <JsonCodeMirror v-if="displayBody" :model-value="displayBody" readonly />
        <p v-else class="rep-empty-body">{{ t('respex.noBody') }}</p>
      </div>
    </template>
    <template v-else>
      <EmptyState
        icon="file"
        :title="t('respex.empty')"
        :description="t('respex.emptyHint')"
        compact
      />
      <div class="rep-empty-actions">
        <button type="button" class="rep-mini-btn" @click="addManual">
          <Icon name="plus" :size="12" /> {{ t('respex.addManual') }}
        </button>
        <button type="button" class="rep-mini-btn" @click="fillFromMock">{{ t('respex.fillFromMock') }}</button>
      </div>
    </template>
  </section>
</template>

<style scoped>
.rep {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.rep-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.rep-count {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.rep-head .miyo-btn {
  margin-left: auto;
}

.rep-tabs :deep(.tabs) {
  border-bottom-color: var(--border);
}

.rep-picker {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.rep-picker-item {
  padding: 2px 10px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-card);
  color: var(--text-2);
  font-size: var(--fs-xs);
  cursor: pointer;
  transition:
    color var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.rep-picker-item:hover {
  color: var(--text-1);
  border-color: var(--border-strong);
}
.rep-picker-item.active {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  background: var(--accent-tint);
}

.rep-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.rep-status {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--success);
}
.rep-status.err {
  color: var(--danger);
}

.rep-ctype {
  font-family: var(--font-mono);
}

.rep-body {
  height: 260px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-code);
  overflow: hidden;
}

.rep-empty-body {
  margin: 0;
  padding: 14px;
  font-size: 12px;
  color: var(--text-3);
}

/* ---- 空态快捷按钮（neutral-800 微型按钮） ---- */

.rep-empty-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.rep-mini-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 12px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  color: var(--text-1);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}
.rep-mini-btn:hover {
  background: var(--bg-hover);
}
.rep-mini-btn:active {
  background: var(--bg-active);
}
</style>
