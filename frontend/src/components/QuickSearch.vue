<script setup lang="ts">
/**
 * QuickSearch：全局命令面板（⌘K / Ctrl+K）。
 *
 * 检索范围：接口（名称 + method + 路径）、场景、环境、已加载的请求历史、
 * 全局操作（新建请求 / 设置 / Mock 启停 / 深浅主题）。空查询展示「最近打开」
 * 与全部操作；模糊匹配走 utils/fuzzyMatch（子序列 + 词首/连续加分，多词过滤）。
 * 键盘：↑↓ 循环选择、Enter 打开、Esc 关闭；设置 / Mock 两类动作涉及宿主
 * 局部状态（弹窗 ref、Mock 忙标志），经 action 事件交 WorkspaceView 处理。
 */
import { computed, nextTick, ref, watch } from 'vue'
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import { useThemeStore } from '../stores/theme'
import { fuzzyScore } from '../utils/fuzzyMatch'
import { methodTextTone } from '../utils/methodTone'
import Icon from './ui/Icon.vue'
import type { IconName } from './ui/Icon.vue'
import type { Endpoint, RequestHistory } from '../types/miyoApi'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{
  'update:open': [v: boolean]
  /** 需要宿主处理的动作（设置弹窗 / Mock 启停依赖 WorkspaceView 局部状态）。 */
  action: [key: 'open-settings' | 'toggle-mock']
}>()

const store = useWorkspaceStore()
const theme = useThemeStore()
const t = useLocaleStore().t

const query = ref('')
const inputEl = ref<HTMLInputElement | null>(null)
const listEl = ref<HTMLElement | null>(null)
const activeIdx = ref(0)

interface Item {
  key: string
  group: 'recent' | 'endpoints' | 'scenarios' | 'environments' | 'history' | 'actions'
  title: string
  subtitle: string
  icon: IconName
  method?: string
  score: number
  run: () => void
}

const close = (): void => emit('update:open', false)

function runItem(item: Item): void {
  close()
  item.run()
}

// ---------- 数据源 ----------

function endpointItem(ep: Endpoint, score: number, group: Item['group']): Item {
  return {
    key: `ep:${ep.id}`,
    group,
    title: ep.name || `${ep.method} ${ep.path}`,
    subtitle: `${ep.method} ${ep.path}`,
    icon: 'file',
    method: ep.method,
    score,
    run: () => store.openEndpoint(ep),
  }
}

const recents = computed<Item[]>(() => {
  if (query.value.trim()) return []
  const byId = new Map(store.endpoints.map((e) => [e.id, e] as const))
  return store.recentEndpointIds
    .map((id, i) => {
      const ep = byId.get(id)
      return ep ? endpointItem(ep, 1000 - i, 'recent') : null
    })
    .filter((v): v is Item => v !== null)
})

const matchedEndpoints = computed<Item[]>(() => {
  const q = query.value.trim()
  if (!q) return []
  return store.endpoints
    .map((ep): Item | null => {
      const s = fuzzyScore(`${ep.name} ${ep.path}`, q)
      return s == null ? null : endpointItem(ep, s, 'endpoints')
    })
    .filter((v): v is Item => v !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, 12)
})

const matchedScenarios = computed<Item[]>(() => {
  const q = query.value.trim()
  if (!q) return []
  return store.scenarios
    .map((s): Item | null => {
      const score = fuzzyScore(s.name, q)
      return score == null
        ? null
        : {
            key: `sc:${s.id}`,
            group: 'scenarios' as const,
            title: s.name,
            subtitle: t('scenario.stepCount', { n: s.steps.length }),
            icon: 'beaker' as const,
            score,
            run: () => store.openScenario(s.id),
          }
    })
    .filter((v): v is Item => v !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, 6)
})

const matchedEnvs = computed<Item[]>(() => {
  const q = query.value.trim()
  if (!q) return []
  return store.environments
    .map((e): Item | null => {
      const score = fuzzyScore(e.name, q)
      return score == null
        ? null
        : {
            key: `env:${e.id}`,
            group: 'environments' as const,
            title: e.name,
            subtitle: e.base_url,
            icon: 'globe' as const,
            score: store.activeEnvId === e.id ? score + 50 : score,
            run: () => void store.setEnvironment(e.id),
          }
    })
    .filter((v): v is Item => v !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, 6)
})

const matchedHistory = computed<Item[]>(() => {
  const q = query.value.trim()
  if (!q) return []
  return store.histories
    .map((h: RequestHistory): Item | null => {
      const score = fuzzyScore(`${h.method} ${h.url}`, q)
      return score == null
        ? null
        : {
            key: `hi:${h.id}`,
            group: 'history' as const,
            title: h.url,
            subtitle: `${h.method} · ${h.status ?? '—'}`,
            icon: 'history' as const,
            score,
            run: () => store.restoreFromHistory(h),
          }
    })
    .filter((v): v is Item => v !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, 8)
})

const ACTIONS = computed<Item[]>(() => [
  {
    key: 'act:new',
    group: 'actions',
    title: t('quicksearch.actionNewRequest'),
    subtitle: '⌘T',
    icon: 'plus',
    score: 0,
    run: () => store.openCreateTab(),
  },
  {
    key: 'act:settings',
    group: 'actions',
    title: t('quicksearch.actionSettings'),
    subtitle: '',
    icon: 'settings',
    score: 0,
    run: () => emit('action', 'open-settings'),
  },
  {
    key: 'act:mock',
    group: 'actions',
    title: t('quicksearch.actionMock'),
    subtitle: '',
    icon: 'server',
    score: 0,
    run: () => emit('action', 'toggle-mock'),
  },
  {
    key: 'act:theme',
    group: 'actions',
    title: t('quicksearch.actionTheme'),
    subtitle: theme.resolved === 'dark' ? t('quicksearch.toLight') : t('quicksearch.toDark'),
    icon: theme.resolved === 'dark' ? 'sun' : 'moon',
    score: 0,
    run: () => theme.setMode(theme.resolved === 'dark' ? 'light' : 'dark'),
  },
])

