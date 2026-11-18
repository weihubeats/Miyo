<script setup lang="ts">
/**
 * Breadcrumbs：面包屑导航（导航规范第二阶段 · 多级页面导航）。
 *
 * 使用约束（规范原文）：
 * - 只在存在明确层级关系时使用（如 工作台 > 场景编辑器），与返回钮不同时
 *   堆叠同一层级入口（父级可点 = 返回动作，此时页面不应再有重复返回钮）；
 * - 当前页面名称不可点击（aria-current="page"），上级可点击跳转；
 * - 层级过深（> max，默认 4）折叠中段为「…」（不可点，title 展开全路径）。
 */
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import Icon from '@/components/ui/Icon.vue'
import { useLocaleStore } from '@/stores/locale'

export interface BreadcrumbItem {
  /** 层级名称（i18n 文案由调用方解析后传入）。 */
  label: string
  /** 可点击层级的目标路由（路由级层级）；缺省看 onSelect。 */
  to?: string
  /** 可点击层级的页面内动作（如编辑器态回列表，经脏确认）；缺省 = 位置展示。 */
  onSelect?: () => void
}

const props = withDefaults(
  defineProps<{
    items: BreadcrumbItem[]
    /** 超出后折叠中段为省略号。 */
    max?: number
  }>(),
  { max: 4 },
)

const router = useRouter()
const t = useLocaleStore().t

/** 渲染项：业务层级 + 折叠标记（省略号挂在后一个可见项之前）。 */
interface VisibleItem extends BreadcrumbItem {
  ellipsisBefore: boolean
  hiddenTitles?: string[]
}

const overflow = computed(() => props.items.length > props.max)

/**
 * 折叠：保留首层与最后 max-1 层，中段并入省略号（title 提示完整路径）。
 * 未超限时原样展示。
 */
const visibleItems = computed<VisibleItem[]>(() => {
  if (!overflow.value) {
    return props.items.map((item) => ({ ...item, ellipsisBefore: false }))
  }
  const head = props.items.slice(0, 1)
  const tail = props.items.slice(-(props.max - 1))
  const hidden = props.items.slice(1, props.items.length - (props.max - 1))
  const visible: VisibleItem[] = [...head, ...tail].map((item) => ({
    ...item,
    ellipsisBefore: false,
  }))
  // 首层之后的第一项前插入省略号（携带被折叠层级的名称提示）
  if (visible[1]) visible[1] = { ...visible[1], ellipsisBefore: true, hiddenTitles: hidden.map((h) => h.label) }
  return visible
})

function go(item: BreadcrumbItem): void {
  if (item.to) void router.push(item.to)
  else item.onSelect?.()
}
</script>

<template>
  <nav class="crumbs" :aria-label="t('nav.breadcrumbAria')">
    <template v-for="(item, index) in visibleItems" :key="`${item.label}-${index}`">
      <span v-if="item.ellipsisBefore" class="crumb-ellipsis" :title="item.hiddenTitles?.join(' / ')" aria-hidden="true">
        …
      </span>
      <button
        v-else-if="(item.to || item.onSelect) && index < visibleItems.length - 1"
        type="button"
        class="crumb-link"
        @click="go(item)"
      >
        {{ item.label }}
      </button>
      <span v-else class="crumb-current" aria-current="page">{{ item.label }}</span>
      <Icon v-if="index < visibleItems.length - 1" class="crumb-sep" name="chevron-right" :size="12" :stroke-width="1.5" />
    </template>
  </nav>
</template>

<style scoped>
.crumbs {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
}

.crumb-link {
  display: inline-flex;
  align-items: center;
  padding: 2px 4px;
  border: none;
  background: none;
  border-radius: var(--radius-sm);
  color: var(--text-2);
  font-size: inherit;
  font-family: inherit;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease);
}
.crumb-link:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.crumb-link:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.crumb-current {
  color: var(--text-1);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.crumb-sep {
  flex: none;
  color: var(--text-3);
}

.crumb-ellipsis {
  flex: none;
  padding: 0 2px;
  color: var(--text-3);
  cursor: default;
}
</style>
