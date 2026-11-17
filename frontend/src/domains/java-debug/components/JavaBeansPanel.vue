<script setup lang="ts">
/**
 * JavaBeansPanel：Spring Bean 探测视图（attach 工作台左栏 Bean 页签）。
 * 分级支持展示：spring_present / 上下文捕获状态 / 不可用原因（诚实）；
 * Bean 条目标注 scope / lazy / instantiated / proxy——未实例化的 Bean
 * 调用方法会触发创建，UI 必须可区分。
 */
import { computed } from 'vue'
import StatusBadge, { type StatusTone } from '@/components/ui/StatusBadge.vue'
import { useLocaleStore } from '@/stores/locale'
import type { JavaBeanInfo, JavaSpringStatus } from '@/types/miyoApi'

const props = defineProps<{
  status: JavaSpringStatus | null
  beans: JavaBeanInfo[]
  total: number
  loading: boolean
  search: string
  activeBeanName: string | null
}>()

const emit = defineEmits<{
  (e: 'update:search', value: string): void
  (e: 'select', bean: JavaBeanInfo): void
  (e: 'reload'): void
}>()

const t = useLocaleStore().t

/** 分级支持：present+captured=已验证支持；waiting=条件支持；否则不支持。 */
const supportTone = computed<StatusTone>(() => {
  if (!props.status) return 'wait'
  if (!props.status.spring_present) return 'fail'
  if (props.status.contexts.length > 0) return 'ok'
  return 'wait'
})

const supportLabel = computed(() => {
  if (!props.status) return t('java2.springChecking')
  if (!props.status.spring_present) return t('java2.springAbsent')
  if (props.status.contexts.length > 0) return t('java2.springReady')
  return t('java2.springConditional')
})
</script>

<template>
  <div class="jbp" data-testid="java-beans-panel">
    <div class="jbp-status">
      <StatusBadge :tone="supportTone" :label="supportLabel" :dot="supportTone === 'ok'" />
      <span v-if="status?.framework_version && status.framework_version !== 'unknown'" class="jbp-ver">
        Spring {{ status.framework_version }}
        <template v-if="status.boot_version && status.boot_version !== 'unknown'">
          · Boot {{ status.boot_version }}
        </template>
      </span>
    </div>
    <div v-if="status && status.note" class="jbp-note">{{ status.note }}</div>

    <input
      class="miyo-input jbp-search"
      type="text"
      :value="search"
      :placeholder="t('java2.beanSearchPlaceholder')"
      data-testid="java-bean-search"
      @input="emit('update:search', ($event.target as HTMLInputElement).value)"
      @keydown.enter="emit('reload')"
    />

    <div class="jbp-list" data-testid="java-bean-list">
      <div v-if="loading && beans.length === 0" class="jbp-empty">{{ t('java2.loading') }}</div>
      <div v-else-if="beans.length === 0" class="jbp-empty">{{ t('java2.noBeans') }}</div>
      <button
        v-for="bean in beans"
        :key="bean.name"
        type="button"
        class="jbp-item"
        :class="{ active: bean.name === activeBeanName }"
        :data-testid="`java-bean-${bean.name}`"
        :title="bean.error ?? bean.definition_type ?? ''"
        @click="emit('select', bean)"
      >
        <div class="jbp-item-top">
          <span class="jbp-name">{{ bean.name }}</span>
          <span v-if="bean.instantiated" class="jbp-tag ok">{{ t('java2.beanInstantiated') }}</span>
          <span v-else class="jbp-tag">{{ t('java2.beanLazy') }}</span>
        </div>
        <div class="jbp-type">{{ bean.actual_type ?? bean.definition_type ?? t('java2.beanTypeUnknown') }}</div>
        <div class="jbp-meta">
          <span v-if="bean.scope" class="jbp-chip">{{ bean.scope }}</span>
          <span v-if="bean.primary" class="jbp-chip">primary</span>
          <span v-if="bean.proxy" class="jbp-chip">{{ bean.proxy_kind === 'jdk_dynamic' ? 'JDK proxy' : 'CGLIB' }}</span>
        </div>
      </button>
    </div>

    <div class="jbp-footer">
      {{ t('java2.beanCount', { total, shown: beans.length }) }}
    </div>
  </div>
</template>

<style scoped>
.jbp {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  gap: 8px;
}
.jbp-status {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.jbp-ver {
  font-size: 11px;
  color: var(--text-secondary);
}
.jbp-note {
  font-size: 11px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-secondary);
  line-height: 1.5;
}
.jbp-search {
  height: 26px;
  font-size: 12px;
  flex-shrink: 0;
}
.jbp-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.jbp-empty {
  font-size: 12px;
  color: var(--text-secondary);
  padding: 16px 8px;
  text-align: center;
}
.jbp-item {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 7px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
  text-align: left;
  color: var(--text);
  background: transparent;
}
.jbp-item:hover {
  background: var(--bg-hover);
}
.jbp-item.active {
  border-color: var(--primary, #6366f1);
}
.jbp-item-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}
.jbp-name {
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jbp-tag {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  white-space: nowrap;
}
.jbp-tag.ok {
  color: var(--ok, #22c55e);
}
.jbp-type {
  font-size: 10.5px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.jbp-meta {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}
.jbp-chip {
  font-size: 10px;
  padding: 0 6px;
  border-radius: 999px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}
.jbp-footer {
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}
</style>
