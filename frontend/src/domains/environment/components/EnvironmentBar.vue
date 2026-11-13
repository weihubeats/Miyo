<script setup lang="ts">
/**
 * EnvironmentBar：环境选择器（工作区顶部栏右侧）。
 * - 选择器本体抽到全局共享组件 EnvironmentSelector（group 外观：融进本组控）；
 * - 与 👁️（环境变量速览）拼成一个边线相连的组控（eb-group）：边框 / 底色 / 焦点环
 *   在组控上统一表达，触发器自身不再叠加；
 * - 环境管理弹窗由 EnvironmentSelector 自持（其底栏「新建 / 管理」是唯一入口）。
 */
import { computed, ref } from 'vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import EnvironmentSelector from '@/domains/environment/components/EnvironmentSelector.vue'
import EnvironmentQuickView from '@/domains/environment/components/EnvironmentQuickView.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Tooltip from '@/components/ui/Tooltip.vue'

const store = useWorkspaceStore()
const t = useLocaleStore().t

const barEl = ref<HTMLElement | null>(null)
const selectorRef = ref<InstanceType<typeof EnvironmentSelector> | null>(null)
const showQuick = ref(false)

const activeEnv = computed(
  () => store.environments.find((e) => e.id === store.activeEnvId) ?? null,
)

/** 环境速览里的「管理环境」：转交选择器自持的管理弹窗（同实例复用）。 */
function onQuickManage(): void {
  showQuick.value = false
  selectorRef.value?.openManager()
}
</script>

<template>
  <div ref="barEl" class="env-bar">
    <div class="eb-group">
      <EnvironmentSelector ref="selectorRef" class="eb-select" variant="group" />
      <span class="eb-sep" aria-hidden="true"></span>
      <Tooltip :content="activeEnv ? t('envbar.viewVars') : t('envbar.noActiveEnv')" placement="bottom">
        <IconButton
          class="eb-eye"
          name="eye"
          :size="14"
          :label="activeEnv ? t('envbar.viewVars') : t('envbar.noActiveEnv')"
          :disabled="!activeEnv"
          @click="showQuick = !showQuick"
        />
      </Tooltip>
    </div>

    <EnvironmentQuickView
      v-if="showQuick && activeEnv"
      :anchor="barEl"
      @close="showQuick = false"
      @manage="onQuickManage"
    />
  </div>
</template>

<style scoped>
.env-bar {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

/* ---- 统一 32px Pill 组控：[下拉 | 👁️] ---- */
.eb-group {
  display: inline-flex;
  align-items: center;
  height: 32px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-hover);
  overflow: hidden;
  transition:
    border-color var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.eb-group:hover {
  border-color: var(--border-strong);
}
.eb-group:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-tint);
}

/* 定宽：触发器宽度不随环境名长短抖动（顶栏布局稳定），过长名称由内部截断 */
.eb-select {
  width: 168px;
  flex-shrink: 0;
}

.eb-sep {
  width: 1px;
  height: 18px;
  flex-shrink: 0;
  background: var(--border);
}

.eb-eye {
  width: 30px;
  height: 30px;
  border-radius: 0;
  background: transparent;
}
.eb-eye:hover:not(:disabled) {
  background: transparent;
}
</style>
