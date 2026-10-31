<script setup lang="ts">
/**
 * HistoryPanel：请求历史面板（侧栏「请求历史」页签）。
 * - 工具栏：仅当前接口复选 + 计数 + 清空；搜索框内嵌状态筛选 Tag；
 * - 记录卡两行：行1 方法 Tag + Path + 状态胶囊，行2 紧凑耗时 + 时间；
 * - 交互：hover 微浮起边框、点击恢复进激活态（accent 微底 + 描边）、
 *   慢请求（>500ms 警示色 / >3s 危险色）；不使用行级红竖线；
 * - 点击记录调用 store.restoreFromHistory：Method/URL/Headers/Body 恢复到主编辑器。
 * 数据源为 workspace store（发送成功后由 EndpointEditor 触发刷新）。
 */
import { computed, onMounted, ref } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import { useLocaleStore } from '../stores/locale'
import { formatDurationShort } from '../utils/format'
import { methodTone } from '../utils/methodTone'
import type { RequestHistory } from '../types/miyoApi'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Popconfirm from './ui/Popconfirm.vue'

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

/**
 * 本地搜索 + 状态筛选：原来仅「仅当前接口」复选，无关键字/状态码检索。
 * 历史按页加载（首屏 50 + 加载更多，后端无 offset 只有 limit），
 * 本地过滤只作用于已加载部分。
 */
const keyword = ref('')
const statusFilter = ref<'all' | '2xx' | '4xx5xx'>('all')
/** 最近一次点击恢复的记录 id（激活态高亮，给用户位置感）。 */
const activeId = ref<string | null>(null)

const filtered = computed(() => {
  const q = keyword.value.trim().toLowerCase()
  return store.histories.filter((h) => {
    if (statusFilter.value === '2xx' && !(h.status != null && h.status < 400)) return false
    if (statusFilter.value === '4xx5xx' && !(h.status != null && h.status >= 400)) return false
    if (!q) return true
    return (
      h.url.toLowerCase().includes(q) ||
      h.method.toLowerCase().includes(q) ||
      String(h.status ?? '').includes(q)
    )
  })
})

function cycleStatusFilter(): void {
  statusFilter.value =
    statusFilter.value === 'all' ? '2xx' : statusFilter.value === '2xx' ? '4xx5xx' : 'all'
}

onMounted(() => {
  void store.loadHistories()
})

function reload(): void {
  void store.loadHistories()
}

/** 加载更多（后端无 offset，按窗口增量重拉）。 */
const loadingMore = ref(false)
async function loadMore(): Promise<void> {
  if (loadingMore.value) return
  loadingMore.value = true
  try {
    await store.loadMoreHistories()
  } finally {
    loadingMore.value = false
  }
}

async function clear(): Promise<void> {
  activeId.value = null
  await store.clearHistories()
}

/** 恢复到编辑器并点亮该行激活态。 */
function restore(h: RequestHistory): void {
  activeId.value = h.id
  store.restoreFromHistory(h)
}

/** 展示用短地址：去协议域名；兼容存量记录剥渲染前 `{{变量}}` 前缀（`{{base_url}}/api` → `/api`）。 */
function shortUrl(url: string): string {
  let stripped = url.replace(/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\/[^/]+/, '')
  stripped = stripped.replace(/^\{\{[^}]+\}\}/, '')
  return stripped || url
}

/** 慢请求耗时分级：>500ms 警示黄，>3s 危险红（正常默认灰）。 */
function durationCls(ms: number | null | undefined): string {
  const v = ms ?? 0
  if (v > 3000) return 'hp-slow-bad'
  if (v > 500) return 'hp-slow'
  return ''
}

/** 本地短时间：今天显示 HH:mm，更早显示 MM-DD HH:mm。 */
function shortTime(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const now = new Date()
  const pad = (n: number): string => String(n).padStart(2, '0')
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  return sameDay ? hm : `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${hm}`
}
</script>

