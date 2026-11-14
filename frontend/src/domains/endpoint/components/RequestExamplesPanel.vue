<script setup lang="ts">
/**
 * RequestExamplesPanel：请求用例（请求快照）管理。
 * - 顶部：名称输入 + 「保存当前请求」：把当前编辑器的完整请求（RequestSpec）存为快照；
 * - 列表：最新在前，点击回填编辑器（深拷贝，还原参数/认证/请求头/Body 等）；
 * - 行操作：复制（另存为「名称 副本」）/ 删除（Popconfirm 确认）。
 */
import { computed, ref, watch } from 'vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import { smartTabFor } from '@/utils/requestBar'
import type { Endpoint, RequestExample } from '@/types/miyoApi.d'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Popconfirm from '@/components/ui/Popconfirm.vue'

const props = defineProps<{ draft: Endpoint | null }>()

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

/** 用例名称输入（默认 = 当前 method + path，打开接口时预填）。 */
const nameInput = ref('')

watch(
  () => props.draft?.id,
  () => {
    const d = props.draft
    nameInput.value = d ? `${d.method} ${d.path}` : ''
  },
  { immediate: true },
)

/** 当前接口的请求用例（最新在前）。 */
const examples = computed<RequestExample[]>(() =>
  props.draft ? (store.requestExamples.get(props.draft.id) ?? []) : [],
)

/** 保存当前请求为用例快照。 */
async function saveCurrent(): Promise<void> {
  const d = props.draft
  if (!d) return
  const ok = await store.saveRequestAsExample(d.id, nameInput.value, d.request)
  if (ok) {
    nameInput.value = `${d.method} ${d.path}`
  }
}

/** 回填用例到编辑器：深拷贝 request；active_tab 缺失时按 Method 智能默认。 */
function apply(example: RequestExample): void {
  const d = props.draft
  if (!d) return
  store.applyRequestExample(d.id, example)
  if (!d.request.active_tab) {
    d.request.active_tab = smartTabFor(d.method)
  }
}

/** 复制用例：以该用例的请求另存一份「名称 副本」。 */
async function duplicate(example: RequestExample): Promise<void> {
  const d = props.draft
  if (!d) return
  const saved = await store.saveRequestAsExample(d.id, t('examples.copyName', { name: example.name }), example.request)
  if (saved) nameInput.value = ''
}

/** 删除用例。 */
async function remove(example: RequestExample): Promise<void> {
  const d = props.draft
  if (!d) return
  await store.deleteRequestExample(d.id, example.id)
}

/** 相对时间：今天显示 HH:mm，跨天显示 MM-DD HH:mm。 */
function shortTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  const now = new Date()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  return sameDay ? hm : `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${hm}`
}
</script>

<template>
  <div class="rex">
    <div class="rex-save">
      <input
        v-model="nameInput"
        class="rex-name miyo-input"
        type="text"
        :placeholder="draft ? `${draft.method} ${draft.path}` : t('examples.namePh')"
        @keyup.enter="saveCurrent"
      />
      <button class="rex-save-btn miyo-btn miyo-btn-primary" type="button" :disabled="!draft" @click="saveCurrent">
        {{ t('examples.saveCurrent') }}
      </button>
    </div>

    <div v-if="examples.length" class="rex-list">
      <div v-for="ex in examples" :key="ex.id" class="rex-row" @dblclick="apply(ex)">
        <div class="rex-main" :title="t('examples.dblclickHint')">
          <div class="rex-title">{{ ex.name }}</div>
          <div class="rex-meta">
            {{ t('examples.savedAt', { v: shortTime(ex.created_at) }) }}
            <template v-if="ex.request.active_tab"> · Tab: {{ ex.request.active_tab }}</template>
          </div>
        </div>
        <div class="rex-actions">
          <IconButton name="download" :size="12" :title="t('examples.applyBack')" @click="apply(ex)" />
          <IconButton name="copy" :size="12" :title="t('examples.duplicate')" @click="duplicate(ex)" />
          <Popconfirm
            :title="t('examples.deleteConfirmTitle', { name: ex.name })"
            :description="t('confirm.undone')"
            :confirm-text="t('common.delete')"
            danger
            @confirm="remove(ex)"
          >
            <IconButton name="trash" :size="12" :title="t('examples.delete')" />
          </Popconfirm>
        </div>
      </div>
    </div>
    <EmptyState
      v-else
      icon="list"
      compact
      :title="t('examples.empty')"
      :description="t('examples.emptyHint')"
    />
  </div>
</template>

<style scoped>
.rex {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.rex-save {
  display: flex;
  gap: 8px;
}

/* 名称输入 / 保存按钮：视觉收敛到全局 miyo-input / miyo-btn-primary，仅留布局 */
.rex-name {
  flex: 1;
  min-width: 0;
}

.rex-save-btn {
  flex-shrink: 0;
}

.rex-list {
  display: flex;
  flex-direction: column;
}

.rex-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 6px;
  border-bottom: 1px solid var(--border);
}
.rex-row:hover {
  background: var(--bg-hover);
}

.rex-main {
  flex: 1;
  min-width: 0;
}

.rex-title {
  font-size: var(--fs-xs);
  color: var(--text-1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.rex-meta {
  margin-top: 1px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.rex-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0.55;
  transition: opacity var(--dur) var(--ease);
}
.rex-row:hover .rex-actions {
  opacity: 1;
}
</style>
