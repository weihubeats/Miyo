<script setup lang="ts">
/**
 * DashboardNav：仪表板左侧导航。
 *
 * - 仅保留已实现的全局入口：仪表板（本页）、实时调试（/realtime 独立路由视图，
 *   此前只能从工作区工具抽屉进入）；
 * - 插件导航项由 PluginManager 动态注册（激活态才出现；停用 / 卸载即消失）；
 * - 项目内模块（接口树 / 场景 / 历史 / 文档）随项目走工作区，不在导航重复；
 * - 设置入口在顶栏右上角（齿轮按钮）、团队管理在工具栏，导航不再重复。
 * 注意：此前「仪表板 / API 项目」曾是两个同路由入口（navActive 恒双真，
 * 两个紫 pill 同时高亮），已合并为单一入口。
 */
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon, { type IconName } from '@/components/ui/Icon.vue'
import { useLocaleStore } from '@/stores/locale'
import { usePluginManager } from '@/plugins/manager'

const router = useRouter()
const route = useRoute()
const locale = useLocaleStore()
const t = locale.t

/** 合并后的导航项（核心静态项 + 插件注册项，按 order 升序）。 */
interface NavEntry {
  key: string
  label: string
  /** IconName 或 data:image/* URI（插件）。 */
  icon: string
  route: string
  order: number
  isPlugin: boolean
  badgeKey?: string
  badgeTipKey?: string
}

/** 核心导航项（静态；插件项经 PluginManager 动态插入）。 */
const CORE_NAV_ITEMS: NavEntry[] = [
  { key: 'dashboard', label: t('pnav.dashboard'), icon: 'gauge', route: '/projects', order: 0, isPlugin: false },
  { key: 'integration', label: t('pnav.integration'), icon: 'beaker', route: '/integration', order: 10, isPlugin: false },
  { key: 'realtime', label: t('pnav.realtime'), icon: 'globe', route: '/realtime', order: 20, isPlugin: false },
  {
    key: 'aiTest',
    label: t('pnav.aiTest'),
    icon: 'zap',
    route: '/ai-test',
    order: 30,
    isPlugin: false,
    // 页面仍在测试：「测试中」标记在导航入口就要可见，进页面前就知道结果可能不可靠
    badgeKey: 'ai.beta',
    badgeTipKey: 'ai.betaTipShort',
  },
  { key: 'plugins', label: t('pnav.plugins'), icon: 'plug', route: '/plugins', order: 90, isPlugin: false },
]

const manager = usePluginManager()

/** 插件注册的导航项（激活态插件；展示名走 i18n 键优先）。 */
const pluginNavItems = computed<NavEntry[]>(() =>
  manager.activeNavItems.map(({ item, pluginId }) => ({
    key: `plugin-${pluginId}-${item.id}`,
    label: item.label_key ? t(item.label_key) : (item.label ?? pluginId),
    icon: item.icon ?? 'package',
    route: item.route,
    order: item.order ?? 100,
    isPlugin: true,
  })),
)

/** 合并导航（核心 + 插件；核心 order 0-90，插件默认 100 起）。 */
const NAV_ITEMS = computed<NavEntry[]>(() =>
  [...CORE_NAV_ITEMS, ...pluginNavItems.value].sort(
    (a, b) => a.order - b.order || a.key.localeCompare(b.key),
  ),
)

/** 激活态跟随当前路由（实时调试视图打开时高亮对应入口）。 */
function navActive(item: NavEntry): boolean {
  return route.path === item.route
}

function onNav(item: NavEntry): void {
  if (!navActive(item)) router.push(item.route)
}

/** 插件 data-uri 图标走 <img>；IconName 校验交给渲染层兜底（未知名回落 package）。 */
const KNOWN_ICONS = new Set<string>(['gauge', 'beaker', 'globe', 'zap', 'plug', 'package'])
function iconOf(item: NavEntry): IconName {
  return KNOWN_ICONS.has(item.icon) ? (item.icon as IconName) : 'package'
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
      <img
        v-if="item.icon.startsWith('data:image/')"
        class="nav-icon-img"
        :src="item.icon"
        alt=""
        aria-hidden="true"
      />
      <Icon v-else :name="iconOf(item)" :size="15" />
      <span class="nav-label">{{ item.label }}</span>
      <span
        v-if="'badgeKey' in item && item.badgeKey"
        class="nav-beta"
        role="status"
        :title="item.badgeTipKey ? t(item.badgeTipKey) : ''"
      >
        {{ t(item.badgeKey) }}
      </span>
    </button>
  </nav>
</template>

<style scoped>
.dash-nav {
  /* 两个入口的极简导航：收窄给内容区让位 */
  width: 176px;
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
  border-radius: var(--radius-lg);
  background: none;
  color: var(--text-2);
  font-size: var(--fs-sm);
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
/* 选中态：tint 底 + accent 字（impeccable side-tab 规则：不用边缘竖条） */
.nav-item.active {
  background: var(--accent-tint);
  color: var(--accent);
  font-weight: 600;
}

.nav-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 第三方插件 data-uri 图标（与 Icon 同视觉尺寸，跟随 currentColor 无从谈起，原样展示） */
.nav-icon-img {
  width: 15px;
  height: 15px;
  flex: none;
  object-fit: contain;
  border-radius: var(--radius-sm);
}

/* 「测试中」标记：未选中 amber 描边，选中（紫底）提亮为半透明白底保证可读 */
.nav-beta {
  flex: none;
  /* 功能状态标签：11px 可读性地板（impeccable undersized-ui-text） */
  font-size: var(--fs-xxs);
  line-height: 1.5;
  padding: 0 5px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--warning) 55%, transparent);
  color: var(--warning);
  background: var(--warning-tint);
  white-space: nowrap;
}

.nav-item.active .nav-beta {
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  background: transparent;
  color: var(--accent);
}
</style>