<template>
  <div class="history-panel">
    <div class="hp-head">
      <label class="hp-filter" :title="t('history.onlyCurrentHint')">
        <input
          v-model="store.historyOnlyCurrent"
          type="checkbox"
          class="hp-check"
          @change="reload"
        />
        {{ t('history.onlyCurrent') }}
      </label>
      <span class="hp-count">{{ filtered.length }}/{{ store.histories.length }}</span>
      <Popconfirm
        :title="t('history.clearConfirm')"
        :confirm-text="t('history.clear')"
        danger
        @confirm="clear"
      >
        <IconButton name="trash" :size="14" tone="danger" :title="t('history.clearTitle')" />
      </Popconfirm>
    </div>

    <div class="hp-search-box">
      <Icon name="search" :size="12" class="hp-search-icon" />
      <input
        v-model="keyword"
        class="hp-search"
        type="text"
        :placeholder="t('history.searchPh')"
        spellcheck="false"
      />
      <button
        class="hp-status-filter"
        type="button"
        :title="t('history.statusFilter', { v: statusFilter === 'all' ? t('history.all') : statusFilter })"
        @click="cycleStatusFilter"
      >
        {{ statusFilter === 'all' ? t('history.all') : statusFilter }}
      </button>
    </div>

    <div class="hp-list">
      <div v-for="h in filtered" :key="h.id" class="hp-item">
        <button
          class="hp-row"
          :class="{ active: activeId === h.id }"
          type="button"
          @click="restore(h)"
        >
        <span class="hp-line1">
          <span class="hp-method" :class="methodTone(h.method)">{{ h.method }}</span>
          <span class="hp-url" v-tooltip-overflow="h.url">{{ shortUrl(h.url) }}</span>
          <span
            v-if="h.status != null"
            class="hp-status"
            :class="(h.status ?? 0) >= 400 ? 'err' : 'ok'"
          >
            {{ h.status }}
          </span>
        </span>
        <span class="hp-line2">
          <span class="hp-meta" :class="durationCls(h.duration_ms)">
            {{ formatDurationShort(h.duration_ms ?? 0) }}
          </span>
          <span class="hp-meta hp-time">{{ shortTime(h.created_at) }}</span>
        </span>
        </button>
        <Popconfirm
          :title="t('ws.historyDeleteConfirm')"
          :confirm-text="t('common.delete')"
          danger
          @confirm="store.deleteHistory(h.id)"
        >
          <IconButton
            class="hp-del"
            name="trash"
            :size="12"
            tone="danger"
            :title="t('ws.historyDelete')"
          />
        </Popconfirm>
      </div>
      <EmptyState
        v-if="!store.histories.length"
        icon="history"
        compact
        :title="t('history.empty')"
        :description="t('history.emptyHint')"
      />
      <p v-else-if="!filtered.length" class="hp-no-match">
        {{ keyword.trim() ? t('history.noMatchQ', { q: keyword.trim() }) : t('history.noMatch') }}
      </p>
      <button
        v-if="store.historyHasMore && store.histories.length"
        class="hp-more"
        type="button"
        :disabled="loadingMore"
        @click="loadMore"
      >
        {{ t('history.loadMore') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.history-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

/* ---- 工具行：过滤 + 计数 + 清空 ---- */
.hp-head {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 30px;
  padding: 0 12px;
  flex-shrink: 0;
}

.hp-filter {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--text-3);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  transition: color var(--dur) var(--ease);
}
.hp-filter:hover {
  color: var(--text-2);
}

.hp-check {
  width: 12px;
  height: 12px;
  accent-color: var(--accent);
}

.hp-count {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* ---- 搜索框：内嵌图标 + 关键字 + 状态筛选 Tag，聚焦描边点亮 ---- */
.hp-search-box {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  margin: 4px 12px 6px;
  padding: 4px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
  transition: border-color var(--dur) var(--ease);
}
.hp-search-box:focus-within {
  border-color: color-mix(in srgb, var(--accent) 60%, transparent);
}
.hp-search-icon {
  color: var(--text-3);
  flex-shrink: 0;
}
.hp-search {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-xs);
}
.hp-search::placeholder {
  color: var(--text-3);
}
.hp-status-filter {
  flex-shrink: 0;
  border: none;
  border-left: 1px solid var(--border);
  background: transparent;
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  padding: 0 0 0 8px;
  cursor: pointer;
  transition: color var(--dur) var(--ease);
}
.hp-status-filter:hover {
  color: var(--text-1);
}
.hp-no-match {
  padding: 12px;
  font-size: var(--fs-xs);
  color: var(--text-3);
  text-align: center;
}

/* ---- 加载更多：列表尾部整宽弱按钮 ---- */
.hp-more {
  flex-shrink: 0;
  width: 100%;
  height: 28px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-card);
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.hp-more:hover:not(:disabled) {
  background: var(--bg-hover);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  color: var(--text-1);
}
.hp-more:disabled {
  opacity: 0.6;
  cursor: default;
}

/* ---- 列表：卡片间距 4px ---- */
.hp-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 2px 8px 12px;
}

/* ---- 记录卡：两行（行1 方法Tag+Path+状态胶囊，行2 耗时·时间） ----
 * 单行 5 列会把 path 挤成短截断；两行让 path 独占整行宽度。 */
.hp-item {
  display: flex;
  align-items: center;
  gap: 2px;
}

/** 行内删除钮：hover 才出现（与树行「⋯」同节奏） */
.hp-del {
  flex: none;
  display: none;
  margin-right: 6px;
}

.hp-item:hover .hp-del {
  display: inline-flex;
}

.hp-row {
  display: flex;
  flex-direction: column;
  gap: 3px;
  width: 100%;
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: none;
  font-family: inherit;
  cursor: pointer;
  text-align: left;
  transition:
    background var(--dur) var(--ease),
    border-color var(--dur) var(--ease);
}

.hp-row:hover {
  background: color-mix(in srgb, var(--text-3) 10%, transparent);
  border-color: color-mix(in srgb, var(--text-3) 16%, transparent);
}

/* 激活态（最近恢复的记录）：accent 微底 + 描边，不用行级红竖线 */
.hp-row.active {
  background: var(--accent-tint);
  border-color: color-mix(in srgb, var(--accent) 30%, transparent);
}

.hp-line1 {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

/* 方法微型 Tag：文字/底/描边色由 methodTone() 单源供给（@theme method-* 令牌），
 * 此处只定尺寸与描边形态（拼接类名是 methodTone.ts 完整字面量，Tailwind 可扫到）。 */
.hp-method {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  border-width: 1px;
  border-style: solid;
  border-radius: var(--radius);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 700;
  line-height: 1.4;
  letter-spacing: 0.04em;
}

.hp-url {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 状态微型胶囊：2xx 翠绿微底 / 4xx5xx 绯红微底（tint 淡底可读性好） */
.hp-status {
  flex-shrink: 0;
  padding: 1px 6px;
  border-radius: var(--radius);
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  font-weight: 600;
  line-height: 1.4;
  font-variant-numeric: tabular-nums;
}
.hp-status.ok {
  background: var(--success-tint);
  color: var(--success);
}
.hp-status.err {
  background: var(--danger-tint);
  color: var(--danger);
}

.hp-line2 {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
  margin-top: 1px;
  padding-left: 4px;
}

.hp-meta {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

/* 慢请求分级：>500ms 警示、>3s 危险 */
.hp-meta.hp-slow {
  color: var(--warning);
}
.hp-meta.hp-slow-bad {
  color: var(--danger);
}
</style>
