<script setup lang="ts">
/**
 * ToolsDrawer：接口工具抽屉，点击请求栏「工具」按钮打开。
 * 承载低频工具：断言测试（miyo-test 运行器）与压测（并发基准）。
 */
import Drawer from './ui/Drawer.vue'
import { lazyComponent } from '../composables/lazyComponent'
import { useLocaleStore } from '../stores/locale'
import type { Endpoint } from '../types/miyoApi'

// 抽屉内容挂在 Drawer 的 v-if="open" 分支下：不打开抽屉就不实例化，
// 两个工具面板（断言 / 压测）各自的 chunk 只在首次打开「工具」时拉取。
const LoadTestPanel = lazyComponent(() => import('./LoadTestPanel.vue'))
const TestsPanel = lazyComponent(() => import('./TestsPanel.vue'))

defineProps<{
  open: boolean
  draft: Endpoint | null
  url: string
}>()

const emit = defineEmits<{ close: [] }>()

const locale = useLocaleStore()
const t = locale.t
</script>

<template>
  <Drawer :open="open" :title="t('tools.title')" width="560px" @close="emit('close')">
    <div class="tools">
      <section class="tool-sec">
        <h3 class="tool-title">{{ t('tools.testsTitle') }}</h3>
        <p class="tool-hint">{{ t('tools.testsHint') }}</p>
        <TestsPanel :draft="draft" :url="url" />
      </section>

      <section class="tool-sec">
        <h3 class="tool-title">{{ t('tools.loadTitle') }}</h3>
        <LoadTestPanel :draft="draft" :url="url" />
      </section>
    </div>
  </Drawer>
</template>

<style scoped>
.tools {
  display: flex;
  flex-direction: column;
  gap: 22px;
}

.tool-sec {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tool-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}

.tool-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
</style>
