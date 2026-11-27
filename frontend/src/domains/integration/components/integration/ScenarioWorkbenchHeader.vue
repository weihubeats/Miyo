<script setup lang="ts">
/**
 * ScenarioWorkbenchHeader：场景详情页的全局导航栏（三层结构的第一层）。
 *
 * 布局：
 * - 左：返回场景列表（始终可见）+ 集成测试图标 + 场景名（内联编辑）+ 所属目录；
 * - 中：编辑 / 脚本 / 执行结果 三个主视图页签；
 * - 右：当前环境 + 保存状态 + 保存 + 运行；低频操作收进「更多」菜单，
 *   避免顶部入口过多。
 *
 * 纯展示组件：保存 / 运行 / 返回 / 切换页签全部 emit 给 WorkflowEditor，
 * 由宿主持有编辑态与运行态。类名 `wf-topbar` / `wf-name` / `wf-save-state`
 * 与既有测试和样式保持兼容。
 */
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Tabs, { type TabItem } from '@/components/ui/Tabs.vue'
import EnvironmentSelector from '@/domains/environment/components/EnvironmentSelector.vue'
import IntegrationNavBar, {
  type NavCrumb,
} from '@/domains/integration/components/integration/IntegrationNavBar.vue'

const props = defineProps<{
  name: string
  /** 返回目标文案（返回测试场景 / 返回批次详情…）。 */
  backLabel?: string
  /** 面包屑祖先层级（不含当前场景名；当前名由 name 输入承载）。 */
  crumbs?: NavCrumb[]
  tab: string
  tabs: TabItem[]
  dirty: boolean
  saving: boolean
  saveError: boolean
  running: boolean
}>()

const emit = defineEmits<{
  (e: 'update:name', value: string): void
  (e: 'update:tab', value: string): void
  (e: 'back'): void
  (e: 'save'): void
  (e: 'run'): void
  (e: 'more', event: MouseEvent): void
}>()

const t = useLocaleStore().t

const effectiveBackLabel = computed(
  () => props.backLabel || t('integration.navBackScenarios'),
)

const saveStateText = computed(() => {
  if (props.saving) return t('integration.saveStateSaving')
  if (props.saveError) return t('integration.saveStateFailed')
  if (props.dirty) return t('integration.saveStateDirty')
  return t('integration.saveStateSaved')
})

const saveStateClass = computed(() => {
  if (props.saving) return 'saving'
  if (props.saveError) return 'failed'
  if (props.dirty) return 'dirty'
  return 'saved'
})
</script>

<template>
  <header class="wf-topbar">
    <div class="wf-topbar-left">
      <span class="wf-scenario-icon" aria-hidden="true">
        <Icon name="beaker" :size="14" />
      </span>
      <IntegrationNavBar
        class="wf-back"
        :back-label="effectiveBackLabel"
        :crumbs="crumbs ?? []"
        suffix
        @back="emit('back')"
      />
      <span class="wf-nav-slash" aria-hidden="true">/</span>
      <input
        :value="name"
        class="wf-name"
        type="text"
        :placeholder="t('integration.editorNamePh')"
        spellcheck="false"
        @input="emit('update:name', ($event.target as HTMLInputElement).value)"
      >
    </div>

    <Tabs
      class="wf-view-tabs"
      :model-value="tab"
      :tabs="tabs"
      size="sm"
      @update:model-value="(v: string) => emit('update:tab', v)"
    />

    <div class="wf-topbar-right">
      <EnvironmentSelector variant="group" />
      <span class="wf-save-state" :class="saveStateClass" :title="saveStateText">
        <span class="wf-save-dot"></span>{{ saveStateText }}
      </span>
      <button class="miyo-btn miyo-btn-sm" type="button" :disabled="saving" @click="emit('save')">
        <Icon name="save" :size="12" /> {{ t('integration.save') }}
      </button>
      <button
        class="miyo-btn miyo-btn-sm"
        :class="running ? 'miyo-btn-danger-solid' : 'miyo-btn-primary'"
        type="button"
        @click="emit('run')"
      >
        <Icon :name="running ? 'stop' : 'play'" :size="12" />
        {{ running ? t('integration.stop') : t('integration.run') }}
      </button>
      <IconButton
        name="more-horizontal"
        :size="15"
        :title="t('integration.moreActions')"
        @click="emit('more', $event)"
      />
    </div>
  </header>
</template>

<style scoped>
.wf-topbar {
  display: flex;
  align-items: center;
  gap: 14px;
  min-height: 46px;
  padding: 6px 14px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
}

.wf-topbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1 1 auto;
}

.wf-back {
  flex: none;
  min-width: 0;
}

.wf-nav-slash {
  flex: none;
  color: var(--text-3);
  font-size: var(--fs-xxs);
}

.wf-scenario-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  flex: none;
  border-radius: var(--radius-sm);
  background: var(--accent-tint);
  color: var(--accent);
}

.wf-name {
  min-width: 120px;
  max-width: 280px;
  flex: 0 1 280px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  padding: 2px 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}

.wf-name:hover {
  border-color: var(--border);
  background: var(--bg-hover);
}

.wf-name:focus {
  outline: none;
  border-color: var(--accent);
  background: var(--bg-card);
}


.wf-view-tabs {
  flex: none;
}

.wf-topbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
  flex: none;
}

.wf-save-state {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xxs);
  color: var(--text-3);
  white-space: nowrap;
}

.wf-save-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-3);
}

.wf-save-state.saved .wf-save-dot {
  background: var(--success);
}

.wf-save-state.dirty .wf-save-dot {
  background: var(--warning);
}

.wf-save-state.failed {
  color: var(--danger);
}

.wf-save-state.failed .wf-save-dot {
  background: var(--danger);
}

.wf-save-state.saving .wf-save-dot {
  background: var(--accent);
}

/* 小窗口：目录与保存状态文字让位，保证页签与运行按钮可用 */
@media (max-width: 1100px) {
  .wf-save-state {
    display: none;
  }

  .wf-name {
    max-width: 180px;
    flex-basis: 180px;
  }
}

@media (max-width: 860px) {
  .wf-topbar {
    gap: 8px;
    padding: 6px 8px;
  }

  .wf-name {
    max-width: 130px;
    flex-basis: 130px;
  }
}
</style>