const matchedActions = computed<Item[]>(() => {
  const q = query.value.trim()
  if (!q) return ACTIONS.value
  return ACTIONS.value.map((a): Item | null => {
    const score = fuzzyScore(a.title, q)
    return score == null ? null : { ...a, score }
  })
    .filter((v): v is Item => v !== null)
    .sort((a, b) => b.score - a.score)
})

const results = computed<Item[]>(() => [
  ...recents.value,
  ...matchedEndpoints.value,
  ...matchedScenarios.value,
  ...matchedEnvs.value,
  ...matchedHistory.value,
  ...matchedActions.value,
])

const GROUP_LABELS = computed<Record<Item['group'], string>>(() => ({
  recent: t('quicksearch.groupRecent'),
  endpoints: t('quicksearch.groupEndpoints'),
  scenarios: t('quicksearch.groupScenarios'),
  environments: t('quicksearch.groupEnvironments'),
  history: t('quicksearch.groupHistory'),
  actions: t('quicksearch.groupActions'),
}))

/** 分组标题仅在该组首行前渲染（扁平列表 + 组变更检测）。 */
function isGroupStart(idx: number): boolean {
  const items = results.value
  return idx === 0 || items[idx - 1]!.group !== items[idx]!.group
}

function resetActive(): void {
  activeIdx.value = 0
  void nextTick(() => inputEl.value?.focus())
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      query.value = ''
      resetActive()
    }
  },
)
watch(results, resetActive)
watch(activeIdx, async () => {
  await nextTick()
  listEl.value
    ?.querySelector('.qs-item.active')
    ?.scrollIntoView({ block: 'nearest' })
})

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    close()
    return
  }
  const total = results.value.length
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    if (total) activeIdx.value = (activeIdx.value + 1) % total
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    if (total) activeIdx.value = (activeIdx.value - 1 + total) % total
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const item = results.value[activeIdx.value]
    if (item) runItem(item)
  }
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="qs-backdrop" @mousedown.self="close">
      <div class="qs-panel" role="dialog" :aria-label="t('quicksearch.title')">
        <div class="qs-inputrow">
          <Icon name="search" :size="15" class="qs-glyph" />
          <input
            ref="inputEl"
            v-model="query"
            class="qs-input"
            type="text"
            :placeholder="t('quicksearch.placeholder')"
            spellcheck="false"
            role="combobox"
            :aria-expanded="results.length > 0"
            @keydown="onKeydown"
          />
        </div>
        <div ref="listEl" class="qs-list" role="listbox">
          <template v-for="(item, idx) in results" :key="item.key">
            <div v-if="isGroupStart(idx)" class="qs-group">{{ GROUP_LABELS[item.group] }}</div>
            <button
              type="button"
              class="qs-item"
              :class="{ active: idx === activeIdx }"
              role="option"
              :aria-selected="idx === activeIdx"
              @mousemove="activeIdx = idx"
              @click="runItem(item)"
            >
              <span class="qs-item-glyph"><Icon :name="item.icon" :size="14" /></span>
              <span
                v-if="item.method"
                class="qs-method"
                :class="methodTextTone(item.method)"
              >{{ item.method }}</span>
              <span class="qs-title">{{ item.title }}</span>
              <span v-if="item.subtitle" class="qs-sub">{{ item.subtitle }}</span>
            </button>
          </template>
          <div v-if="!results.length" class="qs-empty">{{ t('quicksearch.empty') }}</div>
        </div>
        <div class="qs-foot">{{ t('quicksearch.hint') }}</div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.qs-backdrop {
  position: fixed;
  inset: 0;
  z-index: 400;
  background: var(--mask);
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding-top: 14vh;
}

.qs-panel {
  width: min(620px, calc(100vw - 48px));
  max-height: 68vh;
  display: flex;
  flex-direction: column;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-xl);
  box-shadow: 0 25px 50px -12px rgb(0 0 0 / 0.5), var(--shadow-lg);
  overflow: hidden;
}

.qs-inputrow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border);
}

.qs-glyph {
  color: var(--text-3);
  flex: none;
}

.qs-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-1);
  font-size: var(--fs-md);
}

.qs-input::placeholder {
  color: var(--text-3);
}

.qs-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 6px;
}

.qs-group {
  padding: 8px 10px 3px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
}

.qs-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: transparent;
  padding: 7px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  text-align: left;
  color: var(--text-1);
}

.qs-item.active {
  background: var(--bg-hover);
}

.qs-item-glyph {
  flex: none;
  display: inline-flex;
  color: var(--text-3);
}

.qs-item.active .qs-item-glyph {
  color: var(--text-2);
}

.qs-method {
  flex: none;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
}

.qs-title {
  flex: none;
  max-width: 55%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
}

.qs-sub {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  font-family: var(--font-mono);
}

.qs-empty {
  padding: 26px 0;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.qs-foot {
  padding: 7px 14px;
  border-top: 1px solid var(--border);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
