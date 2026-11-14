<script setup lang="ts">
/**
 * CreateChoicePage：「+」新建选择页——独立页面而非下拉弹层（对标 Apifox 新建页）。
 * 占据内容区全部空间，居中两张竖排大卡片（大徽章 + 标签）：
 * 新建 HTTP 请求 / 导入 cURL。Esc 退出；选择动作由父级关闭页面并执行。
 */
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { useLocaleStore } from '@/stores/locale'
import Icon from '@/components/ui/Icon.vue'
import type { CreateActionKey } from '@/domains/workspace/components/CreatePanel.vue'

const emit = defineEmits<{
  pick: [key: CreateActionKey]
  close: []
}>()

const locale = useLocaleStore()
const t = locale.t

/** 随语言刷新：常量数组必须包 computed（i18n 规范）。 */
const actions = computed(() => [
  { key: 'new-request' as const, label: t('workspace.newRequest'), icon: 'zap' as const, tone: 'blue' },
  { key: 'import-curl' as const, label: t('workspace.importCurl'), icon: 'terminal' as const, tone: 'green' },
])

function onDocKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') emit('close')
}

onMounted(() => window.addEventListener('keydown', onDocKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onDocKeydown))
</script>

<template>
  <div class="cc-page" role="menu" :aria-label="t('tabbar.newRequestHint')">
    <div class="cc-cards">
      <button
        v-for="a in actions"
        :key="a.key"
        type="button"
        class="cc-card"
        role="menuitem"
        @click="emit('pick', a.key)"
      >
        <span class="cc-icon" :class="`tone-${a.tone}`">
          <Icon :name="a.icon" :size="36" :stroke-width="1.5" />
        </span>
        <span class="cc-label">{{ a.label }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.cc-page {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow-y: auto;
  background: var(--bg-app);
}

.cc-cards {
  display: flex;
  gap: 18px;
}

/* 竖排大卡片：大徽章居中在上、标签在下（对标 Apifox 新建页） */
.cc-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 30px;
  width: 210px;
  height: 250px;
  padding: 24px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xl);
  background: var(--bg-panel);
  color: var(--text-1);
  cursor: pointer;
  transition:
    border-color var(--dur) var(--ease),
    background var(--dur) var(--ease),
    transform var(--dur) var(--ease),
    box-shadow var(--dur) var(--ease);
}
.cc-card:hover {
  border-color: var(--accent);
  background: var(--bg-elevated);
  transform: translateY(-3px);
  box-shadow: var(--shadow);
}
.cc-card:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.cc-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 76px;
  height: 76px;
  border-radius: 20px;
}
.tone-blue {
  color: var(--info);
  background: color-mix(in srgb, var(--info) 14%, transparent);
}
.tone-green {
  color: var(--success);
  background: color-mix(in srgb, var(--success) 14%, transparent);
}

.cc-label {
  font-size: var(--fs-md);
  font-weight: 600;
}
</style>
