<script setup lang="ts">
/**
 * JavaClassBrowser：左侧类 / Bean 浏览器。
 *
 * 搜索（类名 / 包名 / Bean 名子串，前端过滤）+ 收藏置顶（星标持久化）+
 * 最近使用方法（会话内存）。连接未就绪时给引导空态。
 */
import { computed } from 'vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import Spinner from '@/components/ui/Spinner.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaClassInfo } from '@/types/miyoApi.d'
import type { RecentMethod } from '@/domains/java-debug/stores/javaDebug'

const props = defineProps<{
  classes: JavaClassInfo[]
  loading: boolean
  connected: boolean
  activeClassName: string | null
  search: string
  recent: RecentMethod[]
  isFavorite: (className: string) => boolean
}>()

const emit = defineEmits<{
  'update:search': [value: string]
  select: [className: string]
  toggleFavorite: [className: string]
  pickRecent: [methodId: string]
}>()

const locale = useLocaleStore()
const t = locale.t

/** 搜索过滤 + 收藏置顶（名称排序稳定）。 */
const visibleClasses = computed(() => {
  const term = props.search.trim().toLowerCase()
  const matched = term
    ? props.classes.filter(
        (c) =>
          c.class_name.toLowerCase().includes(term) ||
          c.package_name.toLowerCase().includes(term) ||
          (c.bean_name ?? '').toLowerCase().includes(term),
      )
    : [...props.classes]
  return matched.sort((a, b) => {
    const fa = props.isFavorite(a.class_name) ? 0 : 1
    const fb = props.isFavorite(b.class_name) ? 0 : 1
    if (fa !== fb) return fa - fb
    return a.class_name.localeCompare(b.class_name)
  })
})
</script>

<template>
  <aside class="jcb">
    <div class="jcb-head">
      <SearchInput
        :model-value="search"
        class="jcb-search"
        :placeholder="t('java.searchClasses')"
        @update:model-value="emit('update:search', $event)"
      />
    </div>

    <div v-if="loading" class="jcb-loading">
      <Spinner :size="16" />
    </div>

    <EmptyState
      v-else-if="!connected"
      icon="plug"
      :title="t('java.notConnectedTitle')"
      :description="t('java.notConnectedDesc')"
      compact
    />
    <EmptyState
      v-else-if="visibleClasses.length === 0"
      icon="package"
      :title="t('java.noClassesTitle')"
      :description="t('java.noClassesDesc')"
      compact
    />

    <ul v-else class="jcb-list" data-testid="java-class-list">
      <li v-for="c in visibleClasses" :key="c.class_name">
        <button
          type="button"
          class="jcb-item"
          :class="{ active: c.class_name === activeClassName }"
          @click="emit('select', c.class_name)"
        >
          <span class="jcb-main">
            <span class="jcb-name" :title="c.class_name">{{ c.simple_name }}</span>
            <span class="jcb-meta">
              {{ c.method_count }} {{ t('java.methodsSuffix') }}<template v-if="c.bean_name"> · {{ c.bean_name }}</template>
            </span>
          </span>
          <span
            class="jcb-star"
            role="button"
            :tabindex="0"
            :aria-label="isFavorite(c.class_name) ? t('java.unfavorite') : t('java.favorite')"
            @click.stop="emit('toggleFavorite', c.class_name)"
            @keydown.enter.stop="emit('toggleFavorite', c.class_name)"
          >
            <Icon :name="isFavorite(c.class_name) ? 'star-fill' : 'star'" :size="13" />
          </span>
        </button>
      </li>
    </ul>

    <section v-if="recent.length > 0" class="jcb-recent">
      <h3 class="jcb-recent-title">{{ t('java.recentMethods') }}</h3>
      <ul class="jcb-recent-list">
        <li v-for="r in recent" :key="r.projectId + r.methodId">
          <button type="button" class="jcb-recent-item" @click="emit('pickRecent', r.methodId)">
            {{ r.signature }}
          </button>
        </li>
      </ul>
    </section>
  </aside>
</template>

<style scoped>
.jcb {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
  border-right: 1px solid var(--border);
}
.jcb-head {
  padding: 10px 10px 6px;
}
/* 搜索框：统一 SearchInput，无本地布局占位 */
.jcb-loading {
  display: flex;
  justify-content: center;
  padding: 24px;
}
.jcb-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  list-style: none;
  margin: 0;
  padding: 4px 6px;
}
.jcb-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-1);
  text-align: left;
  cursor: pointer;
}
.jcb-item:hover {
  background: var(--bg-hover);
}
.jcb-item.active {
  background: var(--bg-active, var(--bg-hover));
}
.jcb-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.jcb-name {
  font-size: var(--fs-xs);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jcb-meta {
  font-size: var(--fs-xxs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jcb-star {
  display: inline-flex;
  padding: 3px;
  border-radius: var(--radius-sm);
  color: var(--text-3);
}
.jcb-star:hover {
  color: var(--primary);
  background: var(--bg-hover);
}
.jcb-recent {
  border-top: 1px solid var(--border);
  padding: 8px 6px 10px;
  max-height: 180px;
  overflow-y: auto;
}
.jcb-recent-title {
  margin: 0 6px 6px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
}
.jcb-recent-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.jcb-recent-item {
  width: 100%;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono, monospace);
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jcb-recent-item:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
</style>
