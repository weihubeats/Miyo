<script setup lang="ts">
/**
 * CookiePanel：Cookie 管理面板（侧栏「Cookie」页签）。
 * - 按域名分组展示 Jar 中的登录态：名称 / 值（截断）/ 路径 / 过期 / Secure / HttpOnly；
 * - 域名搜索过滤 + 按域清理 / 全部清理（Popconfirm 二次确认）；
 * - 打开页签时加载（静默 IPC，无全局进度条抖动）。
 *
 * 背景：原来 reqwest 内建 jar 是黑盒，登录态问题只能重启应用；
 * 自管 Jar（miyo-http::cookie）保持自动回放语义，本面板为其管理面。
 */
import { computed, onMounted, ref } from 'vue'
import { useMiyoApi } from '../composables/useMiyoApi'
import { useToast } from '../composables/useToast'
import { useLocaleStore } from '../stores/locale'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import IconButton from './ui/IconButton.vue'
import Popconfirm from './ui/Popconfirm.vue'
import type { CookieEntry } from '../types/miyoApi'

const api = useMiyoApi()
const toast = useToast()
const locale = useLocaleStore()
const t = locale.t

const cookies = ref<CookieEntry[]>([])
const filter = ref('')
const loading = ref(false)
/** Cookie 值默认掩码展示（共享屏幕 / 截图即泄漏登录态），眼睛开关临时揭示。 */
const revealed = ref(false)

/** 过期响应防护：过滤词防抖 + 手动刷新并发时，仅最后一次请求回写。 */
let reloadSeq = 0

async function reload(): Promise<void> {
  const seq = ++reloadSeq
  loading.value = true
  try {
    const list = (await api.cookieList(filter.value.trim() || null)) ?? []
    if (seq !== reloadSeq) return
    cookies.value = list
  } catch (err) {
    if (seq === reloadSeq) {
      toast.error(t('cookie.loadFail'), { message: err instanceof Error ? err.message : String(err) })
    }
  } finally {
    if (seq === reloadSeq) loading.value = false
  }
}

onMounted(() => {
  void reload()
})

let filterTimer: ReturnType<typeof setTimeout> | undefined
function onFilterInput(): void {
  if (filterTimer) clearTimeout(filterTimer)
  filterTimer = setTimeout(() => void reload(), 200)
}

const grouped = computed(() => {
  const map = new Map<string, CookieEntry[]>()
  for (const c of cookies.value) {
    const list = map.get(c.domain)
    if (list) list.push(c)
    else map.set(c.domain, [c])
  }
  return [...map.entries()].sort(([a], [b]) => a.localeCompare(b))
})

