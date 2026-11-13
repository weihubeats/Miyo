<script setup lang="ts">
/**
 * EmptyState：空态（图标 + 标题 + 描述 + 操作插槽）。
 */
import Icon from '@/components/ui/Icon.vue'
import type { IconName } from '@/components/ui/Icon.vue'
import { computed } from 'vue'
import { useLocaleStore } from '@/stores/locale'

const props = withDefaults(
  defineProps<{
    icon?: IconName
    title?: string
    description?: string
    compact?: boolean
  }>(),
  { icon: 'folder-open', title: '', description: '', compact: false },
)

const locale = useLocaleStore()
const t = locale.t

/** 未传标题时按当前语言兜底。 */
const effectiveTitle = computed(() => props.title || t('empty.title'))
</script>

<template>
  <div class="es" :class="{ compact }">
    <span class="es-icon">
      <Icon :name="icon" :size="compact ? 20 : 26" :stroke-width="1.5" />
    </span>
    <p class="es-title">{{ effectiveTitle }}</p>
    <p v-if="description" class="es-desc">{{ description }}</p>
    <div v-if="$slots.default" class="es-actions">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.es {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 40px 16px;
  text-align: center;
}
.es.compact {
  padding: 24px 12px;
  gap: 6px;
}

.es-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 52px;
  height: 52px;
  border-radius: var(--radius-xl);
  background: var(--bg-hover);
  color: var(--text-3);
}
.es.compact .es-icon {
  width: 40px;
  height: 40px;
  border-radius: 12px;
}

.es-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}

.es-desc {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
  max-width: 320px;
}

.es-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}
</style>