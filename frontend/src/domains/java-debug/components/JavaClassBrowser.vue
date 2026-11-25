<script setup lang="ts">
/**
 * JavaClassBrowser：左侧类 / Bean 浏览器。
 *
 * 两种呈现：
 * - **分组树**（attach 模式，行数据带 `category`）：业务代码（Controller /
 *   Service / Manager / Repository / 其他）→ Spring Bean → 第三方依赖（默认
 *   折叠）→ JDK（默认隐藏，需显式打开）→ 生成类 / 未识别；组可折叠，滚动到底
 *   增量加载；
 * - **扁平列表**（runtime / SDK 白名单模式，无分类字段）：与历史行为一致。
 *
 * 搜索由后端（attach 模式，分类 + 分页）或前端（runtime 模式）完成，组件只
 * 负责渲染与收藏置顶。
 */
import { computed, ref, watch } from 'vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import Icon from '@/components/ui/Icon.vue'
import Spinner from '@/components/ui/Spinner.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaClassCategory, JavaClassInfo, JavaClassStats } from '@/types/miyoApi.d'
import type { RecentMethod } from '@/domains/java-debug/stores/javaDebug'
import { groupClasses } from '@/domains/java-debug/utils/classGroups'
import type { ClassGroup } from '@/domains/java-debug/utils/classGroups'

const props = defineProps<{
  classes: JavaClassInfo[]
  loading: boolean
  connected: boolean
  activeClassName: string | null
  search: string
  recent: RecentMethod[]
  isFavorite: (className: string) => boolean
  /** 分类统计（attach 模式）。 */
  stats?: JavaClassStats | null
  /** 是否还有下一页（滚动加载）。 */
  hasMore?: boolean
  loadingMore?: boolean
  /** 当前选中的类别筛选（空 = 默认）。 */
  selectedCategories?: JavaClassCategory[]
  /** 包名前缀筛选值。 */
  packageFilter?: string
  includeDependencies?: boolean
  includeJdk?: boolean
  /** 用户配置的业务包前缀。 */
  businessPackages?: string[]
  /**
   * 分类视图开关（attach 模式）。
   *
   * 必须由调用方显式给出，**不能**用「返回行里有没有 category」推断：
   * Spring Bean 分类要等目标应用访问一次上下文（设计内降级），首屏行全是
   * `application` 时若按行推断，组标题 / 类别 tab / 开关会整块消失，界面退回
   * 显示 `-1 个方法` 的扁平旧列表（真实用户截图复现）。
   *
   * 注意：**不能**写成 `categorized?: boolean`。`defineProps` 的编译期类型推导
   * 会把 `boolean` 当成 Boolean prop 并注入 `default: false`，`?? 行推断` 的兜底
   * 永远不会生效（实测：每行都有 `category` 却仍渲染扁平列表）。
   */
  categorized?: boolean | undefined
}>()

const emit = defineEmits<{
  'update:search': [value: string]
  select: [className: string]
  toggleFavorite: [className: string]
  pickRecent: [methodId: string]
  'update:packageFilter': [value: string]
  'toggleCategory': [category: JavaClassCategory]
  'update:includeDependencies': [value: boolean]
  'update:includeJdk': [value: boolean]
  'loadMore': []
  'saveBusinessPackages': [list: string[]]
  'restoreDefaults': []
}>()

const locale = useLocaleStore()
const t = locale.t

/**
 * 是否启用了分类视图：显式开关优先，其次退回「行数据带 category」推断。
 *
 * 判定顺序不能反过来：attach 首屏可能一个 Bean 都没有（Spring 上下文要等目标
 * 应用访问一次才捕获），此时行里仍有 `category`，推断即可成立；而显式
 * `categorized` 让调用方在「暂时 0 行 / 0 个分类」时也能保持分类 UI 骨架。
 */
const categorized = computed(
  () =>
    props.categorized === true ||
    props.stats != null ||
    props.classes.some((c) => c.category != null),
)

