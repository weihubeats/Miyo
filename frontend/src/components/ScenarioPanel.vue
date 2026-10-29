<script setup lang="ts">
/**
 * ScenarioPanel：侧边栏「场景」页签——项目级集成链路列表。
 * 行为：点击行打开场景编排页（scenario:{id} 伪页签）；+ 新建（默认名落库后打开）；
 * 行内删除（确认后连编排页签一起关闭）。
 */
import { useLocaleStore } from '../stores/locale'
import { useWorkspaceStore } from '../stores/workspace'
import EmptyState from './ui/EmptyState.vue'
import Icon from './ui/Icon.vue'
import Popconfirm from './ui/Popconfirm.vue'

const store = useWorkspaceStore()
const t = useLocaleStore().t

async function onCreate(): Promise<void> {
  const created = await store.createScenario()
  if (created) store.openScenario(created.id)
}
</script>

<template>
  <div class="scenario-panel">
    <div class="sp-toolbar">
      <span class="sp-count">{{ t('scenario.count', { n: store.scenarios.length }) }}</span>
      <button
        class="tool-add"
        type="button"
        :aria-label="t('scenario.new')"
        :title="t('scenario.new')"
        @click="onCreate"
      >
        <Icon name="plus" :size="14" />
      </button>
    </div>

    <div v-if="store.scenarios.length" class="sp-list">
      <div
        v-for="s in store.scenarios"
        :key="s.id"
        class="sp-row"
        role="button"
        tabindex="0"
        :class="{ active: store.activeTabId === `scenario:${s.id}` }"
        @click="store.openScenario(s.id)"
        @keydown.enter="store.openScenario(s.id)"
      >
        <Icon name="beaker" :size="13" class="sp-icon" />
        <span class="sp-name" :title="s.name">{{ s.name }}</span>
        <span class="sp-steps">{{ t('scenario.stepCount', { n: s.steps.length }) }}</span>
        <Popconfirm
          :title="t('scenario.confirmDelete', { name: s.name })"
          :confirm-text="t('common.delete')"
          @confirm="store.removeScenario(s.id)"
        >
          <button
            class="sp-delete"
            type="button"
            :aria-label="t('scenario.delete')"
            @click.stop
          >
            <Icon name="trash" :size="12" />
          </button>
        </Popconfirm>
      </div>
    </div>

    <EmptyState
      v-else
      icon="beaker"
      :title="t('scenario.emptyTitle')"
      :description="t('scenario.emptyDesc')"
      compact
    >
      <button class="miyo-btn miyo-btn-sm" type="button" @click="onCreate">
        <Icon name="plus" :size="13" /> {{ t('scenario.emptyAction') }}
      </button>
    </EmptyState>
  </div>
</template>

<style scoped>
.scenario-panel {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.sp-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px 6px;
}

.sp-count {
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sp-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 0 6px 8px;
}

.sp-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-1);
  min-width: 0;
}

.sp-row:hover {
  background: var(--bg-hover);
}

.sp-row.active {
  background: var(--bg-active, var(--bg-hover));
}

.sp-icon {
  flex: none;
  color: var(--text-3);
}

.sp-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
}

.sp-steps {
  flex: none;
  font-size: var(--fs-xxs);
  color: var(--text-3);
}

.sp-delete {
  flex: none;
  display: none;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: var(--radius-sm);
}

.sp-row:hover .sp-delete {
  display: inline-flex;
}

.sp-delete:hover {
  color: var(--danger);
  background: var(--bg-hover);
}
</style>