async function clearDomain(domain: string): Promise<void> {
  try {
    const n = await api.cookieClear(domain)
    toast.success(t('cookie.clearedDomain', { domain, n }))
    await reload()
  } catch (err) {
    toast.error(t('cookie.clearFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

async function clearAll(): Promise<void> {
  try {
    const n = await api.cookieClear(null)
    toast.success(t('cookie.clearedAll', { n }))
    await reload()
  } catch (err) {
    toast.error(t('cookie.clearFail'), { message: err instanceof Error ? err.message : String(err) })
  }
}

/** 值过长截断展示（揭示态用）。 */
function shortValue(v: string): string {
  return v.length > 48 ? `${v.slice(0, 48)}…` : v
}

/** 掩码展示：前 4 + 后 2，中段 ••••；揭示后仍按长度截断。 */
function displayValue(v: string): string {
  if (!revealed.value) {
    return v.length <= 6 ? '••••••' : `${v.slice(0, 4)}••••${v.slice(-2)}`
  }
  return shortValue(v)
}
</script>

<template>
  <div class="cookie-panel">
    <div class="cp-head">
      <div class="cp-search">
        <Icon name="search" :size="13" class="cp-search-icon" />
        <input
          v-model="filter"
          class="cp-input"
          type="text"
          :placeholder="t('cookie.filterPh')"
          spellcheck="false"
          @input="onFilterInput"
        />
      </div>
      <span class="cp-count">{{ t('cookie.count', { n: cookies.length }) }}</span>
      <button
        class="cp-reload"
        :class="{ active: revealed }"
        type="button"
        :title="t('cookie.toggleValues')"
        :aria-label="t('cookie.toggleValues')"
        :aria-pressed="revealed"
        @click="revealed = !revealed"
      >
        <Icon name="eye" :size="13" />
      </button>
      <button
        class="cp-reload"
        type="button"
        :title="t('common.refresh')"
        :aria-label="t('common.refresh')"
        @click="reload"
      >
        <Icon name="refresh" :size="13" />
      </button>
      <Popconfirm
        :title="t('cookie.clearAllConfirm')"
        :confirm-text="t('cookie.clear')"
        danger
        @confirm="clearAll"
      >
        <IconButton name="trash" :size="14" tone="danger" :title="t('cookie.clearAllTitle')" />
      </Popconfirm>
    </div>

    <div v-if="loading && !cookies.length" class="cp-hint">{{ t('common.loading') }}</div>
    <EmptyState
      v-else-if="!cookies.length"
      :title="t('cookie.empty')"
      :description="t('cookie.emptyHint')"
    />

    <div v-else class="cp-list">
      <div v-for="[domain, items] in grouped" :key="domain" class="cp-group">
        <div class="cp-group-head">
          <span class="cp-domain" :title="domain">{{ domain }}</span>
          <span class="cp-group-count">{{ items.length }}</span>
          <Popconfirm
            :title="t('cookie.clearDomainConfirm', { domain })"
            :confirm-text="t('cookie.clear')"
            danger
            @confirm="clearDomain(domain)"
          >
            <IconButton name="trash" :size="12" tone="danger" :title="t('cookie.clearDomainTitle', { domain })" />
          </Popconfirm>
        </div>
        <div v-for="c in items" :key="`${c.domain}/${c.name}/${c.path}`" class="cp-row">
          <span class="cp-name">{{ c.name }}</span>
          <span class="cp-value" :class="{ masked: !revealed }">{{ displayValue(c.value) }}</span>
          <span class="cp-flags">
            <span v-if="c.http_only" class="cp-flag" :title="t('cookie.httpOnlyHint')">H</span>
            <span v-if="c.secure" class="cp-flag" :title="t('cookie.secureHint')">S</span>
            <span v-if="c.expires_at" class="cp-exp" :title="t('cookie.expiresHint', { v: c.expires_at })">⏱</span>
            <span v-else class="cp-exp" :title="t('cookie.sessionHint')">○</span>
          </span>
        </div>
      </div>
    </div>

    <p class="cp-foot">{{ t('cookie.foot') }}</p>
  </div>
</template>

<style scoped>
.cookie-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.cp-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border);
}
.cp-search {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.cp-search-icon {
  color: var(--text-3);
  flex-shrink: 0;
}
.cp-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-1);
  font-size: 12px;
}
.cp-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}
.cp-reload {
  display: inline-flex;
  border: none;
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
  padding: 4px;
  border-radius: 6px;
}
.cp-reload:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.cp-reload.active {
  color: var(--accent);
}
.cp-value.masked {
  color: var(--text-3);
  letter-spacing: 0.5px;
}
.cp-hint {
  padding: 12px;
  font-size: 12px;
  color: var(--text-3);
}
.cp-list {
  flex: 1;
  overflow-y: auto;
  padding: 6px 0 12px;
}
.cp-group {
  margin-bottom: 4px;
}
.cp-group-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px 2px;
}
.cp-domain {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 700;
  color: var(--text-1);
}
.cp-group-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.cp-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 4px 10px 4px 16px;
  font-size: var(--fs-xs);
}
.cp-row:hover {
  background: var(--bg-hover);
}
.cp-name {
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--text-1);
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cp-value {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cp-flags {
  flex-shrink: 0;
  display: inline-flex;
  gap: 4px;
  align-items: center;
}
.cp-flag {
  font-size: 9.5px;
  font-weight: 700;
  padding: 0 4px;
  border-radius: 4px;
  background: var(--info-tint, var(--bg-hover));
  color: var(--info);
}
.cp-exp {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.cp-foot {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
</style>