/** 前端搜索过滤（runtime 模式；attach 模式由后端过滤）。 */
const visibleClasses = computed(() => {
  if (categorized.value) return props.classes
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

/** 分组树（收藏置顶在组内）。 */
const groups = computed<ClassGroup[]>(() =>
  categorized.value ? groupClasses(visibleClasses.value, props.isFavorite) : [],
)

/** 折叠状态（key → collapsed；用户手动展开/折叠优先于默认）。 */
const collapsedKeys = ref<Set<string>>(new Set())
watch(
  groups,
  (next) => {
    // 新出现的分组沿用默认折叠态；已存在的保持用户选择
    for (const group of next) {
      if (group.collapsed && !collapsedKeys.value.has(group.key)) {
        // 依赖 / JDK 默认折叠
        collapsedKeys.value.add(group.key)
      }
    }
  },
  { immediate: true },
)

function isCollapsed(group: ClassGroup): boolean {
  return collapsedKeys.value.has(group.key)
}

function toggleGroup(group: ClassGroup): void {
  const next = new Set(collapsedKeys.value)
  if (next.has(group.key)) next.delete(group.key)
  else next.add(group.key)
  collapsedKeys.value = next
}

const categoryLabelKeys: Array<[JavaClassCategory, string]> = [
  ['application', 'java2.catApplication'],
  ['spring_bean', 'java2.catSpringBean'],
  ['dependency', 'java2.catDependency'],
  ['jdk_internal', 'java2.catJdk'],
  ['generated', 'java2.catGenerated'],
  ['unknown', 'java2.catUnknown'],
]

const roleLabelKeys: Record<string, string> = {
  controller: 'java2.roleController',
  service: 'java2.roleService',
  manager: 'java2.roleManager',
  repository: 'java2.roleRepository',
  dto: 'java2.roleDto',
  config: 'java2.roleConfig',
  other: 'java2.roleOther',
}

function roleLabel(role: string | undefined): string {
  return role ? t(roleLabelKeys[role] ?? 'java2.roleOther') : t('java2.roleOther')
}

function categoryLabel(category: JavaClassCategory): string {
  const found = categoryLabelKeys.find(([c]) => c === category)
  return found ? t(found[1]) : category
}

/** 统计摘要（业务 / Bean / 依赖 / JDK）。 */
const statsText = computed(() => {
  const stats = props.stats
  if (!stats) return ''
  const by = stats.by_category ?? {}
  return [
    `${t('java2.catApplication')} ${by.application ?? 0}`,
    `${t('java2.catSpringBean')} ${by.spring_bean ?? 0}`,
    `${t('java2.catDependency')} ${by.dependency ?? 0}`,
    `${t('java2.catJdk')} ${by.jdk_internal ?? 0}`,
  ].join(' · ')
})

/** 需要提示的分类退化信息（未识别类数量）。 */
const unknownCount = computed(() => props.stats?.by_category?.unknown ?? 0)

// ---------- 业务包配置 ----------
const configOpen = ref(false)
const configDraft = ref('')

function openConfig(): void {
  configDraft.value = (props.businessPackages ?? []).join('\n')
  configOpen.value = true
}

function saveConfig(): void {
  const list = configDraft.value
    .split(/[\n,;]/)
    .map((v) => v.trim())
    .filter(Boolean)
  emit('saveBusinessPackages', list)
  configOpen.value = false
}

// ---------- 滚动加载 ----------
function onScroll(event: Event): void {
  if (!props.hasMore || props.loadingMore) return
  const el = event.target as HTMLElement
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 120) emit('loadMore')
}
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
      <div v-if="categorized" class="jcb-filters">
        <label class="jcb-prefix">
          <input
            class="jcb-prefix-input"
            :value="packageFilter"
            :placeholder="t('java2.packagePrefixPlaceholder')"
            type="text"
            @change="emit('update:packageFilter', ($event.target as HTMLInputElement).value)"
          />
        </label>
        <button
          type="button"
          class="jcb-filter-btn"
          :title="t('java2.businessPackages')"
          @click="openConfig"
        >
          <Icon name="settings" :size="13" />
        </button>
      </div>
      <div v-if="categorized" class="jcb-cats">
        <button
          v-for="[category, key] in categoryLabelKeys"
          :key="category"
          type="button"
          class="jcb-cat"
          :class="{ on: (selectedCategories ?? []).includes(category) }"
          @click="emit('toggleCategory', category)"
        >
          {{ t(key) }}
        </button>
        <label class="jcb-check">
          <input
            type="checkbox"
            :checked="includeDependencies"
            @change="emit('update:includeDependencies', ($event.target as HTMLInputElement).checked)"
          />
          {{ t('java2.includeDependencies') }}
        </label>
        <label class="jcb-check">
          <input
            type="checkbox"
            :checked="includeJdk"
            @change="emit('update:includeJdk', ($event.target as HTMLInputElement).checked)"
          />
          {{ t('java2.includeJdk') }}
        </label>
      </div>
      <p v-if="statsText" class="jcb-stats">{{ statsText }}</p>
      <p v-if="unknownCount > 0" class="jcb-hint">
        {{ t('java2.unknownClassesHint', { count: unknownCount }) }}
      </p>
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
      :description="t('java2.noClassesInCategoryDesc')"
      compact
    />

    <!-- 分类树（attach 模式） -->
    <div
      v-else-if="categorized"
      class="jcb-list"
      data-testid="java-class-list"
      @scroll="onScroll"
    >
      <section v-for="group in groups" :key="group.key" class="jcb-group">
        <button type="button" class="jcb-group-head" @click="toggleGroup(group)">
          <Icon :name="isCollapsed(group) ? 'chevron-right' : 'chevron-down'" :size="12" />
          <span class="jcb-group-title">
            {{ group.role ? roleLabel(group.role) : categoryLabel(group.category) }}
          </span>
          <span class="jcb-group-count">{{ group.count }}</span>
        </button>
        <ul v-if="!isCollapsed(group)" class="jcb-group-items">
          <li v-for="c in group.items" :key="c.class_name">
            <button
              type="button"
              class="jcb-item"
              :class="{ active: c.class_name === activeClassName }"
              :title="c.category_evidence || c.class_name"
              @click="emit('select', c.class_name)"
            >
              <span class="jcb-main">
                <span class="jcb-name">{{ c.simple_name }}</span>
                <span class="jcb-meta">
                  {{ c.package_name }}<template v-if="c.bean_name"> · {{ c.bean_name }}</template>
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
      </section>
      <div v-if="loadingMore" class="jcb-loading">
        <Spinner :size="14" />
      </div>
    </div>

    <!-- 扁平列表（runtime / SDK 模式） -->
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

    <!-- 业务包配置（分类弱证据；持久化） -->
    <div v-if="configOpen" class="jcb-config">
      <h3 class="jcb-config-title">{{ t('java2.businessPackages') }}</h3>
      <p class="jcb-config-desc">{{ t('java2.businessPackagesDesc') }}</p>
      <textarea
        v-model="configDraft"
        class="jcb-config-input"
        rows="4"
        :placeholder="t('java2.businessPackagesPlaceholder')"
      />
      <div class="jcb-config-actions">
        <button type="button" class="jcb-config-btn" @click="emit('restoreDefaults')">
          {{ t('java2.restoreDefaults') }}
        </button>
        <button type="button" class="jcb-config-btn" @click="configOpen = false">
          {{ t('java2.cancel') }}
        </button>
        <button type="button" class="jcb-config-btn primary" @click="saveConfig">
          {{ t('java2.save') }}
        </button>
      </div>
    </div>
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
  display: flex;
  flex-direction: column;
  gap: 6px;
}
/* 搜索框：统一 SearchInput，无本地布局占位 */
.jcb-filters {
  display: flex;
  align-items: center;
  gap: 4px;
}
.jcb-prefix {
  flex: 1;
  min-width: 0;
  display: flex;
}
.jcb-prefix-input {
  width: 100%;
  padding: 3px 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input, var(--bg-hover));
  color: var(--text-1);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono, monospace);
}
.jcb-filter-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 22px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.jcb-filter-btn:hover {
  color: var(--primary);
  background: var(--bg-hover);
}
.jcb-cats {
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
}
.jcb-cat {
  padding: 1px 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  font-size: var(--fs-xxs);
  cursor: pointer;
}
.jcb-cat.on {
  border-color: var(--primary);
  color: var(--primary);
}
.jcb-check {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  cursor: pointer;
}
.jcb-stats {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.jcb-hint {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  opacity: 0.85;
}
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
.jcb-group {
  margin-bottom: 2px;
}
.jcb-group-head {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  padding: 4px 6px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  font-weight: 600;
  text-align: left;
  cursor: pointer;
}
.jcb-group-head:hover {
  background: var(--bg-hover);
}
.jcb-group-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jcb-group-count {
  color: var(--text-3);
  font-weight: 400;
}
.jcb-group-items {
  list-style: none;
  margin: 0;
  padding: 0 0 0 10px;
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
.jcb-config {
  border-top: 1px solid var(--border);
  padding: 8px 10px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jcb-config-title {
  margin: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-1);
}
.jcb-config-desc {
  margin: 0;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}
.jcb-config-input {
  width: 100%;
  padding: 4px 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input, var(--bg-hover));
  color: var(--text-1);
  font-size: var(--fs-xxs);
  font-family: var(--font-mono, monospace);
  resize: vertical;
}
.jcb-config-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}
.jcb-config-btn {
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-xxs);
  cursor: pointer;
}
.jcb-config-btn.primary {
  border-color: var(--primary);
  color: var(--primary);
}
.jcb-config-btn:hover {
  background: var(--bg-hover);
}
</style>