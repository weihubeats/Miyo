<script setup lang="ts">
/**
 * ProjectCard：项目卡片（网格模式）。
 *
 * 四行信息层级（统一卡片高度）：
 * 1. 图标 + 名称 + 当前状态（打开中 pill）；
 * 2. 描述（最多两行，空描述占位同样占两行保齐）；
 * 3. 指标行：API 数 · 场景数 · 更新时间；
 * 4. 进入项目提示（hover 提亮）。
 * 更多操作（⋯）独立悬浮右上角，click.stop 不触发卡片跳转；
 * hover 轻微提亮边框，删除走父级 ProjectDeleteModal 二次确认。
 */
import Icon from '../ui/Icon.vue'
import IconButton from '../ui/IconButton.vue'
import { useLocaleStore } from '../../stores/locale'
import type { Project } from '../../types/miyoApi'
import { avatarStyle, initials, timeAgo } from './projectMeta'

const locale = useLocaleStore()
const t = locale.t

defineProps<{
  project: Project
  count: number
  /** 场景数（list_project_stats 后发字段，缺省按 0） */
  scenarioCount: number
  active: boolean
  /** 当前展开更多菜单的卡片 id（父级持有，保证同时只开一个） */
  menuOpen: boolean
  /** 手动排序模式下显示拖拽手柄 */
  draggable: boolean
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
  <div class="proj-card" :data-project-id="project.id" @click="emit('open')">
    <span v-if="draggable" class="dnd-handle" :title="t('pcard.dragHint')" @click.stop>
      <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor" aria-hidden="true">
        <circle cx="2" cy="2" r="1.3" />
        <circle cx="8" cy="2" r="1.3" />
        <circle cx="2" cy="7" r="1.3" />
        <circle cx="8" cy="7" r="1.3" />
        <circle cx="2" cy="12" r="1.3" />
        <circle cx="8" cy="12" r="1.3" />
      </svg>
    </span>
    <div class="proj-head">
      <span class="proj-avatar" :style="avatarStyle(project.name)">{{ initials(project.name) }}</span>
      <span class="proj-title" v-tooltip-overflow="project.name">{{ project.name }}</span>
      <span v-if="active" class="proj-status active">
        <span class="status-dot" aria-hidden="true"></span>
        {{ t('pcard.statusActive') }}
      </span>
    </div>
    <p class="proj-desc" :class="{ empty: !project.description }">
      {{ project.description || t('pcard.noDesc') }}
    </p>
    <div class="proj-metrics">
      <span class="metric"><Icon name="plug" :size="12" />{{ t('pcard.apiCount', { n: count }) }}</span>
      <span class="metric-sep">·</span>
      <span class="metric"><Icon name="play" :size="11" />{{ t('pcard.scenarioCount', { n: scenarioCount }) }}</span>
      <span class="metric-sep">·</span>
      <span class="metric"><Icon name="clock" :size="12" />{{ t('pcard.updatedAgo', { v: timeAgo(project.updated_at) }) }}</span>
    </div>
    <div class="proj-foot">
      <span class="proj-open-hint">
        {{ t('pcard.open') }} <Icon name="arrow-up-right" :size="12" />
      </span>
    </div>
    <div class="proj-side" data-no-drag>
      <div class="proj-more" @click.stop>
        <IconButton name="more-horizontal" :size="16" :title="t('common.moreActions')" @click="emit('toggle-menu')" />
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
  </div>
</template>

<style scoped>
/* ---------- 项目卡片：四行结构，统一高度 ---------- */
.proj-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px 16px;
  /* 网格子项：允许收缩到轨道宽，长内容走 ellipsis 而不是撑爆网格 */
  min-width: 0;
  border-radius: var(--radius-xl);
  /* 表面/描边/投影走 --projcard-*（深色微光半透底；浅色档在 style.css = 纯白卡） */
  border: 1px solid var(--projcard-border);
  background: var(--projcard-bg);
  box-shadow: var(--projcard-shadow);
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.proj-card:hover {
  border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  background: var(--projcard-hover-bg);
}

/* ---- 第 1 行：图标 + 名称 + 状态 ---- */
.proj-head {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  /* 右上角 ⋯ 菜单的安全距离 */
  padding-right: 30px;
}

.proj-avatar {
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-md);
  border: 1px solid rgba(255, 255, 255, 0.12);
  /* 内高光 + 投影：玻璃方块质感，替代纯平色块 */
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.18),
    0 2px 8px rgba(0, 0, 0, 0.3);
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.02em;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.35);
  user-select: none;
}

.proj-title {
  flex: 1;
  min-width: 0;
  font-size: 13.5px;
  font-weight: 700;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 状态 Pill：小圆点 + 弱化文字（打开中 = 绿点） */
.proj-status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--success-tint);
  color: var(--success);
}
.proj-status .status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  box-shadow: 0 0 4px currentColor;
}

/* ---- 第 2 行：描述（固定两行高，齐卡片高度） ---- */
.proj-desc {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-2);
  /* 两行截断；min-height 保留两行空间，空描述也不塌陷 */
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
  min-height: 36px;
}
/* 空描述：更淡的占位文案 */
.proj-desc.empty {
  color: var(--text-3);
}

/* ---- 第 3 行：指标（API · 场景 · 更新时间） ---- */
.proj-metrics {
  display: flex;
  align-items: center;
  gap: 7px;
  /* 防换行 + 兜底裁剪：窄卡片下交给 ellipsis 而不是溢出卡片 */
  white-space: nowrap;
  min-width: 0;
  overflow: hidden;
  font-family: var(--font-mono);
  font-size: var(--fs-xxs);
  line-height: 1.5;
  color: var(--text-3);
}

.metric {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
  flex-shrink: 0;
}

.metric-sep {
  color: var(--border-strong);
  flex-shrink: 0;
}

/* ---- 第 4 行：进入项目提示（常驻占位保高度，hover 提亮） ---- */
.proj-foot {
  display: flex;
  justify-content: flex-end;
}

.proj-open-hint {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  white-space: nowrap;
  font-size: var(--fs-xxs);
  font-weight: 600;
  color: var(--text-3);
  transition: color var(--dur) var(--ease);
}
.proj-card:hover .proj-open-hint {
  color: var(--accent);
}

/* 拖拽手柄：左上角 6 点，hover 浮现，仅手柄可拖 */
.dnd-handle {
  position: absolute;
  top: 12px;
  left: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 20px;
  border-radius: 5px;
  color: var(--text-3);
  cursor: grab;
  opacity: 0;
  transition:
    opacity var(--dur) var(--ease),
    color var(--dur) var(--ease),
    background var(--dur) var(--ease);
  user-select: none;
  z-index: 1;
}
.proj-card:hover .dnd-handle {
  opacity: 1;
}
.dnd-handle:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.dnd-handle:active {
  cursor: grabbing;
}

/* ---------- 卡片右上角：更多菜单 ---------- */
.proj-side {
  position: absolute;
  top: 10px;
  right: 10px;
}

.proj-more {
  position: relative;
  flex-shrink: 0;
}

.more-menu {
  position: absolute;
  top: 30px;
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
</style>
