<script setup lang="ts">
/**
 * IntegrationNavBar：集成测试模块统一导航（返回 + 层级面包屑）。
 *
 * 规范：
 * - 每一级详情页都有明确返回入口，返回目标由 backLabel 表达
 *   （返回测试场景 / 返回测试套件 / 返回执行历史 / 返回批次详情），
 *   不使用统一的「返回首页」；
 * - 面包屑展示完整层级：集成测试 / 目录A / 目录B / 当前对象；
 * - 祖先节点可带 onClick 实现「点层级回退」，当前节点不可点。
 *
 * 纯展示组件：返回与层级跳转全部 emit / 回调给宿主，不直接操作路由。
 */
import Icon from '@/components/ui/Icon.vue'

export interface NavCrumb {
  label: string
  /** 可点祖先节点（当前节点不传）。 */
  onClick?: () => void
}

defineProps<{
  backLabel: string
  crumbs: NavCrumb[]
  /** 面包屑后还有「当前对象」（如可编辑名称）：末级不作为 current 标记。 */
  suffix?: boolean
}>()

const emit = defineEmits<{ (e: 'back'): void }>()
</script>

<template>
  <nav class="int-nav" aria-label="breadcrumb">
    <button class="int-nav-back" type="button" @click="emit('back')">
      <Icon name="chevron-left" :size="14" />
      <span class="int-nav-back-label">{{ backLabel }}</span>
    </button>
    <ol v-if="crumbs.length" class="int-nav-crumbs">
      <li v-for="(c, i) in crumbs" :key="`${c.label}-${i}`" class="int-nav-item">
        <button
          v-if="c.onClick && i < crumbs.length - 1"
          type="button"
          class="int-nav-crumb link"
          @click="c.onClick"
        >
          {{ c.label }}
        </button>
        <span
          v-else
          class="int-nav-crumb"
          :class="{ current: !suffix && i === crumbs.length - 1 }"
          :aria-current="!suffix && i === crumbs.length - 1 ? 'page' : undefined"
        >
          {{ c.label }}
        </span>
        <span v-if="i < crumbs.length - 1" class="int-nav-slash" aria-hidden="true">/</span>
      </li>
    </ol>
  </nav>
</template>

<style scoped>
.int-nav {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.int-nav-back {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex: none;
  padding: 3px 8px 3px 5px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  color: var(--text-2);
  font-size: var(--fs-xxs);
  cursor: pointer;
  white-space: nowrap;
}

.int-nav-back:hover {
  border-color: var(--border-strong);
  color: var(--text-1);
  background: var(--bg-hover);
}

.int-nav-crumbs {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
  overflow: hidden;
}

.int-nav-item {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}

.int-nav-crumb {
  display: inline-block;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.int-nav-crumb.link {
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
}

.int-nav-crumb.link:hover {
  color: var(--accent);
}

.int-nav-crumb.current {
  color: var(--text-1);
  font-weight: 600;
}

.int-nav-slash {
  color: var(--text-3);
  font-size: var(--fs-xxs);
  opacity: 0.7;
}

@media (max-width: 900px) {
  .int-nav-crumb {
    max-width: 120px;
  }
}
</style>