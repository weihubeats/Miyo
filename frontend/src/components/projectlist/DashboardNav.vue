<script setup lang="ts">
/**
 * DashboardNav：仪表板左侧导航。
 *
 * - 仅保留已实现入口：仪表板（主页面）；
 * - 设置入口在顶栏右上角（齿轮按钮），导航不再重复；
 * - 集合、API 文档等未实现模块暂不展示。
 * 注意：此前「仪表板 / API 项目」曾是两个同路由入口（navActive 恒双真，
 * 两个紫 pill 同时高亮），已合并为单一入口。
 */
import { useRoute, useRouter } from 'vue-router'
import Icon from '../ui/Icon.vue'
import { useLocaleStore } from '../../stores/locale'

const route = useRoute()
const router = useRouter()
const locale = useLocaleStore()
const t = locale.t

const NAV_ITEMS = [
  { key: 'dashboard', labelKey: 'pnav.dashboard', icon: 'gauge' as const, route: '/projects' },
  { key: 'skills', labelKey: 'pnav.skills', icon: 'package' as const, route: '/skills' },
]

/** 当前路由对应的入口高亮。 */
function navActive(item: (typeof NAV_ITEMS)[number]): boolean {
  return route.path === item.route
}

function onNav(item: (typeof NAV_ITEMS)[number]): void {
  router.push(item.route)
}
</script>

<template>
  <nav class="dash-nav" :aria-label="t('pnav.mainNav')">
    <button
      v-for="item in NAV_ITEMS"
      :key="item.key"
      class="nav-item"
      :class="{ active: navActive(item) }"
      type="button"
      @click="onNav(item)"
    >
      <Icon :name="item.icon" :size="15" />
      <span class="nav-label">{{ t(item.labelKey) }}</span>
    </button>
  </nav>
</template>

<style scoped>
.dash-nav {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 16px 10px;
  border-right: 1px solid var(--border);
  background: var(--bg-panel);
  overflow-y: auto;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 36px;
  padding: 0 12px;
  border: none;
  border-radius: 10px;
  background: none;
  color: var(--text-2);
  font-size: 13px;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    background var(--dur) var(--ease),
    color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.nav-item:active {
  background: var(--bg-active);
}
/* 选中态：Obsidian 全宽主题 pill（渐变 + 光晕 + 白字，跟随主题 accent） */
.nav-item.active {
  background: linear-gradient(135deg, var(--accent), var(--accent-hover));
  color: #fff;
  font-weight: 600;
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.22),
    0 4px 14px var(--accent-tint);
}

.nav-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

</style>
