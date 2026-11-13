<script setup lang="ts">
/**
 * ProjectListRow：项目列表行（列表模式）。
 *
 * 项目数量较多时的紧凑视图：一行展示 图标 + 名称 + 状态 + API 数 +
 * 场景数 + 更新时间 + 更多操作；整行可点击进入项目。
 * 分组归属由外层团队分组头呈现（同一分组结构，不重复造数据关系）。
 */
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { useLocaleStore } from '@/stores/locale'
import type { Project } from '@/types/miyoApi.d'
import { avatarStyle, initials, timeAgo } from '@/domains/workspace/components/projectlist/projectlist/projectMeta'

const locale = useLocaleStore()
const t = locale.t

defineProps<{
  project: Project
  count: number
  scenarioCount: number
  active: boolean
  /** 当前展开更多菜单的项目 id（父级持有，同时只开一个） */
  menuOpen: boolean
}>()

const emit = defineEmits<{
  open: []
  'toggle-menu': []
  rename: []
  duplicate: []
  delete: []
}>()
</script>

<template>
  <div
    class="proj-row"
    :data-project-id="project.id"
    role="button"
    tabindex="0"
    :aria-label="project.name"
    @click="emit('open')"
    @keydown.enter.prevent="emit('open')"
    @keydown.space.prevent="emit('open')"
  >
    <span class="row-avatar" :style="avatarStyle(project.name)">{{ initials(project.name) }}</span>
    <div class="row-name-wrap">
      <span class="row-name" v-tooltip-overflow="project.name">{{ project.name }}</span>
      <span v-if="active" class="row-status">
        <span class="status-dot" aria-hidden="true"></span>
        {{ t('pcard.statusActive') }}
      </span>
    </div>
    <span class="row-col row-metric" :title="t('pcard.apiCount', { n: count })">
      <Icon name="plug" :size="12" />{{ count }}
    </span>
    <span class="row-col row-metric row-scenario" :title="t('pcard.scenarioCount', { n: scenarioCount })">
      <Icon name="play" :size="11" />{{ scenarioCount }}
    </span>
    <span class="row-col row-time">{{ timeAgo(project.updated_at) }}</span>
    <div class="row-more" data-no-drag @click.stop>
      <IconButton name="more-horizontal" :size="15" :title="t('common.moreActions')" @click="emit('toggle-menu')" />
      <div v-if="menuOpen" class="more-menu" role="menu">
        <button class="menu-item" type="button" @click="emit('rename')">
          <Icon name="pencil" :size="13" /> {{ t('common.rename') }}
        </button>
        <button class="menu-item" type="button" @click="emit('duplicate')">
          <Icon name="copy" :size="13" /> {{ t('common.copyAction') }}
        </button>
        <button class="menu-item danger" type="button" @click="emit('delete')">
          <Icon name="trash" :size="13" /> {{ t('common.delete') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.proj-row {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr) 64px 64px 92px 30px;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border-radius: var(--radius-lg);
  border: 1px solid transparent;
  background: var(--projcard-bg);
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease);
}
.proj-row:hover {
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  background: var(--projcard-hover-bg);
}
.proj-row:focus-visible {
  border-color: var(--accent);
  outline: 2px solid var(--focus-ring);
  outline-offset: 1px;
}

.row-avatar {
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius);
  border: 1px solid rgba(255, 255, 255, 0.12);
  font-size: 11px;
  font-weight: 700;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.35);
  user-select: none;
}

.row-name-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.row-name {
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--success-tint);
  color: var(--success);
}
.row-status .status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
}

/* 指标列：等宽字体 + 右对齐，跨行纵向对齐 */
.row-col {
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  color: var(--text-3);
  text-align: right;
  white-space: nowrap;
}
.row-metric {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 5px;
  font-variant-numeric: tabular-nums;
}
.row-time {
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
}

.row-more {
  position: relative;
  display: flex;
  justify-content: flex-end;
}

.more-menu {
  position: absolute;
  top: 28px;
  right: 0;
  z-index: 10;
  min-width: 132px;
  padding: 4px;
  border-radius: var(--radius);
  border: 1px solid var(--border-strong);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-lg);
  animation: menu-in 120ms var(--ease);
}

.menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 30px;
  padding: 0 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-1);
  font-size: 12.5px;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition: background var(--dur) var(--ease);
}
.menu-item:hover {
  background: var(--bg-hover);
}
.menu-item.danger {
  color: var(--danger);
}
.menu-item.danger:hover {
  background: var(--danger-tint);
}

@keyframes menu-in {
  from {
    opacity: 0;
    transform: translateY(-3px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

/* 窄窗口：先收场景列，再收时间列（列表仍有名称 + API 数可用） */
@media (max-width: 1200px) {
  .proj-row {
    grid-template-columns: 30px minmax(0, 1fr) 64px 92px 30px;
  }
  .row-scenario {
    display: none;
  }
}
@media (max-width: 1024px) {
  .proj-row {
    grid-template-columns: 30px minmax(0, 1fr) 64px 30px;
  }
  .row-time {
    display: none;
  }
}
</style>
