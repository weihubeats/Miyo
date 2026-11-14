<script setup lang="ts">
/**
 * ProjectListRow：项目列表行（列表模式）。
 *
 * 项目数量较多时的紧凑视图：一行展示 图标 + 名称 + 状态（/最近失败点）+
 * API 数 + 场景数 + 请求成功率 + 更新时间 + 更多操作；整行可点击进入项目。
 * 分组归属由外层团队分组头呈现（同一分组结构，不重复造数据关系）。
 */
import { computed } from 'vue'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { useLocaleStore } from '@/stores/locale'
import type { Project, ProjectQualityStat } from '@/types/miyoApi.d'
import { avatarStyle, initials, timeAgo } from '@/domains/workspace/components/projectlist/projectlist/projectMeta'

const locale = useLocaleStore()
const t = locale.t

const props = defineProps<{
  project: Project
  count: number
  scenarioCount: number
  active: boolean
  /** 当前展开更多菜单的项目 id（父级持有，同时只开一个） */
  menuOpen: boolean
  /** 请求历史成功 / 失败数（list_project_stats；无记录 0） */
  okCount?: number
  failCount?: number
  /** 运行质量聚合（list_project_quality；未加载为 null） */
  quality?: ProjectQualityStat | null
}>()

/** 请求成功率文案（无请求记录显示 —，不虚构 0%）。 */
const reqRateText = computed(() => {
  const ok = props.okCount ?? 0
  const fail = props.failCount ?? 0
  if (ok + fail === 0) return '—'
  return `${Math.round((ok / (ok + fail)) * 100)}%`
})

/** 最近运行失败红点（场景或用例）。 */
const hasRecentFail = computed(() => {
  const q = props.quality
  return !!q && (q.scenario_last_fail > 0 || q.cases_failed > 0)
})

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
      <span
        v-else-if="hasRecentFail"
        class="row-faildot"
        :title="t('pcard.sceneFailDetail', { n: quality?.scenario_last_fail ?? 0, time: '' })"
        aria-hidden="true"
      ></span>
    </div>
    <span class="row-col row-metric" :title="t('pcard.apiCount', { n: count })">
      <Icon name="plug" :size="12" />{{ count }}
    </span>
    <span class="row-col row-metric row-scenario" :title="t('pcard.scenarioCount', { n: scenarioCount })">
      <Icon name="play" :size="11" />{{ scenarioCount }}
    </span>
    <span class="row-col row-metric row-rate" :title="t('pcard.reqRate', { rate: reqRateText })">
      <Icon name="gauge" :size="11" />{{ reqRateText }}
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
  grid-template-columns: 30px minmax(0, 1fr) 64px 64px 56px 92px 30px;
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
  font-size: var(--fs-xxs);
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
  font-size: var(--fs-sm);
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
  border-radius: var(--radius-pill);
  background: var(--success-tint);
  color: var(--success);
}
.row-status .status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
}

/* 最近运行失败红点（场景/用例最近一次运行有失败） */
.row-faildot {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--danger);
  box-shadow: 0 0 6px color-mix(in srgb, var(--danger) 55%, transparent);
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
  font-size: var(--fs-xs);
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

/* 窄窗口：先收场景列，再收成功率 / 时间列（列表仍有名称 + API 数可用） */
@media (max-width: 1200px) {
  .proj-row {
    grid-template-columns: 30px minmax(0, 1fr) 64px 56px 92px 30px;
  }
  .row-scenario {
    display: none;
  }
}
@media (max-width: 1024px) {
  .proj-row {
    grid-template-columns: 30px minmax(0, 1fr) 64px 92px 30px;
  }
  .row-rate {
    display: none;
  }
}
@media (max-width: 900px) {
  .proj-row {
    grid-template-columns: 30px minmax(0, 1fr) 64px 30px;
  }
  .row-time {
    display: none;
  }
}
</style>
