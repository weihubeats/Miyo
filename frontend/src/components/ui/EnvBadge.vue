<script setup lang="ts">
/**
 * EnvBadge：当前激活环境徽章（语义色点 + 名称 + Base URL）。
 * 用例运行场景（详情抽屉「立即运行」/ 面板「全部运行」）展示「请求将发往
 * 哪个环境」——运行链路统一取 store.activeEnvId，此处同源展示。
 */
import { computed } from 'vue'
import { useWorkspaceStore } from '@/stores/workspace'
import { useLocaleStore } from '@/stores/locale'
import { envBaseUrl, envColorClass } from '@/utils/environment'

const store = useWorkspaceStore()
const locale = useLocaleStore()
const t = locale.t

const env = computed(
  () => store.environments.find((e) => e.id === store.activeEnvId) ?? null,
)
const baseUrl = computed(() => envBaseUrl(env.value))
const colorClass = computed(() => (env.value ? envColorClass(env.value.name) : ''))
</script>

<template>
  <span class="env-badge" :title="env ? `${env.name} · ${baseUrl}` : undefined">
    <span class="env-badge-dot" :class="colorClass"></span>
    <template v-if="env">
      <span class="env-badge-name">{{ env.name }}</span>
      <span v-if="baseUrl" class="env-badge-url">{{ baseUrl }}</span>
    </template>
    <span v-else class="env-badge-name env-badge-none">{{ t('envbar.noEnv') }}</span>
  </span>
</template>

<style scoped>
.env-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  min-width: 0;
  max-width: 280px;
  padding: 3px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-2);
  font-size: 12px;
  line-height: 18px;
}
.env-badge-dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-3);
}
/* 语义色映射 utils/environment.ts 的 envColorClass（与环境条同色系） */
.env-badge-dot.dev {
  background: var(--success);
}
.env-badge-dot.test {
  background: var(--info);
}
.env-badge-dot.staging {
  background: var(--warning);
}
.env-badge-dot.prod {
  background: var(--orange);
}
.env-badge-dot.global {
  background: var(--accent);
}
.env-badge-name {
  flex-shrink: 0;
  color: var(--text-1);
  font-weight: 500;
}
.env-badge-url {
  min-width: 0;
  overflow: hidden;
  color: var(--text-3);
  font-family: var(--font-mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.env-badge-none {
  color: var(--text-3);
  font-weight: 400;
}
</style>
